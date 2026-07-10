use crate::pbir::{PageSelector, find_page, load_report_snapshot, page_detail, page_summary};
use crate::report_page_mutations::{
    add_page, delete_empty_page, reorder_pages, set_active_page, update_page,
};
use crate::{CliError, CliResult, canonical_display, command_arg, resolve_project};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(crate) fn pages_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args("report pages requires a subcommand: list, show, add, update, reorder, set-active, delete-empty")
                .with_hint(
                    "Run `powerbi-cli report pages list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report pages list --project <project-dir-or.pbip> --json",
                ),
        );
    };

    match action.as_str() {
        "list" => list_pages(rest),
        "show" => show_page(rest),
        "add" | "create" => add_page(rest),
        "update" | "patch" => update_page(rest),
        "reorder" | "order" => reorder_pages(rest),
        "set-active" | "setActive" | "activate" => set_active_page(rest),
        "delete-empty" | "deleteEmpty" | "delete" => delete_empty_page(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown report pages command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report pages\"` for supported page commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report pages\"")),
    }
}

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
}

fn list_pages(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "report pages list")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let pages = snapshot.pages.iter().map(page_summary).collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.report.pages.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "counts": {
            "pages": pages.len(),
            "visuals": snapshot.validation.visuals,
            "boundVisuals": snapshot.validation.bound_visuals
        },
        "pages": pages,
        "next": [
            format!("powerbi-cli report pages show --project {} --handle <page-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report pages add --project {} --display-name <name> --dry-run --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

fn show_page(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "report pages show")?;
    require_page_selector(&options.selector, "report pages show")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let page = find_page(&snapshot.pages, &options.selector, "report pages show")?;
    Ok(json!({
        "schema": "powerbi-cli.report.pages.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "page": page_detail(page),
        "next": [
            format!("powerbi-cli report pages update --project {} --handle {} --display-name <name> --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&page.handle)),
            format!("powerbi-cli report visuals list --project {} --page {} --json", command_arg(&resolved.project_dir), shell_arg(&page.handle)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
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
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report pages list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report pages list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report pages list --project <project-dir-or.pbip> --json",
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
            "--page" | "--name" => {
                let value = take_value(args, &mut i, "--page")?;
                if value.starts_with("page:") {
                    options.selector.handle = Some(value);
                } else {
                    options.selector.name = Some(value);
                }
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report pages show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report pages show --project <project-dir-or.pbip> --handle <page-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report pages show --project <project-dir-or.pbip> --handle <page-handle> --json",
                ));
            }
        }
    }
    Ok(options)
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

fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass the PBIP project directory or the .pbip file explicitly with `--project`.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --json"
        ))
    })
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
