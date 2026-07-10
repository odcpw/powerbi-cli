use crate::cli_support::{
    MutationMode, mode_name, require_report_page_mode as require_page_mode, required_project,
    set_report_page_mode as set_mode, shell_arg, take_report_value as take_value, target_project,
};
use crate::pbir::{
    PageRecord, PageSelector, find_page, load_report_snapshot, page_detail, page_summary,
};
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Number, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const PAGES_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/pagesMetadata/1.0.0/schema.json";
const PAGE_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.0.0/schema.json";
const DEFAULT_WIDTH: f64 = 1280.0;
const DEFAULT_HEIGHT: f64 = 720.0;
const DEFAULT_DISPLAY_OPTION: &str = "FitToPage";

#[derive(Debug, Default)]
struct AddOptions {
    project: Option<PathBuf>,
    name: Option<String>,
    display_name: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
    display_option: Option<String>,
    before: Option<String>,
    after: Option<String>,
    set_active: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct UpdateOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    display_name: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
    display_option: Option<String>,
    allow_visuals_outside_page: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct ReorderOptions {
    project: Option<PathBuf>,
    order: Vec<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct SetActiveOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct DeleteOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    confirm: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn add_page(args: &[String]) -> CliResult<Value> {
    let options = parse_add_args(args)?;
    let source_project = required_project(options.project.clone(), "report pages add")?;
    let display_name = options.display_name.as_deref().ok_or_else(|| {
        CliError::invalid_args("report pages add requires --display-name")
            .with_hint("Agents should give every page a human-readable display name.")
            .with_suggested_command(
                "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name \"Executive Summary\" --dry-run --json",
            )
    })?;
    validate_nonempty_text(display_name, "--display-name")?;
    let width = options.width.unwrap_or(DEFAULT_WIDTH);
    let height = options.height.unwrap_or(DEFAULT_HEIGHT);
    validate_page_size(width, height)?;
    let display_option = options
        .display_option
        .as_deref()
        .unwrap_or(DEFAULT_DISPLAY_OPTION);
    validate_display_option(display_option)?;
    if options.before.is_some() && options.after.is_some() {
        return Err(CliError::invalid_args(
            "choose only one insertion point: --before or --after",
        )
        .with_hint("Omit both to append the page at the end.")
        .with_suggested_command(
            "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --dry-run --json",
        ));
    }
    let mode = require_page_mode(options.mode, "report pages add")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, add_page)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let pages_dir = pages_dir(&target_resolved);
    let pages_json_path = pages_dir.join("pages.json");
    let mut pages_json = pages_json(&pages_json_path)?;
    let mut page_order = page_order(&pages_json, &pages_json_path)?;
    let page_name = match options.name.as_deref() {
        Some(name) => validate_new_page_name(name, &snapshot.pages)?,
        None => generated_page_name(display_name, &snapshot.pages),
    };
    let insert_at = add_insert_index(&page_order, &snapshot.pages, options.before, options.after)?;
    page_order.insert(insert_at, page_name.clone());
    set_page_order(&mut pages_json, page_order.clone(), &pages_json_path)?;
    let should_set_active = options.set_active || pages_json["activePageName"].as_str().is_none();
    if should_set_active {
        set_active_name(&mut pages_json, &page_name, &pages_json_path)?;
    }
    let page_dir = pages_dir.join(&page_name);
    let page_json_path = page_dir.join("page.json");
    let page_json = new_page_json(&page_name, display_name, width, height, display_option);
    let target = new_page_summary(
        &page_name,
        insert_at,
        should_set_active,
        &page_json_path,
        &page_json,
    );
    let before_order = snapshot
        .pages
        .iter()
        .map(|page| page.name.clone())
        .collect::<Vec<_>>();
    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        fs::create_dir_all(page_dir.join("visuals")).map_err(|err| {
            CliError::unexpected(format!(
                "create page visuals dir {}: {err}",
                page_dir.join("visuals").display()
            ))
        })?;
        write_json_atomic(&pages_json_path, &pages_json)?;
        write_json_file(&page_json_path, &page_json)?;
    }

    mutation_response(
        &target_resolved,
        mode,
        "add",
        target.clone(),
        vec![
            json!({
                "kind": "pbir.pages.pageOrder",
                "action": "insert",
                "path": canonical_display(&pages_json_path),
                "before": before_order,
                "after": page_order
            }),
            json!({
                "kind": "pbir.page",
                "action": "add",
                "path": canonical_display(&page_json_path),
                "before": Value::Null,
                "after": page_json
            }),
        ],
        readback_show_command(&target_resolved, &format!("page:{page_name}")),
    )
}

pub(crate) fn update_page(args: &[String]) -> CliResult<Value> {
    let options = parse_update_args(args)?;
    let source_project = required_project(options.project.clone(), "report pages update")?;
    require_page_selector(&options.selector, "report pages update")?;
    require_update_patch(&options)?;
    let width = options.width;
    let height = options.height;
    if width.is_some() || height.is_some() {
        validate_page_size(
            width.unwrap_or(DEFAULT_WIDTH),
            height.unwrap_or(DEFAULT_HEIGHT),
        )?;
    }
    if let Some(display_name) = options.display_name.as_deref() {
        validate_nonempty_text(display_name, "--display-name")?;
    }
    if let Some(display_option) = options.display_option.as_deref() {
        validate_display_option(display_option)?;
    }
    let mode = require_page_mode(options.mode, "report pages update")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, update_page)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let page = find_page(&snapshot.pages, &options.selector, "report pages update")?.clone();
    validate_existing_visual_bounds(
        &page,
        options.width.or_else(|| page.width.as_f64()),
        options.height.or_else(|| page.height.as_f64()),
        options.allow_visuals_outside_page,
    )?;
    let page_path = page_path(&page)?;
    let mut page_json = read_json_value(&page_path)?;
    let before_json = page_json.clone();
    let before = page_summary(&page);
    if let Some(display_name) = options.display_name.as_deref() {
        object_mut(&mut page_json, &page_path)?.insert(
            "displayName".to_string(),
            Value::String(display_name.to_string()),
        );
    }
    if let Some(width) = options.width {
        object_mut(&mut page_json, &page_path)?
            .insert("width".to_string(), number_value(width, "--width")?);
    }
    if let Some(height) = options.height {
        object_mut(&mut page_json, &page_path)?
            .insert("height".to_string(), number_value(height, "--height")?);
    }
    if let Some(display_option) = options.display_option.as_deref() {
        object_mut(&mut page_json, &page_path)?.insert(
            "displayOption".to_string(),
            Value::String(display_option.to_string()),
        );
    }
    let mut after = before.clone();
    if let Some(display_name) = options.display_name {
        after["displayName"] = Value::String(display_name);
    }
    if let Some(width) = options.width {
        after["width"] = number_value(width, "--width")?;
    }
    if let Some(height) = options.height {
        after["height"] = number_value(height, "--height")?;
    }
    if let Some(display_option) = options.display_option {
        after["displayOption"] = Value::String(display_option);
    }
    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&page_path, &page_json)?;
    }
    mutation_response(
        &target_resolved,
        mode,
        "update",
        before.clone(),
        vec![json!({
            "kind": "pbir.page",
            "action": "update",
            "path": canonical_display(&page_path),
            "fields": update_fields(&before, &after),
            "before": before_json,
            "after": page_json
        })],
        readback_show_command(&target_resolved, &page.handle),
    )
}

pub(crate) fn reorder_pages(args: &[String]) -> CliResult<Value> {
    let options = parse_reorder_args(args)?;
    let source_project = required_project(options.project.clone(), "report pages reorder")?;
    if options.order.is_empty() {
        return Err(CliError::invalid_args(
            "report pages reorder requires --order or repeated --page entries",
        )
        .with_hint("Pass every page exactly once, using handles from `report pages list`.")
        .with_suggested_command(
            "powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle>,<page-handle> --dry-run --json",
        ));
    }
    let mode = require_page_mode(options.mode, "report pages reorder")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, reorder_pages)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let pages_json_path = pages_dir(&target_resolved).join("pages.json");
    let mut pages_json = pages_json(&pages_json_path)?;
    let before_order = page_order(&pages_json, &pages_json_path)?;
    let after_order = resolve_order(&snapshot.pages, &options.order)?;
    set_page_order(&mut pages_json, after_order.clone(), &pages_json_path)?;
    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&pages_json_path, &pages_json)?;
    }
    mutation_response(
        &target_resolved,
        mode,
        "reorder",
        json!({
            "before": before_order,
            "after": after_order
        }),
        vec![json!({
            "kind": "pbir.pages.pageOrder",
            "action": "reorder",
            "path": canonical_display(&pages_json_path),
            "before": before_order,
            "after": after_order
        })],
        pages_list_command(&target_resolved),
    )
}

pub(crate) fn set_active_page(args: &[String]) -> CliResult<Value> {
    let options = parse_set_active_args(args)?;
    let source_project = required_project(options.project.clone(), "report pages set-active")?;
    require_page_selector(&options.selector, "report pages set-active")?;
    let mode = require_page_mode(options.mode, "report pages set-active")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, set_active_page)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let page = find_page(
        &snapshot.pages,
        &options.selector,
        "report pages set-active",
    )?
    .clone();
    let pages_json_path = pages_dir(&target_resolved).join("pages.json");
    let mut pages_json = pages_json(&pages_json_path)?;
    let before_active = pages_json["activePageName"].clone();
    set_active_name(&mut pages_json, &page.name, &pages_json_path)?;
    let before = page_summary(&page);
    let mut after = before.clone();
    after["isActive"] = Value::Bool(true);
    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&pages_json_path, &pages_json)?;
    }
    mutation_response(
        &target_resolved,
        mode,
        "set-active",
        before.clone(),
        vec![json!({
            "kind": "pbir.pages.activePageName",
            "action": "set-active",
            "path": canonical_display(&pages_json_path),
            "before": before_active,
            "after": page.name
        })],
        readback_show_command(&target_resolved, &page.handle),
    )
}

pub(crate) fn delete_empty_page(args: &[String]) -> CliResult<Value> {
    let options = parse_delete_args(args)?;
    let source_project = required_project(options.project.clone(), "report pages delete-empty")?;
    require_page_selector(&options.selector, "report pages delete-empty")?;
    let mode = require_page_mode(options.mode, "report pages delete-empty")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, delete_empty_page)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let page = find_page(
        &snapshot.pages,
        &options.selector,
        "report pages delete-empty",
    )?
    .clone();
    if !page.visuals.is_empty() {
        return Err(CliError::invalid_args(
            "report pages delete-empty refuses pages that contain visuals",
        )
        .with_hint(
            "Delete or move visuals in a separate future visual command before deleting the page.",
        )
        .with_suggested_command(format!(
            "powerbi-cli report pages show --project {} --handle {} --json",
            command_arg(&target_resolved.project_dir),
            shell_arg(&page.handle)
        )));
    }
    if snapshot.pages.len() <= 1 {
        return Err(CliError::invalid_args(
            "report pages delete-empty refuses to delete the last page",
        )
        .with_hint("Add another page first, then delete the empty page.")
        .with_suggested_command(format!(
            "powerbi-cli report pages add --project {} --display-name <name> --dry-run --json",
            command_arg(&target_resolved.project_dir)
        )));
    }
    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&page.handle) {
        return Err(CliError::invalid_args(
            "in-place page deletion requires --confirm <page-handle>",
        )
        .with_hint("Run the same command with --dry-run first, then confirm the exact page handle.")
        .with_suggested_command(format!(
            "powerbi-cli report pages delete-empty --project {} --handle {} --in-place --confirm {} --json",
            command_arg(&target_resolved.project_dir),
            shell_arg(&page.handle),
            shell_arg(&page.handle)
        )));
    }
    let pages_json_path = pages_dir(&target_resolved).join("pages.json");
    let mut pages_json = pages_json(&pages_json_path)?;
    let before_order = page_order(&pages_json, &pages_json_path)?;
    let after_order = before_order
        .iter()
        .filter(|name| *name != &page.name)
        .cloned()
        .collect::<Vec<_>>();
    set_page_order(&mut pages_json, after_order.clone(), &pages_json_path)?;
    if pages_json["activePageName"].as_str() == Some(&page.name)
        && let Some(next_active) = after_order.first()
    {
        set_active_name(&mut pages_json, next_active, &pages_json_path)?;
    }
    let page_path = page_path(&page)?;
    let page_dir = page_path.parent().ok_or_else(|| {
        CliError::validation_failed(format!("page path has no parent: {}", page_path.display()))
    })?;
    ensure_child_path(page_dir, &pages_dir(&target_resolved))?;
    ensure_empty_page_dir(page_dir, &page_path)?;
    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&pages_json_path, &pages_json)?;
        fs::remove_dir_all(page_dir).map_err(|err| {
            CliError::unexpected(format!("remove page dir {}: {err}", page_dir.display()))
        })?;
    }
    mutation_response(
        &target_resolved,
        mode,
        "delete-empty",
        page_detail(&page),
        vec![
            json!({
                "kind": "pbir.pages.pageOrder",
                "action": "delete",
                "path": canonical_display(&pages_json_path),
                "before": before_order,
                "after": after_order
            }),
            json!({
                "kind": "pbir.page",
                "action": "delete-empty",
                "path": canonical_display(&page_path),
                "before": page_detail(&page),
                "after": Value::Null
            }),
        ],
        pages_list_command(&target_resolved),
    )
}

fn parse_add_args(args: &[String]) -> CliResult<AddOptions> {
    let mut options = AddOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--display-name" | "--displayName" | "--title" => {
                options.display_name = Some(take_value(args, &mut i, "--display-name")?);
            }
            "--width" => options.width = Some(take_f64(args, &mut i, "--width")?),
            "--height" => options.height = Some(take_f64(args, &mut i, "--height")?),
            "--display-option" | "--displayOption" => {
                options.display_option = Some(take_value(args, &mut i, "--display-option")?);
            }
            "--before" => options.before = Some(take_value(args, &mut i, "--before")?),
            "--after" => options.after = Some(take_value(args, &mut i, "--after")?),
            "--set-active" | "--active" => {
                options.set_active = true;
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
            other => return unknown_page_flag("report pages add", other),
        }
    }
    Ok(options)
}

fn parse_update_args(args: &[String]) -> CliResult<UpdateOptions> {
    let mut options = UpdateOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" | "--name" => {
                set_page_selector(&mut options.selector, take_value(args, &mut i, "--page")?)
            }
            "--display-name" | "--displayName" | "--title" => {
                options.display_name = Some(take_value(args, &mut i, "--display-name")?);
            }
            "--width" => options.width = Some(take_f64(args, &mut i, "--width")?),
            "--height" => options.height = Some(take_f64(args, &mut i, "--height")?),
            "--display-option" | "--displayOption" => {
                options.display_option = Some(take_value(args, &mut i, "--display-option")?);
            }
            "--allow-visuals-outside-page" => {
                options.allow_visuals_outside_page = true;
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
            other => return unknown_page_flag("report pages update", other),
        }
    }
    Ok(options)
}

fn parse_reorder_args(args: &[String]) -> CliResult<ReorderOptions> {
    let mut options = ReorderOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--order" => {
                options
                    .order
                    .extend(split_order(&take_value(args, &mut i, "--order")?));
            }
            "--page" => options.order.push(take_value(args, &mut i, "--page")?),
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
            other => return unknown_page_flag("report pages reorder", other),
        }
    }
    Ok(options)
}

fn parse_set_active_args(args: &[String]) -> CliResult<SetActiveOptions> {
    let mut options = SetActiveOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" | "--name" => {
                set_page_selector(&mut options.selector, take_value(args, &mut i, "--page")?)
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
            other => return unknown_page_flag("report pages set-active", other),
        }
    }
    Ok(options)
}

fn parse_delete_args(args: &[String]) -> CliResult<DeleteOptions> {
    let mut options = DeleteOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" | "--name" => {
                set_page_selector(&mut options.selector, take_value(args, &mut i, "--page")?)
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
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
            other => return unknown_page_flag("report pages delete-empty", other),
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

fn unknown_page_flag<T>(command: &str, flag: &str) -> CliResult<T> {
    Err(
        CliError::invalid_args(format!("unknown {command} flag: {flag}"))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for \"report pages\"` for exact flags.",
            )
            .with_suggested_command("powerbi-cli --json capabilities --for \"report pages\""),
    )
}

fn add_insert_index(
    page_order: &[String],
    pages: &[PageRecord],
    before: Option<String>,
    after: Option<String>,
) -> CliResult<usize> {
    if let Some(before) = before {
        let page = find_page(pages, &selector_from_value(before), "report pages add")?;
        return page_order
            .iter()
            .position(|name| name == &page.name)
            .ok_or_else(|| {
                CliError::validation_failed("insertion page is missing from pageOrder")
            });
    }
    if let Some(after) = after {
        let page = find_page(pages, &selector_from_value(after), "report pages add")?;
        return page_order
            .iter()
            .position(|name| name == &page.name)
            .map(|index| index + 1)
            .ok_or_else(|| {
                CliError::validation_failed("insertion page is missing from pageOrder")
            });
    }
    Ok(page_order.len())
}

fn resolve_order(pages: &[PageRecord], requested: &[String]) -> CliResult<Vec<String>> {
    if requested.len() != pages.len() {
        return Err(CliError::invalid_args(
            "report pages reorder requires every page exactly once",
        )
        .with_hint("Use `report pages list` and pass all page handles in the desired order.")
        .with_suggested_command(
            "powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle>,<page-handle> --dry-run --json",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut order = Vec::new();
    for selector in requested {
        let page = find_page(
            pages,
            &selector_from_value(selector.clone()),
            "report pages reorder",
        )?;
        if !seen.insert(page.name.clone()) {
            return Err(CliError::invalid_args(format!(
                "page appears more than once in reorder input: {}",
                page.handle
            ))
            .with_hint("Pass every page exactly once.")
            .with_suggested_command(
                "powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle>,<page-handle> --dry-run --json",
            ));
        }
        order.push(page.name.clone());
    }
    let expected = pages
        .iter()
        .map(|page| page.name.clone())
        .collect::<BTreeSet<_>>();
    if seen != expected {
        return Err(CliError::invalid_args(
            "report pages reorder input does not match the project page set",
        )
        .with_hint("Use `report pages list` and pass all page handles in the desired order.")
        .with_suggested_command(
            "powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle>,<page-handle> --dry-run --json",
        ));
    }
    Ok(order)
}

fn selector_from_value(value: String) -> PageSelector {
    if value.starts_with("page:") {
        PageSelector {
            handle: Some(value),
            name: None,
        }
    } else {
        PageSelector {
            handle: None,
            name: Some(value),
        }
    }
}

fn validate_new_page_name(name: &str, pages: &[PageRecord]) -> CliResult<String> {
    validate_page_name(name)?;
    if pages.iter().any(|page| page.name == name) {
        return Err(CliError::invalid_args(format!("page already exists: {name}"))
            .with_hint("Choose a unique internal --name or omit it so powerbi-cli can generate one.")
            .with_suggested_command(
                "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --dry-run --json",
            ));
    }
    Ok(name.to_string())
}

fn validate_page_name(name: &str) -> CliResult<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch == ':' || ch.is_control())
    {
        return Err(CliError::invalid_args(format!("unsafe page name: {name}"))
            .with_hint("Use a simple internal page name without path separators.")
            .with_suggested_command(
                "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --dry-run --json",
            ));
    }
    Ok(())
}

fn generated_page_name(display_name: &str, pages: &[PageRecord]) -> String {
    let stem = pascal_identifier(display_name).unwrap_or_else(|| "Page".to_string());
    let base = format!("ReportSection{stem}");
    if !pages.iter().any(|page| page.name == base) {
        return base;
    }
    for index in 2..1000 {
        let candidate = format!("{base}{index}");
        if !pages.iter().any(|page| page.name == candidate) {
            return candidate;
        }
    }
    format!("ReportSection{}", pages.len() + 1)
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

fn new_page_json(
    page_name: &str,
    display_name: &str,
    width: f64,
    height: f64,
    display_option: &str,
) -> Value {
    json!({
        "$schema": PAGE_SCHEMA,
        "name": page_name,
        "displayName": display_name,
        "displayOption": display_option,
        "height": height,
        "width": width,
        "annotations": [
            {
                "name": "powerbi-cli.layout",
                "value": "Page created offline by powerbi-cli; visuals can be added by later guarded report commands."
            }
        ]
    })
}

fn new_page_summary(
    page_name: &str,
    ordinal: usize,
    is_active: bool,
    path: &Path,
    page_json: &Value,
) -> Value {
    json!({
        "handle": format!("page:{page_name}"),
        "name": page_name,
        "displayName": page_json["displayName"],
        "ordinal": ordinal,
        "width": page_json["width"],
        "height": page_json["height"],
        "displayOption": page_json["displayOption"],
        "isActive": is_active,
        "path": canonical_display(path),
        "visualCount": 0,
        "visualHandles": []
    })
}

fn mutation_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    action: &str,
    target: Value,
    changes: Vec<Value>,
    readback: String,
) -> CliResult<Value> {
    let dry_run = matches!(mode, MutationMode::DryRun);
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(target_resolved)?)
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
        "schema": "powerbi-cli.report.pages.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": action,
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": target,
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

fn readback_show_command(resolved: &ResolvedProject, page_handle: &str) -> String {
    format!(
        "powerbi-cli report pages show --project {} --handle {} --json",
        command_arg(&resolved.project_dir),
        shell_arg(page_handle)
    )
}

fn pages_list_command(resolved: &ResolvedProject) -> String {
    format!(
        "powerbi-cli report pages list --project {} --json",
        command_arg(&resolved.project_dir)
    )
}

fn pages_json(path: &Path) -> CliResult<Value> {
    let mut value = read_json_value(path)?;
    let object = object_mut(&mut value, path)?;
    object
        .entry("$schema".to_string())
        .or_insert_with(|| Value::String(PAGES_SCHEMA.to_string()));
    Ok(value)
}

fn page_order(value: &Value, path: &Path) -> CliResult<Vec<String>> {
    let Some(items) = value["pageOrder"].as_array() else {
        return Err(CliError::validation_failed(format!(
            "{} has no pageOrder array",
            path.display()
        )));
    };
    items
        .iter()
        .map(|item| {
            item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                CliError::validation_failed(format!(
                    "{} pageOrder contains a non-string entry",
                    path.display()
                ))
            })
        })
        .collect()
}

fn set_page_order(value: &mut Value, order: Vec<String>, path: &Path) -> CliResult<()> {
    object_mut(value, path)?.insert(
        "pageOrder".to_string(),
        Value::Array(order.into_iter().map(Value::String).collect()),
    );
    Ok(())
}

fn set_active_name(value: &mut Value, page_name: &str, path: &Path) -> CliResult<()> {
    object_mut(value, path)?.insert(
        "activePageName".to_string(),
        Value::String(page_name.to_string()),
    );
    Ok(())
}

fn object_mut<'a>(value: &'a mut Value, path: &Path) -> CliResult<&'a mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} is not a JSON object", path.display()))
    })
}

fn pages_dir(resolved: &ResolvedProject) -> PathBuf {
    resolved.report_dir.join("definition").join("pages")
}

fn page_path(page: &PageRecord) -> CliResult<PathBuf> {
    page.path.clone().ok_or_else(|| {
        CliError::validation_failed(format!(
            "page has no path in inspect output: {}",
            page.handle
        ))
    })
}

fn require_update_patch(options: &UpdateOptions) -> CliResult<()> {
    if options.display_name.is_some()
        || options.width.is_some()
        || options.height.is_some()
        || options.display_option.is_some()
    {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "report pages update requires at least one page metadata flag",
    )
    .with_hint("Pass --display-name, --width, --height, or --display-option.")
    .with_suggested_command(
        "powerbi-cli report pages update --project <project-dir-or.pbip> --handle <page-handle> --display-name <name> --dry-run --json",
    ))
}

fn update_fields(before: &Value, after: &Value) -> Vec<&'static str> {
    let mut fields = Vec::new();
    for key in ["displayName", "width", "height", "displayOption"] {
        if before[key] != after[key] {
            fields.push(key);
        }
    }
    fields
}

fn validate_page_size(width: f64, height: f64) -> CliResult<()> {
    if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "page width and height must be positive finite numbers",
    )
    .with_hint("Use values such as --width 1280 --height 720.")
    .with_suggested_command(
        "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --width 1280 --height 720 --dry-run --json",
    ))
}

fn validate_existing_visual_bounds(
    page: &PageRecord,
    page_width: Option<f64>,
    page_height: Option<f64>,
    allow_outside: bool,
) -> CliResult<()> {
    if allow_outside {
        return Ok(());
    }
    let (Some(page_width), Some(page_height)) = (page_width, page_height) else {
        return Ok(());
    };
    let mut max_x = 0.0_f64;
    let mut max_y = 0.0_f64;
    for visual in &page.visuals {
        let x = visual.position["x"].as_f64().unwrap_or_default();
        let y = visual.position["y"].as_f64().unwrap_or_default();
        let width = visual.position["width"].as_f64().unwrap_or_default();
        let height = visual.position["height"].as_f64().unwrap_or_default();
        max_x = max_x.max(x + width);
        max_y = max_y.max(y + height);
    }
    if max_x <= page_width && max_y <= page_height {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "page size would put existing visuals outside page bounds",
    )
    .with_hint("Resize or move visuals first, or pass --allow-visuals-outside-page deliberately.")
    .with_suggested_command(
        "powerbi-cli report pages update --project <project-dir-or.pbip> --handle <page-handle> --allow-visuals-outside-page --dry-run --json",
    ))
}

fn validate_display_option(value: &str) -> CliResult<()> {
    validate_nonempty_text(value, "--display-option")?;
    if value.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "--display-option must be a simple Power BI display option, got {value}"
    ))
    .with_hint("Common values include FitToPage, FitToWidth, and ActualSize.")
    .with_suggested_command(
        "powerbi-cli report pages update --project <project-dir-or.pbip> --handle <page-handle> --display-option FitToPage --dry-run --json",
    ))
}

fn validate_nonempty_text(value: &str, flag: &str) -> CliResult<()> {
    if !value.trim().is_empty() && !value.chars().any(char::is_control) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be nonempty text"
    )))
}

fn ensure_child_path(path: &Path, parent: &Path) -> CliResult<()> {
    let path_abs = fs::canonicalize(path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?;
    let parent_abs = fs::canonicalize(parent)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", parent.display())))?;
    if path_abs.starts_with(parent_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to remove page outside pages directory: {}",
        path.display()
    )))
}

fn ensure_empty_page_dir(page_dir: &Path, page_json_path: &Path) -> CliResult<()> {
    for entry in fs::read_dir(page_dir).map_err(|err| {
        CliError::unexpected(format!("read page dir {}: {err}", page_dir.display()))
    })? {
        let entry = entry.map_err(|err| {
            CliError::unexpected(format!("read page dir entry {}: {err}", page_dir.display()))
        })?;
        let path = entry.path();
        let file_name = entry.file_name();
        if path == page_json_path {
            continue;
        }
        if file_name == "visuals" && path.is_dir() && directory_is_empty(&path)? {
            continue;
        }
        return Err(CliError::invalid_args(
            "report pages delete-empty refuses page directories with unknown files or non-empty subdirectories",
        )
        .with_hint("This command deletes only simple empty pages: page.json plus an empty visuals directory.")
        .with_suggested_command(
            "powerbi-cli report pages show --project <project-dir-or.pbip> --handle <page-handle> --json",
        ));
    }
    Ok(())
}

fn directory_is_empty(path: &Path) -> CliResult<bool> {
    let mut entries = fs::read_dir(path)
        .map_err(|err| CliError::unexpected(format!("read dir {}: {err}", path.display())))?;
    Ok(entries.next().is_none())
}

fn write_json_file(path: &Path, value: &Value) -> CliResult<()> {
    if path.exists() {
        return write_json_atomic(path, value);
    }
    let parent = path
        .parent()
        .ok_or_else(|| CliError::unexpected(format!("path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    fs::write(path, text)
        .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))?;
    Ok(())
}

fn split_order(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn require_page_selector(selector: &PageSelector, command: &str) -> CliResult<()> {
    if selector.handle.is_some() || selector.name.is_some() {
        return Ok(());
    }
    Err(CliError::invalid_args(format!("{command} requires --handle or --page"))
        .with_hint("Use `report pages list` to get stable page handles.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <page-handle> --json"
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
