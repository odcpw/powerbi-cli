//! Typed operation kernel for builtin report theme presets.

use super::{ApplyThemePreset, Op, OpKernel, OpOutcome, Transaction};
use crate::report_themes;
use crate::{CliError, CliResult};

/// Applies [`ApplyThemePreset`] operations to a transaction's staged PBIP tree.
#[derive(Debug, Default)]
pub(crate) struct ApplyThemePresetKernel;

impl OpKernel for ApplyThemePresetKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let Op::ApplyThemePreset(payload) = operation else {
            return Err(CliError::invalid_args(
                "applyThemePreset kernel received a different operation",
            ));
        };
        let project = transaction.working_project()?;
        report_themes::apply_theme_preset_operation(payload, &project)
    }
}

impl ApplyThemePresetKernel {
    /// Parse argv-shaped preset options into the typed operation and mode.
    pub(crate) fn parse_args(
        args: &[String],
    ) -> CliResult<(ApplyThemePreset, crate::cli_support::MutationMode)> {
        report_themes::parse_apply_theme_preset_operation(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{OpPlan, ProjectIndex};
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
    fn parse_args_round_trips_builtin_preset_and_shared_mode() {
        let args = ["--preset", "neutral-ops", "--dry-run"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let (payload, mode) = ApplyThemePresetKernel::parse_args(&args).expect("parse operation");
        assert_eq!(mode, crate::cli_support::MutationMode::DryRun);
        assert_eq!(payload.preset, "neutral-ops");
        let encoded = serde_json::to_value(Op::ApplyThemePreset(payload)).expect("op json");
        assert_eq!(encoded["op"], "applyThemePreset");
    }

    #[test]
    fn staged_kernel_matches_cli_tree_and_declares_report_theme_handle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cli_project = scaffold(&temp.path().join("cli"));
        let op_project = scaffold(&temp.path().join("op"));
        crate::report_themes::themes_command(&[
            "apply-preset".to_string(),
            "--project".to_string(),
            cli_project.project_dir.to_string_lossy().into_owned(),
            "--preset".to_string(),
            "risk-dashboard".to_string(),
            "--in-place".to_string(),
        ])
        .expect("CLI mutation");
        let before = tree(&op_project.project_dir);
        let operation = ApplyThemePreset {
            preset: "risk-dashboard".to_string(),
        };
        let plan = OpPlan::new(vec![Op::ApplyThemePreset(operation)]);
        let index = ProjectIndex::from_project(&op_project).expect("project handles");
        let validated = plan.validate(&index).expect("valid operation plan");
        let mut transaction = Transaction::begin(op_project).expect("transaction");
        let mut kernel = ApplyThemePresetKernel;
        let receipt = transaction
            .apply_all(&validated, &mut kernel)
            .expect("apply staged operation");
        assert!(receipt.outcomes[0].changed);
        assert_eq!(receipt.outcomes[0].created_handles, ["theme:report"]);
        let staged = tree(transaction.work_dir());
        assert_ne!(staged, before, "operation must change the staged tree");
        let out = temp.path().join("op-applied");
        transaction
            .commit_out_dir(&out, false)
            .expect("commit staged operation");
        assert_eq!(tree(&cli_project.project_dir), tree(&out));
    }

    #[test]
    fn applying_theme_preset_twice_is_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold(&temp.path().join("project"));
        let operation = ApplyThemePreset {
            preset: "risk-dashboard".to_string(),
        };
        let mut transaction = Transaction::begin(project).expect("transaction");
        let mut kernel = ApplyThemePresetKernel;
        let first = kernel
            .apply(&Op::ApplyThemePreset(operation.clone()), &mut transaction)
            .expect("first apply");
        let second = kernel
            .apply(&Op::ApplyThemePreset(operation), &mut transaction)
            .expect("second apply");
        assert!(first.changed);
        assert!(!second.changed);
    }

    #[test]
    fn unknown_theme_preset_refusal_preserves_invalid_args_code() {
        let operation = ApplyThemePreset {
            preset: "missing-preset".to_string(),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold(&temp.path().join("project"));
        let mut transaction = Transaction::begin(project).expect("transaction");
        let mut kernel = ApplyThemePresetKernel;
        let error = kernel
            .apply(&Op::ApplyThemePreset(operation), &mut transaction)
            .expect_err("unknown preset must fail");
        assert_eq!(error.code, "invalid_args");
    }
}
