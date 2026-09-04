use crate::help::edit_distance;
use crate::{CliError, CliResult, EXIT_VALIDATION_FAILED};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub(crate) const DASHBOARD_V1: &str = "powerbi-cli.dashboard.v1";
pub(crate) const DASHBOARD_V2: &str = "powerbi-cli.dashboard.v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpecVersion {
    V1,
    V2,
}

#[derive(Clone, Copy)]
struct NodeSchema {
    name: &'static str,
    path: &'static str,
    fields: &'static [&'static str],
}

const ROOT_V1: NodeSchema = node(
    "root",
    "",
    &["schema", "report", "model", "pages", "style", "proof"],
);
const ROOT_V2: NodeSchema = node(
    "root",
    "",
    &[
        "schema", "report", "model", "style", "layout", "filters", "pages", "proof",
    ],
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

const MODEL_V2: NodeSchema = node(
    "model",
    "model",
    &[
        "measures",
        "measurePatterns",
        "calculatedColumns",
        "relationships",
        "staticTables",
        "dateTable",
        "sortBy",
        "formatStrings",
    ],
);
const MODEL_MEASURE_V2: NodeSchema = node(
    "model.measures[]",
    "model.measures",
    &[
        "table",
        "name",
        "expression",
        "expressionFile",
        "formatString",
        "formatStringExpression",
        "displayFolder",
        "description",
    ],
);
const MEASURE_PATTERN: NodeSchema = node(
    "model.measurePatterns[]",
    "model.measurePatterns",
    &["pattern", "base", "date", "name"],
);
const CALCULATED_COLUMN: NodeSchema = node(
    "model.calculatedColumns[]",
    "model.calculatedColumns",
    &["table", "name", "expression", "dataType", "formatString"],
);
const RELATIONSHIP: NodeSchema = node(
    "model.relationships[]",
    "model.relationships",
    &[
        "from",
        "to",
        "cardinality",
        "crossFilteringBehavior",
        "isActive",
    ],
);
const STATIC_TABLE: NodeSchema = node(
    "model.staticTables[]",
    "model.staticTables",
    &["name", "columns", "rows"],
);
const DATE_TABLE: NodeSchema = node(
    "model.dateTable",
    "model.dateTable",
    &["name", "from", "to", "markAsDateTable"],
);
const SORT_BY: NodeSchema = node("model.sortBy[]", "model.sortBy", &["column", "by"]);
const FORMAT_STRING: NodeSchema = node(
    "model.formatStrings[]",
    "model.formatStrings",
    &["measure", "column", "formatString"],
);
const STYLE: NodeSchema = node(
    "style",
    "style",
    &["preset", "bundle", "tokens", "defaults"],
);
const STYLE_TOKENS: NodeSchema = node(
    "style.tokens",
    "style.tokens",
    &[
        "palette",
        "semantic",
        "typography",
        "surfaces",
        "spacing",
        "numberFormats",
    ],
);
const STYLE_SEMANTIC: NodeSchema = node(
    "style.tokens.semantic",
    "style.tokens.semantic",
    &["good", "bad", "neutral", "warning", "emphasis"],
);
const STYLE_TYPOGRAPHY: NodeSchema = node(
    "style.tokens.typography",
    "style.tokens.typography",
    &["family", "scale"],
);
const STYLE_SURFACES: NodeSchema = node(
    "style.tokens.surfaces",
    "style.tokens.surfaces",
    &["page", "card", "border", "alt"],
);
const STYLE_SPACING: NodeSchema = node("style.tokens.spacing", "style.tokens.spacing", &["unit"]);
const STYLE_NUMBER_FORMATS: NodeSchema = node(
    "style.tokens.numberFormats",
    "style.tokens.numberFormats",
    &["currency", "percent", "integer", "compactAbove"],
);
const LAYOUT_ROOT: NodeSchema = node("layout", "layout", &["grid", "pageSize", "rail"]);
const LAYOUT_GRID: NodeSchema = node(
    "layout.grid",
    "layout.grid",
    &["columns", "gutter", "margin"],
);
const LAYOUT_PAGE_SIZE: NodeSchema =
    node("layout.pageSize", "layout.pageSize", &["width", "height"]);
const LAYOUT_RAIL: NodeSchema = node("layout.rail", "layout.rail", &["side", "slicers"]);
const RAIL_SLICER: NodeSchema = node(
    "layout.rail.slicers[]",
    "layout.rail.slicers",
    &["field", "mode", "title"],
);
const FILTER: NodeSchema = node(
    "filters[]",
    "filters",
    &[
        "scope",
        "target",
        "kind",
        "values",
        "min",
        "max",
        "relative",
        "displayName",
    ],
);
const FILTER_RELATIVE: NodeSchema = node(
    "filters[].relative",
    "filters.relative",
    &["direction", "unit", "span", "calendar", "includeToday"],
);
const PAGE_V2: NodeSchema = node(
    "pages[]",
    "pages",
    &[
        "id",
        "name",
        "displayName",
        "size",
        "template",
        "heading",
        "subtitle",
        "filters",
        "slicers",
        "visuals",
        "interactions",
        "drillthrough",
        "tooltipFor",
    ],
);
const PAGE_SLICER: NodeSchema = node(
    "pages[].slicers[]",
    "pages.slicers",
    &["field", "mode", "singleSelect", "title", "slot"],
);
const DRILLTHROUGH: NodeSchema = node(
    "pages[].drillthrough",
    "pages.drillthrough",
    &["target", "hidden", "backButton"],
);
const VISUAL_V2: NodeSchema = node(
    "pages[].visuals[]",
    "pages.visuals",
    &[
        "id",
        "name",
        "type",
        "visualType",
        "title",
        "subtitle",
        "bindings",
        "layout",
        "slot",
        "mode",
        "singleSelect",
        "sort",
        "drilldown",
        "topnGuard",
        "filters",
        "format",
        "conditionalFormatting",
        "sortDirection",
        "x",
        "y",
        "width",
        "height",
    ],
);
const VISUAL_SORT: NodeSchema = node(
    "pages[].visuals[].sort",
    "pages.visuals.sort",
    &["field", "direction"],
);
const VISUAL_DRILLDOWN: NodeSchema = node(
    "pages[].visuals[].drilldown",
    "pages.visuals.drilldown",
    &["fields"],
);
const VISUAL_TOPN: NodeSchema = node(
    "pages[].visuals[].topnGuard",
    "pages.visuals.topnGuard",
    &["orderBy", "top"],
);
const VISUAL_FORMAT: NodeSchema = node(
    "pages[].visuals[].format",
    "pages.visuals.format",
    &[
        "labels.show",
        "labels.fontSize",
        "categoryLabels.show",
        "categoryLabels.fontSize",
        "categoryLabels.wordWrap",
        "categoryAxis.show",
        "categoryAxis.showAxisTitle",
        "valueAxis.show",
        "valueAxis.showAxisTitle",
        "title.show",
        "title.text",
    ],
);
const PROOF: NodeSchema = node("proof", "proof", &["desktop", "goldens"]);
const PROOF_DESKTOP: NodeSchema = node(
    "proof.desktop",
    "proof.desktop",
    &["level", "pages", "expectValues"],
);
const PROOF_EXPECT_VALUE: NodeSchema = node(
    "proof.desktop.expectValues[]",
    "proof.desktop.expectValues",
    &["query", "expected"],
);

const V1_NODES: &[NodeSchema] = &[
    ROOT_V1,
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

const V2_NODES: &[NodeSchema] = &[
    ROOT_V2,
    REPORT,
    MODEL_V2,
    MODEL_MEASURE_V2,
    MEASURE_PATTERN,
    CALCULATED_COLUMN,
    RELATIONSHIP,
    STATIC_TABLE,
    DATE_TABLE,
    SORT_BY,
    FORMAT_STRING,
    STYLE,
    STYLE_TOKENS,
    STYLE_SEMANTIC,
    STYLE_TYPOGRAPHY,
    STYLE_SURFACES,
    STYLE_SPACING,
    STYLE_NUMBER_FORMATS,
    LAYOUT_ROOT,
    LAYOUT_GRID,
    LAYOUT_PAGE_SIZE,
    LAYOUT_RAIL,
    RAIL_SLICER,
    FILTER,
    FILTER_RELATIVE,
    PAGE_V2,
    PAGE_SIZE,
    PAGE_SLICER,
    DRILLTHROUGH,
    VISUAL_V2,
    VISUAL_LAYOUT,
    VISUAL_SORT,
    VISUAL_DRILLDOWN,
    VISUAL_TOPN,
    VISUAL_FORMAT,
    BINDING,
    INTERACTION,
    PROOF,
    PROOF_DESKTOP,
    PROOF_EXPECT_VALUE,
];

const fn node(
    name: &'static str,
    path: &'static str,
    fields: &'static [&'static str],
) -> NodeSchema {
    NodeSchema { name, path, fields }
}

pub(crate) fn validate_known_fields(spec: &Value) -> CliResult<SpecVersion> {
    let root = spec
        .as_object()
        .ok_or_else(|| CliError::invalid_args("dashboard spec root must be an object"))?;
    let version = spec_version(root)?;
    if version == SpecVersion::V2 {
        walk_v2(root)?;
        serde_json::from_value::<SpecV2>(spec.clone()).map_err(|error| {
            CliError::invalid_args(format!("invalid {DASHBOARD_V2} shape: {error}"))
                .with_suggested_command("powerbi-cli report spec fields --json")
        })?;
        return Ok(version);
    }
    walk_object(root, ROOT_V1, "")?;

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
    Ok(version)
}

pub(crate) fn allowed_fields_json() -> Value {
    nodes_json(V2_NODES)
}

pub(crate) fn versioned_allowed_fields_json() -> Value {
    json!([
        {"schema": DASHBOARD_V1, "allowedFields": nodes_json(V1_NODES)},
        {"schema": DASHBOARD_V2, "allowedFields": nodes_json(V2_NODES)}
    ])
}

fn nodes_json(nodes: &[NodeSchema]) -> Value {
    Value::Array(
        nodes
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

pub(crate) fn reject_uncompiled_v2_sections(spec: &Value) -> CliResult<()> {
    let Some(root) = spec.as_object() else {
        return Ok(());
    };
    if spec_version(root)? != SpecVersion::V2 {
        return Ok(());
    }
    let unsupported = first_uncompiled_v2_section(root);
    let Some((section, bead, command)) = unsupported else {
        return Ok(());
    };
    Err(CliError::unsupported_feature(format!(
        "dashboard spec v2 section `{section}` is recognized but not compiled; owning bead: {bead}"
    ))
    .with_hint(format!(
        "Keep the section in the v2 spec for future compilation, or apply its supported primitive after build. Owning bead: {bead}."
    ))
    .with_suggested_command(command))
}

fn spec_version(root: &Map<String, Value>) -> CliResult<SpecVersion> {
    match root.get("schema").and_then(Value::as_str) {
        Some(DASHBOARD_V2) => Ok(SpecVersion::V2),
        Some(DASHBOARD_V1) | None => Ok(SpecVersion::V1),
        Some(other) => Err(CliError::invalid_args(format!(
            "unsupported dashboard spec schema `{other}`"
        ))
        .with_pointer("/schema")
        .with_hint(format!("Use `{DASHBOARD_V1}` or `{DASHBOARD_V2}`."))),
    }
}

fn walk_v2(root: &Map<String, Value>) -> CliResult<()> {
    walk_object(root, ROOT_V2, "")?;
    walk_child_object(root, "report", REPORT, "")?;
    if let Some(model) = walk_child_object(root, "model", MODEL_V2, "")? {
        walk_objects(model, "measures", MODEL_MEASURE_V2, "/model", |_, _| Ok(()))?;
        walk_objects(
            model,
            "measurePatterns",
            MEASURE_PATTERN,
            "/model",
            |_, _| Ok(()),
        )?;
        walk_objects(
            model,
            "calculatedColumns",
            CALCULATED_COLUMN,
            "/model",
            |_, _| Ok(()),
        )?;
        walk_objects(
            model,
            "relationships",
            RELATIONSHIP,
            "/model",
            |_, _| Ok(()),
        )?;
        walk_objects(model, "staticTables", STATIC_TABLE, "/model", |_, _| Ok(()))?;
        walk_child_object_at(model, "dateTable", DATE_TABLE, "/model")?;
        walk_objects(model, "sortBy", SORT_BY, "/model", |_, _| Ok(()))?;
        walk_objects(model, "formatStrings", FORMAT_STRING, "/model", |_, _| {
            Ok(())
        })?;
    }
    if let Some(style) = walk_child_object(root, "style", STYLE, "")? {
        if let Some(tokens) = walk_child_object_at(style, "tokens", STYLE_TOKENS, "/style")? {
            walk_child_object_at(tokens, "semantic", STYLE_SEMANTIC, "/style/tokens")?;
            walk_child_object_at(tokens, "typography", STYLE_TYPOGRAPHY, "/style/tokens")?;
            walk_child_object_at(tokens, "surfaces", STYLE_SURFACES, "/style/tokens")?;
            walk_child_object_at(tokens, "spacing", STYLE_SPACING, "/style/tokens")?;
            walk_child_object_at(
                tokens,
                "numberFormats",
                STYLE_NUMBER_FORMATS,
                "/style/tokens",
            )?;
        }
    }
    if let Some(layout) = walk_child_object(root, "layout", LAYOUT_ROOT, "")? {
        walk_child_object_at(layout, "grid", LAYOUT_GRID, "/layout")?;
        walk_child_object_at(layout, "pageSize", LAYOUT_PAGE_SIZE, "/layout")?;
        if let Some(rail) = walk_child_object_at(layout, "rail", LAYOUT_RAIL, "/layout")? {
            walk_objects(rail, "slicers", RAIL_SLICER, "/layout/rail", |_, _| Ok(()))?;
        }
    }
    walk_filters(root, "filters", "")?;
    walk_objects(root, "pages", PAGE_V2, "", |page, page_pointer| {
        walk_child_object_at(page, "size", PAGE_SIZE, page_pointer)?;
        walk_filters(page, "filters", page_pointer)?;
        walk_objects(page, "slicers", PAGE_SLICER, page_pointer, |_, _| Ok(()))?;
        walk_child_object_at(page, "drillthrough", DRILLTHROUGH, page_pointer)?;
        walk_objects(
            page,
            "visuals",
            VISUAL_V2,
            page_pointer,
            |visual, visual_pointer| {
                walk_child_object_at(visual, "layout", VISUAL_LAYOUT, visual_pointer)?;
                walk_child_object_at(visual, "sort", VISUAL_SORT, visual_pointer)?;
                walk_child_object_at(visual, "drilldown", VISUAL_DRILLDOWN, visual_pointer)?;
                walk_child_object_at(visual, "topnGuard", VISUAL_TOPN, visual_pointer)?;
                walk_child_object_at(visual, "format", VISUAL_FORMAT, visual_pointer)?;
                walk_filters(visual, "filters", visual_pointer)?;
                walk_objects(visual, "bindings", BINDING, visual_pointer, |_, _| Ok(()))
            },
        )?;
        walk_objects(page, "interactions", INTERACTION, page_pointer, |_, _| {
            Ok(())
        })
    })?;
    if let Some(proof) = walk_child_object(root, "proof", PROOF, "")? {
        if let Some(desktop) = walk_child_object_at(proof, "desktop", PROOF_DESKTOP, "/proof")? {
            walk_objects(
                desktop,
                "expectValues",
                PROOF_EXPECT_VALUE,
                "/proof/desktop",
                |_, _| Ok(()),
            )?;
        }
    }
    Ok(())
}

fn walk_filters(parent: &Map<String, Value>, field: &str, pointer: &str) -> CliResult<()> {
    walk_objects(parent, field, FILTER, pointer, |filter, filter_pointer| {
        walk_child_object_at(filter, "relative", FILTER_RELATIVE, filter_pointer)?;
        Ok(())
    })
}

fn walk_child_object<'a>(
    parent: &'a Map<String, Value>,
    field: &str,
    schema: NodeSchema,
    parent_pointer: &str,
) -> CliResult<Option<&'a Map<String, Value>>> {
    walk_child_object_at(parent, field, schema, parent_pointer)
}

fn walk_child_object_at<'a>(
    parent: &'a Map<String, Value>,
    field: &str,
    schema: NodeSchema,
    parent_pointer: &str,
) -> CliResult<Option<&'a Map<String, Value>>> {
    let Some(object) = parent.get(field).and_then(Value::as_object) else {
        return Ok(None);
    };
    let pointer = format!("{parent_pointer}/{}", escape_pointer_token(field));
    walk_object(object, schema, &pointer)?;
    Ok(Some(object))
}

fn walk_objects<F>(
    parent: &Map<String, Value>,
    field: &str,
    schema: NodeSchema,
    parent_pointer: &str,
    mut nested: F,
) -> CliResult<()>
where
    F: FnMut(&Map<String, Value>, &str) -> CliResult<()>,
{
    let Some(values) = parent.get(field).and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, value) in values.iter().enumerate() {
        let Some(object) = value.as_object() else {
            continue;
        };
        let pointer = format!("{parent_pointer}/{}/{index}", escape_pointer_token(field));
        walk_object(object, schema, &pointer)?;
        nested(object, &pointer)?;
    }
    Ok(())
}

fn first_uncompiled_v2_section(
    root: &Map<String, Value>,
) -> Option<(String, &'static str, &'static str)> {
    const FILTER_BEAD: &str = "pbi-t3-compiler-completeness-1qi.1";
    const SLICER_BEAD: &str = "pbi-t3-compiler-completeness-1qi.2";
    const DRILLTHROUGH_BEAD: &str = "pbi-t3-compiler-completeness-1qi.3";
    const VISUAL_BEHAVIOR_BEAD: &str = "pbi-t3-compiler-completeness-1qi.4";
    const MODEL_BEAD: &str = "pbi-t3-compiler-completeness-1qi.5";
    const STYLE_BEAD: &str = "pbi-t3-compiler-completeness-1qi.6";
    const LAYOUT_BEAD: &str = "pbi-t3-compiler-completeness-1qi.7";
    const FORMAT_BEAD: &str = "pbi-t3-compiler-completeness-1qi.8";
    const PROOF_BEAD: &str = "pbi-t3-compiler-completeness-1qi.9";

    if root.contains_key("filters") {
        return Some((
            "filters".to_string(),
            FILTER_BEAD,
            "powerbi-cli report filters add --project <project-dir> --target <Table[Column]> --value <value> --dry-run --json",
        ));
    }
    if let Some(model) = root.get("model").and_then(Value::as_object) {
        for section in [
            "measurePatterns",
            "calculatedColumns",
            "relationships",
            "staticTables",
            "dateTable",
            "sortBy",
            "formatStrings",
        ] {
            if model.contains_key(section) {
                return Some((
                    format!("model.{section}"),
                    MODEL_BEAD,
                    "powerbi-cli --json capabilities --for model",
                ));
            }
        }
        if model
            .get("measures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .any(|measure| {
                measure.contains_key("expressionFile")
                    || measure.contains_key("formatStringExpression")
            })
        {
            return Some((
                "model.measures[].expressionFile|formatStringExpression".to_string(),
                MODEL_BEAD,
                "powerbi-cli model measures add --project <project-dir> --table <table> --name <name> --expression-file <path> --dry-run --json",
            ));
        }
    }
    if root.contains_key("style") {
        return Some((
            "style".to_string(),
            STYLE_BEAD,
            "powerbi-cli report themes apply-preset --project <project-dir> --preset <preset> --dry-run --json",
        ));
    }
    if let Some(layout) = root.get("layout").and_then(Value::as_object) {
        if layout.contains_key("rail") {
            return Some((
                "layout.rail".to_string(),
                SLICER_BEAD,
                "powerbi-cli report visuals add --project <project-dir> --page <page-handle> --visual-type slicer --dry-run --json",
            ));
        }
        return Some((
            "layout".to_string(),
            LAYOUT_BEAD,
            "powerbi-cli report layout auto --project <project-dir> --page <page-handle> --preset overview --dry-run --json",
        ));
    }
    for (page_index, page) in root
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .enumerate()
    {
        if page.contains_key("filters") {
            return Some((
                format!("pages[{page_index}].filters"),
                FILTER_BEAD,
                "powerbi-cli report filters add --project <project-dir> --page <page-handle> --target <Table[Column]> --value <value> --dry-run --json",
            ));
        }
        if page.contains_key("slicers") {
            return Some((
                format!("pages[{page_index}].slicers"),
                SLICER_BEAD,
                "powerbi-cli report visuals add --project <project-dir> --page <page-handle> --visual-type slicer --dry-run --json",
            ));
        }
        if page.contains_key("drillthrough") || page.contains_key("tooltipFor") {
            return Some((
                format!("pages[{page_index}].drillthrough|tooltipFor"),
                DRILLTHROUGH_BEAD,
                "powerbi-cli report drillthrough set --project <project-dir> --page <page-handle> --target <Table[Column]> --dry-run --json",
            ));
        }
        if ["template", "heading", "subtitle"]
            .iter()
            .any(|field| page.contains_key(*field))
        {
            return Some((
                format!("pages[{page_index}].template|heading|subtitle"),
                LAYOUT_BEAD,
                "powerbi-cli report layout auto --project <project-dir> --page <page-handle> --preset overview --dry-run --json",
            ));
        }
        for (visual_index, visual) in page
            .get("visuals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .enumerate()
        {
            if ["sort", "drilldown", "topnGuard", "filters"]
                .iter()
                .any(|field| visual.contains_key(*field))
            {
                return Some((
                    format!(
                        "pages[{page_index}].visuals[{visual_index}].sort|drilldown|topnGuard|filters"
                    ),
                    VISUAL_BEHAVIOR_BEAD,
                    "powerbi-cli --json capabilities --for report",
                ));
            }
            if visual.contains_key("slot") || visual.contains_key("subtitle") {
                return Some((
                    format!("pages[{page_index}].visuals[{visual_index}].slot|subtitle"),
                    LAYOUT_BEAD,
                    "powerbi-cli report visuals set-position --project <project-dir> --handle <visual-handle> --x <x> --y <y> --width <width> --height <height> --dry-run --json",
                ));
            }
            if visual.contains_key("format") || visual.contains_key("conditionalFormatting") {
                return Some((
                    format!(
                        "pages[{page_index}].visuals[{visual_index}].format|conditionalFormatting"
                    ),
                    FORMAT_BEAD,
                    "powerbi-cli report visuals set-object --project <project-dir> --handle <visual-handle> --object <object> --property <property> --value <value> --dry-run --json",
                ));
            }
        }
    }
    if root.contains_key("proof") {
        return Some((
            "proof".to_string(),
            PROOF_BEAD,
            "powerbi-cli desktop open-check <project-dir> --json",
        ));
    }
    None
}

// These DTOs deliberately mirror the public v2 shape. The strict walker owns
// pointer-rich unknown-field diagnostics; serde supplies independent structural
// type validation and deny_unknown_fields at each statically shaped node.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpecV2 {
    schema: String,
    report: ReportV2,
    model: Option<ModelV2>,
    style: Option<StyleV2>,
    layout: Option<LayoutV2>,
    filters: Option<Vec<FilterV2>>,
    pages: Vec<PageV2>,
    proof: Option<ProofV2>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReportV2 {
    name: Option<Value>,
    display_name: Option<Value>,
    description: Option<Value>,
    locale: Option<Value>,
    audience: Option<Value>,
    questions: Option<Vec<Value>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelV2 {
    measures: Option<Vec<MeasureV2>>,
    measure_patterns: Option<Vec<MeasurePatternV2>>,
    calculated_columns: Option<Vec<CalculatedColumnV2>>,
    relationships: Option<Vec<RelationshipV2>>,
    static_tables: Option<Vec<StaticTableV2>>,
    date_table: Option<DateTableV2>,
    sort_by: Option<Vec<SortByV2>>,
    format_strings: Option<Vec<FormatStringV2>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MeasureV2 {
    table: Option<Value>,
    name: Option<Value>,
    expression: Option<Value>,
    expression_file: Option<Value>,
    format_string: Option<Value>,
    format_string_expression: Option<Value>,
    display_folder: Option<Value>,
    description: Option<Value>,
}

macro_rules! value_struct {
    ($name:ident { $($field:ident),* $(,)? }) => {
        #[allow(dead_code)]
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name { $( $field: Option<Value>, )* }
    };
}

value_struct!(MeasurePatternV2 {
    pattern,
    base,
    date,
    name
});
value_struct!(CalculatedColumnV2 {
    table,
    name,
    expression,
    data_type,
    format_string
});
value_struct!(RelationshipV2 {
    from,
    to,
    cardinality,
    cross_filtering_behavior,
    is_active
});
value_struct!(DateTableV2 {
    name,
    from,
    to,
    mark_as_date_table
});
value_struct!(SortByV2 { column, by });
value_struct!(FormatStringV2 {
    measure,
    column,
    format_string
});
value_struct!(PageSizeV2 { width, height });
value_struct!(SlicerV2 {
    field,
    mode,
    single_select,
    title,
    slot
});
value_struct!(RailSlicerV2 { field, mode, title });
value_struct!(DrillthroughV2 {
    target,
    hidden,
    back_button
});
value_struct!(VisualLayoutV2 {
    x,
    y,
    width,
    height
});
value_struct!(VisualSortV2 { field, direction });
value_struct!(VisualDrilldownV2 { fields });
value_struct!(VisualTopnV2 { order_by, top });
value_struct!(BindingV2 {
    role,
    field,
    table,
    column,
    measure,
    display_name,
    format_string,
    sort_direction
});
value_struct!(InteractionV2 {
    source,
    target,
    r#type
});
value_struct!(RelativeFilterV2 {
    direction,
    unit,
    span,
    calendar,
    include_today
});
value_struct!(ProofExpectationV2 { query, expected });

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StaticTableV2 {
    name: Option<Value>,
    columns: Option<Vec<Value>>,
    rows: Option<Vec<Vec<Value>>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StyleV2 {
    preset: Option<Value>,
    bundle: Option<Value>,
    tokens: Option<StyleTokensV2>,
    defaults: Option<BTreeMap<String, Value>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StyleTokensV2 {
    palette: Option<Vec<Value>>,
    semantic: Option<SemanticTokensV2>,
    typography: Option<TypographyTokensV2>,
    surfaces: Option<SurfaceTokensV2>,
    spacing: Option<SpacingTokensV2>,
    number_formats: Option<NumberFormatTokensV2>,
}

value_struct!(SemanticTokensV2 {
    good,
    bad,
    neutral,
    warning,
    emphasis
});
value_struct!(TypographyTokensV2 { family, scale });
value_struct!(SurfaceTokensV2 {
    page,
    card,
    border,
    alt
});
value_struct!(SpacingTokensV2 { unit });
value_struct!(NumberFormatTokensV2 {
    currency,
    percent,
    integer,
    compact_above
});

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LayoutV2 {
    grid: Option<GridV2>,
    page_size: Option<Value>,
    rail: Option<RailV2>,
}

value_struct!(GridV2 {
    columns,
    gutter,
    margin
});

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RailV2 {
    side: Option<Value>,
    slicers: Option<Vec<RailSlicerV2>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FilterV2 {
    scope: Option<Value>,
    target: Option<Value>,
    kind: Option<Value>,
    values: Option<Vec<Value>>,
    min: Option<Value>,
    max: Option<Value>,
    relative: Option<RelativeFilterV2>,
    display_name: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PageV2 {
    id: Option<Value>,
    name: Option<Value>,
    display_name: Option<Value>,
    size: Option<PageSizeV2>,
    template: Option<Value>,
    heading: Option<Value>,
    subtitle: Option<Value>,
    filters: Option<Vec<FilterV2>>,
    slicers: Option<Vec<SlicerV2>>,
    visuals: Option<Vec<VisualV2>>,
    interactions: Option<Vec<InteractionV2>>,
    drillthrough: Option<DrillthroughV2>,
    tooltip_for: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VisualV2 {
    id: Option<Value>,
    name: Option<Value>,
    r#type: Option<Value>,
    visual_type: Option<Value>,
    title: Option<Value>,
    subtitle: Option<Value>,
    bindings: Option<Vec<BindingV2>>,
    layout: Option<VisualLayoutV2>,
    slot: Option<Value>,
    mode: Option<Value>,
    single_select: Option<Value>,
    sort: Option<VisualSortV2>,
    drilldown: Option<VisualDrilldownV2>,
    topn_guard: Option<VisualTopnV2>,
    filters: Option<Vec<FilterV2>>,
    format: Option<BTreeMap<String, Value>>,
    conditional_formatting: Option<Vec<Value>>,
    sort_direction: Option<Value>,
    x: Option<Value>,
    y: Option<Value>,
    width: Option<Value>,
    height: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProofV2 {
    desktop: Option<ProofDesktopV2>,
    goldens: Option<Vec<Value>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProofDesktopV2 {
    level: Option<Value>,
    pages: Option<Vec<Value>>,
    expect_values: Option<Vec<ProofExpectationV2>>,
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
                    error.pointer = Some(format!("{item_pointer}/{relative}").into_boxed_str());
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
        0..=8 => 2,
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

    #[test]
    fn complete_v2_shape_passes_walker_and_deny_unknown_fields_models() {
        let spec = json!({
            "schema": DASHBOARD_V2,
            "report": {"name": "Full", "audience": "analyst", "questions": ["What changed?"]},
            "model": {
                "measures": [{"table": "Facts", "name": "Total", "expression": "1"}],
                "measurePatterns": [{"pattern": "yoy", "base": "Facts[Total]", "date": "Date[Date]"}],
                "calculatedColumns": [{"table": "Facts", "name": "Band", "expression": "\"A\"", "dataType": "string"}],
                "relationships": [{"from": "Facts[Date]", "to": "Date[Date]", "cardinality": "manyToOne"}],
                "staticTables": [{"name": "Selector", "columns": ["Name"], "rows": [["A"]]}],
                "dateTable": {"name": "Date", "from": "2024-01-01", "to": "2024-12-31", "markAsDateTable": true},
                "sortBy": [{"column": "Date[Month]", "by": "Date[MonthNo]"}],
                "formatStrings": [{"measure": "Facts[Total]", "formatString": "#,##0"}]
            },
            "style": {
                "tokens": {
                    "palette": ["#123456"],
                    "semantic": {"good": "#008000", "bad": "#ff0000"},
                    "typography": {"family": "Segoe UI", "scale": 1},
                    "surfaces": {"page": "#ffffff", "card": "#fafafa", "border": "#dddddd", "alt": "#f0f0f0"},
                    "spacing": {"unit": 8},
                    "numberFormats": {"currency": "$#,##0", "percent": "0.0%", "integer": "#,##0", "compactAbove": 1000000}
                },
                "defaults": {"card.title.show": false}
            },
            "layout": {
                "grid": {"columns": 12, "gutter": 16, "margin": 24},
                "pageSize": {"width": 1280, "height": 720},
                "rail": {"side": "left", "slicers": [{"field": "Date[Year]", "mode": "Dropdown", "title": "Year"}]}
            },
            "filters": [{"scope": "report", "target": "Date[Year]", "kind": "relativeDate", "relative": {"direction": "last", "unit": "years", "span": 1}}],
            "pages": [{
                "id": "overview",
                "template": "overview",
                "heading": "Overview",
                "subtitle": "Current period",
                "slicers": [{"field": "Date[Year]", "mode": "Dropdown", "singleSelect": true, "slot": "rail"}],
                "drillthrough": {"target": "Date[Date]", "hidden": true, "backButton": true},
                "visuals": [{
                    "id": "trend",
                    "type": "lineChart",
                    "slot": "primary",
                    "bindings": [{"role": "Category", "field": "Date[Date]"}],
                    "sort": {"field": "Facts[Total]", "direction": "Descending"},
                    "drilldown": {"fields": ["Date[Year]", "Date[Month]"]},
                    "topnGuard": {"orderBy": "Facts[Total]", "top": 10},
                    "filters": [{"target": "Date[Year]", "kind": "categorical", "values": [2024]}],
                    "format": {"title.show": true, "title.text": "Trend"},
                    "conditionalFormatting": []
                }],
                "interactions": [{"source": "trend", "target": "detail", "type": "DataFilter"}]
            }],
            "proof": {"desktop": {"level": "desktop-canvas-refresh", "pages": ["overview"], "expectValues": [{"query": "EVALUATE ROW(\"x\", 1)", "expected": 1}]}, "goldens": ["sales"]}
        });
        assert_eq!(
            validate_known_fields(&spec).expect("valid v2"),
            SpecVersion::V2
        );
    }
}
