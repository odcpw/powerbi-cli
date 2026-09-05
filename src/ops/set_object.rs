//! Operation kernel for the curated visual object-property mutation.
//!
//! The command-facing parser and PBIR patch live in
//! [`crate::report_visual_objects`]. This adapter supplies the typed
//! `ops.v1` boundary and stages the exact same patch through a disposable
//! [`Transaction`].

use super::{Op, OpKernel, OpOutcome, OpPlan, ProjectIndex, SnapshotOptions, Transaction};
use crate::cli_support::{MutationMode, command_arg, shell_arg};
use crate::pbir::{VisualSelector, find_visual, load_report_snapshot};
use crate::report_visual_objects::{
    AppliedObjectMutation, ParsedSetObject, apply_set_object_operation, parse_set_object_args,
    render_set_object_mutation,
};
use crate::{CliError, CliResult, ResolvedProject, canonical_display, resolve_project};
use serde_json::{Value, json};
use std::path::PathBuf;

/// Parsed command context plus its typed operation payload. Project/output
/// arguments remain outside `SetObject` so serialized operations stay
/// portable across projects and callers.
pub(crate) type SetObjectInvocation = ParsedSetObject;

#[derive(Debug, Default)]
pub(crate) struct SetObjectKernel {
    applied: Option<AppliedObjectMutation>,
}

/// Apply one payload through the concrete kernel. This small free-function
/// seam mirrors the operation conversion recipe and is useful to compiler or
/// replay callers that do not need the command renderer.
pub(crate) fn apply(
    operation: &super::SetObject,
    transaction: &mut Transaction,
) -> CliResult<OpOutcome> {
    let mut kernel = SetObjectKernel::default();
    OpKernel::apply(&mut kernel, &Op::SetObject(operation.clone()), transaction)
}

impl SetObjectKernel {
    fn take_applied(&mut self) -> CliResult<AppliedObjectMutation> {
        self.applied.take().ok_or_else(|| {
            CliError::unexpected("set-object kernel completed without an apply result")
        })
    }
}

impl OpKernel for SetObjectKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let Op::SetObject(operation) = operation else {
            return Err(CliError::invalid_args(format!(
                "set-object kernel cannot apply `{}`",
                operation.tag()
            )));
        };
        let working = transaction.working_project()?;
        let applied = apply_set_object_operation(&working, operation)?;
        let relative_path = applied
            .visual_path
            .strip_prefix(transaction.work_dir())
            .map(PathBuf::from)
            .map_err(|error| {
                CliError::unexpected(format!(
                    "set-object kernel visual path is outside its working copy: {error}"
                ))
            })?;
        let source_path = transaction.source.project_dir.join(relative_path);
        let change = json!({
            "kind": "pbir.visual.objectProperty",
            "action": "set-object",
            "path": canonical_display(&source_path),
            "object": applied.object.clone(),
            "property": applied.property.clone(),
            "jsonPointer": applied.json_pointer.clone(),
            "before": applied.before.clone(),
            "after": applied.after.clone()
        });
        let readback = format!(
            "powerbi-cli report visuals show --project {} --handle {} --json",
            command_arg(&transaction.source.project_dir),
            shell_arg(&operation.visual)
        );
        let changed = applied.before != applied.after;
        self.applied = Some(applied);
        Ok(OpOutcome {
            changed,
            changes: vec![change],
            readback: vec![readback],
            warnings: Vec::new(),
            created_handles: Vec::new(),
        })
    }
}

/// Parse one command invocation and run its operation through a validated
/// transaction. The output renderer remains the command's historical JSON
/// envelope, so callers observe no CLI contract change.
pub(crate) fn execute(args: &[String]) -> CliResult<Value> {
    let invocation = parse_invocation(args)?;
    crate::cli_support::preflight_out_dir(args, execute)?;
    let source = resolve_project(&invocation.project)?;
    let handle = resolve_visual_handle(&source, &invocation.selector)?;
    let mut operation = invocation.operation.clone();
    operation.visual = handle;

    let plan = OpPlan::new(vec![Op::SetObject(operation.clone())]);
    let project_index = ProjectIndex::from_project(&source)?;
    let validated = plan
        .validate(&project_index)
        .map_err(|error| error.as_cli_error())?;
    let mut transaction = Transaction::begin(source.clone())?;
    let mut kernel = SetObjectKernel::default();
    transaction
        .apply_all(&validated, &mut kernel)
        .map_err(|failure| *failure.error)?;
    let applied = kernel.take_applied()?;

    let target = match invocation.mode {
        MutationMode::DryRun => source,
        MutationMode::OutDir => {
            let out_dir = invocation
                .out_dir
                .as_deref()
                .ok_or_else(|| CliError::invalid_args("--out-dir requires a directory"))?;
            let receipt = transaction.commit_out_dir(out_dir, false)?;
            resolve_project(&receipt.project_dir)?
        }
        MutationMode::InPlace => {
            transaction.commit_in_place(SnapshotOptions::default())?;
            source
        }
    };
    render_set_object_mutation(&target, invocation.mode, &operation, &applied)
}

fn resolve_visual_handle(
    project: &ResolvedProject,
    selector: &VisualSelector,
) -> CliResult<String> {
    Ok(find_visual(
        &load_report_snapshot(project)?.pages,
        selector,
        "report visuals set-object",
    )?
    .handle
    .clone())
}

/// Parse the typed operation and output mode expected by the operation IR
/// conversion recipe. Project and selector context remains command-local and
/// is retained by [`parse_invocation`] for execution.
pub(crate) fn parse_args(args: &[String]) -> CliResult<(crate::ops::SetObject, MutationMode)> {
    let invocation = parse_invocation(args)?;
    Ok((invocation.operation, invocation.mode))
}

fn parse_invocation(args: &[String]) -> CliResult<SetObjectInvocation> {
    parse_set_object_args(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::SetObject;
    use crate::{ResolvedProject, resolve_project, scaffold_schema_value};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    fn scaffold_project(root: &Path) -> ResolvedProject {
        let schema = serde_json::from_str(include_str!("../../examples/sales.schema.json"))
            .expect("sales schema");
        scaffold_schema_value(schema, Path::new("examples/sales.schema.json"), root, false)
            .expect("scaffold project");
        resolve_project(root).expect("resolve project")
    }

    fn first_visual_handle(project: &ResolvedProject) -> String {
        load_report_snapshot(project)
            .expect("snapshot")
            .pages
            .into_iter()
            .flat_map(|page| page.visuals)
            .next()
            .expect("visual")
            .handle
    }

    fn operation(handle: String) -> Op {
        Op::SetObject(SetObject {
            visual: handle,
            object: "categoryLabels".into(),
            property: "fontSize".into(),
            value: json!({"expr":{"Literal":{"Value":"20D"}}}),
        })
    }

    fn project_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        WalkDir::new(root)
            .into_iter()
            .filter_entry(|entry| entry.file_name() != ".git")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry
                        .path()
                        .strip_prefix(root)
                        .expect("relative")
                        .to_path_buf(),
                    fs::read(entry.path()).expect("read project file"),
                )
            })
            .collect()
    }

    #[test]
    fn set_object_operation_declares_visual_reference_but_no_created_handle() {
        let operation = operation("visual:ReportSectionOverview:VisualContainerRevenue".into());
        assert_eq!(operation.declared_handle(), None);
        assert_eq!(
            operation.references(),
            vec![super::super::HandleReference {
                field: "visual",
                handle: "visual:ReportSectionOverview:VisualContainerRevenue"
            }]
        );
        assert_eq!(
            operation.idempotent_key(),
            serde_json::to_string(&operation).unwrap()
        );
    }

    #[test]
    fn set_object_kernel_and_cli_path_produce_byte_identical_out_dir_trees() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cli_root = temp.path().join("cli");
        let op_root = temp.path().join("op");
        let cli = scaffold_project(&cli_root);
        let op = scaffold_project(&op_root);
        let handle = first_visual_handle(&cli);
        let op_handle = first_visual_handle(&op);
        assert_eq!(handle, op_handle);

        let cli_out = temp.path().join("cli-out");
        let cli_args = vec![
            "visuals".to_string(),
            "set-object".to_string(),
            "--project".to_string(),
            cli_root.display().to_string(),
            "--handle".to_string(),
            handle,
            "--object".to_string(),
            "categoryLabels".to_string(),
            "--property".to_string(),
            "fontSize".to_string(),
            "--value".to_string(),
            "20".to_string(),
            "--out-dir".to_string(),
            cli_out.display().to_string(),
        ];
        crate::report::report_command(&cli_args).expect("CLI path");

        let operation = operation(op_handle);
        let plan = OpPlan::new(vec![operation]);
        let index = ProjectIndex::from_project(&op).expect("project index");
        let validated = plan.validate(&index).expect("validated plan");
        let mut transaction = Transaction::begin(op).expect("transaction");
        let mut kernel = SetObjectKernel::default();
        transaction
            .apply_all(&validated, &mut kernel)
            .expect("kernel apply");
        let op_out = temp.path().join("op-out");
        transaction
            .commit_out_dir(&op_out, false)
            .expect("op commit");

        assert_eq!(project_files(&cli_out), project_files(&op_out));
    }

    #[test]
    fn parse_args_keeps_typed_literal_encoding_and_output_mode_context() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold_project(&temp.path().join("project"));
        let handle = first_visual_handle(&project);
        let parsed = parse_invocation(&[
            "--project".into(),
            project.project_dir.display().to_string(),
            "--handle".into(),
            handle.clone(),
            "--object".into(),
            "categoryLabels".into(),
            "--property".into(),
            "fontSize".into(),
            "--value".into(),
            "20".into(),
            "--dry-run".into(),
        ])
        .expect("parse set-object");
        assert_eq!(parsed.mode, MutationMode::DryRun);
        assert_eq!(parsed.operation.visual, handle);
        assert_eq!(
            parsed.operation.value,
            json!({"expr":{"Literal":{"Value":"20D"}}})
        );
    }
}
