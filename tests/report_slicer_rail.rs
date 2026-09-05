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

fn report_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    tree(&root.join("SlicerRail.Report"))
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
fn registered_slicer_rail_fixture_matches_its_checked_in_summary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = common::load_archetype("slicer-rail");
    let project = temp.path().join("project");
    let built = fixture.build_into(&project);
    assert_eq!(built.code, 0, "{}", built.stderr);
    let verified = run_powerbi(&[
        "fixture",
        "verify",
        project.to_str().unwrap(),
        "--expected",
        fixture.expected_summary.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(verified.code, 0, "{}", verified.stderr);
    assert_eq!(stdout_json(&verified)["verification"]["same"], true);
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
            .any(|operation| operation["op"] == "addVisual"
                && operation["handle"]
                    == "visual:ReportSectionOverview:VisualContainerRailCustomer"
                && operation["mode"] == "Dropdown")
    );
    let segment_operation = first_json["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .find(|operation| {
            operation["handle"] == "visual:ReportSectionOverview:VisualContainerRailSegment"
        })
        .expect("segment AddVisual operation");
    assert_eq!(segment_operation["position"]["x"], 960.0);
    assert_eq!(segment_operation["position"]["y"], 80.0);
    assert_eq!(segment_operation["position"]["width"], 296.0);
    assert_eq!(segment_operation["position"]["height"], 76.0);

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
fn v2_slicer_rail_build_matches_the_corresponding_add_visual_mutations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let compiled = temp.path().join("compiled");
    let mutated = temp.path().join("mutated");
    let compiled_output = build(&compiled, true);
    assert_eq!(
        compiled_output.code, 0,
        "stderr: {}",
        compiled_output.stderr
    );

    let mut base_spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/archetypes/slicer-rail.dashboard.json")
            .expect("fixture spec"),
    )
    .expect("parse fixture spec");
    base_spec
        .as_object_mut()
        .expect("spec object")
        .remove("layout");
    for page in base_spec["pages"].as_array_mut().expect("pages") {
        let page = page.as_object_mut().expect("page object");
        page.remove("rail");
        page.remove("slicers");
    }
    let base_spec_path = temp.path().join("without-slicers.dashboard.json");
    fs::write(
        &base_spec_path,
        serde_json::to_vec_pretty(&base_spec).expect("serialize base spec"),
    )
    .expect("write base spec");
    let base = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/archetypes/slicer-rail.profile.json",
        "--spec",
        base_spec_path.to_str().expect("base spec path"),
        "--out-dir",
        mutated.to_str().expect("mutated project path"),
        "--json",
    ]);
    assert_eq!(base.code, 0, "stderr: {}", base.stderr);

    let additions = [
        (
            "ReportSectionOverview",
            "DimCustomer.Segment",
            "Segment",
            "VisualContainerRailSegment",
            "Basic",
            false,
            960.0,
            80.0,
            76.0,
        ),
        (
            "ReportSectionOverview",
            "DimCustomer.CustomerName",
            "Customer",
            "VisualContainerRailCustomer",
            "Dropdown",
            true,
            960.0,
            172.0,
            76.0,
        ),
        (
            "ReportSectionDetail",
            "DimCustomer.Segment",
            "Segment",
            "VisualContainerRailSegment",
            "Basic",
            false,
            960.0,
            80.0,
            76.0,
        ),
        (
            "ReportSectionDetail",
            "DimCustomer.CustomerName",
            "Customer",
            "VisualContainerRailCustomer",
            "Dropdown",
            true,
            960.0,
            172.0,
            76.0,
        ),
    ];
    for (page, field, title, name, mode, single_select, x, y, height) in additions {
        let mut args = vec![
            "report".to_string(),
            "visuals".to_string(),
            "add-slicer".to_string(),
            "--project".to_string(),
            mutated.to_string_lossy().into_owned(),
            "--page".to_string(),
            format!("page:{page}"),
            "--field".to_string(),
            field.to_string(),
            "--title".to_string(),
            title.to_string(),
            "--name".to_string(),
            name.to_string(),
            "--mode".to_string(),
            mode.to_string(),
            "--x".to_string(),
            x.to_string(),
            "--y".to_string(),
            y.to_string(),
            "--width".to_string(),
            "296".to_string(),
            "--height".to_string(),
            height.to_string(),
        ];
        if single_select {
            args.push("--single-select".to_string());
        }
        args.extend(["--in-place".to_string(), "--json".to_string()]);
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let output = run_powerbi(&refs);
        assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    }

    let between = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        mutated.to_str().expect("mutated project path"),
        "--page",
        "page:ReportSectionOverview",
        "--visual-type",
        "slicer",
        "--title",
        "Date",
        "--name",
        "VisualContainerRailDate",
        "--mode",
        "between",
        "--x",
        "960",
        "--y",
        "264",
        "--width",
        "296",
        "--height",
        "104",
        "--binding",
        "role=Values,table=DimDate,column=Date",
        "--in-place",
        "--json",
    ]);
    assert_eq!(between.code, 0, "stderr: {}", between.stderr);

    let compiled_tree = report_tree(&compiled);
    let mutated_tree = report_tree(&mutated);
    assert_eq!(
        compiled_tree.keys().collect::<Vec<_>>(),
        mutated_tree.keys().collect::<Vec<_>>()
    );
    for (path, compiled_bytes) in compiled_tree {
        let mutated_bytes = &mutated_tree[&path];
        if compiled_bytes != *mutated_bytes {
            let offset = compiled_bytes
                .iter()
                .zip(mutated_bytes)
                .position(|(left, right)| left != right)
                .unwrap_or(compiled_bytes.len().min(mutated_bytes.len()));
            let start = offset.saturating_sub(40);
            let compiled_end = (offset + 80).min(compiled_bytes.len());
            let mutated_end = (offset + 80).min(mutated_bytes.len());
            panic!(
                "compiler and mutation artifact differ at {} byte {offset}: compiled={:?}, mutated={:?}",
                path.display(),
                String::from_utf8_lossy(&compiled_bytes[start..compiled_end]),
                String::from_utf8_lossy(&mutated_bytes[start..mutated_end])
            );
        }
    }
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

#[test]
fn v2_rail_refuses_slicers_that_overflow_the_available_height() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/archetypes/slicer-rail.dashboard.json")
            .expect("fixture spec"),
    )
    .expect("parse fixture spec");
    let slicer = serde_json::json!({
        "field": "DimCustomer[Segment]",
        "title": "Overflow"
    });
    spec["layout"]["rail"]["slicers"] = Value::Array(vec![slicer; 8]);
    let spec_path = temp.path().join("overflow.dashboard.json");
    fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("serialize spec"),
    )
    .expect("write spec");
    let project = temp.path().join("must-not-exist");
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/archetypes/slicer-rail.profile.json",
        "--spec",
        spec_path.to_str().expect("spec path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 2, "stderr: {}", output.stderr);
    let error: Value = serde_json::from_str(output.stderr.trim()).expect("stderr error");
    assert_eq!(error["error"]["code"], "invalid_args");
    assert_eq!(error["error"]["pointer"], "/layout/rail/slicers/6");
    assert!(!project.exists());
}
