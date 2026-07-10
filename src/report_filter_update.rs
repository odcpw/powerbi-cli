use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::pbir_filters::{
    ReportFilterRecord, filter_fingerprint, filter_record_json, refreshed_filter_handle,
};
use crate::project_io::write_json_atomic;
use crate::report_filter_mutations::{
    ensure_filter_path_under_report, filter_array_pointer, filter_list_readback,
    find_filter_by_handle, owner_readback_command, verify_filter_array_origin,
    verify_filter_identity,
};
use crate::report_filter_shapes::{
    categorical_value_rows, parse_values_json, resolve_filter_column, validate_categorical_values,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct UpdateFilterOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    display_name: Option<String>,
    values: Vec<Value>,
    values_supplied: bool,
    condition_type: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

struct FilterUpdatePlan {
    file_json: Value,
    before: Value,
    after: Value,
    parent_pointer: String,
    ordinal: usize,
    changed: bool,
    after_handle: String,
}

pub(crate) fn update_filter(args: &[String]) -> CliResult<Value> {
    let options = parse_update_args(args)?;
    let source_project = required_project(options.project.clone(), "report filters update")?;
    let handle = options.handle.as_deref().ok_or_else(|| {
        CliError::invalid_args("report filters update requires --handle <filter-handle>")
            .with_hint("Use `report filters list` to get stable filter handles.")
            .with_suggested_command(
                "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
            )
    })?;
    let mode = require_mode(options.mode, "report filters update")?;
    if options.display_name.is_none() && !options.values_supplied {
        return Err(CliError::invalid_args(
            "report filters update requires --display-name or replacement categorical values",
        )
        .with_hint(
            "Use --value, --value-json, or --values-json to replace all categorical values.",
        ));
    }

    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, update_filter)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let record = find_filter_by_handle(&target_resolved, handle)?;
    ensure_filter_path_under_report(&target_resolved, &record)?;
    check_requested_type(&record, options.condition_type.as_deref())?;
    let plan = build_update_plan(&target_resolved, &record, &options)?;
    if !matches!(mode, MutationMode::DryRun) && plan.changed {
        write_json_atomic(&record.path, &plan.file_json)?;
    }
    update_filter_response(&target_resolved, mode, &record, &plan, options.include_raw)
}

fn parse_update_args(args: &[String]) -> CliResult<UpdateFilterOptions> {
    let mut options = UpdateFilterOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--display-name" | "--displayName" => {
                options.display_name = Some(take_value(args, &mut i, "--display-name")?);
            }
            "--condition-type" | "--conditionType" => {
                options.condition_type = Some(take_value(args, &mut i, "--condition-type")?);
            }
            "--value" => {
                options.values_supplied = true;
                options
                    .values
                    .push(Value::String(take_value(args, &mut i, "--value")?));
            }
            "--value-json" | "--valueJson" => {
                let text = take_value(args, &mut i, "--value-json")?;
                let value = serde_json::from_str(&text).map_err(|err| {
                    CliError::invalid_args(format!("parse --value-json: {err}"))
                        .with_hint("Pass one JSON string, number, or boolean.")
                })?;
                options.values_supplied = true;
                options.values.push(value);
            }
            "--values-json" | "--valuesJson" => {
                let values = parse_values_json(&take_value(args, &mut i, "--values-json")?)?;
                options.values_supplied = true;
                options.values.extend(values);
            }
            "--min" | "--max" | "--top" | "--bottom" | "--by" | "--relative" | "--unit"
            | "--span" => {
                let flag = args[i].clone();
                let _ = take_value(args, &mut i, &flag)?;
                return Err(unsupported_type_update(&flag));
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report filters update",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report filters update",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report filters update",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report filters update flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report filters update\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report filters update\"",
                ));
            }
        }
    }
    Ok(options)
}

fn check_requested_type(record: &ReportFilterRecord, requested: Option<&str>) -> CliResult<()> {
    let Some(requested) = requested else {
        return Ok(());
    };
    let requested_type = match requested.to_ascii_lowercase().replace('_', "-").as_str() {
        "categorical" => "Categorical",
        "range" | "numeric-range" | "advanced" => "Advanced",
        "topn" | "top-n" => "TopN",
        "relative-date" | "relativedate" => "RelativeDate",
        other => {
            return Err(CliError::unsupported_feature(format!(
                "report filters update cannot change filter type to {other}"
            ))
            .with_hint("Filter type changes are explicitly unsupported; delete and add a supported filter after reviewing both operations."));
        }
    };
    if record.filter_type != requested_type {
        return Err(CliError::unsupported_feature(format!(
            "report filters update refuses type change from {} to {requested_type}",
            record.filter_type
        ))
        .with_hint("This command preserves filter type. Use display-name-only update, or delete and add a supported replacement after review."));
    }
    Ok(())
}

fn unsupported_type_update(flag: &str) -> CliError {
    CliError::unsupported_feature(format!(
        "report filters update does not change filter conditions with {flag}"
    ))
    .with_hint("Update can replace categorical values or change display name without changing filter type. Delete and add a reviewed replacement for other condition changes.")
}

fn build_update_plan(
    resolved: &ResolvedProject,
    record: &ReportFilterRecord,
    options: &UpdateFilterOptions,
) -> CliResult<FilterUpdatePlan> {
    let mut after = record.raw.clone();
    if let Some(display_name) = options.display_name.as_ref() {
        after["displayName"] = Value::String(display_name.clone());
    }
    if options.values_supplied {
        replace_categorical_values(resolved, record, &mut after, &options.values)?;
    }

    let mut file_json = read_json_value(&record.path)?;
    let (parent_pointer, ordinal) = filter_array_pointer(&record.json_pointer)?;
    verify_filter_array_origin(record, &parent_pointer)?;
    let slot = file_json.pointer_mut(&record.json_pointer).ok_or_else(|| {
        CliError::validation_failed(format!(
            "{} is missing filter at {}",
            record.path.display(),
            record.json_pointer
        ))
    })?;
    verify_filter_identity(record, slot)?;
    *slot = after.clone();
    Ok(FilterUpdatePlan {
        file_json,
        changed: record.raw != after,
        after_handle: refreshed_filter_handle(record, &after),
        before: record.raw.clone(),
        after,
        parent_pointer,
        ordinal,
    })
}

fn replace_categorical_values(
    resolved: &ResolvedProject,
    record: &ReportFilterRecord,
    after: &mut Value,
    values: &[Value],
) -> CliResult<()> {
    if record.filter_type != "Categorical" {
        return Err(CliError::unsupported_feature(format!(
            "report filters update cannot replace values on {} filters",
            record.filter_type
        ))
        .with_hint("Only categorical In-filter value replacement is supported. Display-name updates preserve every filter type."));
    }
    let table = record.target["table"].as_str().ok_or_else(|| {
        CliError::unsupported_feature("categorical value update requires a top-level column target")
    })?;
    let column = record.target["column"].as_str().ok_or_else(|| {
        CliError::unsupported_feature("categorical value update requires a top-level column target")
    })?;
    let column = resolve_filter_column(resolved, table, column)?;
    validate_categorical_values(values, &column)?;
    let simple_in_shape = after["filter"]["From"]
        .as_array()
        .filter(|from| from.len() == 1)
        .and_then(|from| Some((from[0]["Name"].as_str()?, from[0]["Entity"].as_str()?)))
        .is_some_and(|(alias, entity)| {
            entity.eq_ignore_ascii_case(&column.table)
                && after["filter"]["Where"]
                    .as_array()
                    .filter(|where_clauses| where_clauses.len() == 1)
                    .and_then(|where_clauses| {
                        where_clauses[0]
                            .pointer("/Condition/In/Expressions")
                            .and_then(Value::as_array)
                    })
                    .filter(|expressions| expressions.len() == 1)
                    .is_some_and(|expressions| {
                        expressions[0]
                            .pointer("/Column/Expression/SourceRef/Source")
                            .and_then(Value::as_str)
                            == Some(alias)
                            && expressions[0]
                                .pointer("/Column/Property")
                                .and_then(Value::as_str)
                                == Some(column.column.as_str())
                    })
        });
    if after["filter"]["Version"].as_i64() != Some(2) || !simple_in_shape {
        return Err(unsupported_categorical_shape());
    }
    let value_slot = after.pointer_mut("/filter/Where/0/Condition/In/Values");
    let Some(value_slot) = value_slot else {
        return Err(unsupported_categorical_shape());
    };
    if !value_slot.is_array() {
        return Err(unsupported_categorical_shape());
    }
    *value_slot = Value::Array(categorical_value_rows(values)?);
    Ok(())
}

fn unsupported_categorical_shape() -> CliError {
    CliError::unsupported_feature(
        "categorical value update supports only one Version 2 Where.Condition.In filter",
    )
    .with_hint("The existing filter has a different expression shape; it is preserved instead of being rewritten by guesswork.")
}

fn update_filter_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    record: &ReportFilterRecord,
    plan: &FilterUpdatePlan,
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
    let readback_handle = if dry_run {
        &record.handle
    } else {
        &plan.after_handle
    };
    let filter_readback = format!(
        "powerbi-cli report filters show --project {project_arg} --handle {} --json",
        shell_arg(readback_handle)
    );
    let owner_readback = owner_readback_command(record, &target_resolved.project_dir);
    let wireframe = format!("powerbi-cli report wireframe export {project_arg} --json");
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let raw_included = dry_run || include_raw;
    let before_record = filter_record_json(record, include_raw);
    let after_record = updated_record_json(record, &plan.after, include_raw);
    let before = if raw_included {
        plan.before.clone()
    } else {
        before_record.clone()
    };
    let after = if raw_included {
        plan.after.clone()
    } else {
        after_record.clone()
    };

    Ok(json!({
        "schema": "powerbi-cli.report.filters.updateMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "update",
        "changed": plan.changed,
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": before_record,
        "filterPlan": {
            "before": before,
            "after": after,
            "rawIncluded": raw_included,
            "changed": plan.changed
        },
        "changes": [{
            "kind": "pbir.filter",
            "action": "update",
            "path": canonical_display(&record.path),
            "jsonPointer": record.json_pointer,
            "parentJsonPointer": plan.parent_pointer,
            "ordinal": plan.ordinal,
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
        "filterReadbackCommand": filter_readback,
        "ownerReadbackCommand": owner_readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [filter_readback, readback, owner_readback, wireframe, inspect, validate]
    }))
}

fn updated_record_json(record: &ReportFilterRecord, raw: &Value, include_raw: bool) -> Value {
    let mut updated = record.clone();
    updated.handle = refreshed_filter_handle(record, raw);
    updated.handle_ambiguous = false;
    updated.display_name = raw["displayName"].as_str().map(ToOwned::to_owned);
    updated.fingerprint = filter_fingerprint(raw);
    updated.literal_count = raw.get("filter").map(count_literals).unwrap_or_default();
    updated.raw = raw.clone();
    filter_record_json(&updated, include_raw)
}

fn count_literals(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Bool(_) | Value::Number(_) | Value::String(_) => 1,
        Value::Array(items) => items.iter().map(count_literals).sum(),
        Value::Object(object) => object.values().map(count_literals).sum(),
    }
}
