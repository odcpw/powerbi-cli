use crate::cli_support::{
    MutationMode, mode_name, required_project, set_mode, take_value, target_project,
};
use crate::pbir::{PageRecord, PageSelector, find_page, load_report_snapshot, page_summary};
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Number, Value, json};
use std::cmp::Ordering;
use std::path::PathBuf;

#[derive(Debug, Default)]
struct LayoutOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    preset: LayoutPreset,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    margin: Option<f64>,
    gap: Option<f64>,
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
    changes: Vec<Value>,
    writes: Vec<VisualWrite>,
}

struct VisualWrite {
    path: PathBuf,
    visual_json: Value,
}

#[derive(Debug, Clone, Copy)]
struct CanvasSlots {
    width: f64,
    height: f64,
    margin: f64,
    gap: f64,
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
        plans.push(build_page_layout_plan(&page, &options)?);
    }

    if !matches!(mode, MutationMode::DryRun) {
        for plan in &plans {
            for write in &plan.writes {
                write_json_atomic(&write.path, &write.visual_json)?;
            }
        }
    }

    layout_response(&target_resolved, mode, &plans, &snapshot.validation)
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
    let page_width = page.width.as_f64().unwrap_or(1280.0);
    let page_height = page.height.as_f64().unwrap_or(720.0);
    let margin = options.margin.unwrap_or(32.0);
    let gap = options.gap.unwrap_or(24.0);
    validate_spacing(page_width, page_height, margin, gap)?;

    let positions = match options.preset {
        LayoutPreset::Overview => overview_positions(page, page_width, page_height, margin, gap)?,
        LayoutPreset::Analysis => analysis_positions(page, page_width, page_height, margin, gap)?,
        LayoutPreset::Detail => detail_positions(page, page_width, page_height, margin, gap)?,
        LayoutPreset::Grid => grid_positions(page, page_width, page_height, margin, gap)?,
    };
    let mut changes = Vec::new();
    let mut writes = Vec::new();
    for (visual, after) in positions {
        let path = visual.path.as_ref().ok_or_else(|| {
            CliError::validation_failed(format!("visual has no path: {}", visual.handle))
        })?;
        let mut visual_json = read_json_value(path)?;
        let before = visual_json["position"].clone();
        if before != after {
            visual_json["position"] = after.clone();
            changes.push(json!({
                "kind": "pbir.visual.position",
                "action": "auto-layout",
                "path": canonical_display(path),
                "page": {
                    "handle": page.handle,
                    "name": page.name,
                    "displayName": page.display_name
                },
                "visual": {
                    "handle": visual.handle,
                    "name": visual.name,
                    "title": visual.title,
                    "visualType": visual.visual_type
                },
                "before": before,
                "after": after
            }));
            writes.push(VisualWrite {
                path: path.clone(),
                visual_json,
            });
        }
    }
    Ok(PageLayoutPlan {
        page: page.clone(),
        changes,
        writes,
    })
}

fn overview_positions(
    page: &PageRecord,
    width: f64,
    height: f64,
    margin: f64,
    gap: f64,
) -> CliResult<Vec<(crate::pbir::VisualRecord, Value)>> {
    let mut visuals = sorted_visuals(page);
    let mut cards = Vec::new();
    let mut others = Vec::new();
    for visual in visuals.drain(..) {
        if visual.visual_type == "card" {
            cards.push(visual);
        } else {
            others.push(visual);
        }
    }
    let mut out = Vec::new();
    let mut y = margin;
    let mut z = 0_u64;
    if !cards.is_empty() {
        let columns = cards.len().min(4);
        let card_height = 116.0;
        let card_width = slot_width(width, margin, gap, columns)?;
        for (index, visual) in cards.into_iter().enumerate() {
            let col = index % columns;
            let row = index / columns;
            let x = margin + col as f64 * (card_width + gap);
            let y_pos = y + row as f64 * (card_height + gap);
            out.push((visual, position(x, y_pos, card_width, card_height, z)?));
            z += 1;
        }
        let rows = div_ceil(out.len(), columns);
        y += rows as f64 * card_height + (rows.saturating_sub(1)) as f64 * gap + gap;
    }
    out.extend(grid_slots(
        others,
        CanvasSlots {
            width,
            height,
            margin,
            gap,
        },
        y,
        2,
        z,
    )?);
    Ok(out)
}

fn analysis_positions(
    page: &PageRecord,
    width: f64,
    height: f64,
    margin: f64,
    gap: f64,
) -> CliResult<Vec<(crate::pbir::VisualRecord, Value)>> {
    let visuals = sorted_visuals(page);
    if visuals.len() <= 2 {
        return grid_slots(
            visuals,
            CanvasSlots {
                width,
                height,
                margin,
                gap,
            },
            margin,
            1,
            0,
        );
    }
    let mut out = Vec::new();
    let usable_width = width - margin * 2.0;
    let usable_height = height - margin * 2.0;
    let main_width = (usable_width * 0.62).max(320.0);
    let side_width = usable_width - main_width - gap;
    let main = visuals[0].clone();
    out.push((
        main,
        position(margin, margin, main_width, usable_height, 0)?,
    ));
    let side = visuals.into_iter().skip(1).collect::<Vec<_>>();
    let side_slots = stacked_slots(
        side,
        margin + main_width + gap,
        margin,
        side_width,
        usable_height,
        gap,
        1,
    )?;
    out.extend(side_slots);
    Ok(out)
}

fn detail_positions(
    page: &PageRecord,
    width: f64,
    height: f64,
    margin: f64,
    gap: f64,
) -> CliResult<Vec<(crate::pbir::VisualRecord, Value)>> {
    let mut visuals = sorted_visuals(page);
    visuals.sort_by(|left, right| {
        detail_rank(left)
            .cmp(&detail_rank(right))
            .then_with(|| compare_visuals(left, right))
    });
    grid_slots(
        visuals,
        CanvasSlots {
            width,
            height,
            margin,
            gap,
        },
        margin,
        1,
        0,
    )
}

fn grid_positions(
    page: &PageRecord,
    width: f64,
    height: f64,
    margin: f64,
    gap: f64,
) -> CliResult<Vec<(crate::pbir::VisualRecord, Value)>> {
    let visuals = sorted_visuals(page);
    let columns = if visuals.len() <= 2 {
        visuals.len().max(1)
    } else {
        3
    };
    grid_slots(
        visuals,
        CanvasSlots {
            width,
            height,
            margin,
            gap,
        },
        margin,
        columns,
        0,
    )
}

fn grid_slots(
    visuals: Vec<crate::pbir::VisualRecord>,
    canvas: CanvasSlots,
    start_y: f64,
    columns: usize,
    start_z: u64,
) -> CliResult<Vec<(crate::pbir::VisualRecord, Value)>> {
    if visuals.is_empty() {
        return Ok(Vec::new());
    }
    let columns = columns.max(1).min(visuals.len());
    let rows = div_ceil(visuals.len(), columns);
    let slot_w = slot_width(canvas.width, canvas.margin, canvas.gap, columns)?;
    let available_h = (canvas.height - start_y - canvas.margin).max(120.0);
    let slot_h =
        ((available_h - canvas.gap * rows.saturating_sub(1) as f64) / rows as f64).max(80.0);
    let mut out = Vec::new();
    for (index, visual) in visuals.into_iter().enumerate() {
        let col = index % columns;
        let row = index / columns;
        let x = canvas.margin + col as f64 * (slot_w + canvas.gap);
        let y = start_y + row as f64 * (slot_h + canvas.gap);
        out.push((
            visual,
            position(x, y, slot_w, slot_h, start_z + index as u64)?,
        ));
    }
    Ok(out)
}

fn stacked_slots(
    visuals: Vec<crate::pbir::VisualRecord>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    gap: f64,
    start_z: u64,
) -> CliResult<Vec<(crate::pbir::VisualRecord, Value)>> {
    if visuals.is_empty() {
        return Ok(Vec::new());
    }
    let slot_h =
        ((height - gap * visuals.len().saturating_sub(1) as f64) / visuals.len() as f64).max(80.0);
    visuals
        .into_iter()
        .enumerate()
        .map(|(index, visual)| {
            Ok((
                visual,
                position(
                    x,
                    y + index as f64 * (slot_h + gap),
                    width,
                    slot_h,
                    start_z + index as u64,
                )?,
            ))
        })
        .collect()
}

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

fn detail_rank(visual: &crate::pbir::VisualRecord) -> usize {
    match visual.visual_type.as_str() {
        "card" => 0,
        "tableEx" => 2,
        _ => 1,
    }
}

fn position_number(value: &Value, field: &str) -> f64 {
    value[field].as_f64().unwrap_or(0.0)
}

fn position(x: f64, y: f64, width: f64, height: f64, z: u64) -> CliResult<Value> {
    let mut object = Map::new();
    object.insert("x".to_string(), number(x, "x")?);
    object.insert("y".to_string(), number(y, "y")?);
    object.insert("z".to_string(), Value::Number(Number::from(z)));
    object.insert("height".to_string(), number(height, "height")?);
    object.insert("width".to_string(), number(width, "width")?);
    object.insert("tabOrder".to_string(), Value::Number(Number::from(z)));
    Ok(Value::Object(object))
}

fn number(value: f64, name: &str) -> CliResult<Value> {
    if !value.is_finite() || value < 0.0 {
        return Err(CliError::invalid_args(format!(
            "layout {name} must be a finite nonnegative number"
        )));
    }
    Number::from_f64((value * 100.0).round() / 100.0)
        .map(Value::Number)
        .ok_or_else(|| CliError::invalid_args(format!("layout {name} is not a JSON number")))
}

fn slot_width(width: f64, margin: f64, gap: f64, columns: usize) -> CliResult<f64> {
    if columns == 0 {
        return Err(CliError::invalid_args(
            "layout requires at least one column",
        ));
    }
    Ok(
        ((width - margin * 2.0 - gap * columns.saturating_sub(1) as f64) / columns as f64)
            .max(80.0),
    )
}

fn div_ceil(value: usize, by: usize) -> usize {
    value.div_ceil(by.max(1))
}

fn validate_spacing(width: f64, height: f64, margin: f64, gap: f64) -> CliResult<()> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(CliError::validation_failed(
            "page width and height must be positive numbers",
        ));
    }
    if !margin.is_finite() || !gap.is_finite() || margin < 0.0 || gap < 0.0 {
        return Err(CliError::invalid_args(
            "--margin and --gap must be finite nonnegative numbers",
        ));
    }
    if margin * 2.0 >= width || margin * 2.0 >= height {
        return Err(CliError::invalid_args(
            "--margin leaves no usable canvas space",
        ));
    }
    Ok(())
}

fn layout_response(
    resolved: &ResolvedProject,
    mode: MutationMode,
    plans: &[PageLayoutPlan],
    dry_validation: &crate::ValidationReport,
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
        "action": "auto-layout",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "layoutPlan": {
            "pages": plans.iter().map(|plan| page_summary(&plan.page)).collect::<Vec<_>>(),
            "changedVisuals": changes.len()
        },
        "changes": changes,
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
                options.preset = parse_preset(&take_value(args, &mut i, "--preset")?)?;
            }
            "--margin" => options.margin = Some(take_f64(args, &mut i, "--margin")?),
            "--gap" => options.gap = Some(take_f64(args, &mut i, "--gap")?),
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
                    "powerbi-cli report layout auto --project <project-dir-or.pbip> --page <page-handle> --preset overview --dry-run --json",
                ));
            }
        }
    }
    Ok(options)
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
