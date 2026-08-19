//! Agent-facing CLI contract façade.
//!
//! Capability descriptors are split by command family while this module keeps
//! every existing `crate::contract` path stable.

mod core;
mod desktop;
mod integrations;
mod model;
mod report;
mod workflow_pkg;

pub(crate) use core::{
    CONTRACT_VERSION, capabilities, command_catalog, help_json, help_text, robot_docs_json,
    robot_docs_markdown, robot_triage, suggested_command_path,
};
