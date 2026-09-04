mod common;

use common::{run_powerbi, stdout_json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[test]
fn report_build_response_exposes_aggregate_handles_and_opt_in_trace() {
    let without_trace = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(without_trace.code, 0, "stderr: {}", without_trace.stderr);
    let value = stdout_json(&without_trace);
    assert_eq!(value["schema"], "powerbi-cli.report.build.v1");
    assert!(
        value["changes"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    assert!(
        value["compiled"]["ops"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert_eq!(
        value["scope"]["operationCount"], value["compiled"]["ops"],
        "scope and compiled operation counts must agree"
    );
    assert!(value["readback"]["report:main"].as_array().is_some());
    assert!(
        value["readback"]["page:ReportSectionOverview"]
            .as_array()
            .is_some()
    );
    assert!(
        value["readback"]["visual:ReportSectionOverview:VisualContainerRevenue"]
            .as_array()
            .is_some()
    );
    assert!(value.get("trace").is_none(), "trace is opt-in");

    let with_trace = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--dry-run",
        "--trace",
        "--json",
    ]);
    assert_eq!(with_trace.code, 0, "stderr: {}", with_trace.stderr);
    let traced = stdout_json(&with_trace);
    let trace = traced["trace"].as_array().expect("trace array");
    assert_eq!(
        trace.len(),
        traced["compiled"]["ops"].as_u64().unwrap() as usize
    );
    assert!(
        trace
            .iter()
            .all(|entry| { entry["op"].as_str().is_some() && entry["ms"].as_u64() == Some(0) })
    );

    let repeated = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--dry-run",
        "--trace",
        "--json",
    ]);
    assert_eq!(
        with_trace.stdout, repeated.stdout,
        "trace output must remain byte-deterministic"
    );
}

#[test]
fn report_build_scorecard_is_shared_with_triage() {
    let root = tempfile::tempdir().expect("tempdir");
    let project = root.path().join("sales");
    let project_arg = project.to_str().expect("project path");
    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        project_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);
    let build_json = stdout_json(&build);
    let triage = run_powerbi(&["triage", project_arg, "--json"]);
    assert_eq!(triage.code, 0, "stderr: {}", triage.stderr);
    let triage_json = stdout_json(&triage);
    assert_eq!(build_json["scorecard"], triage_json["scorecard"]);
    assert_eq!(build_json["scorecard"]["schema"], "scorecard.v1");
    assert_eq!(
        build_json["scorecard"]["designLint"]["status"],
        "unavailable"
    );
    assert!(build_json["scorecard"]["handoff"]["safeForOfflineHandoff"].is_boolean());
}

#[test]
fn report_build_artifacts_are_byte_identical_for_equivalent_outputs() {
    let root = tempfile::tempdir().expect("tempdir");
    let first = root.path().join("first");
    let second = root.path().join("second");
    build_project(&first);
    build_project(&second);
    assert_eq!(snapshot_files(&first), snapshot_files(&second));
}

fn build_project(project: &Path) {
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let relative = entry
                .path()
                .strip_prefix(root)
                .expect("relative artifact path")
                .to_path_buf();
            (relative, fs::read(entry.path()).expect("artifact bytes"))
        })
        .collect()
}
