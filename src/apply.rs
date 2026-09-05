//! Public, atomic replay of typed operation plans.

use crate::cli_support::{
    MutationMode, mode_name, require_mode_with_contract, set_report_visual_mode, take_report_value,
};
use crate::ops::{self, Op, OpKernel, OpOutcome, ProjectIndex, SnapshotOptions, Transaction};
use crate::{CliError, CliResult, canonical_display, command_arg, resolve_project};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const USAGE: &str =
    "powerbi-cli ops apply --project <project-dir-or.pbip> --ops <ops.json> --dry-run --json";

#[derive(Default)]
struct Options {
    project: Option<PathBuf>,
    ops: Option<PathBuf>,
    mode: Option<MutationMode>,
    out: Option<PathBuf>,
}

pub(crate) fn command(args: &[String]) -> CliResult<Value> {
    match args.split_first() {
        Some((action, rest)) if action == "apply" => execute(rest),
        _ => Err(args_error("ops requires the apply subcommand")),
    }
}

struct RegistryKernel;

impl OpKernel for RegistryKernel {
    fn apply(&mut self, op: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        if let Op::SetObject(operation) = op {
            crate::report_visual_objects::validate_replay_object_value(operation)?;
        }
        if let Op::BookmarkMetadata(payload) = op
            && let Some(action) = payload.fields.get("action")
            && !matches!(
                action.as_str(),
                Some("set-display-name" | "reorder" | "delete")
            )
        {
            return Err(CliError::unsupported_feature(
                "bookmarkMetadata supports only set-display-name, reorder, and delete",
            )
            .with_pointer("/action"));
        }
        let mut kernel = ops::kernel_for(op).ok_or_else(|| {
            CliError::unsupported_feature(format!("no kernel is registered for {}", op.tag()))
        })?;
        kernel.apply(op, transaction)
    }
}

fn execute(args: &[String]) -> CliResult<Value> {
    let options = parse(args)?;
    let plan_path = options.ops.as_deref().expect("validated ops path");
    // The bounded loader and OpPlan handle/stage checks precede staging.
    let plan = ops::read_plan_file(plan_path).map_err(recovery)?;
    let source = resolve_project(options.project.as_deref().expect("validated project"))?;
    let index = ProjectIndex::from_project(&source)?;
    let validated = plan
        .validate(&index)
        .map_err(|error| error.as_cli_error())?;
    let mode = options.mode.expect("validated mode");
    if let Some(out) = &options.out
        && out.symlink_metadata().is_ok()
    {
        return Err(
            args_error("ops apply requires a fresh, nonexistent --out-dir").with_pointer("/outDir"),
        );
    }
    let mut transaction = Transaction::begin(source.clone())?;
    let working = transaction.working_project()?;
    let receipt = transaction
        .apply_all(&validated, &mut RegistryKernel)
        .map_err(|failure| {
            let mut error = *failure.error;
            if failure.failed_index == plan.ops.len() {
                error = error.with_pointer("/ops");
            }
            // Kernel-relative pointers belong under the failing operation. Plan
            // validation errors already use the public envelope pointer convention.
            if failure.failed_index < plan.ops.len()
                && !error
                    .pointer()
                    .is_some_and(|pointer| pointer.starts_with("/ops/"))
            {
                let suffix = error.pointer().unwrap_or("").to_string();
                error = error.with_pointer(format!("/ops/{}{suffix}", failure.failed_index));
            }
            error.message = format!(
                "operation {} failed; no output committed: {}",
                failure.failed_index, error.message
            );
            error.message = error.message.replace(
                &canonical_display(&working.project_dir),
                &canonical_display(&source.project_dir),
            );
            if let Some(hint) = &mut error.hint {
                *hint = hint.replace(
                    &canonical_display(&working.project_dir),
                    &canonical_display(&source.project_dir),
                );
            }
            for command in &mut error.suggested_commands {
                *command = command.replace(
                    &command_arg(&working.project_dir),
                    &command_arg(&source.project_dir),
                );
            }
            recovery(error)
        })?;
    // Score the actual staged tree, including in dry-run mode. No Desktop
    // execution is implied, and a validation failure cannot reach commit.
    let mut scorecard = crate::scorecard::project_scorecard(&working, "unit-smoke");
    let (target, snapshot) = match mode {
        MutationMode::DryRun => (source.project_dir.clone(), None),
        MutationMode::OutDir => {
            let committed =
                transaction.commit_out_dir(options.out.as_deref().expect("out-dir"), false)?;
            (committed.project_dir, committed.snapshot_dir)
        }
        MutationMode::InPlace => {
            let committed = transaction.commit_in_place(SnapshotOptions::default())?;
            (committed.project_dir, committed.snapshot_dir)
        }
    };
    relocate(&mut scorecard, &working.project_dir, &target);
    let mut changes = Vec::new();
    let mut outcomes = Vec::new();
    let mut warnings = Vec::new();
    let mut readback = BTreeMap::<String, BTreeSet<String>>::new();
    let inspect = format!("powerbi-cli inspect --deep {} --json", command_arg(&target));
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&target)
    );
    for (index, (op, outcome)) in plan.ops.iter().zip(&receipt.outcomes).enumerate() {
        let mut value = serde_json::to_value(outcome).expect("outcome serializes");
        relocate(&mut value, &source.project_dir, &target);
        relocate(&mut value, &working.project_dir, &target);
        value["index"] = json!(index);
        value["operation"] = json!(op.tag());
        changes.extend(
            value["changes"]
                .as_array()
                .expect("changes")
                .iter()
                .cloned(),
        );
        warnings.extend(
            value["warnings"]
                .as_array()
                .expect("warnings")
                .iter()
                .cloned(),
        );
        let commands = value["readback"]
            .as_array()
            .expect("readback")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        let mut handles = op
            .references()
            .into_iter()
            .map(|reference| reference.handle.to_string())
            .collect::<BTreeSet<_>>();
        handles.extend(op.declared_handle_owned());
        // Flattened legacy payloads do not yet enumerate all references in
        // Op::references. Preserve their explicit stable targets in readback.
        let payload = serde_json::to_value(op).expect("operation serializes");
        for key in [
            "handle",
            "page",
            "visual",
            "from",
            "targetPage",
            "source",
            "target",
        ] {
            if let Some(handle) = payload.get(key).and_then(Value::as_str)
                && [
                    "report:",
                    "page:",
                    "visual:",
                    "filter:",
                    "bookmark:",
                    "measure:",
                    "column:",
                    "table:",
                    "relationship:",
                ]
                .iter()
                .any(|prefix| handle.starts_with(prefix))
            {
                handles.insert(handle.to_string());
            }
        }
        if handles.is_empty() {
            handles.insert("report:main".to_string());
        }
        for handle in handles {
            readback
                .entry(handle)
                .or_default()
                .extend(if commands.is_empty() {
                    BTreeSet::from([inspect.clone()])
                } else {
                    commands.clone()
                });
        }
        outcomes.push(value);
    }
    let next = if mode == MutationMode::DryRun {
        vec![format!(
            "powerbi-cli ops apply --project {} --ops {} --out-dir <fresh-project-dir> --json",
            command_arg(&source.project_dir),
            command_arg(plan_path)
        )]
    } else {
        vec![
            inspect.clone(),
            validate.clone(),
            format!("powerbi-cli handoff check {} --json", command_arg(&target)),
        ]
    };
    scorecard["next"] = json!(next);
    Ok(json!({
        "schema": "powerbi-cli.ops.apply.v1", "ok": true, "exitCode": 0,
        "dryRun": mode == MutationMode::DryRun, "mode": mode_name(mode),
        "changed": !receipt.changes.is_empty(), "projectDir": canonical_display(&target),
        "inputs": {"project": canonical_display(&source.project_dir), "ops": canonical_display(plan_path)},
        "scope": {"kind": "ops-apply", "mode": mode_name(mode), "projectDir": canonical_display(&target),
            "operationCount": plan.ops.len(), "handles": readback.keys().collect::<Vec<_>>()},
        "operationCount": plan.ops.len(), "operationOutcomes": outcomes,
        "changes": changes, "journal": receipt.changes, "readback": readback,
        "scorecard": scorecard, "scorecardScope": "staged-result", "warnings": warnings,
        "snapshotDir": snapshot.as_deref().map(canonical_display),
        "inspectCommand": inspect, "validateCommand": validate, "next": next
    }))
}

fn relocate(value: &mut Value, from: &Path, to: &Path) {
    match value {
        Value::String(text) if text.starts_with("powerbi-cli ") => {
            *text = text.replace(&command_arg(from), &command_arg(to));
        }
        Value::String(text) => {
            *text = text.replace(&canonical_display(from), &canonical_display(to))
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| relocate(value, from, to)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| relocate(value, from, to)),
        _ => {}
    }
}

fn parse(args: &[String]) -> CliResult<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                if options.project.is_some() {
                    return Err(args_error("--project may be supplied only once"));
                }
                options.project = Some(take_report_value(args, &mut index, "--project")?.into());
            }
            "--ops" => {
                if options.ops.is_some() {
                    return Err(args_error("--ops may be supplied only once"));
                }
                options.ops = Some(take_report_value(args, &mut index, "--ops")?.into());
            }
            "--dry-run" | "--in-place" => {
                let mode = if args[index] == "--dry-run" {
                    MutationMode::DryRun
                } else {
                    MutationMode::InPlace
                };
                set_report_visual_mode(&mut options.mode, mode)?;
                index += 1;
            }
            "--out-dir" => {
                set_report_visual_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out = Some(take_report_value(args, &mut index, "--out-dir")?.into());
            }
            other => return Err(args_error(format!("unknown ops apply flag: {other}"))),
        }
    }
    if options.project.is_none() || options.ops.is_none() {
        return Err(args_error("ops apply requires --project and --ops"));
    }
    options.mode = Some(require_mode_with_contract(
        options.mode,
        "ops apply",
        "Preview with --dry-run; --in-place commits atomically with a sibling snapshot.",
        USAGE,
    )?);
    Ok(options)
}

fn args_error(message: impl Into<String>) -> CliError {
    CliError::invalid_args(message)
        .with_hint("Use a typed ops.v1 plan and exactly one explicit project output mode.")
        .with_suggested_command(USAGE)
}

fn recovery(mut error: CliError) -> CliError {
    if error.hint.is_none() {
        error = error
            .with_hint("Correct the operation at the reported pointer and retry with --dry-run.");
    }
    if error.suggested_commands.is_empty() {
        error = error.with_suggested_command("powerbi-cli capabilities --for 'ops apply' --json");
    }
    error
}
