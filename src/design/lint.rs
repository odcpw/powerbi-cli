//! Deterministic design-system linting for existing PBIP report trees.
//!
//! Design lint is intentionally read-only.  It consumes the embedded grid and
//! formatting catalogs, reports every finding with a stable rule id and RFC
//! 6901 pointer, and describes a mechanical action when one is safe to plan.
//! Applying those actions belongs to the operation/auto-improve beads; this
//! module never writes guessed PBIR.

use super::grid::{self, Grid, PageSize, RailSide, Slot, SlotPosition, Template};
use crate::formatting_catalog::formatting_catalog_entries;
use crate::rules::{self, RuleFamily};
use crate::{CliResult, ResolvedProject, canonical_display, read_json_value};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) const DESIGN_LINT_SCHEMA: &str = "powerbi-cli.design.lint.v1";
const GRID_EPSILON: f64 = 0.01;
const MIN_NORMAL_TEXT_CONTRAST: f64 = 4.5;
const MIN_FONT_SIZE: f64 = 10.0;

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
    // Loading both catalogs here is deliberate: a malformed embedded catalog
    // is a validation failure, never a reason to silently skip a check.
    let grid_catalog = grid::catalog()?;
    let formatting_entries = formatting_catalog_entries()?;
    let theme_palette = report_theme_palette(resolved)?;
    let pages = deep["report"]["pages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut findings = Vec::new();
    let mut contexts = Vec::new();

    for (page_index, page) in pages.iter().enumerate() {
        let context = PageContext::load(
            resolved,
            page,
            page_index,
            &grid_catalog.grid,
            &grid_catalog.templates,
        )?;
        lint_page_geometry(&context, &mut findings)?;
        lint_page_titles(&context, &mut findings)?;
        lint_page_visual_styles(&context, formatting_entries, &theme_palette, &mut findings)?;
        lint_page_model_and_sort(&context, deep, &mut findings)?;
        contexts.push(context);
    }
    lint_rail_sync(&contexts, &mut findings)?;

    sort_findings(&mut findings);
    rules::ensure_finding_ids_registered(&findings, "ruleId")?;
    let counts = counts(&findings);
    let deferred = deferred_rules(&findings);
    let project = canonical_display(&resolved.project_dir);
    Ok(json!({
        "schema": DESIGN_LINT_SCHEMA,
        "status": "available",
        "ok": counts["errors"].as_u64().unwrap_or_default() == 0,
        "projectDir": project,
        "counts": counts,
        "ruleIds": design_rule_ids(),
        "evaluatedRules": evaluated_rules(&findings, &deferred),
        "deferredRules": deferred,
        "grid": {
            "schema": grid_catalog.schema,
            "columns": grid_catalog.grid.columns,
            "gutter": grid_catalog.grid.gutter,
            "margin": grid_catalog.grid.margin,
            "rowUnit": grid_catalog.grid.row_unit,
            "templates": grid_catalog.templates.iter().map(|template| template.name.clone()).collect::<Vec<_>>()
        },
        "formattingCatalog": {
            "entryCount": formatting_entries.len(),
            "entries": formatting_entries.iter().map(|entry| format!("{}.{}", entry.object, entry.property)).collect::<Vec<_>>()
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
    let mut best: Option<(usize, String, Template)> = None;
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
        let candidate = (score, template.name.clone(), template.clone());
        let replace = best.as_ref().is_none_or(|current| {
            score > current.0 || (score == current.0 && candidate.1 < current.1)
        });
        if replace {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, template)| template)
}

fn lint_page_geometry(context: &PageContext, findings: &mut Vec<Value>) -> CliResult<()> {
    for (left_index, left) in context.visuals.iter().enumerate() {
        for (right_index, right) in context.visuals.iter().enumerate().skip(left_index + 1) {
            let left_position = rect(left.get("position").unwrap_or(&Value::Null));
            let right_position = rect(right.get("position").unwrap_or(&Value::Null));
            if rectangles_overlap(left_position, right_position) {
                findings.push(design_finding(
                    rules::DESIGN_VISUAL_OVERLAP,
                    visual_handle(left),
                    visual_path(left),
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
        if !aligned_to_grid(position, Grid::default().row_unit) {
            findings.push(design_finding(
                rules::DESIGN_VISUAL_OFF_GRID,
                visual_handle(visual),
                visual_path(visual),
                format!(
                    "/report/pages/{}/visuals/{}/position",
                    context.page_index, visual_index
                ),
                format!(
                    "visual edges are not aligned to the {} px design grid: {}",
                    Grid::default().row_unit,
                    visual_title(visual).unwrap_or_else(|| "visual".to_string())
                ),
                json!({
                    "position": position_summary(visual),
                    "rowUnit": Grid::default().row_unit
                }),
            ));
        }
        let mode = visual_slicer_mode(visual);
        let minimum_height = if mode == Some("between") { 104.0 } else { 76.0 };
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
            && context.visuals.len() > budget
        {
            findings.push(design_finding(
                rules::DESIGN_PAGE_OVERCROWDED,
                context.handle(),
                context.page_path.as_deref(),
                format!("/report/pages/{}/visuals", context.page_index),
                format!(
                    "page {} contains {} visuals but template {} budgets {}",
                    context.display_name(),
                    context.visuals.len(),
                    template.name,
                    budget
                ),
                json!({
                    "template": template.name,
                    "visualCount": context.visuals.len(),
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

        let row_groups = group_visuals_by(&context.visuals, |visual| rect(&visual["position"]).y);
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
        let column_groups =
            group_visuals_by(&context.visuals, |visual| rect(&visual["position"]).x);
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
            "/pageBinding".to_string(),
            format!(
                "drillthrough page {} has no back-navigation button",
                context.display_name()
            ),
            json!({"pageBinding": context.page.get("pageBinding").cloned().unwrap_or(Value::Null)}),
        ));
    }
    Ok(())
}

fn lint_page_titles(context: &PageContext, findings: &mut Vec<Value>) -> CliResult<()> {
    let mut title_styles = BTreeMap::<TitleCase, usize>::new();
    let mut title_values = Vec::new();
    for (index, visual) in context.visuals.iter().enumerate() {
        let raw = raw_visual(visual)?;
        let title = raw
            .as_ref()
            .and_then(raw_title)
            .or_else(|| visual_title(visual));
        let has_visible_title = raw.as_ref().is_some_and(|value| raw_title(value).is_some());
        if !has_visible_title {
            findings.push(design_finding(
                rules::DESIGN_TITLE_MISSING,
                visual_handle(visual),
                visual_path(visual),
                format!(
                    "/report/pages/{}/visuals/{}/title",
                    context.page_index, index
                ),
                "visual is missing a visible title".to_string(),
                json!({"visualType": visual_type(visual)}),
            ));
            continue;
        }
        let Some(title) = title.filter(|title| !title.trim().is_empty()) else {
            continue;
        };
        let style = title_case(&title);
        *title_styles.entry(style).or_default() += 1;
        title_values.push((index, visual, title, style));
    }

    let duplicate_groups = title_values.iter().fold(
        BTreeMap::<String, Vec<usize>>::new(),
        |mut groups, (index, _, title, _)| {
            groups
                .entry(normalized_title(title))
                .or_default()
                .push(*index);
            groups
        },
    );
    for indexes in duplicate_groups
        .values()
        .filter(|indexes| indexes.len() > 1)
    {
        let first = indexes[0];
        let visual = &context.visuals[first];
        findings.push(design_finding(
            rules::DESIGN_TITLE_DUPLICATE,
            visual_handle(visual),
            visual_path(visual),
            format!("/report/pages/{}/visuals/{}/title", context.page_index, first),
            format!("title is duplicated on page {}", context.display_name()),
            json!({"visuals": indexes.iter().map(|index| visual_handle(&context.visuals[*index])).collect::<Vec<_>>()}),
        ));
    }
    if title_styles.len() > 1 {
        let majority = title_styles
            .iter()
            .max_by(|(left_style, left_count), (right_style, right_count)| {
                left_count
                    .cmp(right_count)
                    .then_with(|| right_style.cmp(left_style))
            })
            .map(|(style, _)| *style)
            .unwrap_or(TitleCase::Sentence);
        for (index, visual, title, style) in title_values {
            if style != majority {
                findings.push(design_finding(
                    rules::DESIGN_TITLE_CASE_INCONSISTENT,
                    visual_handle(visual),
                    visual_path(visual),
                    format!("/report/pages/{}/visuals/{}/title", context.page_index, index),
                    format!("title casing differs from the page convention: {title}"),
                    json!({"title": title, "style": style.as_str(), "pageStyle": majority.as_str()}),
                ));
            }
        }
    }
    Ok(())
}

fn lint_page_visual_styles(
    context: &PageContext,
    formatting_entries: &[crate::formatting_catalog::FormattingCatalogEntry],
    theme_palette: &[String],
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    let catalog_has_font_size = formatting_entries
        .iter()
        .filter(|entry| entry.property.eq_ignore_ascii_case("fontSize"))
        .next()
        .is_some();
    for (index, visual) in context.visuals.iter().enumerate() {
        let raw = raw_visual(visual)?;
        let Some(raw) = raw else { continue };
        for (pointer, value) in find_property_literals(&raw, "fontSize") {
            let Some(size) = literal_number(value) else {
                continue;
            };
            if size < MIN_FONT_SIZE && catalog_has_font_size {
                findings.push(design_finding(
                    rules::DESIGN_FONT_BELOW_MINIMUM,
                    visual_handle(visual),
                    visual_path(visual),
                    format!(
                        "/report/pages/{}/visuals/{}/{}",
                        context.page_index,
                        index,
                        pointer.trim_start_matches('/')
                    ),
                    format!(
                        "font size {} is below the {} px minimum",
                        trim_number(size),
                        trim_number(MIN_FONT_SIZE)
                    ),
                    json!({"fontSize": size, "minimum": MIN_FONT_SIZE, "source": pointer}),
                ));
            }
        }
        let colors = find_color_literals(&raw);
        if let Some((foreground, background, pointer)) = contrast_pair(&colors) {
            let ratio = contrast_ratio(foreground, background);
            if ratio + GRID_EPSILON < MIN_NORMAL_TEXT_CONTRAST {
                findings.push(design_finding(
                    rules::DESIGN_CONTRAST_BELOW_AA,
                    visual_handle(visual),
                    visual_path(visual),
                    format!("/report/pages/{}/visuals/{}/{}", context.page_index, index, pointer.trim_start_matches('/')),
                    format!("foreground/background contrast is {:.2}:1, below the 4.5:1 WCAG AA threshold", ratio),
                    json!({"foreground": foreground, "background": background, "ratio": ratio, "minimum": MIN_NORMAL_TEXT_CONTRAST}),
                ));
            }
        }
        if has_large_magnitude_marker(&raw) && !has_display_units(&raw) {
            findings.push(design_finding(
                rules::DESIGN_DISPLAY_UNITS_MISSING,
                visual_handle(visual),
                visual_path(visual),
                format!(
                    "/report/pages/{}/visuals/{}/displayUnits",
                    context.page_index, index
                ),
                "large-magnitude visual has no display-units policy".to_string(),
                json!({"marker": "largeMagnitude", "displayUnits": Value::Null}),
            ));
        }
        for (pointer, color) in find_color_literals_with_pointers(&raw) {
            if !theme_palette.is_empty()
                && !theme_palette
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&color))
            {
                findings.push(design_finding(
                    rules::DESIGN_PALETTE_DRIFT,
                    visual_handle(visual),
                    visual_path(visual),
                    format!(
                        "/report/pages/{}/visuals/{}/{}",
                        context.page_index,
                        index,
                        pointer.trim_start_matches('/')
                    ),
                    format!("visual color {color} is outside the registered theme palette"),
                    json!({"color": color, "palette": theme_palette}),
                ));
            }
        }
    }
    Ok(())
}

fn lint_page_model_and_sort(
    context: &PageContext,
    deep: &Value,
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    if let Some(tables) = deep["model"]["tables"].as_array() {
        for table in tables {
            if let Some(measures) = table["measures"].as_array() {
                for measure in measures {
                    let format = measure["properties"]["formatString"]
                        .as_str()
                        .filter(|value| !value.trim().is_empty());
                    let dynamic = measure["properties"]["formatStringDefinition"]
                        .as_object()
                        .filter(|value| !value.is_empty());
                    if format.is_none() && dynamic.is_none() {
                        findings.push(design_finding(
                            rules::DESIGN_NUMBER_FORMAT_MISSING,
                            measure["handle"].as_str(),
                            measure["path"].as_str().map(Path::new),
                            format!(
                                "/model/tables/{}/measures/{}/properties/formatString",
                                table["name"].as_str().unwrap_or("table"),
                                measure["name"].as_str().unwrap_or("measure")
                            ),
                            format!(
                                "measure {} has no explicit number format",
                                measure["name"].as_str().unwrap_or("measure")
                            ),
                            json!({"table": table["name"], "measure": measure["name"]}),
                        ));
                    }
                }
            }
        }
    }
    for (index, visual) in context.visuals.iter().enumerate() {
        let visual_type_name = visual_type(visual).to_ascii_lowercase();
        let candidate = visual_type_name.contains("bar") || visual_type_name.contains("column");
        let has_category = visual["bindings"].as_array().is_some_and(|bindings| {
            bindings.iter().any(|binding| {
                binding["role"]
                    .as_str()
                    .is_some_and(|role| role.eq_ignore_ascii_case("category"))
            })
        });
        let has_measure = visual["bindings"].as_array().is_some_and(|bindings| {
            bindings
                .iter()
                .any(|binding| binding["kind"] == "measure" || binding["measure"].is_string())
        });
        let explicit_ranking = raw_visual(visual)?.as_ref().is_some_and(has_ranking_marker);
        if candidate
            && has_category
            && has_measure
            && (context.template.is_some() || explicit_ranking)
            && !has_descending_sort(visual)?
        {
            findings.push(design_finding(
                rules::DESIGN_RANKING_NOT_SORTED,
                visual_handle(visual),
                visual_path(visual),
                format!("/report/pages/{}/visuals/{}/bindings", context.page_index, index),
                "ranking visual has no explicit descending measure sort".to_string(),
                json!({"visualType": visual_type_name, "hasCategory": has_category, "hasMeasure": has_measure}),
            ));
        }
    }
    Ok(())
}

fn lint_rail_sync(contexts: &[PageContext], findings: &mut Vec<Value>) -> CliResult<()> {
    let mut groups = BTreeMap::<RailSide, Vec<&PageContext>>::new();
    for context in contexts {
        if let Some(rail) = context.rail {
            groups.entry(rail).or_default().push(context);
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
            .is_some_and(|expected| position_matches(position, *expected))
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

fn has_ranking_marker(raw: &Value) -> bool {
    raw["ranking"].as_bool() == Some(true)
        || raw["largeMagnitude"].as_bool() == Some(true)
        || raw["annotations"].as_array().is_some_and(|annotations| {
            annotations.iter().any(|annotation| {
                let name = annotation["name"].as_str().unwrap_or_default();
                let value = annotation["value"].as_str().unwrap_or_default();
                (name.eq_ignore_ascii_case("powerbi-cli.ranking")
                    || name.eq_ignore_ascii_case("powerbi-cli.design.ranking"))
                    && (value.is_empty() || value.eq_ignore_ascii_case("true"))
            })
        })
}

fn has_descending_sort(visual: &Value) -> CliResult<bool> {
    if visual["bindings"].as_array().is_some_and(|bindings| {
        bindings.iter().any(|binding| {
            binding["sortDirection"].as_str().is_some_and(|direction| {
                direction.eq_ignore_ascii_case("descending")
                    || direction.eq_ignore_ascii_case("desc")
            })
        })
    }) {
        return Ok(true);
    }
    let Some(raw) = raw_visual(visual)? else {
        return Ok(false);
    };
    Ok(raw
        .pointer("/visual/query/sortDefinition/sort")
        .and_then(Value::as_array)
        .is_some_and(|sorts| {
            sorts.iter().any(|sort| {
                sort["direction"].as_str().is_some_and(|direction| {
                    direction.eq_ignore_ascii_case("descending")
                        || direction.eq_ignore_ascii_case("desc")
                })
            })
        }))
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

fn report_theme_palette(resolved: &ResolvedProject) -> CliResult<Vec<String>> {
    let path = resolved.report_dir.join("definition").join("report.json");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let report = read_json_value(&path)?;
    let mut colors = BTreeSet::new();
    collect_hex_colors(&report["themeCollection"], &mut colors);
    Ok(colors.into_iter().collect())
}

fn collect_hex_colors(value: &Value, colors: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for child in object.values() {
                if let Some(text) = child.as_str()
                    && let Some(color) = normalize_color(text.to_string())
                {
                    colors.insert(color);
                }
                collect_hex_colors(child, colors);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_hex_colors(child, colors);
            }
        }
        _ => {}
    }
}

fn raw_title(raw: &Value) -> Option<String> {
    for (text_pointer, show_pointer) in [
        (
            "/visual/visualContainerObjects/title/0/properties/text/expr/Literal/Value",
            "/visual/visualContainerObjects/title/0/properties/show/expr/Literal/Value",
        ),
        (
            "/visual/objects/title/0/properties/text/expr/Literal/Value",
            "/visual/objects/title/0/properties/show/expr/Literal/Value",
        ),
        ("/title", ""),
    ] {
        let Some(value) = raw.pointer(text_pointer).and_then(Value::as_str) else {
            continue;
        };
        if !show_pointer.is_empty()
            && raw
                .pointer(show_pointer)
                .and_then(Value::as_str)
                .is_some_and(|show| show.eq_ignore_ascii_case("false"))
        {
            continue;
        }
        return Some(decode_literal_text(value));
    }
    None
}

fn visual_title(visual: &Value) -> Option<String> {
    visual["title"]
        .as_str()
        .map(ToOwned::to_owned)
        .filter(|value| !value.trim().is_empty())
}

fn decode_literal_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        trimmed[1..trimmed.len() - 1].replace("''", "'")
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TitleCase {
    Sentence,
    Title,
    Upper,
    Lower,
    Mixed,
}

impl TitleCase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sentence => "sentence",
            Self::Title => "title",
            Self::Upper => "upper",
            Self::Lower => "lower",
            Self::Mixed => "mixed",
        }
    }
}

fn title_case(value: &str) -> TitleCase {
    if value == value.to_ascii_uppercase() && value.chars().any(char::is_alphabetic) {
        return TitleCase::Upper;
    }
    if value == value.to_ascii_lowercase() && value.chars().any(char::is_alphabetic) {
        return TitleCase::Lower;
    }
    let words = value.split_whitespace().collect::<Vec<_>>();
    if words.iter().all(|word| {
        word.chars()
            .next()
            .is_some_and(|character| character.is_uppercase())
    }) {
        return TitleCase::Title;
    }
    if value
        .chars()
        .next()
        .is_some_and(|character| character.is_uppercase())
    {
        TitleCase::Sentence
    } else {
        TitleCase::Mixed
    }
}

fn normalized_title(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn group_visuals_by<F>(visuals: &[Value], key: F) -> BTreeMap<i64, Vec<(usize, &Value)>>
where
    F: Fn(&Value) -> f64,
{
    let mut groups = BTreeMap::<i64, Vec<(usize, &Value)>>::new();
    for (index, visual) in visuals.iter().enumerate() {
        let value = key(visual);
        let bucket = (value / GRID_EPSILON).round() as i64;
        groups.entry(bucket).or_default().push((index, visual));
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

fn aligned_to_grid(position: Rect, row_unit: f64) -> bool {
    [position.x, position.y, position.width, position.height]
        .iter()
        .all(|value| ((*value / row_unit).round() * row_unit - *value).abs() <= GRID_EPSILON)
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

fn visual_slicer_mode(visual: &Value) -> Option<&str> {
    visual["slicerMode"]
        .as_str()
        .or_else(|| visual["mode"].as_str())
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

fn design_finding(
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
        "supportedAction": rule.sanitize_action.is_some(),
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

fn evaluated_rules(_findings: &[Value], deferred: &[Value]) -> Vec<String> {
    let deferred_ids = deferred
        .iter()
        .filter_map(|item| item["ruleId"].as_str())
        .collect::<BTreeSet<_>>();
    design_rule_ids()
        .into_iter()
        .filter(|id| !deferred_ids.contains(id))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>()
}

fn deferred_rules(findings: &[Value]) -> Vec<Value> {
    let has_contrast = findings
        .iter()
        .any(|finding| finding["ruleId"] == rules::DESIGN_CONTRAST_BELOW_AA);
    let has_palette = findings
        .iter()
        .any(|finding| finding["ruleId"] == rules::DESIGN_PALETTE_DRIFT);
    let mut deferred = Vec::new();
    if !has_contrast {
        deferred.push(json!({
            "ruleId": rules::DESIGN_CONTRAST_BELOW_AA,
            "reason": "no explicit foreground/background pair was present in the report visuals"
        }));
    }
    if !has_palette {
        deferred.push(json!({
            "ruleId": rules::DESIGN_PALETTE_DRIFT,
            "reason": "no registered theme palette was present in the report"
        }));
    }
    deferred
}

fn find_property_literals<'a>(value: &'a Value, property: &str) -> Vec<(String, &'a Value)> {
    let mut result = Vec::new();
    walk_property_literals(value, "", property, &mut result);
    result
}

fn walk_property_literals<'a>(
    value: &'a Value,
    pointer: &str,
    property: &str,
    result: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_pointer = format!("{pointer}/{key}");
                if key.eq_ignore_ascii_case(property) {
                    result.push((child_pointer.clone(), child));
                }
                walk_property_literals(child, &child_pointer, property, result);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                walk_property_literals(child, &format!("{pointer}/{index}"), property, result);
            }
        }
        _ => {}
    }
}

fn literal_number(value: &Value) -> Option<f64> {
    value
        .pointer("/expr/Literal/Value")
        .and_then(Value::as_str)
        .and_then(|value| value.trim_end_matches('D').parse::<f64>().ok())
        .or_else(|| value.as_f64())
}

fn find_color_literals(value: &Value) -> Vec<(String, String)> {
    find_color_literals_with_pointers(value)
}

fn find_color_literals_with_pointers(value: &Value) -> Vec<(String, String)> {
    let mut result = Vec::new();
    walk_color_literals(value, "", &mut result);
    result
}

fn walk_color_literals(value: &Value, pointer: &str, result: &mut Vec<(String, String)>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_pointer = format!("{pointer}/{key}");
                let key_color = key.to_ascii_lowercase().contains("color")
                    || key.eq_ignore_ascii_case("foreground")
                    || key.eq_ignore_ascii_case("background");
                if key_color {
                    if let Some(color) = literal_text(child).and_then(normalize_color) {
                        result.push((child_pointer.clone(), color));
                    }
                }
                walk_color_literals(child, &child_pointer, result);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                walk_color_literals(child, &format!("{pointer}/{index}"), result);
            }
        }
        _ => {}
    }
}

fn literal_text(value: &Value) -> Option<String> {
    value
        .pointer("/expr/Literal/Value")
        .and_then(Value::as_str)
        .map(decode_literal_text)
        .or_else(|| value.as_str().map(ToOwned::to_owned))
}

fn normalize_color(value: String) -> Option<String> {
    let value = value.trim().to_ascii_uppercase();
    let value = value.strip_prefix('#')?;
    if (value.len() == 6 || value.len() == 8)
        && value.chars().all(|character| character.is_ascii_hexdigit())
    {
        Some(format!("#{value}"))
    } else {
        None
    }
}

fn contrast_pair(colors: &[(String, String)]) -> Option<([f64; 3], [f64; 3], String)> {
    let foreground = colors
        .iter()
        .find(|(pointer, _)| pointer.to_ascii_lowercase().contains("foreground"))?;
    let background = colors
        .iter()
        .find(|(pointer, _)| pointer.to_ascii_lowercase().contains("background"))?;
    Some((
        parse_rgb(&foreground.1)?,
        parse_rgb(&background.1)?,
        foreground.0.clone(),
    ))
}

fn parse_rgb(color: &str) -> Option<[f64; 3]> {
    let hex = color.strip_prefix('#')?;
    let (r, g, b) = if hex.len() == 8 {
        (&hex[2..4], &hex[4..6], &hex[6..8])
    } else {
        (&hex[0..2], &hex[2..4], &hex[4..6])
    };
    Some([
        u8::from_str_radix(r, 16).ok()? as f64 / 255.0,
        u8::from_str_radix(g, 16).ok()? as f64 / 255.0,
        u8::from_str_radix(b, 16).ok()? as f64 / 255.0,
    ])
}

fn contrast_ratio(foreground: [f64; 3], background: [f64; 3]) -> f64 {
    let foreground = relative_luminance(foreground);
    let background = relative_luminance(background);
    (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
}

fn relative_luminance(rgb: [f64; 3]) -> f64 {
    let linear = rgb.map(|component| {
        if component <= 0.03928 {
            component / 12.92
        } else {
            ((component + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
}

fn has_large_magnitude_marker(raw: &Value) -> bool {
    raw["largeMagnitude"].as_bool() == Some(true)
        || raw["annotations"].as_array().is_some_and(|annotations| {
            annotations.iter().any(|annotation| {
                let name = annotation["name"].as_str().unwrap_or_default();
                name.eq_ignore_ascii_case("powerbi-cli.largeMagnitude")
                    && annotation["value"]
                        .as_str()
                        .unwrap_or("true")
                        .eq_ignore_ascii_case("true")
            })
        })
}

fn has_display_units(raw: &Value) -> bool {
    find_property_literals(raw, "displayUnits")
        .iter()
        .any(|(_, value)| literal_text(value).is_some_and(|value| !value.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_alignment_is_deterministic_and_tolerates_rounding() {
        assert!(aligned_to_grid(
            Rect {
                x: 32.0,
                y: 184.0,
                width: 600.0,
                height: 320.0
            },
            8.0
        ));
        assert!(!aligned_to_grid(
            Rect {
                x: 31.0,
                y: 184.0,
                width: 600.0,
                height: 320.0
            },
            8.0
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
    fn title_case_classification_supports_page_convention_checks() {
        assert_eq!(title_case("Revenue trend"), TitleCase::Sentence);
        assert_eq!(title_case("Revenue Trend"), TitleCase::Title);
        assert_eq!(title_case("REVENUE"), TitleCase::Upper);
    }

    #[test]
    fn contrast_ratio_matches_wcag_reference_pair() {
        let white = parse_rgb("#FFFFFF").expect("white");
        let black = parse_rgb("#000000").expect("black");
        assert!((contrast_ratio(white, black) - 21.0).abs() < 0.01);
    }
}
