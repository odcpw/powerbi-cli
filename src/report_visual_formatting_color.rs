use crate::pbir::{VisualRecord, VisualSelector, find_visual, load_report_snapshot, visual_detail};
use crate::project_io::{copy_project_dir, write_json_atomic};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationMode {
    DryRun,
    InPlace,
    OutDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorSlot {
    TitleFontColor,
    DataPointFill,
}

impl ColorSlot {
    fn as_str(self) -> &'static str {
        match self {
            Self::TitleFontColor => "title.fontColor",
            Self::DataPointFill => "dataPoint.fill",
        }
    }

    fn pointer(self) -> &'static str {
        match self {
            Self::TitleFontColor => {
                "/visual/objects/title/0/properties/fontColor/solid/color/expr/Literal/Value"
            }
            Self::DataPointFill => {
                "/visual/objects/dataPoint/0/properties/fill/solid/color/expr/Literal/Value"
            }
        }
    }
}

#[derive(Debug, Default)]
struct ColorOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    slot: Option<ColorSlot>,
    color: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

pub(crate) fn set_color_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_color_args(args)?;
    let source_project = required_project(
        options.project.clone(),
        "report visuals formatting set-color",
    )?;
    require_visual_selector(&options.selector, "report visuals formatting set-color")?;
    require_color_intent(&options)?;
    let mode = require_mode(options.mode, "report visuals formatting set-color")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, set_color_formatting)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals formatting set-color",
    )?;
    let visual_path = visual_path(visual, "report visuals formatting set-color")?;
    let mut visual_json = read_json_value(visual_path)?;
    let before = visual_color_state(&visual_json, options.include_raw);
    let pointers = apply_color_patch(&mut visual_json, &options)?;
    let after = visual_color_state(&visual_json, options.include_raw);

    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        write_json_atomic(visual_path, &visual_json)?;
    }
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(&target_resolved)?)
    };
    let validation_ok = validation
        .as_ref()
        .map(|report| report.errors.is_empty())
        .unwrap_or(true);
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&target_resolved.project_dir);
    let readback = format!(
        "powerbi-cli report visuals formatting show --project {} --handle {} --json",
        project_arg,
        shell_arg(&visual.handle)
    );
    let raw_review = format!(
        "powerbi-cli report visuals formatting show --project {} --handle {} --include-raw --json",
        project_arg,
        shell_arg(&visual.handle)
    );
    let visual_readback = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&visual.handle)
    );
    let wireframe = format!(
        "powerbi-cli report wireframe export {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let inspect = format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let slot = options.slot.expect("slot checked by require_color_intent");
    let color = options
        .color
        .as_deref()
        .expect("color checked by require_color_intent");

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.formatting.colorMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "set-color",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": visual_detail(visual),
        "colorPlan": {
            "strategy": "patch-static-literal",
            "rawIncluded": options.include_raw,
            "requested": {
                "slot": slot.as_str(),
                "color": color
            },
            "before": before,
            "after": after
        },
        "changes": [{
            "kind": "pbir.visual.colorFormatting",
            "action": "patch",
            "path": canonical_display(visual_path),
            "jsonPointers": pointers,
            "rawIncluded": options.include_raw,
            "before": before,
            "after": after
        }],
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
        "rawReviewCommand": raw_review,
        "visualReadbackCommand": visual_readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, raw_review, visual_readback, wireframe, inspect, validate]
    }))
}

fn apply_color_patch(visual_json: &mut Value, options: &ColorOptions) -> CliResult<Vec<String>> {
    let slot = options.slot.expect("slot checked by require_color_intent");
    let color = options
        .color
        .as_deref()
        .expect("color checked by require_color_intent");
    match slot {
        ColorSlot::TitleFontColor => {
            let properties = ensure_visual_object_properties(
                visual_json,
                "title",
                json!({ "properties": {} }),
                false,
            )?;
            properties.insert("fontColor".to_string(), solid_color_expression(color));
        }
        ColorSlot::DataPointFill => {
            let properties = ensure_visual_object_properties(
                visual_json,
                "dataPoint",
                json!({
                    "selector": {
                        "data": [{ "dataViewWildcard": { "matchingOption": "InstancesAndTotals" } }]
                    },
                    "properties": {}
                }),
                true,
            )?;
            properties.insert("fill".to_string(), solid_color_expression(color));
        }
    }
    Ok(vec![slot.pointer().to_string()])
}

fn visual_color_state(visual_json: &Value, include_raw: bool) -> Value {
    let title_font_color = color_at(
        visual_json,
        "/visual/objects/title/0/properties/fontColor/solid/color/expr/Literal/Value",
    );
    let data_point_fill = color_at(
        visual_json,
        "/visual/objects/dataPoint/0/properties/fill/solid/color/expr/Literal/Value",
    );
    let mut state = json!({
        "titleFontColor": title_font_color,
        "dataPointFill": data_point_fill,
        "slots": {
            "title.fontColor": title_font_color,
            "dataPoint.fill": data_point_fill
        }
    });
    if include_raw {
        state["raw"] = json!({
            "title": visual_json.pointer("/visual/objects/title/0").cloned().unwrap_or(Value::Null),
            "dataPoint": visual_json.pointer("/visual/objects/dataPoint/0").cloned().unwrap_or(Value::Null)
        });
    }
    state
}

fn color_at(visual_json: &Value, pointer: &str) -> Option<String> {
    visual_json
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(decode_text_literal)
}

fn ensure_visual_object_properties<'a>(
    visual_json: &'a mut Value,
    object_name: &str,
    default_card: Value,
    require_static_selector: bool,
) -> CliResult<&'a mut Map<String, Value>> {
    let root = json_object_mut(visual_json, "visual.json root")?;
    let visual = root
        .entry("visual".to_string())
        .or_insert_with(|| json!({}));
    let visual = json_object_mut(visual, "visual.json visual")?;
    let objects = visual
        .entry("objects".to_string())
        .or_insert_with(|| json!({}));
    let objects = json_object_mut(objects, "/visual/objects")?;
    let value = objects
        .entry(object_name.to_string())
        .or_insert_with(|| json!([default_card]));
    if let Some(cards) = value.as_array_mut()
        && cards.is_empty()
    {
        cards.push(default_card);
    }
    object_properties(value, object_name, require_static_selector)
}

fn object_properties<'a>(
    value: &'a mut Value,
    object_name: &str,
    require_static_selector: bool,
) -> CliResult<&'a mut Map<String, Value>> {
    let cards = value.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "/visual/objects/{object_name} must be an array before set-color can patch it"
        ))
        .with_hint("Use `report visuals formatting show --include-raw` to inspect this visual before editing raw PBIR.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --include-raw --json",
        )
    })?;
    if cards.is_empty() {
        return Err(CliError::validation_failed(format!(
            "/visual/objects/{object_name} has no formatting card to patch"
        )));
    }
    let card = json_object_mut(&mut cards[0], "formatting card")?;
    if require_static_selector {
        ensure_static_or_wildcard_selector(card)?;
    }
    let properties = card
        .entry("properties".to_string())
        .or_insert_with(|| json!({}));
    json_object_mut(properties, "formatting properties")
}

fn ensure_static_or_wildcard_selector(card: &Map<String, Value>) -> CliResult<()> {
    let Some(selector) = card.get("selector") else {
        return Ok(());
    };
    if is_static_or_wildcard_selector(selector) {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "report visuals formatting set-color refuses dataPoint cards with data-bound selectors",
    )
    .with_hint("Only static or dataViewWildcard dataPoint.fill patches are supported; use formatting extract/apply for reviewed raw bundles.")
    .with_suggested_command(
        "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --include-raw --json",
    ))
}

fn is_static_or_wildcard_selector(selector: &Value) -> bool {
    let Some(selector) = selector.as_object() else {
        return false;
    };
    if selector.is_empty() {
        return true;
    }
    if selector.len() != 1 {
        return false;
    }
    let Some(items) = selector.get("data").and_then(Value::as_array) else {
        return false;
    };
    !items.is_empty() && items.iter().all(is_data_view_wildcard_entry)
}

fn is_data_view_wildcard_entry(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 1 && object.contains_key("dataViewWildcard")
}

fn json_object_mut<'a>(value: &'a mut Value, label: &str) -> CliResult<&'a mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{label} must be a JSON object"))
            .with_hint("Run `validate --strict` before editing PBIR formatting.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })
}

fn solid_color_expression(color: &str) -> Value {
    json!({
        "solid": {
            "color": {
                "expr": {
                    "Literal": {
                        "Value": encode_text_literal(color)
                    }
                }
            }
        }
    })
}

fn encode_text_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn decode_text_literal(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        return trimmed[1..trimmed.len() - 1].replace("''", "'");
    }
    trimmed.to_string()
}

fn require_color_intent(options: &ColorOptions) -> CliResult<()> {
    if options.slot.is_none() || options.color.is_none() {
        return Err(CliError::invalid_args(
            "report visuals formatting set-color requires --slot <slot> and --color <hex>, or a slot-specific color flag",
        )
        .with_hint("Supported slots are title.fontColor and dataPoint.fill.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json",
        ));
    }
    Ok(())
}

fn parse_color_args(args: &[String]) -> CliResult<ColorOptions> {
    let mut options = ColorOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" => options.selector.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" => {
                let value = take_value(args, &mut i, "--visual")?;
                if value.starts_with("visual:") {
                    options.selector.handle = Some(value);
                } else {
                    options.selector.visual = Some(value);
                }
            }
            "--slot" => {
                set_slot(
                    &mut options.slot,
                    parse_color_slot(&take_value(args, &mut i, "--slot")?)?,
                )?;
            }
            "--color" | "--colour" => {
                options.color = Some(normalize_color(&take_value(args, &mut i, "--color")?)?);
            }
            "--title-font-color" | "--title-font-colour" => {
                set_slot_color(
                    &mut options,
                    ColorSlot::TitleFontColor,
                    &take_value(args, &mut i, "--title-font-color")?,
                )?;
            }
            "--data-point-fill" | "--dataPoint-fill" | "--fill-color" | "--fill-colour" => {
                set_slot_color(
                    &mut options,
                    ColorSlot::DataPointFill,
                    &take_value(args, &mut i, "--data-point-fill")?,
                )?;
            }
            "--include-raw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report visuals formatting set-color",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report visuals formatting set-color",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report visuals formatting set-color",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals formatting set-color flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals formatting set-color\"` for exact flags.")
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report visuals formatting set-color\"",
                ));
            }
        }
    }
    if options.slot.is_some() && options.color.is_none() {
        return Err(CliError::invalid_args("--slot requires --color <hex>")
            .with_hint("Pass --color '#123456' with the selected slot.")
            .with_suggested_command(
                "powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json",
            ));
    }
    Ok(options)
}

fn parse_color_slot(value: &str) -> CliResult<ColorSlot> {
    match value {
        "title.fontColor" | "title.color" | "titleFontColor" => Ok(ColorSlot::TitleFontColor),
        "dataPoint.fill" | "dataPoint.color" | "fill" | "dataPointFill" => {
            Ok(ColorSlot::DataPointFill)
        }
        other => Err(CliError::unsupported_feature(format!(
            "unsupported visual formatting color slot: {other}"
        ))
        .with_hint("Supported slots are title.fontColor and dataPoint.fill.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json",
        )),
    }
}

fn set_slot(current: &mut Option<ColorSlot>, slot: ColorSlot) -> CliResult<()> {
    if current.is_some_and(|current| current != slot) {
        return Err(CliError::invalid_args(
            "choose exactly one color slot per set-color command",
        )
        .with_hint("Run one command for title.fontColor and a separate command for dataPoint.fill.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json",
        ));
    }
    *current = Some(slot);
    Ok(())
}

fn set_slot_color(options: &mut ColorOptions, slot: ColorSlot, raw_color: &str) -> CliResult<()> {
    set_slot(&mut options.slot, slot)?;
    options.color = Some(normalize_color(raw_color)?);
    Ok(())
}

fn normalize_color(value: &str) -> CliResult<String> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(CliError::invalid_args(format!(
            "invalid color literal: {value}"
        ))
        .with_hint("Use #RRGGBB or #AARRGGBB hex colors.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting set-color --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json",
        ));
    }
    Ok(format!("#{}", hex.to_ascii_uppercase()))
}

fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass the PBIP project directory or the .pbip file explicitly with `--project`.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json"
        ))
    })
}

fn require_visual_selector(selector: &VisualSelector, command: &str) -> CliResult<()> {
    if selector.handle.is_some() || (selector.page.is_some() && selector.visual.is_some()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{command} requires --handle or --page plus --visual"
    ))
    .with_hint("Use `report visuals list` to get stable visual handles.")
    .with_suggested_command(format!(
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json"
    )))
}

fn visual_path<'a>(visual: &'a VisualRecord, command: &str) -> CliResult<&'a PathBuf> {
    visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual has no visual.json path in inspect output: {}",
            visual.handle
        ))
        .with_hint("Run `validate --strict` before editing formatting.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle {} --json",
            shell_arg(&visual.handle)
        ))
    })
}

fn target_project(
    source_resolved: &ResolvedProject,
    mode: MutationMode,
    out_dir: Option<&Path>,
) -> CliResult<ResolvedProject> {
    match (mode, out_dir) {
        (MutationMode::DryRun | MutationMode::InPlace, _) => Ok(source_resolved.clone()),
        (MutationMode::OutDir, Some(out_dir)) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)
        }
        (MutationMode::OutDir, None) => {
            Err(CliError::invalid_args("--out-dir requires a directory"))
        }
    }
}

fn require_mode(mode: Option<MutationMode>, command: &str) -> CliResult<MutationMode> {
    mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --dry-run, --in-place, or --out-dir <dir>"
        ))
        .with_hint("Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json"
        ))
    })
}

fn set_mode(
    current: &mut Option<MutationMode>,
    next: MutationMode,
    command: &str,
) -> CliResult<()> {
    if current.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one output mode: --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --slot title.fontColor --color '#123456' --dry-run --json"
        )));
    }
    *current = Some(next);
    Ok(())
}

fn mode_name(mode: MutationMode) -> &'static str {
    match mode {
        MutationMode::DryRun => "dry-run",
        MutationMode::InPlace => "in-place",
        MutationMode::OutDir => "out-dir",
    }
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals formatting set-color\"` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for \"report visuals formatting set-color\"")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
