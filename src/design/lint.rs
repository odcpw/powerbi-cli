//! Deterministic design-system linting for existing PBIP report trees.
//!
//! Design lint is intentionally read-only. It consumes the embedded grid and
//! template catalog, reports every finding with a stable rule id and RFC 6901
//! pointer, and describes a mechanical action when one is safe to plan.
//! Applying those actions belongs to the operation/auto-improve beads; this
//! module never writes guessed PBIR.

use super::grid::{self, Grid, PageSize, RailSide, Slot, SlotPosition, Template};
use crate::rules::{self, RuleFamily};
use crate::{CliResult, ResolvedProject, canonical_display, read_json_value};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const DESIGN_LINT_SCHEMA: &str = "powerbi-cli.design.lint.v1";
const GRID_EPSILON: f64 = 0.01;

/// Return the ids classified as part of the typed design family.
pub(crate) fn design_rule_ids() -> Vec<&'static str> {
    rules::rules_for_family(RuleFamily::Design)
        .map(|rule| rule.id)
        .collect()
}

pub(crate) fn is_design_rule_id(id: &str) -> bool {
    rules::find_rule(id).is_some_and(|rule| rule.family == RuleFamily::Design)
}

/// Lint the report portion of a deep-inspection document.  The deep document
/// is passed in by the caller so `lint`, `triage`, and the build scorecard do
/// not each perform another expensive filesystem walk.
pub(crate) fn lint_report(resolved: &ResolvedProject, deep: &Value) -> CliResult<Value> {
    // A malformed embedded grid catalog is a validation failure, never a
    // reason to silently skip a geometry check.
    let grid_catalog = grid::catalog()?;
    let pages = deep["report"]["pages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut findings = Vec::new();
    let mut contexts = Vec::new();
    let style_policy = crate::design_lint_style::resolved_policy(resolved)?;

    for (page_index, page) in pages.iter().enumerate() {
        let context = PageContext::load(
            resolved,
            page,
            page_index,
            &grid_catalog.grid,
            &grid_catalog.templates,
        )?;
        lint_page_geometry(&context, &mut findings)?;
        crate::design_lint_content::lint_page(
            page,
            page_index,
            context
                .template
                .as_ref()
                .is_some_and(|template| template.name == "ranking"),
            &mut findings,
            style_policy.as_ref(),
        )?;
        contexts.push(context);
    }
    lint_rail_sync(&contexts, &mut findings)?;
    crate::design_lint_content::lint_measure_formats(deep, &mut findings);

    sort_findings(&mut findings);
    rules::ensure_finding_ids_registered(&findings, "ruleId")?;
    let counts = counts(&findings);
    let project = canonical_display(&resolved.project_dir);
    Ok(json!({
        "schema": DESIGN_LINT_SCHEMA,
        "status": "available",
        "proofLevel": "unit-smoke",
        "ok": counts["errors"].as_u64().unwrap_or_default() == 0,
        "projectDir": project,
        "counts": counts,
        "ruleIds": design_rule_ids(),
        "evaluatedRules": design_rule_ids().into_iter().filter(|id| style_policy.is_some() || !crate::design_lint_content::is_deferred(id)).collect::<Vec<_>>(),
        "deferredRules": if style_policy.is_some() { json!([]) } else { crate::design_lint_content::deferred_rules() },
        "grid": {
            "schema": grid_catalog.schema,
            "columns": grid_catalog.grid.columns,
            "gutter": grid_catalog.grid.gutter,
            "margin": grid_catalog.grid.margin,
            "rowUnit": grid_catalog.grid.row_unit,
            "templates": grid_catalog.templates.iter().map(|template| template.name.clone()).collect::<Vec<_>>()
        },
        "findings": findings,
        "next": [
            format!("powerbi-cli report audit --project {} --rules design --json", crate::command_arg(&resolved.project_dir)),
            format!("powerbi-cli report design-plan --project {} --json", crate::command_arg(&resolved.project_dir))
        ]
    }))
}

#[derive(Debug)]
struct PageContext {
    page: Value,
    page_index: usize,
    page_path: Option<PathBuf>,
    raw_page: Option<Value>,
    width: f64,
    height: f64,
    visuals: Vec<Value>,
    template: Option<Template>,
    template_positions: BTreeMap<String, SlotPosition>,
    rail: Option<RailSide>,
}

impl PageContext {
    fn load(
        _resolved: &ResolvedProject,
        page: &Value,
        page_index: usize,
        grid: &Grid,
        templates: &[Template],
    ) -> CliResult<Self> {
        let width = finite_or(page["width"].as_f64(), PageSize::STANDARD.width);
        let height = finite_or(page["height"].as_f64(), PageSize::STANDARD.height);
        let page_path = page["path"].as_str().map(PathBuf::from);
        let raw_page = page_path
            .as_deref()
            .filter(|path| path.is_file())
            .map(read_json_value)
            .transpose()?;
        let visuals = page["visuals"].as_array().cloned().unwrap_or_default();

        let declared = declared_template(raw_page.as_ref(), page);
        let template = if let Some(name) = declared {
            Some(grid::template(&name)?)
        } else {
            infer_template(&visuals, width, height, grid, templates)
        };
        let (template_positions, rail) = if let Some(template) = template.as_ref() {
            let positions =
                grid::resolve_with_grid(template, PageSize { width, height }, *grid, None)?;
            (positions, template.rail)
        } else {
            (BTreeMap::new(), None)
        };
        Ok(Self {
            page: page.clone(),
            page_index,
            page_path,
            raw_page,
            width,
            height,
            visuals,
            template,
            template_positions,
            rail,
        })
    }

    fn handle(&self) -> Option<&str> {
        self.page["handle"].as_str()
    }

    fn display_name(&self) -> &str {
        self.page["displayName"]
            .as_str()
            .or_else(|| self.page["name"].as_str())
            .unwrap_or("page")
    }
}

fn declared_template(raw_page: Option<&Value>, deep_page: &Value) -> Option<String> {
    for value in [
        deep_page.get("template"),
        deep_page.pointer("/layout/template"),
        raw_page.and_then(|page| page.get("template")),
        raw_page.and_then(|page| page.pointer("/layout/template")),
    ] {
        if let Some(name) = value
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
        {
            return Some(name.to_string());
        }
    }
    raw_page
        .and_then(|page| page["annotations"].as_array())
        .and_then(|annotations| {
            annotations.iter().find_map(|annotation| {
                let name = annotation["name"].as_str().unwrap_or_default();
                let value = annotation["value"].as_str().unwrap_or_default();
                (name.eq_ignore_ascii_case("powerbi-cli.template")
                    || name.eq_ignore_ascii_case("powerbi-cli.design.template")
                    || name.eq_ignore_ascii_case("powerbi-cli.layout.template"))
                .then(|| value.to_string())
            })
        })
        .filter(|name| !name.trim().is_empty())
}

fn infer_template(
    visuals: &[Value],
    width: f64,
    height: f64,
    grid: &Grid,
    templates: &[Template],
) -> Option<Template> {
    let mut best: Option<(usize, Template)> = None;
    let mut best_count = 0_usize;
    for template in templates {
        let Ok(positions) =
            grid::resolve_with_grid(template, PageSize { width, height }, *grid, None)
        else {
            continue;
        };
        let score = visuals
            .iter()
            .filter(|visual| {
                let position = visual.get("position").unwrap_or(&Value::Null);
                positions
                    .values()
                    .any(|slot| position_matches(position, *slot))
            })
            .count();
        if score == 0 {
            continue;
        }
        match best.as_ref().map(|current| score.cmp(&current.0)) {
            None | Some(std::cmp::Ordering::Greater) => {
                best = Some((score, template.clone()));
                best_count = 1;
            }
            Some(std::cmp::Ordering::Equal) => best_count += 1,
            Some(std::cmp::Ordering::Less) => {}
        }
    }
    best.filter(|(score, _)| *score >= 2 && best_count == 1)
        .map(|(_, template)| template)
}

fn lint_page_geometry(context: &PageContext, findings: &mut Vec<Value>) -> CliResult<()> {
    for (left_index, left) in context.visuals.iter().enumerate() {
        for (right_index, right) in context.visuals.iter().enumerate().skip(left_index + 1) {
            let left_position = rect(left.get("position").unwrap_or(&Value::Null));
            let right_position = rect(right.get("position").unwrap_or(&Value::Null));
            if rectangles_overlap(left_position, right_position) {
                findings.push(design_finding(
                    rules::DESIGN_VISUAL_OVERLAP,
                    visual_handle(right),
                    visual_path(right),
                    format!(
                        "/report/pages/{}/visuals/{}/position",
                        context.page_index, right_index
                    ),
                    format!(
                        "visuals {} and {} overlap on page {}",
                        visual_handle(left).unwrap_or("visual"),
                        visual_handle(right).unwrap_or("visual"),
                        context.display_name()
                    ),
                    json!({
                        "otherVisual": visual_handle(right),
                        "visualPosition": position_summary(left),
                        "otherPosition": position_summary(right)
                    }),
                ));
            }
        }
    }

    for (visual_index, visual) in context.visuals.iter().enumerate() {
        let position = rect(visual.get("position").unwrap_or(&Value::Null));
        if outside_page(position, context.width, context.height) {
            findings.push(design_finding(
                rules::REPORT_VISUAL_OUTSIDE_PAGE,
                visual_handle(visual),
                visual_path(visual),
                format!(
                    "/report/pages/{}/visuals/{}/position",
                    context.page_index, visual_index
                ),
                format!(
                    "visual is outside page bounds: {}",
                    visual_title(visual).unwrap_or_else(|| "visual".to_string())
                ),
                position_summary(visual),
            ));
        }
        if !aligned_to_grid(position, context.width, context.height, Grid::default()) {
            findings.push(design_finding(
                rules::DESIGN_VISUAL_OFF_GRID,
                visual_handle(visual),
                visual_path(visual),
                format!(
                    "/report/pages/{}/visuals/{}/position",
                    context.page_index, visual_index
                ),
                format!(
                    "visual misses the twelve-column grid guides (left edge must sit on a column start guide, right edge on a column end guide, and top/bottom on {} px row-unit multiples): {}",
                    Grid::default().row_unit,
                    visual_title(visual).unwrap_or_else(|| "visual".to_string())
                ),
                json!({
                    "position": position_summary(visual),
                    "columns": Grid::default().columns,
                    "rowUnit": Grid::default().row_unit
                }),
            ));
        }
        let mode = visual_slicer_mode(visual)?;
        let minimum_height = if mode.as_deref() == Some("between") {
            104.0
        } else {
            76.0
        };
        if is_slicer(visual) && position.height + GRID_EPSILON < minimum_height {
            findings.push(design_finding(
                rules::DESIGN_SLICER_TOO_SHORT,
                visual_handle(visual),
                visual_path(visual),
                format!(
                    "/report/pages/{}/visuals/{}/position/height",
                    context.page_index, visual_index
                ),
                format!(
                    "slicer height {} is below the {} pixel minimum",
                    trim_number(position.height),
                    trim_number(minimum_height)
                ),
                json!({
                    "height": position.height,
                    "minimumHeight": minimum_height,
                    "mode": mode
                }),
            ));
        }
    }

    if let Some(template) = context.template.as_ref() {
        if let Some(budget) = template.budget
            && budgeted_visual_count(context) > budget
        {
            let visual_count = budgeted_visual_count(context);
            findings.push(design_finding(
                rules::DESIGN_PAGE_OVERCROWDED,
                context.handle(),
                context.page_path.as_deref(),
                format!("/report/pages/{}/visuals", context.page_index),
                format!(
                    "page {} contains {} visuals but template {} budgets {}",
                    context.display_name(),
                    visual_count,
                    template.name,
                    budget
                ),
                json!({
                    "template": template.name,
                    "visualCount": visual_count,
                    "budget": budget
                }),
            ));
        }

        let heading_expected = template.heading_band.is_some()
            || template.slots.iter().any(|slot| slot.name == "heading");
        if heading_expected
            && !context.visuals.iter().any(|visual| {
                slot_for_visual(context, visual).is_some_and(|slot| slot.name == "heading")
                    || is_heading_visual(visual)
            })
        {
            findings.push(design_finding(
                rules::DESIGN_PAGE_MISSING_HEADING,
                context.handle(),
                context.page_path.as_deref(),
                format!("/report/pages/{}/visuals", context.page_index),
                format!(
                    "page {} has a heading band but no heading visual",
                    context.display_name()
                ),
                json!({"template": template.name, "headingBand": template.heading_band}),
            ));
        }

        let row_groups = group_visuals_by_slot(context, |slot| (slot.row, slot.row_span));
        for group in row_groups.values() {
            let heights = group
                .iter()
                .map(|(_, visual)| rect(&visual["position"]).height)
                .collect::<Vec<_>>();
            if heights.len() > 1 && range(&heights) > GRID_EPSILON {
                let (_, visual) = group[0];
                findings.push(design_finding(
                    rules::DESIGN_ROW_HEIGHT_INCONSISTENT,
                    visual_handle(visual),
                    visual_path(visual),
                    format!(
                        "/report/pages/{}/visuals/{}/position/height",
                        context.page_index, group[0].0
                    ),
                    format!(
                        "visuals sharing row y={} have inconsistent heights",
                        trim_number(rect(&visual["position"]).y)
                    ),
                    json!({"row": rect(&visual["position"]).y, "heights": heights}),
                ));
            }
        }
        let column_groups = group_visuals_by_slot(context, |slot| (slot.col, slot.col_span));
        for group in column_groups.values() {
            let widths = group
                .iter()
                .map(|(_, visual)| rect(&visual["position"]).width)
                .collect::<Vec<_>>();
            if widths.len() > 1 && range(&widths) > GRID_EPSILON {
                let (_, visual) = group[0];
                findings.push(design_finding(
                    rules::DESIGN_COLUMN_WIDTH_INCONSISTENT,
                    visual_handle(visual),
                    visual_path(visual),
                    format!(
                        "/report/pages/{}/visuals/{}/position/width",
                        context.page_index, group[0].0
                    ),
                    format!(
                        "visuals sharing column x={} have inconsistent widths",
                        trim_number(rect(&visual["position"]).x)
                    ),
                    json!({"column": rect(&visual["position"]).x, "widths": widths}),
                ));
            }
        }

        for visual in &context.visuals {
            if let Some(slot) = slot_for_visual(context, visual)
                && !preferred_family_matches(visual_type(visual), &slot.preferred_families)
            {
                findings.push(design_finding(
                    rules::DESIGN_SLOT_FAMILY_MISMATCH,
                    visual_handle(visual),
                    visual_path(visual),
                    visual_pointer(context, visual, "/slot"),
                    format!(
                        "visual {} ({}) is assigned to slot {} preferred for {}",
                        visual_handle(visual).unwrap_or("visual"),
                        visual_type(visual),
                        slot.name,
                        slot.preferred_families.join(", ")
                    ),
                    json!({
                        "slot": slot.name,
                        "visualType": visual_type(visual),
                        "preferredFamilies": slot.preferred_families
                    }),
                ));
            }
        }

        if context.rail.is_some() && context.raw_page.is_some() {
            // A rail is evaluated by lint_rail_sync after all pages are loaded;
            // retaining the context here keeps page-level geometry pure.
        }
    }
    if is_drillthrough_page(context) && !has_back_button(context)? {
        findings.push(design_finding(
            rules::DESIGN_DRILLTHROUGH_NO_BACK_BUTTON,
            context.handle(),
            context.page_path.as_deref(),
            format!("/report/pages/{}/pageBinding", context.page_index),
            format!(
                "drillthrough page {} has no back-navigation button",
                context.display_name()
            ),
            json!({"pageBinding": context.page.get("pageBinding").cloned().unwrap_or(Value::Null)}),
        ));
    }
    Ok(())
}

fn lint_rail_sync(contexts: &[PageContext], findings: &mut Vec<Value>) -> CliResult<()> {
    let mut groups = BTreeMap::<(String, RailSide), Vec<&PageContext>>::new();
    for context in contexts {
        if let (Some(template), Some(rail)) = (&context.template, context.rail) {
            groups
                .entry((template.name.clone(), rail))
                .or_default()
                .push(context);
        }
    }
    for pages in groups.values() {
        if pages.len() < 2 {
            continue;
        }
        let baseline = rail_signature(pages[0])?;
        for page in pages.iter().skip(1) {
            let signature = rail_signature(page)?;
            if signature != baseline {
                findings.push(design_finding(
                    rules::DESIGN_RAIL_NOT_SYNCED,
                    page.handle(),
                    page.page_path.as_deref(),
                    format!("/report/pages/{}/rail", page.page_index),
                    format!(
                        "rail slicers on page {} differ from the shared rail",
                        page.display_name()
                    ),
                    json!({"baseline": baseline, "actual": signature}),
                ));
            }
        }
    }
    Ok(())
}

fn rail_signature(context: &PageContext) -> CliResult<Vec<String>> {
    let mut signature = context
        .visuals
        .iter()
        .filter(|visual| is_slicer(visual))
        .filter(|visual| {
            slot_for_visual(context, visual).is_some_and(|slot| slot.name == "rail")
                || visual_is_on_rail(context, visual)
        })
        .flat_map(|visual| {
            visual["bindings"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|binding| {
                    format!(
                        "{}:{}:{}",
                        binding["role"].as_str().unwrap_or_default(),
                        binding["table"].as_str().unwrap_or_default(),
                        binding["column"]
                            .as_str()
                            .or_else(|| binding["field"].as_str())
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    signature.sort();
    Ok(signature)
}

fn slot_for_visual<'a>(context: &'a PageContext, visual: &Value) -> Option<&'a Slot> {
    let template = context.template.as_ref()?;
    if let Some(name) = explicit_slot_name(visual) {
        return template.slots.iter().find(|slot| slot.name == name);
    }
    let position = visual.get("position").unwrap_or(&Value::Null);
    template.slots.iter().find(|slot| {
        context
            .template_positions
            .get(&slot.name)
            .is_some_and(|expected| position_origin_matches(position, *expected))
    })
}

fn explicit_slot_name(visual: &Value) -> Option<String> {
    visual
        .get("slot")
        .and_then(Value::as_str)
        .or_else(|| visual.pointer("/layout/slot").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .or_else(|| {
            visual["annotations"].as_array().and_then(|annotations| {
                annotations.iter().find_map(|annotation| {
                    let name = annotation["name"].as_str().unwrap_or_default();
                    (name.eq_ignore_ascii_case("powerbi-cli.slot")
                        || name.eq_ignore_ascii_case("powerbi-cli.layout.slot"))
                    .then(|| annotation["value"].as_str().unwrap_or_default().to_string())
                })
            })
        })
}

fn visual_pointer(context: &PageContext, visual: &Value, suffix: &str) -> String {
    let index = context
        .visuals
        .iter()
        .position(|candidate| std::ptr::eq(candidate, visual))
        .unwrap_or_default();
    format!(
        "/report/pages/{}/visuals/{}{}",
        context.page_index, index, suffix
    )
}

fn preferred_family_matches(visual_type: &str, preferred: &[String]) -> bool {
    let visual_type = visual_type.to_ascii_lowercase();
    preferred.iter().any(|family| {
        let family = family.to_ascii_lowercase();
        family == visual_type
            || (family == "chart"
                && matches!(
                    visual_type.as_str(),
                    "linechart"
                        | "areachart"
                        | "stackedareachart"
                        | "barchart"
                        | "clusteredbarchart"
                        | "columnchart"
                        | "clusteredcolumnchart"
                        | "combochart"
                        | "lineclusteredcolumncombochart"
                ))
            || (family == "areachart" && visual_type == "stackedareachart")
            || (family == "barchart" && visual_type == "clusteredbarchart")
            || (family == "columnchart" && visual_type == "clusteredcolumnchart")
            || (family == "combochart" && visual_type == "lineclusteredcolumncombochart")
            || (family == "table" && matches!(visual_type.as_str(), "tableex" | "table"))
            || (family == "matrix" && matches!(visual_type.as_str(), "matrix" | "pivottable"))
            || (family == "card" && visual_type == "kpi")
    })
}

fn is_heading_visual(visual: &Value) -> bool {
    let visual_type = visual_type(visual).to_ascii_lowercase();
    visual_type == "textbox"
        || visual_type == "text"
        || visual_title(visual).is_some_and(|title| title.to_ascii_lowercase().contains("heading"))
}

fn visual_is_on_rail(context: &PageContext, visual: &Value) -> bool {
    let position = rect(&visual["position"]);
    match context.rail {
        Some(RailSide::Left) => position.x <= context.width * 0.2,
        Some(RailSide::Right) => position.x + position.width >= context.width * 0.8,
        None => false,
    }
}

fn is_drillthrough_page(context: &PageContext) -> bool {
    context.page["pageBinding"]["type"].as_str() == Some("Drillthrough")
        || context.page["type"].as_str() == Some("Drillthrough")
        || context.raw_page.as_ref().is_some_and(|page| {
            page["filterConfig"]["filters"]
                .as_array()
                .is_some_and(|filters| {
                    filters
                        .iter()
                        .any(|filter| filter["howCreated"] == "Drillthrough")
                })
        })
}

fn has_back_button(context: &PageContext) -> CliResult<bool> {
    for visual in &context.visuals {
        let title = visual_title(visual)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let visual_type = visual_type(visual).to_ascii_lowercase();
        if (visual_type.contains("button") || title == "back" || title.contains("back"))
            && (title.contains("back")
                || raw_visual(visual)?
                    .as_ref()
                    .is_some_and(raw_has_back_action))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn raw_has_back_action(raw: &Value) -> bool {
    let text = serde_json::to_string(raw)
        .unwrap_or_default()
        .to_ascii_lowercase();
    text.contains("back") || text.contains("backbutton")
}

fn raw_visual(visual: &Value) -> CliResult<Option<Value>> {
    let Some(path) = visual["path"].as_str() else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(read_json_value(&path)?))
}

fn visual_title(visual: &Value) -> Option<String> {
    visual["title"]
        .as_str()
        .map(ToOwned::to_owned)
        .filter(|value| !value.trim().is_empty())
}

fn budgeted_visual_count(context: &PageContext) -> usize {
    context
        .visuals
        .iter()
        .filter(|visual| {
            !slot_for_visual(context, visual).is_some_and(|slot| slot.name == "heading")
                && !is_heading_visual(visual)
        })
        .count()
}

fn group_visuals_by_slot<F>(
    context: &PageContext,
    key: F,
) -> BTreeMap<(u32, u32), Vec<(usize, &Value)>>
where
    F: Fn(&Slot) -> (u32, u32),
{
    let mut groups = BTreeMap::<(u32, u32), Vec<(usize, &Value)>>::new();
    for (index, visual) in context.visuals.iter().enumerate() {
        if let Some(slot) = slot_for_visual(context, visual) {
            groups.entry(key(slot)).or_default().push((index, visual));
        }
    }
    groups
}

fn range(values: &[f64]) -> f64 {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    max - min
}

#[derive(Debug, Clone, Copy, Default)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn rect(value: &Value) -> Rect {
    Rect {
        x: finite_or(value["x"].as_f64(), 0.0),
        y: finite_or(value["y"].as_f64(), 0.0),
        width: finite_or(value["width"].as_f64(), 0.0),
        height: finite_or(value["height"].as_f64(), 0.0),
    }
}

fn rectangles_overlap(left: Rect, right: Rect) -> bool {
    left.x < right.x + right.width - GRID_EPSILON
        && right.x < left.x + left.width - GRID_EPSILON
        && left.y < right.y + right.height - GRID_EPSILON
        && right.y < left.y + left.height - GRID_EPSILON
}

fn outside_page(position: Rect, width: f64, height: f64) -> bool {
    position.x < -GRID_EPSILON
        || position.y < -GRID_EPSILON
        || position.x + position.width > width + GRID_EPSILON
        || position.y + position.height > height + GRID_EPSILON
}

fn aligned_to_grid(position: Rect, page_width: f64, page_height: f64, grid: Grid) -> bool {
    let guides = grid::grid_guides(
        PageSize {
            width: page_width,
            height: page_height,
        },
        grid,
    );
    let x_aligned = guides
        .column_starts
        .iter()
        .any(|guide| (guide - position.x).abs() <= GRID_EPSILON);
    let right = position.x + position.width;
    let right_aligned = guides
        .column_ends
        .iter()
        .any(|guide| (guide - right).abs() <= GRID_EPSILON);
    let row_unit = guides.row_unit;
    let vertical_aligned = [position.y, position.y + position.height]
        .iter()
        .all(|value| ((*value / row_unit).round() * row_unit - *value).abs() <= GRID_EPSILON);
    x_aligned && right_aligned && vertical_aligned
}

fn position_matches(position: &Value, expected: SlotPosition) -> bool {
    let actual = rect(position);
    [
        (actual.x, expected.x),
        (actual.y, expected.y),
        (actual.width, expected.width),
        (actual.height, expected.height),
    ]
    .iter()
    .all(|(actual, expected)| (actual - expected).abs() <= GRID_EPSILON)
}

fn position_origin_matches(position: &Value, expected: SlotPosition) -> bool {
    let actual = rect(position);
    (actual.x - expected.x).abs() <= GRID_EPSILON && (actual.y - expected.y).abs() <= GRID_EPSILON
}

fn position_summary(visual: &Value) -> Value {
    let position = rect(&visual["position"]);
    json!({"x": position.x, "y": position.y, "width": position.width, "height": position.height})
}

fn visual_handle(visual: &Value) -> Option<&str> {
    visual["handle"].as_str()
}

fn visual_path(visual: &Value) -> Option<&Path> {
    visual["path"].as_str().map(Path::new)
}

fn visual_type(visual: &Value) -> &str {
    visual["visualType"].as_str().unwrap_or("unknown")
}

fn is_slicer(visual: &Value) -> bool {
    visual_type(visual).eq_ignore_ascii_case("slicer")
}

fn visual_slicer_mode(visual: &Value) -> CliResult<Option<String>> {
    if let Some(mode) = visual["slicerMode"]
        .as_str()
        .or_else(|| visual["mode"].as_str())
    {
        return Ok(Some(mode.to_ascii_lowercase()));
    }

    Ok(raw_visual(visual)?
        .as_ref()
        .and_then(|raw| raw.pointer("/visual/objects/data"))
        .and_then(Value::as_array)
        .and_then(|cards| {
            cards.iter().find_map(|card| {
                card.pointer("/properties/mode/expr/Literal/Value")
                    .and_then(Value::as_str)
            })
        })
        .map(|mode| mode.trim_matches('\'').to_ascii_lowercase()))
}

fn trim_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn finite_or(value: Option<f64>, fallback: f64) -> f64 {
    value.filter(|value| value.is_finite()).unwrap_or(fallback)
}

pub(crate) fn design_finding(
    rule_id: &str,
    handle: Option<&str>,
    path: Option<&Path>,
    pointer: String,
    message: String,
    evidence: Value,
) -> Value {
    let rule = rules::find_rule(rule_id).expect("design finding id is registered");
    json!({
        "code": rule.id,
        "ruleId": rule.id,
        "severity": rule.severity,
        "message": message,
        "handle": handle,
        "path": path.map(canonical_display),
        "pointer": pointer,
        "hint": rule.remediation,
        "sanitizeAction": rule.sanitize_action,
        "evidence": evidence,
        "recommendedActions": rule.sanitize_action.map(|action| vec![action]).unwrap_or_default()
    })
}

fn counts(findings: &[Value]) -> Value {
    let mut errors = 0_u64;
    let mut warnings = 0_u64;
    let mut info = 0_u64;
    for finding in findings {
        match finding["severity"].as_str() {
            Some("error") => errors += 1,
            Some("warning") => warnings += 1,
            Some("info") => info += 1,
            _ => {}
        }
    }
    json!({"errors": errors, "warnings": warnings, "info": info, "findings": findings.len()})
}

fn sort_findings(findings: &mut [Value]) {
    findings.sort_by(|left, right| {
        severity_rank(left)
            .cmp(&severity_rank(right))
            .then_with(|| {
                left["path"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["path"].as_str().unwrap_or_default())
            })
            .then_with(|| {
                left["pointer"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["pointer"].as_str().unwrap_or_default())
            })
            .then_with(|| {
                left["ruleId"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["ruleId"].as_str().unwrap_or_default())
            })
            .then_with(|| {
                left["handle"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["handle"].as_str().unwrap_or_default())
            })
            .then_with(|| {
                left["message"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["message"].as_str().unwrap_or_default())
            })
    });
}

fn severity_rank(finding: &Value) -> u8 {
    match finding["severity"].as_str() {
        Some("error") => 0,
        Some("warning") => 1,
        Some("info") => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn context(template_name: Option<&str>, visuals: Vec<Value>) -> PageContext {
        let template = template_name.map(|name| grid::template(name).expect("template"));
        let template_positions = template
            .as_ref()
            .map(|template| {
                grid::resolve(template, PageSize::STANDARD, None).expect("template positions")
            })
            .unwrap_or_default();
        let rail = template.as_ref().and_then(|template| template.rail);
        PageContext {
            page: json!({"handle": "page:Test", "displayName": "Test"}),
            page_index: 0,
            page_path: None,
            raw_page: None,
            width: PageSize::STANDARD.width,
            height: PageSize::STANDARD.height,
            visuals,
            template,
            template_positions,
            rail,
        }
    }

    fn visual(name: &str, visual_type: &str, position: SlotPosition) -> Value {
        json!({
            "handle": format!("visual:Test:{name}"),
            "title": name,
            "visualType": visual_type,
            "position": position
        })
    }

    fn slot_visual(context: &PageContext, slot: &str, visual_type: &str) -> Value {
        visual(
            slot,
            visual_type,
            *context.template_positions.get(slot).expect("slot position"),
        )
    }

    fn lint_context(context: &PageContext) -> BTreeSet<String> {
        let mut findings = Vec::new();
        lint_page_geometry(context, &mut findings).expect("lint geometry");
        findings
            .iter()
            .filter_map(|finding| finding["ruleId"].as_str().map(ToOwned::to_owned))
            .collect()
    }

    fn assert_planted_rule(rule: &str, clean: PageContext, planted: PageContext) {
        assert!(
            !lint_context(&clean).contains(rule),
            "clean fixture emitted {rule}"
        );
        assert!(
            lint_context(&planted).contains(rule),
            "planted fixture did not emit {rule}"
        );
    }

    #[test]
    fn every_named_template_is_clean_when_filled_with_preferred_slot_families() {
        let catalog = grid::catalog().expect("template catalog");
        for template in catalog.templates {
            let empty = context(Some(&template.name), Vec::new());
            let visuals = template
                .slots
                .iter()
                .map(|slot| {
                    slot_visual(
                        &empty,
                        &slot.name,
                        slot.preferred_families.first().expect("preferred family"),
                    )
                })
                .collect();
            let filled = context(Some(&template.name), visuals);
            assert!(
                lint_context(&filled).is_empty(),
                "template {} emitted findings: {:?}",
                template.name,
                lint_context(&filled)
            );
        }
    }

    #[test]
    fn grid_alignment_is_deterministic_and_tolerates_rounding() {
        assert!(aligned_to_grid(
            Rect {
                x: 24.0,
                y: 184.0,
                width: 608.0,
                height: 320.0
            },
            1280.0,
            720.0,
            Grid::default()
        ));
        assert!(!aligned_to_grid(
            Rect {
                x: 25.0,
                y: 184.0,
                width: 607.0,
                height: 320.0
            },
            1280.0,
            720.0,
            Grid::default()
        ));
    }

    #[test]
    fn overlap_and_outside_checks_use_half_open_rectangles() {
        assert!(rectangles_overlap(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0
            },
            Rect {
                x: 9.0,
                y: 9.0,
                width: 10.0,
                height: 10.0
            }
        ));
        assert!(!rectangles_overlap(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0
            },
            Rect {
                x: 10.0,
                y: 0.0,
                width: 10.0,
                height: 10.0
            }
        ));
        assert!(outside_page(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 11.0,
                height: 10.0
            },
            10.0,
            10.0
        ));
    }

    #[test]
    fn visual_overlap_has_positive_and_negative_fixtures() {
        let first = SlotPosition {
            x: 24.0,
            y: 24.0,
            width: 88.0,
            height: 80.0,
        };
        let adjacent = SlotPosition { x: 128.0, ..first };
        let overlapping = SlotPosition { x: 104.0, ..first };
        assert_planted_rule(
            rules::DESIGN_VISUAL_OVERLAP,
            context(
                None,
                vec![visual("a", "card", first), visual("b", "card", adjacent)],
            ),
            context(
                None,
                vec![visual("a", "card", first), visual("b", "card", overlapping)],
            ),
        );
    }

    #[test]
    fn visual_off_grid_has_positive_and_negative_fixtures() {
        let clean = SlotPosition {
            x: 24.0,
            y: 24.0,
            width: 88.0,
            height: 80.0,
        };
        let planted = SlotPosition { x: 25.0, ..clean };
        assert_planted_rule(
            rules::DESIGN_VISUAL_OFF_GRID,
            context(None, vec![visual("clean", "card", clean)]),
            context(None, vec![visual("planted", "card", planted)]),
        );
    }

    #[test]
    fn visual_outside_page_has_positive_and_negative_fixtures() {
        let clean = SlotPosition {
            x: 24.0,
            y: 24.0,
            width: 88.0,
            height: 80.0,
        };
        let planted = SlotPosition { x: -8.0, ..clean };
        assert_planted_rule(
            rules::REPORT_VISUAL_OUTSIDE_PAGE,
            context(None, vec![visual("clean", "card", clean)]),
            context(None, vec![visual("planted", "card", planted)]),
        );
    }

    #[test]
    fn row_height_inconsistent_has_positive_and_negative_fixtures() {
        let base = context(Some("overview"), Vec::new());
        let first = slot_visual(&base, "kpi.1", "card");
        let second = slot_visual(&base, "kpi.2", "card");
        let mut changed = second.clone();
        changed["position"]["height"] =
            Value::from(changed["position"]["height"].as_f64().expect("height") + 8.0);
        assert_planted_rule(
            rules::DESIGN_ROW_HEIGHT_INCONSISTENT,
            context(Some("overview"), vec![first.clone(), second]),
            context(Some("overview"), vec![first, changed]),
        );
    }

    #[test]
    fn column_width_inconsistent_has_positive_and_negative_fixtures() {
        let base = context(Some("overview"), Vec::new());
        let heading = slot_visual(&base, "heading", "textbox");
        let detail = slot_visual(&base, "detail", "tableEx");
        let mut changed = detail.clone();
        changed["position"]["width"] =
            Value::from(changed["position"]["width"].as_f64().expect("width") - 8.0);
        assert_planted_rule(
            rules::DESIGN_COLUMN_WIDTH_INCONSISTENT,
            context(Some("overview"), vec![heading.clone(), detail]),
            context(Some("overview"), vec![heading, changed]),
        );
    }

    #[test]
    fn page_overcrowded_has_positive_and_negative_fixtures() {
        let base = context(Some("time-series"), Vec::new());
        let position = *base.template_positions.get("primary").expect("primary");
        let make = |count| {
            (0..count)
                .map(|index| visual(&format!("v{index}"), "lineChart", position))
                .collect()
        };
        assert_planted_rule(
            rules::DESIGN_PAGE_OVERCROWDED,
            context(Some("time-series"), make(6)),
            context(Some("time-series"), make(7)),
        );
    }

    #[test]
    fn page_missing_heading_has_positive_and_negative_fixtures() {
        let base = context(Some("time-series"), Vec::new());
        let heading = slot_visual(&base, "heading", "textbox");
        let primary = slot_visual(&base, "primary", "lineChart");
        assert_planted_rule(
            rules::DESIGN_PAGE_MISSING_HEADING,
            context(Some("time-series"), vec![heading, primary.clone()]),
            context(Some("time-series"), vec![primary]),
        );
    }

    #[test]
    fn slicer_too_short_has_positive_and_negative_fixtures() {
        let clean = SlotPosition {
            x: 24.0,
            y: 24.0,
            width: 88.0,
            height: 104.0,
        };
        let planted = SlotPosition {
            height: 96.0,
            ..clean
        };
        let mut clean_visual = visual("clean", "slicer", clean);
        clean_visual["slicerMode"] = Value::String("between".to_string());
        let mut planted_visual = visual("planted", "slicer", planted);
        planted_visual["slicerMode"] = Value::String("between".to_string());
        assert_planted_rule(
            rules::DESIGN_SLICER_TOO_SHORT,
            context(None, vec![clean_visual]),
            context(None, vec![planted_visual]),
        );
    }

    #[test]
    fn slot_family_mismatch_has_positive_and_negative_fixtures() {
        let base = context(Some("time-series"), Vec::new());
        let clean = slot_visual(&base, "primary", "lineChart");
        let planted = slot_visual(&base, "primary", "tableEx");
        assert_planted_rule(
            rules::DESIGN_SLOT_FAMILY_MISMATCH,
            context(Some("time-series"), vec![clean]),
            context(Some("time-series"), vec![planted]),
        );
    }

    #[test]
    fn drillthrough_without_back_button_has_positive_and_negative_fixtures() {
        let position = SlotPosition {
            x: 24.0,
            y: 24.0,
            width: 88.0,
            height: 80.0,
        };
        let mut clean = context(None, vec![visual("Back", "button", position)]);
        clean.page["pageBinding"] = json!({"type": "Drillthrough"});
        let mut planted = context(None, vec![visual("Detail", "tableEx", position)]);
        planted.page["pageBinding"] = json!({"type": "Drillthrough"});
        assert_planted_rule(rules::DESIGN_DRILLTHROUGH_NO_BACK_BUTTON, clean, planted);
    }

    #[test]
    fn rail_not_synced_has_positive_and_negative_fixtures() {
        let base = context(Some("overview"), Vec::new());
        let mut rail = slot_visual(&base, "rail", "slicer");
        rail["bindings"] = json!([{"role": "Values", "table": "Dim", "column": "Region"}]);
        let first = context(Some("overview"), vec![rail.clone()]);
        let matching = context(Some("overview"), vec![rail.clone()]);
        let mut changed = rail;
        changed["bindings"][0]["column"] = Value::String("Category".to_string());
        let differing = context(Some("overview"), vec![changed]);
        let mut clean_findings = Vec::new();
        lint_rail_sync(&[first, matching], &mut clean_findings).expect("clean rail lint");
        assert!(clean_findings.is_empty());
        let mut planted_findings = Vec::new();
        lint_rail_sync(
            &[
                context(Some("overview"), vec![slot_visual(&base, "rail", "slicer")]),
                differing,
            ],
            &mut planted_findings,
        )
        .expect("planted rail lint");
        assert!(planted_findings.iter().any(|finding| {
            finding["ruleId"] == rules::DESIGN_RAIL_NOT_SYNCED
                && finding["pointer"]
                    .as_str()
                    .is_some_and(|pointer| pointer.starts_with('/'))
        }));
    }
}
