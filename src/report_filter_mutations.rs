use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::pbir_filters::{
    FilterArrayOrigin, FilterHandleIdentity, FilterOwner, ReportFilterRecord, filter_fingerprint,
    filter_record_json, list_report_filters, select_filter_by_handle,
};
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct DeleteFilterOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    confirm: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

struct FilterDeletePlan {
    file_json: Value,
    before: Value,
    removed: Value,
    parent_pointer: String,
    ordinal: usize,
}

pub(crate) fn delete_filter(args: &[String]) -> CliResult<Value> {
    let options = parse_delete_args(args)?;
    let source_project = required_project(options.project.clone(), "report filters delete")?;
    let handle = options.handle.clone().ok_or_else(|| {
        CliError::invalid_args("report filters delete requires --handle <filter-handle>")
            .with_hint("Use `report filters list` to get stable filter handles.")
            .with_suggested_command(
                "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
            )
    })?;
    let mode = require_mode(options.mode, "report filters delete")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, delete_filter)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let record = find_filter_by_handle(&target_resolved, &handle)?;

    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&record.handle) {
        return Err(CliError::invalid_args(format!(
            "in-place filter deletion requires --confirm {}",
            record.handle
        ))
        .with_hint("Run the same command with --dry-run first, then confirm the exact filter handle.")
        .with_suggested_command(format!(
            "powerbi-cli report filters delete --project {} --handle {} --in-place --confirm {} --json",
            command_arg(&target_resolved.project_dir),
            shell_arg(&record.handle),
            shell_arg(&record.handle)
        )));
    }

    ensure_filter_path_under_report(&target_resolved, &record)?;
    let plan = delete_filter_from_file(&record)?;
    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&record.path, &plan.file_json)?;
    }
    mutation_response(&target_resolved, mode, &record, &plan, options.include_raw)
}

fn parse_delete_args(args: &[String]) -> CliResult<DeleteFilterOptions> {
    let mut options = DeleteFilterOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report filters delete",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report filters delete",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report filters delete",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report filters delete flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report filters delete\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report filters delete\"",
                ));
            }
        }
    }
    Ok(options)
}

pub(crate) fn find_filter_by_handle(
    resolved: &ResolvedProject,
    handle: &str,
) -> CliResult<ReportFilterRecord> {
    let (records, _) = list_report_filters(resolved)?;
    select_filter_by_handle(&records, handle, &resolved.project_dir, true).cloned()
}

fn delete_filter_from_file(record: &ReportFilterRecord) -> CliResult<FilterDeletePlan> {
    let mut file_json = read_json_value(&record.path)?;
    let (parent_pointer, ordinal) = filter_array_pointer(&record.json_pointer)?;
    verify_filter_array_origin(record, &parent_pointer)?;
    let items = file_json
        .pointer_mut(&parent_pointer)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} filter array is missing or not an array at {parent_pointer}",
                record.path.display()
            ))
        })?;
    if ordinal >= items.len() {
        return Err(CliError::validation_failed(format!(
            "{} filter index {ordinal} is outside array {parent_pointer}",
            record.path.display()
        )));
    }
    verify_filter_identity(record, &items[ordinal])?;
    let before = items.clone();
    let removed = items.remove(ordinal);
    Ok(FilterDeletePlan {
        file_json,
        before: Value::Array(before),
        removed,
        parent_pointer,
        ordinal,
    })
}

pub(crate) fn verify_filter_identity(
    record: &ReportFilterRecord,
    current: &Value,
) -> CliResult<()> {
    let identity_matches = match record.handle_identity {
        FilterHandleIdentity::Name => current["name"].as_str() == record.name.as_deref(),
        FilterHandleIdentity::Fingerprint => filter_fingerprint(current) == record.fingerprint,
    };
    if identity_matches {
        return Ok(());
    }

    Err(CliError::invalid_args(format!(
        "stale filter handle: identity no longer matches {} at {}",
        record.handle, record.json_pointer
    ))
    .with_hint(
        "The filter array changed after this handle was resolved. Re-run `report filters list` and retry with the current handle.",
    )
    .with_suggested_command(
        "powerbi-cli report filters list --project <project-dir-or.pbip> --include-raw --json",
    ))
}

pub(crate) fn filter_array_pointer(json_pointer: &str) -> CliResult<(String, usize)> {
    let Some((parent, index)) = json_pointer.rsplit_once('/') else {
        return Err(CliError::validation_failed(format!(
            "filter JSON pointer does not include an array index: {json_pointer}"
        )));
    };
    if parent != "/filterConfig/filters" && parent != "/filters" {
        return Err(CliError::validation_failed(format!(
            "filter deletion only supports known filter arrays, got {parent}"
        ))
        .with_hint("Use `report filters list` to inspect supported filter handles.")
        .with_suggested_command(
            "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
        ));
    }
    let ordinal = index.parse::<usize>().map_err(|_| {
        CliError::validation_failed(format!(
            "filter JSON pointer index is not numeric: {json_pointer}"
        ))
    })?;
    Ok((parent.to_string(), ordinal))
}

pub(crate) fn verify_filter_array_origin(
    record: &ReportFilterRecord,
    parent_pointer: &str,
) -> CliResult<()> {
    let expected = match record.array_origin {
        FilterArrayOrigin::FilterConfig => "/filterConfig/filters",
        FilterArrayOrigin::Legacy => "/filters",
    };
    if parent_pointer == expected {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "stale filter handle: array origin changed for {}",
        record.handle
    ))
    .with_hint("Re-run `report filters list` and use the current handle."))
}

fn mutation_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    record: &ReportFilterRecord,
    plan: &FilterDeletePlan,
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
    let readback = filter_list_readback(record, &target_resolved.project_dir);
    let owner_readback = owner_readback_command(record, &target_resolved.project_dir);
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
    let before = filter_record_json(record, include_raw);
    let raw_review = dry_run.then(|| {
        format!(
            "powerbi-cli report filters show --project {} --handle {} --include-raw --json",
            project_arg,
            shell_arg(&record.handle)
        )
    });

    Ok(json!({
        "schema": "powerbi-cli.report.filters.deleteMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "delete",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": before,
        "filterPlan": {
            "before": before,
            "after": Value::Null,
            "arrayBeforeCount": plan.before.as_array().map(Vec::len).unwrap_or_default(),
            "arrayAfterCount": plan.before.as_array().map(Vec::len).unwrap_or_default().saturating_sub(1),
            "rawBeforeIncluded": include_raw
        },
        "changes": [{
            "kind": "pbir.filter",
            "action": "delete",
            "path": canonical_display(&record.path),
            "jsonPointer": record.json_pointer,
            "parentJsonPointer": plan.parent_pointer,
            "ordinal": plan.ordinal,
            "before": if include_raw { plan.removed.clone() } else { filter_record_json(record, false) },
            "after": Value::Null
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
        "ownerReadbackCommand": owner_readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "rawReviewCommand": raw_review,
        "next": [readback, owner_readback, wireframe, inspect, validate],
    }))
}

pub(crate) fn ensure_filter_path_under_report(
    resolved: &ResolvedProject,
    record: &ReportFilterRecord,
) -> CliResult<()> {
    let file_name = record.path.file_name().and_then(|value| value.to_str());
    if !matches!(file_name, Some("report.json" | "page.json" | "visual.json")) {
        return Err(CliError::validation_failed(format!(
            "refusing to mutate filter from unsupported file path: {}",
            record.path.display()
        )));
    }
    let report_abs = fs::canonicalize(&resolved.report_dir).map_err(|err| {
        CliError::unexpected(format!("resolve {}: {err}", resolved.report_dir.display()))
    })?;
    let path_abs = fs::canonicalize(&record.path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", record.path.display())))?;
    if path_abs.starts_with(report_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to mutate filter outside report directory: {}",
        record.path.display()
    ))
    .with_hint("Run `validate --strict` before mutating this report.")
    .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json"))
}

pub(crate) fn filter_list_readback(record: &ReportFilterRecord, project_dir: &Path) -> String {
    let project = command_arg(project_dir);
    match &record.owner {
        FilterOwner::Report { .. } => {
            format!("powerbi-cli report filters list --project {project} --scope report --json")
        }
        FilterOwner::Page { handle, .. } => format!(
            "powerbi-cli report filters list --project {project} --scope page --page {} --json",
            shell_arg(handle)
        ),
        FilterOwner::Visual { handle, .. } => format!(
            "powerbi-cli report filters list --project {project} --scope visual --visual {} --json",
            shell_arg(handle)
        ),
    }
}

pub(crate) fn owner_readback_command(record: &ReportFilterRecord, project_dir: &Path) -> String {
    let project = command_arg(project_dir);
    match &record.owner {
        FilterOwner::Report { .. } => {
            format!("powerbi-cli report wireframe export {project} --json")
        }
        FilterOwner::Page { handle, .. } => format!(
            "powerbi-cli report pages show --project {project} --handle {} --json",
            shell_arg(handle)
        ),
        FilterOwner::Visual { handle, .. } => format!(
            "powerbi-cli report visuals show --project {project} --handle {} --json",
            shell_arg(handle)
        ),
    }
}
