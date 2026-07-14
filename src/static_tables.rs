use crate::project_io::{copy_project_dir, write_text_atomic_validated};
use crate::safety_scan::contains_credential_like_text_str;
use crate::tmdl::{load_table_documents, same_name};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

const MAX_VALUES: usize = 100;
const MAX_VALUE_CHARS: usize = 200;

pub(crate) fn static_tables_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model tables requires a subcommand: add-static",
        )
        .with_hint(
            "Run `powerbi-cli model tables add-static --project <project> --table <table> --column <column> --values-json '[\"One\",\"Two\"]' --dry-run --json`.",
        )
        .with_suggested_command(
            "powerbi-cli model tables add-static --project <project-dir-or.pbip> --table Metric --column Metric --values-json '[\"Count\",\"Cost\"]' --dry-run --json",
        ));
    };

    match action.as_str() {
        "add-static" | "addStatic" | "add-selector" | "addSelector" => add_static_table(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown model tables command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for add-static` for the supported command.")
        .with_suggested_command("powerbi-cli --json capabilities --for add-static")),
    }
}

#[derive(Debug)]
enum MutationMode {
    DryRun,
    InPlace,
    OutDir(PathBuf),
}

#[derive(Debug, Default)]
struct AddOptions {
    project: Option<PathBuf>,
    table: Option<String>,
    column: Option<String>,
    values: Option<Vec<String>>,
    mode: Option<MutationMode>,
    include_raw: bool,
}

fn add_static_table(args: &[String]) -> CliResult<Value> {
    let options = parse_add_args(args)?;
    let source_project = options.project.as_ref().expect("validated project");
    let source_resolved = resolve_project(source_project)?;
    let mode = options.mode.as_ref().expect("validated mode");
    let target_resolved = match mode {
        MutationMode::DryRun | MutationMode::InPlace => source_resolved,
        MutationMode::OutDir(out_dir) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)?
        }
    };

    let table = options.table.as_deref().expect("validated table");
    let column = options.column.as_deref().expect("validated column");
    let values = options.values.as_ref().expect("validated values");
    let docs = load_table_documents(&target_resolved)?;
    if docs.iter().any(|doc| same_name(&doc.table, table)) {
        return Err(CliError::invalid_args(format!(
            "semantic model table already exists: {table}"
        ))
        .with_hint("Choose a new disconnected control-table name; this command never replaces an existing table.")
        .with_suggested_command(format!(
            "powerbi-cli inspect --deep {} --json",
            command_arg(&target_resolved.project_dir)
        )));
    }

    let tables_dir = target_resolved
        .semantic_model_dir
        .join("definition")
        .join("tables");
    let tables_dir = std::fs::canonicalize(&tables_dir).unwrap_or(tables_dir);
    let path = tables_dir.join(format!("{table}.tmdl"));
    if path.exists() {
        return Err(CliError::invalid_args(format!(
            "static table target already exists: {}",
            path.display()
        ))
        .with_hint("The command never overwrites a table file."));
    }

    let tmdl = static_table_tmdl(table, column, values);
    let dry_run = matches!(mode, MutationMode::DryRun);
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        let (validation, modified) = write_text_atomic_validated(
            &path,
            &tmdl,
            || validate_project(&target_resolved),
            |report| report.errors.is_empty(),
        )?;
        (Some(validation), modified)
    };
    let validation_ok = validation
        .as_ref()
        .map(|report| report.errors.is_empty())
        .unwrap_or(true);
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&target_resolved.project_dir);
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let partition_readback = format!(
        "powerbi-cli model partitions show --project {project_arg} --handle {} --json",
        shell_arg(&format!("partition:{table}:{table}"))
    );

    Ok(json!({
        "schema": "powerbi-cli.model.tables.staticMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "add-static",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({
            "performed": true,
            "projectModified": false,
            "reason": "post-mutation validation failed; the new TMDL table file was removed"
        })),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "semanticModelDir": canonical_display(&target_resolved.semantic_model_dir),
        "target": {
            "handle": format!("table:{table}"),
            "table": table,
            "column": column,
            "path": canonical_display(&path)
        },
        "tablePlan": {
            "kind": "staticDisconnectedControlTable",
            "dataType": "string",
            "rowCount": values.len(),
            "uniqueValues": true,
            "relationshipCount": 0,
            "values": options.include_raw.then(|| values.clone()),
            "tmdl": options.include_raw.then(|| tmdl.clone())
        },
        "changes": [{
            "kind": "tmdl.staticTable",
            "action": "add",
            "path": canonical_display(&path),
            "before": Value::Null,
            "after": options.include_raw.then(|| tmdl.clone()).unwrap_or_else(|| format!("static string table {table}[{column}] with {} rows", values.len()))
        }],
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors,
            "counts": {
                "tables": report.tables,
                "relationships": report.relationships,
                "measures": report.measures,
                "pages": report.pages,
                "visuals": report.visuals
            }
        })),
        "readbackCommand": partition_readback,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [partition_readback, inspect, validate]
    }))
}

fn parse_add_args(args: &[String]) -> CliResult<AddOptions> {
    let mut options = AddOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => {
                options.project = Some(PathBuf::from(required_value(args, i, "--project")?));
                i += 2;
            }
            "--table" => {
                options.table = Some(required_value(args, i, "--table")?.to_string());
                i += 2;
            }
            "--column" => {
                options.column = Some(required_value(args, i, "--column")?.to_string());
                i += 2;
            }
            "--values-json" => {
                let raw = required_value(args, i, "--values-json")?;
                let value: Value = serde_json::from_str(raw).map_err(|err| {
                    CliError::invalid_args(format!("--values-json is not valid JSON: {err}"))
                        .with_hint("Pass a JSON array of short strings, for example '[\"Count\",\"Cost\"]'.")
                })?;
                let array = value.as_array().ok_or_else(|| {
                    CliError::invalid_args("--values-json must be a JSON array of strings")
                })?;
                let mut values = Vec::with_capacity(array.len());
                for item in array {
                    let text = item.as_str().ok_or_else(|| {
                        CliError::invalid_args("--values-json must contain only strings")
                    })?;
                    values.push(text.to_string());
                }
                options.values = Some(values);
                i += 2;
            }
            "--include-raw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" => {
                let out_dir = PathBuf::from(required_value(args, i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir(out_dir))?;
                i += 2;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model tables add-static flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for add-static`."));
            }
        }
    }

    let project = options
        .project
        .as_ref()
        .ok_or_else(|| CliError::invalid_args("model tables add-static requires --project"))?;
    if project.as_os_str().is_empty() {
        return Err(CliError::invalid_args("--project cannot be empty"));
    }
    let table = options
        .table
        .as_deref()
        .ok_or_else(|| CliError::invalid_args("model tables add-static requires --table"))?;
    validate_table_file_name(table)?;
    let column = options
        .column
        .as_deref()
        .ok_or_else(|| CliError::invalid_args("model tables add-static requires --column"))?;
    validate_object_name(column, "--column")?;
    let values = options
        .values
        .as_ref()
        .ok_or_else(|| CliError::invalid_args("model tables add-static requires --values-json"))?;
    validate_values(values)?;
    if options.mode.is_none() {
        return Err(CliError::invalid_args(
            "model tables add-static requires --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with --dry-run and inspect the emitted plan."));
    }
    Ok(options)
}

fn validate_table_file_name(value: &str) -> CliResult<()> {
    validate_object_name(value, "--table")?;
    if value.ends_with('.')
        || value.ends_with(' ')
        || value
            .chars()
            .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
    {
        return Err(CliError::invalid_args(format!(
            "--table is not a portable table/file name: {value}"
        ))
        .with_hint("Use a short semantic-model table name without path separators or filesystem-reserved characters."));
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err(CliError::invalid_args(format!(
            "--table uses a filesystem-reserved name: {value}"
        )));
    }
    Ok(())
}

fn validate_object_name(value: &str, flag: &str) -> CliResult<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed != value || value.chars().count() > 100 {
        return Err(CliError::invalid_args(format!(
            "{flag} must be a non-empty name of at most 100 characters without leading or trailing whitespace"
        )));
    }
    Ok(())
}

fn validate_values(values: &[String]) -> CliResult<()> {
    if values.is_empty() || values.len() > MAX_VALUES {
        return Err(CliError::invalid_args(format!(
            "--values-json must contain between 1 and {MAX_VALUES} strings"
        )));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty()
            || trimmed != value
            || value.chars().count() > MAX_VALUE_CHARS
            || value.contains(['\r', '\n'])
        {
            return Err(CliError::invalid_args(format!(
                "static control-table values must be non-empty, single-line strings of at most {MAX_VALUE_CHARS} characters without surrounding whitespace"
            )));
        }
        if !unique.insert(value.to_lowercase()) {
            return Err(CliError::invalid_args(format!(
                "static control-table values must be unique (case-insensitive): {value}"
            )));
        }
    }
    let joined = values.join("\n");
    if contains_credential_like_text_str(&joined) {
        return Err(CliError::invalid_args(
            "static control-table values contain credential-like text",
        )
        .with_hint("Keep credentials in Power BI Desktop; this command is only for short non-sensitive selector labels."));
    }
    Ok(())
}

fn static_table_tmdl(table: &str, column: &str, values: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("table {}\n", tmdl_object_name(table)));
    out.push_str(&format!(
        "    lineageTag: {}\n\n",
        stable_guid(&format!("table:{table}"))
    ));
    out.push_str(&format!("    column {}\n", tmdl_object_name(column)));
    out.push_str("        dataType: string\n");
    out.push_str(&format!(
        "        lineageTag: {}\n",
        stable_guid(&format!("column:{table}:{column}"))
    ));
    out.push_str("        summarizeBy: none\n");
    out.push_str(&format!(
        "        sourceColumn: {}\n\n",
        tmdl_object_name(column)
    ));
    out.push_str(&format!("    partition {} = m\n", tmdl_object_name(table)));
    out.push_str("        mode: import\n");
    out.push_str("        source =\n");
    out.push_str("            let\n");
    out.push_str("                Source = #table(\n");
    out.push_str(&format!(
        "                    type table [{} = text],\n",
        m_identifier(column)
    ));
    out.push_str("                    {\n");
    for (index, value) in values.iter().enumerate() {
        let suffix = if index + 1 == values.len() { "" } else { "," };
        out.push_str(&format!(
            "                        {{\"{}\"}}{suffix}\n",
            m_escape_string(value)
        ));
    }
    out.push_str("                    }\n");
    out.push_str("                )\n");
    out.push_str("            in\n");
    out.push_str("                Source\n");
    out
}

fn tmdl_object_name(value: &str) -> String {
    if is_simple_identifier(value) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn m_identifier(value: &str) -> String {
    if is_simple_identifier(value) {
        value.to_string()
    } else {
        format!("#\"{}\"", value.replace('"', "\"\""))
    }
}

fn m_escape_string(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn is_simple_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn stable_guid(value: &str) -> String {
    let a = hash_hex(value);
    let b = hash_hex(&format!("{value}:powerbi-cli"));
    let hex = format!("{a}{b}");
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[16..19],
        &hex[19..31]
    )
}

fn hash_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn required_value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))
}

fn set_mode(target: &mut Option<MutationMode>, mode: MutationMode) -> CliResult<()> {
    if target.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one of --dry-run, --in-place, or --out-dir",
        ));
    }
    *target = Some(mode);
    Ok(())
}

fn mode_name(mode: &MutationMode) -> &'static str {
    match mode {
        MutationMode::DryRun => "dry-run",
        MutationMode::InPlace => "in-place",
        MutationMode::OutDir(_) => "out-dir",
    }
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_table_tmdl_is_deterministic_and_escapes_labels() {
        let tmdl = static_table_tmdl(
            "Kennzahl",
            "Kennzahl",
            &["Anzahl Unfälle".to_string(), "Kosten \"CHF\"".to_string()],
        );
        assert!(tmdl.contains("table Kennzahl"));
        assert!(tmdl.contains("type table [Kennzahl = text]"));
        assert!(tmdl.contains("{\"Anzahl Unfälle\"}"));
        assert!(tmdl.contains("{\"Kosten \"\"CHF\"\"\"}"));
        assert!(!tmdl.ends_with("\n\n"));
        assert_eq!(
            tmdl,
            static_table_tmdl(
                "Kennzahl",
                "Kennzahl",
                &["Anzahl Unfälle".to_string(), "Kosten \"CHF\"".to_string()]
            )
        );
    }

    #[test]
    fn static_values_reject_duplicates_and_credentials() {
        assert!(validate_values(&["Count".to_string(), "count".to_string()]).is_err());
        assert!(validate_values(&["password=secret".to_string()]).is_err());
    }
}
