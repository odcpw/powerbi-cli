use crate::schema::{load_schema_value, validate_schema_value};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct ProfileArgs {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
    rows: Option<PathBuf>,
    out: Option<PathBuf>,
}

pub(crate) fn profile_command(args: &[String]) -> CliResult<Value> {
    match args {
        [action, rest @ ..] if action == "infer" => infer_command(rest),
        [action, rest @ ..] if action == "validate" => validate_command(rest),
        [action, rest @ ..] if action == "summarize" => summarize_command(rest),
        [] => Err(CliError::invalid_args(
            "profile requires a subcommand: infer, validate, or summarize",
        )
        .with_hint("Run `powerbi-cli profile infer --schema <schema.json> --json`.")
        .with_suggested_command("powerbi-cli profile infer --schema <schema.json> --json")),
        _ => Err(CliError::invalid_args("unknown profile command")
            .with_hint("Run `powerbi-cli --json capabilities --for profile`.")
            .with_suggested_command("powerbi-cli --json capabilities --for profile")),
    }
}

fn infer_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args, "profile infer")?;
    let schema_path = options.schema.ok_or_else(|| {
        CliError::invalid_args("profile infer requires --schema <schema.json>")
            .with_suggested_command("powerbi-cli profile infer --schema <schema.json> --json")
    })?;
    if let Some(rows) = &options.rows {
        return Err(CliError::unsupported_feature(format!(
            "profile infer from external rows is not implemented yet: {}; use dummy rows embedded in --schema",
            rows.display()
        ))
        .with_suggested_command(format!(
            "powerbi-cli profile infer --schema {} --json",
            command_arg(&schema_path)
        )));
    }
    let schema = load_schema_value(&schema_path)?;
    let validation = validate_schema_value(&schema);
    if !validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "cannot infer profile from invalid schema: {}",
            validation.errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli schema validate {} --json",
            command_arg(&schema_path)
        )));
    }
    let profile = infer_profile(&schema, &schema_path);
    if let Some(out) = &options.out {
        if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|err| {
                CliError::unexpected(format!(
                    "create output directory {}: {err}",
                    parent.display()
                ))
            })?;
        }
        fs::write(
            out,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&profile).expect("serialize profile")
            ),
        )
        .map_err(|err| CliError::unexpected(format!("write {}: {err}", out.display())))?;
    }
    let next = if let Some(out) = &options.out {
        vec![
            format!("powerbi-cli profile validate {} --json", command_arg(out)),
            format!(
                "powerbi-cli report plan --schema {} --profile {} --objective <dashboard-goal> --out <dashboard.json> --json",
                command_arg(&schema_path),
                command_arg(out)
            ),
            format!(
                "powerbi-cli report spec validate --schema {} --profile {} --spec <dashboard.json> --json",
                command_arg(&schema_path),
                command_arg(out)
            ),
            format!(
                "powerbi-cli report build --schema {} --profile {} --spec <dashboard.json> --out-dir <project-dir> --json",
                command_arg(&schema_path),
                command_arg(out)
            ),
        ]
    } else {
        vec![
            format!(
                "powerbi-cli profile infer --schema {} --out <profile.json> --json",
                command_arg(&schema_path)
            ),
            format!(
                "powerbi-cli report plan --schema {} --profile <profile.json> --objective <dashboard-goal> --out <dashboard.json> --json",
                command_arg(&schema_path)
            ),
            format!(
                "powerbi-cli report spec validate --schema {} --profile <profile.json> --spec <dashboard.json> --json",
                command_arg(&schema_path)
            ),
            format!(
                "powerbi-cli report build --schema {} --profile <profile.json> --spec <dashboard.json> --out-dir <project-dir> --json",
                command_arg(&schema_path)
            ),
        ]
    };
    Ok(json!({
        "schema": "powerbi-cli.profile.infer.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "schemaPath": canonical_display(&schema_path),
        "profilePath": options.out.as_ref().map(|path| canonical_display(path)),
        "profile": profile,
        "next": next
    }))
}

fn validate_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args, "profile validate")?;
    let profile_path = required_profile_path(options.profile, "profile validate")?;
    let profile = load_profile_value(&profile_path)?;
    let errors = validate_profile_value(&profile);
    let ok = errors.is_empty();
    Ok(json!({
        "schema": "powerbi-cli.profile.validate.v1",
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "profilePath": canonical_display(&profile_path),
        "errors": errors,
        "summary": profile_summary(&profile),
        "next": if ok { vec![
            format!("powerbi-cli profile summarize {} --json", command_arg(&profile_path))
        ] } else { Vec::<String>::new() }
    }))
}

fn summarize_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args, "profile summarize")?;
    let profile_path = required_profile_path(options.profile, "profile summarize")?;
    let profile = load_profile_value(&profile_path)?;
    let errors = validate_profile_value(&profile);
    let ok = errors.is_empty();
    Ok(json!({
        "schema": "powerbi-cli.profile.summary.v1",
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "profilePath": canonical_display(&profile_path),
        "summary": profile_summary(&profile),
        "errors": errors
    }))
}

fn infer_profile(schema: &Value, schema_path: &Path) -> Value {
    let tables = schema["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(table_profile)
        .collect::<Vec<_>>();
    let fact_tables = tables
        .iter()
        .filter(|table| table["role"] == "fact")
        .map(|table| table["name"].clone())
        .collect::<Vec<_>>();
    let dimension_tables = tables
        .iter()
        .filter(|table| table["role"] == "dimension")
        .map(|table| table["name"].clone())
        .collect::<Vec<_>>();
    let date_columns = collect_columns(&tables, "dateLike");
    let numeric_columns = collect_columns(&tables, "numeric");
    let category_columns = collect_columns(&tables, "categorical");
    json!({
        "schema": "powerbi-cli.dataProfile.v1",
        "source": {
            "kind": "schema-embedded-dummy-rows",
            "schemaPath": schema_path.to_string_lossy()
        },
        "tables": tables,
        "candidates": {
            "factTables": fact_tables,
            "dimensionTables": dimension_tables,
            "dateColumns": date_columns,
            "numericColumns": numeric_columns,
            "categoryColumns": category_columns
        },
        "warnings": profile_warnings(schema)
    })
}

fn table_profile(table: &Map<String, Value>) -> Value {
    let name = string_field(table, "name").unwrap_or_default();
    let columns = table
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|column| column_profile(column, table.get("rows").and_then(Value::as_array)))
        .collect::<Vec<_>>();
    let rows = table
        .get("rows")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let numeric_count = columns
        .iter()
        .filter(|column| column["roles"]["numeric"].as_bool() == Some(true))
        .count();
    let key_count = columns
        .iter()
        .filter(|column| column["isKey"].as_bool() == Some(true))
        .count();
    let lower_name = name.to_ascii_lowercase();
    let role = if lower_name.starts_with("fact") {
        "fact"
    } else if lower_name.starts_with("dim") || key_count > 0 {
        "dimension"
    } else if numeric_count >= 2 && rows > 0 {
        "fact"
    } else {
        "unknown"
    };
    json!({
        "name": name,
        "role": role,
        "rowCount": rows,
        "columns": columns
    })
}

fn column_profile(column: &Map<String, Value>, rows: Option<&Vec<Value>>) -> Value {
    let name = string_field(column, "name").unwrap_or_default();
    let data_type = string_field(column, "dataType").unwrap_or_else(|| "string".to_string());
    let mut null_count = 0usize;
    let mut distinct = BTreeSet::new();
    let mut sample_values = Vec::new();
    if let Some(rows) = rows {
        for row in rows.iter().filter_map(Value::as_object) {
            match row.get(&name) {
                None | Some(Value::Null) => null_count += 1,
                Some(value) => {
                    let rendered = render_value(value);
                    distinct.insert(rendered.clone());
                    if sample_values.len() < 5
                        && !sample_values.contains(&Value::String(rendered.clone()))
                    {
                        sample_values.push(Value::String(rendered));
                    }
                }
            }
        }
    }
    let lower_name = name.to_ascii_lowercase();
    let lower_type = data_type.to_ascii_lowercase();
    let date_like = lower_type.contains("date")
        || lower_type.contains("time")
        || lower_name.contains("date")
        || lower_name.contains("datum")
        || lower_name.contains("year")
        || lower_name.contains("jahr")
        || lower_name.contains("month")
        || lower_name.contains("monat");
    let numeric = matches!(
        lower_type.as_str(),
        "int"
            | "integer"
            | "whole"
            | "whole_number"
            | "int64"
            | "double"
            | "float"
            | "number"
            | "decimal"
            | "fixed_decimal"
            | "currency"
    );
    let is_key = column
        .get("isKey")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let categorical = !is_key
        && (matches!(
            lower_type.as_str(),
            "string" | "text" | "boolean" | "bool" | "logical"
        ) || lower_name.contains("branch")
            || lower_name.contains("category")
            || lower_name.contains("segment")
            || lower_name.contains("status")
            || lower_name.contains("type")
            || lower_name.contains("group"));
    json!({
        "name": name,
        "dataType": data_type,
        "isKey": is_key,
        "nullCount": null_count,
        "distinctCount": distinct.len(),
        "sampleValues": sample_values,
        "roles": {
            "dateLike": date_like,
            "numeric": numeric,
            "categorical": categorical
        }
    })
}

fn collect_columns(tables: &[Value], role: &str) -> Vec<Value> {
    let mut items = Vec::new();
    for table in tables {
        let table_name = table["name"].as_str().unwrap_or_default();
        for column in table["columns"].as_array().into_iter().flatten() {
            if column["roles"][role].as_bool() == Some(true) {
                items.push(json!({
                    "table": table_name,
                    "column": column["name"],
                    "dataType": column["dataType"],
                    "field": format!("{}[{}]", table_name, column["name"].as_str().unwrap_or_default())
                }));
            }
        }
    }
    items
}

fn profile_warnings(schema: &Value) -> Vec<Value> {
    let mut warnings = Vec::new();
    for table in schema["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        let row_count = table
            .get("rows")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if row_count == 0 {
            warnings.push(json!({
                "code": "profile.no_dummy_rows",
                "message": format!(
                    "table {} has no embedded dummy rows; profile will rely on schema metadata only",
                    string_field(table, "name").unwrap_or_default()
                )
            }));
        }
    }
    warnings
}

pub(crate) fn validate_profile_value(profile: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if profile["schema"].as_str() != Some("powerbi-cli.dataProfile.v1") {
        errors.push("profile schema must be powerbi-cli.dataProfile.v1".to_string());
    }
    if profile["tables"]
        .as_array()
        .is_none_or(|tables| tables.is_empty())
    {
        errors.push("profile must contain a non-empty tables array".to_string());
    }
    errors
}

pub(crate) fn profile_summary(profile: &Value) -> Value {
    let empty = Vec::new();
    let tables = profile["tables"].as_array().unwrap_or(&empty);
    let mut roles: BTreeMap<&str, usize> = BTreeMap::new();
    for table in tables {
        let role = table["role"].as_str().unwrap_or("unknown");
        *roles.entry(role).or_default() += 1;
    }
    json!({
        "tables": tables.len(),
        "columns": tables
            .iter()
            .map(|table| table["columns"].as_array().map_or(0, Vec::len))
            .sum::<usize>(),
        "tableRoles": roles,
        "candidateFactTables": profile["candidates"]["factTables"].as_array().map_or(0, Vec::len),
        "candidateDateColumns": profile["candidates"]["dateColumns"].as_array().map_or(0, Vec::len),
        "candidateNumericColumns": profile["candidates"]["numericColumns"].as_array().map_or(0, Vec::len),
        "candidateCategoryColumns": profile["candidates"]["categoryColumns"].as_array().map_or(0, Vec::len)
    })
}

pub(crate) fn load_profile_value(path: &Path) -> CliResult<Value> {
    let text = fs::read_to_string(path).map_err(|err| {
        CliError::file_not_found(format!("read profile {}: {err}", path.display()))
    })?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::invalid_args(format!("parse profile {}: {err}", path.display())))
}

fn parse_args(args: &[String], command: &str) -> CliResult<ProfileArgs> {
    let mut options = ProfileArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                options.schema = Some(PathBuf::from(take_value(args, &mut i, "--schema")?));
            }
            "--rows" => {
                options.rows = Some(PathBuf::from(take_value(args, &mut i, "--rows")?));
            }
            "--out" => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown {command} flag: {other}"))
                        .with_suggested_command(format!("powerbi-cli {command} <path> --json")),
                );
            }
            other => {
                if options.profile.is_some() {
                    return Err(CliError::invalid_args(format!(
                        "{command} accepts exactly one profile path"
                    ))
                    .with_suggested_command(format!(
                        "powerbi-cli {command} <profile.json> --json"
                    )));
                }
                options.profile = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(options)
}

fn required_profile_path(path: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    path.ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires <profile.json>"))
            .with_suggested_command(format!("powerbi-cli {command} <profile.json> --json"))
    })
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
    *index += 2;
    Ok(value.clone())
}

fn string_field(object: &Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
