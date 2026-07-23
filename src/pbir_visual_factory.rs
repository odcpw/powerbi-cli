use crate::pbir_bindings::{VisualBindingResolved, visual_query_json};
use crate::{CliError, CliResult};
use serde_json::{Map, Value, json};

const VISUAL_CONTAINER_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json";
pub(crate) const SLICER_MIN_HEIGHT: f64 = 76.0;
pub(crate) const BETWEEN_SLICER_MIN_HEIGHT: f64 = 104.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlicerMode {
    Basic,
    Dropdown,
    Between,
}

impl SlicerMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Dropdown => "Dropdown",
            Self::Between => "Between",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VisualBuildSpec {
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) visual_type: String,
    pub(crate) bindings: Vec<VisualBindingResolved>,
    pub(crate) slicer_mode: Option<SlicerMode>,
    pub(crate) slicer_single_select: bool,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: u64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) tab_order: u64,
}

pub(crate) fn visual_container_json(spec: &VisualBuildSpec) -> CliResult<Value> {
    if spec.bindings.is_empty() {
        return Err(CliError::invalid_args(format!(
            "{} visual requires at least one field binding",
            spec.visual_type
        ))
        .with_hint(
            "Microsoft's consumed PBIR surface rejects unbound data visuals. Add the visual's required role bindings.",
        )
        .with_suggested_command(format!(
            "powerbi-cli report visuals catalog --visual-type {} --json",
            spec.visual_type
        )));
    }
    validate_slicer_mode_binding(spec)?;
    validate_slicer_height(spec)?;
    let mut visual_config = Map::new();
    visual_config.insert(
        "visualType".to_string(),
        Value::String(spec.visual_type.clone()),
    );
    visual_config.insert("drillFilterOtherVisuals".to_string(), Value::Bool(true));
    visual_config.insert(
        "query".to_string(),
        visual_query_json(&spec.visual_type, &spec.bindings),
    );
    let mut objects = Map::new();
    if let Some(mode) = spec.slicer_mode {
        objects.insert(
            "data".to_string(),
            json!([{
                "properties": {
                    "mode": literal_text_expression(mode.as_str())
                }
            }]),
        );
        if mode == SlicerMode::Between {
            objects.insert(
                "slider".to_string(),
                json!([{
                    "properties": {
                        "show": literal_bool_expression(true)
                    }
                }]),
            );
        }
    }
    if spec.slicer_single_select {
        if spec.visual_type != "slicer" {
            return Err(CliError::invalid_args(
                "singleSelect is supported only for slicer visuals",
            ));
        }
        objects.insert(
            "selection".to_string(),
            json!([{
                "properties": {
                    "singleSelect": literal_bool_expression(true)
                }
            }]),
        );
    }
    if spec.visual_type == "pivotTable"
        && spec
            .bindings
            .iter()
            .filter(|binding| binding.role == "Rows")
            .count()
            > 1
    {
        objects.insert(
            "rowHeaders".to_string(),
            json!([{
                "properties": {
                    "showExpandCollapseButtons": literal_bool_expression(true)
                }
            }]),
        );
    }
    if !objects.is_empty() {
        visual_config.insert("objects".to_string(), Value::Object(objects));
    }
    // Desktop-authored visualContainer/2.4.0 fixtures place visual chrome titles
    // under /visual/visualContainerObjects/title. The literal-text variant is
    // archived in docs/reference/desktop-authored-visuals/slicer.visual.json.
    // Do not emit general.altText here: Microsoft powerbi-report-authoring-cli
    // v0.1.4 rejects it as PBIR_FORMATTING_PROP_UNKNOWN.
    visual_config.insert(
        "visualContainerObjects".to_string(),
        json!({
            "title": [{
                "properties": {
                    "text": literal_text_expression(&spec.title),
                    "show": literal_bool_expression(true)
                }
            }]
        }),
    );
    Ok(json!({
        "$schema": VISUAL_CONTAINER_SCHEMA,
        "name": spec.name,
        "position": {
            "x": spec.x,
            "y": spec.y,
            "z": spec.z,
            "height": spec.height,
            "width": spec.width,
            "tabOrder": spec.tab_order
        },
        "visual": Value::Object(visual_config),
        "howCreated": "DraggedToFieldWell",
        "annotations": [
            {
                "name": "powerbi-cli.placeholderTitle",
                "value": spec.title
            },
            {
                "name": "powerbi-cli.bindingStatus",
                "value": "bound"
            }
        ]
    }))
}

fn validate_slicer_height(spec: &VisualBuildSpec) -> CliResult<()> {
    if spec.visual_type != "slicer" {
        return Ok(());
    }
    let minimum = if spec.slicer_mode == Some(SlicerMode::Between) {
        BETWEEN_SLICER_MIN_HEIGHT
    } else {
        SLICER_MIN_HEIGHT
    };
    if spec.height >= minimum {
        return Ok(());
    }
    let qualifier = if spec.slicer_mode == Some(SlicerMode::Between) {
        "Between slicer"
    } else {
        "slicer"
    };
    Err(CliError::invalid_args(format!(
        "{qualifier} height {} is below the Power BI minimum of {minimum}",
        spec.height
    ))
    .with_hint(format!(
        "Increase --height to at least {minimum}; range slicers need room for both handles and the draggable band."
    )))
}

fn validate_slicer_mode_binding(spec: &VisualBuildSpec) -> CliResult<()> {
    if spec.slicer_mode != Some(SlicerMode::Between) {
        return Ok(());
    }
    validate_between_slicer_data_type(
        spec.bindings
            .first()
            .and_then(|binding| binding.data_type.as_deref()),
    )
}

pub(crate) fn validate_between_slicer_data_type(data_type: Option<&str>) -> CliResult<()> {
    let data_type = data_type.unwrap_or_default();
    if slicer_between_data_type_is_supported(data_type) {
        return Ok(());
    }
    Err(CliError::unsupported_feature(format!(
        "Between slicer requires a numeric or date column; resolved data type was {}",
        if data_type.is_empty() {
            "unknown"
        } else {
            data_type
        }
    ))
    .with_hint("Use Basic/Dropdown for text categories, or bind Between to an int64, double, decimal, or dateTime column.")
    .with_suggested_command(
        "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type slicer --mode between --title <title> --binding \"role=Values,table=<table>,column=<numeric-or-date-column>\" --dry-run --json",
    ))
}

pub(crate) fn slicer_between_data_type_is_supported(data_type: &str) -> bool {
    matches!(
        data_type.to_ascii_lowercase().as_str(),
        "int64" | "double" | "decimal" | "datetime" | "date"
    )
}

pub(crate) fn resolve_slicer_mode(
    visual_type: &str,
    requested: Option<&str>,
) -> CliResult<Option<SlicerMode>> {
    if visual_type != "slicer" {
        return if requested.is_some() {
            Err(
                CliError::invalid_args("--mode is supported only when --visual-type is slicer")
                    .with_hint("Remove --mode or use --visual-type slicer."),
            )
        } else {
            Ok(None)
        };
    }
    match requested.unwrap_or("basic").trim().to_ascii_lowercase().as_str() {
        "basic" => Ok(Some(SlicerMode::Basic)),
        "dropdown" => Ok(Some(SlicerMode::Dropdown)),
        "between" => Ok(Some(SlicerMode::Between)),
        other => Err(CliError::unsupported_feature(format!(
            "unsupported slicer mode: {other}"
        ))
        .with_hint(
            "Generated slicers support basic, dropdown, and between modes. Use between for a numeric or date range slider.",
        )
        .with_suggested_command(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type slicer --mode basic --title <title> --binding \"role=Values,table=<table>,column=<column>\" --dry-run --json",
        )),
    }
}

fn literal_text_expression(text: &str) -> Value {
    json!({ "expr": { "Literal": { "Value": encode_text_literal(text) } } })
}

fn literal_bool_expression(value: bool) -> Value {
    json!({ "expr": { "Literal": { "Value": value.to_string() } } })
}

fn encode_text_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
