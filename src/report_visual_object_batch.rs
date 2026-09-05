//! Atomic `report visuals set-object --batch` execution.
//!
//! The batch boundary accepts the same bounded `powerbi-cli.ops.v1` envelope
//! used by the operation IR, but deliberately narrows its contents to
//! `setObject` operations.  Every payload is decoded and checked against the
//! formatting catalog before a project transaction is created; the existing
//! SetObject kernel then applies the complete plan to one disposable working
//! copy.

use crate::cli_support::{
    MutationMode, mode_name, preflight_out_dir, required_project, set_report_visual_mode,
    take_report_value,
};
use crate::input_safety::{self, INPUT_SAFETY_ERROR_CODE};
use crate::ops::{
    Op, OpOutcome, OpPlan, ProjectIndex, SetObjectKernel, SnapshotOptions, Transaction,
    TransactionReceipt,
};
use crate::report_visual_objects::validate_set_object_operation;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, resolve_project,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const COMMAND: &str = "report visuals set-object";
const BATCH_SCHEMA: &str = "powerbi-cli.report.visuals.objectBatchMutation.v1";
const BATCH_INPUT_USAGE: &str = "powerbi-cli report visuals set-object --project <project-dir-or.pbip> --batch <ops.v1.json> --dry-run --json";
const LEGACY_SET_OBJECT_KIND: &str = "SetObject";

#[derive(Debug, Default)]
struct BatchOptions {
    project: Option<PathBuf>,
    batch: Option<PathBuf>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

/// Execute a bounded set-object operation plan through one transaction.
pub(crate) fn execute(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    preflight_out_dir(args, execute)?;

    // Decode and catalog-check every operation before resolving or copying the
    // selected project.  A malformed later item therefore cannot leave an
    // earlier item staged in a transaction or create an output directory.
    let operations = read_batch_operations(
        options
            .batch
            .as_deref()
            .expect("parse_args requires --batch"),
    )?;

    let project_path = required_project(options.project.clone(), COMMAND)?;
    let source = resolve_project(&project_path)?;
    let index = ProjectIndex::from_project(&source)?;
    let plan = OpPlan::new(operations.clone());
    let validated = plan
        .validate(&index)
        .map_err(|error| error.as_cli_error())?;

    let mut transaction = Transaction::begin(source.clone())?;
    let mut kernel = SetObjectKernel::default();
    let receipt = transaction
        .apply_all(&validated, &mut kernel)
        .map_err(|failure| batch_failure(failure.failed_index, *failure.error))?;

    let mode = options.mode.expect("parse_args requires an output mode");
    let target = match mode {
        MutationMode::DryRun => source.clone(),
        MutationMode::OutDir => {
            let out_dir = options
                .out_dir
                .as_deref()
                .ok_or_else(|| CliError::invalid_args("--out-dir requires a directory"))?;
            let committed = transaction.commit_out_dir(out_dir, false)?;
            resolve_project(&committed.project_dir)?
        }
        MutationMode::InPlace => {
            transaction.commit_in_place(SnapshotOptions::default())?;
            source.clone()
        }
    };

    render_response(&target, &source, mode, &operations, receipt)
}

fn parse_args(args: &[String]) -> CliResult<BatchOptions> {
    let mut options = BatchOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_report_value(
                    args,
                    &mut index,
                    "--project",
                )?));
            }
            "--batch" => {
                if options.batch.is_some() {
                    return Err(batch_args_error("--batch may be specified only once"));
                }
                options.batch = Some(PathBuf::from(take_report_value(
                    args, &mut index, "--batch",
                )?));
            }
            "--dry-run" => {
                set_report_visual_mode(&mut options.mode, MutationMode::DryRun)?;
                index += 1;
            }
            "--in-place" => {
                set_report_visual_mode(&mut options.mode, MutationMode::InPlace)?;
                index += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_report_value(args, &mut index, "--out-dir")?);
                set_report_visual_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(batch_args_error(format!(
                    "unknown {COMMAND} batch flag: {other}"
                )));
            }
        }
    }
    if options.batch.is_none() {
        return Err(batch_args_error(format!(
            "{COMMAND} --batch requires <ops.v1.json>"
        )));
    }
    options.mode = Some(crate::cli_support::require_mode_with_contract(
        options.mode,
        COMMAND,
        "Start with `--dry-run`; use `--out-dir` or `--in-place` only after reviewing every batch entry.",
        BATCH_INPUT_USAGE,
    )?);
    Ok(options)
}

fn read_batch_operations(path: &Path) -> CliResult<Vec<Op>> {
    let mut known_kinds = crate::ops::registered_kernel_tags().to_vec();
    known_kinds.push(LEGACY_SET_OBJECT_KIND);
    let mut value =
        input_safety::read_ops(path, &known_kinds).map_err(normalize_batch_input_error)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| batch_input_error("ops batch must be a JSON object", "/"))?;
    let allowed = BTreeSet::from(["schema", "ops"]);
    if let Some(key) = object.keys().find(|key| !allowed.contains(key.as_str())) {
        return Err(batch_input_error(
            format!("ops batch contains unknown top-level field `{key}`"),
            &format!("/{key}"),
        ));
    }
    if object.get("schema").and_then(Value::as_str) != Some(crate::ops::OPS_SCHEMA) {
        // `read_ops` normally catches this first; retain a pointer here for
        // callers that provide a non-string schema value.
        return Err(batch_input_error(
            "ops batch schema must be powerbi-cli.ops.v1",
            "/schema",
        ));
    }
    let ops = object
        .get_mut("ops")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| batch_input_error("ops batch must contain an ops array", "/ops"))?;
    let mut parsed = Vec::with_capacity(ops.len());
    for (index, raw) in ops.iter_mut().enumerate() {
        let map = raw.as_object_mut().ok_or_else(|| {
            batch_input_error(
                format!("ops[{index}] must be an object"),
                &format!("/ops/{index}"),
            )
        })?;
        let tag = map
            .get("op")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| map.get("kind").and_then(Value::as_str).map(str::to_owned))
            .ok_or_else(|| {
                batch_input_error(
                    format!("ops[{index}] requires string field `op`"),
                    &format!("/ops/{index}/op"),
                )
            })?;
        if tag != "setObject" && tag != LEGACY_SET_OBJECT_KIND {
            return Err(batch_input_error(
                format!(
                    "ops[{index}] uses unsupported batch operation `{tag}`; only setObject is accepted"
                ),
                &format!("/ops/{index}/op"),
            ));
        }
        map.remove("kind");
        map.insert("op".to_string(), Value::String("setObject".to_string()));
        let decoded: Op = serde_json::from_value(Value::Object(map.clone())).map_err(|error| {
            batch_input_error(
                format!("ops[{index}] is not a valid setObject operation: {error}"),
                &format!("/ops/{index}"),
            )
        })?;
        let Op::SetObject(operation) = decoded else {
            return Err(batch_input_error(
                format!("ops[{index}] is not a setObject operation"),
                &format!("/ops/{index}/op"),
            ));
        };
        validate_set_object_operation(&operation).map_err(|mut error| {
            let field = if operation.visual.trim().is_empty() {
                "visual"
            } else if operation.object.trim().is_empty() {
                "object"
            } else {
                "property"
            };
            let catalog_hint = error.hint.take().unwrap_or_default();
            error.hint = Some(format!(
                "{catalog_hint} Fix batch entry {index} in the bounded ops.v1 file before retrying."
            ));
            error.suggested_commands = vec![BATCH_INPUT_USAGE.to_string()];
            error.with_pointer(format!("/ops/{index}/{field}"))
        })?;
        parsed.push(Op::SetObject(operation));
    }
    Ok(parsed)
}

fn render_response(
    target: &ResolvedProject,
    source: &ResolvedProject,
    mode: MutationMode,
    operations: &[Op],
    receipt: TransactionReceipt,
) -> CliResult<Value> {
    let dry_run = matches!(mode, MutationMode::DryRun);
    let validation = if dry_run {
        None
    } else {
        Some(crate::validate_project(target)?)
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
    let source_root = canonical_display(&source.project_dir);
    let target_root = canonical_display(&target.project_dir);
    let mut operation_outcomes = Vec::with_capacity(receipt.outcomes.len());
    for (index, (operation, mut outcome)) in operations.iter().zip(receipt.outcomes).enumerate() {
        relocate_outcome(&mut outcome, &source_root, &target_root);
        outcome.readback = vec![readback_command(target, operation)];
        let mut object = serde_json::to_value(outcome)
            .map_err(|error| CliError::unexpected(format!("serialize batch outcome: {error}")))?;
        if let Some(map) = object.as_object_mut() {
            map.insert("index".to_string(), Value::from(index));
            map.insert(
                "operation".to_string(),
                serde_json::to_value(operation).map_err(|error| {
                    CliError::unexpected(format!("serialize batch operation: {error}"))
                })?,
            );
        }
        operation_outcomes.push(object);
    }
    let changes = operation_outcomes
        .iter()
        .filter_map(|outcome| outcome.get("changes"))
        .filter_map(Value::as_array)
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    let readback_commands = operation_outcomes
        .iter()
        .filter_map(|outcome| outcome.get("readback"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let changed_count = operation_outcomes
        .iter()
        .filter(|outcome| outcome["changed"].as_bool() == Some(true))
        .count();
    let inspect = format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&target.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&target.project_dir)
    );
    let mut next = readback_commands.clone();
    next.push(inspect.clone());
    next.push(validate.clone());
    Ok(json!({
        "schema": BATCH_SCHEMA,
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "set-object-batch",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target.project_dir),
        "pbip": canonical_display(&target.pbip_path),
        "reportDir": canonical_display(&target.report_dir),
        "batch": {
            "schema": crate::ops::OPS_SCHEMA,
            "operationCount": operations.len(),
            "operationKind": "setObject"
        },
        "count": operations.len(),
        "changedCount": changed_count,
        "operationOutcomes": operation_outcomes,
        "changes": changes,
        "readbackCommands": readback_commands,
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
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": next,
    }))
}

fn readback_command(target: &ResolvedProject, operation: &Op) -> String {
    let Op::SetObject(operation) = operation else {
        return String::new();
    };
    format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        command_arg(&target.project_dir),
        crate::cli_support::shell_arg(&operation.visual)
    )
}

fn relocate_outcome(outcome: &mut OpOutcome, source_root: &str, target_root: &str) {
    for change in &mut outcome.changes {
        let Some(path) = change
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        if path == source_root {
            change["path"] = Value::String(target_root.to_string());
        } else if let Some(relative) = path
            .strip_prefix(source_root)
            .and_then(|rest| rest.strip_prefix('/').or_else(|| rest.strip_prefix('\\')))
        {
            change["path"] = Value::String(format!("{target_root}/{relative}"));
        }
    }
}

fn batch_failure(index: usize, mut error: CliError) -> CliError {
    if error.pointer().is_none() {
        error = error.with_pointer(format!("/ops/{index}"));
    }
    if error.hint.is_none() {
        error = error.with_hint(format!(
            "Batch entry {index} failed; no project output was committed. Review the entry and rerun {BATCH_INPUT_USAGE}."
        ));
    }
    error
}

fn batch_args_error(message: impl Into<String>) -> CliError {
    CliError::invalid_args(message)
        .with_hint("Use --batch <ops.v1.json> with exactly one output mode; the file may contain only setObject operations.")
        .with_suggested_command(BATCH_INPUT_USAGE)
}

fn batch_input_error(message: impl Into<String>, pointer: &str) -> CliError {
    CliError::new(
        INPUT_SAFETY_ERROR_CODE,
        EXIT_VALIDATION_FAILED,
        format!("ops batch input refused: {}", message.into()),
    )
    .with_pointer(pointer)
    .with_hint(
        "Supply a bounded powerbi-cli.ops.v1 JSON file containing only setObject operations.",
    )
    .with_suggested_command(BATCH_INPUT_USAGE)
}

fn normalize_batch_input_error(mut error: CliError) -> CliError {
    if error.code == INPUT_SAFETY_ERROR_CODE {
        error.hint = Some(
            "Supply a bounded powerbi-cli.ops.v1 JSON file containing only setObject operations."
                .to_string(),
        );
        error.suggested_commands = vec![BATCH_INPUT_USAGE.to_string()];
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formatting_catalog::formatting_catalog_entries;
    use crate::ops::SetObject;
    use serde_json::json;
    use std::fs;

    fn operation() -> Value {
        json!({
            "op": "setObject",
            "visual": "visual:ReportSectionOverview:VisualContainerRevenue",
            "object": "categoryLabels",
            "property": "fontSize",
            "value": {"expr": {"Literal": {"Value": "20D"}}}
        })
    }

    #[test]
    fn batch_reader_accepts_canonical_and_legacy_set_object_tags() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("batch.json");
        let mut legacy = operation();
        legacy
            .as_object_mut()
            .expect("operation object")
            .remove("op");
        legacy["kind"] = Value::String("SetObject".into());
        fs::write(
            &path,
            serde_json::to_vec(
                &json!({"schema": crate::ops::OPS_SCHEMA, "ops": [operation(), legacy]}),
            )
            .expect("serialize batch"),
        )
        .expect("write batch");
        let parsed = read_batch_operations(&path).expect("read batch");
        assert_eq!(parsed.len(), 2);
        assert!(
            parsed
                .iter()
                .all(|operation| operation.tag() == "setObject")
        );
    }

    #[test]
    fn batch_reader_rejects_non_set_object_before_project_access() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("batch.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": crate::ops::OPS_SCHEMA,
                "ops": [{"op": "setPosition", "visual": "visual:x:y"}]
            }))
            .expect("serialize batch"),
        )
        .expect("write batch");
        let error = read_batch_operations(&path).expect_err("non-set-object operation");
        assert_eq!(error.code, INPUT_SAFETY_ERROR_CODE);
        assert_eq!(error.pointer(), Some("/ops/0/op"));
    }

    #[test]
    fn set_object_payload_preflight_uses_the_embedded_formatting_catalog() {
        let supported = SetObject {
            visual: "visual:Page:Visual".into(),
            object: "title".into(),
            property: "text".into(),
            value: json!({"expr": {"Literal": {"Value": "'Title'"}}}),
        };
        validate_set_object_operation(&supported).expect("supported catalog pair");
        assert!(!formatting_catalog_entries().expect("catalog").is_empty());
    }

    #[test]
    fn batch_payload_preflight_points_to_the_invalid_entry_field() {
        let temp = tempfile::tempdir().expect("tempdir");
        for (field, value, expected_pointer) in [
            ("visual", "", "/ops/0/visual"),
            ("object", "", "/ops/0/object"),
            ("property", "", "/ops/0/property"),
        ] {
            let path = temp.path().join(format!("invalid-{field}.json"));
            let mut invalid = operation();
            invalid[field] = Value::String(value.to_string());
            fs::write(
                &path,
                serde_json::to_vec(&json!({
                    "schema": crate::ops::OPS_SCHEMA,
                    "ops": [invalid]
                }))
                .expect("serialize batch"),
            )
            .expect("write batch");
            let error = read_batch_operations(&path).expect_err("invalid entry field");
            assert_eq!(error.pointer(), Some(expected_pointer));
            assert_eq!(
                error.suggested_commands,
                vec![BATCH_INPUT_USAGE.to_string()]
            );
        }
    }
}
