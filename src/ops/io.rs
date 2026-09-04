//! Operation-plan persistence boundary.
//!
//! All plan-file reads go through [`read_plan_file`]. This is intentionally a
//! narrow seam: the shared input-safety gate is invoked here before operation
//! parsing or application, without duplicating budget or path policy in the
//! operation layer.

use super::{OPS_SCHEMA, OpPlan};
use crate::{CliError, CliResult, input_safety};
use serde_json::Value;
use std::path::Path;

const OP_KIND_CATALOG: &[&str] = &[
    "addMeasure",
    "addRelationship",
    "addVisual",
    "addFilter",
    "setDrillthrough",
    "setInteraction",
    "applyThemePreset",
    "setObject",
    // Compatibility spellings accepted by the safety harness while older
    // plans are migrated to the tagged IR representation.
    "AddMeasure",
    "AddRelationship",
    "AddVisual",
    "AddFilter",
    "SetDrillthrough",
    "SetInteraction",
    "ApplyThemePreset",
    "SetObject",
];

pub(crate) fn read_plan_file(path: &Path) -> CliResult<OpPlan> {
    let mut value = input_safety::read_ops(path, OP_KIND_CATALOG)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| CliError::validation_failed("operation plan must be a JSON object"))?;
    let schema = object
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::validation_failed("operation plan requires schema"))?;
    if schema != OPS_SCHEMA {
        return Err(CliError::validation_failed(format!(
            "operation plan schema must be {OPS_SCHEMA}"
        ))
        .with_pointer("/schema"));
    }
    object.remove("schema");
    if let Some(ops) = object.get_mut("ops").and_then(Value::as_array_mut) {
        for operation in ops {
            if let Some(map) = operation.as_object_mut() {
                let tag = map.get("op").cloned().or_else(|| map.remove("kind"));
                if let Some(tag) = tag {
                    map.insert("op".to_string(), normalize_tag(tag));
                }
            }
        }
    }
    serde_json::from_value(value).map_err(|error| {
        CliError::validation_failed(format!("decode operation plan {}: {error}", path.display()))
    })
}

fn normalize_tag(value: Value) -> Value {
    let Some(tag) = value.as_str() else {
        return value;
    };
    let normalized = match tag {
        "AddMeasure" => "addMeasure",
        "AddRelationship" => "addRelationship",
        "AddVisual" => "addVisual",
        "AddFilter" => "addFilter",
        "SetDrillthrough" => "setDrillthrough",
        "SetInteraction" => "setInteraction",
        "ApplyThemePreset" => "applyThemePreset",
        "SetObject" => "setObject",
        _ => tag,
    };
    Value::String(normalized.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::{AddMeasure, Op};
    use super::*;

    fn plan() -> OpPlan {
        OpPlan::new(vec![Op::AddMeasure(AddMeasure {
            handle: "measure:Sales:Revenue".into(),
            table: "Sales".into(),
            name: "Revenue".into(),
            expression: "SUM(Sales[Revenue])".into(),
            format_string: None,
            format_string_definition: None,
            description: None,
            display_folder: None,
        })])
    }

    #[test]
    fn plan_file_boundary_reads_schema_envelope_and_round_trips() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("plan.json");
        let value = serde_json::to_value(plan()).expect("plan json");
        assert_eq!(value["schema"], OPS_SCHEMA);
        std::fs::write(&path, serde_json::to_vec(&value).expect("write json")).expect("write plan");
        assert_eq!(read_plan_file(&path).expect("read plan"), plan());
    }

    #[test]
    fn plan_file_boundary_normalizes_legacy_kind_tags_after_safety_check() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("legacy-plan.json");
        let mut value = serde_json::to_value(plan()).expect("plan json");
        assert_eq!(value["schema"], OPS_SCHEMA);
        let operation = value["ops"][0].as_object_mut().expect("operation object");
        let tag = operation.remove("op").expect("typed operation tag");
        operation.insert("kind".to_string(), Value::String("AddMeasure".into()));
        assert_eq!(tag, Value::String("addMeasure".into()));
        std::fs::write(&path, serde_json::to_vec(&value).expect("write json")).expect("write plan");
        assert_eq!(read_plan_file(&path).expect("read legacy plan"), plan());
    }

    #[test]
    fn plan_file_boundary_rejects_wrong_schema_with_pointer() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("plan.json");
        std::fs::write(&path, r#"{"schema":"other.v1","ops":[]}"#).expect("write plan");
        let error = read_plan_file(&path).expect_err("wrong schema must fail");
        assert_eq!(error.code, "input_safety_violation");
    }
}
