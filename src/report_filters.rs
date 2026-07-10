use crate::cli_support::{required_project, shell_arg, take_value};
use crate::pbir_filters::{
    FilterScope, ReportFilterRecord, filter_record_json, list_report_filters, owner_matches_page,
    owner_matches_visual, select_filter_by_handle,
};
use crate::report_filter_add::add_filter;
use crate::report_filter_clear::clear_filters;
use crate::report_filter_mutations::delete_filter;
use crate::report_filter_update::update_filter;
use crate::{CliError, CliResult, canonical_display, command_arg, resolve_project};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug)]
struct ListOptions {
    project: Option<PathBuf>,
    scope: FilterScope,
    page: Option<String>,
    visual: Option<String>,
    include_raw: bool,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            project: None,
            scope: FilterScope::All,
            page: None,
            visual: None,
            include_raw: false,
        }
    }
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    include_raw: bool,
}

pub(crate) fn filters_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report filters requires a subcommand: list, show, add, update, delete, or clear",
        )
        .with_hint("Run `powerbi-cli report filters list --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" | "ls" => list_filters(rest),
        "show" | "get" => show_filter(rest),
        "add" | "create" => add_filter(rest),
        "update" | "edit" => update_filter(rest),
        "delete" | "remove" => delete_filter(rest),
        "clear" | "reset" => clear_filters(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown report filters command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report filters\"` for supported filter commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report filters\"")),
    }
}

fn list_filters(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "report filters list")?;
    require_selector_shape(&options.page, &options.visual, "report filters list")?;
    let resolved = resolve_project(&project)?;
    let (records, validation) = list_report_filters(&resolved)?;
    let filtered = filter_records(&records, options.scope, &options.page, &options.visual);
    let filters = filtered
        .iter()
        .map(|record| filter_record_json(record, options.include_raw))
        .collect::<Vec<_>>();
    let counts = filter_counts(&filtered);

    Ok(json!({
        "schema": "powerbi-cli.report.filters.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "filter": {
            "scope": options.scope.as_str(),
            "page": options.page,
            "visual": options.visual,
            "includeRaw": options.include_raw
        },
        "counts": counts,
        "filters": filters,
        "next": [
            format!("powerbi-cli report filters show --project {} --handle <filter-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report pages list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": validation.warnings,
        "errors": validation.errors
    }))
}

fn show_filter(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "report filters show")?;
    let handle = options.handle.ok_or_else(|| {
        CliError::invalid_args("report filters show requires --handle <filter-handle>")
            .with_hint("Use `report filters list` to get stable filter handles.")
            .with_suggested_command(
                "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
            )
    })?;
    let resolved = resolve_project(&project)?;
    let (records, validation) = list_report_filters(&resolved)?;
    let record = select_filter_by_handle(&records, &handle, &resolved.project_dir, false)?;
    let readback = owner_readback_command(record, &resolved.project_dir);
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );

    Ok(json!({
        "schema": "powerbi-cli.report.filters.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "filter": filter_record_json(record, options.include_raw),
        "readbackCommand": readback,
        "validateCommand": validate,
        "next": [readback, validate],
        "warnings": validation.warnings,
        "errors": validation.errors
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
            "--scope" => options.scope = parse_scope(&take_value(args, &mut i, "--scope")?)?,
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" => options.visual = Some(take_value(args, &mut i, "--visual")?),
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report filters list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report filters list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_show_args(args: &[String]) -> CliResult<ShowOptions> {
    let mut options = ShowOptions {
        include_raw: true,
        ..ShowOptions::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--no-raw" | "--noRaw" => {
                options.include_raw = false;
                i += 1;
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report filters show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report filters show --project <project-dir-or.pbip> --handle <filter-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report filters show --project <project-dir-or.pbip> --handle <filter-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_scope(value: &str) -> CliResult<FilterScope> {
    match value {
        "all" => Ok(FilterScope::All),
        "report" => Ok(FilterScope::Report),
        "page" => Ok(FilterScope::Page),
        "visual" => Ok(FilterScope::Visual),
        other => Err(CliError::invalid_args(format!(
            "invalid report filters scope: {other}"
        ))
        .with_hint("Use --scope all, report, page, or visual.")
        .with_suggested_command(
            "powerbi-cli report filters list --project <project-dir-or.pbip> --scope all --json",
        )),
    }
}

fn filter_records<'a>(
    records: &'a [ReportFilterRecord],
    scope: FilterScope,
    page: &Option<String>,
    visual: &Option<String>,
) -> Vec<&'a ReportFilterRecord> {
    records
        .iter()
        .filter(|record| scope == FilterScope::All || record.scope == scope)
        .filter(|record| {
            page.as_ref()
                .is_none_or(|page| owner_matches_page(&record.owner, page))
        })
        .filter(|record| {
            visual
                .as_ref()
                .is_none_or(|visual| owner_matches_visual(&record.owner, visual))
        })
        .collect::<Vec<_>>()
}

fn filter_counts(records: &[&ReportFilterRecord]) -> Value {
    json!({
        "filters": records.len(),
        "reportFilters": records.iter().filter(|record| record.scope == FilterScope::Report).count(),
        "pageFilters": records.iter().filter(|record| record.scope == FilterScope::Page).count(),
        "visualFilters": records.iter().filter(|record| record.scope == FilterScope::Visual).count(),
        "unsupported": records.iter().filter(|record| record.unsupported).count(),
        "possibleDataValueFilters": records.iter().filter(|record| record.may_contain_data_values).count()
    })
}

fn owner_readback_command(record: &ReportFilterRecord, project_dir: &std::path::Path) -> String {
    let project = command_arg(project_dir);
    match &record.owner {
        crate::pbir_filters::FilterOwner::Report { .. } => {
            format!("powerbi-cli report wireframe export {project} --json")
        }
        crate::pbir_filters::FilterOwner::Page { handle, .. } => format!(
            "powerbi-cli report pages show --project {project} --handle {} --json",
            shell_arg(handle)
        ),
        crate::pbir_filters::FilterOwner::Visual { handle, .. } => format!(
            "powerbi-cli report visuals show --project {project} --handle {} --json",
            shell_arg(handle)
        ),
    }
}

fn require_selector_shape(
    page: &Option<String>,
    visual: &Option<String>,
    command: &str,
) -> CliResult<()> {
    if let Some(visual) = visual
        && !visual.starts_with("visual:")
        && page.is_none()
    {
        return Err(CliError::invalid_args(format!(
            "{command} requires --page when --visual is not a full visual handle"
        ))
        .with_hint("Pass a full visual handle or combine --page <page> --visual <visual>.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-handle> --visual <visual-name> --json"
        )));
    }
    Ok(())
}
