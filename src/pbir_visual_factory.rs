use crate::pbir_bindings::{VisualBindingResolved, visual_query_json};
use crate::{CliError, CliResult};
use serde_json::{Map, Value, json};

const VISUAL_CONTAINER_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlicerMode {
    Basic,
    Dropdown,
}

impl SlicerMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Dropdown => "Dropdown",
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
    }
    if !objects.is_empty() {
        visual_config.insert("objects".to_string(), Value::Object(objects));
    }
    // Desktop-authored visualContainer/2.4.0 fixtures place visual chrome titles
    // under /visual/visualContainerObjects/title. The literal-text variant is
    // archived in docs/reference/desktop-authored-visuals/slicer.visual.json.
    visual_config.insert(
        "visualContainerObjects".to_string(),
        json!({
            "general": [{
                "properties": {
                    "altText": literal_text_expression(&format!("{} visual", spec.title))
                }
            }],
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
        other => Err(CliError::unsupported_feature(format!(
            "unsupported slicer mode: {other}"
        ))
        .with_hint(
            "Generated slicers support only basic and dropdown modes until other modes receive Desktop golden proof.",
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
