use crate::cli_support::{
    MutationMode, mode_name, required_project, set_mode, take_value, target_project,
};
use crate::design::grid::{
    Grid, GridGuides, PageSize, RailSide, SlotPosition, Template, content_slots, grid_guides,
    resolve_with_grid, round, template,
};
use crate::pbir::{PageRecord, PageSelector, find_page, load_report_snapshot, page_summary};
use crate::report_visuals::apply_positions;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, resolve_project, validate_project,
};
use serde_json::{Map, Number, Value, json};
use std::cmp::Ordering;
use std::path::PathBuf;

#[derive(Debug, Default)]
struct LayoutOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    preset: LayoutPreset,
    preset_explicit: bool,
    template: Option<String>,
    snap: bool,
    grid: Grid,
    page_size: Option<PageSize>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum LayoutPreset {
    #[default]
    Overview,
    Analysis,
    Detail,
    Grid,
}

struct PageLayoutPlan {
    page: PageRecord,
    /// `None` in snap mode, which keeps the existing arrangement instead of
    /// assigning template slots.
    template: Option<Template>,
    grid: Grid,
    preview: Value,
    assignments: Vec<LayoutAssignment>,
    updates: Vec<(PathBuf, Value)>,
    changes: Vec<Value>,
    warnings: Vec<Value>,
}

struct LayoutAssignment {
    visual: crate::pbir::VisualRecord,
    slot_name: String,
    position: Value,
}

pub(crate) fn layout_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report layout requires a subcommand: auto",
        )
        .with_hint("Auto layout moves existing visuals into deterministic canvas slots.")
        .with_suggested_command(
            "powerbi-cli report layout auto --project <project-dir-or.pbip> --page <page-handle> --preset overview --dry-run --json",
        ));
    };
    match action.as_str() {
        "auto" | "autofit" | "arrange" => auto_layout(rest),
        other => Err(CliError::invalid_args(format!(
            "unknown report layout command: {other}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report layout\"` for exact usage.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report layout\"")),
    }
}

fn auto_layout(args: &[String]) -> CliResult<Value> {
    let options = parse_auto_args(args)?;
    let source_project = required_project(options.project.clone(), "report layout auto")?;
    let mode = require_layout_mode(options.mode, "report layout auto")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, auto_layout)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let pages = selected_pages(&snapshot.pages, &options)?;
    let mut plans = Vec::new();
    for page in pages {
        plans.push(if options.snap {
            build_page_snap_plan(&page, &options)?
        } else {
            build_page_layout_plan(&page, &options)?
        });
    }

    let action = if options.snap { "snap" } else { "auto-layout" };
    let dry_run = matches!(mode, MutationMode::DryRun);
    for plan in &mut plans {
        let applied = apply_positions(
            &plan.updates,
            plan.page.width.as_f64(),
            plan.page.height.as_f64(),
            false,
            dry_run,
            "report layout auto",
        )?;
        for application in applied {
            if application.before == application.after {
                continue;
            }
            let visual = plan
                .assignments
                .iter()
                .find(|assignment| {
                    assignment
                        .visual
                        .path
                        .as_ref()
                        .is_some_and(|path| path == &application.path)
                })
                .map(|assignment| &assignment.visual)
                .ok_or_else(|| {
                    CliError::validation_failed(format!(
                        "layout application path is not assigned to a visual: {}",
                        application.path.display()
                    ))
                })?;
            plan.changes.push(json!({
                "kind": "pbir.visual.position",
                "action": action,
                "path": canonical_display(&application.path),
                "page": {
                    "handle": plan.page.handle,
                    "name": plan.page.name,
                    "displayName": plan.page.display_name
                },
                "visual": {
                    "handle": visual.handle,
                    "name": visual.name,
                    "title": visual.title,
                    "visualType": visual.visual_type
                },
                "before": application.before,
                "after": application.after
            }));
        }
    }

    layout_response(
        &target_resolved,
        mode,
        &plans,
        &snapshot.validation,
        options.snap,
    )
}

fn selected_pages(pages: &[PageRecord], options: &LayoutOptions) -> CliResult<Vec<PageRecord>> {
    if options.selector.handle.is_some() || options.selector.name.is_some() {
        return Ok(vec![
            find_page(pages, &options.selector, "report layout auto")?.clone(),
        ]);
    }
    Ok(pages.to_vec())
}

fn build_page_layout_plan(page: &PageRecord, options: &LayoutOptions) -> CliResult<PageLayoutPlan> {
    let page_size = options
        .page_size
        .unwrap_or_else(|| page_size_for_page(page));
    let template_name = options
        .template
        .as_deref()
        .unwrap_or_else(|| preset_template(options.preset));
    let template = template(template_name)?;
    let positions = if options.grid == Grid::default() {
        crate::design::grid::resolve(&template, page_size, None)?
    } else {
        resolve_with_grid(&template, page_size, options.grid, None)?
    };
    let slots = template.slots.iter().collect::<Vec<_>>();
    let content_slot_count = content_slots(&template).count();
    let visuals = sorted_visuals(page);
    if visuals.len() > slots.len() {
        return Err(CliError::invalid_args(format!(
            "layout template {} has {} slots ({} content slots) but page {} contains {} visuals",
            template.name,
            slots.len(),
            content_slot_count,
            page.handle,
            visuals.len()
        ))
        .with_pointer(format!("/pages/{}/visuals/{}", page.ordinal, slots.len()))
        .with_hint("Choose a template with more slots, remove visuals, or run `report layout auto --snap` to align the existing arrangement to the column guides.")
        .with_suggested_command(format!(
            "powerbi-cli report layout auto --project <project-dir-or.pbip> --page {} --snap --dry-run --json",
            page.handle
        )));
    }

    let mut assignments = Vec::new();
    let mut updates = Vec::new();
    let mut warnings = Vec::new();
    let mut used_slots = vec![false; slots.len()];
    for visual in visuals {
        // Prefer a slot whose catalog family matches the visual.  If no
        // preferred slot remains, use the next deterministic slot and expose
        // the mismatch as design.slot_family_mismatch for the design linter.
        let slot_index = slots
            .iter()
            .enumerate()
            .find(|(index, slot)| {
                !used_slots[*index]
                    && preferred_family_matches(&visual.visual_type, &slot.preferred_families)
            })
            .map(|(index, _)| index)
            // Structural slots (heading and rail) are reserved for matching
            // textbox/slicer visuals.  A chart or table that has no matching
            // preferred slot should consume a content slot before it can
            // displace a structural feature.
            .or_else(|| {
                slots.iter().enumerate().find_map(|(index, slot)| {
                    (!used_slots[index] && !is_structural_slot(slot)).then_some(index)
                })
            })
            .or_else(|| used_slots.iter().position(|used| !*used))
            .ok_or_else(|| {
                CliError::validation_failed("layout ran out of unassigned visual slots")
                    .with_pointer(format!("/pages/{}/visuals", page.ordinal))
            })?;
        used_slots[slot_index] = true;
        let slot = slots[slot_index];
        let resolved = positions.get(&slot.name).ok_or_else(|| {
            CliError::validation_failed(format!(
                "template {} did not resolve slot {}",
                template.name, slot.name
            ))
            .with_pointer(format!("/template/slots/{}", slot.name))
        })?;
        let after = position_from_slot(*resolved, assignments.len() as u64)?;
        let path = visual.path.clone().ok_or_else(|| {
            CliError::validation_failed(format!("visual has no path: {}", visual.handle))
        })?;
        let family_mismatch =
            !preferred_family_matches(&visual.visual_type, &slot.preferred_families);
        if family_mismatch {
            warnings.push(json!({
                "code": "design.slot_family_mismatch",
                "pointer": format!("/pages/{}/visuals/{}/slot", page.ordinal, assignments.len()),
                "message": format!("visual {} ({}) is assigned to slot {} preferred for {}", visual.handle, visual.visual_type, slot.name, slot.preferred_families.join(", ")),
                "visual": visual.handle,
                "slot": slot.name,
                "preferredFamilies": slot.preferred_families
            }));
        }
        assignments.push(LayoutAssignment {
            visual,
            slot_name: slot.name.clone(),
            position: after.clone(),
        });
        updates.push((path, after));
    }

    let preview_slots = template
        .slots
        .iter()
        .map(|slot| {
            let position = positions
                .get(&slot.name)
                .copied()
                .map(slot_position_json)
                .unwrap_or(Value::Null);
            json!({
                "name": slot.name,
                "col": slot.col,
                "row": slot.row,
                "colSpan": slot.col_span,
                "rowSpan": slot.row_span,
                "preferredFamilies": slot.preferred_families,
                "minFamily": slot.min_family,
                "position": position
            })
        })
        .collect::<Vec<_>>();
    let preview = json!({
        "page": page_summary(page),
        "pageSize": page_size,
        "grid": options.grid,
        "template": {
            "name": template.name,
            "rail": template.rail.map(RailSide::as_str),
            "headingBand": template.heading_band,
            "budget": template.budget
        },
        "slots": preview_slots,
        "assignments": assignments.iter().map(|assignment| json!({
            "visual": assignment.visual.handle,
            "visualType": assignment.visual.visual_type,
            "slot": assignment.slot_name,
            "preferredFamilies": template.slots.iter().find(|slot| slot.name == assignment.slot_name).map(|slot| slot.preferred_families.clone()).unwrap_or_default(),
            "position": assignment.position
        })).collect::<Vec<_>>(),
        "invariants": {
            "overlapFree": true,
            "minimumSizes": true,
            "withinPage": true
        }
    });
    Ok(PageLayoutPlan {
        page: page.clone(),
        template: Some(template),
        grid: options.grid,
        preview,
        assignments,
        updates,
        changes: Vec::new(),
        warnings,
    })
}

/// Snap the existing arrangement onto the twelve-column guides without
/// assigning template slots: each visual's left edge moves to the nearest
/// column start guide, its right edge to the nearest column end guide, and its
/// top/bottom to the nearest row-unit multiples.  Emergent overlaps are
/// reported as design.visual_overlap warnings, never silently written.
fn build_page_snap_plan(page: &PageRecord, options: &LayoutOptions) -> CliResult<PageLayoutPlan> {
    let page_size = options
        .page_size
        .unwrap_or_else(|| page_size_for_page(page))
        .validate()?;
    let grid = options.grid.validate()?;
    let guides = grid_guides(page_size, grid);
    let column_width = guides.column_width();
    if !column_width.is_finite() || column_width <= 0.0 {
        return Err(
            CliError::invalid_args("layout grid leaves no usable page width").with_pointer("/grid"),
        );
    }

    let mut assignments = Vec::new();
    let mut updates = Vec::new();
    let mut snapped_previews = Vec::new();
    for visual in sorted_visuals(page) {
        let before = position_rect(&visual.position);
        let after = snap_rect(before, page_size, &guides);
        let path = visual.path.clone().ok_or_else(|| {
            CliError::validation_failed(format!("visual has no path: {}", visual.handle))
        })?;
        let position = snapped_position_value(&visual.position, after)?;
        snapped_previews.push(json!({
            "visual": visual.handle,
            "visualType": visual.visual_type,
            "before": slot_position_json(before),
            "after": slot_position_json(after),
            "changed": !slot_positions_close(before, after)
        }));
        assignments.push(LayoutAssignment {
            visual,
            slot_name: "snap".to_string(),
            position: position.clone(),
        });
        updates.push((path, position));
    }

    // Snapping preserves the arrangement, so two visuals can land on the same
    // guides.  Report every emergent overlap with the design lint's rule id so
    // the caller sees the collision in this response instead of discovering it
    // in a later audit.
    let mut warnings = Vec::new();
    for (left_index, left) in assignments.iter().enumerate() {
        for (right_index, right) in assignments.iter().enumerate().skip(left_index + 1) {
            let left_rect = position_rect(&left.position);
            let right_rect = position_rect(&right.position);
            if snapped_rects_overlap(left_rect, right_rect) {
                warnings.push(json!({
                    "code": crate::rules::DESIGN_VISUAL_OVERLAP,
                    "pointer": format!("/pages/{}/visuals/{}/position", page.ordinal, right_index),
                    "message": format!(
                        "snapping to the twelve-column guides makes visuals {} and {} overlap on page {}",
                        left.visual.handle, right.visual.handle, page.display_name
                    ),
                    "visual": left.visual.handle,
                    "otherVisual": right.visual.handle,
                    "visualPosition": slot_position_json(left_rect),
                    "otherPosition": slot_position_json(right_rect)
                }));
            }
        }
    }

    let preview = json!({
        "page": page_summary(page),
        "pageSize": page_size,
        "grid": grid,
        "template": Value::Null,
        "snap": {
            "columnStarts": guides.column_starts.iter().copied().map(round).collect::<Vec<_>>(),
            "columnEnds": guides.column_ends.iter().copied().map(round).collect::<Vec<_>>(),
            "rowUnit": round(guides.row_unit)
        },
        "visuals": snapped_previews,
        "invariants": {
            "overlapFree": warnings.is_empty(),
            "withinPage": true
        }
    });
    Ok(PageLayoutPlan {
        page: page.clone(),
        template: None,
        grid,
        preview,
        assignments,
        updates,
        changes: Vec::new(),
        warnings,
    })
}

fn position_rect(position: &Value) -> SlotPosition {
    SlotPosition {
        x: position_number(position, "x"),
        y: position_number(position, "y"),
        width: position_number(position, "width"),
        height: position_number(position, "height"),
    }
}

fn snap_rect(before: SlotPosition, page_size: PageSize, guides: &GridGuides) -> SlotPosition {
    let start = nearest_guide(&guides.column_starts, before.x);
    let end = nearest_guide(&guides.column_ends, before.x + before.width).max(start);
    let left = round(guides.column_starts[start]);
    let right = round(guides.column_ends[end]);
    let row_unit = guides.row_unit;
    let max_bottom = (page_size.height / row_unit).floor() * row_unit;
    let mut top = ((before.y / row_unit).round() * row_unit).max(0.0);
    let mut bottom = (((before.y + before.height) / row_unit).round() * row_unit).min(max_bottom);
    if bottom < top + row_unit {
        bottom = top + row_unit;
    }
    if bottom > max_bottom {
        bottom = max_bottom;
        top = (bottom - row_unit).max(0.0);
    }
    let top = round(top);
    let bottom = round(bottom);
    SlotPosition {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn nearest_guide(guides: &[f64], value: f64) -> usize {
    let mut best = 0;
    let mut best_distance = f64::INFINITY;
    for (index, guide) in guides.iter().enumerate() {
        let distance = (guide - value).abs();
        if distance < best_distance {
            best = index;
            best_distance = distance;
        }
    }
    best
}

/// Replace only the geometry fields so z, tabOrder, and unknown position
/// fields survive the snap untouched.  Whole numbers are written as integers,
/// which keeps a second snap (and an already aligned visual) a byte-level
/// no-op.
fn snapped_position_value(before: &Value, after: SlotPosition) -> CliResult<Value> {
    let mut object = before.as_object().cloned().unwrap_or_default();
    for (name, value) in [
        ("x", after.x),
        ("y", after.y),
        ("width", after.width),
        ("height", after.height),
    ] {
        object.insert(name.to_string(), snapped_number(value, name)?);
    }
    Ok(Value::Object(object))
}

fn snapped_number(value: f64, name: &str) -> CliResult<Value> {
    let number = finite_number(value, name)?;
    if value.fract() == 0.0 && value <= u64::MAX as f64 {
        return Ok(Value::Number(Number::from(value as u64)));
    }
    Ok(number)
}

fn slot_positions_close(left: SlotPosition, right: SlotPosition) -> bool {
    [
        (left.x, right.x),
        (left.y, right.y),
        (left.width, right.width),
        (left.height, right.height),
    ]
    .iter()
    .all(|(before, after)| (before - after).abs() <= SNAP_EPSILON)
}

/// Same half-open, epsilon-tolerant predicate the design lint uses for
/// design.visual_overlap, so a collision reported here is exactly one the
/// audit would flag.
fn snapped_rects_overlap(left: SlotPosition, right: SlotPosition) -> bool {
    left.x < right.x + right.width - SNAP_EPSILON
        && right.x < left.x + left.width - SNAP_EPSILON
        && left.y < right.y + right.height - SNAP_EPSILON
        && right.y < left.y + left.height - SNAP_EPSILON
}

const SNAP_EPSILON: f64 = 0.01;

fn sorted_visuals(page: &PageRecord) -> Vec<crate::pbir::VisualRecord> {
    let mut visuals = page.visuals.clone();
    visuals.sort_by(compare_visuals);
    visuals
}

fn compare_visuals(
    left: &crate::pbir::VisualRecord,
    right: &crate::pbir::VisualRecord,
) -> Ordering {
    let left_y = position_number(&left.position, "y");
    let right_y = position_number(&right.position, "y");
    left_y
        .partial_cmp(&right_y)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            position_number(&left.position, "x")
                .partial_cmp(&position_number(&right.position, "x"))
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| left.name.cmp(&right.name))
}

fn position_number(value: &Value, field: &str) -> f64 {
    value[field].as_f64().unwrap_or(0.0)
}

fn preset_template(preset: LayoutPreset) -> &'static str {
    match preset {
        LayoutPreset::Overview => "overview",
        LayoutPreset::Analysis => "time-series",
        LayoutPreset::Detail => "drillthrough-detail",
        LayoutPreset::Grid => "kpi-strip-trend-breakdown",
    }
}

fn page_size_for_page(page: &PageRecord) -> PageSize {
    PageSize {
        width: page.width.as_f64().unwrap_or(PageSize::STANDARD.width),
        height: page.height.as_f64().unwrap_or(PageSize::STANDARD.height),
    }
}

fn position_from_slot(slot: SlotPosition, z: u64) -> CliResult<Value> {
    let mut object = Map::new();
    object.insert("x".to_string(), finite_number(slot.x, "x")?);
    object.insert("y".to_string(), finite_number(slot.y, "y")?);
    object.insert("z".to_string(), Value::Number(Number::from(z)));
    object.insert("height".to_string(), finite_number(slot.height, "height")?);
    object.insert("width".to_string(), finite_number(slot.width, "width")?);
    object.insert("tabOrder".to_string(), Value::Number(Number::from(z)));
    Ok(Value::Object(object))
}

fn slot_position_json(slot: SlotPosition) -> Value {
    json!({
        "x": slot.x,
        "y": slot.y,
        "width": slot.width,
        "height": slot.height
    })
}

fn finite_number(value: f64, name: &str) -> CliResult<Value> {
    if !value.is_finite() || value < 0.0 {
        return Err(CliError::invalid_args(format!(
            "layout {name} must be a finite nonnegative number"
        )));
    }
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| CliError::invalid_args(format!("layout {name} is not a JSON number")))
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

fn is_structural_slot(slot: &crate::design::grid::Slot) -> bool {
    matches!(slot.name.as_str(), "heading" | "rail")
}

fn layout_response(
    resolved: &ResolvedProject,
    mode: MutationMode,
    plans: &[PageLayoutPlan],
    dry_validation: &crate::ValidationReport,
    snap: bool,
) -> CliResult<Value> {
    let dry_run = matches!(mode, MutationMode::DryRun);
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(resolved)?)
    };
    let validation_ok = validation
        .as_ref()
        .map(|report| report.errors.is_empty())
        .unwrap_or_else(|| dry_validation.errors.is_empty());
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let changes = plans
        .iter()
        .flat_map(|plan| plan.changes.iter().cloned())
        .collect::<Vec<_>>();
    let readback = format!(
        "powerbi-cli report visuals list --project {} --json",
        command_arg(&resolved.project_dir)
    );
    let wireframe = format!(
        "powerbi-cli report wireframe export {} --json",
        command_arg(&resolved.project_dir)
    );
    let inspect = format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&resolved.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );
    Ok(json!({
        "schema": "powerbi-cli.report.layout.autoMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": if snap { "snap" } else { "auto-layout" },
        "snap": snap,
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "layoutPlan": {
            "template": plans
                .first()
                .and_then(|plan| plan.template.as_ref())
                .map(|template| template.name.clone()),
            "grid": plans.first().map(|plan| plan.grid),
            "pages": plans.iter().map(|plan| page_summary(&plan.page)).collect::<Vec<_>>(),
            "changedVisuals": changes.len()
        },
        "preview": {
            "pages": plans.iter().map(|plan| plan.preview.clone()).collect::<Vec<_>>(),
            "svg": false
        },
        "changes": changes,
        "warnings": plans.iter().flat_map(|plan| plan.warnings.iter().cloned()).collect::<Vec<_>>(),
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors,
            "counts": {
                "tables": report.tables,
                "relationships": report.relationships,
                "measures": report.measures,
                "pages": report.pages,
                "visuals": report.visuals,
                "boundVisuals": report.bound_visuals
            }
        })),
        "readbackCommand": readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, wireframe, inspect, validate]
    }))
}

fn parse_auto_args(args: &[String]) -> CliResult<LayoutOptions> {
    let mut options = LayoutOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--page" | "--handle" => {
                set_page_selector(&mut options.selector, take_value(args, &mut i, "--page")?);
            }
            "--preset" => {
                if options.template.is_some() || options.preset_explicit {
                    return Err(CliError::invalid_args(
                        "report layout auto accepts either --preset or --template, not both",
                    )
                    .with_pointer("/template")
                    .with_hint("Use a named --template for the design-system grid, or keep the legacy --preset alias."));
                }
                options.preset = parse_preset(&take_value(args, &mut i, "--preset")?)?;
                options.preset_explicit = true;
            }
            "--template" => {
                if options.template.is_some() || options.preset_explicit {
                    return Err(CliError::invalid_args(
                        "report layout auto accepts either --template or --preset, not both",
                    )
                    .with_pointer("/template"));
                }
                options.template = Some(take_value(args, &mut i, "--template")?);
            }
            "--snap" => {
                options.snap = true;
                i += 1;
            }
            "--grid" => {
                parse_grid_override(&mut options.grid, &take_value(args, &mut i, "--grid")?)?
            }
            "--page-size" | "--size" => {
                options.page_size =
                    Some(parse_page_size(&take_value(args, &mut i, "--page-size")?)?);
            }
            "--margin" => options.grid.margin = take_f64(args, &mut i, "--margin")?,
            "--gap" | "--gutter" => options.grid.gutter = take_f64(args, &mut i, "--gap")?,
            "--row-unit" | "--rowUnit" => {
                options.grid.row_unit = take_f64(args, &mut i, "--row-unit")?;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report layout auto",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report layout auto",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report layout auto",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report layout auto flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report layout auto\"`.")
                .with_suggested_command(
                    "powerbi-cli report layout auto --project <project-dir-or.pbip> --page <page-handle> --template overview --dry-run --json",
                ));
            }
        }
    }
    if options.snap && (options.template.is_some() || options.preset_explicit) {
        return Err(CliError::invalid_args(
            "report layout auto --snap preserves the existing arrangement and cannot combine with --template or --preset",
        )
        .with_pointer("/snap")
        .with_hint("Use --snap alone to move edges onto the nearest column guides, or drop --snap to assign template slots.")
        .with_suggested_command(
            "powerbi-cli report layout auto --project <project-dir-or.pbip> --page <page-handle> --snap --dry-run --json",
        ));
    }
    Ok(options)
}

fn parse_grid_override(grid: &mut Grid, raw: &str) -> CliResult<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::invalid_args("--grid requires a value")
            .with_pointer("/grid")
            .with_hint("Use --grid columns=12,gutter=16,margin=24,rowUnit=8."));
    }
    if trimmed.starts_with('{') {
        let parsed: Grid = serde_json::from_str(trimmed).map_err(|error| {
            CliError::invalid_args(format!("--grid must be a grid object: {error}"))
                .with_pointer("/grid")
        })?;
        *grid = parsed;
        return Ok(());
    }
    for entry in trimmed.split(',') {
        let (key, value) = entry.split_once('=').ok_or_else(|| {
            CliError::invalid_args(format!("--grid entry must be key=value: {entry}"))
                .with_pointer("/grid")
                .with_hint("Use --grid columns=12,gutter=16,margin=24,rowUnit=8.")
        })?;
        let key = key.trim();
        let value = value.trim();
        match key.to_ascii_lowercase().as_str() {
            "columns" => {
                grid.columns = value.parse::<u32>().map_err(|_| {
                    CliError::invalid_args("--grid columns must be a positive integer")
                        .with_pointer("/grid/columns")
                })?;
            }
            "gutter" => grid.gutter = parse_grid_number(value, "/grid/gutter")?,
            "margin" => grid.margin = parse_grid_number(value, "/grid/margin")?,
            "rowunit" | "row-unit" => grid.row_unit = parse_grid_number(value, "/grid/rowUnit")?,
            _ => {
                return Err(
                    CliError::invalid_args(format!("unknown --grid setting: {key}"))
                        .with_pointer(format!(
                            "/grid/{}",
                            crate::diagnostics::escape_pointer_token(key)
                        ))
                        .with_hint("Supported settings are columns, gutter, margin, and rowUnit."),
                );
            }
        }
    }
    Ok(())
}

fn parse_grid_number(value: &str, pointer: &str) -> CliResult<f64> {
    value.parse::<f64>().map_err(|_| {
        CliError::invalid_args(format!("grid value must be a number: {value}"))
            .with_pointer(pointer)
    })
}

fn parse_page_size(raw: &str) -> CliResult<PageSize> {
    if let Some(page_size) = PageSize::preset(raw) {
        return Ok(page_size);
    }
    let (width, height) = raw
        .split_once('x')
        .or_else(|| raw.split_once('X'))
        .ok_or_else(|| {
            CliError::invalid_args(format!("unknown page-size preset: {raw}"))
                .with_pointer("/pageSize")
                .with_hint("Use 1280x720, 1920x1080, standard, or wide.")
        })?;
    let width = width.parse::<f64>().map_err(|_| {
        CliError::invalid_args("page-size width must be a number").with_pointer("/pageSize/width")
    })?;
    let height = height.parse::<f64>().map_err(|_| {
        CliError::invalid_args("page-size height must be a number").with_pointer("/pageSize/height")
    })?;
    Ok(PageSize { width, height })
}

fn set_page_selector(selector: &mut PageSelector, value: String) {
    if value.starts_with("page:") {
        selector.handle = Some(value);
    } else {
        selector.name = Some(value);
    }
}

fn parse_preset(value: &str) -> CliResult<LayoutPreset> {
    match value.to_ascii_lowercase().as_str() {
        "overview" | "dashboard" => Ok(LayoutPreset::Overview),
        "analysis" | "focus" => Ok(LayoutPreset::Analysis),
        "detail" | "details" => Ok(LayoutPreset::Detail),
        "grid" => Ok(LayoutPreset::Grid),
        other => Err(CliError::invalid_args(format!(
            "invalid layout preset: {other}"
        ))
        .with_hint("Use overview, analysis, detail, or grid.")
        .with_suggested_command(
            "powerbi-cli report layout auto --project <project-dir-or.pbip> --preset overview --dry-run --json",
        )),
    }
}

fn take_f64(args: &[String], index: &mut usize, flag: &str) -> CliResult<f64> {
    let raw = take_value(args, index, flag)?;
    raw.parse::<f64>().map_err(|_| {
        CliError::invalid_args(format!("{flag} must be a number"))
            .with_suggested_command(
                "powerbi-cli report layout auto --project <project-dir-or.pbip> --preset overview --dry-run --json",
            )
    })
}

fn require_layout_mode(mode: Option<MutationMode>, command: &str) -> CliResult<MutationMode> {
    mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --dry-run, --in-place, or --out-dir <dir>"
        ))
        .with_hint("Start with `--dry-run`; use `--in-place` or `--out-dir` only after reviewing the returned positions.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-handle> --preset overview --dry-run --json"
        ))
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_grid_setting_escapes_rfc6901_pointer_tokens() {
        let error = super::parse_grid_override(&mut super::Grid::default(), "a/b~c=1")
            .expect_err("unknown setting");
        assert_eq!(error.code, "invalid_args");
        assert_eq!(error.pointer(), Some("/grid/a~1b~0c"));
    }
}
