//! Shared runners for CLI/operation and metamorphic artifact comparisons.

use super::{
    ArchetypeFixture, CliRun, assert_tree_equal, assert_tree_equal_with_ignored, load_archetype,
    run_powerbi_owned,
};
use powerbi_cli::test_support;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The result of one CLI path and one typed-operation path over the same
/// fixture.
#[derive(Debug)]
pub struct OperationExecution {
    pub operation: Value,
    pub cli: CliRun,
    pub op: DirectOperationRun,
    pub cli_tree: PathBuf,
    pub op_tree: PathBuf,
}

/// Process-like result for the in-process typed operation bridge.
#[derive(Debug)]
pub struct DirectOperationRun {
    pub argv: Vec<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
    pub elapsed: Duration,
}

/// A table row for one registered operation kernel.
pub struct OperationEquivalenceCase {
    pub name: &'static str,
    pub fixture: &'static str,
    pub operation_tag: &'static str,
    pub execute: fn(&ArchetypeFixture, &Path) -> OperationExecution,
}

/// The result of compiling a spec fragment and applying its equivalent op.
#[derive(Debug)]
pub struct MetamorphicExecution {
    pub fragment: Value,
    pub operation: Value,
    pub spec_build: CliRun,
    pub base_build: CliRun,
    pub op: DirectOperationRun,
    pub spec_tree: PathBuf,
    pub applied_tree: PathBuf,
}

/// A table row for one spec section and its operation equivalent.
pub struct MetamorphicCase {
    pub name: &'static str,
    pub fixture: &'static str,
    pub fragment_pointer: &'static str,
    pub operation_tag: &'static str,
    pub execute: fn(&ArchetypeFixture, &Path) -> MetamorphicExecution,
}

/// Run every operation-equivalence row and fail with both trees plus the first
/// differing file when a kernel and CLI diverge.
pub fn run_operation_equivalence(cases: &[OperationEquivalenceCase]) {
    assert!(
        !cases.is_empty(),
        "operation equivalence case table is empty"
    );
    let registered = test_support::registered_kernel_tags();
    let registered = registered
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        test_support::registered_kernel_tags().len(),
        registered.len(),
        "operation kernel registry contains duplicate tags"
    );
    let covered = cases
        .iter()
        .map(|case| case.operation_tag)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        cases.len(),
        covered.len(),
        "operation equivalence case table contains duplicate operation tags"
    );
    for tag in &registered {
        assert!(
            covered.contains(tag),
            "registered operation kernel {tag} has no equivalence case"
        );
    }
    for tag in &covered {
        assert!(
            registered.contains(tag),
            "equivalence case {tag} has no registered operation kernel"
        );
    }

    for case in cases {
        assert_eq!(
            case.operation_tag,
            case.operation_tag.trim(),
            "{} has an empty or padded operation tag",
            case.name
        );
        let fixture = load_archetype(case.fixture);
        let workspace = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("create {} operation tempdir: {error}", case.name));
        let execution = (case.execute)(&fixture, workspace.path());
        assert_eq!(
            execution.operation["op"].as_str(),
            Some(case.operation_tag),
            "{} operation payload has the wrong op tag",
            case.name
        );
        assert_eq!(
            execution.cli.exit, execution.op.exit,
            "{} CLI and operation exit codes differ (CLI stderr: {}; op stderr: {})",
            case.name, execution.cli.stderr, execution.op.stderr
        );
        assert_eq!(
            execution.cli.stderr, execution.op.stderr,
            "{} CLI and operation diagnostics differ",
            case.name
        );
        assert!(
            serde_json::from_str::<Value>(execution.cli.stdout.trim()).is_ok(),
            "{} CLI stdout is not JSON",
            case.name
        );
        assert!(
            serde_json::from_str::<Value>(execution.op.stdout.trim()).is_ok(),
            "{} operation receipt is not JSON",
            case.name
        );
        assert_tree_equal(
            &execution.cli_tree,
            &execution.op_tree,
            &format!("operation equivalence {}", case.name),
        );
    }
}

/// Run every spec/op metamorphic row and compare the generated project tree
/// with the tree produced by applying the typed operation to a base build.
pub fn run_metamorphic_cases(cases: &[MetamorphicCase]) {
    assert!(!cases.is_empty(), "metamorphic case table is empty");
    let registered = test_support::registered_kernel_tags();
    for case in cases {
        assert!(
            registered.contains(&case.operation_tag),
            "{} uses an operation without a registered kernel: {}",
            case.name,
            case.operation_tag
        );
        assert!(
            case.fragment_pointer.starts_with('/'),
            "{} fragment pointer must be a JSON pointer",
            case.name
        );
        let fixture = load_archetype(case.fixture);
        let workspace = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("create {} metamorphic tempdir: {error}", case.name));
        let execution = (case.execute)(&fixture, workspace.path());
        assert_eq!(
            execution.operation["op"].as_str(),
            Some(case.operation_tag),
            "{} operation payload has the wrong op tag",
            case.name
        );
        assert!(
            !execution.fragment.is_null(),
            "{} spec fragment must be present",
            case.name
        );
        assert_eq!(
            execution.spec_build.exit, 0,
            "{} spec+fragment build failed: {}",
            case.name, execution.spec_build.stderr
        );
        assert_eq!(
            execution.base_build.exit, 0,
            "{} base build failed: {}",
            case.name, execution.base_build.stderr
        );
        assert_eq!(
            execution.op.exit, 0,
            "{} operation application failed: {}",
            case.name, execution.op.stderr
        );
        assert!(
            serde_json::from_str::<Value>(execution.spec_build.stdout.trim()).is_ok(),
            "{} spec+fragment build did not return JSON",
            case.name
        );
        assert!(
            serde_json::from_str::<Value>(execution.base_build.stdout.trim()).is_ok(),
            "{} base build did not return JSON",
            case.name
        );
        assert_tree_equal_with_ignored(
            &execution.spec_tree,
            &execution.applied_tree,
            &format!("metamorphic {}", case.name),
            &["powerbi-cli.manifest.copy.json"],
        );
    }
}

/// Run a typed operation against a source project and commit it to a fresh
/// output directory. The receipt is represented in the same process-like
/// shape as CliRun so generic assertions can compare diagnostics and status.
pub fn run_direct_operation(
    operation: &Value,
    project: &Path,
    out_dir: &Path,
) -> DirectOperationRun {
    let started = Instant::now();
    let argv = vec![
        "<typed-operation>".to_string(),
        operation["op"].as_str().unwrap_or("<unknown>").to_string(),
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        out_dir.to_string_lossy().into_owned(),
    ];
    let result = test_support::apply_operation_to_out_dir(operation.clone(), project, out_dir);
    let elapsed = started.elapsed();
    let run = match result {
        Ok(value) => DirectOperationRun {
            argv,
            stdout: serde_json::to_string_pretty(&value).expect("serialize operation receipt")
                + "\n",
            stderr: String::new(),
            exit: 0,
            elapsed,
        },
        Err(error) => DirectOperationRun {
            argv,
            stdout: String::new(),
            stderr: serde_json::to_string(&error).expect("serialize operation error") + "\n",
            exit: error["error"]["exitCode"].as_i64().unwrap_or(70) as i32,
            elapsed,
        },
    };
    if std::env::var("POWERBI_CLI_TEST_LOG").as_deref() == Ok("1") {
        eprintln!(
            "{}",
            serde_json::json!({
                "schema": "powerbi-cli.test-run.v1",
                "argv": run.argv,
                "operation": operation,
                "stdout": run.stdout,
                "stderr": run.stderr,
                "exit": run.exit,
                "elapsedMs": run.elapsed.as_millis(),
                "path": "typed-operation"
            })
        );
    }
    run
}

/// Scaffold one repository-backed fixture into a clean project directory.
pub fn scaffold_fixture(fixture: &ArchetypeFixture, out_dir: &Path) -> CliRun {
    let output = run_powerbi_owned(&[
        "scaffold".to_string(),
        "--schema".to_string(),
        fixture.schema.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        out_dir.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    assert_eq!(
        output.exit, 0,
        "scaffold fixture {} failed: {}",
        fixture.name, output.stderr
    );
    output
}

/// Build one fixture with an explicitly authored dashboard spec.
pub fn build_fixture_with_spec(fixture: &ArchetypeFixture, spec: &Path, out_dir: &Path) -> CliRun {
    let output = run_powerbi_owned(&[
        "report".to_string(),
        "build".to_string(),
        "--schema".to_string(),
        fixture.schema.to_string_lossy().into_owned(),
        "--profile".to_string(),
        fixture.profile.to_string_lossy().into_owned(),
        "--spec".to_string(),
        spec.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        out_dir.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    assert_eq!(
        output.exit, 0,
        "build fixture {} failed: {}",
        fixture.name, output.stderr
    );
    output
}
