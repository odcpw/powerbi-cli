use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::pbir::{VisualSelector, find_visual, load_report_snapshot};
use crate::pbir_filters::filter_target;
use crate::pbir_slicers::{
    ReportSlicerRecord, is_slicer_visual_type, list_report_slicers,
    slicer_matches_handle_or_visual, slicer_record_json, slicer_state_summary_from_bindings,
};
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const SLICER_FILTER_POINTERS: [&str; 2] = ["/filterConfig/filters", "/filters"];

#[derive(Debug, Default)]
struct ClearSlicerOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    page: Option<String>,
    visual: Option<String>,
    confirm: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

#[derive(Debug, Clone)]
enum SlicerSelector {
    Handle { handle: String },
    PageVisual { page: String, visual: String },
}

#[derive(Debug, Clone)]
struct SlicerClearEdit {
    pointer: &'static str,
    before_count: usize,
    after_count: usize,
    removed: Vec<SlicerRemovedFilter>,
}

#[derive(Debug, Clone)]
struct SlicerRemovedFilter {
    ordinal: usize,
    value: Value,
}

struct SlicerClearPlan {
    file_json: Value,
    before_state: Value,
    after_state: Value,
    edits: Vec<SlicerClearEdit>,
}

pub(crate) fn clear_slicer(args: &[String]) -> CliResult<Value> {
    let options = parse_clear_args(args)?;
    let source_project = required_project(options.project.clone(), "report slicers clear")?;
    let mode = require_mode(options.mode, "report slicers clear")?;
    let selector = clear_selector(&options)?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, clear_slicer)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let record = resolve_slicer(&target_resolved, &selector)?;
    ensure_slicer_path_under_report(&target_resolved, &record)?;

    let confirm_token = clear_confirm_token(&record);
    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&confirm_token) {
        return Err(CliError::invalid_args(format!(
            "in-place slicer clear requires --confirm {confirm_token}"
        ))
        .with_hint("Run the same command with --dry-run first, then confirm the exact token.")
        .with_suggested_command(format!(
            "powerbi-cli report slicers clear --project {} {} --in-place --confirm {} --json",
            command_arg(&target_resolved.project_dir),
            selector_args_for_command(&record),
            shell_arg(&confirm_token)
        )));
    }

    let plan = clear_slicer_from_file(&record)?;
    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(record_path(&record)?, &plan.file_json)?;
    }

    slicer_clear_response(
        &target_resolved,
        mode,
        &record,
        &plan,
        &confirm_token,
        options.include_raw,
    )
}

fn parse_clear_args(args: &[String]) -> CliResult<ClearSlicerOptions> {
    let mut options = ClearSlicerOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" | "--slicer" => {
                options.visual = Some(take_value(args, &mut i, "--visual")?);
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
                    "report slicers clear",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report slicers clear",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report slicers clear",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report slicers clear flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report slicers clear\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report slicers clear\"",
                ));
            }
        }
    }
    Ok(options)
}

fn clear_selector(options: &ClearSlicerOptions) -> CliResult<SlicerSelector> {
    if let Some(handle) = options.handle.as_ref() {
        if options.page.is_some() || options.visual.is_some() {
            return Err(CliError::invalid_args(
                "report slicers clear --handle cannot be combined with --page or --visual",
            )
            .with_hint("Use the stable slicer handle alone, or use --page plus --visual.")
            .with_suggested_command(
                "powerbi-cli report slicers clear --project <project-dir-or.pbip> --handle <slicer-handle> --dry-run --json",
            ));
        }
        return Ok(SlicerSelector::Handle {
            handle: handle.clone(),
        });
    }

    match (&options.page, &options.visual) {
        (Some(page), Some(visual)) => Ok(SlicerSelector::PageVisual {
            page: page.clone(),
            visual: visual.clone(),
        }),
        (None, None) => Err(CliError::invalid_args(
            "report slicers clear requires --handle or --page plus --visual",
        )
        .with_hint("Use `report slicers list` to get stable slicer and visual handles.")
        .with_suggested_command(
            "powerbi-cli report slicers list --project <project-dir-or.pbip> --json",
        )),
        (Some(_), None) => Err(CliError::invalid_args(
            "report slicers clear requires --visual when --page is used",
        )
        .with_hint("Use page and visual together for a name-based selector.")
        .with_suggested_command(
            "powerbi-cli report slicers clear --project <project-dir-or.pbip> --page <page-name-or-handle> --visual <visual-name-or-title> --dry-run --json",
        )),
        (None, Some(_)) => Err(CliError::invalid_args(
            "report slicers clear requires --page when --visual is used",
        )
        .with_hint("A bare visual name can be ambiguous; use --handle for stable visual handles.")
        .with_suggested_command(
            "powerbi-cli report slicers clear --project <project-dir-or.pbip> --handle <slicer-or-visual-handle> --dry-run --json",
        )),
    }
}

fn resolve_slicer(
    resolved: &ResolvedProject,
    selector: &SlicerSelector,
) -> CliResult<ReportSlicerRecord> {
    let (records, _) = list_report_slicers(resolved)?;
    match selector {
        SlicerSelector::Handle { handle } => {
            let matches = records
                .iter()
                .filter(|record| slicer_matches_handle_or_visual(record, handle))
                .cloned()
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [record] => Ok(record.clone()),
                [] => {
                    reject_non_slicer_visual_handle(resolved, handle)?;
                    Err(CliError::invalid_args("slicer not found")
                        .with_hint("Use `report slicers list` to get stable slicer handles.")
                        .with_suggested_command(format!(
                            "powerbi-cli report slicers list --project {} --json",
                            command_arg(&resolved.project_dir)
                        )))
                }
                _ => Err(CliError::invalid_args("slicer selector matched multiple slicers")
                    .with_hint("Use the exact slicer handle returned by `report slicers list`.")
                    .with_suggested_command(format!(
                        "powerbi-cli report slicers clear --project {} --handle <slicer-handle> --dry-run --json",
                        command_arg(&resolved.project_dir)
                    ))),
            }
        }
        SlicerSelector::PageVisual { page, visual } => {
            let snapshot = load_report_snapshot(resolved)?;
            let visual_record = if visual.starts_with("visual:") {
                let visual_record = find_visual(
                    &snapshot.pages,
                    &VisualSelector {
                        handle: Some(visual.clone()),
                        ..VisualSelector::default()
                    },
                    "report slicers clear",
                )?;
                if !visual_page_matches(visual_record, page) {
                    return Err(CliError::invalid_args(format!(
                        "visual {} is not on page {page}",
                        visual_record.handle
                    ))
                    .with_hint(
                        "Use `report slicers list` to pair the slicer visual handle with its page.",
                    )
                    .with_suggested_command(format!(
                        "powerbi-cli report slicers list --project {} --json",
                        command_arg(&resolved.project_dir)
                    )));
                }
                visual_record
            } else {
                find_visual(
                    &snapshot.pages,
                    &VisualSelector {
                        page: Some(page.clone()),
                        visual: Some(visual.clone()),
                        ..VisualSelector::default()
                    },
                    "report slicers clear",
                )?
            };
            if !is_slicer_visual_type(&visual_record.visual_type) {
                return Err(CliError::invalid_args(format!(
                    "target visual is not a slicer: {} ({})",
                    visual_record.handle, visual_record.visual_type
                ))
                .with_hint("Use `report slicers list` to choose a slicer visual.")
                .with_suggested_command(format!(
                    "powerbi-cli report slicers list --project {} --json",
                    command_arg(&resolved.project_dir)
                )));
            }
            records
                .iter()
                .find(|record| record.visual_handle == visual_record.handle)
                .cloned()
                .ok_or_else(|| {
                    CliError::invalid_args("slicer not found")
                        .with_hint("Use `report slicers list` to get stable slicer handles.")
                        .with_suggested_command(format!(
                            "powerbi-cli report slicers list --project {} --json",
                            command_arg(&resolved.project_dir)
                        ))
                })
        }
    }
}

fn visual_page_matches(visual: &crate::pbir::VisualRecord, page: &str) -> bool {
    visual.page_handle == page
        || visual.page_name == page
        || visual.page_display_name == page
        || visual.page_name.eq_ignore_ascii_case(page)
        || visual.page_display_name.eq_ignore_ascii_case(page)
}

fn reject_non_slicer_visual_handle(resolved: &ResolvedProject, handle: &str) -> CliResult<()> {
    if !handle.starts_with("visual:") {
        return Ok(());
    }
    let snapshot = load_report_snapshot(resolved)?;
    match find_visual(
        &snapshot.pages,
        &VisualSelector {
            handle: Some(handle.to_string()),
            ..VisualSelector::default()
        },
        "report slicers clear",
    ) {
        Ok(visual) if !is_slicer_visual_type(&visual.visual_type) => {
            Err(CliError::invalid_args(format!(
                "target visual is not a slicer: {} ({})",
                visual.handle, visual.visual_type
            ))
            .with_hint("Use `report slicers list` to choose a slicer visual.")
            .with_suggested_command(format!(
                "powerbi-cli report slicers list --project {} --json",
                command_arg(&resolved.project_dir)
            )))
        }
        _ => Ok(()),
    }
}

fn clear_slicer_from_file(record: &ReportSlicerRecord) -> CliResult<SlicerClearPlan> {
    let path = record_path(record)?;
    let mut file_json = read_json_value(path)?;
    let before_state = slicer_state_summary_from_bindings(&record.bindings, Some(&file_json));
    let mut edits = Vec::new();

    for pointer in SLICER_FILTER_POINTERS {
        if let Some(array) = file_json.pointer_mut(pointer).and_then(Value::as_array_mut) {
            let before_count = array.len();
            let removed = remove_matching_slicer_filters(array, &record.bindings);
            edits.push(SlicerClearEdit {
                pointer,
                before_count,
                after_count: array.len(),
                removed,
            });
        }
    }

    let after_state = slicer_state_summary_from_bindings(&record.bindings, Some(&file_json));
    Ok(SlicerClearPlan {
        file_json,
        before_state,
        after_state,
        edits,
    })
}

fn remove_matching_slicer_filters(
    array: &mut Vec<Value>,
    bindings: &[Value],
) -> Vec<SlicerRemovedFilter> {
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for (ordinal, item) in array.drain(..).enumerate() {
        if filter_matches_slicer_binding(&item, bindings) {
            removed.push(SlicerRemovedFilter {
                ordinal,
                value: item,
            });
        } else {
            kept.push(item);
        }
    }
    *array = kept;
    removed
}

fn filter_matches_slicer_binding(filter: &Value, bindings: &[Value]) -> bool {
    let target = filter_target(filter);
    bindings
        .iter()
        .any(|binding| target_matches_binding(&target, binding))
}

fn target_matches_binding(target: &Value, binding: &Value) -> bool {
    let Some(kind) = target["kind"].as_str() else {
        return false;
    };
    if !string_eq(target["table"].as_str(), binding["table"].as_str()) {
        return false;
    }
    match kind {
        "column" => string_eq(
            target["column"]
                .as_str()
                .or_else(|| target["field"].as_str()),
            binding["column"]
                .as_str()
                .or_else(|| binding["field"].as_str()),
        ),
        "measure" => string_eq(
            target["measure"]
                .as_str()
                .or_else(|| target["field"].as_str()),
            binding["measure"]
                .as_str()
                .or_else(|| binding["field"].as_str()),
        ),
        _ => false,
    }
}

fn string_eq(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

fn slicer_clear_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    record: &ReportSlicerRecord,
    plan: &SlicerClearPlan,
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
    let project_arg = command_arg(&target_resolved.project_dir);
    let readback = format!(
        "powerbi-cli report slicers show --project {project_arg} --handle {} --json",
        shell_arg(&record.handle)
    );
    let visual_readback = format!(
        "powerbi-cli report visuals show --project {project_arg} --handle {} --json",
        shell_arg(&record.visual_handle)
    );
    let raw_review = dry_run.then(|| {
        format!(
            "powerbi-cli report slicers show --project {project_arg} --handle {} --include-raw --json",
            shell_arg(&record.handle)
        )
    });
    let wireframe = format!("powerbi-cli report wireframe export {project_arg} --json");
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let path = record_path(record)?;
    let changes = plan
        .edits
        .iter()
        .flat_map(|edit| {
            edit.removed.iter().map(|removed| {
                json!({
                    "kind": "pbir.slicerState",
                    "action": "clear",
                    "path": canonical_display(path),
                    "jsonPointer": format!("{}/{}", edit.pointer, removed.ordinal),
                    "parentJsonPointer": edit.pointer,
                    "ordinal": removed.ordinal,
                    "before": removed_filter_json(removed, include_raw),
                    "after": Value::Null
                })
            })
        })
        .collect::<Vec<_>>();
    let cleared_filter_entries = plan
        .edits
        .iter()
        .map(|edit| edit.removed.len())
        .sum::<usize>();
    let cleared_filter_config_filters = cleared_count_for_pointer(plan, "/filterConfig/filters");
    let cleared_legacy_filters = cleared_count_for_pointer(plan, "/filters");

    Ok(json!({
        "schema": "powerbi-cli.report.slicers.clearMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "clear",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": slicer_record_json(record, include_raw),
        "confirmToken": confirm_token,
        "counts": {
            "matchedSlicers": 1,
            "changedFiles": usize::from(cleared_filter_entries > 0),
            "clearedFilterEntries": cleared_filter_entries,
            "filterConfigFilters": cleared_filter_config_filters,
            "legacyFilters": cleared_legacy_filters,
            "stateArraysFound": plan.edits.len()
        },
        "slicerPlan": {
            "beforeState": plan.before_state,
            "afterState": plan.after_state,
            "arrayEdits": plan.edits.iter().map(|edit| json!({
                "jsonPointer": edit.pointer,
                "beforeCount": edit.before_count,
                "afterCount": edit.after_count,
                "clearedCount": edit.removed.len(),
                "removedOrdinals": edit.removed.iter().map(|removed| removed.ordinal).collect::<Vec<_>>()
            })).collect::<Vec<_>>(),
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
        "readbackCommand": readback,
        "visualReadbackCommand": visual_readback,
        "rawReviewCommand": raw_review,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, visual_readback, wireframe, inspect, validate],
    }))
}

fn removed_filter_json(removed: &SlicerRemovedFilter, include_raw: bool) -> Value {
    if include_raw {
        removed.value.clone()
    } else {
        json!({
            "name": removed.value["name"],
            "filterType": removed.value["type"],
            "target": filter_target(&removed.value)
        })
    }
}

fn cleared_count_for_pointer(plan: &SlicerClearPlan, pointer: &str) -> usize {
    plan.edits
        .iter()
        .find(|edit| edit.pointer == pointer)
        .map(|edit| edit.removed.len())
        .unwrap_or_default()
}

fn ensure_slicer_path_under_report(
    resolved: &ResolvedProject,
    record: &ReportSlicerRecord,
) -> CliResult<()> {
    let path = record_path(record)?;
    let file_name = path.file_name().and_then(|value| value.to_str());
    if !matches!(file_name, Some("visual.json")) {
        return Err(CliError::validation_failed(format!(
            "refusing to mutate slicer from unsupported file path: {}",
            path.display()
        )));
    }
    let report_abs = fs::canonicalize(&resolved.report_dir).map_err(|err| {
        CliError::unexpected(format!("resolve {}: {err}", resolved.report_dir.display()))
    })?;
    let path_abs = fs::canonicalize(path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?;
    if path_abs.starts_with(report_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to mutate slicer outside report directory: {}",
        path.display()
    ))
    .with_hint("Run `validate --strict` before mutating this report.")
    .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json"))
}

fn record_path(record: &ReportSlicerRecord) -> CliResult<&Path> {
    record.path.as_deref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "slicer {} has no backing visual.json path",
            record.handle
        ))
    })
}

fn clear_confirm_token(record: &ReportSlicerRecord) -> String {
    let state_count = record.state["filterConfigFilters"]
        .as_u64()
        .unwrap_or_default()
        + record.state["legacyFilters"].as_u64().unwrap_or_default();
    format!("clear:slicer:{}:{state_count}", record.handle)
}

fn selector_args_for_command(record: &ReportSlicerRecord) -> String {
    format!("--handle {}", shell_arg(&record.handle))
}
