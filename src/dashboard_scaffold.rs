//! Dashboard page and visual scaffolding with manifest binding adaptation.

use crate::pbir_bindings::{VisualBindingKind, VisualBindingResolved, resolved_binding_kind};
use crate::pbir_visual_factory::{VisualBuildSpec, resolve_slicer_mode, visual_container_json};
use crate::scaffold::{DashboardSpec, normalize_data_type, object_name};
use crate::{CliResult, report_visual_mutations};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageSpec {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) width: Option<f64>,
    #[serde(default)]
    pub(super) height: Option<f64>,
    #[serde(default)]
    pub(super) visuals: Vec<VisualSpec>,
    #[serde(default)]
    pub(super) interactions: Vec<VisualInteractionSpec>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct VisualInteractionSpec {
    pub(super) source: String,
    pub(super) target: String,
    #[serde(rename = "type")]
    pub(super) interaction_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VisualSpec {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) visual_type: Option<String>,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) single_select: bool,
    #[serde(default)]
    pub(super) bindings: Vec<VisualBindingSpec>,
    #[serde(default)]
    pub(super) x: Option<f64>,
    #[serde(default)]
    pub(super) y: Option<f64>,
    #[serde(default)]
    pub(super) width: Option<f64>,
    #[serde(default)]
    pub(super) height: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VisualBindingSpec {
    pub(super) role: String,
    pub(super) table: String,
    #[serde(default)]
    pub(super) column: Option<String>,
    #[serde(default)]
    pub(super) measure: Option<String>,
    #[serde(default)]
    pub(super) display_name: Option<String>,
    #[serde(default)]
    pub(super) format_string: Option<String>,
    #[serde(default)]
    pub(super) sort_direction: Option<String>,
}

pub(super) fn effective_pages(spec: &DashboardSpec) -> Vec<PageSpec> {
    if spec.pages.is_empty() {
        vec![PageSpec {
            name: Some("ReportSectionOverview".to_string()),
            display_name: Some("Overview".to_string()),
            width: Some(1280.0),
            height: Some(720.0),
            // A blank page is valid PBIR. Inventing data visuals without model bindings is not:
            // Microsoft's consumed report surface rejects them with PBIR_QUERY_STATE_MISSING.
            visuals: Vec::new(),
            interactions: Vec::new(),
        }]
    } else {
        spec.pages.clone()
    }
}

impl Clone for PageSpec {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            width: self.width,
            height: self.height,
            visuals: self.visuals.clone(),
            interactions: self.interactions.clone(),
        }
    }
}

impl Clone for VisualSpec {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            visual_type: self.visual_type.clone(),
            title: self.title.clone(),
            mode: self.mode.clone(),
            single_select: self.single_select,
            bindings: self.bindings.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

impl Clone for VisualBindingSpec {
    fn clone(&self) -> Self {
        Self {
            role: self.role.clone(),
            table: self.table.clone(),
            column: self.column.clone(),
            measure: self.measure.clone(),
            display_name: self.display_name.clone(),
            format_string: self.format_string.clone(),
            sort_direction: self.sort_direction.clone(),
        }
    }
}

pub(super) fn visual_json(
    dashboard: &DashboardSpec,
    visual: &VisualSpec,
    visual_index: usize,
) -> CliResult<Value> {
    let title = visual
        .title
        .clone()
        .unwrap_or_else(|| format!("Visual {}", visual_index + 1));
    let visual_type = visual
        .visual_type
        .clone()
        .unwrap_or_else(|| "card".to_string());
    let bindings = visual
        .bindings
        .iter()
        .map(|binding| scaffold_visual_binding(dashboard, &visual_type, binding))
        .collect::<CliResult<Vec<_>>>()?;
    report_visual_mutations::validate_binding_cardinality(&visual_type, &bindings)?;
    crate::pbir_bindings::validate_sort_bindings(&bindings)?;
    let slicer_mode = resolve_slicer_mode(&visual_type, visual.mode.as_deref())?;
    visual_container_json(&VisualBuildSpec {
        name: visual
            .name
            .clone()
            .unwrap_or_else(|| object_name("VisualContainer", &title, visual_index)),
        title,
        visual_type,
        bindings,
        slicer_mode,
        slicer_single_select: visual.single_select,
        x: visual.x.unwrap_or(40.0 + (visual_index as f64 * 40.0)),
        y: visual.y.unwrap_or(40.0 + (visual_index as f64 * 40.0)),
        z: visual_index as u64,
        height: visual.height.unwrap_or(180.0),
        width: visual.width.unwrap_or(320.0),
        tab_order: visual_index as u64,
    })
}

fn scaffold_visual_binding(
    dashboard: &DashboardSpec,
    visual_type: &str,
    binding: &VisualBindingSpec,
) -> CliResult<VisualBindingResolved> {
    if let Some(measure) = &binding.measure {
        Ok(VisualBindingResolved {
            role: binding.role.clone(),
            table: binding.table.clone(),
            field: measure.clone(),
            kind: resolved_binding_kind(visual_type, &binding.role, true)?,
            data_type: None,
            display_name: binding.display_name.clone(),
            format_string: binding.format_string.clone(),
            sort_direction: binding.sort_direction.clone(),
        })
    } else if let Some(column) = &binding.column {
        Ok(VisualBindingResolved {
            role: binding.role.clone(),
            table: binding.table.clone(),
            field: column.clone(),
            kind: resolved_binding_kind(visual_type, &binding.role, false)?,
            data_type: dashboard
                .tables
                .iter()
                .find(|table| table.name.eq_ignore_ascii_case(&binding.table))
                .and_then(|table| {
                    table
                        .columns
                        .iter()
                        .find(|candidate| candidate.name.eq_ignore_ascii_case(column))
                })
                .and_then(|column| normalize_data_type(column.data_type.as_deref()).ok())
                .map(|data_type| data_type.tmdl.to_string()),
            display_name: binding.display_name.clone(),
            format_string: binding.format_string.clone(),
            sort_direction: binding.sort_direction.clone(),
        })
    } else {
        Ok(VisualBindingResolved {
            role: binding.role.clone(),
            table: binding.table.clone(),
            field: "<invalid>".to_string(),
            kind: VisualBindingKind::Column,
            data_type: None,
            display_name: binding.display_name.clone(),
            format_string: binding.format_string.clone(),
            sort_direction: binding.sort_direction.clone(),
        })
    }
}
