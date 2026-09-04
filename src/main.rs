#![recursion_limit = "512"]

mod bridge;
mod calculated_columns;
mod child_process;
mod cli;
mod cli_error;
mod cli_support;
mod contract;
mod dashboard_scaffold;
mod dax_execute;
mod desktop;
mod desktop_proof;
mod desktop_session;
mod desktop_target;
mod diagnostics;
mod diff;
mod doctor;
mod feature_catalog;
mod fixture;
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
mod project_io;
mod project_resolution;
#[cfg(test)]
mod project_resolution_tests;
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
mod report_slicer_clear;
mod report_slicers;
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
mod rules;
mod safety_scan;
mod scaffold;
mod schema;
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
pub(crate) use scaffold::{
    PBIP_SCHEMA, REPORT_DEFINITION_SCHEMA, SEMANTIC_MODEL_DEFINITION_SCHEMA, scaffold_command,
    scaffold_schema_value,
};
pub(crate) use validation::{
    ValidationReport, report_schema_major, validate_command, validate_desktop_runtime_project,
    validate_project,
};

pub(crate) use crate::project_resolution::{ResolvedProject, canonical_display, resolve_project};

fn main() {
    cli::main_entry();
}
