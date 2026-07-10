use crate::cli_support::{
    MutationMode, mode_name, require_mode_with_contract, required_project, set_mode_with_contract,
    shell_arg, take_report_value as take_value, target_project,
};
use crate::pbir::{
    PageRecord, PageSelector, VisualRecord, VisualSelector, find_page, find_visual,
    load_report_snapshot, visual_detail,
};
use crate::project_io::write_json_pretty;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_WIDTH: f64 = 320.0;
const DEFAULT_HEIGHT: f64 = 180.0;
const CLONE_DRY_RUN_COMMAND: &str = "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json";
const REQUIRE_MODE_HINT: &str =
    "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.";
const SET_MODE_HINT: &str =
    "Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.";

#[derive(Debug, Default)]
struct CloneOptions {
    project: Option<PathBuf>,
    source: VisualSelector,
    ambiguous_page: Option<String>,
    target_page: Option<String>,
    name: Option<String>,
    title: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    z: Option<u64>,
    tab_order: Option<u64>,
    allow_outside_page: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn clone_visual(args: &[String]) -> CliResult<Value> {
    let mut options = parse_clone_args(args)?;
    normalize_page_alias(&mut options)?;
    let source_project = required_project(options.project.clone(), "report visuals clone")?;
    require_visual_selector(&options.source, "report visuals clone")?;
    let mode = require_mode_with_contract(
        options.mode,
        "report visuals clone",
        REQUIRE_MODE_HINT,
        CLONE_DRY_RUN_COMMAND,
    )?;

    let source_resolved = resolve_project(&source_project)?;
    let source_snapshot = load_report_snapshot(&source_resolved)?;
    let source_visual = find_visual(
        &source_snapshot.pages,
        &options.source,
        "report visuals clone",
    )?;
    let source_visual_path = visual_path(source_visual, "report visuals clone")?;
    let source_visual_dir = visual_dir(source_visual_path)?;
    validate_simple_visual_dir(source_visual_dir)?;

    crate::cli_support::preflight_out_dir(args, clone_visual)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let target_snapshot = load_report_snapshot(&target_resolved)?;
    let source_visual = find_visual(
        &target_snapshot.pages,
        &options.source,
        "report visuals clone",
    )?;
    let source_visual_path = visual_path(source_visual, "report visuals clone")?;
    let source_visual_dir = visual_dir(source_visual_path)?;
    validate_simple_visual_dir(source_visual_dir)?;
    let mut cloned_json = read_json_value(source_visual_path)?;

    let target_page_selector = target_page_selector(source_visual, options.target_page.as_deref());
    let target_page = find_page(
        &target_snapshot.pages,
        &target_page_selector,
        "report visuals clone",
    )?;
    let title = options
        .title
        .clone()
        .unwrap_or_else(|| format!("{} Copy", source_visual.title));
    validate_nonempty_text(&title, "--title")?;
    let new_name = match options.name.as_deref() {
        Some(name) => validate_new_visual_name(name, target_page)?,
        None => generated_clone_name(source_visual, &title, target_page),
    };
    let name_generated = options.name.is_none();
    let target_visual_dir = page_visuals_dir(target_page)?.join(&new_name);
    ensure_child_path(&target_visual_dir, &page_visuals_dir(target_page)?)?;
    if target_visual_dir.exists() {
        return Err(CliError::invalid_args(format!(
            "target visual directory already exists: {}",
            target_visual_dir.display()
        ))
        .with_hint("Choose a unique --name or omit it so powerbi-cli can generate one.")
        .with_suggested_command(
            "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
        ));
    }

    let before_position = source_visual.position.clone();
    let after_position = cloned_position(
        &before_position,
        target_page,
        &options,
        target_page.visuals.len() as u64,
    )?;
    validate_position_bounds(
        &after_position,
        target_page.width.as_f64(),
        target_page.height.as_f64(),
        options.allow_outside_page,
    )?;
    patch_cloned_visual_json(
        &mut cloned_json,
        &new_name,
        &title,
        &after_position,
        source_visual,
    )?;

    let target_visual_path = target_visual_dir.join("visual.json");
    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        fs::create_dir_all(&target_visual_dir).map_err(|err| {
            CliError::unexpected(format!(
                "create visual dir {}: {err}",
                target_visual_dir.display()
            ))
        })?;
        write_json_pretty(&target_visual_path, &cloned_json)?;
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
    let target_handle = format!("visual:{}:{new_name}", target_page.name);
    let project_arg = command_arg(&target_resolved.project_dir);
    let readback = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&target_handle)
    );
    let slicer_readback = is_slicer_type(&source_visual.visual_type).then(|| {
        format!(
            "powerbi-cli report slicers show --project {} --handle {} --json",
            project_arg,
            shell_arg(&format!("slicer:{}:{new_name}", target_page.name))
        )
    });
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
    let mut next = vec![
        readback.clone(),
        wireframe.clone(),
        inspect.clone(),
        validate.clone(),
    ];
    if let Some(command) = &slicer_readback {
        next.insert(1, command.clone());
    }

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.cloneMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "clone",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "source": visual_detail(source_visual),
        "target": {
            "handle": target_handle,
            "name": new_name,
            "title": title,
            "visualType": source_visual.visual_type,
            "page": {
                "handle": target_page.handle,
                "name": target_page.name,
                "displayName": target_page.display_name,
                "ordinal": target_page.ordinal
            },
            "path": canonical_display(&target_visual_path),
            "position": after_position,
            "bindingCount": source_visual.bindings.len(),
            "bindings": source_visual.bindings,
            "nameGenerated": name_generated,
            "titleGenerated": options.title.is_none()
        },
        "clonePlan": {
            "strategy": "copy-simple-visual-json",
            "sourcePath": canonical_display(source_visual_path),
            "targetPath": canonical_display(&target_visual_path),
            "copiedSidecars": false,
            "nameGenerated": name_generated,
            "position": {
                "before": before_position,
                "after": after_position
            },
            "note": "This first clone slice copies simple visual containers that contain only visual.json. It preserves visual type, bindings, formatting, filters, and raw PBIR objects already in visual.json, then patches only name, position, and powerbi-cli clone annotations."
        },
        "changes": [{
            "kind": "pbir.visual",
            "action": "clone",
            "path": canonical_display(&target_visual_path),
            "before": Value::Null,
            "after": cloned_json
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
        "slicerReadbackCommand": slicer_readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": next
    }))
}

fn patch_cloned_visual_json(
    visual_json: &mut Value,
    name: &str,
    title: &str,
    position: &Value,
    source: &VisualRecord,
) -> CliResult<()> {
    let root = visual_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed("source visual.json root must be a JSON object")
    })?;
    root.insert("name".to_string(), Value::String(name.to_string()));
    root.insert("position".to_string(), position.clone());
    upsert_annotation(
        visual_json,
        "powerbi-cli.placeholderTitle",
        Value::String(title.to_string()),
    );
    upsert_annotation(
        visual_json,
        "powerbi-cli.clonedFromVisual",
        Value::String(source.handle.clone()),
    );
    upsert_annotation(
        visual_json,
        "powerbi-cli.cloneSourceName",
        Value::String(source.name.clone()),
    );
    Ok(())
}

fn upsert_annotation(visual_json: &mut Value, name: &str, value: Value) {
    let annotation = json!({
        "name": name,
        "value": value
    });
    if !visual_json["annotations"].is_array() {
        visual_json["annotations"] = Value::Array(Vec::new());
    }
    let annotations = visual_json["annotations"]
        .as_array_mut()
        .expect("annotations was just made an array");
    if let Some(existing) = annotations
        .iter_mut()
        .find(|item| item["name"].as_str() == Some(name))
    {
        *existing = annotation;
    } else {
        annotations.push(annotation);
    }
}

fn cloned_position(
    source: &Value,
    page: &PageRecord,
    options: &CloneOptions,
    next_index: u64,
) -> CliResult<Value> {
    let width = options
        .width
        .or_else(|| source["width"].as_f64())
        .unwrap_or(DEFAULT_WIDTH);
    let height = options
        .height
        .or_else(|| source["height"].as_f64())
        .unwrap_or(DEFAULT_HEIGHT);
    validate_positive_number(width, "--width")?;
    validate_positive_number(height, "--height")?;
    let default_x = default_offset_position(
        source["x"].as_f64().unwrap_or(40.0),
        width,
        page.width.as_f64(),
    );
    let default_y = default_offset_position(
        source["y"].as_f64().unwrap_or(40.0),
        height,
        page.height.as_f64(),
    );
    Ok(json!({
        "x": options.x.unwrap_or(default_x),
        "y": options.y.unwrap_or(default_y),
        "z": options.z.unwrap_or(next_index),
        "height": height,
        "width": width,
        "tabOrder": options.tab_order.unwrap_or(next_index)
    }))
}

fn default_offset_position(start: f64, span: f64, page_span: Option<f64>) -> f64 {
    let shifted = start + 40.0;
    if let Some(page_span) = page_span
        && shifted + span > page_span
    {
        return 40.0;
    }
    shifted
}

fn validate_simple_visual_dir(visual_dir: &Path) -> CliResult<()> {
    let entries = fs::read_dir(visual_dir)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", visual_dir.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", visual_dir.display())))?;
    let unsupported = entries
        .iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_none_or(|name| name != "visual.json")
        })
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if unsupported.is_empty() {
        return Ok(());
    }
    Err(CliError::unsupported_feature(format!(
        "report visuals clone currently supports simple visual containers only; unsupported sidecar entries in {}: {}",
        visual_dir.display(),
        unsupported.join(", ")
    ))
    .with_hint("Use a template visual whose container directory contains only visual.json, or extract/apply formatting separately.")
    .with_suggested_command(
        "powerbi-cli report visuals formatting extract --project <project-dir-or.pbip> --handle <visual-handle> --out visual-formatting-bundle.json --json",
    ))
}

fn generated_clone_name(source: &VisualRecord, title: &str, page: &PageRecord) -> String {
    let stem = pascal_identifier(title).unwrap_or_else(|| format!("{}Copy", source.name));
    let mut base = if stem.starts_with("VisualContainer") {
        stem
    } else {
        format!("VisualContainer{stem}")
    };
    if page.visuals.iter().any(|visual| visual.name == base) {
        base = format!("{}Copy", source.name);
    }
    if !page.visuals.iter().any(|visual| visual.name == base) {
        return base;
    }
    for index in 2..1000 {
        let candidate = format!("{base}{index}");
        if !page.visuals.iter().any(|visual| visual.name == candidate) {
            return candidate;
        }
    }
    format!("VisualContainer{}", page.visuals.len() + 1)
}

fn validate_new_visual_name(name: &str, page: &PageRecord) -> CliResult<String> {
    validate_visual_name(name)?;
    if page.visuals.iter().any(|visual| visual.name == name) {
        return Err(CliError::invalid_args(format!(
            "visual already exists on page {}: {name}",
            page.handle
        ))
        .with_hint("Choose a unique internal --name or omit it so powerbi-cli can generate one.")
        .with_suggested_command(
            "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
        ));
    }
    Ok(name.to_string())
}

fn validate_visual_name(name: &str) -> CliResult<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch == ':' || ch.is_control())
    {
        return Err(CliError::invalid_args(format!("unsafe visual name: {name}"))
            .with_hint("Use a simple internal visual name without path separators.")
            .with_suggested_command(
                "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
            ));
    }
    Ok(())
}

fn pascal_identifier(value: &str) -> Option<String> {
    let mut output = String::new();
    for part in value.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if part.is_empty() {
            continue;
        }
        let mut chars = part.chars();
        let first = chars.next()?.to_ascii_uppercase();
        output.push(first);
        output.extend(chars.map(|ch| ch.to_ascii_lowercase()));
    }
    (!output.is_empty()).then_some(output)
}

fn target_page_selector(source: &VisualRecord, page: Option<&str>) -> PageSelector {
    match page {
        Some(page) if page.starts_with("page:") => PageSelector {
            handle: Some(page.to_string()),
            name: None,
        },
        Some(page) => PageSelector {
            handle: None,
            name: Some(page.to_string()),
        },
        None => PageSelector {
            handle: Some(source.page_handle.clone()),
            name: None,
        },
    }
}

fn visual_path<'a>(visual: &'a VisualRecord, command: &str) -> CliResult<&'a PathBuf> {
    visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual has no visual.json path in inspect output: {}",
            visual.handle
        ))
        .with_hint("Run `validate --strict` before cloning visuals.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle {} --json",
            shell_arg(&visual.handle)
        ))
    })
}

fn visual_dir(visual_path: &Path) -> CliResult<&Path> {
    visual_path.parent().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual path has no parent: {}",
            visual_path.display()
        ))
    })
}

fn page_visuals_dir(page: &PageRecord) -> CliResult<PathBuf> {
    let page_json = page.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "page has no path in inspect output: {}",
            page.handle
        ))
    })?;
    let page_dir = page_json.parent().ok_or_else(|| {
        CliError::validation_failed(format!("page path has no parent: {}", page_json.display()))
    })?;
    Ok(page_dir.join("visuals"))
}

fn validate_position_bounds(
    position: &Value,
    page_width: Option<f64>,
    page_height: Option<f64>,
    allow_outside_page: bool,
) -> CliResult<()> {
    let x = position["x"].as_f64().unwrap_or_default();
    let y = position["y"].as_f64().unwrap_or_default();
    let width = position["width"].as_f64().unwrap_or_default();
    let height = position["height"].as_f64().unwrap_or_default();
    if x < 0.0 || y < 0.0 {
        return Err(CliError::invalid_args(
            "visual position x/y must be nonnegative",
        )
        .with_hint("Use --allow-outside-page only for page overflow, not negative coordinates.")
        .with_suggested_command(
            "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --x 0 --y 0 --dry-run --json",
        ));
    }
    if width <= 0.0 || height <= 0.0 {
        return Err(CliError::invalid_args(
            "visual position width/height must be positive",
        )
        .with_hint("Pass positive --width and --height values.")
        .with_suggested_command(
            "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --width 320 --height 180 --dry-run --json",
        ));
    }
    if !allow_outside_page
        && let (Some(page_width), Some(page_height)) = (page_width, page_height)
        && (x + width > page_width || y + height > page_height)
    {
        return Err(CliError::invalid_args(
            "visual position would extend outside page bounds",
        )
        .with_hint("Keep the cloned visual inside the page or pass --allow-outside-page deliberately.")
        .with_suggested_command(
            "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --allow-outside-page --dry-run --json",
        ));
    }
    Ok(())
}

fn ensure_child_path(path: &Path, parent: &Path) -> CliResult<()> {
    let parent_abs = if parent.exists() {
        fs::canonicalize(parent)
            .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", parent.display())))?
    } else {
        parent.to_path_buf()
    };
    let path_abs = if path.exists() {
        fs::canonicalize(path)
            .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?
    } else {
        parent_abs.join(path.file_name().unwrap_or(path.as_os_str()))
    };
    if path_abs.starts_with(parent_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to write visual outside page visuals directory: {}",
        path.display()
    )))
}

fn normalize_page_alias(options: &mut CloneOptions) -> CliResult<()> {
    if options.source.handle.is_some() && options.source.visual.is_some() {
        return Err(CliError::invalid_args(
            "report visuals clone accepts either --handle or --from-page/--visual, not both",
        )
        .with_hint("Use the exact source visual handle when available.")
        .with_suggested_command(
            "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
        ));
    }
    if let Some(page) = options.ambiguous_page.take() {
        if options.source.handle.is_some() && options.target_page.is_none() {
            options.target_page = Some(page);
        } else if options.source.visual.is_some() && options.source.page.is_none() {
            options.source.page = Some(page);
        } else if options.target_page.is_none() {
            options.target_page = Some(page);
        } else {
            return Err(CliError::invalid_args(
                "--page is ambiguous for report visuals clone",
            )
            .with_hint("Use --from-page for the source selector and --target-page for the destination page.")
            .with_suggested_command(
                "powerbi-cli report visuals clone --project <project-dir-or.pbip> --from-page <source-page> --visual <source-visual> --target-page <target-page> --dry-run --json",
            ));
        }
    }
    Ok(())
}

fn parse_clone_args(args: &[String]) -> CliResult<CloneOptions> {
    let mut options = CloneOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" | "--source" | "--source-handle" => {
                options.source.handle = Some(take_value(args, &mut i, "--handle")?);
            }
            "--from-page" | "--source-page" => {
                options.source.page = Some(take_value(args, &mut i, "--from-page")?);
            }
            "--page" => options.ambiguous_page = Some(take_value(args, &mut i, "--page")?),
            "--target-page" | "--to-page" => {
                options.target_page = Some(take_value(args, &mut i, "--target-page")?);
            }
            "--visual" | "--source-visual" => {
                let value = take_value(args, &mut i, "--visual")?;
                if value.starts_with("visual:") {
                    options.source.handle = Some(value);
                } else {
                    options.source.visual = Some(value);
                }
            }
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--title" => options.title = Some(take_value(args, &mut i, "--title")?),
            "--x" => options.x = Some(take_f64(args, &mut i, "--x")?),
            "--y" => options.y = Some(take_f64(args, &mut i, "--y")?),
            "--width" => options.width = Some(take_f64(args, &mut i, "--width")?),
            "--height" => options.height = Some(take_f64(args, &mut i, "--height")?),
            "--z" => options.z = Some(take_u64(args, &mut i, "--z")?),
            "--tab-order" | "--tabOrder" => {
                options.tab_order = Some(take_u64(args, &mut i, "--tab-order")?);
            }
            "--allow-outside-page" => {
                options.allow_outside_page = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::DryRun,
                    SET_MODE_HINT,
                    CLONE_DRY_RUN_COMMAND,
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::InPlace,
                    SET_MODE_HINT,
                    CLONE_DRY_RUN_COMMAND,
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::OutDir,
                    SET_MODE_HINT,
                    CLONE_DRY_RUN_COMMAND,
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals clone flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals clone\"` for exact flags.")
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report visuals clone\"",
                ));
            }
        }
    }
    Ok(options)
}

fn require_visual_selector(selector: &VisualSelector, command: &str) -> CliResult<()> {
    if selector.handle.is_some() || (selector.page.is_some() && selector.visual.is_some()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{command} requires --handle or --from-page plus --visual"
    ))
    .with_hint("Use `report visuals list` to get stable source visual handles.")
    .with_suggested_command(format!(
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json"
    )))
}

fn take_f64(args: &[String], index: &mut usize, flag: &str) -> CliResult<f64> {
    let value = take_value(args, index, flag)?;
    let parsed = value
        .parse::<f64>()
        .map_err(|_| CliError::invalid_args(format!("{flag} must be a number")))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(CliError::invalid_args(format!(
            "{flag} must be a finite number"
        )))
    }
}

fn take_u64(args: &[String], index: &mut usize, flag: &str) -> CliResult<u64> {
    let value = take_value(args, index, flag)?;
    value
        .parse::<u64>()
        .map_err(|_| CliError::invalid_args(format!("{flag} must be a nonnegative integer")))
}

fn validate_nonempty_text(value: &str, flag: &str) -> CliResult<()> {
    if !value.trim().is_empty() && !value.chars().any(char::is_control) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be nonempty text"
    )))
}

fn validate_positive_number(value: f64, flag: &str) -> CliResult<()> {
    if value.is_finite() && value > 0.0 {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be a positive finite number"
    )))
}

fn is_slicer_type(visual_type: &str) -> bool {
    visual_type.to_ascii_lowercase().contains("slicer")
}
