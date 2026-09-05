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
    "addCalculatedColumn",
    "addFilter",
    "addMeasure",
    "addPage",
    "addRelationship",
    "addStaticTable",
    "addVisual",
    "applyStyleBundle",
    "applyThemeBundle",
    "applyThemePreset",
    "bookmarkMetadata",
    "clearFilter",
    "clonePage",
    "cloneVisual",
    "deleteEmptyPage",
    "deleteFilter",
    "deleteVisual",
    "formattingApply",
    "reorderPages",
    "resetInteraction",
    "sanitizeAction",
    "setActivePage",
    "setBindings",
    "setColor",
    "setDisplayName",
    "setDrilldownHierarchy",
    "setDrillthrough",
    "setInteraction",
    "setObject",
    "setPosition",
    "setSortBy",
    "setText",
    "setTopNGuard",
    "slicerClear",
    "sourceTemplateApply",
    "updateFilter",
    "updatePage",
    // Compatibility spellings accepted by the safety harness while older
    // plans are migrated to the tagged IR representation.
    "AddCalculatedColumn",
    "AddFilter",
    "AddMeasure",
    "AddPage",
    "AddRelationship",
    "AddStaticTable",
    "AddVisual",
    "ApplyStyleBundle",
    "ApplyThemeBundle",
    "ApplyThemePreset",
    "BookmarkMetadata",
    "ClearFilter",
    "ClonePage",
    "CloneVisual",
    "DeleteEmptyPage",
    "DeleteFilter",
    "DeleteVisual",
    "FormattingApply",
    "ReorderPages",
    "ResetInteraction",
    "SanitizeAction",
    "SetActivePage",
    "SetBindings",
    "SetColor",
    "SetDisplayName",
    "SetDrilldownHierarchy",
    "SetDrillthrough",
    "SetInteraction",
    "SetObject",
    "SetPosition",
    "SetSortBy",
    "SetText",
    "SetTopNGuard",
    "SlicerClear",
    "SourceTemplateApply",
    "UpdateFilter",
    "UpdatePage",
    // Recognize raw-patch spellings only to return the public refusal code.
    "rawPatch",
    "raw-patch",
    "RawPatch",
    "patch",
];

pub(crate) fn read_plan_file(path: &Path) -> CliResult<OpPlan> {
    let mut value = input_safety::read_ops(path, OP_KIND_CATALOG).map_err(|error| {
        if error.pointer().is_none() {
            error.with_pointer("")
        } else {
            error
        }
    })?;
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
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "schema" | "ops"))
    {
        return Err(plan_input_error(
            format!("unknown plan field `{key}`"),
            format!("/{}", crate::diagnostics::escape_pointer_token(key)),
        ));
    }
    let mut decoded = Vec::new();
    if let Some(ops) = object.get_mut("ops").and_then(Value::as_array_mut) {
        for (index, operation) in ops.iter_mut().enumerate() {
            let pointer = format!("/ops/{index}");
            if let Some(map) = operation.as_object_mut() {
                if let (Some(op), Some(kind)) = (map.get("op"), map.get("kind"))
                    && normalize_tag(op.clone()) != normalize_tag(kind.clone())
                {
                    return Err(plan_input_error(
                        "conflicting op and kind tags",
                        format!("{pointer}/kind"),
                    ));
                }
                let legacy_tag = map.remove("kind");
                let tag = map.get("op").cloned().or(legacy_tag);
                if let Some(tag) = tag {
                    if matches!(
                        tag.as_str(),
                        Some("rawPatch" | "raw-patch" | "RawPatch" | "patch")
                    ) {
                        return Err(CliError::unsupported_feature("raw-patch operations are not supported")
                            .with_pointer(format!("{pointer}/op"))
                            .with_hint("Use typed operations from the registered kernel catalog; arbitrary JSON patches cannot be replayed safely.")
                            .with_suggested_command("powerbi-cli capabilities --for 'ops apply' --json"));
                    }
                    map.insert("op".to_string(), normalize_tag(tag));
                }
                for key in map.keys() {
                    if matches!(
                        key.as_str(),
                        "project"
                            | "p"
                            | "out"
                            | "outDir"
                            | "dryRun"
                            | "inPlace"
                            | "emitOp"
                            | "json"
                            | "format"
                            | "snapshotDir"
                            | "_"
                    ) || !key.chars().all(|ch| ch.is_ascii_alphanumeric())
                    {
                        return Err(plan_input_error(
                            format!(
                                "operation field `{key}` is not a payload field; project/output transport belongs to ops apply"
                            ),
                            format!(
                                "{pointer}/{}",
                                crate::diagnostics::escape_pointer_token(key)
                            ),
                        ));
                    }
                }
            }
            let op: super::Op = serde_json::from_value(operation.clone()).map_err(|error| {
                plan_input_error(
                    format!("decode operation {index}: {error}"),
                    pointer.clone(),
                )
            })?;
            let encoded = serde_json::to_value(&op).expect("Op serializes");
            if let Some(key) = operation
                .as_object()
                .and_then(|map| map.keys().find(|key| encoded.get(key.as_str()).is_none()))
            {
                return Err(plan_input_error(
                    format!("unknown operation field `{key}`"),
                    format!(
                        "{pointer}/{}",
                        crate::diagnostics::escape_pointer_token(key)
                    ),
                ));
            }
            decoded.push(op);
        }
    }
    Ok(OpPlan::new(decoded))
}

fn plan_input_error(message: impl Into<String>, pointer: impl Into<String>) -> CliError {
    CliError::new(crate::rules::OPS_INVALID_PLAN, crate::EXIT_VALIDATION_FAILED, message)
        .with_pointer(pointer)
        .with_hint("Supply a bounded powerbi-cli.ops.v1 envelope with typed payloads and project/output options only on the command line.")
        .with_suggested_command("powerbi-cli capabilities --for 'ops apply' --json")
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
        "ResetInteraction" => "resetInteraction",
        "ApplyThemePreset" => "applyThemePreset",
        "SetObject" => "setObject",
        "SetPosition" => "setPosition",
        "AddCalculatedColumn" => "addCalculatedColumn",
        "AddStaticTable" => "addStaticTable",
        "SetSortBy" => "setSortBy",
        "SourceTemplateApply" => "sourceTemplateApply",
        "AddPage" => "addPage",
        "UpdatePage" => "updatePage",
        "ReorderPages" => "reorderPages",
        "SetActivePage" => "setActivePage",
        "DeleteEmptyPage" => "deleteEmptyPage",
        "ClonePage" => "clonePage",
        "SetBindings" => "setBindings",
        "SetDisplayName" => "setDisplayName",
        "SetTopNGuard" => "setTopNGuard",
        "SetDrilldownHierarchy" => "setDrilldownHierarchy",
        "CloneVisual" => "cloneVisual",
        "DeleteVisual" => "deleteVisual",
        "UpdateFilter" => "updateFilter",
        "DeleteFilter" => "deleteFilter",
        "ClearFilter" => "clearFilter",
        "SlicerClear" => "slicerClear",
        "SetText" => "setText",
        "SetColor" => "setColor",
        "FormattingApply" => "formattingApply",
        "ApplyThemeBundle" => "applyThemeBundle",
        "ApplyStyleBundle" => "applyStyleBundle",
        "BookmarkMetadata" => "bookmarkMetadata",
        "SanitizeAction" => "sanitizeAction",
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
    fn bounded_loader_catalog_covers_every_registered_kernel() {
        for tag in crate::ops::registered_kernel_tags() {
            assert!(
                OP_KIND_CATALOG.contains(tag),
                "missing input-safety kind: {tag}"
            );
        }
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
