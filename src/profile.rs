use crate::input_safety::{InputKind, RowsDocument, read_rows, read_utf8};
use crate::profile_shape::classify_profile;
use crate::safety_scan::{contains_credential_like_text_str, contains_pii_suspect_text};
use crate::schema::{load_schema_value, validate_schema_value};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const PROFILE_V1: &str = "powerbi-cli.dataProfile.v1";
pub(crate) const PROFILE_V2: &str = "powerbi-cli.dataProfile.v2";

const MAX_TOP_VALUES: usize = 5;
const MAX_TOP_VALUE_CHARS: usize = 128;
const MAX_COERCION_DIAGNOSTICS: usize = 20;

#[derive(Debug, Default)]
struct ProfileArgs {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
    rows: Option<PathBuf>,
    out: Option<PathBuf>,
    include_data_values: bool,
    redact: bool,
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
    if options.include_data_values && options.rows.is_none() {
        return Err(CliError::invalid_args(
            "--include-data-values requires --rows <csv|json>; embedded schema rows remain redacted",
        )
        .with_hint(
            "Supply a bounded CSV or JSON rows file and review the PII/credential scan before opting in.",
        )
        .with_suggested_command(format!(
            "powerbi-cli profile infer --schema {} --rows <rows.csv|rows.json> --json",
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
    let (profile, response_schema) = if let Some(rows_path) = &options.rows {
        (
            infer_profile_from_rows(
                &schema,
                &schema_path,
                rows_path,
                options.include_data_values,
            )?,
            "powerbi-cli.profile.infer.v2",
        )
    } else {
        (
            infer_profile(&schema, &schema_path),
            "powerbi-cli.profile.infer.v1",
        )
    };
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
    let rows_suffix = options
        .rows
        .as_ref()
        .map(|rows| format!(" --rows {}", command_arg(rows)))
        .unwrap_or_default();
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
                "powerbi-cli profile infer --schema {}{} --out <profile.json> --json",
                command_arg(&schema_path),
                rows_suffix
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
    let deprecations = if options.redact {
        vec![json!({
            "code": "profile.redact_deprecated",
            "flag": "--redact",
            "message": "--redact is retained as a no-op; profile infer redacts top values by default. Use --include-data-values only for bounded values after the safety scan."
        })]
    } else {
        Vec::new()
    };
    Ok(json!({
        "schema": response_schema,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "schemaPath": canonical_display(&schema_path),
        "profilePath": options.out.as_ref().map(|path| canonical_display(path)),
        "profile": profile,
        "deprecations": deprecations,
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
        "schema": PROFILE_V1,
        "source": {
            "kind": "schema-embedded-dummy-rows",
            "schemaPath": schema_path.to_string_lossy()
        },
        "dataValues": false,
        "tables": tables,
        "relationships": schema
            .get("relationships")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
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

#[derive(Debug)]
struct ExternalRows {
    rows: Vec<Map<String, Value>>,
    headers: Vec<String>,
    format: &'static str,
}

#[derive(Debug)]
struct CoercedValue {
    value: Value,
    numeric: Option<f64>,
    temporal: Option<String>,
    was_coerced: bool,
}

fn infer_profile_from_rows(
    schema: &Value,
    schema_path: &Path,
    rows_path: &Path,
    include_data_values: bool,
) -> CliResult<Value> {
    let bounded = read_rows(rows_path)?;
    let external = rows_document_to_records(bounded.document)?;
    let schema_tables = schema["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect::<Vec<_>>();
    let selected_index = select_rows_table(&schema_tables, &external.headers)?;
    if include_data_values {
        scan_opt_in_columns(schema_tables[selected_index], &external.rows)?;
    }

    let mut tables = Vec::with_capacity(schema_tables.len());
    let mut diagnostics = Vec::new();
    let mut grain_conflicts = Vec::new();
    for (index, table) in schema_tables.iter().enumerate() {
        let rows = if index == selected_index {
            external.rows.as_slice()
        } else {
            &[]
        };
        let (profile, table_diagnostics) = table_profile_v2(table, rows, include_data_values);
        diagnostics.extend(table_diagnostics);
        if let Some(conflicts) = profile["grainConflicts"].as_array() {
            grain_conflicts.extend(conflicts.iter().cloned());
        }
        tables.push(profile);
    }

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
    let mut warnings = profile_warnings(schema);
    if external.rows.is_empty() {
        warnings.push(json!({
            "code": "profile.rows_empty",
            "message": format!("rows input {} contains no data records after the header", rows_path.display())
        }));
    }
    for table in &schema_tables {
        let name = string_field(table, "name").unwrap_or_default();
        if !name.eq_ignore_ascii_case(
            schema_tables
                .get(selected_index)
                .and_then(|selected| string_field(selected, "name"))
                .as_deref()
                .unwrap_or_default(),
        ) {
            warnings.push(json!({
                "code": "profile.table_without_rows",
                "message": format!("table {name} was not matched by the external rows input; metadata-only profile emitted")
            }));
        }
    }

    Ok(json!({
        "schema": PROFILE_V2,
        "source": {
            "kind": "external-rows",
            "format": external.format,
            "schemaPath": schema_path.to_string_lossy(),
            "rowsPath": rows_path.to_string_lossy(),
            "table": tables[selected_index]["name"],
            "rowCount": external.rows.len(),
            "columnCount": external.headers.len()
        },
        "dataValues": include_data_values,
        "tables": tables,
        "relationships": schema
            .get("relationships")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
        "candidates": {
            "factTables": fact_tables,
            "dimensionTables": dimension_tables,
            "dateColumns": date_columns,
            "numericColumns": numeric_columns,
            "categoryColumns": category_columns
        },
        "grainConflicts": grain_conflicts,
        "diagnostics": diagnostics,
        "warnings": warnings
    }))
}

fn rows_document_to_records(document: RowsDocument) -> CliResult<ExternalRows> {
    match document {
        RowsDocument::Csv(rows) => csv_rows_to_records(rows),
        RowsDocument::Json(value) => json_rows_to_records(value),
    }
}

fn csv_rows_to_records(rows: Vec<Vec<String>>) -> CliResult<ExternalRows> {
    let Some(header) = rows.first() else {
        return Ok(ExternalRows {
            rows: Vec::new(),
            headers: Vec::new(),
            format: "csv",
        });
    };
    let headers = header.clone();
    validate_headers(&headers)?;
    let records = rows
        .into_iter()
        .skip(1)
        .map(|fields| {
            let mut record = Map::new();
            for (index, name) in headers.iter().enumerate() {
                let value = fields
                    .get(index)
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| Value::String(value.clone()))
                    .unwrap_or(Value::Null);
                record.insert(name.clone(), value);
            }
            record
        })
        .collect();
    Ok(ExternalRows {
        rows: records,
        headers,
        format: "csv",
    })
}

fn json_rows_to_records(value: Value) -> CliResult<ExternalRows> {
    let values = value
        .as_array()
        .ok_or_else(|| CliError::validation_failed("rows JSON root must be an array"))?;
    let Some(first) = values.first() else {
        return Ok(ExternalRows {
            rows: Vec::new(),
            headers: Vec::new(),
            format: "json",
        });
    };
    if first.is_object() {
        if values.iter().any(|row| !row.is_object()) {
            return Err(CliError::validation_failed(
                "rows JSON cannot mix object and array records",
            ));
        }
        let mut headers = BTreeSet::new();
        for row in values {
            if let Some(object) = row.as_object() {
                headers.extend(object.keys().cloned());
            }
        }
        let headers = headers.into_iter().collect::<Vec<_>>();
        validate_headers(&headers)?;
        let rows = values
            .iter()
            .filter_map(Value::as_object)
            .cloned()
            .collect();
        return Ok(ExternalRows {
            rows,
            headers,
            format: "json",
        });
    }

    if !first.is_array() || values.iter().any(|row| !row.is_array()) {
        return Err(CliError::validation_failed(
            "rows JSON items must be objects or arrays with a header row",
        ));
    }
    let header_values = first.as_array().expect("array header");
    let headers = header_values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                CliError::validation_failed(format!(
                    "rows JSON header item {index} must be a string"
                ))
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    validate_headers(&headers)?;
    let rows = values
        .iter()
        .skip(1)
        .map(|row| {
            let values = row.as_array().expect("array row");
            let mut record = Map::new();
            for (index, name) in headers.iter().enumerate() {
                record.insert(
                    name.clone(),
                    values.get(index).cloned().unwrap_or(Value::Null),
                );
            }
            record
        })
        .collect();
    Ok(ExternalRows {
        rows,
        headers,
        format: "json",
    })
}

fn validate_headers(headers: &[String]) -> CliResult<()> {
    let mut seen = BTreeSet::new();
    for (index, header) in headers.iter().enumerate() {
        if header.trim().is_empty() {
            return Err(CliError::validation_failed(format!(
                "rows header item {index} must not be empty"
            )));
        }
        let canonical = header.to_ascii_lowercase();
        if !seen.insert(canonical) {
            return Err(CliError::validation_failed(format!(
                "rows header contains duplicate column {header}"
            )));
        }
    }
    Ok(())
}

fn select_rows_table(tables: &[&Map<String, Value>], headers: &[String]) -> CliResult<usize> {
    if tables.is_empty() {
        return Err(CliError::validation_failed(
            "schema must contain a table before rows can be profiled",
        ));
    }
    if headers.is_empty() {
        return Ok(0);
    }
    let mut scores = Vec::with_capacity(tables.len());
    for table in tables {
        let columns = table
            .get("columns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .filter_map(|column| string_field(column, "name"))
            .collect::<Vec<_>>();
        let score = headers
            .iter()
            .filter(|header| {
                columns
                    .iter()
                    .any(|column| column.eq_ignore_ascii_case(header))
            })
            .count();
        scores.push(score);
    }
    let best = scores.iter().copied().max().unwrap_or_default();
    if best == 0 {
        return Err(CliError::validation_failed(
            "rows headers do not match any schema table columns",
        )
        .with_hint("Use a header row containing columns from exactly one schema table.")
        .with_suggested_command(
            "powerbi-cli profile infer --schema <schema.json> --rows <rows.csv|rows.json> --json",
        ));
    }
    let matches = scores.iter().filter(|score| **score == best).count();
    if matches != 1 {
        return Err(CliError::validation_failed(
            "rows headers match multiple schema tables equally; refusing to guess the table",
        )
        .with_hint("Supply rows whose headers identify one table unambiguously.")
        .with_suggested_command(
            "powerbi-cli profile infer --schema <schema.json> --rows <rows.csv|rows.json> --json",
        ));
    }
    Ok(scores
        .iter()
        .position(|score| *score == best)
        .expect("best score exists"))
}

fn row_value<'a>(row: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    row.get(name).or_else(|| {
        row.iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}

fn scan_opt_in_columns(table: &Map<String, Value>, rows: &[Map<String, Value>]) -> CliResult<()> {
    let mut refused = Vec::new();
    for column in table
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
    {
        let name = string_field(column, "name").unwrap_or_default();
        let scan_rows = rows
            .iter()
            .map(|row| {
                let mut object = Map::new();
                object.insert(
                    name.clone(),
                    row_value(row, &name).cloned().unwrap_or(Value::Null),
                );
                Value::Object(object)
            })
            .collect::<Vec<_>>();
        let text = json!({"rows": scan_rows}).to_string();
        let credential = contains_credential_like_text_str(&text);
        let pii = contains_pii_suspect_text(&text);
        if credential || pii {
            let reason = match (credential, pii) {
                (true, true) => "credential and PII",
                (true, false) => "credential",
                (false, true) => "PII",
                (false, false) => unreachable!("scan flags are non-empty"),
            };
            refused.push(format!("{name} ({reason} scan)"));
        }
    }
    if refused.is_empty() {
        Ok(())
    } else {
        Err(CliError::validation_failed(format!(
            "--include-data-values is refused for columns flagged by the credential/PII scan: {}",
            refused.join(", ")
        ))
        .with_hint(
            "Remove or anonymize flagged values, rerun without --include-data-values, or keep the default redacted profile.",
        )
        .with_suggested_command(
            "powerbi-cli profile infer --schema <schema.json> --rows <rows.csv|rows.json> --json",
        ))
    }
}

fn table_profile_v2(
    table: &Map<String, Value>,
    rows: &[Map<String, Value>],
    include_data_values: bool,
) -> (Value, Vec<Value>) {
    let name = string_field(table, "name").unwrap_or_default();
    let mut diagnostics = Vec::new();
    let columns = table
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .map(|column| {
            let (profile, column_diagnostics) =
                column_profile_v2(&name, column, rows, include_data_values);
            diagnostics.extend(column_diagnostics);
            profile
        })
        .collect::<Vec<_>>();
    let numeric_count = columns
        .iter()
        .filter(|column| column["roles"]["numeric"].as_bool() == Some(true))
        .count();
    let key_count = columns
        .iter()
        .filter(|column| column["isKey"].as_bool() == Some(true))
        .count();
    let role = table_role(&name, key_count, numeric_count, rows.len());
    let grain_conflicts = grain_conflicts(table, rows);
    (
        json!({
            "name": name,
            "role": role,
            "rowCount": rows.len(),
            "columns": columns,
            "grainConflicts": grain_conflicts
        }),
        diagnostics,
    )
}

fn column_profile_v2(
    table_name: &str,
    column: &Map<String, Value>,
    rows: &[Map<String, Value>],
    include_data_values: bool,
) -> (Value, Vec<Value>) {
    let name = string_field(column, "name").unwrap_or_default();
    let data_type = string_field(column, "dataType").unwrap_or_else(|| "string".to_string());
    let date_like = is_date_like(&name, &data_type);
    let numeric = is_numeric_type(&data_type);
    let is_key = column
        .get("isKey")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let categorical = is_categorical(&name, &data_type, is_key);
    let mut null_count = 0usize;
    let mut distinct = BTreeMap::<String, (Value, usize)>::new();
    let mut min_numeric: Option<(f64, Value)> = None;
    let mut max_numeric: Option<(f64, Value)> = None;
    let mut min_temporal: Option<String> = None;
    let mut max_temporal: Option<String> = None;
    let mut temporal_count = 0usize;
    let mut coerced_count = 0usize;
    let mut failed_count = 0usize;
    let mut diagnostics = Vec::new();

    for (row_index, row) in rows.iter().enumerate() {
        let Some(raw) = row_value(row, &name) else {
            null_count += 1;
            continue;
        };
        if is_null_like(raw) {
            null_count += 1;
            continue;
        }
        match coerce_value(raw, &data_type, &name) {
            Ok(coerced) => {
                if coerced.was_coerced {
                    coerced_count += 1;
                }
                let key = render_value(&coerced.value);
                distinct
                    .entry(key)
                    .and_modify(|(_, count)| *count += 1)
                    .or_insert((coerced.value.clone(), 1));
                if let Some(number) = coerced.numeric {
                    if min_numeric
                        .as_ref()
                        .is_none_or(|(current, _)| number < *current)
                    {
                        min_numeric = Some((number, coerced.value.clone()));
                    }
                    if max_numeric
                        .as_ref()
                        .is_none_or(|(current, _)| number > *current)
                    {
                        max_numeric = Some((number, coerced.value.clone()));
                    }
                }
                if let Some(temporal) = coerced.temporal {
                    temporal_count += 1;
                    if min_temporal
                        .as_ref()
                        .is_none_or(|current| temporal < *current)
                    {
                        min_temporal = Some(temporal.clone());
                    }
                    if max_temporal
                        .as_ref()
                        .is_none_or(|current| temporal > *current)
                    {
                        max_temporal = Some(temporal);
                    }
                }
            }
            Err(reason) => {
                failed_count += 1;
                if diagnostics.len() < MAX_COERCION_DIAGNOSTICS {
                    diagnostics.push(json!({
                        "code": "profile.type_coercion_failed",
                        "severity": "warning",
                        "table": table_name,
                        "column": name,
                        "rowIndex": row_index,
                        "row": row_index + 1,
                        "expectedType": data_type,
                        "observedType": observed_value_kind(raw),
                        "reason": reason
                    }));
                }
            }
        }
    }
    if coerced_count > 0 {
        diagnostics.insert(
            0,
            json!({
                "code": "profile.type_coercion",
                "severity": "info",
                "table": table_name,
                "column": name,
                "expectedType": data_type,
                "coercedCount": coerced_count
            }),
        );
    }
    let mut top_entries = distinct.into_iter().collect::<Vec<_>>();
    top_entries.sort_by(|(left, (_, left_count)), (right, (_, right_count))| {
        right_count.cmp(left_count).then_with(|| left.cmp(right))
    });
    let top_values = top_entries
        .iter()
        .take(MAX_TOP_VALUES)
        .map(|(_, (value, count))| {
            if include_data_values {
                json!({
                    "value": bounded_value(value),
                    "count": count,
                    "redacted": false
                })
            } else {
                json!({
                    "value": "[REDACTED]",
                    "count": count,
                    "redacted": true
                })
            }
        })
        .collect::<Vec<_>>();
    let top_value_counts = top_entries
        .iter()
        .take(MAX_TOP_VALUES)
        .map(|(_, (_, count))| json!({"count": count}))
        .collect::<Vec<_>>();
    let min = if date_like {
        min_temporal
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null)
    } else if numeric {
        min_numeric
            .as_ref()
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let max = if date_like {
        max_temporal
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null)
    } else if numeric {
        max_numeric
            .as_ref()
            .map(|(_, value)| value.clone())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let time_coverage = match (min_temporal, max_temporal) {
        (Some(minimum), Some(maximum)) => json!({
            "min": minimum.clone(),
            "max": maximum.clone(),
            "start": minimum,
            "end": maximum,
            "count": temporal_count
        }),
        _ => Value::Null,
    };
    let null_rate = if rows.is_empty() {
        0.0
    } else {
        null_count as f64 / rows.len() as f64
    };
    let profile = json!({
        "name": name,
        "dataType": data_type,
        "isKey": is_key,
        "nullCount": null_count,
        "nullRate": null_rate,
        "distinctCount": top_entries.len(),
        "min": min,
        "max": max,
        "topValues": top_values,
        "topValueCounts": top_value_counts,
        "valuesRedacted": !include_data_values,
        "timeCoverage": time_coverage,
        "typeCoercion": {
            "coercedCount": coerced_count,
            "failedCount": failed_count,
            "diagnostics": diagnostics.clone()
        },
        "coercionDiagnostics": diagnostics.clone(),
        "roles": {
            "dateLike": date_like,
            "numeric": numeric,
            "categorical": categorical
        }
    });
    (profile, diagnostics)
}

fn grain_conflicts(table: &Map<String, Value>, rows: &[Map<String, Value>]) -> Vec<Value> {
    let key_columns = table
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|column| column.get("isKey").and_then(Value::as_bool) == Some(true))
        .filter_map(|column| {
            let name = string_field(column, "name")?;
            let data_type =
                string_field(column, "dataType").unwrap_or_else(|| "string".to_string());
            Some((name, data_type))
        })
        .collect::<Vec<_>>();
    if key_columns.is_empty() {
        return Vec::new();
    }
    let mut seen = BTreeMap::<String, usize>::new();
    let mut duplicate_rows = 0usize;
    for row in rows {
        let Some(key) = key_columns
            .iter()
            .map(|(column, data_type)| {
                row_value(row, column)
                    .filter(|value| !is_null_like(value))
                    .and_then(|value| coerce_value(value, data_type, column).ok())
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let rendered = key
            .iter()
            .map(|value| render_value(&value.value))
            .collect::<Vec<_>>()
            .join("\u{1f}");
        let count = seen.entry(rendered).or_default();
        if *count > 0 {
            duplicate_rows += 1;
        }
        *count += 1;
    }
    let duplicate_keys = seen.values().filter(|count| **count > 1).count();
    if duplicate_keys == 0 {
        Vec::new()
    } else {
        vec![json!({
            "code": "profile.grain_conflict",
            "table": string_field(table, "name").unwrap_or_default(),
            "columns": key_columns
                .iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            "duplicateKeyCount": duplicate_keys,
            "duplicateRowCount": duplicate_rows,
            "rowCount": rows.len(),
            "message": "key columns contain duplicate values; declared table grain is not unique"
        })]
    }
}

fn is_null_like(value: &Value) -> bool {
    value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty())
}

fn observed_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn coerce_value(raw: &Value, data_type: &str, column_name: &str) -> Result<CoercedValue, String> {
    let lower_type = data_type.to_ascii_lowercase();
    let was_string = raw.is_string();
    let value = if is_numeric_type(data_type) {
        let number = match raw {
            Value::Number(number) => number
                .as_f64()
                .ok_or_else(|| "number is not finite".to_string())?,
            Value::String(text) => {
                parse_number(text).ok_or_else(|| "value is not a number".to_string())?
            }
            _ => {
                return Err(format!(
                    "expected numeric input, observed {}",
                    observed_value_kind(raw)
                ));
            }
        };
        if is_integer_type(data_type) && number.fract().abs() > f64::EPSILON {
            return Err("fractional value cannot be coerced to an integer".to_string());
        }
        if is_integer_type(data_type) {
            let integer = number as i128;
            if integer < i64::MIN as i128 || integer > i64::MAX as i128 {
                return Err("integer value is outside the supported range".to_string());
            }
            Value::Number(serde_json::Number::from(integer as i64))
        } else {
            serde_json::Number::from_f64(number)
                .map(Value::Number)
                .ok_or_else(|| "number is not finite".to_string())?
        }
    } else if is_boolean_type(data_type) {
        match raw {
            Value::Bool(value) => Value::Bool(*value),
            Value::Number(number) => match number.as_i64() {
                Some(0) => Value::Bool(false),
                Some(1) => Value::Bool(true),
                _ => return Err("boolean numbers must be 0 or 1".to_string()),
            },
            Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "y" | "1" => Value::Bool(true),
                "false" | "no" | "n" | "0" => Value::Bool(false),
                _ => return Err("value is not a boolean".to_string()),
            },
            _ => {
                return Err(format!(
                    "expected boolean input, observed {}",
                    observed_value_kind(raw)
                ));
            }
        }
    } else if lower_type.contains("date") || lower_type.contains("time") {
        let text = match raw {
            Value::String(text) => text.trim().to_string(),
            Value::Number(number) if is_date_like(column_name, data_type) => number.to_string(),
            _ => {
                return Err(format!(
                    "expected date/time input, observed {}",
                    observed_value_kind(raw)
                ));
            }
        };
        if temporal_string(&text, column_name).is_none() {
            return Err("value is not an ISO date/time or supported year/date key".to_string());
        }
        Value::String(text)
    } else if matches!(lower_type.as_str(), "string" | "text" | "varchar") {
        match raw {
            Value::String(text) => Value::String(text.clone()),
            Value::Bool(value) => Value::String(value.to_string()),
            Value::Number(value) => Value::String(value.to_string()),
            _ => {
                return Err(format!(
                    "expected scalar text, observed {}",
                    observed_value_kind(raw)
                ));
            }
        }
    } else {
        match raw {
            Value::String(text) => Value::String(text.clone()),
            Value::Bool(value) => Value::String(value.to_string()),
            Value::Number(value) => Value::String(value.to_string()),
            _ => {
                return Err(format!(
                    "expected scalar input, observed {}",
                    observed_value_kind(raw)
                ));
            }
        }
    };
    let numeric = value.as_f64();
    let temporal = if is_date_like(column_name, data_type) {
        temporal_string(&render_value(&value), column_name)
    } else {
        None
    };
    Ok(CoercedValue {
        value,
        numeric,
        temporal,
        was_coerced: was_string && !(matches!(lower_type.as_str(), "string" | "text" | "varchar")),
    })
}

fn parse_number(text: &str) -> Option<f64> {
    let normalized = text.trim().replace(',', "");
    if normalized.is_empty() {
        return None;
    }
    let value = normalized.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn bounded_value(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        let bounded = text.chars().take(MAX_TOP_VALUE_CHARS).collect::<String>();
        if bounded.chars().count() < text.chars().count() {
            return Value::String(format!("{bounded}…"));
        }
    }
    value.clone()
}

fn is_integer_type(data_type: &str) -> bool {
    matches!(
        data_type.to_ascii_lowercase().as_str(),
        "int" | "integer" | "whole" | "whole_number" | "int8" | "int16" | "int32" | "int64"
    )
}

fn is_numeric_type(data_type: &str) -> bool {
    matches!(
        data_type.to_ascii_lowercase().as_str(),
        "int"
            | "integer"
            | "whole"
            | "whole_number"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "double"
            | "float"
            | "number"
            | "decimal"
            | "fixed_decimal"
            | "currency"
    )
}

fn is_boolean_type(data_type: &str) -> bool {
    matches!(
        data_type.to_ascii_lowercase().as_str(),
        "bool" | "boolean" | "logical"
    )
}

fn is_date_like(name: &str, data_type: &str) -> bool {
    let lower_name = name.to_ascii_lowercase();
    let lower_type = data_type.to_ascii_lowercase();
    lower_type.contains("date")
        || lower_type.contains("time")
        || lower_name.contains("date")
        || lower_name.contains("datum")
        || lower_name.contains("year")
        || lower_name.contains("jahr")
        || lower_name.contains("month")
        || lower_name.contains("monat")
}

fn temporal_string(value: &str, column_name: &str) -> Option<String> {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    let lower_name = column_name.to_ascii_lowercase();
    if bytes.len() >= 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        return Some(trimmed.to_string());
    }
    if bytes.len() == 4 && bytes.iter().all(u8::is_ascii_digit) && lower_name.contains("year") {
        return Some(trimmed.to_string());
    }
    if bytes.len() == 8
        && bytes.iter().all(u8::is_ascii_digit)
        && (lower_name.contains("date") || lower_name.contains("datum"))
    {
        return Some(format!(
            "{}-{}-{}",
            &trimmed[..4],
            &trimmed[4..6],
            &trimmed[6..8]
        ));
    }
    None
}

fn is_categorical(name: &str, data_type: &str, is_key: bool) -> bool {
    let lower_name = name.to_ascii_lowercase();
    let lower_type = data_type.to_ascii_lowercase();
    !is_key
        && (matches!(
            lower_type.as_str(),
            "string" | "text" | "varchar" | "boolean" | "bool" | "logical"
        ) || lower_name.contains("branch")
            || lower_name.contains("category")
            || lower_name.contains("segment")
            || lower_name.contains("status")
            || lower_name.contains("type")
            || lower_name.contains("group"))
}

fn table_role(name: &str, key_count: usize, numeric_count: usize, rows: usize) -> &'static str {
    let lower_name = name.to_ascii_lowercase();
    if lower_name.starts_with("fact") {
        "fact"
    } else if lower_name.starts_with("dim") || key_count > 0 {
        "dimension"
    } else if numeric_count >= 2 && rows > 0 {
        "fact"
    } else {
        "unknown"
    }
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
    let mut sample_value_count = 0usize;
    if let Some(rows) = rows {
        for row in rows.iter().filter_map(Value::as_object) {
            match row.get(&name) {
                None | Some(Value::Null) => null_count += 1,
                Some(value) => {
                    let rendered = render_value(value);
                    distinct.insert(rendered.clone());
                    sample_value_count = distinct.len();
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
        "nullRate": rows
            .map(|values| {
                if values.is_empty() {
                    0.0
                } else {
                    null_count as f64 / values.len() as f64
                }
            })
            .unwrap_or(0.0),
        "distinctCount": distinct.len(),
        "sampleValues": if sample_value_count > 0 {
            vec![Value::String("[REDACTED]".to_string())]
        } else {
            Vec::new()
        },
        "sampleValueCount": sample_value_count,
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
    let schema = profile["schema"].as_str();
    if !matches!(schema, Some(PROFILE_V1) | Some(PROFILE_V2)) {
        errors.push(format!(
            "profile schema must be {PROFILE_V1} or {PROFILE_V2}"
        ));
    }
    if profile["tables"]
        .as_array()
        .is_none_or(|tables| tables.is_empty())
    {
        errors.push("profile must contain a non-empty tables array".to_string());
    }
    if schema == Some(PROFILE_V2) {
        validate_v2_profile(profile, &mut errors);
    }
    errors
}

fn validate_v2_profile(profile: &Value, errors: &mut Vec<String>) {
    let data_values = match profile["dataValues"].as_bool() {
        Some(data_values) => data_values,
        None => {
            errors.push("profile.dataValues must be a boolean".to_string());
            false
        }
    };
    let Some(tables) = profile["tables"].as_array() else {
        return;
    };
    for (table_index, table) in tables.iter().enumerate() {
        let Some(columns) = table["columns"].as_array() else {
            errors.push(format!("tables[{table_index}].columns must be an array"));
            continue;
        };
        for (column_index, column) in columns.iter().enumerate() {
            let path = format!("tables[{table_index}].columns[{column_index}]");
            let null_rate = column["nullRate"].as_f64();
            if null_rate.is_none_or(|rate| !(0.0..=1.0).contains(&rate)) {
                errors.push(format!("{path}.nullRate must be a number between 0 and 1"));
            }
            if column["distinctCount"].as_u64().is_none() {
                errors.push(format!(
                    "{path}.distinctCount must be a non-negative integer"
                ));
            }
            let Some(top_values) = column["topValues"].as_array() else {
                errors.push(format!("{path}.topValues must be an array"));
                continue;
            };
            if top_values.len() > MAX_TOP_VALUES {
                errors.push(format!(
                    "{path}.topValues may contain at most {MAX_TOP_VALUES} entries"
                ));
            }
            for (value_index, top) in top_values.iter().enumerate() {
                if top["count"].as_u64().is_none() {
                    errors.push(format!(
                        "{path}.topValues[{value_index}].count must be a non-negative integer"
                    ));
                }
                if !data_values && top["value"].as_str() != Some("[REDACTED]") {
                    errors.push(format!(
                        "{path}.topValues[{value_index}] must remain redacted when dataValues is false"
                    ));
                }
            }
        }
    }
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
        "schema": profile["schema"],
        "dataValues": profile["dataValues"].as_bool().unwrap_or(false),
        "tables": tables.len(),
        "columns": tables
            .iter()
            .map(|table| table["columns"].as_array().map_or(0, Vec::len))
            .sum::<usize>(),
        "tableRoles": roles,
        "candidateFactTables": profile["candidates"]["factTables"].as_array().map_or(0, Vec::len),
        "candidateDateColumns": profile["candidates"]["dateColumns"].as_array().map_or(0, Vec::len),
        "candidateNumericColumns": profile["candidates"]["numericColumns"].as_array().map_or(0, Vec::len),
        "candidateCategoryColumns": profile["candidates"]["categoryColumns"].as_array().map_or(0, Vec::len),
        "grainConflicts": profile["grainConflicts"].as_array().map_or(0, Vec::len),
        "diagnostics": profile["diagnostics"].as_array().map_or(0, Vec::len),
        "shape": classify_profile(profile).into_value()
    })
}

pub(crate) fn profile_is_data_bearing(profile: &Value) -> bool {
    profile["schema"].as_str() == Some(PROFILE_V2) && profile["dataValues"].as_bool() == Some(true)
}

pub(crate) fn text_is_data_bearing_profile(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|profile| profile_is_data_bearing(&profile))
}

pub(crate) fn load_profile_value(path: &Path) -> CliResult<Value> {
    let text = read_utf8(path, InputKind::Profile)?;
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
            "--include-data-values" => {
                options.include_data_values = true;
                i += 1;
            }
            "--redact" => {
                options.redact = true;
                i += 1;
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
