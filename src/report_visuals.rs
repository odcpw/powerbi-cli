use crate::cli_support::{
    MutationMode, mode_name, require_mode_with_contract, required_project,
    set_report_visual_mode as set_mode, shell_arg, take_report_value as take_value, target_project,
};
use crate::pbir::{
    VisualSelector, find_visual, load_report_snapshot, visual_detail, visual_list_item,
    visuals_for_page,
};
use crate::pbir_bindings::{
    VisualBindingInput, binding_summary, parse_binding_spec, parse_bindings_json_file,
    parse_bindings_json_text, resolve_visual_bindings, set_binding_status_annotation,
    visual_query_json,
};
use crate::project_io::write_json_atomic;
use crate::report_visual_clone::clone_visual;
use crate::report_visual_delete::delete_visual;
use crate::report_visual_formatting::formatting_command;
use crate::report_visual_mutations::{add_visual, validate_binding_cardinality};
use crate::tmdl::load_table_documents;
use crate::visual_catalog::visual_catalog_command;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    read_json_value, resolve_project, validate_project,
};
use serde_json::{Number, Value, json};
use std::path::PathBuf;

pub(crate) fn visuals_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report visuals requires a subcommand: list, show, catalog, formatting, add, clone, delete, set-position, set-bindings",
        )
        .with_hint("Run `powerbi-cli report visuals list --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli report visuals list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" => list_visuals(rest),
        "show" => show_visual(rest),
        "catalog" | "types" | "visual-types" => visual_catalog_command(rest),
        "formatting" | "format" => formatting_command(rest),
        "add" | "create" => add_visual(rest),
        "clone" | "duplicate" | "copy" => clone_visual(rest),
        "delete" | "remove" => delete_visual(rest),
        "set-position" | "setPosition" => set_position(rest),
        "set-bindings" | "setBindings" | "bind" => set_bindings(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown report visuals command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals\"` for supported visual commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report visuals\"")),
    }
}

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
    page: Option<String>,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
}

#[derive(Debug, Default)]
struct PositionPatch {
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    z: Option<u64>,
    tab_order: Option<u64>,
}

#[derive(Debug, Default)]
struct PositionOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    patch: PositionPatch,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    allow_outside_page: bool,
}

#[derive(Debug, Default)]
struct BindingOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    bindings: Vec<VisualBindingInput>,
    bindings_file: Option<PathBuf>,
    clear_bindings: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

fn list_visuals(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "report visuals list")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visuals = visuals_for_page(&snapshot.pages, options.page.as_deref())?
        .into_iter()
        .map(visual_list_item)
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.report.visuals.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "filter": {
            "page": options.page
        },
        "counts": {
            "visuals": visuals.len(),
            "boundVisuals": visuals.iter().filter(|visual| visual["bindingCount"].as_u64().unwrap_or_default() > 0).count()
        },
        "visuals": visuals,
        "next": [
            format!("powerbi-cli report visuals show --project {} --handle <visual-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals add --project {} --page <page-handle> --visual-type card --title <title> --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals clone --project {} --handle <visual-handle> --dry-run --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals delete --project {} --handle <visual-handle> --dry-run --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals set-position --project {} --handle <visual-handle> --x 40 --y 40 --dry-run --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

fn show_visual(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "report visuals show")?;
    require_visual_selector(&options.selector, "report visuals show")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visual = find_visual(&snapshot.pages, &options.selector, "report visuals show")?;
    Ok(json!({
        "schema": "powerbi-cli.report.visuals.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "visual": visual_detail(visual),
        "next": [
            format!("powerbi-cli report visuals set-position --project {} --handle {} --x 40 --y 40 --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&visual.handle)),
            format!("powerbi-cli report visuals delete --project {} --handle {} --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&visual.handle)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

fn set_position(args: &[String]) -> CliResult<Value> {
    let options = parse_position_args(args)?;
    let source_project = required_project(options.project.clone(), "report visuals set-position")?;
    require_visual_selector(&options.selector, "report visuals set-position")?;
    require_position_patch(&options.patch)?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = require_mode_with_contract(
        options.mode,
        "report visuals set-position",
        "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.",
        "powerbi-cli report visuals set-position --project <project-dir-or.pbip> --handle <visual-handle> --x 40 --y 40 --dry-run --json",
    )?;

    crate::cli_support::preflight_out_dir(args, set_position)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;

    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals set-position",
    )?;
    let visual_path = visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual has no path in inspect output: {}",
            visual.handle
        ))
    })?;
    let mut visual_json = read_json_value(visual_path)?;
    let before = visual_json["position"].clone();
    let page_width = snapshot.pages[visual.page_ordinal].width.as_f64();
    let page_height = snapshot.pages[visual.page_ordinal].height.as_f64();
    let after = patched_position(&before, &options.patch)?;
    validate_position_bounds(
        &after,
        page_width,
        page_height,
        options.allow_outside_page,
        "report visuals set-position",
    )?;
    visual_json["position"] = after.clone();

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
        "schema": "powerbi-cli.report.visuals.positionMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "set-position",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": visual_detail(visual),
        "changes": [{
            "kind": "pbir.visual.position",
            "action": "set-position",
            "path": canonical_display(visual_path),
            "fields": changed_fields(&options.patch),
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
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, wireframe, inspect, validate]
    }))
}

fn set_bindings(args: &[String]) -> CliResult<Value> {
    let options = parse_binding_args(args)?;
    let source_project = required_project(options.project.clone(), "report visuals set-bindings")?;
    require_visual_selector(&options.selector, "report visuals set-bindings")?;
    require_binding_intent(&options)?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = require_mode_with_contract(
        options.mode,
        "report visuals set-bindings",
        "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.",
        "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
    )?;

    crate::cli_support::preflight_out_dir(args, set_bindings)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;

    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals set-bindings",
    )?;
    let visual_path = visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual has no path in inspect output: {}",
            visual.handle
        ))
    })?;
    let mut visual_json = read_json_value(visual_path)?;
    let before = visual_json["visual"]["query"].clone();
    let before_bindings = visual.bindings.clone();

    let after_bindings = if options.clear_bindings {
        remove_visual_query(&mut visual_json)?;
        visual_json["howCreated"] = Value::String("Default".to_string());
        set_binding_status_annotation(&mut visual_json, "unbound");
        Vec::new()
    } else {
        let docs = load_table_documents(&target_resolved)?;
        let resolved_bindings =
            resolve_visual_bindings(&docs, &visual.visual_type, &options.bindings)?;
        validate_binding_cardinality(&visual.visual_type, &resolved_bindings)?;
        let query = visual_query_json(&visual.visual_type, &resolved_bindings);
        set_visual_query(&mut visual_json, query)?;
        visual_json["howCreated"] = Value::String("DraggedToFieldWell".to_string());
        set_binding_status_annotation(&mut visual_json, "bound");
        resolved_bindings
            .iter()
            .map(binding_summary)
            .collect::<Vec<_>>()
    };
    let after = visual_json["visual"]["query"].clone();

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
        "schema": "powerbi-cli.report.visuals.bindingMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": if options.clear_bindings { "clear-bindings" } else { "set-bindings" },
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": visual_detail(visual),
        "bindingPlan": {
            "clear": options.clear_bindings,
            "bindingsFile": options.bindings_file.as_ref().map(|path| canonical_display(path)),
            "before": before_bindings,
            "after": after_bindings
        },
        "changes": [{
            "kind": "pbir.visual.bindings",
            "action": if options.clear_bindings { "clear-bindings" } else { "set-bindings" },
            "path": canonical_display(visual_path),
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
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, wireframe, inspect, validate]
    }))
}

fn parse_list_args(args: &[String]) -> CliResult<ListOptions> {
    let mut options = ListOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report visuals list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report visuals list --project <project-dir-or.pbip> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_show_args(args: &[String]) -> CliResult<ShowOptions> {
    let mut options = ShowOptions::default();
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
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report visuals show --project <project-dir-or.pbip> --handle <visual-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report visuals show --project <project-dir-or.pbip> --handle <visual-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_position_args(args: &[String]) -> CliResult<PositionOptions> {
    let mut options = PositionOptions::default();
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
            "--x" => options.patch.x = Some(take_f64(args, &mut i, "--x")?),
            "--y" => options.patch.y = Some(take_f64(args, &mut i, "--y")?),
            "--width" => options.patch.width = Some(take_f64(args, &mut i, "--width")?),
            "--height" => options.patch.height = Some(take_f64(args, &mut i, "--height")?),
            "--z" => options.patch.z = Some(take_u64(args, &mut i, "--z")?),
            "--tab-order" | "--tabOrder" => {
                options.patch.tab_order = Some(take_u64(args, &mut i, "--tab-order")?);
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out_dir = Some(out_dir);
            }
            "--allow-outside-page" => {
                options.allow_outside_page = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals set-position flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals\"` for exact flags.")
                .with_suggested_command("powerbi-cli --json capabilities --for \"report visuals\""));
            }
        }
    }
    Ok(options)
}

fn parse_binding_args(args: &[String]) -> CliResult<BindingOptions> {
    let mut options = BindingOptions::default();
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
            "--binding" | "--bind" => {
                let value = take_value(args, &mut i, "--binding")?;
                options.bindings.push(parse_binding_spec(&value)?);
            }
            "--bindings-json" => {
                let value = take_value(args, &mut i, "--bindings-json")?;
                options.bindings.extend(parse_bindings_json_text(&value)?);
            }
            "--bindings-file" => {
                let path = PathBuf::from(take_value(args, &mut i, "--bindings-file")?);
                options.bindings.extend(parse_bindings_json_file(&path)?);
                options.bindings_file = Some(path);
            }
            "--clear-bindings" | "--clear" => {
                options.clear_bindings = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals set-bindings flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals\"` for exact flags.")
                .with_suggested_command("powerbi-cli --json capabilities --for \"report visuals\""));
            }
        }
    }
    Ok(options)
}

fn require_binding_intent(options: &BindingOptions) -> CliResult<()> {
    if options.clear_bindings && !options.bindings.is_empty() {
        return Err(CliError::invalid_args(
            "choose either --clear-bindings or binding inputs, not both",
        )
        .with_hint("Run one dry-run to clear bindings, or one dry-run to replace them.")
        .with_suggested_command(
            "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --clear-bindings --dry-run --json",
        ));
    }
    if options.clear_bindings || !options.bindings.is_empty() {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "report visuals set-bindings requires --binding, --bindings-json, --bindings-file, or --clear-bindings",
    )
    .with_hint("Use structured JSON for reliable agent-generated bindings.")
    .with_suggested_command(
        "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --bindings-json '[{\"role\":\"Values\",\"table\":\"FactSales\",\"measure\":\"Total Revenue\"}]' --dry-run --json",
    ))
}

fn set_visual_query(visual_json: &mut Value, query: Value) -> CliResult<()> {
    let visual_object = visual_json["visual"].as_object_mut().ok_or_else(|| {
        CliError::validation_failed("visual.json has no visual object")
            .with_hint("Run `validate --strict` before mutating this report.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })?;
    visual_object.insert("query".to_string(), query);
    Ok(())
}

fn remove_visual_query(visual_json: &mut Value) -> CliResult<()> {
    let visual_object = visual_json["visual"].as_object_mut().ok_or_else(|| {
        CliError::validation_failed("visual.json has no visual object")
            .with_hint("Run `validate --strict` before mutating this report.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })?;
    visual_object.remove("query");
    Ok(())
}

fn patched_position(before: &Value, patch: &PositionPatch) -> CliResult<Value> {
    let mut after = before.as_object().cloned().unwrap_or_default();
    if let Some(value) = patch.x {
        after.insert("x".to_string(), number_value(value, "--x")?);
    }
    if let Some(value) = patch.y {
        after.insert("y".to_string(), number_value(value, "--y")?);
    }
    if let Some(value) = patch.width {
        after.insert("width".to_string(), number_value(value, "--width")?);
    }
    if let Some(value) = patch.height {
        after.insert("height".to_string(), number_value(value, "--height")?);
    }
    if let Some(value) = patch.z {
        after.insert("z".to_string(), Value::Number(Number::from(value)));
    }
    if let Some(value) = patch.tab_order {
        after.insert("tabOrder".to_string(), Value::Number(Number::from(value)));
    }
    Ok(Value::Object(after))
}

fn validate_position_bounds(
    position: &Value,
    page_width: Option<f64>,
    page_height: Option<f64>,
    allow_outside_page: bool,
    command: &str,
) -> CliResult<()> {
    let x = position_number(position, "x").unwrap_or(0.0);
    let y = position_number(position, "y").unwrap_or(0.0);
    let width = position_number(position, "width").unwrap_or(0.0);
    let height = position_number(position, "height").unwrap_or(0.0);
    if x < 0.0 || y < 0.0 {
        return Err(CliError::invalid_args(
            "visual position x/y must be nonnegative",
        )
        .with_hint("Use --allow-outside-page only for page overflow, not negative coordinates.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --x 0 --y 0 --dry-run --json"
        )));
    }
    if width <= 0.0 || height <= 0.0 {
        return Err(CliError::invalid_args(
            "visual position width/height must be positive",
        )
        .with_hint("Pass positive --width and --height values.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --width 320 --height 180 --dry-run --json"
        )));
    }
    if !allow_outside_page
        && let (Some(page_width), Some(page_height)) = (page_width, page_height)
        && (x + width > page_width || y + height > page_height)
    {
        return Err(CliError::invalid_args(
            "visual position would extend outside page bounds",
        )
        .with_hint("Keep the visual inside the page or pass --allow-outside-page deliberately.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --allow-outside-page --dry-run --json"
        )));
    }
    Ok(())
}

fn changed_fields(patch: &PositionPatch) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if patch.x.is_some() {
        fields.push("x");
    }
    if patch.y.is_some() {
        fields.push("y");
    }
    if patch.width.is_some() {
        fields.push("width");
    }
    if patch.height.is_some() {
        fields.push("height");
    }
    if patch.z.is_some() {
        fields.push("z");
    }
    if patch.tab_order.is_some() {
        fields.push("tabOrder");
    }
    fields
}

fn position_number(position: &Value, key: &str) -> Option<f64> {
    position[key].as_f64()
}

fn number_value(value: f64, flag: &str) -> CliResult<Value> {
    if !value.is_finite() {
        return Err(CliError::invalid_args(format!(
            "{flag} must be a finite number"
        )));
    }
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| CliError::invalid_args(format!("{flag} must be a JSON number")))
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

fn require_position_patch(patch: &PositionPatch) -> CliResult<()> {
    if patch.x.is_some()
        || patch.y.is_some()
        || patch.width.is_some()
        || patch.height.is_some()
        || patch.z.is_some()
        || patch.tab_order.is_some()
    {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "report visuals set-position requires at least one geometry flag",
    )
    .with_hint("Pass one or more of --x, --y, --width, --height, --z, or --tab-order.")
    .with_suggested_command(
        "powerbi-cli report visuals set-position --project <project-dir-or.pbip> --handle <visual-handle> --x 40 --y 40 --dry-run --json",
    ))
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
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --json"
    )))
}
