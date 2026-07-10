use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub(crate) struct SchemaCounts {
    pub(crate) tables: usize,
    pub(crate) columns: usize,
    pub(crate) measures: usize,
    pub(crate) relationships: usize,
    pub(crate) pages: usize,
    pub(crate) visuals: usize,
    pub(crate) bindings: usize,
    pub(crate) rows: usize,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct SchemaValidation {
    pub(crate) errors: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) counts: SchemaCounts,
    pub(crate) tables: Vec<Value>,
}

#[derive(Debug, Default)]
struct SchemaArgs {
    path: Option<PathBuf>,
    out: Option<PathBuf>,
}

pub(crate) fn schema_command(args: &[String]) -> CliResult<Value> {
    match args {
        [action, rest @ ..] if action == "validate" => validate_command(rest),
        [action, rest @ ..] if action == "normalize" => normalize_command(rest),
        [] => Err(
            CliError::invalid_args("schema requires a subcommand: validate or normalize")
                .with_hint("Run `powerbi-cli schema validate <schema.json> --json`.")
                .with_suggested_command("powerbi-cli schema validate <schema.json> --json"),
        ),
        _ => Err(CliError::invalid_args("unknown schema command")
            .with_hint("Run `powerbi-cli --json capabilities --for schema`.")
            .with_suggested_command("powerbi-cli --json capabilities --for schema")),
    }
}

fn validate_command(args: &[String]) -> CliResult<Value> {
    let options = parse_schema_args(args, "schema validate", false)?;
    let path = required_path(options.path, "schema validate")?;
    let value = load_schema_value(&path)?;
    let report = validate_schema_value(&value);
    Ok(validation_json(
        "powerbi-cli.schema.validate.v1",
        &path,
        &report,
        None,
    ))
}

fn normalize_command(args: &[String]) -> CliResult<Value> {
    let options = parse_schema_args(args, "schema normalize", true)?;
    let path = required_path(options.path, "schema normalize")?;
    let out = options.out.ok_or_else(|| {
        CliError::invalid_args("schema normalize requires --out <canonical.json>")
            .with_hint(
                "Run `powerbi-cli schema normalize <schema.json> --out <canonical.json> --json`.",
            )
            .with_suggested_command(
                "powerbi-cli schema normalize <schema.json> --out <canonical.json> --json",
            )
    })?;
    let value = load_schema_value(&path)?;
    let report = validate_schema_value(&value);
    if !report.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "schema is not valid: {}",
            report.errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli schema validate {} --json",
            command_arg(&path)
        )));
    }
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|err| {
            CliError::unexpected(format!(
                "create output directory {}: {err}",
                parent.display()
            ))
        })?;
    }
    fs::write(
        &out,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&value).expect("serialize normalized schema")
        ),
    )
    .map_err(|err| CliError::unexpected(format!("write {}: {err}", out.display())))?;
    Ok(validation_json(
        "powerbi-cli.schema.normalize.v1",
        &path,
        &report,
        Some(&out),
    ))
}

fn validation_json(
    schema: &str,
    path: &Path,
    report: &SchemaValidation,
    normalized_out: Option<&Path>,
) -> Value {
    let ok = report.errors.is_empty();
    let mut next = if ok {
        vec![
            format!("powerbi-cli schema validate {} --json", command_arg(path)),
            format!(
                "powerbi-cli profile infer --schema {} --out <profile.json> --json",
                command_arg(path)
            ),
            "powerbi-cli profile validate <profile.json> --json".to_string(),
            format!(
                "powerbi-cli report spec validate --schema {} --profile <profile.json> --spec <dashboard.json> --json",
                command_arg(path)
            ),
            format!(
                "powerbi-cli report build --schema {} --profile <profile.json> --spec <dashboard.json> --out-dir <project-dir> --json",
                command_arg(path)
            ),
        ]
    } else {
        vec![format!(
            "powerbi-cli schema validate {} --json",
            command_arg(path)
        )]
    };
    if let Some(out) = normalized_out {
        next.insert(
            1,
            format!("powerbi-cli schema validate {} --json", command_arg(out)),
        );
    }
    json!({
        "schema": schema,
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "schemaPath": canonical_display(path),
        "normalizedOut": normalized_out.map(canonical_display),
        "counts": counts_json(&report.counts),
        "tables": report.tables,
        "warnings": report.warnings,
        "errors": report.errors,
        "next": next
    })
}

pub(crate) fn load_schema_value(path: &Path) -> CliResult<Value> {
    let text = fs::read_to_string(path).map_err(|err| {
        CliError::file_not_found(format!("read schema {}: {err}", path.display()))
    })?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::invalid_args(format!("parse schema {}: {err}", path.display())))
}

pub(crate) fn validate_schema_value(value: &Value) -> SchemaValidation {
    let mut report = SchemaValidation::default();
    let Some(object) = value.as_object() else {
        report
            .errors
            .push("schema root must be a JSON object".to_string());
        return report;
    };
    if string_field(object, "name").is_none_or(|name| name.trim().is_empty()) {
        report
            .errors
            .push("schema name must not be empty".to_string());
    }

    let mut table_columns: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut table_measures: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let Some(tables) = object.get("tables").and_then(Value::as_array) else {
        report
            .errors
            .push("schema must contain a tables array".to_string());
        return report;
    };
    if tables.is_empty() {
        report
            .errors
            .push("schema must contain at least one table".to_string());
    }

    let mut table_names = BTreeSet::new();
    for (table_index, table) in tables.iter().enumerate() {
        let Some(table_object) = table.as_object() else {
            report
                .errors
                .push(format!("tables[{table_index}] must be an object"));
            continue;
        };
        let table_name = string_field(table_object, "name").unwrap_or_default();
        if table_name.trim().is_empty() {
            report
                .errors
                .push(format!("tables[{table_index}].name must not be empty"));
            continue;
        }
        let table_key = table_name.to_ascii_lowercase();
        if !table_names.insert(table_key.clone()) {
            report
                .errors
                .push(format!("duplicate table name: {table_name}"));
        }
        report.counts.tables += 1;
        let mut columns = BTreeSet::new();
        let mut measures = BTreeSet::new();
        let column_values = table_object
            .get("columns")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if column_values.is_empty() {
            report.errors.push(format!(
                "table {table_name} must contain at least one column"
            ));
        }
        for (column_index, column) in column_values.iter().enumerate() {
            let Some(column_object) = column.as_object() else {
                report.errors.push(format!(
                    "table {table_name} columns[{column_index}] must be an object"
                ));
                continue;
            };
            let column_name = string_field(column_object, "name").unwrap_or_default();
            if column_name.trim().is_empty() {
                report.errors.push(format!(
                    "table {table_name} columns[{column_index}].name must not be empty"
                ));
                continue;
            }
            if !columns.insert(column_name.to_ascii_lowercase()) {
                report.errors.push(format!(
                    "duplicate column {column_name} in table {table_name}"
                ));
            }
            if let Some(kind) = string_field(column_object, "dataType")
                && !is_supported_data_type(&kind)
            {
                report.errors.push(format!(
                    "unsupported dataType {kind} for column {table_name}.{column_name}"
                ));
            }
            report.counts.columns += 1;
        }
        for (measure_index, measure) in table_object
            .get("measures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(measure_object) = measure.as_object() else {
                report.errors.push(format!(
                    "table {table_name} measures[{measure_index}] must be an object"
                ));
                continue;
            };
            let measure_name = string_field(measure_object, "name").unwrap_or_default();
            if measure_name.trim().is_empty() {
                report.errors.push(format!(
                    "table {table_name} measures[{measure_index}].name must not be empty"
                ));
                continue;
            }
            if string_field(measure_object, "expression").is_none_or(|expr| expr.trim().is_empty())
            {
                report.errors.push(format!(
                    "measure {table_name}.{measure_name} requires expression"
                ));
            }
            if !measures.insert(measure_name.to_ascii_lowercase()) {
                report.errors.push(format!(
                    "duplicate measure {measure_name} in table {table_name}"
                ));
            }
            report.counts.measures += 1;
        }
        let row_count = table_object
            .get("rows")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        report.counts.rows += row_count;
        report.tables.push(json!({
            "name": table_name,
            "columns": column_values.len(),
            "measures": measures.len(),
            "rows": row_count
        }));
        table_columns.insert(table_key.clone(), columns);
        table_measures.insert(table_key, measures);
    }

    validate_relationships(object, &table_columns, &mut report);
    validate_pages(object, &table_columns, &table_measures, &mut report);
    report
}

fn validate_relationships(
    object: &Map<String, Value>,
    table_columns: &BTreeMap<String, BTreeSet<String>>,
    report: &mut SchemaValidation,
) {
    for (index, relationship) in object
        .get("relationships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        report.counts.relationships += 1;
        let Some(relationship) = relationship.as_object() else {
            report
                .errors
                .push(format!("relationships[{index}] must be an object"));
            continue;
        };
        require_endpoint(
            relationship,
            "fromTable",
            "fromColumn",
            index,
            table_columns,
            report,
        );
        require_endpoint(
            relationship,
            "toTable",
            "toColumn",
            index,
            table_columns,
            report,
        );
    }
}

fn require_endpoint(
    relationship: &Map<String, Value>,
    table_field: &str,
    column_field: &str,
    index: usize,
    table_columns: &BTreeMap<String, BTreeSet<String>>,
    report: &mut SchemaValidation,
) {
    let table = string_field(relationship, table_field).unwrap_or_default();
    let column = string_field(relationship, column_field).unwrap_or_default();
    if !has_column(table_columns, &table, &column) {
        report.errors.push(format!(
            "relationships[{index}] references missing endpoint {table}.{column}"
        ));
    }
}

fn validate_pages(
    object: &Map<String, Value>,
    table_columns: &BTreeMap<String, BTreeSet<String>>,
    table_measures: &BTreeMap<String, BTreeSet<String>>,
    report: &mut SchemaValidation,
) {
    for (page_index, page) in object
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        report.counts.pages += 1;
        let Some(page) = page.as_object() else {
            report
                .errors
                .push(format!("pages[{page_index}] must be an object"));
            continue;
        };
        for (visual_index, visual) in page
            .get("visuals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            report.counts.visuals += 1;
            let Some(visual) = visual.as_object() else {
                report.errors.push(format!(
                    "pages[{page_index}].visuals[{visual_index}] must be an object"
                ));
                continue;
            };
            validate_visual_bindings(
                page_index,
                visual_index,
                visual,
                table_columns,
                table_measures,
                report,
            );
        }
    }
}

fn validate_visual_bindings(
    page_index: usize,
    visual_index: usize,
    visual: &Map<String, Value>,
    table_columns: &BTreeMap<String, BTreeSet<String>>,
    table_measures: &BTreeMap<String, BTreeSet<String>>,
    report: &mut SchemaValidation,
) {
    for (binding_index, binding) in visual
        .get("bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        report.counts.bindings += 1;
        let Some(binding) = binding.as_object() else {
            report.errors.push(format!(
                "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}] must be an object"
            ));
            continue;
        };
        let table = string_field(binding, "table").unwrap_or_default();
        let role = string_field(binding, "role").unwrap_or_default();
        if role.trim().is_empty() {
            report.errors.push(format!(
                "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}] requires role"
            ));
        }
        let column = string_field(binding, "column");
        let measure = string_field(binding, "measure");
        match (column, measure) {
            (Some(column), None) if !has_column(table_columns, &table, &column) => {
                report.errors.push(format!(
                    "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}] references missing column {table}.{column}"
                ));
            }
            (None, Some(measure)) if !has_measure(table_measures, &table, &measure) => {
                report.errors.push(format!(
                    "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}] references missing measure {table}.{measure}"
                ));
            }
            (Some(_), Some(_)) => report.errors.push(format!(
                "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}] must not set both column and measure"
            )),
            (None, None) => report.errors.push(format!(
                "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}] requires column or measure"
            )),
            _ => {}
        }
    }
}

pub(crate) fn merge_schema_and_spec(
    schema: Value,
    spec: Option<&Value>,
) -> CliResult<(Value, Vec<String>)> {
    let mut merged = schema;
    let mut notes = Vec::new();
    let Some(spec) = spec else {
        notes.push(
            "no external dashboard spec supplied; using pages embedded in schema manifest"
                .to_string(),
        );
        return Ok((merged, notes));
    };
    let Some(merged_object) = merged.as_object_mut() else {
        return Err(CliError::invalid_args("schema root must be a JSON object"));
    };
    let Some(spec_object) = spec.as_object() else {
        return Err(CliError::invalid_args(
            "dashboard spec root must be a JSON object",
        ));
    };
    for key in ["name", "displayName", "description", "locale", "pages"] {
        if let Some(value) = spec_object.get(key) {
            merged_object.insert(key.to_string(), value.clone());
            notes.push(format!("dashboard spec overrode `{key}`"));
        }
    }
    if spec_object.get("tables").is_some() {
        return Err(CliError::unsupported_feature(
            "report build dashboard specs must not redefine tables; put model tables in --schema",
        )
        .with_suggested_command("powerbi-cli schema validate <schema.json> --json"));
    }
    if spec_object.get("relationships").is_some() {
        return Err(CliError::unsupported_feature(
            "report build dashboard specs must not redefine relationships in this slice; put relationships in --schema"
        )
        .with_suggested_command("powerbi-cli schema validate <schema.json> --json"));
    }
    Ok((merged, notes))
}

fn parse_schema_args(args: &[String], command: &str, allow_out: bool) -> CliResult<SchemaArgs> {
    let mut options = SchemaArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                options.path = Some(PathBuf::from(take_value(args, &mut i, "--schema")?));
            }
            "--out" if allow_out => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            "--out-dir" if allow_out => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out-dir")?));
            }
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown {command} flag: {other}"))
                        .with_suggested_command(format!(
                            "powerbi-cli {command} <schema.json> --json"
                        )),
                );
            }
            other => {
                if options.path.is_some() {
                    return Err(CliError::invalid_args(format!(
                        "{command} accepts exactly one schema path"
                    ))
                    .with_suggested_command(format!(
                        "powerbi-cli {command} <schema.json> --json"
                    )));
                }
                options.path = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(options)
}

fn required_path(path: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    path.ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires <schema.json>"))
            .with_suggested_command(format!("powerbi-cli {command} <schema.json> --json"))
    })
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
    *index += 2;
    Ok(value.clone())
}

fn counts_json(counts: &SchemaCounts) -> Value {
    json!({
        "tables": counts.tables,
        "columns": counts.columns,
        "measures": counts.measures,
        "relationships": counts.relationships,
        "pages": counts.pages,
        "visuals": counts.visuals,
        "bindings": counts.bindings,
        "rows": counts.rows
    })
}

fn string_field(object: &Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn has_column(
    table_columns: &BTreeMap<String, BTreeSet<String>>,
    table: &str,
    column: &str,
) -> bool {
    table_columns
        .get(&table.to_ascii_lowercase())
        .is_some_and(|columns| columns.contains(&column.to_ascii_lowercase()))
}

fn has_measure(
    table_measures: &BTreeMap<String, BTreeSet<String>>,
    table: &str,
    measure: &str,
) -> bool {
    table_measures
        .get(&table.to_ascii_lowercase())
        .is_some_and(|measures| measures.contains(&measure.to_ascii_lowercase()))
}

fn is_supported_data_type(kind: &str) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "" | "text"
            | "string"
            | "int"
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
            | "date"
            | "datetime"
            | "date_time"
            | "bool"
            | "boolean"
            | "logical"
    )
}
