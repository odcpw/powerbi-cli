//! Typed operation intermediate representation.
//!
//! The public CLI commands still own their argv-shaped contracts. This module
//! is the additive seam used by the compiler and replay surfaces: kernels can
//! accept an [`Op`] without synthesizing command-line strings, while the
//! operation JSON remains stable and inspectable by agents.
//!
//! A minimal plan is a self-identifying JSON envelope:
//!
//! ```text
//! {"schema":"powerbi-cli.ops.v1","ops":[{"op":"applyThemePreset","preset":"operations"}]}
//! ```

// The operation IR is an additive internal seam. Its public consumers land in
// the compiler/replay beads, so suppress reachability noise until those
// consumers are registered without weakening clippy's correctness lints.
#![allow(dead_code)]

mod add_measure;
mod add_relationship;
mod add_visual;
mod apply_theme_preset;
mod handles;
mod io;
mod legacy;
mod model_kernels;
mod page_kernels;
mod plan;
mod set_interaction;
mod transaction;

#[allow(unused_imports)]
pub(crate) use crate::report_drillthrough::{
    SetDrillthroughKernel, parse_args as parse_set_drillthrough_args,
};
#[allow(unused_imports)]
pub(crate) use crate::report_filter_add::{AddFilterKernel, parse_args as parse_add_filter_args};
#[allow(unused_imports)]
pub(crate) use add_measure::*;
#[allow(unused_imports)]
pub(crate) use add_relationship::*;
#[allow(unused_imports)]
pub(crate) use add_visual::*;
pub(crate) use apply_theme_preset::*;
#[allow(unused_imports)]
pub(crate) use handles::*;
#[allow(unused_imports)]
pub(crate) use io::*;
pub(crate) use legacy::*;
pub(crate) use model_kernels::*;
pub(crate) use page_kernels::*;
pub(crate) use plan::*;
pub(crate) use set_interaction::*;
#[allow(unused_imports)]
pub(crate) use transaction::*;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

pub(crate) const OPS_SCHEMA: &str = "powerbi-cli.ops.v1";

/// Return the concrete kernel registered for an operation variant.
///
/// The registry is intentionally additive: each converted mutation contributes
/// one match arm while the public `ops apply` dispatcher remains a later bead.
/// Every T1b mutation registered by this bead has a kernel; the `Option`
/// return type keeps the seam compatible with pending sibling registrations and
/// future read-only IR variants.
pub(crate) fn kernel_for(operation: &Op) -> Option<Box<dyn OpKernel>> {
    match operation {
        Op::AddMeasure(_) => Some(Box::new(AddMeasureKernel)),
        Op::AddRelationship(_) => Some(Box::new(AddRelationshipKernel)),
        Op::AddVisual(_) => Some(Box::new(AddVisualKernel)),
        Op::AddFilter(_) => Some(Box::new(AddFilterKernel)),
        Op::SetDrillthrough(_) => Some(Box::new(SetDrillthroughKernel)),
        Op::SetInteraction(_) => Some(Box::new(SetInteractionKernel)),
        Op::ApplyThemePreset(_) => Some(Box::new(ApplyThemePresetKernel)),
        Op::SetObject(_) => None,
        Op::AddCalculatedColumn(_)
        | Op::AddStaticTable(_)
        | Op::SetSortBy(_)
        | Op::SourceTemplateApply(_) => Some(Box::new(ModelKernel)),
        Op::AddPage(_)
        | Op::UpdatePage(_)
        | Op::ReorderPages(_)
        | Op::SetActivePage(_)
        | Op::DeleteEmptyPage(_)
        | Op::ClonePage(_) => Some(Box::new(PageKernel)),
        Op::SetBindings(_)
        | Op::SetDisplayName(_)
        | Op::SetTopNGuard(_)
        | Op::SetDrilldownHierarchy(_)
        | Op::CloneVisual(_)
        | Op::DeleteVisual(_)
        | Op::UpdateFilter(_)
        | Op::DeleteFilter(_)
        | Op::ClearFilter(_)
        | Op::SlicerClear(_)
        | Op::SetText(_)
        | Op::SetColor(_)
        | Op::FormattingApply(_)
        | Op::ApplyThemeBundle(_)
        | Op::ApplyStyleBundle(_)
        | Op::BookmarkMetadata(_)
        | Op::SanitizeAction(_) => None,
    }
}

/// A typed operation accepted by the operation-plan compiler.
///
/// Serialization is internally tagged (`op`) and flattens each payload into
/// the operation object. A hand-written implementation is used instead of
/// `serde`'s derive because internally tagged enums cannot use tuple variants;
/// retaining tuple variants keeps the payload types additive for future
/// kernels while preserving the flat JSON contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    AddMeasure(AddMeasure),
    AddRelationship(AddRelationship),
    AddVisual(AddVisual),
    AddFilter(AddFilter),
    SetDrillthrough(SetDrillthrough),
    SetInteraction(SetInteraction),
    ApplyThemePreset(ApplyThemePreset),
    SetObject(SetObject),
    AddCalculatedColumn(AddCalculatedColumn),
    AddStaticTable(AddStaticTable),
    SetSortBy(SetSortBy),
    SourceTemplateApply(SourceTemplateApply),
    AddPage(AddPage),
    UpdatePage(UpdatePage),
    ReorderPages(ReorderPages),
    SetActivePage(SetActivePage),
    DeleteEmptyPage(DeleteEmptyPage),
    ClonePage(ClonePage),
    SetBindings(SetBindings),
    SetDisplayName(SetDisplayName),
    SetTopNGuard(SetTopNGuard),
    SetDrilldownHierarchy(SetDrilldownHierarchy),
    CloneVisual(CloneVisual),
    DeleteVisual(DeleteVisual),
    UpdateFilter(UpdateFilter),
    DeleteFilter(DeleteFilter),
    ClearFilter(ClearFilter),
    SlicerClear(SlicerClear),
    SetText(SetText),
    SetColor(SetColor),
    FormattingApply(FormattingApply),
    ApplyThemeBundle(ApplyThemeBundle),
    ApplyStyleBundle(ApplyStyleBundle),
    BookmarkMetadata(BookmarkMetadata),
    SanitizeAction(SanitizeAction),
}

/// Flattened, JSON-native payload used by mutation kernels whose command
/// options are already validated by a focused legacy parser. Keys are
/// canonical camelCase flags; repeated flags are represented as arrays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MutationPayload {
    #[serde(flatten)]
    pub(crate) fields: std::collections::BTreeMap<String, Value>,
}

// These aliases keep each operation's Rust surface named and discoverable
// while allowing the canonical command parsers to evolve without duplicating
// dozens of option fields in the IR.
pub(crate) type AddCalculatedColumn = MutationPayload;
pub(crate) type AddStaticTable = MutationPayload;
pub(crate) type SetSortBy = MutationPayload;
pub(crate) type SourceTemplateApply = MutationPayload;
pub(crate) type AddPage = MutationPayload;
pub(crate) type UpdatePage = MutationPayload;
pub(crate) type ReorderPages = MutationPayload;
pub(crate) type SetActivePage = MutationPayload;
pub(crate) type DeleteEmptyPage = MutationPayload;
pub(crate) type ClonePage = MutationPayload;
pub(crate) type SetBindings = MutationPayload;
pub(crate) type SetDisplayName = MutationPayload;
pub(crate) type SetTopNGuard = MutationPayload;
pub(crate) type SetDrilldownHierarchy = MutationPayload;
pub(crate) type CloneVisual = MutationPayload;
pub(crate) type DeleteVisual = MutationPayload;
pub(crate) type UpdateFilter = MutationPayload;
pub(crate) type DeleteFilter = MutationPayload;
pub(crate) type ClearFilter = MutationPayload;
pub(crate) type SlicerClear = MutationPayload;
pub(crate) type SetText = MutationPayload;
pub(crate) type SetColor = MutationPayload;
pub(crate) type FormattingApply = MutationPayload;
pub(crate) type ApplyThemeBundle = MutationPayload;
pub(crate) type ApplyStyleBundle = MutationPayload;
pub(crate) type BookmarkMetadata = MutationPayload;
pub(crate) type SanitizeAction = MutationPayload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddMeasure {
    pub(crate) handle: String,
    pub(crate) table: String,
    pub(crate) name: String,
    pub(crate) expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) format_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) format_string_definition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddRelationship {
    pub(crate) handle: String,
    pub(crate) from_table: String,
    pub(crate) from_column: String,
    pub(crate) to_table: String,
    pub(crate) to_column: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) from_cardinality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) to_cardinality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cross_filtering_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) is_active: Option<bool>,
}

/// Add a visual, including the `card`, `slicer`, and `textbox` shorthand
/// families. The visual kernel owns the accepted `visualType` values and
/// binding shape; this IR only carries the typed request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddVisual {
    pub(crate) handle: String,
    pub(crate) page: String,
    pub(crate) visual_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) single_select: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) position: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) bindings: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddFilter {
    pub(crate) handle: String,
    /// `report`, `page`, or `visual`; the kernel validates the closed set.
    pub(crate) scope: String,
    /// A stable owner handle (`report:main`, `page:<Name>`, or
    /// `visual:<Page>:<Container>`), except for report shorthand `report`.
    pub(crate) owner: String,
    pub(crate) filter_type: String,
    pub(crate) target: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) condition: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) values: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) relative: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetDrillthrough {
    pub(crate) page: String,
    pub(crate) target: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) column: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) keep_all_filters: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) keep_visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) back_button: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetInteraction {
    pub(crate) page: String,
    pub(crate) source: String,
    pub(crate) target: String,
    pub(crate) interaction_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyThemePreset {
    pub(crate) preset: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetObject {
    pub(crate) visual: String,
    pub(crate) object: String,
    pub(crate) property: String,
    pub(crate) value: Value,
}

/// A handle reference together with the payload field that supplied it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HandleReference<'a> {
    pub(crate) field: &'static str,
    pub(crate) handle: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OpStage {
    Model = 0,
    Page = 1,
    Visual = 2,
    Behavior = 3,
    Style = 4,
}

impl OpStage {
    pub(crate) const fn number(self) -> u8 {
        self as u8
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Page => "page",
            Self::Visual => "visual",
            Self::Behavior => "behavior",
            Self::Style => "style",
        }
    }
}

impl Op {
    pub(crate) fn tag(&self) -> &'static str {
        match self {
            Self::AddMeasure(_) => "addMeasure",
            Self::AddRelationship(_) => "addRelationship",
            Self::AddVisual(_) => "addVisual",
            Self::AddFilter(_) => "addFilter",
            Self::SetDrillthrough(_) => "setDrillthrough",
            Self::SetInteraction(_) => "setInteraction",
            Self::ApplyThemePreset(_) => "applyThemePreset",
            Self::SetObject(_) => "setObject",
            Self::AddCalculatedColumn(_) => "addCalculatedColumn",
            Self::AddStaticTable(_) => "addStaticTable",
            Self::SetSortBy(_) => "setSortBy",
            Self::SourceTemplateApply(_) => "sourceTemplateApply",
            Self::AddPage(_) => "addPage",
            Self::UpdatePage(_) => "updatePage",
            Self::ReorderPages(_) => "reorderPages",
            Self::SetActivePage(_) => "setActivePage",
            Self::DeleteEmptyPage(_) => "deleteEmptyPage",
            Self::ClonePage(_) => "clonePage",
            Self::SetBindings(_) => "setBindings",
            Self::SetDisplayName(_) => "setDisplayName",
            Self::SetTopNGuard(_) => "setTopNGuard",
            Self::SetDrilldownHierarchy(_) => "setDrilldownHierarchy",
            Self::CloneVisual(_) => "cloneVisual",
            Self::DeleteVisual(_) => "deleteVisual",
            Self::UpdateFilter(_) => "updateFilter",
            Self::DeleteFilter(_) => "deleteFilter",
            Self::ClearFilter(_) => "clearFilter",
            Self::SlicerClear(_) => "slicerClear",
            Self::SetText(_) => "setText",
            Self::SetColor(_) => "setColor",
            Self::FormattingApply(_) => "formattingApply",
            Self::ApplyThemeBundle(_) => "applyThemeBundle",
            Self::ApplyStyleBundle(_) => "applyStyleBundle",
            Self::BookmarkMetadata(_) => "bookmarkMetadata",
            Self::SanitizeAction(_) => "sanitizeAction",
        }
    }

    pub(crate) fn stage(&self) -> OpStage {
        match self {
            Self::AddMeasure(_) | Self::AddRelationship(_) => OpStage::Model,
            // There is no AddPage in T1a. SetDrillthrough is deliberately in
            // the behavior stage so it follows every visual declaration.
            Self::AddVisual(_) => OpStage::Visual,
            Self::AddFilter(_) | Self::SetInteraction(_) | Self::SetDrillthrough(_) => {
                OpStage::Behavior
            }
            Self::ApplyThemePreset(_) | Self::SetObject(_) => OpStage::Style,
            Self::AddCalculatedColumn(_)
            | Self::AddStaticTable(_)
            | Self::SetSortBy(_)
            | Self::SourceTemplateApply(_) => OpStage::Model,
            Self::AddPage(_)
            | Self::UpdatePage(_)
            | Self::ReorderPages(_)
            | Self::SetActivePage(_)
            | Self::DeleteEmptyPage(_)
            | Self::ClonePage(_) => OpStage::Page,
            Self::SetBindings(_)
            | Self::SetDisplayName(_)
            | Self::SetTopNGuard(_)
            | Self::SetDrilldownHierarchy(_)
            | Self::CloneVisual(_)
            | Self::DeleteVisual(_) => OpStage::Visual,
            Self::UpdateFilter(_)
            | Self::DeleteFilter(_)
            | Self::ClearFilter(_)
            | Self::SlicerClear(_) => OpStage::Behavior,
            Self::SetText(_)
            | Self::SetColor(_)
            | Self::FormattingApply(_)
            | Self::ApplyThemeBundle(_)
            | Self::ApplyStyleBundle(_)
            | Self::BookmarkMetadata(_) => OpStage::Style,
            Self::SanitizeAction(_) => OpStage::Behavior,
        }
    }

    pub(crate) fn declared_handle(&self) -> Option<&str> {
        match self {
            Self::AddMeasure(value) => Some(&value.handle),
            Self::AddRelationship(value) => Some(&value.handle),
            Self::AddVisual(value) => Some(&value.handle),
            Self::AddFilter(value) => Some(&value.handle),
            Self::SetDrillthrough(_)
            | Self::SetInteraction(_)
            | Self::ApplyThemePreset(_)
            | Self::SetObject(_) => None,
            Self::AddCalculatedColumn(value)
            | Self::AddStaticTable(value)
            | Self::SetSortBy(value)
            | Self::SourceTemplateApply(value)
            | Self::AddPage(value)
            | Self::UpdatePage(value)
            | Self::ReorderPages(value)
            | Self::SetActivePage(value)
            | Self::DeleteEmptyPage(value)
            | Self::ClonePage(value)
            | Self::SetBindings(value)
            | Self::SetDisplayName(value)
            | Self::SetTopNGuard(value)
            | Self::SetDrilldownHierarchy(value)
            | Self::CloneVisual(value)
            | Self::DeleteVisual(value)
            | Self::UpdateFilter(value)
            | Self::DeleteFilter(value)
            | Self::ClearFilter(value)
            | Self::SlicerClear(value)
            | Self::SetText(value)
            | Self::SetColor(value)
            | Self::FormattingApply(value)
            | Self::ApplyThemeBundle(value)
            | Self::ApplyStyleBundle(value)
            | Self::BookmarkMetadata(value)
            | Self::SanitizeAction(value) => {
                let _ = value;
                None
            }
        }
    }

    /// Return the stable handle produced by an operation, deriving it from a
    /// flattened payload when the legacy command did not accept an explicit
    /// handle flag. The owned form lets plan validation retain the same
    /// collision checks as the original typed kernels without leaking a
    /// temporary string reference.
    pub(crate) fn declared_handle_owned(&self) -> Option<String> {
        if let Some(handle) = self.declared_handle() {
            return Some(handle.to_string());
        }
        match self {
            Self::AddCalculatedColumn(payload) => Some(crate::tmdl::column_handle(
                payload_text(payload, "table")?,
                payload_text(payload, "name")?,
            )),
            Self::AddStaticTable(payload) => {
                Some(crate::tmdl::table_handle(payload_text(payload, "table")?))
            }
            Self::AddPage(payload) => Some(crate::ops::handles::page_handle(payload_text(
                payload, "name",
            )?)),
            Self::ClonePage(payload) => Some(crate::ops::handles::page_handle(payload_text(
                payload, "newName",
            )?)),
            Self::CloneVisual(payload) => {
                let page = payload_text(payload, "targetPage")
                    .or_else(|| payload_text(payload, "page"))?;
                let name = payload_text(payload, "name")?;
                Some(format!(
                    "visual:{}:{}",
                    page.trim_start_matches("page:"),
                    name
                ))
            }
            Self::AddMeasure(_)
            | Self::AddRelationship(_)
            | Self::AddVisual(_)
            | Self::AddFilter(_)
            | Self::SetDrillthrough(_)
            | Self::SetInteraction(_)
            | Self::ApplyThemePreset(_)
            | Self::SetObject(_)
            | Self::SetSortBy(_)
            | Self::SourceTemplateApply(_)
            | Self::UpdatePage(_)
            | Self::ReorderPages(_)
            | Self::SetActivePage(_)
            | Self::DeleteEmptyPage(_)
            | Self::SetBindings(_)
            | Self::SetDisplayName(_)
            | Self::SetTopNGuard(_)
            | Self::SetDrilldownHierarchy(_)
            | Self::DeleteVisual(_)
            | Self::UpdateFilter(_)
            | Self::DeleteFilter(_)
            | Self::ClearFilter(_)
            | Self::SlicerClear(_)
            | Self::SetText(_)
            | Self::SetColor(_)
            | Self::FormattingApply(_)
            | Self::ApplyThemeBundle(_)
            | Self::ApplyStyleBundle(_)
            | Self::BookmarkMetadata(_)
            | Self::SanitizeAction(_) => None,
        }
    }

    pub(crate) fn references(&self) -> Vec<HandleReference<'_>> {
        match self {
            Self::AddVisual(value) => vec![HandleReference {
                field: "page",
                handle: &value.page,
            }],
            Self::AddFilter(value) if is_handle_reference(&value.owner) => {
                vec![HandleReference {
                    field: "owner",
                    handle: &value.owner,
                }]
            }
            Self::SetDrillthrough(value) => vec![HandleReference {
                field: "page",
                handle: &value.page,
            }],
            Self::SetInteraction(value) => vec![
                HandleReference {
                    field: "page",
                    handle: &value.page,
                },
                HandleReference {
                    field: "source",
                    handle: &value.source,
                },
                HandleReference {
                    field: "target",
                    handle: &value.target,
                },
            ],
            Self::SetObject(value) => vec![HandleReference {
                field: "visual",
                handle: &value.visual,
            }],
            Self::AddMeasure(_)
            | Self::AddRelationship(_)
            | Self::AddFilter(_)
            | Self::ApplyThemePreset(_)
            | Self::AddCalculatedColumn(_)
            | Self::AddStaticTable(_)
            | Self::SetSortBy(_)
            | Self::SourceTemplateApply(_)
            | Self::AddPage(_)
            | Self::UpdatePage(_)
            | Self::ReorderPages(_)
            | Self::SetActivePage(_)
            | Self::DeleteEmptyPage(_)
            | Self::ClonePage(_)
            | Self::SetBindings(_)
            | Self::SetDisplayName(_)
            | Self::SetTopNGuard(_)
            | Self::SetDrilldownHierarchy(_)
            | Self::CloneVisual(_)
            | Self::DeleteVisual(_)
            | Self::UpdateFilter(_)
            | Self::DeleteFilter(_)
            | Self::ClearFilter(_)
            | Self::SlicerClear(_)
            | Self::SetText(_)
            | Self::SetColor(_)
            | Self::FormattingApply(_)
            | Self::ApplyThemeBundle(_)
            | Self::ApplyStyleBundle(_)
            | Self::BookmarkMetadata(_)
            | Self::SanitizeAction(_) => Vec::new(),
        }
    }

    /// A deterministic key used by kernels to make replay idempotent.
    pub(crate) fn idempotent_key(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| self.tag().to_string())
    }
}

fn payload_text<'a>(payload: &'a MutationPayload, key: &str) -> Option<&'a str> {
    payload.fields.get(key).and_then(Value::as_str)
}

fn is_handle_reference(value: &str) -> bool {
    value.starts_with("report:") || value.starts_with("page:") || value.starts_with("visual:")
}

impl Serialize for Op {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::AddMeasure(value) => serialize_tagged(self.tag(), value, serializer),
            Self::AddRelationship(value) => serialize_tagged(self.tag(), value, serializer),
            Self::AddVisual(value) => serialize_tagged(self.tag(), value, serializer),
            Self::AddFilter(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetDrillthrough(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetInteraction(value) => serialize_tagged(self.tag(), value, serializer),
            Self::ApplyThemePreset(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetObject(value) => serialize_tagged(self.tag(), value, serializer),
            Self::AddCalculatedColumn(value) => serialize_tagged(self.tag(), value, serializer),
            Self::AddStaticTable(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetSortBy(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SourceTemplateApply(value) => serialize_tagged(self.tag(), value, serializer),
            Self::AddPage(value) => serialize_tagged(self.tag(), value, serializer),
            Self::UpdatePage(value) => serialize_tagged(self.tag(), value, serializer),
            Self::ReorderPages(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetActivePage(value) => serialize_tagged(self.tag(), value, serializer),
            Self::DeleteEmptyPage(value) => serialize_tagged(self.tag(), value, serializer),
            Self::ClonePage(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetBindings(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetDisplayName(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetTopNGuard(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetDrilldownHierarchy(value) => serialize_tagged(self.tag(), value, serializer),
            Self::CloneVisual(value) => serialize_tagged(self.tag(), value, serializer),
            Self::DeleteVisual(value) => serialize_tagged(self.tag(), value, serializer),
            Self::UpdateFilter(value) => serialize_tagged(self.tag(), value, serializer),
            Self::DeleteFilter(value) => serialize_tagged(self.tag(), value, serializer),
            Self::ClearFilter(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SlicerClear(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetText(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SetColor(value) => serialize_tagged(self.tag(), value, serializer),
            Self::FormattingApply(value) => serialize_tagged(self.tag(), value, serializer),
            Self::ApplyThemeBundle(value) => serialize_tagged(self.tag(), value, serializer),
            Self::ApplyStyleBundle(value) => serialize_tagged(self.tag(), value, serializer),
            Self::BookmarkMetadata(value) => serialize_tagged(self.tag(), value, serializer),
            Self::SanitizeAction(value) => serialize_tagged(self.tag(), value, serializer),
        }
    }
}

fn serialize_tagged<T, S>(tag: &str, payload: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: Serializer,
{
    let mut object = serde_json::to_value(payload)
        .map_err(serde::ser::Error::custom)?
        .as_object()
        .cloned()
        .ok_or_else(|| serde::ser::Error::custom("operation payload must be an object"))?;
    object.insert("op".to_string(), Value::String(tag.to_string()));
    Value::Object(object).serialize(serializer)
}

impl<'de> Deserialize<'de> for Op {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let mut object = value
            .as_object()
            .cloned()
            .ok_or_else(|| serde::de::Error::custom("operation must be a JSON object"))?;
        let tag = object
            .remove("op")
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| {
                serde::de::Error::custom("operation object requires string field `op`")
            })?;
        let payload = Value::Object(object);
        deserialize_tagged(&tag, payload).map_err(serde::de::Error::custom)
    }
}

fn deserialize_tagged(tag: &str, payload: Value) -> Result<Op, String> {
    match tag {
        "addMeasure" => serde_json::from_value(payload)
            .map(Op::AddMeasure)
            .map_err(|error| format!("invalid addMeasure operation: {error}")),
        "addRelationship" => serde_json::from_value(payload)
            .map(Op::AddRelationship)
            .map_err(|error| format!("invalid addRelationship operation: {error}")),
        "addVisual" => serde_json::from_value(payload)
            .map(Op::AddVisual)
            .map_err(|error| format!("invalid addVisual operation: {error}")),
        "addFilter" => serde_json::from_value(payload)
            .map(Op::AddFilter)
            .map_err(|error| format!("invalid addFilter operation: {error}")),
        "setDrillthrough" => serde_json::from_value(payload)
            .map(Op::SetDrillthrough)
            .map_err(|error| format!("invalid setDrillthrough operation: {error}")),
        "setInteraction" => serde_json::from_value(payload)
            .map(Op::SetInteraction)
            .map_err(|error| format!("invalid setInteraction operation: {error}")),
        "applyThemePreset" => serde_json::from_value(payload)
            .map(Op::ApplyThemePreset)
            .map_err(|error| format!("invalid applyThemePreset operation: {error}")),
        "setObject" => serde_json::from_value(payload)
            .map(Op::SetObject)
            .map_err(|error| format!("invalid setObject operation: {error}")),
        "addCalculatedColumn" => serde_json::from_value(payload)
            .map(Op::AddCalculatedColumn)
            .map_err(|error| format!("invalid addCalculatedColumn operation: {error}")),
        "addStaticTable" => serde_json::from_value(payload)
            .map(Op::AddStaticTable)
            .map_err(|error| format!("invalid addStaticTable operation: {error}")),
        "setSortBy" => serde_json::from_value(payload)
            .map(Op::SetSortBy)
            .map_err(|error| format!("invalid setSortBy operation: {error}")),
        "sourceTemplateApply" => serde_json::from_value(payload)
            .map(Op::SourceTemplateApply)
            .map_err(|error| format!("invalid sourceTemplateApply operation: {error}")),
        "addPage" => serde_json::from_value(payload)
            .map(Op::AddPage)
            .map_err(|error| format!("invalid addPage operation: {error}")),
        "updatePage" => serde_json::from_value(payload)
            .map(Op::UpdatePage)
            .map_err(|error| format!("invalid updatePage operation: {error}")),
        "reorderPages" => serde_json::from_value(payload)
            .map(Op::ReorderPages)
            .map_err(|error| format!("invalid reorderPages operation: {error}")),
        "setActivePage" => serde_json::from_value(payload)
            .map(Op::SetActivePage)
            .map_err(|error| format!("invalid setActivePage operation: {error}")),
        "deleteEmptyPage" => serde_json::from_value(payload)
            .map(Op::DeleteEmptyPage)
            .map_err(|error| format!("invalid deleteEmptyPage operation: {error}")),
        "clonePage" => serde_json::from_value(payload)
            .map(Op::ClonePage)
            .map_err(|error| format!("invalid clonePage operation: {error}")),
        "setBindings" => serde_json::from_value(payload)
            .map(Op::SetBindings)
            .map_err(|error| format!("invalid setBindings operation: {error}")),
        "setDisplayName" => serde_json::from_value(payload)
            .map(Op::SetDisplayName)
            .map_err(|error| format!("invalid setDisplayName operation: {error}")),
        "setTopNGuard" => serde_json::from_value(payload)
            .map(Op::SetTopNGuard)
            .map_err(|error| format!("invalid setTopNGuard operation: {error}")),
        "setDrilldownHierarchy" => serde_json::from_value(payload)
            .map(Op::SetDrilldownHierarchy)
            .map_err(|error| format!("invalid setDrilldownHierarchy operation: {error}")),
        "cloneVisual" => serde_json::from_value(payload)
            .map(Op::CloneVisual)
            .map_err(|error| format!("invalid cloneVisual operation: {error}")),
        "deleteVisual" => serde_json::from_value(payload)
            .map(Op::DeleteVisual)
            .map_err(|error| format!("invalid deleteVisual operation: {error}")),
        "updateFilter" => serde_json::from_value(payload)
            .map(Op::UpdateFilter)
            .map_err(|error| format!("invalid updateFilter operation: {error}")),
        "deleteFilter" => serde_json::from_value(payload)
            .map(Op::DeleteFilter)
            .map_err(|error| format!("invalid deleteFilter operation: {error}")),
        "clearFilter" => serde_json::from_value(payload)
            .map(Op::ClearFilter)
            .map_err(|error| format!("invalid clearFilter operation: {error}")),
        "slicerClear" => serde_json::from_value(payload)
            .map(Op::SlicerClear)
            .map_err(|error| format!("invalid slicerClear operation: {error}")),
        "setText" => serde_json::from_value(payload)
            .map(Op::SetText)
            .map_err(|error| format!("invalid setText operation: {error}")),
        "setColor" => serde_json::from_value(payload)
            .map(Op::SetColor)
            .map_err(|error| format!("invalid setColor operation: {error}")),
        "formattingApply" => serde_json::from_value(payload)
            .map(Op::FormattingApply)
            .map_err(|error| format!("invalid formattingApply operation: {error}")),
        "applyThemeBundle" => serde_json::from_value(payload)
            .map(Op::ApplyThemeBundle)
            .map_err(|error| format!("invalid applyThemeBundle operation: {error}")),
        "applyStyleBundle" => serde_json::from_value(payload)
            .map(Op::ApplyStyleBundle)
            .map_err(|error| format!("invalid applyStyleBundle operation: {error}")),
        "bookmarkMetadata" => serde_json::from_value(payload)
            .map(Op::BookmarkMetadata)
            .map_err(|error| format!("invalid bookmarkMetadata operation: {error}")),
        "sanitizeAction" => serde_json::from_value(payload)
            .map(Op::SanitizeAction)
            .map_err(|error| format!("invalid sanitizeAction operation: {error}")),
        other => Err(format!("unsupported operation tag `{other}`")),
    }
}

/// JSON Schema for an operation plan. The oneOf list covers the original T1a
/// kernels and the T1b mutation families; read-only commands stay outside it.
pub(crate) fn schema_json() -> Value {
    let operation = |tag: &str, properties: Value, required: &[&str]| {
        let mut value = serde_json::json!({
            "type": "object",
            "properties": {
                "op": {"const": tag},
            },
            "required": ["op"],
            "additionalProperties": true
        });
        if let Some(object) = properties.as_object() {
            for (key, property) in object {
                value["properties"][key] = property.clone();
            }
        }
        if let Some(required_values) = value["required"].as_array_mut() {
            required_values.extend(required.iter().map(|field| Value::String((*field).into())));
        }
        value
    };
    serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": OPS_SCHEMA,
        "schema": OPS_SCHEMA,
        "title": "powerbi-cli operation plan",
        "type": "object",
        "additionalProperties": false,
        "required": ["schema", "ops"],
        "properties": {
            "schema": {"const": OPS_SCHEMA},
            "ops": {
                "type": "array",
                "items": {
                    "oneOf": [
                        operation("addMeasure", serde_json::json!({
                            "handle": {"type": "string"}, "table": {"type": "string"},
                            "name": {"type": "string"}, "expression": {"type": "string"},
                            "formatString": {"type": "string"},
                            "formatStringDefinition": {"type": "string"}
                        }), &["handle", "table", "name", "expression"]),
                        operation("addRelationship", serde_json::json!({
                            "handle": {"type": "string"}, "fromTable": {"type": "string"},
                            "fromColumn": {"type": "string"}, "toTable": {"type": "string"},
                            "toColumn": {"type": "string"}
                        }), &["handle", "fromTable", "fromColumn", "toTable", "toColumn"]),
                        operation("addVisual", serde_json::json!({
                            "handle": {"type": "string"}, "page": {"type": "string"},
                            "visualType": {"type": "string"}
                        }), &["handle", "page", "visualType"]),
                        operation("addFilter", serde_json::json!({
                            "handle": {"type": "string"}, "scope": {"type": "string"},
                            "owner": {"type": "string"}, "filterType": {"type": "string"},
                            "target": {}, "name": {"type": "string"},
                            "displayName": {"type": "string"}
                        }), &["handle", "scope", "owner", "filterType", "target"]),
                        operation("setDrillthrough", serde_json::json!({
                            "page": {"type": "string"}, "target": {"type": "string"},
                            "fields": {"type": "array", "items": {"type": "string"}},
                            "table": {"type": "string"}, "column": {"type": "string"},
                            "keepAllFilters": {"type": "boolean"},
                            "keepVisible": {"type": "boolean"},
                            "hidden": {"type": "boolean"}, "backButton": {"type": "boolean"}
                        }), &["page", "target"]),
                        operation("setInteraction", serde_json::json!({
                            "page": {"type": "string"}, "source": {"type": "string"},
                            "target": {"type": "string"}, "interactionType": {"type": "string"}
                        }), &["page", "source", "target", "interactionType"]),
                        operation("applyThemePreset", serde_json::json!({
                            "preset": {"type": "string"}
                        }), &["preset"]),
                        operation("setObject", serde_json::json!({
                            "visual": {"type": "string"}, "object": {"type": "string"},
                            "property": {"type": "string"}, "value": {}
                        }), &["visual", "object", "property", "value"]),
                        // T1b mutation payloads intentionally allow the
                        // focused command catalog to add fields without
                        // changing the operation envelope.
                        operation("addCalculatedColumn", serde_json::json!({}), &[]),
                        operation("addStaticTable", serde_json::json!({}), &[]),
                        operation("setSortBy", serde_json::json!({}), &[]),
                        operation("sourceTemplateApply", serde_json::json!({}), &[]),
                        operation("addPage", serde_json::json!({}), &[]),
                        operation("updatePage", serde_json::json!({}), &[]),
                        operation("reorderPages", serde_json::json!({}), &[]),
                        operation("setActivePage", serde_json::json!({}), &[]),
                        operation("deleteEmptyPage", serde_json::json!({}), &[]),
                        operation("clonePage", serde_json::json!({}), &[]),
                        operation("setBindings", serde_json::json!({}), &[]),
                        operation("setDisplayName", serde_json::json!({}), &[]),
                        operation("setTopNGuard", serde_json::json!({}), &[]),
                        operation("setDrilldownHierarchy", serde_json::json!({}), &[]),
                        operation("cloneVisual", serde_json::json!({}), &[]),
                        operation("deleteVisual", serde_json::json!({}), &[]),
                        operation("updateFilter", serde_json::json!({}), &[]),
                        operation("deleteFilter", serde_json::json!({}), &[]),
                        operation("clearFilter", serde_json::json!({}), &[]),
                        operation("slicerClear", serde_json::json!({}), &[]),
                        operation("setText", serde_json::json!({}), &[]),
                        operation("setColor", serde_json::json!({}), &[]),
                        operation("formattingApply", serde_json::json!({}), &[]),
                        operation("applyThemeBundle", serde_json::json!({}), &[]),
                        operation("applyStyleBundle", serde_json::json!({}), &[]),
                        operation("bookmarkMetadata", serde_json::json!({}), &[]),
                        operation("sanitizeAction", serde_json::json!({}), &[]),
                    ]
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variants() -> Vec<Op> {
        vec![
            Op::AddMeasure(AddMeasure {
                handle: "measure:Sales:Revenue".into(),
                table: "Sales".into(),
                name: "Revenue".into(),
                expression: "SUM(Sales[Revenue])".into(),
                format_string: None,
                format_string_definition: Some("[FormatString]".into()),
                description: Some("Revenue measure".into()),
                display_folder: Some("Executive".into()),
            }),
            Op::AddRelationship(AddRelationship {
                handle: "relationship:Sales.Customer:Customers.Id".into(),
                from_table: "Sales".into(),
                from_column: "Customer".into(),
                to_table: "Customers".into(),
                to_column: "Id".into(),
                from_cardinality: Some("many".into()),
                to_cardinality: Some("one".into()),
                cross_filtering_behavior: Some("oneDirection".into()),
                is_active: Some(true),
            }),
            Op::AddVisual(AddVisual {
                handle: "visual:ReportSectionOverview:VisualContainerRevenue".into(),
                page: "page:ReportSectionOverview".into(),
                visual_type: "card".into(),
                name: Some("Revenue".into()),
                title: Some("Revenue".into()),
                mode: None,
                single_select: None,
                position: Some(serde_json::json!({"x": 1})),
                bindings: vec![serde_json::json!({"role": "Values"})],
            }),
            Op::AddFilter(AddFilter {
                handle: "filter:page:ReportSectionOverview:RevenueFilter".into(),
                scope: "page".into(),
                owner: "page:ReportSectionOverview".into(),
                filter_type: "Categorical".into(),
                target: serde_json::json!({"table": "Customers", "column": "Segment"}),
                name: Some("RevenueFilter".into()),
                display_name: None,
                condition: Some(serde_json::json!({"values": ["Enterprise"]})),
                values: vec![serde_json::json!("Enterprise")],
                relative: None,
            }),
            Op::SetDrillthrough(SetDrillthrough {
                page: "page:ReportSectionDetail".into(),
                target: "Customers[CustomerId]".into(),
                fields: vec!["Customers[CustomerId]".into()],
                table: Some("Customers".into()),
                column: Some("CustomerId".into()),
                keep_all_filters: Some(false),
                keep_visible: Some(true),
                hidden: Some(false),
                back_button: Some(true),
            }),
            Op::SetInteraction(SetInteraction {
                page: "page:ReportSectionOverview".into(),
                source: "visual:ReportSectionOverview:VisualContainerRevenue".into(),
                target: "visual:ReportSectionOverview:VisualContainerTable".into(),
                interaction_type: "DataFilter".into(),
            }),
            Op::ApplyThemePreset(ApplyThemePreset {
                preset: "operations".into(),
            }),
            Op::SetObject(SetObject {
                visual: "visual:ReportSectionOverview:VisualContainerRevenue".into(),
                object: "title".into(),
                property: "text".into(),
                value: serde_json::json!("Revenue"),
            }),
        ]
    }

    #[test]
    fn every_t1a_operation_round_trips_as_flat_ops_v1_json() {
        for operation in variants() {
            let value = serde_json::to_value(&operation).expect("serialize operation");
            assert_eq!(value["op"].as_str(), Some(operation.tag()));
            let decoded: Op = serde_json::from_value(value).expect("deserialize operation");
            assert_eq!(decoded, operation);
        }
    }

    #[test]
    fn schema_identifies_the_closed_operation_plan_contract() {
        let schema = schema_json();
        assert_eq!(schema["$id"], OPS_SCHEMA);
        assert_eq!(schema["schema"], OPS_SCHEMA);
        assert_eq!(schema["properties"]["schema"]["const"], OPS_SCHEMA);
        assert_eq!(schema["required"], serde_json::json!(["schema", "ops"]));
        assert_eq!(schema["properties"]["ops"]["type"], "array");
        assert_eq!(
            schema["properties"]["ops"]["items"]["oneOf"]
                .as_array()
                .map(Vec::len),
            Some(35)
        );
    }

    #[test]
    fn operation_plan_serializes_as_a_self_identifying_ops_v1_envelope() {
        let plan = OpPlan::new(vec![Op::ApplyThemePreset(ApplyThemePreset {
            preset: "operations".into(),
        })]);
        let value = serde_json::to_value(plan).expect("operation plan json");
        assert_eq!(value["schema"], OPS_SCHEMA);
        assert_eq!(value["ops"][0]["op"], "applyThemePreset");
        assert_eq!(value["ops"][0]["preset"], "operations");
    }
}
