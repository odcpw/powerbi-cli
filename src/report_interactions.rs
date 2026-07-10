use crate::feature_catalog::unsupported_feature_error;
use crate::pbir_interactions::{
    ReportInteractionRecord, interaction_matches_handle, interaction_matches_page,
    interaction_matches_source, interaction_matches_target, interaction_record_json,
    interaction_semantics, list_report_interactions,
};
use crate::report_interaction_mutations::{disable_interaction, set_interaction};
use crate::{CliError, CliResult, canonical_display, command_arg, resolve_project};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
    page: Option<String>,
    source: Option<String>,
    target: Option<String>,
    interaction_type: Option<String>,
    include_raw: bool,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    page: Option<String>,
    source: Option<String>,
    target: Option<String>,
    include_raw: bool,
}

pub(crate) fn interactions_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report interactions requires a subcommand: list, show, set, or disable",
        )
        .with_hint(
            "Run `powerbi-cli report interactions list --project <project-dir-or.pbip> --json`.",
        )
        .with_suggested_command(
            "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" | "ls" => list_interactions(rest),
        "show" | "get" => show_interaction(rest),
        "set" | "update" => set_interaction(rest),
        "disable" => disable_interaction(rest),
        "reset" | "default" | "delete" | "remove" => {
            Err(unsupported_feature_error("report.interaction-default-reset"))
        }
        _ => Err(CliError::invalid_args(format!(
            "unknown report interactions command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report interactions\"` for supported interaction commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report interactions\"")),
    }
}

fn list_interactions(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "report interactions list")?;
    let resolved = resolve_project(&project)?;
    let (records, validation) = list_report_interactions(&resolved)?;
    let filtered = filter_records(
        &records,
        &options.page,
        &options.source,
        &options.target,
        &options.interaction_type,
    );
    let interactions = filtered
        .iter()
        .map(|record| interaction_record_json(record, options.include_raw))
        .collect::<Vec<_>>();

    Ok(json!({
        "schema": "powerbi-cli.report.interactions.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "filter": {
            "page": options.page,
            "source": options.source,
            "target": options.target,
            "type": options.interaction_type,
            "includeRaw": options.include_raw
        },
        "counts": interaction_counts(&filtered),
        "semantics": interaction_semantics(),
        "interactions": interactions,
        "next": [
            format!("powerbi-cli report interactions show --project {} --handle <interaction-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report pages list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": validation.warnings,
        "errors": validation.errors
    }))
}

fn show_interaction(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    require_interaction_selector(&options)?;
    let project = required_project(options.project.clone(), "report interactions show")?;
    let resolved = resolve_project(&project)?;
    let (records, validation) = list_report_interactions(&resolved)?;
    let record = find_interaction(&records, &options)?;
    let readback = format!(
        "powerbi-cli report interactions list --project {} --json",
        command_arg(&resolved.project_dir)
    );
    let page_readback = format!(
        "powerbi-cli report pages show --project {} --handle {} --json",
        command_arg(&resolved.project_dir),
        shell_arg(&record.page_handle)
    );
    let source_visual_readback = record.source_visual.as_ref().map(|visual| {
        format!(
            "powerbi-cli report visuals show --project {} --handle {} --json",
            command_arg(&resolved.project_dir),
            shell_arg(&visual.handle)
        )
    });
    let target_visual_readback = record.target_visual.as_ref().map(|visual| {
        format!(
            "powerbi-cli report visuals show --project {} --handle {} --json",
            command_arg(&resolved.project_dir),
            shell_arg(&visual.handle)
        )
    });
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );
    let mut next = vec![readback.clone(), page_readback.clone(), validate.clone()];
    if let Some(command) = source_visual_readback.as_ref() {
        next.push(command.clone());
    }
    if let Some(command) = target_visual_readback.as_ref()
        && source_visual_readback.as_ref() != Some(command)
    {
        next.push(command.clone());
    }

    Ok(json!({
        "schema": "powerbi-cli.report.interactions.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "interaction": interaction_record_json(record, options.include_raw),
        "readbackCommand": readback,
        "pageReadbackCommand": page_readback,
        "sourceVisualReadbackCommand": source_visual_readback,
        "targetVisualReadbackCommand": target_visual_readback,
        "validateCommand": validate,
        "next": next,
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
            "--source" => options.source = Some(take_value(args, &mut i, "--source")?),
            "--target" => options.target = Some(take_value(args, &mut i, "--target")?),
            "--type" | "--interaction-type" | "--interactionType" => {
                options.interaction_type = Some(parse_interaction_type(&take_value(
                    args, &mut i, "--type",
                )?)?);
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report interactions list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report interactions list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
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
            "--source" => options.source = Some(take_value(args, &mut i, "--source")?),
            "--target" => options.target = Some(take_value(args, &mut i, "--target")?),
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
                    "unknown report interactions show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report interactions show --project <project-dir-or.pbip> --handle <interaction-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report interactions show --project <project-dir-or.pbip> --handle <interaction-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_interaction_type(value: &str) -> CliResult<String> {
    let normalized = match value.to_ascii_lowercase().as_str() {
        "default" => "Default",
        "datafilter" | "data-filter" | "filter" => "DataFilter",
        "highlightfilter" | "highlight-filter" | "highlight" => "HighlightFilter",
        "nofilter" | "no-filter" | "none" | "disabled" => "NoFilter",
        other => {
            return Err(CliError::invalid_args(format!(
                "invalid report interaction type: {other}"
            ))
            .with_hint("Use Default, DataFilter, HighlightFilter, or NoFilter.")
            .with_suggested_command(
                "powerbi-cli report interactions list --project <project-dir-or.pbip> --type NoFilter --json",
            ));
        }
    };
    Ok(normalized.to_string())
}

fn filter_records<'a>(
    records: &'a [ReportInteractionRecord],
    page: &Option<String>,
    source: &Option<String>,
    target: &Option<String>,
    interaction_type: &Option<String>,
) -> Vec<&'a ReportInteractionRecord> {
    records
        .iter()
        .filter(|record| {
            page.as_ref()
                .is_none_or(|page| interaction_matches_page(record, page))
        })
        .filter(|record| {
            source
                .as_ref()
                .is_none_or(|source| interaction_matches_source(record, source))
        })
        .filter(|record| {
            target
                .as_ref()
                .is_none_or(|target| interaction_matches_target(record, target))
        })
        .filter(|record| {
            interaction_type
                .as_ref()
                .is_none_or(|value| record.interaction_type == *value)
        })
        .collect::<Vec<_>>()
}

fn find_interaction<'a>(
    records: &'a [ReportInteractionRecord],
    options: &ShowOptions,
) -> CliResult<&'a ReportInteractionRecord> {
    let matches = records
        .iter()
        .filter(|record| {
            if let Some(handle) = &options.handle {
                interaction_matches_handle(record, handle)
            } else {
                options
                    .page
                    .as_ref()
                    .is_some_and(|page| interaction_matches_page(record, page))
                    && options
                        .source
                        .as_ref()
                        .is_some_and(|source| interaction_matches_source(record, source))
                    && options
                        .target
                        .as_ref()
                        .is_some_and(|target| interaction_matches_target(record, target))
            }
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(*record),
        [] => Err(CliError::invalid_args("interaction not found")
            .with_hint("Use `report interactions list` to get stable interaction handles.")
            .with_suggested_command(
                "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
            )),
        _ => Err(CliError::invalid_args(
            "interaction selector matched multiple interactions",
        )
        .with_hint("Use the exact interaction handle returned by `report interactions list`.")
        .with_suggested_command(
            "powerbi-cli report interactions show --project <project-dir-or.pbip> --handle <interaction-handle> --json",
        )),
    }
}

fn interaction_counts(records: &[&ReportInteractionRecord]) -> Value {
    let mut by_type = BTreeMap::new();
    let mut pages = BTreeSet::new();
    for record in records {
        *by_type
            .entry(record.interaction_type.clone())
            .or_insert(0usize) += 1;
        pages.insert(record.page_handle.clone());
    }
    let by_type = by_type
        .into_iter()
        .map(|(key, value)| (key, Value::from(value)))
        .collect::<Map<_, _>>();
    json!({
        "interactions": records.len(),
        "pagesWithExplicitInteractions": pages.len(),
        "unsupported": records.iter().filter(|record| record.unsupported).count(),
        "staleVisualReferences": records.iter().filter(|record| record.source_visual.is_none() || record.target_visual.is_none()).count(),
        "byType": by_type
    })
}

fn require_interaction_selector(options: &ShowOptions) -> CliResult<()> {
    if options.handle.is_some()
        || (options.page.is_some() && options.source.is_some() && options.target.is_some())
    {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "report interactions show requires --handle or --page plus --source and --target",
    )
    .with_hint("Use `report interactions list` to get stable interaction handles.")
    .with_suggested_command(
        "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
    ))
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
                "Run `powerbi-cli --json capabilities --for \"report interactions\"` for exact usage.",
            )
            .with_suggested_command("powerbi-cli --json capabilities --for \"report interactions\"")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
