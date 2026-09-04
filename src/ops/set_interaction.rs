//! Typed operation kernel for report visual interactions.

use super::{Op, OpKernel, OpOutcome, SetInteraction, Transaction};
use crate::report_interaction_mutations;
use crate::{CliError, CliResult};

/// Applies [`SetInteraction`] operations to a transaction's staged PBIP tree.
#[derive(Debug, Default)]
pub(crate) struct SetInteractionKernel;

impl OpKernel for SetInteractionKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let Op::SetInteraction(payload) = operation else {
            return Err(CliError::invalid_args(
                "setInteraction kernel received a different operation",
            ));
        };
        let project = transaction.working_project()?;
        report_interaction_mutations::apply_set_interaction_operation(payload, &project)
    }
}

impl SetInteractionKernel {
    /// Parse argv-shaped endpoint selectors into the typed operation and mode.
    pub(crate) fn parse_args(
        args: &[String],
    ) -> CliResult<(SetInteraction, crate::cli_support::MutationMode)> {
        report_interaction_mutations::parse_set_interaction_operation(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{Op, OpPlan, ProjectIndex, Transaction};
    use crate::pbir::load_report_snapshot;
    use crate::{ResolvedProject, resolve_project, scaffold_schema_value};
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    fn scaffold(root: &Path) -> ResolvedProject {
        let schema: Value = serde_json::from_str(include_str!("../../examples/sales.schema.json"))
            .expect("sales schema");
        scaffold_schema_value(schema, Path::new("examples/sales.schema.json"), root, false)
            .expect("scaffold project");
        resolve_project(root).expect("resolve project")
    }

    fn operation_for(project: &ResolvedProject) -> SetInteraction {
        let snapshot = load_report_snapshot(project).expect("report snapshot");
        let page = snapshot.pages.first().expect("sales page");
        let source = page.visuals.first().expect("source visual");
        let target = page.visuals.get(1).expect("target visual");
        SetInteraction {
            page: page.handle.clone(),
            source: source.handle.clone(),
            target: target.handle.clone(),
            interaction_type: "DataFilter".to_string(),
        }
    }

    fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
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
    fn parse_args_round_trips_endpoint_selectors_and_shared_mode() {
        let args = [
            "--page",
            "page:ReportSectionOverview",
            "--source",
            "visual:ReportSectionOverview:VisualContainerRevenue",
            "--target",
            "visual:ReportSectionOverview:VisualContainerTable",
            "--type",
            "filter",
            "--dry-run",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let (payload, mode) = SetInteractionKernel::parse_args(&args).expect("parse operation");
        assert_eq!(mode, crate::cli_support::MutationMode::DryRun);
        assert_eq!(payload.interaction_type, "DataFilter");
        let encoded = serde_json::to_value(Op::SetInteraction(payload)).expect("op json");
        assert_eq!(encoded["op"], "setInteraction");
    }

    #[test]
    fn staged_kernel_matches_cli_tree_and_returns_deterministic_handle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cli_project = scaffold(&temp.path().join("cli"));
        let op_project = scaffold(&temp.path().join("op"));
        let operation = operation_for(&cli_project);
        let cli_args = vec![
            "--project".to_string(),
            cli_project.project_dir.to_string_lossy().into_owned(),
            "--page".to_string(),
            operation.page.clone(),
            "--source".to_string(),
            operation.source.clone(),
            "--target".to_string(),
            operation.target.clone(),
            "--type".to_string(),
            operation.interaction_type.clone(),
            "--in-place".to_string(),
        ];
        crate::report_interaction_mutations::set_interaction(&cli_args).expect("CLI mutation");
        let before = tree(&op_project.project_dir);
        let plan = OpPlan::new(vec![Op::SetInteraction(operation.clone())]);
        let index = ProjectIndex::from_project(&op_project).expect("project handles");
        let validated = plan.validate(&index).expect("valid operation plan");
        let mut transaction = Transaction::begin(op_project).expect("transaction");
        let mut kernel = SetInteractionKernel;
        let receipt = transaction
            .apply_all(&validated, &mut kernel)
            .expect("apply staged operation");
        assert!(receipt.outcomes[0].changed);
        assert_eq!(receipt.outcomes[0].created_handles.len(), 1);
        assert!(receipt.outcomes[0].created_handles[0].starts_with("interaction:"));
        let staged = tree(transaction.work_dir());
        assert_ne!(staged, before, "operation must change the staged tree");
        let out = temp.path().join("op-applied");
        transaction
            .commit_out_dir(&out, false)
            .expect("commit staged operation");
        assert_eq!(tree(&cli_project.project_dir), tree(&out));
    }

    #[test]
    fn applying_set_interaction_twice_is_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold(&temp.path().join("project"));
        let operation = operation_for(&project);
        let mut transaction = Transaction::begin(project).expect("transaction");
        let mut kernel = SetInteractionKernel;
        let first = kernel
            .apply(&Op::SetInteraction(operation.clone()), &mut transaction)
            .expect("first apply");
        let second = kernel
            .apply(&Op::SetInteraction(operation), &mut transaction)
            .expect("second apply");
        assert!(first.changed);
        assert!(!second.changed);
    }

    #[test]
    fn default_interaction_refusal_uses_registered_unsupported_code() {
        let operation = SetInteraction {
            page: "page:Overview".to_string(),
            source: "visual:Overview:Source".to_string(),
            target: "visual:Overview:Target".to_string(),
            interaction_type: "Default".to_string(),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold(&temp.path().join("project"));
        let mut transaction = Transaction::begin(project).expect("transaction");
        let mut kernel = SetInteractionKernel;
        let error = kernel
            .apply(&Op::SetInteraction(operation), &mut transaction)
            .expect_err("Default must remain unsupported");
        assert_eq!(error.code, "unsupported_feature");
    }
}
