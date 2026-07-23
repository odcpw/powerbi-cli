use crate::tmdl::{TableDocument, same_name};
use crate::visual_catalog::{column_binding_is_proven, normalize_role};
use crate::{CliError, CliResult};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub(crate) struct VisualBindingInput {
    pub(crate) role: String,
    pub(crate) table: String,
    pub(crate) column: Option<String>,
    pub(crate) measure: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) format_string: Option<String>,
    pub(crate) sort_direction: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualBindingResolved {
    pub(crate) role: String,
    pub(crate) table: String,
    pub(crate) field: String,
    pub(crate) kind: VisualBindingKind,
    pub(crate) data_type: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) format_string: Option<String>,
    pub(crate) sort_direction: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum VisualBindingKind {
    Column,
    Measure,
}

impl VisualBindingKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Column => "column",
            Self::Measure => "measure",
        }
    }
}

pub(crate) fn parse_binding_spec(text: &str) -> CliResult<VisualBindingInput> {
    let mut input = VisualBindingInput::default();
    for part in text.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            return Err(CliError::invalid_args(format!(
                "binding part must be key=value: {trimmed}"
            ))
            .with_hint("Use `role=Values,table=FactSales,measure=Total Revenue` or pass --bindings-json.")
            .with_suggested_command(
                "powerbi-cli report visuals set-bindings --project <project> --handle <visual-handle> --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
            ));
        };
        set_binding_field(&mut input, key.trim(), value.trim().to_string())?;
    }
    validate_input_shape(input)
}

pub(crate) fn parse_bindings_json_text(text: &str) -> CliResult<Vec<VisualBindingInput>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|err| CliError::invalid_args(format!("parse --bindings-json: {err}")))?;
    parse_bindings_json_value(&value)
}

pub(crate) fn parse_bindings_json_file(path: &Path) -> CliResult<Vec<VisualBindingInput>> {
    let text = fs::read_to_string(path).map_err(|err| {
        CliError::file_not_found(format!("read bindings file {}: {err}", path.display()))
    })?;
    parse_bindings_json_text(&text)
}

pub(crate) fn parse_bindings_json_value(value: &Value) -> CliResult<Vec<VisualBindingInput>> {
    let array = if let Some(items) = value.as_array() {
        items
    } else if let Some(items) = value["bindings"].as_array() {
        items
    } else {
        return Err(CliError::invalid_args(
            "--bindings-json must be an array or an object with a bindings array",
        )
        .with_hint(
            "Example: [{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]",
        )
        .with_suggested_command(
            "powerbi-cli report visuals set-bindings --project <project> --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json",
        ));
    };
    array.iter().map(parse_binding_json_object).collect()
}

pub(crate) fn resolve_visual_bindings(
    docs: &[TableDocument],
    visual_type: &str,
    inputs: &[VisualBindingInput],
) -> CliResult<Vec<VisualBindingResolved>> {
    let resolved = inputs
        .iter()
        .map(|input| resolve_binding(docs, visual_type, input))
        .collect::<CliResult<Vec<_>>>()?;
    reject_duplicate_fields(&resolved)?;
    validate_sort_bindings(&resolved)?;
    Ok(resolved)
}

pub(crate) fn visual_query_json(visual_type: &str, bindings: &[VisualBindingResolved]) -> Value {
    let mut query_state = Map::new();
    for binding in bindings {
        let entry = query_state
            .entry(binding.role.clone())
            .or_insert_with(|| json!({ "projections": [] }));
        if let Some(projections) = entry.get_mut("projections").and_then(Value::as_array_mut) {
            let active =
                projections.is_empty() && active_projection_role(visual_type, &binding.role);
            projections.push(visual_projection_json(binding, active));
        }
    }
    let mut query = Map::new();
    query.insert("queryState".to_string(), Value::Object(query_state));
    if let Some(sort) = bindings
        .iter()
        .find(|binding| binding.sort_direction.is_some())
    {
        query.insert(
            "sortDefinition".to_string(),
            json!({
                "sort": [{
                    "field": visual_field_expression(sort),
                    "direction": sort.sort_direction.as_deref().unwrap_or("Descending")
                }]
            }),
        );
    } else if matches!(visual_type, "pieChart" | "donutChart")
        && let Some(first_y) = bindings.iter().find(|binding| binding.role == "Y")
    {
        query.insert(
            "sortDefinition".to_string(),
            json!({
                "sort": [{
                    "field": visual_field_expression(first_y),
                    "direction": "Descending"
                }],
                "isDefaultSort": true
            }),
        );
    }
    Value::Object(query)
}

pub(crate) fn binding_summary(binding: &VisualBindingResolved) -> Value {
    json!({
        "role": binding.role,
        "kind": binding.kind.as_str(),
        "table": binding.table,
        "field": binding.field,
        "column": matches!(binding.kind, VisualBindingKind::Column).then_some(binding.field.clone()),
        "measure": matches!(binding.kind, VisualBindingKind::Measure).then_some(binding.field.clone()),
        "queryRef": format!("{}.{}", binding.table, binding.field),
        "nativeQueryRef": binding.field,
        "displayName": binding.display_name,
        "format": binding.format_string,
        "sortDirection": binding.sort_direction
    })
}

pub(crate) fn set_binding_status_annotation(visual_json: &mut Value, status: &str) {
    let annotation = json!({
        "name": "powerbi-cli.bindingStatus",
        "value": status
    });
    if !visual_json["annotations"].is_array() {
        visual_json["annotations"] = Value::Array(Vec::new());
    }
    let annotations = visual_json["annotations"]
        .as_array_mut()
        .expect("annotations was just made an array");
    if let Some(existing) = annotations
        .iter_mut()
        .find(|item| item["name"].as_str() == Some("powerbi-cli.bindingStatus"))
    {
        *existing = annotation;
    } else {
        annotations.push(annotation);
    }
}

fn parse_binding_json_object(value: &Value) -> CliResult<VisualBindingInput> {
    let object = value.as_object().ok_or_else(|| {
        CliError::invalid_args("each binding JSON item must be an object")
            .with_hint("Use objects with role, table, and exactly one of column or measure.")
            .with_suggested_command(
                "powerbi-cli report visuals set-bindings --project <project> --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json",
            )
    })?;
    let mut input = VisualBindingInput::default();
    for (key, value) in object {
        let value = value.as_str().ok_or_else(|| {
            CliError::invalid_args(format!("binding field {key} must be a string"))
                .with_hint("Binding JSON values are strings so agents can preserve exact Power BI names.")
                .with_suggested_command(
                    "powerbi-cli report visuals set-bindings --project <project> --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json",
                )
        })?;
        set_binding_field(&mut input, key, value.to_string())?;
    }
    validate_input_shape(input)
}

fn set_binding_field(input: &mut VisualBindingInput, key: &str, value: String) -> CliResult<()> {
    match key {
        "role" => input.role = value,
        "table" => input.table = value,
        "column" => input.column = Some(value),
        "measure" => input.measure = Some(value),
        "display" | "displayName" | "display_name" => input.display_name = Some(value),
        "format" | "formatString" | "format_string" => input.format_string = Some(value),
        "sort" | "sortDirection" | "sort_direction" => {
            input.sort_direction = Some(normalize_sort_direction(&value)?)
        }
        other => {
            return Err(CliError::invalid_args(format!(
                "unknown binding field: {other}"
            ))
            .with_hint("Supported binding fields are role, table, column, measure, displayName, format, and sortDirection.")
            .with_suggested_command(
                "powerbi-cli report visuals set-bindings --project <project> --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json",
            ));
        }
    }
    Ok(())
}

fn validate_input_shape(input: VisualBindingInput) -> CliResult<VisualBindingInput> {
    if input.role.trim().is_empty() {
        return Err(binding_shape_error("binding requires role"));
    }
    if input.table.trim().is_empty() {
        return Err(binding_shape_error("binding requires table"));
    }
    match (&input.column, &input.measure) {
        (Some(_), Some(_)) => Err(binding_shape_error(
            "binding accepts either column or measure, not both",
        )),
        (None, None) => Err(binding_shape_error(
            "binding requires exactly one of column or measure",
        )),
        _ => Ok(input),
    }
}

fn binding_shape_error(message: &str) -> CliError {
    CliError::invalid_args(message)
        .with_hint("Use one binding per field well projection.")
        .with_suggested_command(
            "powerbi-cli report visuals set-bindings --project <project> --handle <visual-handle> --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
        )
}

fn resolve_binding(
    docs: &[TableDocument],
    visual_type: &str,
    input: &VisualBindingInput,
) -> CliResult<VisualBindingResolved> {
    let role = normalize_role(visual_type, &input.role)?;
    if input.column.is_some() && !column_binding_is_proven(visual_type, &role)? {
        return Err(CliError::unsupported_feature(format!(
            "raw column bindings are not Desktop-proven for {visual_type}.{role}"
        ))
        .with_hint(
            "Define a measure, or wait for aggregation-binding support. A bare Column expression is refused for measure/value roles.",
        )
        .with_suggested_command(format!(
            "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
        )));
    }
    let table = docs
        .iter()
        .find(|doc| same_name(&doc.table, &input.table))
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "table not found for visual binding: {}",
                input.table
            ))
            .with_hint(
                "Run `inspect --deep` or `model measures list` to discover canonical table names.",
            )
            .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json")
        })?;
    let (kind, field, data_type) = if let Some(measure) = input.measure.as_deref() {
        let measure = table
            .measures
            .iter()
            .find(|candidate| same_name(&candidate.name, measure))
            .ok_or_else(|| {
                CliError::validation_failed(format!(
                    "measure not found for visual binding: {}.{}",
                    table.table, measure
                ))
                .with_hint("Run `model measures list` to discover available measures.")
                .with_suggested_command(
                    "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
                )
            })?;
        (VisualBindingKind::Measure, measure.name.clone(), None)
    } else {
        let column = input
            .column
            .as_deref()
            .expect("validated column or measure");
        let column = table
            .columns
            .iter()
            .find(|candidate| same_name(&candidate.name, column))
            .ok_or_else(|| {
                CliError::validation_failed(format!(
                    "column not found for visual binding: {}.{}",
                    table.table, column
                ))
                .with_hint("Run `inspect --deep` to discover available columns.")
                .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json")
            })?;
        (
            VisualBindingKind::Column,
            column.name.clone(),
            column.data_type.clone(),
        )
    };
    Ok(VisualBindingResolved {
        role,
        table: table.table.clone(),
        field,
        kind,
        data_type,
        display_name: input.display_name.clone(),
        format_string: input.format_string.clone(),
        sort_direction: input.sort_direction.clone(),
    })
}

fn normalize_sort_direction(value: &str) -> CliResult<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "descending" | "desc" => Ok("Descending".to_string()),
        other => Err(CliError::unsupported_feature(format!(
            "unsupported visual sort direction: {other}"
        ))
        .with_hint(
            "The first typed slice supports descending measure sort only; ascending and multi-key sort remain fixture-gated.",
        )),
    }
}

pub(crate) fn validate_sort_bindings(bindings: &[VisualBindingResolved]) -> CliResult<()> {
    let sorted = bindings
        .iter()
        .filter(|binding| binding.sort_direction.is_some())
        .collect::<Vec<_>>();
    if sorted.len() > 1 {
        return Err(CliError::unsupported_feature(
            "generated visuals support exactly one explicit sort binding",
        )
        .with_hint("Remove sortDirection from all but one projected measure."));
    }
    let Some(binding) = sorted.first() else {
        return Ok(());
    };
    if !matches!(binding.kind, VisualBindingKind::Measure) {
        return Err(CliError::unsupported_feature(
            "explicit visual sort is currently proven only for measures",
        )
        .with_hint("Sort by a projected measure, not a raw category column."));
    }
    if !matches!(binding.role.as_str(), "Y" | "Y2" | "Values" | "Tooltips") {
        return Err(CliError::unsupported_feature(format!(
            "explicit visual sort is not supported on role {}",
            binding.role
        ))
        .with_hint("Use a projected measure in Y, Y2, Values, or Tooltips."));
    }
    Ok(())
}

fn reject_duplicate_fields(bindings: &[VisualBindingResolved]) -> CliResult<()> {
    for (index, binding) in bindings.iter().enumerate() {
        if let Some(previous) = bindings[..index]
            .iter()
            .find(|previous| previous.table == binding.table && previous.field == binding.field)
        {
            return Err(CliError::unsupported_feature(format!(
                "duplicate visual field usage is not Desktop-proven: {}[{}] is bound to both {} and {}",
                binding.table, binding.field, previous.role, binding.role
            ))
            .with_hint(
                "Use distinct fields. Desktop-authored ground truth for duplicate queryRef/nativeQueryRef numbering is not available, so powerbi-cli refuses to guess.",
            )
            .with_suggested_command(
                "powerbi-cli report visuals catalog --visual-type <visual-type> --json",
            ));
        }
    }
    Ok(())
}

fn visual_projection_json(binding: &VisualBindingResolved, active: bool) -> Value {
    let query_ref = format!("{}.{}", binding.table, binding.field);
    let mut projection = Map::new();
    projection.insert("field".to_string(), visual_field_expression(binding));
    projection.insert("queryRef".to_string(), Value::String(query_ref));
    projection.insert(
        "nativeQueryRef".to_string(),
        Value::String(binding.field.clone()),
    );
    if let Some(display_name) = &binding.display_name {
        projection.insert(
            "displayName".to_string(),
            Value::String(display_name.clone()),
        );
    }
    if let Some(format_string) = &binding.format_string {
        projection.insert("format".to_string(), Value::String(format_string.clone()));
    }
    if active {
        projection.insert("active".to_string(), Value::Bool(true));
    }
    Value::Object(projection)
}

fn active_projection_role(visual_type: &str, role: &str) -> bool {
    match visual_type {
        "pieChart" | "donutChart" => role == "Category",
        "pivotTable" => matches!(role, "Rows" | "Columns"),
        "slicer" => role == "Values",
        // Cartesian Category hierarchies intentionally omit `active`. A CLI-emitted
        // three-level hierarchy drilled correctly in Desktop Store
        // 2.155.756.0 on 2026-07-10; see docs/pbir-desktop-oracle.md.
        _ => false,
    }
}

fn visual_field_expression(binding: &VisualBindingResolved) -> Value {
    let source = json!({
        "SourceRef": {
            "Entity": binding.table
        }
    });
    match binding.kind {
        VisualBindingKind::Measure => json!({
            "Measure": {
                "Expression": source,
                "Property": binding.field
            }
        }),
        VisualBindingKind::Column => json!({
            "Column": {
                "Expression": source,
                "Property": binding.field
            }
        }),
    }
}
