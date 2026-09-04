//! Typed operation kernel for restoring a visual pair's default interaction.

use super::{Op, OpKernel, OpOutcome, ResetInteraction, Transaction};
use crate::report_interaction_mutations;
use crate::{CliError, CliResult};

/// Applies [`ResetInteraction`] operations to a transaction's staged PBIP tree.
#[derive(Debug, Default)]
pub(crate) struct ResetInteractionKernel;

impl OpKernel for ResetInteractionKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let Op::ResetInteraction(payload) = operation else {
            return Err(CliError::invalid_args(
                "resetInteraction kernel received a different operation",
            ));
        };
        let project = transaction.working_project()?;
        report_interaction_mutations::apply_reset_interaction_operation(payload, &project)
    }
}

impl ResetInteractionKernel {
    /// Parse argv-shaped endpoint selectors into the typed operation and mode.
    pub(crate) fn parse_args(
        args: &[String],
    ) -> CliResult<(ResetInteraction, crate::cli_support::MutationMode)> {
        report_interaction_mutations::parse_reset_interaction_operation(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_support::MutationMode;
    use crate::ops::{Op, ProjectIndex, Transaction};
    use crate::pbir::load_report_snapshot;
    use crate::{ResolvedProject, resolve_project, scaffold_schema_value};
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;

    fn scaffold(root: &Path) -> ResolvedProject {
        let schema: Value = serde_json::from_str(include_str!("../../examples/sales.schema.json"))
            .expect("sales schema");
        scaffold_schema_value(schema, Path::new("examples/sales.schema.json"), root, false)
            .expect("scaffold project");
        resolve_project(root).expect("resolve project")
    }

    fn operation_for(project: &ResolvedProject) -> ResetInteraction {
        let snapshot = load_report_snapshot(project).expect("report snapshot");
        let page = snapshot.pages.first().expect("sales page");
        let source = page.visuals.first().expect("source visual");
        let target = page.visuals.get(1).expect("target visual");
        let page_path = page.path.as_ref().expect("page path");
        let mut page_json: Value =
            serde_json::from_str(&fs::read_to_string(page_path).expect("page json"))
                .expect("page json value");
        page_json["visualInteractions"] = json!([{
            "source": source.name,
            "target": target.name,
            "type": "NoFilter"
        }]);
        fs::write(
            page_path,
            serde_json::to_string_pretty(&page_json).expect("page json text"),
        )
        .expect("write interaction fixture");
        ResetInteraction {
            page: page.handle.clone(),
            source: source.handle.clone(),
            target: target.handle.clone(),
        }
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
            "--dry-run",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();

        let (payload, mode) = ResetInteractionKernel::parse_args(&args).expect("parse operation");
        assert_eq!(mode, MutationMode::DryRun);
        assert_eq!(payload.page, "page:ReportSectionOverview");
        assert_eq!(
            payload.source,
            "visual:ReportSectionOverview:VisualContainerRevenue"
        );
        assert_eq!(
            payload.target,
            "visual:ReportSectionOverview:VisualContainerTable"
        );
        let encoded = serde_json::to_value(Op::ResetInteraction(payload)).expect("op json");
        assert_eq!(encoded["op"], "resetInteraction");
    }

    #[test]
    fn staged_kernel_removes_row_and_replays_as_idempotent_noop() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold(&temp.path().join("project"));
        let operation = operation_for(&project);
        let index = ProjectIndex::from_project(&project).expect("project handles");
        let plan = crate::ops::OpPlan::new(vec![Op::ResetInteraction(operation.clone())]);
        let validated = plan.validate(&index).expect("valid reset operation");
        let mut transaction = Transaction::begin(project).expect("transaction");
        let mut kernel = ResetInteractionKernel;
        let first = kernel
            .apply(&Op::ResetInteraction(operation.clone()), &mut transaction)
            .expect("first apply");
        let second = kernel
            .apply(&Op::ResetInteraction(operation), &mut transaction)
            .expect("second apply");
        assert!(first.changed);
        assert!(!second.changed);
        assert_eq!(first.changes[0]["action"], "remove");
        assert_eq!(second.changes[0]["action"], "noop");
        transaction
            .validate_working_copy()
            .expect("reset working copy validation");
        assert_eq!(validated.ops.len(), 1);
    }
}
