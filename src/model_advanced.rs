use crate::cli_support::{required_project, take_value};
use crate::{
    CliError, CliResult, canonical_display, command_arg, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvancedFamily {
    Roles,
    Perspectives,
    Cultures,
    Expressions,
}

impl AdvancedFamily {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "roles" => Some(Self::Roles),
            "perspectives" => Some(Self::Perspectives),
            "cultures" => Some(Self::Cultures),
            "expressions" => Some(Self::Expressions),
            _ => None,
        }
    }

    fn singular(self) -> &'static str {
        match self {
            Self::Roles => "role",
            Self::Perspectives => "perspective",
            Self::Cultures => "culture",
            Self::Expressions => "expression",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Roles => "roles",
            Self::Perspectives => "perspectives",
            Self::Cultures => "cultures",
            Self::Expressions => "expressions",
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::Roles => "roles",
            Self::Perspectives => "perspectives",
            Self::Cultures => "cultures",
            Self::Expressions => "expressions",
        }
    }
}

#[derive(Debug, Default)]
struct Options {
    project: Option<PathBuf>,
    handle: Option<String>,
    name: Option<String>,
    include_raw: bool,
}

#[derive(Debug, Clone)]
struct AdvancedRecord {
    family: AdvancedFamily,
    handle: String,
    name: String,
    path: PathBuf,
    start_line: usize,
    end_line: usize,
    block: String,
    summary: Value,
}

pub(crate) fn advanced_model_command(family: &str, args: &[String]) -> CliResult<Value> {
    if family == "inventory" {
        return inventory(args);
    }
    let family = AdvancedFamily::from_str(family)
        .ok_or_else(|| CliError::unexpected(format!("unknown advanced model family: {family}")))?;
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(format!(
            "model {} requires a subcommand: list or show",
            family.plural()
        ))
        .with_hint("Advanced semantic model mutation is intentionally not exposed until TMDL fixtures exist.")
        .with_suggested_command(format!(
            "powerbi-cli model {} list --project <project-dir-or.pbip> --json",
            family.plural()
        )));
    };
    match action.as_str() {
        "list" | "ls" => list_family(family, rest),
        "show" | "get" => show_family(family, rest),
        "add" | "create" | "update" | "delete" | "remove" => Err(CliError::unsupported_feature(
            format!("model {} mutation is not implemented", family.plural()),
        )
        .with_hint("Use list/show for source inventory. Authoring roles, perspectives, cultures, and named expressions requires Desktop-authored TMDL goldens first.")
        .with_suggested_command(format!(
            "powerbi-cli model {} list --project <project-dir-or.pbip> --json",
            family.plural()
        ))),
        other => Err(CliError::invalid_args(format!(
            "unknown model {} command: {other}",
            family.plural()
        ))
        .with_hint("Run list/show for advanced semantic model inventory.")
        .with_suggested_command(format!(
            "powerbi-cli model {} list --project <project-dir-or.pbip> --json",
            family.plural()
        ))),
    }
}

fn inventory(args: &[String]) -> CliResult<Value> {
    let options = parse_options("model advanced inventory", args)?;
    let project = required_project(options.project, "model advanced inventory")?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let families = [
        AdvancedFamily::Roles,
        AdvancedFamily::Perspectives,
        AdvancedFamily::Cultures,
        AdvancedFamily::Expressions,
    ];
    let mut results = Vec::new();
    for family in families {
        let records = load_family_records(&resolved.semantic_model_dir, family)?;
        results.push(json!({
            "family": family.plural(),
            "count": records.len(),
            "records": records.iter().map(|record| record_json(record, false)).collect::<Vec<_>>()
        }));
    }
    let project_arg = command_arg(&resolved.project_dir);
    Ok(json!({
        "schema": "powerbi-cli.model.advanced.inventory.v1",
        "ok": validation.errors.is_empty(),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "families": results,
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli model roles list --project {project_arg} --json"),
            format!("powerbi-cli model perspectives list --project {project_arg} --json"),
            format!("powerbi-cli model cultures list --project {project_arg} --json")
        ]
    }))
}

fn list_family(family: AdvancedFamily, args: &[String]) -> CliResult<Value> {
    let options = parse_options(&format!("model {} list", family.plural()), args)?;
    let project = required_project(options.project, &format!("model {} list", family.plural()))?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let records = load_family_records(&resolved.semantic_model_dir, family)?;
    let mut counts = serde_json::Map::new();
    counts.insert(family.plural().to_string(), json!(records.len()));
    let project_arg = command_arg(&resolved.project_dir);
    Ok(json!({
        "schema": format!("powerbi-cli.model.{}.list.v1", family.plural()),
        "ok": validation.errors.is_empty(),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "family": family.plural(),
        "counts": Value::Object(counts),
        "records": records.iter().map(|record| record_json(record, options.include_raw)).collect::<Vec<_>>(),
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!(
                "powerbi-cli model {} show --project {project_arg} --handle <{}-handle> --json",
                family.plural(),
                family.singular()
            ),
            format!("powerbi-cli validate --strict {project_arg} --json")
        ]
    }))
}

fn show_family(family: AdvancedFamily, args: &[String]) -> CliResult<Value> {
    let options = parse_options(&format!("model {} show", family.plural()), args)?;
    let project = required_project(
        options.project.clone(),
        &format!("model {} show", family.plural()),
    )?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let records = load_family_records(&resolved.semantic_model_dir, family)?;
    let record = find_record(&records, &options, family)?;
    let project_arg = command_arg(&resolved.project_dir);
    Ok(json!({
        "schema": format!("powerbi-cli.model.{}.show.v1", family.plural()),
        "ok": validation.errors.is_empty(),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "family": family.plural(),
        "record": record_json(record, true),
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli model {} list --project {project_arg} --json", family.plural()),
            format!("powerbi-cli validate --strict {project_arg} --json")
        ]
    }))
}

fn load_family_records(
    semantic_model_dir: &Path,
    family: AdvancedFamily,
) -> CliResult<Vec<AdvancedRecord>> {
    let definition = semantic_model_dir.join("definition");
    let folder = definition.join(family.folder());
    let mut paths = Vec::new();
    if folder.is_dir() {
        for entry in fs::read_dir(&folder)
            .map_err(|err| CliError::unexpected(format!("read {}: {err}", folder.display())))?
        {
            let path = entry
                .map_err(|err| CliError::unexpected(format!("read {}: {err}", folder.display())))?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("tmdl") {
                paths.push(path);
            }
        }
    }
    if family == AdvancedFamily::Expressions {
        let root_file = definition.join("expressions.tmdl");
        if root_file.is_file() {
            paths.push(root_file);
        }
    }
    paths.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
    let mut records = Vec::new();
    for path in paths {
        records.extend(parse_records_from_file(family, &path)?);
    }
    Ok(records)
}

fn parse_records_from_file(family: AdvancedFamily, path: &Path) -> CliResult<Vec<AdvancedRecord>> {
    let text = fs::read_to_string(path)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?;
    let lines = text.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if family != AdvancedFamily::Expressions {
        let name = lines
            .iter()
            .find_map(|line| object_name_from_line(line, family.singular()))
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or(family.singular())
                    .to_string()
            });
        return Ok(vec![record_from_block(
            family,
            name,
            path.to_path_buf(),
            0,
            lines.len(),
            text,
        )]);
    }

    let mut starts = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if object_name_from_line(line, "expression").is_some() {
            starts.push(index);
        }
    }
    if starts.is_empty() {
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("expressions")
            .to_string();
        return Ok(vec![record_from_block(
            family,
            name,
            path.to_path_buf(),
            0,
            lines.len(),
            text,
        )]);
    }
    let mut records = Vec::new();
    for (ordinal, start) in starts.iter().enumerate() {
        let end = starts.get(ordinal + 1).copied().unwrap_or(lines.len());
        let block = lines[*start..end].join("\n");
        let name = object_name_from_line(&lines[*start], "expression")
            .unwrap_or_else(|| format!("expression-{}", ordinal + 1));
        records.push(record_from_block(
            family,
            name,
            path.to_path_buf(),
            *start,
            end,
            block,
        ));
    }
    Ok(records)
}

fn record_from_block(
    family: AdvancedFamily,
    name: String,
    path: PathBuf,
    start_line: usize,
    end_line: usize,
    block: String,
) -> AdvancedRecord {
    let handle = format!("{}:{}", family.singular(), name);
    let summary = summary_for_block(family, &block);
    AdvancedRecord {
        family,
        handle,
        name,
        path,
        start_line,
        end_line,
        block,
        summary,
    }
}

fn summary_for_block(family: AdvancedFamily, block: &str) -> Value {
    let lower = block.to_ascii_lowercase();
    match family {
        AdvancedFamily::Roles => json!({
            "modelPermission": property_value(block, "modelPermission"),
            "tablePermissions": lower.matches("tablepermission").count(),
            "members": lower.lines().filter(|line| line.trim_start().starts_with("member ")).count()
        }),
        AdvancedFamily::Perspectives => json!({
            "perspectiveTables": lower.matches("perspectivetable").count(),
            "perspectiveColumns": lower.matches("perspectivecolumn").count(),
            "perspectiveMeasures": lower.matches("perspectivemeasure").count()
        }),
        AdvancedFamily::Cultures => json!({
            "translations": lower.matches("translation").count()
        }),
        AdvancedFamily::Expressions => json!({
            "expressionKind": "named-expression",
            "lineCount": block.lines().count()
        }),
    }
}

fn property_value(block: &str, property: &str) -> Option<String> {
    let prefix = format!("{property}:");
    block.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .map(|value| value.trim().to_string())
    })
}

fn object_name_from_line(line: &str, keyword: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix(keyword)?.trim();
    if rest.is_empty() {
        return None;
    }
    if let Some(rest) = rest.strip_prefix('\'') {
        let mut value = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\'' {
                if matches!(chars.peek(), Some('\'')) {
                    chars.next();
                    value.push('\'');
                    continue;
                }
                return Some(value);
            }
            value.push(ch);
        }
        return Some(value);
    }
    Some(
        rest.split_whitespace()
            .next()
            .unwrap_or(rest)
            .trim_end_matches('=')
            .to_string(),
    )
}

fn record_json(record: &AdvancedRecord, include_raw: bool) -> Value {
    let mut value = json!({
        "handle": record.handle,
        "family": record.family.plural(),
        "kind": record.family.singular(),
        "name": record.name,
        "path": canonical_display(&record.path),
        "lineRange": {
            "start": record.start_line + 1,
            "end": record.end_line
        },
        "summary": record.summary,
        "mutationSupport": {
            "status": "read-only",
            "reason": "Advanced TMDL object mutation is fixture-gated; this command inventories source files only."
        }
    });
    if include_raw {
        value["block"] = Value::String(record.block.clone());
    }
    value
}

fn find_record<'a>(
    records: &'a [AdvancedRecord],
    options: &Options,
    family: AdvancedFamily,
) -> CliResult<&'a AdvancedRecord> {
    if let Some(handle) = &options.handle {
        return records
            .iter()
            .find(|record| &record.handle == handle)
            .ok_or_else(|| {
                CliError::validation_failed(format!("{} not found: {handle}", family.singular()))
                    .with_hint("Run list to get stable handles.")
                    .with_suggested_command(format!(
                        "powerbi-cli model {} list --project <project-dir-or.pbip> --json",
                        family.plural()
                    ))
            });
    }
    if let Some(name) = &options.name {
        let matches = records
            .iter()
            .filter(|record| record.name.eq_ignore_ascii_case(name))
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [record] => Ok(record),
            [] => Err(CliError::validation_failed(format!(
                "{} not found: {name}",
                family.singular()
            ))
            .with_suggested_command(format!(
                "powerbi-cli model {} list --project <project-dir-or.pbip> --json",
                family.plural()
            ))),
            _ => Err(CliError::validation_failed(format!(
                "{} selector is ambiguous: {name}",
                family.singular()
            ))
            .with_hint("Use the exact handle returned by list.")),
        };
    }
    Err(CliError::invalid_args(format!(
        "model {} show requires --handle or --name",
        family.plural()
    ))
    .with_hint("Use list to get stable handles.")
    .with_suggested_command(format!(
        "powerbi-cli model {} show --project <project-dir-or.pbip> --handle {}:<name> --json",
        family.plural(),
        family.singular()
    )))
}

fn parse_options(command: &str, args: &[String]) -> CliResult<Options> {
    let mut options = Options::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(
                    CliError::invalid_args(format!("unknown {command} flag: {other}"))
                        .with_hint("Run capabilities for exact advanced model command flags.")
                        .with_suggested_command("powerbi-cli --json capabilities --for model"),
                );
            }
        }
    }
    Ok(options)
}
