use crate::profile::{load_profile_value, profile_summary, validate_profile_value};
use crate::schema::{load_schema_value, validate_schema_value};
use crate::visual_catalog::supported_visual_type_names;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default)]
struct FieldsOptions {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
}

pub(crate) fn fields_command(args: &[String]) -> CliResult<Value> {
    let options = parse_fields_args(args)?;
    let schema_path = options.schema.ok_or_else(|| {
        CliError::invalid_args("report spec fields requires --schema <schema.json>")
            .with_suggested_command("powerbi-cli report spec fields --schema <schema.json> --json")
    })?;
    let schema = load_schema_value(&schema_path)?;
    let schema_validation = validate_schema_value(&schema);
    if !schema_validation.errors.is_empty() {
        return Ok(json!({
            "schema": "powerbi-cli.report.spec.fields.v1",
            "ok": false,
            "exitCode": EXIT_VALIDATION_FAILED,
            "schemaPath": canonical_display(&schema_path),
            "errors": schema_validation.errors,
            "warnings": schema_validation.warnings,
            "tables": [],
            "next": [
                format!("powerbi-cli schema validate {} --json", command_arg(&schema_path))
            ]
        }));
    }

    let profile = match options.profile.as_deref() {
        Some(path) => {
            let value = load_profile_value(path)?;
            let errors = validate_profile_value(&value);
            if !errors.is_empty() {
                return Ok(json!({
                    "schema": "powerbi-cli.report.spec.fields.v1",
                    "ok": false,
                    "exitCode": EXIT_VALIDATION_FAILED,
                    "schemaPath": canonical_display(&schema_path),
                    "profilePath": canonical_display(path),
                    "errors": errors,
                    "warnings": [],
                    "tables": [],
                    "next": [
                        format!("powerbi-cli profile validate {} --json", command_arg(path))
                    ]
                }));
            }
            Some(value)
        }
        None => None,
    };

    let profile_lookup = profile.as_ref().map(ProfileLookup::from_profile);
    let tables = schema["tables"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|table| table_fields_json(table, profile_lookup.as_ref()))
        .collect::<Vec<_>>();
    let fields = tables
        .iter()
        .flat_map(|table| {
            table["columns"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(table["measures"].as_array().into_iter().flatten())
        })
        .map(|field| {
            json!({
                "reference": field["reference"],
                "kind": field["kind"],
                "table": field["table"],
                "name": field["name"],
                "dataType": field.get("dataType").cloned().unwrap_or(Value::Null),
                "roles": field.get("roles").cloned().unwrap_or(Value::Null)
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "schema": "powerbi-cli.report.spec.fields.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "schemaPath": canonical_display(&schema_path),
        "profilePath": options.profile.as_ref().map(|path| canonical_display(path)),
        "profileSummary": profile.as_ref().map(profile_summary),
        "supportedVisualTypes": supported_visual_type_names(),
        "bindingFields": ["role", "field", "table", "column", "measure", "displayName", "formatString"],
        "tables": tables,
        "fields": fields,
        "rules": [
            "Use table+column for Category, Series, Legend, and scatter Category roles.",
            "Use table+measure for DAX measures, especially when a column and measure might share a name.",
            "Legacy field strings use Table[Name]; structured bindings are safer for generated specs."
        ],
        "examples": binding_examples(&schema),
        "next": [
            format!("powerbi-cli report spec validate --schema {} --spec <dashboard.json> --json", command_arg(&schema_path)),
            format!("powerbi-cli report build --schema {} --spec <dashboard.json> --out-dir <project-dir> --json", command_arg(&schema_path)),
            "powerbi-cli report visuals catalog --json".to_string()
        ]
    }))
}

fn parse_fields_args(args: &[String]) -> CliResult<FieldsOptions> {
    let mut options = FieldsOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                options.schema = Some(PathBuf::from(take_value(args, &mut i, "--schema")?));
            }
            "--profile" => {
                options.profile = Some(PathBuf::from(take_value(args, &mut i, "--profile")?));
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report spec fields flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report spec fields --schema <schema.json> --json`.")
                .with_suggested_command(
                    "powerbi-cli report spec fields --schema <schema.json> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_suggested_command("powerbi-cli report spec fields --schema <schema.json> --json")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn table_fields_json(table: &Value, profile: Option<&ProfileLookup>) -> Option<Value> {
    let table_name = table["name"].as_str()?;
    let columns = table["columns"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|column| column_fields_json(table_name, column, profile))
        .collect::<Vec<_>>();
    let measures = table["measures"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|measure| measure_fields_json(table_name, measure))
        .collect::<Vec<_>>();
    Some(json!({
        "name": table_name,
        "profileRole": profile.and_then(|lookup| lookup.table_roles.get(table_name)).cloned(),
        "rowCount": profile.and_then(|lookup| lookup.table_row_counts.get(table_name)).copied(),
        "columns": columns,
        "measures": measures
    }))
}

fn column_fields_json(
    table_name: &str,
    column: &Value,
    profile: Option<&ProfileLookup>,
) -> Option<Value> {
    let column_name = column["name"].as_str()?;
    let reference = field_reference(table_name, column_name);
    let roles = profile
        .and_then(|lookup| {
            lookup
                .column_roles
                .get(&(table_name.to_string(), column_name.to_string()))
        })
        .cloned()
        .unwrap_or_else(|| inferred_column_roles(column));
    Some(json!({
        "kind": "column",
        "table": table_name,
        "name": column_name,
        "reference": reference,
        "dataType": column.get("dataType").cloned().unwrap_or_else(|| Value::from("string")),
        "isKey": column.get("isKey").cloned().unwrap_or(Value::Bool(false)),
        "formatString": column.get("formatString").cloned().unwrap_or(Value::Null),
        "roles": roles,
        "structuredBinding": {
            "table": table_name,
            "column": column_name
        }
    }))
}

fn measure_fields_json(table_name: &str, measure: &Value) -> Option<Value> {
    let measure_name = measure["name"].as_str()?;
    Some(json!({
        "kind": "measure",
        "table": table_name,
        "name": measure_name,
        "reference": field_reference(table_name, measure_name),
        "expression": measure.get("expression").cloned().unwrap_or(Value::Null),
        "formatString": measure.get("formatString").cloned().unwrap_or(Value::Null),
        "roles": {
            "numeric": true,
            "categorical": false,
            "dateLike": false
        },
        "structuredBinding": {
            "table": table_name,
            "measure": measure_name
        }
    }))
}

fn inferred_column_roles(column: &Value) -> Value {
    let data_type = column["dataType"]
        .as_str()
        .unwrap_or("string")
        .to_ascii_lowercase();
    let name = column["name"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let numeric = matches!(
        data_type.as_str(),
        "int64" | "integer" | "double" | "decimal" | "number"
    );
    let date_like = matches!(data_type.as_str(), "date" | "datetime")
        || name.contains("date")
        || name.contains("month")
        || name.contains("year");
    json!({
        "numeric": numeric,
        "categorical": !numeric || column["isKey"].as_bool() == Some(false),
        "dateLike": date_like
    })
}

fn binding_examples(schema: &Value) -> Vec<Value> {
    let Some(first_table) = schema["tables"]
        .as_array()
        .and_then(|tables| tables.first())
    else {
        return Vec::new();
    };
    let Some(table_name) = first_table["name"].as_str() else {
        return Vec::new();
    };
    let column = first_table["columns"]
        .as_array()
        .and_then(|columns| columns.first())
        .and_then(|column| column["name"].as_str());
    let measure = schema["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|table| {
            let table = table.as_object()?;
            let table_name = table.get("name")?.as_str()?;
            let measure_name = table
                .get("measures")?
                .as_array()?
                .first()?
                .get("name")?
                .as_str()?;
            Some((table_name.to_string(), measure_name.to_string()))
        });

    let mut examples = Vec::new();
    if let Some(column_name) = column {
        examples.push(json!({
            "role": "Category",
            "field": field_reference(table_name, column_name),
            "structured": {"role": "Category", "table": table_name, "column": column_name}
        }));
    }
    if let Some((measure_table, measure_name)) = measure {
        examples.push(json!({
            "role": "Y",
            "field": field_reference(&measure_table, &measure_name),
            "structured": {"role": "Y", "table": measure_table, "measure": measure_name}
        }));
    }
    examples
}

fn field_reference(table: &str, field: &str) -> String {
    format!("{table}[{field}]")
}

#[derive(Debug, Default)]
struct ProfileLookup {
    table_roles: BTreeMap<String, Value>,
    table_row_counts: BTreeMap<String, u64>,
    column_roles: BTreeMap<(String, String), Value>,
}

impl ProfileLookup {
    fn from_profile(profile: &Value) -> Self {
        let mut lookup = ProfileLookup::default();
        for table in profile["tables"].as_array().unwrap_or(&Vec::new()) {
            let Some(table_name) = table["name"].as_str() else {
                continue;
            };
            if let Some(role) = table.get("role").cloned() {
                lookup.table_roles.insert(table_name.to_string(), role);
            }
            if let Some(row_count) = table["rowCount"].as_u64() {
                lookup
                    .table_row_counts
                    .insert(table_name.to_string(), row_count);
            }
            for column in table["columns"].as_array().unwrap_or(&Vec::new()) {
                let Some(column_name) = column["name"].as_str() else {
                    continue;
                };
                if let Some(roles) = column.get("roles").cloned() {
                    lookup
                        .column_roles
                        .insert((table_name.to_string(), column_name.to_string()), roles);
                }
            }
        }
        lookup
    }
}
