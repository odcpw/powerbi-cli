mod common;

use common::{run_powerbi, stdout_json};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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

fn visual(project: &Path, page: &str, name: &str) -> Value {
    let path = project
        .join("SlicerRail.Report")
        .join("definition")
        .join("pages")
        .join(page)
        .join("visuals")
        .join(name)
        .join("visual.json");
    serde_json::from_str(&fs::read_to_string(path).expect("visual json")).expect("parse visual")
}

fn build(project: &Path, profile: bool) -> common::CliRun {
    let mut args = vec!["report", "build", "--schema", "examples/sales.schema.json"];
    if profile {
        args.extend(["--profile", "examples/archetypes/slicer-rail.profile.json"]);
    }
    args.extend([
        "--spec",
        "examples/archetypes/slicer-rail.dashboard.json",
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    run_powerbi(&args)
}

#[test]
fn v2_rail_slicers_emit_typed_add_visuals_with_stable_handles_and_coordinates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let first_output = build(&first, true);
    assert_eq!(first_output.code, 0, "stderr: {}", first_output.stderr);
    let first_json = stdout_json(&first_output);
    assert_eq!(first_json["compiled"]["counts"]["visuals"], 7);
    assert!(
        first_json["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .any(|operation| operation["kind"] == "addVisual"
                && operation["handle"]
                    == "visual:ReportSectionOverview:VisualContainerRailCustomer"
                && operation["mode"] == "Dropdown")
    );

    let overview_segment = visual(
        &first,
        "ReportSectionOverview",
        "VisualContainerRailSegment",
    );
    let detail_segment = visual(&first, "ReportSectionDetail", "VisualContainerRailSegment");
    assert_eq!(overview_segment["position"], detail_segment["position"]);
    assert_eq!(overview_segment["position"]["height"], 76.0);
    assert_eq!(overview_segment["position"]["width"], 296.0);
    assert_eq!(overview_segment["position"]["x"], 960.0);
    assert_eq!(overview_segment["position"]["y"], 80.0);
    assert_eq!(
        visual(
            &first,
            "ReportSectionOverview",
            "VisualContainerRailCustomer"
        )["visual"]["objects"]["data"][0]["properties"]["mode"]["expr"]["Literal"]["Value"],
        "'Dropdown'"
    );
    assert_eq!(
        visual(
            &first,
            "ReportSectionOverview",
            "VisualContainerRailCustomer"
        )["visual"]["objects"]["selection"][0]["properties"]["singleSelect"]["expr"]["Literal"]["Value"],
        "true"
    );
    assert_eq!(
        visual(&first, "ReportSectionOverview", "VisualContainerRailDate")["position"]["height"],
        104.0
    );
    assert_eq!(
        visual(&first, "ReportSectionOverview", "VisualContainerRailDate")["position"]["y"],
        264.0
    );

    let second_output = build(&second, true);
    assert_eq!(second_output.code, 0, "stderr: {}", second_output.stderr);
    assert_eq!(tree(&first), tree(&second));
}

#[test]
fn v2_rail_without_profile_defaults_basic_and_reports_pending_planner_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("no-profile");
    let output = build(&project, false);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert!(
        value["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning["code"] == "spec.feature_pending"
                && warning["owningBead"] == "pbi-t6-planner-v2-szr.1"
                && warning["field"] == "DimCustomer[Segment]")
    );
    assert_eq!(
        visual(
            &project,
            "ReportSectionOverview",
            "VisualContainerRailCustomer"
        )["visual"]["objects"]["data"][0]["properties"]["mode"]["expr"]["Literal"]["Value"],
        "'Basic'"
    );
}

#[test]
fn v2_rail_unknown_fields_are_refused_without_writing_a_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/archetypes/slicer-rail.dashboard.json")
            .expect("fixture spec"),
    )
    .expect("parse fixture spec");
    spec["layout"]["rail"]["unexpected"] = Value::Bool(true);
    let spec_path = temp.path().join("unknown.dashboard.json");
    fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("serialize spec"),
    )
    .expect("write spec");
    let project = temp.path().join("must-not-exist");
    let output = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec_path.to_str().expect("spec path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let error: Value = serde_json::from_str(output.stdout.trim()).expect("stdout error");
    assert_eq!(error["errors"][0]["code"], "spec.unknown_field");
    assert_eq!(error["errors"][0]["pointer"], "/layout/rail/unexpected");
    assert!(!project.exists());
}
