//! Typed operation kernel for `model relationships add`.
//!
//! The command-owned parser remains in [`crate::relationships`], preserving
//! its established diagnostics and argv contract. This additive module applies
//! the existing relationship TMDL planner to a transaction working copy.

use super::{AddRelationship, Op, OpKernel, OpOutcome, Transaction};
use crate::project_io::write_text_atomic;
use crate::relationship_tmdl::{
    RelationshipDefinition, RelationshipRecord, add_relationship_plan,
    load_relationships_and_tables, normalize_cross_filtering_behavior,
    normalize_relationship_cardinality, relationship_handle,
};
use crate::relationships::parse_add_operation_args;
use crate::{CliError, CliResult, command_arg};
use serde_json::json;

/// The concrete kernel registered for [`Op::AddRelationship`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AddRelationshipKernel;

/// Parse the existing relationship-add flags into a typed operation and its
/// requested output mode. The project flag is accepted by the command parser,
/// but the transaction caller supplies the resolved project separately.
pub(crate) fn parse_args(args: &[String]) -> CliResult<(Op, crate::cli_support::MutationMode)> {
    let (payload, mode) = parse_add_operation_args(args)?;
    Ok((Op::AddRelationship(payload), mode))
}

/// Apply one AddRelationship payload to the transaction's disposable working
/// copy. Relationship validation, duplicate detection, and canonical TMDL
/// rendering are delegated to the existing relationship planner.
pub(crate) fn apply(
    payload: &AddRelationship,
    transaction: &mut Transaction,
) -> CliResult<OpOutcome> {
    let name = relationship_name_from_handle(&payload.handle)?;
    let expected_handle = relationship_handle(name);
    if payload.handle != expected_handle {
        return Err(CliError::validation_failed(format!(
            "addRelationship handle must be {expected_handle}, got {}",
            payload.handle
        ))
        .with_pointer("/handle"));
    }

    let from_cardinality = payload
        .from_cardinality
        .as_deref()
        .map(normalize_relationship_cardinality)
        .transpose()?
        .unwrap_or_else(|| "many".to_string());
    let to_cardinality = payload
        .to_cardinality
        .as_deref()
        .map(normalize_relationship_cardinality)
        .transpose()?
        .unwrap_or_else(|| "one".to_string());
    let cross_filtering_behavior = payload
        .cross_filtering_behavior
        .as_deref()
        .map(normalize_cross_filtering_behavior)
        .transpose()?
        .unwrap_or_else(|| "oneDirection".to_string());
    let definition = RelationshipDefinition {
        name: name.to_string(),
        from_table: payload.from_table.clone(),
        from_column: payload.from_column.clone(),
        to_table: payload.to_table.clone(),
        to_column: payload.to_column.clone(),
        from_cardinality,
        to_cardinality,
        cross_filtering_behavior,
        is_active: payload.is_active.unwrap_or(true),
    };

    let project = transaction.working_project()?;
    let (doc, tables) = load_relationships_and_tables(&project)?;
    if let Some(existing) = doc
        .relationships
        .iter()
        .find(|relationship| relationship.name.eq_ignore_ascii_case(name))
        && relationship_matches_definition(existing, &definition)
    {
        return Ok(outcome(transaction, &payload.handle, false, Vec::new()));
    }

    let plan = add_relationship_plan(&doc, &tables, definition)?;
    let change = json!({
        "kind": "tmdl.relationship",
        "action": "add",
        "path": crate::canonical_display(&plan.path),
        "before": plan.before_block,
        "after": plan.after_block
    });
    write_text_atomic(&plan.path, &plan.new_text)?;
    Ok(outcome(transaction, &plan.handle, true, vec![change]))
}

impl OpKernel for AddRelationshipKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let Op::AddRelationship(payload) = operation else {
            return Err(CliError::invalid_args(format!(
                "AddRelationshipKernel cannot apply operation `{}`",
                operation.tag()
            )));
        };
        apply(payload, transaction)
    }
}

fn relationship_name_from_handle(handle: &str) -> CliResult<&str> {
    let Some(name) = handle.strip_prefix("relationship:") else {
        return Err(
            CliError::invalid_args(format!("invalid relationship handle: {handle}"))
                .with_hint("Relationship handles look like `relationship:<relationship name>`."),
        );
    };
    if name.is_empty() {
        return Err(
            CliError::invalid_args(format!("invalid relationship handle: {handle}"))
                .with_hint("Relationship handles look like `relationship:<relationship name>`."),
        );
    }
    Ok(name)
}

fn relationship_matches_definition(
    existing: &RelationshipRecord,
    definition: &RelationshipDefinition,
) -> bool {
    existing
        .from_table
        .eq_ignore_ascii_case(&definition.from_table)
        && existing
            .from_column
            .eq_ignore_ascii_case(&definition.from_column)
        && existing.to_table.eq_ignore_ascii_case(&definition.to_table)
        && existing
            .to_column
            .eq_ignore_ascii_case(&definition.to_column)
        && existing.from_cardinality == definition.from_cardinality
        && existing.to_cardinality == definition.to_cardinality
        && existing.cross_filtering_behavior == definition.cross_filtering_behavior
        && existing.is_active == definition.is_active
}

fn outcome(
    transaction: &Transaction,
    handle: &str,
    changed: bool,
    changes: Vec<serde_json::Value>,
) -> OpOutcome {
    let project_arg = command_arg(&transaction.source.project_dir);
    let readback = format!(
        "powerbi-cli model relationships show --project {} --handle {} --json",
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
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    fn payload() -> AddRelationship {
        AddRelationship {
            handle: relationship_handle("KernelFactDim"),
            from_table: "FactWork".into(),
            from_column: "Key".into(),
            to_table: "DimWork".into(),
            to_column: "Key".into(),
            from_cardinality: Some("many".into()),
            to_cardinality: Some("one".into()),
            cross_filtering_behavior: Some("bothDirections".into()),
            is_active: Some(false),
        }
    }

    fn scaffold(root: &Path) -> ResolvedProject {
        let schema = json!({
            "name": "RelationshipKernel",
            "displayName": "Relationship Kernel",
            "tables": [
                {
                    "name": "FactWork",
                    "columns": [
                        {"name": "Key", "dataType": "int64", "isKey": true},
                        {"name": "Amount", "dataType": "double"}
                    ]
                },
                {
                    "name": "DimWork",
                    "columns": [
                        {"name": "Key", "dataType": "int64", "isKey": true},
                        {"name": "Name", "dataType": "string"}
                    ]
                }
            ],
            "relationships": [],
            "pages": []
        });
        scaffold_schema_value(
            schema,
            Path::new("examples/relationship-kernel.schema.json"),
            root,
            false,
        )
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
    fn add_relationship_parser_round_trips_ops_v1_and_derives_the_handle() {
        let args = vec![
            "--name".into(),
            "KernelFactDim".into(),
            "--from-table".into(),
            "FactWork".into(),
            "--from-column".into(),
            "Key".into(),
            "--to-table".into(),
            "DimWork".into(),
            "--to-column".into(),
            "Key".into(),
            "--to-cardinality".into(),
            "one".into(),
            "--cross-filter".into(),
            "both".into(),
            "--inactive".into(),
            "--dry-run".into(),
        ];
        let (operation, mode) = parse_args(&args).expect("parse operation");
        assert_eq!(mode, crate::cli_support::MutationMode::DryRun);
        let value = serde_json::to_value(&operation).expect("operation JSON");
        assert_eq!(value["op"], "addRelationship");
        assert_eq!(value["handle"], "relationship:KernelFactDim");
        assert_eq!(value["crossFilteringBehavior"], "bothDirections");
        let decoded: Op = serde_json::from_value(value).expect("decode operation");
        assert_eq!(decoded, operation);
    }

    #[test]
    fn add_relationship_kernel_is_idempotent_and_reports_declared_handle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = scaffold(temp.path());
        let operation = Op::AddRelationship(payload());
        let plan = OpPlan::new(vec![operation.clone()]);
        let index = ProjectIndex::from_project(&source).expect("project handles");
        let validated = plan.validate(&index).expect("plan");
        let mut transaction = Transaction::begin(source).expect("transaction");
        let mut kernel = AddRelationshipKernel;
        let first = transaction
            .apply_all(&validated, &mut kernel)
            .expect("first apply");
        assert!(first.outcomes[0].changed);
        assert_eq!(first.outcomes[0].created_handles, vec![payload().handle]);

        let second = apply(
            match &operation {
                Op::AddRelationship(value) => value,
                _ => unreachable!(),
            },
            &mut transaction,
        )
        .expect("second apply");
        assert!(!second.changed);
        assert_eq!(second.created_handles, vec![payload().handle]);
    }

    #[test]
    fn add_relationship_kernel_matches_cli_artifact_tree() {
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
            "--name".to_string(),
            "KernelFactDim".to_string(),
            "--from-table".to_string(),
            "FactWork".to_string(),
            "--from-column".to_string(),
            "Key".to_string(),
            "--to-table".to_string(),
            "DimWork".to_string(),
            "--to-column".to_string(),
            "Key".to_string(),
            "--from-cardinality".to_string(),
            "many".to_string(),
            "--to-cardinality".to_string(),
            "one".to_string(),
            "--cross-filtering-behavior".to_string(),
            "bothDirections".to_string(),
            "--inactive".to_string(),
            "--out-dir".to_string(),
            cli_out.display().to_string(),
        ];
        crate::relationships::relationships_command(&cli_args).expect("CLI mutation");

        let operation = Op::AddRelationship(payload());
        let index = ProjectIndex::from_project(&op_source).expect("project handles");
        let validated = OpPlan::new(vec![operation]).validate(&index).expect("plan");
        let mut transaction = Transaction::begin(op_source).expect("transaction");
        let mut kernel = AddRelationshipKernel;
        transaction
            .apply_all(&validated, &mut kernel)
            .expect("kernel mutation");
        let op_out = temp.path().join("op-out");
        transaction
            .commit_out_dir(&op_out, false)
            .expect("kernel commit");
        assert_eq!(files(&cli_out), files(&op_out));
    }
}
