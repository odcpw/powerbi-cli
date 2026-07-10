use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::pbir::{PageSelector, VisualSelector, find_page, find_visual, load_report_snapshot};
use crate::pbir_filters::{
    FilterOwner, FilterScope, ReportFilterRecord, filter_record_json, list_report_filters,
};
use crate::project_io::write_json_atomic;
use crate::report_filter_mutations::{
    ensure_filter_path_under_report, filter_array_pointer, filter_list_readback,
    find_filter_by_handle, owner_readback_command, verify_filter_array_origin,
    verify_filter_identity,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct ClearFilterOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    scope: Option<FilterScope>,
    page: Option<String>,
    visual: Option<String>,
    all: bool,
    confirm: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

#[derive(Debug, Clone)]
enum ClearSelector {
    Handle {
        handle: String,
    },
    Report,
    Page {
        requested: String,
    },
    Visual {
        page: Option<String>,
        visual: String,
    },
    All,
}

#[derive(Debug, Clone)]
struct ResolvedClearSelector {
    kind: &'static str,
    requested: Value,
    stable_id: String,
    page_handle: Option<String>,
    visual_handle: Option<String>,
    readback_command: String,
    owner_readback_command: String,
    raw_review_command: String,
}

#[derive(Debug, Clone)]
struct FilterClearChange {
    record: ReportFilterRecord,
    parent_pointer: String,
    ordinal: usize,
}

#[derive(Debug, Clone)]
struct FilterClearArrayEdit {
    path: PathBuf,
    parent_pointer: String,
    ordinals: Vec<usize>,
    array_before_count: usize,
    array_after_count: usize,
}

struct FilterClearPlan {
    file_writes: Vec<(PathBuf, Value)>,
    changes: Vec<FilterClearChange>,
    array_edits: Vec<FilterClearArrayEdit>,
}

pub(crate) fn clear_filters(args: &[String]) -> CliResult<Value> {
    let options = parse_clear_args(args)?;
    let source_project = required_project(options.project.clone(), "report filters clear")?;
    let mode = require_mode(options.mode, "report filters clear")?;
    let selector = clear_selector(&options)?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, clear_filters)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let (records, _) = list_report_filters(&target_resolved)?;
    let (resolved_selector, targets) = resolve_clear_targets(&target_resolved, &records, selector)?;
    for record in &targets {
        ensure_filter_path_under_report(&target_resolved, record)?;
    }

    let confirm_token = clear_confirm_token(&resolved_selector, targets.len());
    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&confirm_token) {
        return Err(CliError::invalid_args(format!(
            "in-place filter clear requires --confirm {confirm_token}"
        ))
        .with_hint("Run the same command with --dry-run first, then confirm the exact token.")
        .with_suggested_command(format!(
            "powerbi-cli report filters clear --project {} {} --in-place --confirm {} --json",
            command_arg(&target_resolved.project_dir),
            clear_selector_args_for_command(&resolved_selector),
            shell_arg(&confirm_token)
        )));
    }

    let plan = clear_filters_from_files(&targets)?;
    if !matches!(mode, MutationMode::DryRun) {
        for (path, value) in &plan.file_writes {
            write_json_atomic(path, value)?;
        }
    }

    clear_mutation_response(
        &target_resolved,
        mode,
        &resolved_selector,
        &targets,
        &plan,
        &confirm_token,
        options.include_raw,
    )
}

fn parse_clear_args(args: &[String]) -> CliResult<ClearFilterOptions> {
    let mut options = ClearFilterOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--scope" => {
                options.scope = Some(parse_clear_scope(&take_value(args, &mut i, "--scope")?)?);
            }
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" => options.visual = Some(take_value(args, &mut i, "--visual")?),
            "--all" => {
                options.all = true;
                i += 1;
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report filters clear",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report filters clear",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report filters clear",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report filters clear flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report filters clear\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report filters clear\"",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_clear_scope(value: &str) -> CliResult<FilterScope> {
    match value {
        "report" => Ok(FilterScope::Report),
        "page" => Ok(FilterScope::Page),
        "visual" => Ok(FilterScope::Visual),
        "all" => Err(CliError::invalid_args(
            "report filters clear uses --all instead of --scope all",
        )
        .with_hint("Bulk clearing is guarded behind an explicit --all flag.")
        .with_suggested_command(
            "powerbi-cli report filters clear --project <project-dir-or.pbip> --all --dry-run --json",
        )),
        other => Err(CliError::invalid_args(format!(
            "invalid report filters clear scope: {other}"
        ))
        .with_hint("Use --scope report, --scope page, --scope visual, or explicit --all.")
        .with_suggested_command(
            "powerbi-cli report filters clear --project <project-dir-or.pbip> --scope report --dry-run --json",
        )),
    }
}

fn clear_selector(options: &ClearFilterOptions) -> CliResult<ClearSelector> {
    if options.all
        && (options.handle.is_some()
            || options.scope.is_some()
            || options.page.is_some()
            || options.visual.is_some())
    {
        return Err(CliError::invalid_args(
            "report filters clear --all cannot be combined with another selector",
        )
        .with_hint("Use --all by itself, or target one report/page/visual/filter handle.")
        .with_suggested_command(
            "powerbi-cli report filters clear --project <project-dir-or.pbip> --all --dry-run --json",
        ));
    }
    if let Some(handle) = options.handle.as_ref() {
        if options.scope.is_some() || options.page.is_some() || options.visual.is_some() {
            return Err(CliError::invalid_args(
                "report filters clear --handle cannot be combined with --scope, --page, or --visual",
            )
            .with_hint("Use the filter handle alone for a single-filter clear.")
            .with_suggested_command(
                "powerbi-cli report filters clear --project <project-dir-or.pbip> --handle <filter-handle> --dry-run --json",
            ));
        }
        return Ok(ClearSelector::Handle {
            handle: handle.clone(),
        });
    }
    if options.all {
        return Ok(ClearSelector::All);
    }

    match options.scope {
        Some(FilterScope::Report) => {
            if options.page.is_some() || options.visual.is_some() {
                return Err(CliError::invalid_args(
                    "report filters clear --scope report cannot be combined with --page or --visual",
                )
                .with_hint("Report-scope clear targets only report-level filters.")
                .with_suggested_command(
                    "powerbi-cli report filters clear --project <project-dir-or.pbip> --scope report --dry-run --json",
                ));
            }
            Ok(ClearSelector::Report)
        }
        Some(FilterScope::Page) => {
            if options.visual.is_some() {
                return Err(CliError::invalid_args(
                    "report filters clear --scope page cannot be combined with --visual",
                )
                .with_hint("Use --scope visual with --visual when clearing visual filters.")
                .with_suggested_command(
                    "powerbi-cli report filters clear --project <project-dir-or.pbip> --page <page-handle> --visual <visual-name-or-handle> --dry-run --json",
                ));
            }
            let page = options.page.clone().ok_or_else(|| {
                CliError::invalid_args("report filters clear --scope page requires --page")
                    .with_hint("Use `report pages list` to get stable page handles.")
                    .with_suggested_command(
                        "powerbi-cli report filters clear --project <project-dir-or.pbip> --page <page-handle> --dry-run --json",
                    )
            })?;
            Ok(ClearSelector::Page { requested: page })
        }
        Some(FilterScope::Visual) => {
            let visual = options.visual.clone().ok_or_else(|| {
                CliError::invalid_args("report filters clear --scope visual requires --visual")
                    .with_hint("Use a full visual handle, or combine --page and --visual name.")
                    .with_suggested_command(
                        "powerbi-cli report filters clear --project <project-dir-or.pbip> --page <page-handle> --visual <visual-name-or-handle> --dry-run --json",
                    )
            })?;
            if !visual.starts_with("visual:") && options.page.is_none() {
                return Err(visual_name_requires_page("report filters clear"));
            }
            Ok(ClearSelector::Visual {
                page: options.page.clone(),
                visual,
            })
        }
        Some(FilterScope::All) => unreachable!("parse_clear_scope rejects all"),
        None => match (options.page.clone(), options.visual.clone()) {
            (Some(page), Some(visual)) => Ok(ClearSelector::Visual {
                page: Some(page),
                visual,
            }),
            (Some(page), None) => Ok(ClearSelector::Page { requested: page }),
            (None, Some(visual)) if visual.starts_with("visual:") => {
                Ok(ClearSelector::Visual { page: None, visual })
            }
            (None, Some(_)) => Err(visual_name_requires_page("report filters clear")),
            (None, None) => Err(CliError::invalid_args(
                "report filters clear requires --handle, --scope report, --page, --visual, or --all",
            )
            .with_hint("Start with a dry-run against one exact owner; use --all only for an explicit full clear.")
            .with_suggested_command(
                "powerbi-cli report filters clear --project <project-dir-or.pbip> --page <page-handle> --dry-run --json",
            )),
        },
    }
}

fn visual_name_requires_page(command: &str) -> CliError {
    CliError::invalid_args(format!(
        "{command} requires --page when --visual is not a full visual handle"
    ))
    .with_hint("Pass a full visual handle or combine --page <page> --visual <visual>.")
    .with_suggested_command(format!(
        "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-handle> --visual <visual-name> --dry-run --json"
    ))
}

fn resolve_clear_targets(
    resolved: &ResolvedProject,
    records: &[ReportFilterRecord],
    selector: ClearSelector,
) -> CliResult<(ResolvedClearSelector, Vec<ReportFilterRecord>)> {
    let project = command_arg(&resolved.project_dir);
    match selector {
        ClearSelector::Handle { handle } => {
            let record = find_filter_by_handle(resolved, &handle)?;
            let readback = filter_list_readback(&record, &resolved.project_dir);
            let owner_readback = owner_readback_command(&record, &resolved.project_dir);
            let raw_review = format!(
                "powerbi-cli report filters show --project {project} --handle {} --include-raw --json",
                shell_arg(&record.handle)
            );
            Ok((
                ResolvedClearSelector {
                    kind: "handle",
                    requested: json!({ "handle": handle }),
                    stable_id: record.handle.clone(),
                    page_handle: None,
                    visual_handle: None,
                    readback_command: readback,
                    owner_readback_command: owner_readback,
                    raw_review_command: raw_review,
                },
                vec![record],
            ))
        }
        ClearSelector::Report => {
            let targets = records
                .iter()
                .filter(|record| record.scope == FilterScope::Report)
                .cloned()
                .collect::<Vec<_>>();
            Ok((
                ResolvedClearSelector {
                    kind: "report",
                    requested: json!({ "scope": "report" }),
                    stable_id: "report:main".to_string(),
                    page_handle: None,
                    visual_handle: None,
                    readback_command: format!(
                        "powerbi-cli report filters list --project {project} --scope report --json"
                    ),
                    owner_readback_command: format!(
                        "powerbi-cli report wireframe export {project} --json"
                    ),
                    raw_review_command: format!(
                        "powerbi-cli report filters list --project {project} --scope report --include-raw --json"
                    ),
                },
                targets,
            ))
        }
        ClearSelector::Page { requested } => {
            let snapshot = load_report_snapshot(resolved)?;
            let page_selector = PageSelector {
                handle: requested.starts_with("page:").then(|| requested.clone()),
                name: (!requested.starts_with("page:")).then(|| requested.clone()),
            };
            let page = find_page(&snapshot.pages, &page_selector, "report filters clear")?;
            let targets = records
                .iter()
                .filter(|record| {
                    record.scope == FilterScope::Page
                        && matches!(
                            &record.owner,
                            FilterOwner::Page { handle, .. } if handle == &page.handle
                        )
                })
                .cloned()
                .collect::<Vec<_>>();
            Ok((
                ResolvedClearSelector {
                    kind: "page",
                    requested: json!({
                        "page": requested,
                        "pageHandle": page.handle,
                        "pageName": page.name,
                        "pageDisplayName": page.display_name
                    }),
                    stable_id: page.handle.clone(),
                    page_handle: Some(page.handle.clone()),
                    visual_handle: None,
                    readback_command: format!(
                        "powerbi-cli report filters list --project {project} --scope page --page {} --json",
                        shell_arg(&page.handle)
                    ),
                    owner_readback_command: format!(
                        "powerbi-cli report pages show --project {project} --handle {} --json",
                        shell_arg(&page.handle)
                    ),
                    raw_review_command: format!(
                        "powerbi-cli report filters list --project {project} --scope page --page {} --include-raw --json",
                        shell_arg(&page.handle)
                    ),
                },
                targets,
            ))
        }
        ClearSelector::Visual { page, visual } => {
            let snapshot = load_report_snapshot(resolved)?;
            let visual_selector = if visual.starts_with("visual:") {
                VisualSelector {
                    handle: Some(visual.clone()),
                    page: None,
                    visual: None,
                }
            } else {
                VisualSelector {
                    handle: None,
                    page: page.clone(),
                    visual: Some(visual.clone()),
                }
            };
            let visual_record =
                find_visual(&snapshot.pages, &visual_selector, "report filters clear")?;
            if let Some(page_request) = page.as_ref() {
                let page_selector = PageSelector {
                    handle: page_request
                        .starts_with("page:")
                        .then(|| page_request.clone()),
                    name: (!page_request.starts_with("page:")).then(|| page_request.clone()),
                };
                let page_record =
                    find_page(&snapshot.pages, &page_selector, "report filters clear")?;
                if page_record.handle != visual_record.page_handle {
                    return Err(CliError::invalid_args(
                        "visual handle does not belong to the selected page",
                    )
                    .with_hint("Use the visual handle by itself, or pass the visual name with its page.")
                    .with_suggested_command(
                        "powerbi-cli report filters clear --project <project-dir-or.pbip> --visual <visual-handle> --dry-run --json",
                    ));
                }
            }
            let targets = records
                .iter()
                .filter(|record| {
                    record.scope == FilterScope::Visual
                        && matches!(
                            &record.owner,
                            FilterOwner::Visual { handle, .. } if handle == &visual_record.handle
                        )
                })
                .cloned()
                .collect::<Vec<_>>();
            Ok((
                ResolvedClearSelector {
                    kind: "visual",
                    requested: json!({
                        "page": page,
                        "visual": visual,
                        "pageHandle": visual_record.page_handle,
                        "visualHandle": visual_record.handle,
                        "visualName": visual_record.name,
                        "visualTitle": visual_record.title
                    }),
                    stable_id: visual_record.handle.clone(),
                    page_handle: Some(visual_record.page_handle.clone()),
                    visual_handle: Some(visual_record.handle.clone()),
                    readback_command: format!(
                        "powerbi-cli report filters list --project {project} --scope visual --visual {} --json",
                        shell_arg(&visual_record.handle)
                    ),
                    owner_readback_command: format!(
                        "powerbi-cli report visuals show --project {project} --handle {} --json",
                        shell_arg(&visual_record.handle)
                    ),
                    raw_review_command: format!(
                        "powerbi-cli report filters list --project {project} --scope visual --visual {} --include-raw --json",
                        shell_arg(&visual_record.handle)
                    ),
                },
                targets,
            ))
        }
        ClearSelector::All => {
            let targets = records.to_vec();
            Ok((
                ResolvedClearSelector {
                    kind: "all",
                    requested: json!({ "all": true }),
                    stable_id: "all".to_string(),
                    page_handle: None,
                    visual_handle: None,
                    readback_command: format!(
                        "powerbi-cli report filters list --project {project} --json"
                    ),
                    owner_readback_command: format!(
                        "powerbi-cli report wireframe export {project} --json"
                    ),
                    raw_review_command: format!(
                        "powerbi-cli report filters list --project {project} --include-raw --json"
                    ),
                },
                targets,
            ))
        }
    }
}

fn clear_filters_from_files(records: &[ReportFilterRecord]) -> CliResult<FilterClearPlan> {
    let mut by_path: BTreeMap<PathBuf, BTreeMap<String, Vec<ReportFilterRecord>>> = BTreeMap::new();
    for record in records {
        let (parent_pointer, _) = filter_array_pointer(&record.json_pointer)?;
        by_path
            .entry(record.path.clone())
            .or_default()
            .entry(parent_pointer)
            .or_default()
            .push(record.clone());
    }

    let mut file_writes = Vec::new();
    let mut changes = Vec::new();
    let mut array_edits = Vec::new();

    for (path, by_pointer) in by_path {
        let mut file_json = read_json_value(&path)?;
        for (parent_pointer, group) in by_pointer {
            for record in &group {
                verify_filter_array_origin(record, &parent_pointer)?;
            }
            let mut ordinals = group
                .iter()
                .map(|record| {
                    filter_array_pointer(&record.json_pointer).map(|(_, ordinal)| ordinal)
                })
                .collect::<CliResult<Vec<_>>>()?;
            ordinals.sort_unstable();
            let unique_ordinals = ordinals.iter().copied().collect::<BTreeSet<_>>();
            let array_before_count;
            let array_after_count;
            {
                let items = file_json
                    .pointer_mut(&parent_pointer)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| {
                        CliError::validation_failed(format!(
                            "{} filter array is missing or not an array at {parent_pointer}",
                            path.display()
                        ))
                    })?;
                array_before_count = items.len();
                for ordinal in unique_ordinals.iter().rev() {
                    if *ordinal >= items.len() {
                        return Err(CliError::validation_failed(format!(
                            "{} filter index {ordinal} is outside array {parent_pointer}",
                            path.display()
                        )));
                    }
                    let record = group
                        .iter()
                        .find(|record| {
                            filter_array_pointer(&record.json_pointer)
                                .is_ok_and(|(_, record_ordinal)| record_ordinal == *ordinal)
                        })
                        .expect("group was built from the selected ordinals");
                    verify_filter_identity(record, &items[*ordinal])?;
                    items.remove(*ordinal);
                }
                array_after_count = items.len();
            }
            for record in group {
                let (_, ordinal) = filter_array_pointer(&record.json_pointer)?;
                changes.push(FilterClearChange {
                    record,
                    parent_pointer: parent_pointer.clone(),
                    ordinal,
                });
            }
            array_edits.push(FilterClearArrayEdit {
                path: path.clone(),
                parent_pointer,
                ordinals: unique_ordinals.into_iter().collect(),
                array_before_count,
                array_after_count,
            });
        }
        file_writes.push((path, file_json));
    }

    Ok(FilterClearPlan {
        file_writes,
        changes,
        array_edits,
    })
}

fn clear_mutation_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    selector: &ResolvedClearSelector,
    targets: &[ReportFilterRecord],
    plan: &FilterClearPlan,
    confirm_token: &str,
    include_raw: bool,
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
    let target_json = targets
        .iter()
        .map(|record| filter_record_json(record, include_raw))
        .collect::<Vec<_>>();
    let changes = plan
        .changes
        .iter()
        .map(|change| {
            json!({
                "kind": "pbir.filter",
                "action": "clear",
                "path": canonical_display(&change.record.path),
                "handle": change.record.handle,
                "jsonPointer": change.record.json_pointer,
                "parentJsonPointer": change.parent_pointer,
                "ordinal": change.ordinal,
                "before": if include_raw {
                    change.record.raw.clone()
                } else {
                    filter_record_json(&change.record, false)
                },
                "after": Value::Null
            })
        })
        .collect::<Vec<_>>();
    let array_edits = plan
        .array_edits
        .iter()
        .map(|edit| {
            json!({
                "path": canonical_display(&edit.path),
                "parentJsonPointer": edit.parent_pointer,
                "ordinals": edit.ordinals,
                "arrayBeforeCount": edit.array_before_count,
                "arrayAfterCount": edit.array_after_count
            })
        })
        .collect::<Vec<_>>();
    let raw_review = dry_run.then(|| selector.raw_review_command.clone());

    Ok(json!({
        "schema": "powerbi-cli.report.filters.clearMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "clear",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "selector": {
            "kind": selector.kind,
            "requested": selector.requested,
            "stableId": selector.stable_id,
            "pageHandle": selector.page_handle,
            "visualHandle": selector.visual_handle
        },
        "confirmToken": confirm_token,
        "counts": filter_clear_counts(targets, plan),
        "targets": target_json,
        "filterPlan": {
            "before": target_json,
            "after": [],
            "arrayEdits": array_edits,
            "rawBeforeIncluded": include_raw
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
        "readbackCommand": selector.readback_command,
        "ownerReadbackCommand": selector.owner_readback_command,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "rawReviewCommand": raw_review,
        "next": [selector.readback_command, selector.owner_readback_command, wireframe, inspect, validate],
    }))
}

fn filter_clear_counts(targets: &[ReportFilterRecord], plan: &FilterClearPlan) -> Value {
    json!({
        "matchedFilters": targets.len(),
        "clearedFilters": targets.len(),
        "reportFilters": targets.iter().filter(|record| record.scope == FilterScope::Report).count(),
        "pageFilters": targets.iter().filter(|record| record.scope == FilterScope::Page).count(),
        "visualFilters": targets.iter().filter(|record| record.scope == FilterScope::Visual).count(),
        "unsupported": targets.iter().filter(|record| record.unsupported).count(),
        "possibleDataValueFilters": targets.iter().filter(|record| record.may_contain_data_values).count(),
        "filesChanged": plan.file_writes.len(),
        "arrayEdits": plan.array_edits.len()
    })
}

fn clear_confirm_token(selector: &ResolvedClearSelector, count: usize) -> String {
    format!(
        "clear:filters:{}:{}:{count}",
        selector.kind, selector.stable_id
    )
}

fn clear_selector_args_for_command(selector: &ResolvedClearSelector) -> String {
    match selector.kind {
        "handle" => format!("--handle {}", shell_arg(&selector.stable_id)),
        "report" => "--scope report".to_string(),
        "page" => format!("--page {}", shell_arg(&selector.stable_id)),
        "visual" => format!("--visual {}", shell_arg(&selector.stable_id)),
        "all" => "--all".to_string(),
        _ => "--dry-run".to_string(),
    }
}
