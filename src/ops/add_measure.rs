//! Typed operation kernel for `model measures add`.
//!
//! The command-owned parser remains in [`crate::measures`], so the CLI keeps
//! its established diagnostics and argv contract. This module only supplies
//! the typed operation seam and applies the existing TMDL planner to a
//! transaction working copy.

use super::{AddMeasure, Op, OpKernel, OpOutcome, Transaction};
use crate::measures::parse_add_operation_args;
use crate::project_io::write_text_atomic;
use crate::tmdl::{
    MeasureDefinition, add_measure_plan, load_table_documents, measure_handle, same_name,
};
use crate::{CliError, CliResult, canonical_display, command_arg};
use serde_json::json;

/// The concrete kernel registered for [`Op::AddMeasure`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AddMeasureKernel;

/// Parse the existing `model measures add` flags into a typed operation and
/// its requested output mode. The project flag is accepted for compatibility
/// with the CLI parser but is resolved by the transaction caller.
pub(crate) fn parse_args(args: &[String]) -> CliResult<(Op, crate::cli_support::MutationMode)> {
    let (payload, mode) = parse_add_operation_args(args)?;
    Ok((Op::AddMeasure(payload), mode))
}

/// Apply one AddMeasure payload to the transaction's disposable working copy.
/// Existing measure planning and TMDL rendering are reused verbatim, keeping
/// operation artifacts byte-identical to the command path.
pub(crate) fn apply(payload: &AddMeasure, transaction: &mut Transaction) -> CliResult<OpOutcome> {
    let expected_handle = measure_handle(&payload.table, &payload.name);
    if payload.handle != expected_handle {
        return Err(CliError::validation_failed(format!(
            "addMeasure handle must be {expected_handle}, got {}",
            payload.handle
        ))
        .with_pointer("/handle"));
    }

    let project = transaction.working_project()?;
    let docs = load_table_documents(&project)?;
    if let Some(existing) = docs
        .iter()
        .flat_map(|document| document.measures.iter())
        .find(|measure| {
            same_name(&measure.table, &payload.table) && same_name(&measure.name, &payload.name)
        })
        && measure_matches_payload(existing, payload)
    {
        return Ok(outcome(transaction, &payload.handle, false, Vec::new()));
    }

    let definition = MeasureDefinition {
        name: payload.name.clone(),
        expression: payload.expression.clone(),
        lineage_tag: None,
        format_string: payload.format_string.clone(),
        format_string_definition: payload.format_string_definition.clone(),
        display_folder: payload.display_folder.clone(),
        description: payload.description.clone(),
        is_hidden: false,
    };
    let plan = add_measure_plan(&docs, &payload.table, definition)?;
    let change = json!({
        "kind": "tmdl.measure",
        "action": "add",
        "path": canonical_display(&plan.path),
        "before": plan.before_block,
        "after": plan.after_block
    });
    write_text_atomic(&plan.path, &plan.new_text)?;
    Ok(outcome(transaction, &plan.handle, true, vec![change]))
}

impl OpKernel for AddMeasureKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let Op::AddMeasure(payload) = operation else {
            return Err(CliError::invalid_args(format!(
                "AddMeasureKernel cannot apply operation `{}`",
                operation.tag()
            )));
        };
        apply(payload, transaction)
    }
}

fn measure_matches_payload(existing: &crate::tmdl::MeasureRecord, payload: &AddMeasure) -> bool {
    existing.expression.trim() == payload.expression.trim()
        && existing.format_string == payload.format_string
        && existing.format_string_definition == payload.format_string_definition
        && existing.display_folder == payload.display_folder
        && existing.description == payload.description
        && !existing.is_hidden
}

fn outcome(
    transaction: &Transaction,
    handle: &str,
    changed: bool,
    changes: Vec<serde_json::Value>,
) -> OpOutcome {
    let project_arg = command_arg(&transaction.source.project_dir);
    let readback = format!(
        "powerbi-cli model measures show --project {} --handle {} --json",
        project_arg,
        crate::cli_support::shell_arg(handle)
    );
    OpOutcome {
        changed,
        changes,
        readback: vec![readback],
        warnings: Vec::new(),
        created_handles: vec![handle.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{OpPlan, ProjectIndex};
    use crate::project_io::copy_project_dir;
    use crate::{ResolvedProject, resolve_project, scaffold_schema_value};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    fn payload() -> AddMeasure {
        AddMeasure {
            handle: measure_handle("FactSales", "Kernel Revenue"),
            table: "FactSales".into(),
            name: "Kernel Revenue".into(),
            expression: "SUM('FactSales'[Revenue])".into(),
            format_string: Some("$#,0.00".into()),
            format_string_definition: None,
            description: Some("Kernel test measure".into()),
            display_folder: Some("Kernel".into()),
        }
    }

    fn scaffold(root: &Path) -> ResolvedProject {
        let schema: Value =
            serde_json::from_str(include_str!("../../examples/sales.schema.json")).expect("schema");
        scaffold_schema_value(schema, Path::new("examples/sales.schema.json"), root, false)
            .expect("scaffold");
        resolve_project(root).expect("resolve")
    }

    fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                let relative = entry.path().strip_prefix(root).ok()?.to_path_buf();
                Some((relative, fs::read(entry.path()).ok()?))
            })
            .collect()
    }

    #[test]
    fn add_measure_parser_round_trips_ops_v1_and_derives_the_handle() {
        let args = vec![
            "--table".into(),
            "FactSales".into(),
            "--name".into(),
            "Kernel Revenue".into(),
            "--expression".into(),
            "SUM('FactSales'[Revenue])".into(),
            "--dry-run".into(),
        ];
        let (operation, mode) = parse_args(&args).expect("parse operation");
        assert_eq!(mode, crate::cli_support::MutationMode::DryRun);
        let value = serde_json::to_value(&operation).expect("operation JSON");
        assert_eq!(value["op"], "addMeasure");
        assert_eq!(value["handle"], "measure:FactSales:Kernel Revenue");
        let decoded: Op = serde_json::from_value(value).expect("decode operation");
        assert_eq!(decoded, operation);
    }

    #[test]
    fn add_measure_kernel_is_idempotent_and_reports_declared_handle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = scaffold(temp.path());
        let operation = Op::AddMeasure(payload());
        let plan = OpPlan::new(vec![operation.clone()]);
        let index = ProjectIndex::from_project(&source).expect("project handles");
        let validated = plan.validate(&index).expect("plan");
        let mut transaction = Transaction::begin(source).expect("transaction");
        let mut kernel = AddMeasureKernel;
        let first = transaction
            .apply_all(&validated, &mut kernel)
            .expect("first apply");
        assert!(first.outcomes[0].changed);
        assert_eq!(first.outcomes[0].created_handles, vec![payload().handle]);

        let second = apply(
            match &operation {
                Op::AddMeasure(value) => value,
                _ => unreachable!(),
            },
            &mut transaction,
        )
        .expect("second apply");
        assert!(!second.changed);
        assert_eq!(second.created_handles, vec![payload().handle]);
    }

    #[test]
    fn add_measure_kernel_matches_cli_artifact_tree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cli_source_root = temp.path().join("cli-source");
        let op_source_root = temp.path().join("op-source");
        let cli_source = scaffold(&cli_source_root);
        copy_project_dir(&cli_source_root, &op_source_root).expect("copy source");
        let op_source = resolve_project(&op_source_root).expect("resolve copied source");
        let cli_out = temp.path().join("cli-out");
        let cli_args = vec![
            "add".to_string(),
            "--project".to_string(),
            cli_source.project_dir.display().to_string(),
            "--table".to_string(),
            "FactSales".to_string(),
            "--name".to_string(),
            "Kernel Revenue".to_string(),
            "--expression".to_string(),
            "SUM('FactSales'[Revenue])".to_string(),
            "--format-string".to_string(),
            "$#,0.00".to_string(),
            "--description".to_string(),
            "Kernel test measure".to_string(),
            "--display-folder".to_string(),
            "Kernel".to_string(),
            "--out-dir".to_string(),
            cli_out.display().to_string(),
        ];
        crate::measures::measures_command(&cli_args).expect("CLI mutation");

        let operation = Op::AddMeasure(payload());
        let index = ProjectIndex::from_project(&op_source).expect("project handles");
        let validated = OpPlan::new(vec![operation]).validate(&index).expect("plan");
        let mut transaction = Transaction::begin(op_source).expect("transaction");
        let mut kernel = AddMeasureKernel;
        transaction
            .apply_all(&validated, &mut kernel)
            .expect("kernel mutation");
        let op_out = temp.path().join("op-out");
        transaction
            .commit_out_dir(&op_out, false)
            .expect("kernel commit");
        let cli_files = files(&cli_out);
        let op_files = files(&op_out);
        if cli_files != op_files {
            for path in cli_files.keys().chain(op_files.keys()) {
                if cli_files.get(path) != op_files.get(path) {
                    eprintln!("artifact differs: {}", path.display());
                    if let (Some(left), Some(right)) = (cli_files.get(path), op_files.get(path)) {
                        eprintln!("CLI:\n{}", String::from_utf8_lossy(left));
                        eprintln!("OPS:\n{}", String::from_utf8_lossy(right));
                    }
                }
            }
            panic!("CLI and operation artifact trees differ");
        }
    }
}
