use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir as CapabilityDir, OpenOptions as CapabilityOpenOptions};
use file_id::{FileId, get_file_id};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::mcp::{
    StagedModelResult, StagedPartitionReplacement, StagedPartitionReplacementRequest,
    execute_staged_model_export_proof, execute_staged_partition_replacements,
    staged_partition_source_fingerprint,
};
use crate::microsoft::{MicrosoftComponent, resolve_installed_component};
use crate::project_io::write_json_new_atomic;
use crate::safety_scan::contains_credential_like_text_str;
use crate::tmdl::{
    MutationPlan, PartitionSelector, find_partition, load_table_documents,
    load_table_documents_from_semantic_model, replace_partition_source_plan,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_command,
};

mod plan;
mod run;
mod shared;
mod verify;

use plan::workflow_plan;
use run::workflow_run;
pub(crate) use shared::{
    ExportShapeProof, PreparedStagedModel, SourceTreeEvidence, SourceTreeSnapshot,
    validate_tmdl_definition,
};
use shared::{
    MAX_HASHED_TREE_BYTES, MAX_HASHED_TREE_FILES, MAX_PROFILE_BYTES, MAX_RESOURCE_BYTES, MToken,
    OwnedWorkflowOutput, canonical_plain_directory, canonical_plain_file, claim_for_file,
    copy_new_output_file, m_tokens, metadata_is_link_or_reparse, read_bounded,
    resolve_new_directory_candidate, unicode_path, validate_relative_path,
};
use verify::workflow_verify;

#[cfg(test)]
use run::{copy_claimed_files, copy_resources};

const MAX_SOURCE_TEXT_BYTES: u64 = 16 * 1024 * 1024;
const WORKFLOW_SYNTHESIZE_SCHEMA: &str = "powerbi-cli.workflow-synthesize.v1";
const INTEGRATION_LOCK_BYTES: &[u8] =
    include_bytes!("../integrations/microsoft/integration-lock.json");

#[derive(Debug)]
struct SynthesizeOptions {
    project: PathBuf,
    expressions: PathBuf,
    output_dir: PathBuf,
    mappings: BTreeMap<(String, String), String>,
}

#[derive(Debug)]
struct SynthesizePartitionEdit {
    path: PathBuf,
    line_index: usize,
    handle: String,
    server_strings: BTreeSet<String>,
}

#[derive(Debug)]
struct SynthesizeDiscovery {
    navigation_pairs: BTreeSet<(String, String)>,
    edits: Vec<SynthesizePartitionEdit>,
}

#[derive(Debug)]
struct PreservedLine {
    content: String,
    ending: String,
}

#[derive(Debug, Default)]
struct SynthesizeCopyStats {
    files_written: usize,
    directories_written: usize,
    files_excluded: usize,
    bytes_written: u64,
}

pub(crate) fn workflow_synthesize_command(args: &[String]) -> CliResult<Value> {
    let options = parse_synthesize_args(args)?;
    let resolved = resolve_project(&options.project)?;
    let source_root = canonical_plain_directory(&resolved.project_dir, "project root")?;
    let output_dir = resolve_new_directory_candidate(&options.output_dir)?;
    if output_dir.starts_with(&source_root) {
        return Err(CliError::invalid_args(
            "workflow synthesize output must be outside the source project tree",
        ));
    }

    let expressions_path = canonical_plain_file(
        &options.expressions,
        "synthetic expressions",
        MAX_SOURCE_TEXT_BYTES,
    )?;
    let expressions_bytes = read_bounded(
        &expressions_path,
        MAX_SOURCE_TEXT_BYTES,
        "synthetic expressions",
    )?;
    let expressions_text = std::str::from_utf8(&expressions_bytes).map_err(|_| {
        CliError::validation_failed("synthetic expressions file must be UTF-8 TMDL")
    })?;
    let defined_expressions = parse_expression_declarations(expressions_text)?;
    let discovery = discover_synthesize_edits(&resolved)?;
    let expression_mappings = resolve_synthesize_mappings(
        &discovery.navigation_pairs,
        &options.mappings,
        &defined_expressions,
    )?;
    let shim = synthesize_navigation_shim(&expression_mappings)?;

    let mut overrides = transformed_synthesize_tables(&source_root, &discovery.edits, &shim)?;
    let semantic_relative = resolved
        .semantic_model_dir
        .strip_prefix(&source_root)
        .map_err(|_| {
            CliError::validation_failed("semantic model escaped the source project tree")
        })?;
    let expressions_relative = semantic_relative
        .join("definition")
        .join("expressions.tmdl");
    if overrides
        .insert(expressions_relative.clone(), expressions_bytes)
        .is_some()
    {
        return Err(CliError::validation_failed(
            "synthetic expressions target collides with a transformed table file",
        ));
    }

    let output = OwnedWorkflowOutput::create(&output_dir)?;
    let copy = copy_synthesize_project(&source_root, &output, &overrides)?;
    output.verify_root()?;

    let selected_pbip = canonical_plain_file(&resolved.pbip_path, "PBIP", MAX_PROFILE_BYTES)?;
    let pbip_relative = selected_pbip.strip_prefix(&source_root).map_err(|_| {
        CliError::validation_failed("selected PBIP escaped the source project tree")
    })?;
    let output_pbip = output_dir.join(pbip_relative);
    let output_resolved = resolve_project(&output_pbip)?;
    let validation = validate_command(&[unicode_path(&output_pbip, "synthesized PBIP")?])?;
    let server_strings = discovery
        .edits
        .iter()
        .flat_map(|edit| edit.server_strings.iter().cloned())
        .collect::<BTreeSet<_>>();
    let offline_safety = synthesize_offline_safety(&output_resolved, &server_strings)?;
    let validation_ok = validation["ok"].as_bool().unwrap_or(false);
    let offline_ok = offline_safety["ok"].as_bool().unwrap_or(false);
    let ok = validation_ok && offline_ok;
    let mappings = expression_mappings
        .iter()
        .map(|((schema, item), expression)| {
            json!({
                "schema": schema,
                "item": item,
                "expression": expression
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "schema": WORKFLOW_SYNTHESIZE_SCHEMA,
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "sourceProjectDir": canonical_display(&source_root),
        "projectDir": canonical_display(&output_resolved.project_dir),
        "pbip": canonical_display(&output_resolved.pbip_path),
        "expressions": canonical_display(&output_dir.join(expressions_relative)),
        "mappings": mappings,
        "counts": {
            "navigationPairs": expression_mappings.len(),
            "partitionsModified": discovery.edits.len(),
            "filesWritten": copy.files_written,
            "directoriesWritten": copy.directories_written,
            "filesExcluded": copy.files_excluded,
            "bytesWritten": copy.bytes_written
        },
        "validation": validation,
        "offlineSafety": offline_safety,
        "next": [
            format!("powerbi-cli validate {} --json", command_arg(&output_resolved.pbip_path)),
            format!("powerbi-cli inspect {} --json", command_arg(&output_resolved.pbip_path))
        ]
    }))
}

fn parse_synthesize_args(args: &[String]) -> CliResult<SynthesizeOptions> {
    let mut project = None;
    let mut expressions = None;
    let mut output_dir = None;
    let mut mappings = BTreeMap::new();
    let mut index = 0_usize;
    while index < args.len() {
        match args[index].as_str() {
            "--project" => {
                if project.is_some() {
                    return Err(CliError::invalid_args(
                        "--project may be specified only once",
                    ));
                }
                project =
                    Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                        CliError::invalid_args("--project requires a path")
                    })?));
                index += 2;
            }
            "--expressions" => {
                if expressions.is_some() {
                    return Err(CliError::invalid_args(
                        "--expressions may be specified only once",
                    ));
                }
                expressions = Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    CliError::invalid_args("--expressions requires a TMDL file path")
                })?));
                index += 2;
            }
            "--out-dir" => {
                if output_dir.is_some() {
                    return Err(CliError::invalid_args(
                        "--out-dir may be specified only once",
                    ));
                }
                output_dir =
                    Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                        CliError::invalid_args("--out-dir requires a path")
                    })?));
                index += 2;
            }
            "--map" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    CliError::invalid_args("--map requires <schema.item>=<ExpressionName>")
                })?;
                let (pair, expression) = value.split_once('=').ok_or_else(|| {
                    CliError::invalid_args(format!(
                        "invalid --map value {value}; expected <schema.item>=<ExpressionName>"
                    ))
                })?;
                let (schema, item) = pair.rsplit_once('.').ok_or_else(|| {
                    CliError::invalid_args(format!(
                        "invalid --map key {pair}; expected <schema>.<item>"
                    ))
                })?;
                validate_synthesize_name(schema, "mapped schema")?;
                validate_synthesize_name(item, "mapped item")?;
                validate_synthesize_name(expression, "mapped expression")?;
                let key = (schema.to_string(), item.to_string());
                if mappings.insert(key, expression.to_string()).is_some() {
                    return Err(CliError::invalid_args(format!(
                        "duplicate --map for {pair}"
                    )));
                }
                index += 2;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown workflow synthesize argument: {other}"
                ))
                .with_hint(
                    "Use --project, --expressions, --out-dir, and optional repeated --map flags.",
                ));
            }
        }
    }
    Ok(SynthesizeOptions {
        project: project
            .ok_or_else(|| CliError::invalid_args("workflow synthesize requires --project"))?,
        expressions: expressions
            .ok_or_else(|| CliError::invalid_args("workflow synthesize requires --expressions"))?,
        output_dir: output_dir
            .ok_or_else(|| CliError::invalid_args("workflow synthesize requires --out-dir"))?,
        mappings,
    })
}

fn validate_synthesize_name(value: &str, label: &str) -> CliResult<()> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > 256
        || value.chars().any(char::is_control)
    {
        return Err(CliError::invalid_args(format!("invalid {label}")));
    }
    Ok(())
}

fn parse_expression_declarations(text: &str) -> CliResult<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("expression") else {
            continue;
        };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim_start();
        let (name, tail) = if let Some(quoted) = rest.strip_prefix('\'') {
            let mut name = String::new();
            let mut chars = quoted.chars().peekable();
            let mut consumed = 1_usize;
            let mut closed = false;
            while let Some(character) = chars.next() {
                consumed += character.len_utf8();
                if character == '\'' {
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                        consumed += 1;
                        name.push('\'');
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    name.push(character);
                }
            }
            if !closed {
                return Err(CliError::validation_failed(
                    "synthetic expressions file has an unterminated quoted expression name",
                ));
            }
            (name, &rest[consumed..])
        } else {
            let equals = rest.find('=').ok_or_else(|| {
                CliError::validation_failed("synthetic expression declaration must contain '='")
            })?;
            let name = rest[..equals].trim();
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return Err(CliError::validation_failed(
                    "synthetic expression declaration has an invalid name",
                ));
            }
            (name.to_string(), &rest[equals..])
        };
        if !tail.trim_start().starts_with('=') {
            return Err(CliError::validation_failed(format!(
                "synthetic expression declaration for {name} must contain '='"
            )));
        }
        names.insert(name);
    }
    Ok(names)
}

fn discover_synthesize_edits(resolved: &crate::ResolvedProject) -> CliResult<SynthesizeDiscovery> {
    let docs = load_table_documents(resolved)?;
    let mut navigation_pairs = BTreeSet::new();
    let mut edits = Vec::new();
    let mut unpaired_connectors = Vec::new();
    let mut file_lines = BTreeMap::<PathBuf, Vec<PreservedLine>>::new();

    for doc in &docs {
        if !file_lines.contains_key(&doc.path) {
            let bytes = read_bounded(&doc.path, MAX_SOURCE_TEXT_BYTES, "TMDL table")?;
            let text = String::from_utf8(bytes)
                .map_err(|_| CliError::validation_failed("TMDL table must be UTF-8"))?;
            file_lines.insert(doc.path.clone(), split_preserved_lines(&text));
        }
        let lines = file_lines
            .get(&doc.path)
            .expect("table lines inserted before partition scan");
        for partition in &doc.partitions {
            let Some(source) = partition.source.as_deref() else {
                continue;
            };
            let local_pairs = navigation_pairs_from_source(source)?;
            navigation_pairs.extend(local_pairs.iter().cloned());
            let start = partition.source_start_line.unwrap_or(partition.start_line);
            let end = partition.source_end_line.unwrap_or(partition.end_line);
            let mut connector_lines = Vec::new();
            for (line_index, line) in lines
                .iter()
                .enumerate()
                .take(end.min(lines.len()))
                .skip(start)
            {
                if let Some(server_strings) = connector_binding(&line.content)? {
                    connector_lines.push((line_index, server_strings));
                }
            }
            if local_pairs.is_empty() {
                if !connector_lines.is_empty() {
                    unpaired_connectors.push(partition.handle());
                }
                continue;
            }
            match connector_lines.as_slice() {
                [(line_index, server_strings)] => edits.push(SynthesizePartitionEdit {
                    path: doc.path.clone(),
                    line_index: *line_index,
                    handle: partition.handle(),
                    server_strings: server_strings.clone(),
                }),
                [] => {}
                _ => {
                    return Err(CliError::validation_failed(format!(
                        "partition {} contains multiple recognizable Database connector lines",
                        partition.handle()
                    )));
                }
            }
        }
    }
    if edits.is_empty() {
        return Err(CliError::validation_failed(
            "project has no recognizable shared Database = <Connector>.Database(...) step followed by Schema/Item navigation",
        ));
    }
    if !unpaired_connectors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "connector-bearing partitions have no recognizable Schema/Item navigation and cannot be made offline-safe: {}",
            unpaired_connectors.join(", ")
        )));
    }
    Ok(SynthesizeDiscovery {
        navigation_pairs,
        edits,
    })
}

fn connector_binding(line: &str) -> CliResult<Option<BTreeSet<String>>> {
    let tokens = m_tokens(line.trim_start())?;
    let server = match tokens.as_slice() {
        [
            MToken::Ident(binding),
            MToken::Equals,
            MToken::Ident(connector),
            MToken::LParen,
            MToken::String(server),
            ..,
        ] if binding == "Database" && is_database_connector(connector) => server,
        _ => return Ok(None),
    };
    let server_strings = [server.clone()].into_iter().collect();
    Ok(Some(server_strings))
}

fn is_database_connector(value: &str) -> bool {
    value
        .rsplit_once('.')
        .is_some_and(|(namespace, function)| !namespace.is_empty() && function == "Database")
}

fn navigation_pairs_from_source(source: &str) -> CliResult<BTreeSet<(String, String)>> {
    let tokens = m_tokens(source)?;
    let mut pairs = BTreeSet::new();
    for window in tokens.windows(15) {
        if let [
            MToken::Ident(database),
            MToken::Other('{'),
            MToken::Other('['),
            MToken::Ident(schema_key),
            MToken::Equals,
            MToken::String(schema),
            MToken::Comma,
            MToken::Ident(item_key),
            MToken::Equals,
            MToken::String(item),
            MToken::Other(']'),
            MToken::Other('}'),
            MToken::Other('['),
            MToken::Ident(data),
            MToken::Other(']'),
        ] = window
            && database == "Database"
            && schema_key == "Schema"
            && item_key == "Item"
            && data == "Data"
        {
            validate_synthesize_name(schema, "navigation schema")?;
            validate_synthesize_name(item, "navigation item")?;
            pairs.insert((schema.clone(), item.clone()));
        }
    }
    Ok(pairs)
}

fn resolve_synthesize_mappings(
    pairs: &BTreeSet<(String, String)>,
    overrides: &BTreeMap<(String, String), String>,
    defined: &BTreeSet<String>,
) -> CliResult<BTreeMap<(String, String), String>> {
    let unknown = overrides
        .keys()
        .filter(|pair| !pairs.contains(*pair))
        .map(|(schema, item)| format!("{schema}.{item}"))
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(CliError::invalid_args(format!(
            "--map targets were not discovered in any partition: {}",
            unknown.join(", ")
        )));
    }
    let mut mappings = BTreeMap::new();
    let mut missing = BTreeSet::new();
    for pair in pairs {
        let expression = overrides
            .get(pair)
            .cloned()
            .unwrap_or_else(|| format!("Synth{}", upper_camel(&pair.1)));
        if !defined.contains(&expression) {
            missing.insert(expression.clone());
        }
        mappings.insert(pair.clone(), expression);
    }
    if !missing.is_empty() {
        return Err(CliError::validation_failed(format!(
            "synthetic expressions file is missing required expression definitions: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(mappings)
}

fn upper_camel(value: &str) -> String {
    let mut result = String::new();
    let mut capitalize = true;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if capitalize {
                result.extend(character.to_uppercase());
                capitalize = false;
            } else {
                result.push(character);
            }
        } else {
            capitalize = true;
        }
    }
    result
}

fn synthesize_navigation_shim(mappings: &BTreeMap<(String, String), String>) -> CliResult<String> {
    let rows = mappings
        .iter()
        .map(|((schema, item), expression)| {
            Ok(format!(
                "{{{}, {}, {}}}",
                m_navigation_string(schema)?,
                m_navigation_string(item)?,
                m_expression_reference(expression)?
            ))
        })
        .collect::<CliResult<Vec<_>>>()?;
    Ok(format!(
        "Database = #table({{\"Schema\", \"Item\", \"Data\"}}, {{{}}}),",
        rows.join(", ")
    ))
}

fn m_navigation_string(value: &str) -> CliResult<String> {
    if value.chars().any(char::is_control) {
        return Err(CliError::validation_failed(
            "navigation schema/item contains an unsupported control character",
        ));
    }
    Ok(format!("\"{}\"", value.replace('"', "\"\"")))
}

fn m_expression_reference(value: &str) -> CliResult<String> {
    validate_synthesize_name(value, "expression name")?;
    let mut characters = value.chars();
    let regular = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric());
    if regular {
        Ok(value.to_string())
    } else {
        Ok(format!("#\"{}\"", value.replace('"', "\"\"")))
    }
}

fn transformed_synthesize_tables(
    source_root: &Path,
    edits: &[SynthesizePartitionEdit],
    shim: &str,
) -> CliResult<BTreeMap<PathBuf, Vec<u8>>> {
    let mut by_path = BTreeMap::<PathBuf, Vec<&SynthesizePartitionEdit>>::new();
    for edit in edits {
        by_path.entry(edit.path.clone()).or_default().push(edit);
    }
    let mut transformed = BTreeMap::new();
    for (path, file_edits) in by_path {
        let bytes = read_bounded(&path, MAX_SOURCE_TEXT_BYTES, "TMDL table")?;
        let text = String::from_utf8(bytes)
            .map_err(|_| CliError::validation_failed("TMDL table must be UTF-8"))?;
        let mut lines = split_preserved_lines(&text);
        for edit in file_edits {
            let line = lines.get_mut(edit.line_index).ok_or_else(|| {
                CliError::validation_failed(format!(
                    "connector line moved while preparing {}",
                    edit.handle
                ))
            })?;
            if connector_binding(&line.content)?.is_none() {
                return Err(CliError::validation_failed(format!(
                    "connector line drifted while preparing {}",
                    edit.handle
                )));
            }
            let indent_bytes = line
                .content
                .find(|character: char| !character.is_whitespace())
                .unwrap_or(line.content.len());
            line.content = format!("{}{shim}", &line.content[..indent_bytes]);
        }
        let relative = path.strip_prefix(source_root).map_err(|_| {
            CliError::validation_failed("TMDL table escaped the source project tree")
        })?;
        transformed.insert(
            relative.to_path_buf(),
            join_preserved_lines(lines).into_bytes(),
        );
    }
    Ok(transformed)
}

fn split_preserved_lines(text: &str) -> Vec<PreservedLine> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut index = 0_usize;
    while index < bytes.len() {
        if bytes[index] == b'\r' || bytes[index] == b'\n' {
            let content = text[start..index].to_string();
            let end = if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index + 2
            } else {
                index + 1
            };
            lines.push(PreservedLine {
                content,
                ending: text[index..end].to_string(),
            });
            start = end;
            index = end;
        } else {
            index += 1;
        }
    }
    if start < text.len() {
        lines.push(PreservedLine {
            content: text[start..].to_string(),
            ending: String::new(),
        });
    }
    lines
}

fn join_preserved_lines(lines: Vec<PreservedLine>) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(&line.content);
        text.push_str(&line.ending);
    }
    text
}

fn copy_synthesize_project(
    source_root: &Path,
    output: &OwnedWorkflowOutput,
    overrides: &BTreeMap<PathBuf, Vec<u8>>,
) -> CliResult<SynthesizeCopyStats> {
    let mut stats = SynthesizeCopyStats::default();
    let mut overrides_written = BTreeSet::new();
    let mut entries_seen = 0_usize;
    for entry in WalkDir::new(source_root).follow_links(false) {
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen
            > MAX_HASHED_TREE_FILES
                .saturating_mul(4)
                .saturating_add(1_024)
        {
            return Err(CliError::validation_failed(
                "source project exceeds the workflow synthesize filesystem-entry cap",
            ));
        }
        let entry = entry.map_err(|error| {
            CliError::unexpected(format!(
                "walk source project {}: {error}",
                source_root.display()
            ))
        })?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            CliError::unexpected(format!(
                "inspect source project entry {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(CliError::validation_failed(format!(
                "source project contains a link or reparse point: {}",
                entry.path().display()
            )));
        }
        let relative = entry.path().strip_prefix(source_root).map_err(|_| {
            CliError::validation_failed("source project entry escaped the project root")
        })?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let output_relative = synthesize_output_relative(relative)?;
        if metadata.is_dir() {
            output.ensure_relative_directory(&output_relative, "synthesized project directory")?;
            stats.directories_written += 1;
        } else if metadata.is_file() {
            if synthesize_file_excluded(relative) {
                stats.files_excluded += 1;
                continue;
            }
            if stats.files_written >= MAX_HASHED_TREE_FILES
                || stats.bytes_written.saturating_add(metadata.len()) > MAX_HASHED_TREE_BYTES
            {
                return Err(CliError::validation_failed(
                    "source project exceeds the workflow synthesize file or byte cap",
                ));
            }
            if let Some(bytes) = overrides.get(relative) {
                output.write_new_file(&output_relative, bytes, "synthesized project file")?;
                overrides_written.insert(relative.to_path_buf());
                stats.bytes_written = stats.bytes_written.saturating_add(bytes.len() as u64);
            } else {
                let claim = claim_for_file(entry.path(), MAX_RESOURCE_BYTES)?;
                copy_new_output_file(entry.path(), output, &output_relative, &claim)?;
                stats.bytes_written = stats.bytes_written.saturating_add(claim.bytes);
            }
            stats.files_written += 1;
        } else {
            return Err(CliError::validation_failed(format!(
                "source project contains an unsupported filesystem object: {}",
                entry.path().display()
            )));
        }
    }
    for (relative, bytes) in overrides {
        if overrides_written.contains(relative) {
            continue;
        }
        if stats.files_written >= MAX_HASHED_TREE_FILES
            || stats.bytes_written.saturating_add(bytes.len() as u64) > MAX_HASHED_TREE_BYTES
        {
            return Err(CliError::validation_failed(
                "synthetic override files exceed the workflow synthesize file or byte cap",
            ));
        }
        let output_relative = synthesize_output_relative(relative)?;
        output.write_new_file(&output_relative, bytes, "synthesized project file")?;
        stats.files_written += 1;
        stats.bytes_written = stats.bytes_written.saturating_add(bytes.len() as u64);
    }
    Ok(stats)
}

fn synthesize_output_relative(relative: &Path) -> CliResult<PathBuf> {
    let value = relative
        .to_str()
        .ok_or_else(|| {
            CliError::validation_failed("source project relative paths must be Unicode")
        })?
        .replace('\\', "/");
    validate_relative_path(&value, "synthesized project path")
}

fn synthesize_file_excluded(relative: &Path) -> bool {
    relative
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("localSettings.json")
                || name.eq_ignore_ascii_case("cache.abf")
        })
}

fn synthesize_offline_safety(
    resolved: &crate::ResolvedProject,
    server_strings: &BTreeSet<String>,
) -> CliResult<Value> {
    let docs = load_table_documents(resolved)?;
    let mut connector_matches = BTreeSet::new();
    let mut server_matches = BTreeSet::new();
    let mut partitions_scanned = 0_usize;
    for doc in docs {
        for partition in doc.partitions {
            let Some(source) = partition.source.as_deref() else {
                continue;
            };
            partitions_scanned += 1;
            let lower = source.to_ascii_lowercase();
            let tokens = m_tokens(source)?;
            let connector_token = tokens.windows(2).any(|window| {
                matches!(window, [MToken::Ident(connector), MToken::LParen] if is_database_connector(connector))
            });
            if connector_token
                || lower.contains("postgresql.database")
                || lower.contains("sql.database")
            {
                connector_matches.insert(partition.handle());
            }
            if tokens.iter().any(
                |token| matches!(token, MToken::String(value) if server_strings.contains(value)),
            ) {
                server_matches.insert(partition.handle());
            }
        }
    }
    let mut forbidden_files = BTreeSet::new();
    for entry in WalkDir::new(&resolved.project_dir).follow_links(false) {
        let entry = entry.map_err(|error| {
            CliError::unexpected(format!(
                "walk synthesized project for offline safety: {error}"
            ))
        })?;
        if entry.file_type().is_file() {
            let relative = entry
                .path()
                .strip_prefix(&resolved.project_dir)
                .map_err(|_| {
                    CliError::validation_failed("offline-safety scan escaped the output project")
                })?;
            if synthesize_file_excluded(relative) {
                forbidden_files.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let ok =
        connector_matches.is_empty() && server_matches.is_empty() && forbidden_files.is_empty();
    Ok(json!({
        "ok": ok,
        "partitionsScanned": partitions_scanned,
        "connectorTextGone": connector_matches.is_empty(),
        "serverStringsGone": server_matches.is_empty(),
        "forbiddenRuntimeFilesGone": forbidden_files.is_empty(),
        "connectorMatches": connector_matches,
        "serverStringMatches": server_matches,
        "forbiddenRuntimeFiles": forbidden_files
    }))
}

pub(crate) fn workflow_command(args: &[String]) -> CliResult<Value> {
    match args.split_first() {
        Some((command, rest)) if command == "plan" => workflow_plan(rest),
        Some((command, rest)) if command == "run" => workflow_run(rest),
        Some((command, rest)) if command == "verify" => workflow_verify(rest),
        Some((command, _)) => Err(CliError::invalid_args(format!(
            "unknown workflow command: {command}"
        ))
        .with_hint("Use workflow synthesize, workflow plan, workflow run, or workflow verify.")),
        None => Err(CliError::invalid_args(
            "workflow requires one subcommand: synthesize, plan, run, or verify",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::shared::*;
    use super::*;

    #[test]
    fn database_profile_needs_no_resource_and_registered_resources_must_be_used() {
        let mut profile = SourceProfile {
            schema: SOURCE_PROFILE_SCHEMA.into(),
            profile_id: "postgres-work".into(),
            resources: BTreeMap::new(),
            replacements: vec![ReplacementSpec {
                operation: "partition.replaceSource".into(),
                table: "FactSales".into(),
                partition: "FactSales".into(),
                expected_before_sha256:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
                template: "templates/FactSales.m".into(),
                expected_connector: "PostgreSQL.Database".into(),
                resources: Vec::new(),
            }],
        };
        assert!(validate_profile_shape(&profile).is_ok());
        profile
            .resources
            .insert(
                "unused".into(),
                ResourceSpec {
                    path: None,
                    expected_sha256:
                        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                            .into(),
                },
            );
        assert!(validate_profile_shape(&profile).is_err());
    }

    #[test]
    fn staged_resource_path_is_encoded_as_m_string_content() {
        assert_eq!(
            m_string_content("C:\\data\\book#(cr)\"copy.xlsx").expect("M content"),
            "C:\\data\\book#(0023)(cr)\"\"copy.xlsx"
        );
        assert!(m_string_content("bad\npath").is_err());
    }

    #[test]
    fn staged_resource_path_uses_power_query_compatible_windows_spelling() {
        assert_eq!(
            m_file_path_content(Path::new(r"\\?\C:\data\book.xlsx"), "resource")
                .expect("drive path"),
            r"C:\data\book.xlsx"
        );
        assert_eq!(
            m_file_path_content(Path::new(r"\\?\UNC\server\share\book.xlsx"), "resource")
                .expect("UNC path"),
            r"\\server\share\book.xlsx"
        );
        assert_eq!(
            m_file_path_content(Path::new(r"C:\data\book.xlsx"), "resource")
                .expect("ordinary path"),
            r"C:\data\book.xlsx"
        );
    }

    #[test]
    fn export_guard_accepts_only_fresh_definition_shape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        let export = workflow.join("mcp-export");
        let prepared = PreparedStagedModel::prepare(&source, &stage, &workflow, &export)
            .expect("prepared paths")
            .commit();
        copy_definition(&stage.join("definition"), &export.join("definition"));
        let proof = prepared.validate_export().expect("valid export");
        assert_eq!(proof.file_count, 3);

        fs::remove_dir_all(export.join("definition")).expect("remove definition");
        fs::write(export.join("database.tmdl"), "database Unsafe").expect("root-level tmdl");
        assert!(prepared.validate_export().is_err());
    }

    #[test]
    fn protected_or_existing_export_targets_are_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        fs::create_dir(workflow.join("occupied")).expect("occupied");
        fs::write(workflow.join("occupied").join("keep.txt"), "keep").expect("occupied file");
        assert!(
            PreparedStagedModel::prepare(&source, &stage, &workflow, &workflow.join("occupied"))
                .is_err()
        );
        assert!(
            PreparedStagedModel::prepare(&source, &stage, &workflow, &stage.join("definition"))
                .is_err()
        );
    }

    #[test]
    fn failed_export_reservation_cleans_only_owned_state_and_retry_succeeds() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        let export = workflow.join("mcp-export");
        fs::create_dir(&export).expect("preexisting empty export");
        assert!(PreparedStagedModel::prepare(&source, &stage, &workflow, &export).is_err());
        assert!(export.is_dir(), "preexisting caller directory was removed");
        assert!(!workflow.join(".mcp-export.powerbi-cli-quarantine").exists());
        fs::remove_dir(&export).expect("remove caller directory");
        let prepared = PreparedStagedModel::prepare(&source, &stage, &workflow, &export)
            .expect("retry after failed reservation")
            .commit();
        assert!(prepared.export_root.join("definition").is_dir());
    }

    #[test]
    fn export_guard_never_deletes_a_replacement_at_the_reserved_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        let export = workflow.join("mcp-export");
        let reservation = PreparedStagedModel::prepare(&source, &stage, &workflow, &export)
            .expect("prepared paths");
        let moved_owned = workflow.join("owned-moved-away");
        fs::rename(&export, &moved_owned).expect("move the originally owned directory");
        fs::create_dir(&export).expect("replacement export directory");
        fs::write(export.join("keep.txt"), "foreign replacement").expect("replacement content");

        drop(reservation);

        assert_eq!(
            fs::read_to_string(export.join("keep.txt")).expect("replacement survives"),
            "foreign replacement"
        );
        assert!(moved_owned.join("definition").is_dir());
        assert!(!workflow.join(".mcp-export.powerbi-cli-cleanup").exists());
        assert!(!workflow.join(".mcp-export.powerbi-cli-quarantine").exists());
    }

    #[test]
    fn source_snapshot_proves_byte_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        copy_fixture(&source);
        let snapshot = SourceTreeSnapshot::capture(&source).expect("snapshot");
        let unchanged = snapshot.verify().expect("verify unchanged");
        assert!(unchanged.byte_identical);
        fs::write(source.join("definition.pbism"), "changed").expect("change source");
        let changed = snapshot.verify().expect("verify changed");
        assert!(!changed.byte_identical);
    }

    #[test]
    fn source_profile_plan_is_deterministic_and_selects_only_the_pbip_closure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        fs::create_dir(fixture.project.join("Sibling.Report")).expect("sibling");
        fs::write(
            fixture
                .project
                .join("Sibling.Report")
                .join("do-not-copy.json"),
            "{}",
        )
        .expect("sibling file");
        fs::create_dir(fixture.project.join(".git")).expect("git dir");
        fs::write(fixture.project.join(".git").join("config"), "private").expect("git config");
        fs::create_dir(fixture.project.join("data")).expect("data dir");
        fs::write(
            fixture.project.join("data").join("unregistered.xlsx"),
            "private",
        )
        .expect("unregistered data");
        let source_before = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project root"),
        )
        .expect("before manifest");

        let first = temp.path().join("first.plan.json");
        let second = temp.path().join("second.plan.json");
        let output = temp.path().join("output");
        let a = plan_fixture(&fixture, &first, &output).expect("first plan");
        let b = plan_fixture(&fixture, &second, &output).expect("second plan");
        assert_eq!(a["planFingerprint"], b["planFingerprint"]);
        assert_eq!(
            fs::read(&first).expect("first"),
            fs::read(&second).expect("second")
        );
        let plan = load_plan(&first).expect("load plan");
        assert!(plan.source.files.iter().all(|file| {
            !file.path.starts_with("Sibling.Report/")
                && !file.path.starts_with(".git/")
                && !file.path.starts_with("data/")
        }));
        let source_after = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project root"),
        )
        .expect("after manifest");
        assert_eq!(source_before.closure_sha256, source_after.closure_sha256);
    }

    #[test]
    fn plan_rejects_drift_overwrite_path_escape_and_credentials() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        assert!(plan_fixture(&fixture, &plan_path, &output).is_err());
        fs::write(&fixture.template, "let Password = \"secret\" in Password")
            .expect("credential template");
        let plan = load_plan(&plan_path).expect("load fingerprinted plan");
        assert!(
            verify_plan_inputs(&plan).is_err(),
            "template drift must fail"
        );

        let unsafe_profile = temp.path().join("unsafe-profile.json");
        let mut value: Value =
            serde_json::from_slice(&fs::read(&fixture.profile).expect("profile"))
                .expect("profile JSON");
        value["resources"]["workbook"]["path"] = Value::String("../outside.xlsx".into());
        fs::write(
            &unsafe_profile,
            serde_json::to_vec_pretty(&value).expect("unsafe JSON"),
        )
        .expect("unsafe profile");
        let args = plan_args(
            &fixture.pbip,
            &unsafe_profile,
            &temp.path().join("unsafe.plan.json"),
            &temp.path().join("unsafe-output"),
        );
        assert!(
            workflow_plan(&args).is_err(),
            "profile path escape must fail"
        );

        fs::create_dir(&output).expect("occupied output");
        let another = temp.path().join("another.plan.json");
        assert!(plan_fixture(&fixture, &another, &output).is_err());
    }

    #[test]
    fn credential_like_override_path_is_rejected_before_plan_or_output_persistence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let override_path = temp.path().join("password=secret.xlsx");
        fs::write(&override_path, "neutral bytes").expect("override resource");
        let plan_path = temp.path().join("credential-path.plan.json");
        let output = temp.path().join("credential-path-output");
        let mut args = plan_args(&fixture.pbip, &fixture.profile, &plan_path, &output);
        args.extend([
            "--resource".into(),
            format!("workbook={}", override_path.display()),
        ]);

        assert!(workflow_plan(&args).is_err());
        assert!(!plan_path.exists(), "credential-like path reached the plan");
        assert!(!output.exists(), "credential-like path reached the output");
    }

    #[test]
    fn plan_rejects_unsafe_files_inside_selected_artifacts() {
        for relative in [
            Path::new("Synthetic.Report/.pbi/cache.abf"),
            Path::new("Synthetic.Report/localSettings.json"),
            Path::new("Synthetic.Report/definition/pages/private/data.csv"),
            Path::new("Synthetic.Report/definition/pages/private/data.json"),
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let fixture = workflow_fixture(temp.path());
            let unsafe_path = fixture.project.join(relative);
            fs::create_dir_all(unsafe_path.parent().expect("unsafe parent")).expect("unsafe dirs");
            fs::write(&unsafe_path, "private").expect("unsafe file");
            assert!(
                plan_fixture(
                    &fixture,
                    &temp.path().join("unsafe.plan.json"),
                    &temp.path().join("unsafe-output")
                )
                .is_err(),
                "unsafe selected artifact file was accepted: {}",
                relative.display()
            );
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let table = fixture
            .project
            .join("Synthetic.SemanticModel/definition/tables/Synthetic.tmdl");
        let mut text = fs::read_to_string(&table).expect("table");
        text.push_str("\n\tannotation password = \"secret\"\n");
        fs::write(&table, text).expect("credential-bearing TMDL");
        assert!(
            plan_fixture(
                &fixture,
                &temp.path().join("credential.plan.json"),
                &temp.path().join("credential-output")
            )
            .is_err()
        );

        for (name, bytes) in [
            (
                "credential.svg",
                b"<svg><!-- password=secret --></svg>".as_slice(),
            ),
            ("invalid.svg", &[0xff, 0xfe][..]),
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let fixture = workflow_fixture(temp.path());
            let svg = fixture
                .project
                .join("Synthetic.Report/StaticResources/RegisteredResources")
                .join(name);
            fs::create_dir_all(svg.parent().expect("SVG parent")).expect("SVG directory");
            fs::write(&svg, bytes).expect("unsafe SVG");
            assert!(
                plan_fixture(
                    &fixture,
                    &temp.path().join("unsafe-svg.plan.json"),
                    &temp.path().join("unsafe-svg-output"),
                )
                .is_err(),
                "unsafe SVG text was accepted: {name}"
            );
        }
    }

    #[test]
    fn plan_and_recomputed_fingerprint_cannot_write_inside_source_project() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let inside_plan = fixture.project.join("workflow.plan.json");
        assert!(plan_fixture(&fixture, &inside_plan, &temp.path().join("outside-output")).is_err());
        assert!(!inside_plan.exists());
        assert!(
            plan_fixture(
                &fixture,
                &temp.path().join("inside-output.plan.json"),
                &fixture.project.join("generated-output")
            )
            .is_err()
        );
        assert!(!fixture.project.join("generated-output").exists());

        let plan_path = temp.path().join("normal.plan.json");
        plan_fixture(&fixture, &plan_path, &temp.path().join("normal-output"))
            .expect("normal plan");
        let mut recomputed: WorkflowPlan =
            read_json_bounded(&plan_path, MAX_PROFILE_BYTES, "workflow plan").expect("plan JSON");
        recomputed.output_dir = canonical_display(&fixture.project.join("recomputed-output"));
        recomputed.plan_fingerprint =
            plan_fingerprint(&recomputed).expect("recomputed fingerprint");
        fs::write(
            &plan_path,
            serde_json::to_vec_pretty(&recomputed).expect("recomputed JSON"),
        )
        .expect("recomputed plan");
        assert!(load_plan(&plan_path).is_err());
        assert!(
            workflow_run(&[
                "--plan".into(),
                plan_path.to_string_lossy().into_owned(),
                "--confirm".into(),
                recomputed.plan_fingerprint,
            ])
            .is_err()
        );
        assert!(!fixture.project.join("recomputed-output").exists());
    }

    #[test]
    fn resealed_plan_cannot_widen_profile_derived_semantics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        plan_fixture(&fixture, &plan_path, &temp.path().join("output")).expect("plan");
        let original = load_plan(&plan_path).expect("load plan");

        let mut cases = Vec::new();
        let mut connector = original.clone();
        connector.replacements[0].expected_connector = "PostgreSQL.Database".into();
        connector.replacements[0].resources.clear();
        cases.push(connector);

        let mut resource = original.clone();
        resource
            .resources
            .get_mut("workbook")
            .expect("resource")
            .output_relative = "resources/workbook/renamed.xlsx".into();
        cases.push(resource);

        let mut template = original.clone();
        template.replacements[0].template = "templates/Other.m".into();
        cases.push(template);

        for mut resealed in cases {
            resealed.plan_fingerprint = plan_fingerprint(&resealed).expect("reseal");
            assert!(
                validate_profile_derived_plan(
                    &resealed,
                    &read_json_bounded(
                        Path::new(&resealed.profile.path),
                        MAX_PROFILE_BYTES,
                        "profile",
                    )
                    .expect("profile"),
                )
                .is_err(),
                "recomputed self-hash widened profile-derived semantics"
            );
        }
    }

    #[test]
    fn connector_identity_ignores_comments_and_strings_and_rejects_other_connectors() {
        assert!(
            validate_expected_connector_call(
                &m_tokens("let Note = \"Excel.Workbook(\", Source = Web.Contents(\"https://invalid\") in Source").expect("tokens"),
                "Excel.Workbook"
            )
            .is_err()
        );
        assert!(
            validate_expected_connector_call(
                &m_tokens(
                    "let /* Excel.Workbook( */ Source = Web.Contents(\"https://invalid\") in Source"
                )
                .expect("tokens"),
                "Excel.Workbook"
            )
            .is_err()
        );
        assert!(
            validate_expected_connector_call(
                &m_tokens("let Good = Excel.Workbook(File.Contents(\"book.xlsx\"), null, true), Bad = Web.Contents(\"https://invalid\") in Good").expect("tokens"),
                "Excel.Workbook"
            )
            .is_err()
        );
        assert!(
            validate_expected_connector_call(
                &m_tokens("let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true) in Source").expect("tokens"),
                "Excel.Workbook"
            )
            .is_ok()
        );

        let replacement = ReplacementSpec {
            operation: "partition.replaceSource".into(),
            table: "Fact".into(),
            partition: "Fact".into(),
            expected_before_sha256:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            template: "templates/Fact.m".into(),
            expected_connector: "Excel.Workbook".into(),
            resources: vec!["workbook".into()],
        };
        let good = "let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), Typed = Table.TransformColumnTypes(Source, {}) in Typed";
        assert!(validate_template(good, &replacement).is_ok());
        assert_eq!(
            m_semantic_sha256(good).expect("semantic M"),
            m_semantic_sha256("let\n Source=Excel.Workbook( File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"),null,true),/* vendor formatting */Typed=Table.TransformColumnTypes(Source,{})\nin Typed").expect("reformatted semantic M")
        );
        for unsafe_m in [
            "let Source = Excel.Workbook(File.Contents(\"C:\\\\private\\\\book.xlsx\"), null, true) in Source",
            "let Source = Excel.Workbook(File.Contents(\"https://invalid/book.xlsx\"), null, true) in Source",
            "let Connector = Excel.Workbook, Source = Connector(\"{{powerbi-cli.resourcePath:workbook}}\") in Source",
            "let Root = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), Source = Root in Source",
            "let Source = Excel.Workbook(File.Contents(\"book-{{powerbi-cli.resourcePath:workbook}}\"), null, true) in Source",
            "let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), Leak = Mystery.Cloud(\"x\") in Source",
            "let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), F = Web.Contents, Leak = Value.Invoke(F, {\"x\"}) in Source",
        ] {
            assert!(
                validate_template(unsafe_m, &replacement).is_err(),
                "unsafe M template was accepted: {unsafe_m}"
            );
        }

        let postgres = ReplacementSpec {
            expected_connector: "PostgreSQL.Database".into(),
            resources: Vec::new(),
            ..replacement.clone()
        };
        assert!(
            validate_template(
                "let Source = PostgreSQL.Database(\"db.internal:5432\", \"analytics\"), Rows = Table.SelectRows(Source, each true) in Rows",
                &postgres,
            )
            .is_ok()
        );
        assert!(
            validate_template(
                "let Source = PostgreSQL.Database(\"db.internal:5432\", \"analytics\"), Extra = ([Run = PostgreSQL.Database][Run])(\"other.internal:5432\", \"other\") in Source",
                &postgres,
            )
            .is_err(),
            "computed connector invocation bypassed the closed M grammar"
        );
        let postgres_with_file = ReplacementSpec {
            resources: vec!["workbook".into()],
            ..postgres
        };
        assert!(
            validate_template(
                "let Source = PostgreSQL.Database(\"db.internal:5432\", \"analytics\") in Source",
                &postgres_with_file,
            )
            .is_err()
        );
    }

    #[test]
    fn complete_transformed_m_is_materialized_without_template_payload_in_plan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        let plan = load_plan(&plan_path).expect("load plan");
        let serialized = fs::read_to_string(&plan_path).expect("plan text");
        assert!(!serialized.contains("Table.TransformColumnTypes"));
        let owned_output = OwnedWorkflowOutput::create(&output).expect("output");
        copy_resources(&plan, &owned_output).expect("copy resources");
        let replacements = materialize_replacements(&plan, &output).expect("materialize M");
        let expression = &replacements[0].complete_m_expression;
        assert!(expression.contains("Excel.Workbook"));
        assert!(expression.contains("Navigation"));
        assert!(expression.contains("Table.TransformColumnTypes"));
        assert!(!expression.contains("{{powerbi-cli."));
        assert!(expression.contains("resources"));
        #[cfg(windows)]
        {
            assert!(expression.contains("File.Contents(\""));
            assert!(!expression.contains(r"\\?\"));
        }
        assert!(
            template_placeholders(
                "let Source = File.Contents({{powerbi-cli.resourcePath:workbook}}) in Source",
                &m_tokens(
                    "let Source = File.Contents({{powerbi-cli.resourcePath:workbook}}) in Source"
                )
                .expect("tokens")
            )
            .is_err()
        );
    }

    #[test]
    fn recomputed_receipt_checksum_cannot_bypass_semantics_and_copy_failure_preserves_source() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        let plan = load_plan(&plan_path).expect("load plan");
        let source_before = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project"),
        )
        .expect("before");
        let owned_output = OwnedWorkflowOutput::create(&output).expect("output");
        owned_output
            .write_new_file(Path::new("occupied"), b"keep", "occupied test file")
            .expect("occupied");
        let claim = claim_for_file(&fixture.resource, MAX_RESOURCE_BYTES).expect("claim");
        assert!(
            copy_new_output_file(
                &fixture.resource,
                &owned_output,
                Path::new("occupied"),
                &claim
            )
            .is_err()
        );
        let source_after = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project"),
        )
        .expect("after");
        assert_eq!(source_before.closure_sha256, source_after.closure_sha256);

        let mut tampered = WorkflowReceipt {
            schema: WORKFLOW_RECEIPT_SCHEMA.into(),
            receipt_checksum: String::new(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
            output_tree_sha256:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            source_closure_sha256: plan.source.closure_sha256.clone(),
            model: valid_model_receipt(&plan),
            validation: ValidationClaim {
                native_version: env!("CARGO_PKG_VERSION").into(),
                native_errors: 0,
                native_warnings: 0,
                official_errors: 0,
                official_warnings: 0,
                official_version: "0.1.4".into(),
            },
        };
        assert!(validate_receipt_semantics(&plan, &tampered).is_ok());
        tampered.model.children_reaped = false;
        tampered.receipt_checksum = receipt_checksum(&tampered).expect("recomputed checksum");
        assert!(validate_receipt_semantics(&plan, &tampered).is_err());
        fs::write(
            output.join(WORKFLOW_RECEIPT_FILE),
            serde_json::to_vec_pretty(&tampered).expect("receipt JSON"),
        )
        .expect("receipt");
        assert!(
            workflow_verify(&["--plan".into(), plan_path.to_string_lossy().into_owned()]).is_err()
        );
    }

    #[test]
    fn workflow_output_identity_swap_keeps_copy_bound_to_opened_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, "planned bytes").expect("source");
        let claim = claim_for_file(&source, MAX_RESOURCE_BYTES).expect("source claim");
        let output_path = temp.path().join("output");
        let output = OwnedWorkflowOutput::create(&output_path).expect("owned output");
        let displaced = temp.path().join("displaced-output");
        if let Err(error) = fs::rename(&output_path, &displaced) {
            #[cfg(windows)]
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(32)
            {
                assert!(output_path.is_dir(), "opened root remained at its path");
                return;
            }
            panic!("displace owned output: {error}");
        }
        fs::create_dir(&output_path).expect("replacement output");

        copy_new_output_file(&source, &output, Path::new("copied/source.bin"), &claim)
            .expect("copy remains bound to opened output root");
        assert!(!output_path.join("copied/source.bin").exists());
        assert_eq!(
            fs::read(displaced.join("copied/source.bin")).expect("capability destination"),
            b"planned bytes"
        );
        assert!(
            output.verify_root().is_err(),
            "publication identity changed"
        );
    }

    #[test]
    fn workflow_output_capability_cannot_be_redirected_by_root_alias_swap() {
        use std::cell::Cell;

        let temp = tempfile::tempdir().expect("tempdir");
        let output_path = temp.path().join("output");
        let displaced = temp.path().join("opened-output");
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).expect("outside");
        let output = OwnedWorkflowOutput::create(&output_path).expect("owned output");
        let renamed = Cell::new(false);
        let aliased = Cell::new(false);

        let mut file = output
            .create_new_file_after(Path::new("proof.bin"), "capability race proof", || {
                match fs::rename(&output_path, &displaced) {
                    Ok(()) => renamed.set(true),
                    Err(error) => {
                        #[cfg(windows)]
                        if error.kind() == std::io::ErrorKind::PermissionDenied
                            || error.raw_os_error() == Some(32)
                        {
                            return;
                        }
                        panic!("rename opened output root at capability boundary: {error}");
                    }
                }

                #[cfg(unix)]
                let alias_result = std::os::unix::fs::symlink(&outside, &output_path);
                #[cfg(windows)]
                let alias_result = std::os::windows::fs::symlink_dir(&outside, &output_path);
                match alias_result {
                    Ok(()) => aliased.set(true),
                    Err(error) => {
                        #[cfg(windows)]
                        if error.kind() == std::io::ErrorKind::PermissionDenied
                            || error.raw_os_error() == Some(1314)
                        {
                            return;
                        }
                        panic!("install outside directory alias: {error}");
                    }
                }
            })
            .expect("capability-relative create");
        file.write_all(b"capability bytes")
            .expect("capability write");
        file.sync_all().expect("capability sync");
        drop(file);

        #[cfg(unix)]
        {
            assert!(
                renamed.get(),
                "opened root was renamed at the write boundary"
            );
            assert!(aliased.get(), "outside symlink replaced the ambient path");
        }
        let landed = if renamed.get() {
            displaced.join("proof.bin")
        } else {
            output_path.join("proof.bin")
        };
        assert_eq!(
            fs::read(&landed).expect("capability-owned file"),
            b"capability bytes"
        );
        assert!(
            !outside.join("proof.bin").exists(),
            "outside alias received a workflow write"
        );
        if aliased.get() {
            assert!(
                !output_path.join("proof.bin").exists(),
                "ambient replacement path received a workflow write"
            );
        }
        if renamed.get() {
            assert!(
                output.verify_root().is_err(),
                "publication identity changed"
            );
        }
    }

    #[test]
    fn reconstructed_stage_and_copy_evidence_reject_resealed_artifact_swaps() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        let plan = load_plan(&plan_path).expect("load plan");
        let owned_output = OwnedWorkflowOutput::create(&output).expect("output");
        copy_claimed_files(
            Path::new(&plan.source.project_root),
            &owned_output,
            &plan.source.files,
        )
        .expect("copy closure");
        copy_resources(&plan, &owned_output).expect("copy resources");
        let staged = resolve_project(&output.join(&plan.source.pbip_relative)).expect("stage");
        let materialized = materialize_replacements(&plan, &output).expect("materialized");
        let docs = load_table_documents_from_semantic_model(&staged.semantic_model_dir)
            .expect("stage docs");
        let replacement = &materialized[0];
        let mutation = replace_partition_source_plan(
            &docs,
            &PartitionSelector {
                table: Some(replacement.table.clone()),
                name: Some(replacement.partition.clone()),
                ..PartitionSelector::default()
            },
            &replacement.complete_m_expression,
        )
        .expect("stage mutation");
        fs::write(&mutation.path, &mutation.new_text).expect("materialize stage");

        let expected = expected_stage(&plan, &output).expect("expected stage");
        let actual = validate_tmdl_definition(&staged.semantic_model_dir.join("definition"))
            .expect("actual stage");
        assert_eq!(actual.sha256, expected.after_sha256);
        verify_staged_copies(&plan, &output, &expected.modified_source_files)
            .expect("exact staged copies");

        let report_file = output.join("Synthetic.Report/definition/report.json");
        let report_before = fs::read(&report_file).expect("report before");
        fs::write(&report_file, "{\"swapped\":true}").expect("swap report");
        assert!(verify_staged_copies(&plan, &output, &expected.modified_source_files).is_err());
        fs::write(&report_file, report_before).expect("restore report");

        let model_file = staged.semantic_model_dir.join("definition/model.tmdl");
        let model_before = fs::read(&model_file).expect("model before");
        fs::write(&model_file, "model Swapped\n").expect("swap unrelated TMDL");
        assert_ne!(
            validate_tmdl_definition(&staged.semantic_model_dir.join("definition"))
                .expect("tampered definition")
                .sha256,
            expected.after_sha256
        );
        fs::write(&model_file, model_before).expect("restore model");

        let evidence_root = output.join(WORKFLOW_EVIDENCE_DIR);
        copy_definition(
            &staged.semantic_model_dir.join("definition"),
            &evidence_root.join("definition"),
        );
        let requested = expected
            .requested_sha256
            .get(&(replacement.table.clone(), replacement.partition.clone()))
            .expect("request hash");
        assert_eq!(
            &staged_partition_source_fingerprint(
                &evidence_root,
                &replacement.table,
                &replacement.partition,
            )
            .expect("evidence fingerprint"),
            requested
        );
        let canonical_evidence = validate_evidence_claim(
            &output,
            &EvidenceClaim {
                path: WORKFLOW_EVIDENCE_DIR.into(),
                definition_sha256: String::new(),
                file_count: 0,
                total_bytes: 0,
            },
        )
        .expect("canonical evidence");
        let injected = evidence_root.join("definition/tables/Injected.tmdl");
        fs::write(
            &injected,
            "table Injected\n\n\tpartition Injected = m\n\t\tmode: import\n\t\tsource =\n\t\t\tlet Source = 1 in Source\n",
        )
        .expect("inject unrelated evidence table");
        let tampered_evidence = validate_evidence_claim(
            &output,
            &EvidenceClaim {
                path: WORKFLOW_EVIDENCE_DIR.into(),
                definition_sha256: String::new(),
                file_count: 0,
                total_bytes: 0,
            },
        )
        .expect("shape-valid injected evidence");
        let mut resealed = WorkflowReceipt {
            schema: WORKFLOW_RECEIPT_SCHEMA.into(),
            receipt_checksum: String::new(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
            output_tree_sha256: hash_workflow_output(&output)
                .expect("tampered output hash")
                .sha256,
            source_closure_sha256: plan.source.closure_sha256.clone(),
            model: valid_model_receipt(&plan),
            validation: ValidationClaim {
                native_version: env!("CARGO_PKG_VERSION").into(),
                native_errors: 0,
                native_warnings: 0,
                official_errors: 0,
                official_warnings: 0,
                official_version: "0.1.4".into(),
            },
        };
        resealed.model.evidence = EvidenceClaim {
            path: WORKFLOW_EVIDENCE_DIR.into(),
            definition_sha256: tampered_evidence.definition_sha256.clone(),
            file_count: tampered_evidence.file_count,
            total_bytes: tampered_evidence.total_bytes,
        };
        resealed.receipt_checksum = receipt_checksum(&resealed).expect("resealed receipt");
        assert_eq!(
            resealed.receipt_checksum,
            receipt_checksum(&resealed).unwrap()
        );
        assert!(
            validate_canonical_export_binding(&tampered_evidence, &canonical_evidence).is_err(),
            "recomputed evidence and receipt claims bypassed canonical stage binding"
        );
        fs::remove_file(&injected).expect("remove injected table");

        let evidence_model = evidence_root.join("definition/model.tmdl");
        let evidence_model_before = fs::read(&evidence_model).expect("evidence model before");
        fs::OpenOptions::new()
            .append(true)
            .open(&evidence_model)
            .and_then(|mut file| file.write_all(b"\n// password=secret\n"))
            .expect("credential comment");
        assert!(
            validate_evidence_claim(
                &output,
                &EvidenceClaim {
                    path: WORKFLOW_EVIDENCE_DIR.into(),
                    definition_sha256: String::new(),
                    file_count: 0,
                    total_bytes: 0,
                },
            )
            .is_err(),
            "credential-bearing evidence comment was accepted"
        );
        fs::write(&evidence_model, evidence_model_before).expect("restore evidence model");

        let evidence_docs =
            load_table_documents_from_semantic_model(&evidence_root).expect("evidence docs");
        let swapped = replace_partition_source_plan(
            &evidence_docs,
            &PartitionSelector {
                table: Some(replacement.table.clone()),
                name: Some(replacement.partition.clone()),
                ..PartitionSelector::default()
            },
            "let Source = 1 in Source",
        )
        .expect("swap evidence");
        fs::write(swapped.path, swapped.new_text).expect("write swapped evidence");
        assert_ne!(
            &staged_partition_source_fingerprint(
                &evidence_root,
                &replacement.table,
                &replacement.partition,
            )
            .expect("swapped evidence fingerprint"),
            requested
        );
    }

    #[test]
    fn tree_hash_is_bounded_and_rejects_links() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("one"), "1").expect("one");
        fs::write(temp.path().join("two"), "22").expect("two");
        assert!(hash_tree_inner_bounded(temp.path(), &BTreeMap::new(), &[], 1, 100).is_err());
        assert!(hash_tree_inner_bounded(temp.path(), &BTreeMap::new(), &[], 10, 1).is_err());
        assert!(hash_tree_inner_bounded(temp.path(), &BTreeMap::new(), &[], 10, 100).is_ok());

        let oversized = tempfile::tempdir().expect("oversized tempdir");
        File::create(oversized.path().join("large"))
            .and_then(|file| file.set_len(1024 * 1024))
            .expect("sparse oversized file");
        let mut opens = 0_usize;
        let result = hash_tree_inner_bounded_with_opener(
            oversized.path(),
            &BTreeMap::new(),
            &[],
            10,
            1,
            |path| {
                opens += 1;
                File::open(path).map_err(|error| error.to_string())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            opens, 0,
            "oversized file was opened before byte-cap rejection"
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temp.path().join("one"), temp.path().join("link"))
                .expect("symlink");
            assert!(hash_tree(temp.path()).is_err());
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(temp.path().join("one"), temp.path().join("link"))
                .is_ok()
            {
                assert!(hash_tree(temp.path()).is_err());
            }
        }

        let dangling_root = tempfile::tempdir().expect("dangling tempdir");
        let marker = dangling_root.path().join(WORKFLOW_INCOMPLETE_FILE);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dangling_root.path().join("missing"), &marker)
                .expect("dangling marker");
            assert!(
                hash_tree_with_exclusions(
                    dangling_root.path(),
                    &[Path::new(WORKFLOW_INCOMPLETE_FILE)]
                )
                .is_err(),
                "excluded dangling marker bypassed link inspection"
            );
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(dangling_root.path().join("missing"), &marker)
                .is_ok()
            {
                assert!(
                    hash_tree_with_exclusions(
                        dangling_root.path(),
                        &[Path::new(WORKFLOW_INCOMPLETE_FILE)]
                    )
                    .is_err(),
                    "excluded dangling marker bypassed reparse inspection"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires exact installed Microsoft modeling MCP and report validator sidecars"]
    fn workflow_plan_run_verify_with_exact_installed_sidecars() {
        let temp = tempfile::tempdir().expect("tempdir");
        let schema_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/sales.schema.json");
        let schema: Value =
            serde_json::from_slice(&fs::read(&schema_path).expect("schema")).expect("schema JSON");
        let project = temp.path().join("exact-source");
        crate::scaffold_schema_value(schema, &schema_path, &project, false)
            .expect("scaffold exact source fixture");
        let resolved = resolve_project(&project).expect("resolve scaffold");
        let expected =
            staged_partition_source_fingerprint(&resolved.semantic_model_dir, "DimDate", "DimDate")
                .expect("source fingerprint");
        let profile_dir = temp.path().join("exact-profile");
        fs::create_dir_all(profile_dir.join("templates")).expect("templates");
        fs::create_dir_all(profile_dir.join("data")).expect("data");
        let resource = profile_dir.join("data/synthetic.xlsx");
        fs::write(&resource, "neutral bytes").expect("resource");
        let resource_sha256 = sha256_file(&resource).expect("resource hash");
        fs::write(
            profile_dir.join("templates/DimDate.m"),
            "let\n    Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true),\n    Navigation = Source{[Item=\"DimDate\",Kind=\"Table\"]}[Data],\n    Typed = Table.TransformColumnTypes(Navigation, {{\"DateKey\", Int64.Type}})\nin\n    Typed\n",
        )
        .expect("template");
        let profile = profile_dir.join("source-profile.json");
        fs::write(
            &profile,
            serde_json::to_vec_pretty(&json!({
                "schema": SOURCE_PROFILE_SCHEMA,
                "profileId": "exact-neutral",
                "resources": {"workbook": {
                    "path": "data/synthetic.xlsx",
                    "expectedSha256": resource_sha256
                }},
                "replacements": [{
                    "operation": "partition.replaceSource",
                    "table": "DimDate",
                    "partition": "DimDate",
                    "expectedBeforeSha256": expected,
                    "template": "templates/DimDate.m",
                    "expectedConnector": "Excel.Workbook",
                    "resources": ["workbook"]
                }]
            }))
            .expect("profile JSON"),
        )
        .expect("profile");
        let plan_path = temp.path().join("exact.plan.json");
        let output = temp.path().join("exact-output");
        let planned = workflow_plan(&plan_args(&project, &profile, &plan_path, &output))
            .expect("exact workflow plan");
        workflow_run(&[
            "--plan".into(),
            plan_path.to_string_lossy().into_owned(),
            "--confirm".into(),
            planned["planFingerprint"]
                .as_str()
                .expect("fingerprint")
                .into(),
        ])
        .expect("exact workflow run");
        let receipt_text =
            fs::read_to_string(output.join(WORKFLOW_RECEIPT_FILE)).expect("exact workflow receipt");
        assert!(!receipt_text.contains("Excel.Workbook"));
        assert!(!receipt_text.contains("Table.TransformColumnTypes"));
        assert!(!receipt_text.contains("synthetic.xlsx"));
        let receipt: WorkflowReceipt =
            serde_json::from_str(&receipt_text).expect("exact receipt JSON");
        assert!(receipt.model.children_reaped && receipt.model.pumps_joined);
        assert_eq!(
            receipt.model.source_before_sha256,
            receipt.model.source_after_sha256
        );
        assert_eq!(
            receipt.model.stage_after_sha256,
            receipt.model.expected_stage_sha256
        );
        let output_before_verify = hash_tree(&output).expect("output before verify").sha256;
        workflow_verify(&["--plan".into(), plan_path.to_string_lossy().into_owned()])
            .expect("exact workflow verify");
        let output_after_verify = hash_tree(&output).expect("output after verify").sha256;
        assert_eq!(
            output_before_verify, output_after_verify,
            "verify mutated output"
        );
    }

    struct WorkflowFixture {
        project: PathBuf,
        pbip: PathBuf,
        profile: PathBuf,
        template: PathBuf,
        resource: PathBuf,
    }

    fn workflow_fixture(root: &Path) -> WorkflowFixture {
        let project = root.join("Project");
        let report = project.join("Synthetic.Report");
        let model = project.join("Synthetic.SemanticModel");
        fs::create_dir_all(&report).expect("report");
        let fixture_model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/conformance/microsoft/modeling-mcp/Synthetic.SemanticModel");
        copy_tree_test(&fixture_model, &model);
        let pbip = project.join("Synthetic.pbip");
        fs::write(
            &pbip,
            r#"{"version":"1.0","artifacts":[{"report":{"path":"Synthetic.Report"}}]}"#,
        )
        .expect("pbip");
        fs::write(
            report.join("definition.pbir"),
            r#"{"version":"4.0","datasetReference":{"byPath":{"path":"../Synthetic.SemanticModel"}}}"#,
        )
        .expect("pbir");
        fs::create_dir_all(report.join("definition")).expect("report definition");
        fs::write(report.join("definition/report.json"), "{}\n").expect("report file");
        let profile_dir = root.join("profile");
        fs::create_dir_all(profile_dir.join("templates")).expect("templates");
        fs::create_dir_all(profile_dir.join("data")).expect("data");
        let template = profile_dir.join("templates/Synthetic.m");
        fs::write(
            &template,
            "let\n    Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true),\n    Navigation = Source{[Item=\"Sheet1\",Kind=\"Sheet\"]}[Data],\n    Typed = Table.TransformColumnTypes(Navigation, {{\"Value\", Int64.Type}})\nin\n    Typed\n",
        )
        .expect("template");
        let resource = profile_dir.join("data/synthetic.xlsx");
        fs::write(&resource, "neutral synthetic workbook bytes").expect("resource");
        let resource_sha256 = sha256_file(&resource).expect("resource hash");
        let expected = staged_partition_source_fingerprint(&model, "Synthetic", "Synthetic")
            .expect("source fingerprint");
        let profile = profile_dir.join("source-profile.json");
        let value = json!({
            "schema": SOURCE_PROFILE_SCHEMA,
            "profileId": "neutral-synthetic",
            "resources": {"workbook": {
                "path": "data/synthetic.xlsx",
                "expectedSha256": resource_sha256
            }},
            "replacements": [{
                "operation": "partition.replaceSource",
                "table": "Synthetic",
                "partition": "Synthetic",
                "expectedBeforeSha256": expected,
                "template": "templates/Synthetic.m",
                "expectedConnector": "Excel.Workbook",
                "resources": ["workbook"]
            }]
        });
        fs::write(
            &profile,
            serde_json::to_vec_pretty(&value).expect("profile JSON"),
        )
        .expect("profile");
        WorkflowFixture {
            project,
            pbip,
            profile,
            template,
            resource,
        }
    }

    fn plan_args(project: &Path, profile: &Path, plan: &Path, output: &Path) -> Vec<String> {
        vec![
            "--project".into(),
            project.to_string_lossy().into_owned(),
            "--profile".into(),
            profile.to_string_lossy().into_owned(),
            "--out".into(),
            plan.to_string_lossy().into_owned(),
            "--out-dir".into(),
            output.to_string_lossy().into_owned(),
        ]
    }

    fn plan_fixture(fixture: &WorkflowFixture, plan: &Path, output: &Path) -> CliResult<Value> {
        workflow_plan(&plan_args(&fixture.pbip, &fixture.profile, plan, output))
    }

    fn copy_tree_test(source: &Path, target: &Path) {
        for entry in WalkDir::new(source) {
            let entry = entry.expect("fixture entry");
            let relative = entry.path().strip_prefix(source).expect("relative");
            let output = target.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir_all(output).expect("fixture directory");
            } else {
                fs::copy(entry.path(), output).expect("fixture file");
            }
        }
    }

    fn valid_model_receipt(plan: &WorkflowPlan) -> ModelReceipt {
        let hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        ModelReceipt {
            component: "modeling-mcp".into(),
            package_version: "0.5.0-beta.11".into(),
            server_version: "0.5.0.0".into(),
            local_process: true,
            transport: "stdio".into(),
            children_reaped: true,
            pumps_joined: true,
            forced_cleanup: false,
            source_before_sha256: hash.into(),
            source_after_sha256: hash.into(),
            stage_before_sha256: hash.into(),
            stage_after_sha256: hash.into(),
            expected_stage_sha256: hash.into(),
            evidence: EvidenceClaim {
                path: WORKFLOW_EVIDENCE_DIR.into(),
                definition_sha256: hash.into(),
                file_count: 0,
                total_bytes: 0,
            },
            replacements: plan
                .replacements
                .iter()
                .map(|replacement| ReplacementReceipt {
                    table: replacement.table.clone(),
                    partition: replacement.partition.clone(),
                    before_sha256: replacement.expected_before_sha256.clone(),
                    requested_sha256: hash.into(),
                    readback_sha256: hash.into(),
                    materialized_sha256: hash.into(),
                })
                .collect(),
        }
    }

    fn copy_fixture(target: &Path) {
        fs::create_dir_all(target.join("definition").join("tables")).expect("fixture dirs");
        fs::write(target.join("definition.pbism"), "{\"version\":\"4.0\"}").expect("pbism");
        fs::write(
            target.join("definition").join("database.tmdl"),
            "database Synthetic\n\tcompatibilityLevel: 1600\n",
        )
        .expect("database");
        fs::write(
            target.join("definition").join("model.tmdl"),
            "model Model\n\tculture: en-US\n",
        )
        .expect("model");
        fs::write(
            target
                .join("definition")
                .join("tables")
                .join("Synthetic.tmdl"),
            "table Synthetic\n",
        )
        .expect("table");
    }

    fn copy_definition(source: &Path, target: &Path) {
        fs::create_dir_all(target.join("tables")).expect("tables");
        for name in ["database.tmdl", "model.tmdl"] {
            fs::copy(source.join(name), target.join(name)).expect("copy root TMDL");
        }
        fs::copy(
            source.join("tables").join("Synthetic.tmdl"),
            target.join("tables").join("Synthetic.tmdl"),
        )
        .expect("copy table");
    }
}
