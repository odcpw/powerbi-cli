use crate::help::edit_distance;
use crate::{CliError, CliResult, EXIT_VALIDATION_FAILED};
use serde_json::{Map, Value, json};

#[derive(Clone, Copy)]
struct NodeSchema {
    name: &'static str,
    path: &'static str,
    fields: &'static [&'static str],
}

const ROOT: NodeSchema = node(
    "root",
    "",
    &["schema", "report", "model", "pages", "style", "proof"],
);
const REPORT: NodeSchema = node(
    "report",
    "report",
    &[
        "name",
        "displayName",
        "description",
        "locale",
        "audience",
        "questions",
    ],
);
const MODEL: NodeSchema = node("model", "model", &["measures", "relationships"]);
const MODEL_MEASURE: NodeSchema = node(
    "model.measures[]",
    "model.measures",
    &[
        "table",
        "name",
        "expression",
        "formatString",
        "description",
        "displayFolder",
    ],
);
const PAGE: NodeSchema = node(
    "pages[]",
    "pages",
    &[
        "id",
        "name",
        "displayName",
        "size",
        "filters",
        "visuals",
        "interactions",
    ],
);
const PAGE_SIZE: NodeSchema = node("pages[].size", "pages.size", &["width", "height"]);
const VISUAL: NodeSchema = node(
    "pages[].visuals[]",
    "pages.visuals",
    &[
        "id",
        "name",
        "type",
        "visualType",
        "title",
        "bindings",
        "layout",
        "mode",
        "singleSelect",
        "drilldown",
        "sortDirection",
        "x",
        "y",
        "width",
        "height",
    ],
);
const VISUAL_LAYOUT: NodeSchema = node(
    "pages[].visuals[].layout",
    "pages.visuals.layout",
    &["x", "y", "width", "height"],
);
const BINDING: NodeSchema = node(
    "pages[].visuals[].bindings[]",
    "pages.visuals.bindings",
    &[
        "role",
        "field",
        "table",
        "column",
        "measure",
        "displayName",
        "formatString",
        "sortDirection",
    ],
);
const INTERACTION: NodeSchema = node(
    "pages[].interactions[]",
    "pages.interactions",
    &["source", "target", "type"],
);

const V1_NODES: &[NodeSchema] = &[
    ROOT,
    REPORT,
    MODEL,
    MODEL_MEASURE,
    PAGE,
    PAGE_SIZE,
    VISUAL,
    VISUAL_LAYOUT,
    BINDING,
    INTERACTION,
];

const fn node(
    name: &'static str,
    path: &'static str,
    fields: &'static [&'static str],
) -> NodeSchema {
    NodeSchema { name, path, fields }
}

pub(crate) fn validate_known_fields(spec: &Value) -> CliResult<()> {
    let root = spec
        .as_object()
        .ok_or_else(|| CliError::invalid_args("dashboard spec root must be an object"))?;
    walk_object(root, ROOT, "")?;

    walk_optional_object(root, "report", REPORT, "/report", |_| Ok(()))?;
    walk_optional_object(root, "model", MODEL, "/model", |model| {
        walk_array_objects(model, "measures", MODEL_MEASURE, "/model/measures", |_| {
            Ok(())
        })
    })?;
    walk_array_objects(root, "pages", PAGE, "/pages", |page| {
        walk_optional_object(page, "size", PAGE_SIZE, "size", |_| Ok(()))?;
        walk_array_objects(page, "visuals", VISUAL, "visuals", |visual| {
            walk_optional_object(visual, "layout", VISUAL_LAYOUT, "layout", |_| Ok(()))?;
            walk_array_objects(visual, "bindings", BINDING, "bindings", |_| Ok(()))
        })?;
        walk_array_objects(
            page,
            "interactions",
            INTERACTION,
            "interactions",
            |_| Ok(()),
        )
    })?;
    Ok(())
}

pub(crate) fn allowed_fields_json() -> Value {
    Value::Array(
        V1_NODES
            .iter()
            .map(|node| {
                json!({
                    "node": node.name,
                    "fields": node.fields,
                })
            })
            .collect(),
    )
}

fn walk_optional_object<F>(
    parent: &Map<String, Value>,
    field: &str,
    schema: NodeSchema,
    pointer: &str,
    nested: F,
) -> CliResult<()>
where
    F: FnOnce(&Map<String, Value>) -> CliResult<()>,
{
    let Some(value) = parent.get(field) else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    walk_object(object, schema, pointer)?;
    nested(object)
}

fn walk_array_objects<F>(
    parent: &Map<String, Value>,
    field: &str,
    schema: NodeSchema,
    pointer: &str,
    mut nested: F,
) -> CliResult<()>
where
    F: FnMut(&Map<String, Value>) -> CliResult<()>,
{
    let Some(values) = parent.get(field).and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, value) in values.iter().enumerate() {
        let Some(object) = value.as_object() else {
            continue;
        };
        let item_pointer = format!("{pointer}/{index}");
        walk_object(object, schema, &item_pointer)?;
        nested(object).map_err(|mut error| {
            if let Some(relative) = error.pointer.as_deref() {
                if !relative.starts_with('/') {
                    error.pointer = Some(format!("{item_pointer}/{relative}"));
                }
            }
            error
        })?;
    }
    Ok(())
}

fn walk_object(object: &Map<String, Value>, schema: NodeSchema, pointer: &str) -> CliResult<()> {
    for key in object.keys() {
        if !schema.fields.contains(&key.as_str()) {
            return Err(unknown_field(schema, key, pointer));
        }
    }
    Ok(())
}

fn unknown_field(schema: NodeSchema, key: &str, parent_pointer: &str) -> CliError {
    let pointer = format!("{parent_pointer}/{}", escape_pointer_token(key));
    let parent = if schema.name == "root" {
        "dashboard spec root"
    } else {
        schema.name
    };
    let mut error = CliError::new(
        "spec.unknown_field",
        EXIT_VALIDATION_FAILED,
        format!("unknown dashboard spec field `{key}` under {parent}"),
    )
    .with_pointer(pointer)
    .with_suggested_command("powerbi-cli report spec fields --schema <schema.json> --json");
    if let Some(suggestion) = suggestion(schema, key) {
        error = error.with_did_you_mean(suggestion);
    }
    error
}

fn suggestion(schema: NodeSchema, key: &str) -> Option<String> {
    let direct = schema.fields.iter().copied().min_by_key(|candidate| {
        edit_distance(&key.to_ascii_lowercase(), &candidate.to_ascii_lowercase())
    });
    if let Some(candidate) = direct {
        let distance = edit_distance(&key.to_ascii_lowercase(), &candidate.to_ascii_lowercase());
        if distance <= suggestion_threshold(key.len(), candidate.len()) {
            return Some(candidate.to_string());
        }
    }

    V1_NODES
        .iter()
        .flat_map(|node| node.fields.iter().map(move |field| (node, *field)))
        .filter(|(_, field)| field.eq_ignore_ascii_case(key))
        .map(|(node, field)| {
            if node.path.is_empty() {
                field.to_string()
            } else {
                format!("{}.{}", node.path, field)
            }
        })
        .min_by_key(String::len)
}

fn suggestion_threshold(a: usize, b: usize) -> usize {
    match a.max(b) {
        0..=4 => 1,
        5..=8 => 2,
        _ => 3,
    }
}

fn escape_pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unknown(value: Value, pointer: &str) -> CliError {
        let error = validate_known_fields(&value).expect_err("unknown field must fail");
        assert_eq!(error.code, "spec.unknown_field");
        assert_eq!(error.pointer.as_deref(), Some(pointer));
        error
    }

    #[test]
    fn rejects_unknown_fields_at_every_v1_object_level() {
        let cases = [
            (json!({"measures": []}), "/measures"),
            (json!({"report": {"colour": "red"}}), "/report/colour"),
            (json!({"model": {"measure": []}}), "/model/measure"),
            (
                json!({"model": {"measures": [{"formula": "1"}]}}),
                "/model/measures/0/formula",
            ),
            (json!({"pages": [{"colour": "red"}]}), "/pages/0/colour"),
            (
                json!({"pages": [{"size": {"wide": 1}}]}),
                "/pages/0/size/wide",
            ),
            (
                json!({"pages": [{"visuals": [{"colour": "red"}]}]}),
                "/pages/0/visuals/0/colour",
            ),
            (
                json!({"pages": [{"visuals": [{"layout": {"left": 1}}]}]}),
                "/pages/0/visuals/0/layout/left",
            ),
            (
                json!({"pages": [{"visuals": [{"bindings": [{"colour": "red"}]}]}]}),
                "/pages/0/visuals/0/bindings/0/colour",
            ),
            (
                json!({"pages": [{"interactions": [{"effect": "filter"}]}]}),
                "/pages/0/interactions/0/effect",
            ),
        ];
        for (value, pointer) in cases {
            assert_unknown(value, pointer);
        }
    }

    #[test]
    fn suggests_a_misplaced_known_field_with_its_qualified_path() {
        let error = assert_unknown(json!({"measures": []}), "/measures");
        assert_eq!(error.did_you_mean.as_deref(), Some("model.measures"));
    }

    #[test]
    fn pointer_escapes_rfc_6901_tokens() {
        assert_unknown(json!({"bad/key~name": true}), "/bad~1key~0name");
    }

    #[test]
    fn hand_rolled_unknown_insertion_property_always_returns_resolvable_pointer() {
        let mut seed = 0x5eed_u64;
        for iteration in 0..128 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let unknown = format!("unknown_{iteration}_{:x}", seed >> 32);
            let mut spec = json!({
                "schema": "powerbi-cli.dashboard.v1",
                "report": {"name": "Property"},
                "model": {"measures": [{"table": "Facts", "name": "Count", "expression": "1"}]},
                "pages": [{
                    "id": "overview",
                    "size": {"width": 1280, "height": 720},
                    "visuals": [{
                        "id": "card",
                        "type": "card",
                        "layout": {"x": 0, "y": 0, "width": 100, "height": 100},
                        "bindings": [{"role": "Values", "field": "Facts[Count]"}]
                    }],
                    "interactions": [{"source": "card", "target": "other", "type": "NoFilter"}]
                }]
            });
            let (object, expected_prefix) = match seed as usize % 10 {
                0 => (spec.as_object_mut().expect("root"), ""),
                1 => (spec["report"].as_object_mut().expect("report"), "/report"),
                2 => (spec["model"].as_object_mut().expect("model"), "/model"),
                3 => (
                    spec["model"]["measures"][0]
                        .as_object_mut()
                        .expect("measure"),
                    "/model/measures/0",
                ),
                4 => (spec["pages"][0].as_object_mut().expect("page"), "/pages/0"),
                5 => (
                    spec["pages"][0]["size"].as_object_mut().expect("size"),
                    "/pages/0/size",
                ),
                6 => (
                    spec["pages"][0]["visuals"][0]
                        .as_object_mut()
                        .expect("visual"),
                    "/pages/0/visuals/0",
                ),
                7 => (
                    spec["pages"][0]["visuals"][0]["layout"]
                        .as_object_mut()
                        .expect("layout"),
                    "/pages/0/visuals/0/layout",
                ),
                8 => (
                    spec["pages"][0]["visuals"][0]["bindings"][0]
                        .as_object_mut()
                        .expect("binding"),
                    "/pages/0/visuals/0/bindings/0",
                ),
                _ => (
                    spec["pages"][0]["interactions"][0]
                        .as_object_mut()
                        .expect("interaction"),
                    "/pages/0/interactions/0",
                ),
            };
            object.insert(unknown.clone(), Value::Bool(true));
            let expected = format!("{expected_prefix}/{unknown}");
            assert_unknown(spec, &expected);
        }
    }
}
