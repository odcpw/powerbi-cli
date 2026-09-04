//! Offline synthetic source swap: the `workflow synthesize` command family.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::MAX_SOURCE_TEXT_BYTES;
use super::shared::{
    MAX_HASHED_TREE_BYTES, MAX_HASHED_TREE_FILES, MAX_PROFILE_BYTES, MAX_RESOURCE_BYTES, MToken,
    OwnedWorkflowOutput, canonical_plain_directory, canonical_plain_file, claim_for_file,
    copy_new_output_file, m_tokens, metadata_is_link_or_reparse, read_bounded,
    resolve_new_directory_candidate, unicode_path, validate_relative_path,
};
use crate::tmdl::load_table_documents;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_command,
};

const WORKFLOW_SYNTHESIZE_SCHEMA: &str = "powerbi-cli.workflow-synthesize.v1";
const MAX_EXACT_M_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug)]
struct SynthesizeOptions {
    project: PathBuf,
    expressions: PathBuf,
    output_dir: PathBuf,
    mappings: BTreeMap<(String, String), String>,
    row_scale: Option<u64>,
    seed: Option<u64>,
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
    let generation_parameters = options
        .row_scale
        .zip(options.seed)
        .map(|(row_scale, seed)| SynthesizeGenerationParameters { row_scale, seed });
    let shim = synthesize_navigation_shim(&expression_mappings, generation_parameters)?;

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
        "generationParameters": generation_parameters.map(|parameters| json!({
            "rowScale": parameters.row_scale,
            "seed": parameters.seed
        })),
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
    let mut row_scale = None;
    let mut seed = None;
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
            "--row-scale" => {
                if row_scale.is_some() {
                    return Err(synthesize_generation_args_error(
                        "--row-scale may be specified only once",
                    ));
                }
                row_scale = Some(parse_generation_integer(
                    args.get(index + 1).ok_or_else(|| {
                        synthesize_generation_args_error("--row-scale requires a positive integer")
                    })?,
                    "--row-scale",
                    false,
                )?);
                index += 2;
            }
            "--seed" => {
                if seed.is_some() {
                    return Err(synthesize_generation_args_error(
                        "--seed may be specified only once",
                    ));
                }
                seed = Some(parse_generation_integer(
                    args.get(index + 1).ok_or_else(|| {
                        synthesize_generation_args_error("--seed requires an integer")
                    })?,
                    "--seed",
                    true,
                )?);
                index += 2;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown workflow synthesize argument: {other}"
                ))
                .with_hint(
                    "Use --project, --expressions, --out-dir, optional repeated --map flags, and optional --row-scale/--seed generation parameters.",
                ));
            }
        }
    }
    if row_scale.is_some() || seed.is_some() {
        row_scale.get_or_insert(1);
        seed.get_or_insert(0);
    }
    Ok(SynthesizeOptions {
        project: project
            .ok_or_else(|| CliError::invalid_args("workflow synthesize requires --project"))?,
        expressions: expressions
            .ok_or_else(|| CliError::invalid_args("workflow synthesize requires --expressions"))?,
        output_dir: output_dir
            .ok_or_else(|| CliError::invalid_args("workflow synthesize requires --out-dir"))?,
        mappings,
        row_scale,
        seed,
    })
}

fn parse_generation_integer(value: &str, flag: &str, allow_zero: bool) -> CliResult<u64> {
    let parsed = value.parse::<u64>().map_err(|_| {
        synthesize_generation_args_error(format!(
            "{flag} must be {} integer no greater than {MAX_EXACT_M_INTEGER}",
            if allow_zero {
                "a non-negative"
            } else {
                "a positive"
            }
        ))
    })?;
    if (!allow_zero && parsed == 0) || parsed > MAX_EXACT_M_INTEGER {
        return Err(synthesize_generation_args_error(format!(
            "{flag} must be {} integer no greater than {MAX_EXACT_M_INTEGER}",
            if allow_zero {
                "a non-negative"
            } else {
                "a positive"
            }
        )));
    }
    Ok(parsed)
}

fn synthesize_generation_args_error(message: impl Into<String>) -> CliError {
    CliError::invalid_args(message)
        .with_hint("Use exact non-negative integer literals; row scale must be at least one.")
        .with_suggested_command("powerbi-cli capabilities --for workflow --json")
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

#[derive(Clone, Copy, Debug)]
struct SynthesizeGenerationParameters {
    row_scale: u64,
    seed: u64,
}

fn synthesize_navigation_shim(
    mappings: &BTreeMap<(String, String), String>,
    generation_parameters: Option<SynthesizeGenerationParameters>,
) -> CliResult<String> {
    let rows = mappings
        .iter()
        .map(|((schema, item), expression)| {
            let expression = m_expression_reference(expression)?;
            let expression = match generation_parameters {
                Some(parameters) => format!(
                    "{expression}({}, {})",
                    parameters.row_scale, parameters.seed
                ),
                None => expression,
            };
            Ok(format!(
                "{{{}, {}, {}}}",
                m_navigation_string(schema)?,
                m_navigation_string(item)?,
                expression
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_flags_fill_only_the_missing_paired_default() {
        let row_only = parse_synthesize_args(&[
            "--project".into(),
            "p.pbip".into(),
            "--expressions".into(),
            "e.tmdl".into(),
            "--out-dir".into(),
            "out".into(),
            "--row-scale".into(),
            "25".into(),
        ])
        .expect("row-scale options");
        assert_eq!(row_only.row_scale, Some(25));
        assert_eq!(row_only.seed, Some(0));

        let seed_only = parse_synthesize_args(&[
            "--project".into(),
            "p.pbip".into(),
            "--expressions".into(),
            "e.tmdl".into(),
            "--out-dir".into(),
            "out".into(),
            "--seed".into(),
            "9".into(),
        ])
        .expect("seed options");
        assert_eq!(seed_only.row_scale, Some(1));
        assert_eq!(seed_only.seed, Some(9));
    }

    #[test]
    fn scaled_navigation_shim_passes_parameters_to_every_expression() {
        let mappings = BTreeMap::from([
            (("crm".into(), "customers".into()), "Customers".into()),
            (("sales".into(), "orders".into()), "Order Generator".into()),
        ]);
        let shim = synthesize_navigation_shim(
            &mappings,
            Some(SynthesizeGenerationParameters {
                row_scale: 100,
                seed: 42,
            }),
        )
        .expect("scaled shim");
        assert_eq!(
            shim,
            "Database = #table({\"Schema\", \"Item\", \"Data\"}, {{\"crm\", \"customers\", Customers(100, 42)}, {\"sales\", \"orders\", #\"Order Generator\"(100, 42)}}),"
        );
    }
}
