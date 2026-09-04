#![recursion_limit = "512"]
#![allow(dead_code)]

mod bridge;
mod calculated_columns;
mod child_process;
mod cli;
mod cli_error;
mod cli_support;
mod contract;
mod dashboard_scaffold;
mod dax_execute;
mod design;
mod desktop;
mod desktop_proof;
mod desktop_session;
mod desktop_target;
mod diagnostics;
mod diff;
mod doctor;
mod feature_catalog;
mod fixture;
mod formatting_catalog;
mod guid_util;
mod handoff;
mod handoff_rebind_check;
mod help;
mod input_safety;
mod inspect;
mod json_composition;
mod json_io;
mod lint;
mod live_model;
mod mcp;
mod measures;
mod microsoft;
mod model;
mod model_advanced;
mod model_dax;
mod model_expressions;
mod model_live;
mod model_partitions_grouped_rank;
mod model_tables;
mod ops;
mod package;
mod partitions;
mod pbir;
mod pbir_bindings;
mod pbir_bookmarks;
mod pbir_filters;
mod pbir_interactions;
mod pbir_slicers;
mod pbir_themes;
mod pbir_visual_factory;
mod profile;
mod profile_shape;
mod project_io;
mod project_resolution;
mod rebind_plan;
mod relationship_tmdl;
mod relationships;
mod report;
mod report_bookmarks;
mod report_build;
mod report_conditional_formatting;
mod report_design;
mod report_drilldown;
mod report_drillthrough;
mod report_filter_add;
mod report_filter_clear;
mod report_filter_mutations;
mod report_filter_shapes;
mod report_filter_update;
mod report_filters;
mod report_hygiene;
mod report_interaction_mutations;
mod report_interactions;
mod report_layout;
mod report_objects;
mod report_page_mutations;
mod report_pages;
mod report_plan;
mod report_proof;
mod report_slicer_clear;
mod report_slicers;
mod report_spec_explain;
mod report_spec_fields;
mod report_spec_normalize;
mod report_spec_schema;
mod report_spec_upgrade;
mod report_style;
mod report_themes;
mod report_topn_guard;
mod report_visual_binding_repair;
mod report_visual_clone;
mod report_visual_delete;
mod report_visual_formatting;
mod report_visual_formatting_bundle;
mod report_visual_formatting_color;
mod report_visual_formatting_text;
mod report_visual_mutations;
mod report_visual_objects;
mod report_visual_scaffold;
mod report_visuals;
mod report_wireframe;
mod robot_docs;
mod rules;
mod safety_scan;
mod scaffold;
mod schema;
mod scorecard;
mod skill_package;
mod source_template;
mod source_template_paths;
mod source_templates;
mod static_tables;
mod tmdl;
mod triage;
mod validation;
mod visual_catalog;
mod workflow;

pub(crate) use cli_error::*;
pub(crate) use cli_support::command_arg;
pub(crate) use diagnostics::Finding;
pub(crate) use doctor::doctor_json;
pub(crate) use inspect::inspect_command;
pub(crate) use json_io::read_json_value;
pub(crate) use project_resolution::{ResolvedProject, canonical_display, resolve_project};
pub(crate) use scaffold::{
    PBIP_SCHEMA, REPORT_DEFINITION_SCHEMA, SEMANTIC_MODEL_DEFINITION_SCHEMA, scaffold_command,
    scaffold_schema_value,
};
pub(crate) use validation::{
    ValidationReport, report_schema_major, validate_command, validate_desktop_runtime_project,
    validate_project,
};

/// Internal integration-test bridge for exercising the typed operation path.
///
/// This is intentionally a library-only helper rather than a CLI command. It
/// starts the same working-copy transaction and kernel registry used by the
/// future ops apply command, allowing artifact-equivalence tests to run
/// before that command lands.
#[doc(hidden)]
pub mod test_support {
    use super::ops::{Op, OpPlan, ProjectIndex, kernel_for};
    use super::{CliError, ResolvedProject, resolve_project};
    use serde_json::{Value, json};
    use std::path::Path;

    /// Apply one serialized ops.v1 operation to project, committing the
    /// validated transaction to out_dir and returning a JSON receipt.
    pub fn apply_operation_to_out_dir(
        operation: Value,
        project: &Path,
        out_dir: &Path,
    ) -> Result<Value, Value> {
        let operation: Op = serde_json::from_value(operation).map_err(|error| {
            error_value(&CliError::invalid_args(format!(
                "decode operation: {error}"
            )))
        })?;
        let source: ResolvedProject =
            resolve_project(project).map_err(|error| error_value(&error))?;
        let index = ProjectIndex::from_project(&source).map_err(|error| error_value(&error))?;
        let plan = OpPlan::new(vec![operation.clone()]);
        let validated = plan
            .validate(&index)
            .map_err(|error| error_value(&error.as_cli_error()))?;
        let mut transaction =
            super::ops::Transaction::begin(source).map_err(|error| error_value(&error))?;
        let mut kernel = kernel_for(&operation).ok_or_else(|| {
            error_value(&CliError::unsupported_feature(format!(
                "no registered kernel for {}",
                operation.tag()
            )))
        })?;
        let receipt = transaction
            .apply_all(&validated, kernel.as_mut())
            .map_err(|failure| error_value(&failure.error))?;
        let commit = transaction
            .commit_out_dir(out_dir, false)
            .map_err(|error| error_value(&error))?;
        Ok(json!({
            "ok": true,
            "operation": serde_json::to_value(operation)
                .map_err(|error| format!("serialize operation: {error}"))?,
            "receipt": serde_json::to_value(receipt)
                .map_err(|error| format!("serialize operation receipt: {error}"))?,
            "commit": serde_json::to_value(commit)
                .map_err(|error| format!("serialize operation commit: {error}"))?,
        }))
    }

    fn error_value(error: &CliError) -> Value {
        let mut object = serde_json::Map::new();
        object.insert("code".to_string(), Value::String(error.code.to_string()));
        object.insert("exitCode".to_string(), Value::from(error.exit_code));
        object.insert("message".to_string(), Value::String(error.message.clone()));
        if let Some(hint) = &error.hint {
            object.insert("hint".to_string(), Value::String(hint.clone()));
        }
        if !error.suggested_commands.is_empty() {
            object.insert(
                "suggestedCommands".to_string(),
                Value::Array(
                    error
                        .suggested_commands
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(pointer) = error.pointer() {
            object.insert("pointer".to_string(), Value::String(pointer.to_string()));
        }
        if let Some(did_you_mean) = error.did_you_mean() {
            object.insert(
                "didYouMean".to_string(),
                Value::String(did_you_mean.to_string()),
            );
        }
        if let Some(field) = error.field() {
            object.insert("field".to_string(), Value::String(field.to_string()));
        }
        if let Some(reason) = error.reason() {
            object.insert("reason".to_string(), Value::String(reason.to_string()));
        }
        if let Some(candidates_command) = error.candidates_command() {
            object.insert(
                "candidatesCommand".to_string(),
                Value::String(candidates_command.to_string()),
            );
        }
        if let Some(example) = error.example() {
            object.insert("example".to_string(), example.clone());
        }
        json!({"error": object})
    }

    /// Return operation tags with a registered typed kernel in this build.
    pub fn registered_kernel_tags() -> &'static [&'static str] {
        super::ops::registered_kernel_tags()
    }
}
