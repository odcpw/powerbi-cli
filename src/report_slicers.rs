use crate::feature_catalog::unsupported_feature_error;
use crate::pbir_slicers::{
    ReportSlicerRecord, list_report_slicers, slicer_matches_handle_or_visual, slicer_matches_page,
    slicer_record_json,
};
use crate::report_slicer_clear::clear_slicer;
use crate::{CliError, CliResult, canonical_display, command_arg, resolve_project};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
    page: Option<String>,
    include_raw: bool,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    page: Option<String>,
    visual: Option<String>,
    include_raw: bool,
}

pub(crate) fn slicers_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report slicers requires a subcommand: list, show, or clear",
        )
        .with_hint("Run `powerbi-cli report slicers list --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli report slicers list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" | "ls" => list_slicers(rest),
        "show" | "get" => show_slicer(rest),
        "clear" => clear_slicer(rest),
        "sync" | "sync-group" | "syncGroup" => {
            Err(unsupported_feature_error("report.slicer-sync-authoring"))
        }
        "add" | "create" | "update" | "set" | "state" | "set-state" | "select" => {
            Err(unsupported_feature_error("report.slicer-authoring"))
        }
        _ => Err(CliError::invalid_args(format!(
            "unknown report slicers command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report slicers\"` for supported slicer commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report slicers\"")),
    }
}

fn list_slicers(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "report slicers list")?;
    let resolved = resolve_project(&project)?;
    let (records, validation) = list_report_slicers(&resolved)?;
    let filtered = filter_records(&records, &options.page);
    let slicers = filtered
        .iter()
        .map(|record| slicer_record_json(record, options.include_raw))
        .collect::<Vec<_>>();

    Ok(json!({
        "schema": "powerbi-cli.report.slicers.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "filter": {
            "page": options.page,
            "includeRaw": options.include_raw
        },
        "counts": slicer_counts(&filtered),
        "slicers": slicers,
        "next": [
            format!("powerbi-cli report slicers show --project {} --handle <slicer-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report filters list --project {} --scope visual --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": validation.warnings,
        "errors": validation.errors
    }))
}

fn show_slicer(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    require_slicer_selector(&options)?;
    let project = required_project(options.project.clone(), "report slicers show")?;
    let resolved = resolve_project(&project)?;
    let (records, validation) = list_report_slicers(&resolved)?;
    let record = find_slicer(&records, &options)?;
    let readback = format!(
        "powerbi-cli report slicers list --project {} --json",
        command_arg(&resolved.project_dir)
    );
    let visual_readback = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        command_arg(&resolved.project_dir),
        shell_arg(&record.visual_handle)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );

    Ok(json!({
        "schema": "powerbi-cli.report.slicers.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "slicer": slicer_record_json(record, options.include_raw),
        "readbackCommand": readback,
        "visualReadbackCommand": visual_readback,
        "validateCommand": validate,
        "next": [readback, visual_readback, validate],
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
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report slicers list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report slicers list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report slicers list --project <project-dir-or.pbip> --json",
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
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" | "--slicer" => options.visual = Some(take_value(args, &mut i, "--visual")?),
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
                    "unknown report slicers show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report slicers show --project <project-dir-or.pbip> --handle <slicer-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report slicers show --project <project-dir-or.pbip> --handle <slicer-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn filter_records<'a>(
    records: &'a [ReportSlicerRecord],
    page: &Option<String>,
) -> Vec<&'a ReportSlicerRecord> {
    records
        .iter()
        .filter(|record| {
            page.as_ref()
                .is_none_or(|page| slicer_matches_page(record, page))
        })
        .collect::<Vec<_>>()
}

fn find_slicer<'a>(
    records: &'a [ReportSlicerRecord],
    options: &ShowOptions,
) -> CliResult<&'a ReportSlicerRecord> {
    let matches = records
        .iter()
        .filter(|record| {
            if let Some(handle) = &options.handle {
                slicer_matches_handle_or_visual(record, handle)
            } else {
                options
                    .page
                    .as_ref()
                    .is_none_or(|page| slicer_matches_page(record, page))
                    && options
                        .visual
                        .as_ref()
                        .is_some_and(|visual| slicer_matches_handle_or_visual(record, visual))
            }
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(*record),
        [] => Err(CliError::invalid_args("slicer not found")
            .with_hint("Use `report slicers list` to get stable slicer handles.")
            .with_suggested_command(
                "powerbi-cli report slicers list --project <project-dir-or.pbip> --json",
            )),
        _ => Err(CliError::invalid_args("slicer selector matched multiple slicers")
            .with_hint("Use the exact slicer handle returned by `report slicers list`.")
            .with_suggested_command(
                "powerbi-cli report slicers show --project <project-dir-or.pbip> --handle <slicer-handle> --json",
            )),
    }
}

fn slicer_counts(records: &[&ReportSlicerRecord]) -> Value {
    json!({
        "slicers": records.len(),
        "boundSlicers": records.iter().filter(|record| !record.bindings.is_empty()).count(),
        "possibleDataValueSlicers": records.iter().filter(|record| record.may_contain_data_values).count()
    })
}

fn require_slicer_selector(options: &ShowOptions) -> CliResult<()> {
    if options.handle.is_some() || (options.page.is_some() && options.visual.is_some()) {
        return Ok(());
    }
    Err(
        CliError::invalid_args("report slicers show requires --handle or --page plus --visual")
            .with_hint("Use `report slicers list` to get stable slicer handles.")
            .with_suggested_command(
                "powerbi-cli report slicers list --project <project-dir-or.pbip> --json",
            ),
    )
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
            .with_hint(
                "Run `powerbi-cli --json capabilities --for \"report slicers\"` for exact usage.",
            )
            .with_suggested_command("powerbi-cli --json capabilities --for \"report slicers\"")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
