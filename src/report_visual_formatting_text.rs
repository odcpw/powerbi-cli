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

#[derive(Debug, Default)]
struct TextOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    title: Option<String>,
    show_title: Option<bool>,
    alt_text: Option<String>,
    clear_alt_text: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

pub(crate) fn set_text_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_text_args(args)?;
    let source_project = required_project(
        options.project.clone(),
        "report visuals formatting set-text",
    )?;
    require_visual_selector(&options.selector, "report visuals formatting set-text")?;
    require_text_intent(&options)?;
    let mode = require_mode(options.mode, "report visuals formatting set-text")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, set_text_formatting)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals formatting set-text",
    )?;
    let visual_path = visual_path(visual, "report visuals formatting set-text")?;
    let mut visual_json = read_json_value(visual_path)?;
    let before = visual_text_state(&visual_json, options.include_raw);
    let pointers = apply_text_patch(&mut visual_json, &options)?;
    let after = visual_text_state(&visual_json, options.include_raw);

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

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.formatting.textMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "set-text",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": visual_detail(visual),
        "textPlan": {
            "strategy": "patch",
            "rawIncluded": options.include_raw,
            "requested": {
                "title": options.title,
                "showTitle": options.show_title,
                "altText": options.alt_text,
                "clearAltText": options.clear_alt_text,
                "autoShowTitle": options.title.is_some() && options.show_title.is_none()
            },
            "before": before,
            "after": after
        },
        "changes": [{
            "kind": "pbir.visual.textFormatting",
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

fn apply_text_patch(visual_json: &mut Value, options: &TextOptions) -> CliResult<Vec<String>> {
    let mut pointers = Vec::new();
    if options.title.is_some() || options.show_title.is_some() {
        patch_title_properties(visual_json, options, &mut pointers)?;
    }
    if let Some(title) = &options.title {
        update_placeholder_title_annotation(visual_json, title, &mut pointers)?;
    }
    if options.clear_alt_text {
        if let Some(properties) =
            existing_visual_container_object_properties(visual_json, "general")?
        {
            properties.remove("altText");
        }
        pointers.push("/visual/visualContainerObjects/general/0/properties/altText".to_string());
        if let Some(properties) = existing_visual_object_properties(visual_json, "general")?
            && properties.remove("altText").is_some()
        {
            pointers.push("/visual/objects/general/0/properties/altText".to_string());
        }
    }
    Ok(pointers)
}

pub(crate) fn patch_visible_title(visual_json: &mut Value, title: &str) -> CliResult<()> {
    let options = TextOptions {
        title: Some(title.to_string()),
        ..TextOptions::default()
    };
    patch_title_properties(visual_json, &options, &mut Vec::new())
}

fn patch_title_properties(
    visual_json: &mut Value,
    options: &TextOptions,
    pointers: &mut Vec<String>,
) -> CliResult<()> {
    let has_container_title = visual_json
        .pointer("/visual/visualContainerObjects/title")
        .is_some();
    let has_object_title = visual_json.pointer("/visual/objects/title").is_some();

    if has_container_title || !has_object_title {
        let properties = ensure_visual_container_object_properties(visual_json, "title")?;
        patch_one_title_properties(
            properties,
            options,
            "/visual/visualContainerObjects/title/0/properties",
            pointers,
        );
    }
    if has_object_title {
        let properties = ensure_visual_object_properties(visual_json, "title")?;
        patch_one_title_properties(
            properties,
            options,
            "/visual/objects/title/0/properties",
            pointers,
        );
    }
    Ok(())
}

fn patch_one_title_properties(
    properties: &mut Map<String, Value>,
    options: &TextOptions,
    pointer_prefix: &str,
    pointers: &mut Vec<String>,
) {
    if let Some(title) = &options.title {
        properties.insert("text".to_string(), literal_text_expression(title));
        pointers.push(format!("{pointer_prefix}/text/expr/Literal/Value"));
        if options.show_title.is_none() {
            properties.insert("show".to_string(), literal_bool_expression(true));
            pointers.push(format!("{pointer_prefix}/show/expr/Literal/Value"));
        }
    }
    if let Some(show_title) = options.show_title {
        properties.insert("show".to_string(), literal_bool_expression(show_title));
        let pointer = format!("{pointer_prefix}/show/expr/Literal/Value");
        if !pointers.iter().any(|item| item == &pointer) {
            pointers.push(pointer);
        }
    }
}

fn update_placeholder_title_annotation(
    visual_json: &mut Value,
    title: &str,
    pointers: &mut Vec<String>,
) -> CliResult<()> {
    let root = json_object_mut(visual_json, "visual.json root")?;
    let Some(annotations) = root.get_mut("annotations") else {
        return Ok(());
    };
    let annotations = annotations.as_array_mut().ok_or_else(|| {
        CliError::validation_failed("/annotations must be an array before set-text can patch it")
    })?;
    if let Some((index, annotation)) = annotations.iter_mut().enumerate().find(|(_, item)| {
        item.get("name").and_then(Value::as_str) == Some("powerbi-cli.placeholderTitle")
    }) {
        let annotation = json_object_mut(annotation, "title annotation")?;
        annotation.insert("value".to_string(), Value::String(title.to_string()));
        pointers.push(format!("/annotations/{index}/value"));
    }
    Ok(())
}

fn visual_text_state(visual_json: &Value, include_raw: bool) -> Value {
    let title_literal = visual_json
        .pointer("/visual/visualContainerObjects/title/0/properties/text/expr/Literal/Value")
        .or_else(|| {
            visual_json.pointer("/visual/objects/title/0/properties/text/expr/Literal/Value")
        })
        .and_then(Value::as_str);
    let show_literal = visual_json
        .pointer("/visual/visualContainerObjects/title/0/properties/show/expr/Literal/Value")
        .or_else(|| {
            visual_json.pointer("/visual/objects/title/0/properties/show/expr/Literal/Value")
        })
        .and_then(Value::as_str);
    let canonical_alt_literal = visual_json
        .pointer("/visual/visualContainerObjects/general/0/properties/altText/expr/Literal/Value")
        .and_then(Value::as_str);
    let legacy_alt_literal = visual_json
        .pointer("/visual/objects/general/0/properties/altText/expr/Literal/Value")
        .and_then(Value::as_str);
    let alt_literal = canonical_alt_literal.or(legacy_alt_literal);
    let mut state = json!({
        "title": title_literal.map(decode_text_literal),
        "showTitle": show_literal.and_then(decode_bool_literal),
        "altText": alt_literal.map(decode_text_literal),
        "altTextSource": if canonical_alt_literal.is_some() {
            "visualContainerObjects"
        } else if legacy_alt_literal.is_some() {
            "legacyVisualObjects"
        } else {
            "missing"
        }
    });
    if include_raw {
        state["raw"] = json!({
            "visualContainerTitle": visual_json.pointer("/visual/visualContainerObjects/title/0").cloned().unwrap_or(Value::Null),
            "visualObjectTitle": visual_json.pointer("/visual/objects/title/0").cloned().unwrap_or(Value::Null),
            "visualContainerGeneral": visual_json.pointer("/visual/visualContainerObjects/general/0").cloned().unwrap_or(Value::Null),
            "general": visual_json.pointer("/visual/objects/general/0").cloned().unwrap_or(Value::Null)
        });
    }
    state
}

fn ensure_visual_object_properties<'a>(
    visual_json: &'a mut Value,
    object_name: &str,
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
    ensure_object_properties(objects, object_name, "/visual/objects")
}

fn ensure_visual_container_object_properties<'a>(
    visual_json: &'a mut Value,
    object_name: &str,
) -> CliResult<&'a mut Map<String, Value>> {
    let root = json_object_mut(visual_json, "visual.json root")?;
    let visual = root
        .entry("visual".to_string())
        .or_insert_with(|| json!({}));
    let visual = json_object_mut(visual, "visual.json visual")?;
    let objects = visual
        .entry("visualContainerObjects".to_string())
        .or_insert_with(|| json!({}));
    let objects = json_object_mut(objects, "/visual/visualContainerObjects")?;
    ensure_object_properties(objects, object_name, "/visual/visualContainerObjects")
}

fn existing_visual_object_properties<'a>(
    visual_json: &'a mut Value,
    object_name: &str,
) -> CliResult<Option<&'a mut Map<String, Value>>> {
    let root = json_object_mut(visual_json, "visual.json root")?;
    let Some(visual) = root.get_mut("visual") else {
        return Ok(None);
    };
    let visual = json_object_mut(visual, "visual.json visual")?;
    let Some(objects) = visual.get_mut("objects") else {
        return Ok(None);
    };
    let objects = json_object_mut(objects, "/visual/objects")?;
    let Some(value) = objects.get_mut(object_name) else {
        return Ok(None);
    };
    object_properties(value, object_name, "/visual/objects").map(Some)
}

fn existing_visual_container_object_properties<'a>(
    visual_json: &'a mut Value,
    object_name: &str,
) -> CliResult<Option<&'a mut Map<String, Value>>> {
    let root = json_object_mut(visual_json, "visual.json root")?;
    let Some(visual) = root.get_mut("visual") else {
        return Ok(None);
    };
    let visual = json_object_mut(visual, "visual.json visual")?;
    let Some(objects) = visual.get_mut("visualContainerObjects") else {
        return Ok(None);
    };
    let objects = json_object_mut(objects, "/visual/visualContainerObjects")?;
    let Some(value) = objects.get_mut(object_name) else {
        return Ok(None);
    };
    object_properties(value, object_name, "/visual/visualContainerObjects").map(Some)
}

fn ensure_object_properties<'a>(
    objects: &'a mut Map<String, Value>,
    object_name: &str,
    parent_pointer: &str,
) -> CliResult<&'a mut Map<String, Value>> {
    let value = objects
        .entry(object_name.to_string())
        .or_insert_with(|| json!([{ "properties": {} }]));
    if let Some(cards) = value.as_array_mut()
        && cards.is_empty()
    {
        cards.push(json!({ "properties": {} }));
    }
    object_properties(value, object_name, parent_pointer)
}

fn object_properties<'a>(
    value: &'a mut Value,
    object_name: &str,
    parent_pointer: &str,
) -> CliResult<&'a mut Map<String, Value>> {
    let cards = value.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "{parent_pointer}/{object_name} must be an array before set-text can patch it"
        ))
        .with_hint("Use `report visuals formatting show --include-raw` to inspect this visual before editing raw PBIR.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --include-raw --json",
        )
    })?;
    if cards.is_empty() {
        return Err(CliError::validation_failed(format!(
            "{parent_pointer}/{object_name} has no formatting card to patch"
        )));
    }
    let card = json_object_mut(&mut cards[0], "formatting card")?;
    let properties = card
        .entry("properties".to_string())
        .or_insert_with(|| json!({}));
    json_object_mut(properties, "formatting properties")
}

fn json_object_mut<'a>(value: &'a mut Value, label: &str) -> CliResult<&'a mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{label} must be a JSON object"))
            .with_hint("Run `validate --strict` before editing PBIR formatting.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })
}

fn literal_text_expression(text: &str) -> Value {
    json!({ "expr": { "Literal": { "Value": encode_text_literal(text) } } })
}

fn literal_bool_expression(value: bool) -> Value {
    json!({ "expr": { "Literal": { "Value": if value { "true" } else { "false" } } } })
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

fn decode_bool_literal(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn require_text_intent(options: &TextOptions) -> CliResult<()> {
    if options.title.is_none()
        && options.show_title.is_none()
        && options.alt_text.is_none()
        && !options.clear_alt_text
    {
        return Err(CliError::invalid_args(
            "report visuals formatting set-text requires --title, --show-title, or --clear-alt-text",
        )
        .with_hint("Start with `--dry-run` and specify at least one text formatting field.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json",
        ));
    }
    if let Some(title) = &options.title {
        validate_nonempty_text(title, "--title")?;
    }
    if let Some(alt_text) = &options.alt_text {
        validate_nonempty_text(alt_text, "--alt-text")?;
        return Err(CliError::unsupported_feature(
            "--alt-text authoring is unavailable because Microsoft powerbi-report-authoring-cli v0.1.4 rejects every proven general.altText placement as PBIR_FORMATTING_PROP_UNKNOWN",
        )
        .with_hint(
            "Use --clear-alt-text to remove rejected metadata. Do not author a replacement until Microsoft exposes a validator-supported PBIR location.",
        )
        .with_suggested_command(
            "powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> --handle <visual-handle> --clear-alt-text --dry-run --json",
        ));
    }
    Ok(())
}

fn validate_nonempty_text(value: &str, flag: &str) -> CliResult<()> {
    if value.trim().is_empty() {
        return Err(CliError::invalid_args(format!("{flag} must not be empty"))
            .with_hint("Pass visible text, or use an explicit clear/hide flag where available.")
            .with_suggested_command(
                "powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json",
            ));
    }
    Ok(())
}

fn parse_text_args(args: &[String]) -> CliResult<TextOptions> {
    let mut options = TextOptions::default();
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
            "--title" => options.title = Some(take_value(args, &mut i, "--title")?),
            "--show-title" | "--title-visible" => {
                let value = take_value(args, &mut i, "--show-title")?;
                options.show_title = Some(parse_bool_flag(&value, "--show-title")?);
            }
            "--alt-text" | "--altText" => {
                options.alt_text = Some(take_value(args, &mut i, "--alt-text")?);
            }
            "--clear-alt-text" | "--clear-altText" => {
                options.clear_alt_text = true;
                i += 1;
            }
            "--include-raw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report visuals formatting set-text",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report visuals formatting set-text",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report visuals formatting set-text",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals formatting set-text flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals formatting set-text\"` for exact flags.")
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report visuals formatting set-text\"",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_bool_flag(value: &str, flag: &str) -> CliResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(CliError::invalid_args(format!(
            "{flag} expects true or false, got {value}"
        ))
        .with_hint("Use `--show-title true` to show the title or `--show-title false` to hide it.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting set-text --project <project-dir-or.pbip> --handle <visual-handle> --show-title true --dry-run --json",
        )),
    }
}

fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass the PBIP project directory or the .pbip file explicitly with `--project`.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json"
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
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json"
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
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json"
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
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --title <text> --dry-run --json"
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
            .with_hint("Run `powerbi-cli --json capabilities --for report` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for report")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
