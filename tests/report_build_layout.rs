mod common;

use common::{run_powerbi, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn write_spec(root: &Path, name: &str, spec: Value) -> PathBuf {
    let path = root.join(format!("{name}.dashboard.json"));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&spec).expect("serialize dashboard spec"),
    )
    .expect("write dashboard spec");
    path
}

fn layout_spec() -> Value {
    json!({
        "schema": "powerbi-cli.dashboard.v2",
        "report": {"name": "LayoutCompiler", "displayName": "Layout Compiler"},
        "style": {"tokens": {"typography": {"family": "Aptos Display", "scale": 1.2}}},
        "pages": [{
            "id": "overview",
            "template": "overview",
            "heading": "Revenue overview",
            "subtitle": "Offline-safe sample",
            "visuals": [
                {
                    "id": "revenue",
                    "type": "card",
                    "title": "Revenue",
                    "slot": "kpi.1",
                    "bindings": [{"role": "Values", "field": "FactSales[Total Revenue]"}]
                },
                {
                    "id": "trend",
                    "type": "lineChart",
                    "title": "Trend",
                    "slot": "primary",
                    "bindings": [
                        {"role": "Category", "field": "DimDate[Date]"},
                        {"role": "Y", "field": "FactSales[Total Revenue]"}
                    ]
                },
                {
                    "id": "detail",
                    "type": "tableEx",
                    "title": "Detail",
                    "slot": "detail",
                    "bindings": [
                        {"role": "Values", "field": "DimCustomer[CustomerName]"},
                        {"role": "Values", "field": "FactSales[Total Revenue]"}
                    ]
                }
            ]
        }]
    })
}

fn build(root: &Path, spec: &Value, name: &str) -> (PathBuf, Value) {
    let spec_path = write_spec(root, name, spec.clone());
    let out_dir = root.join(format!("{name}-project"));
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec_path.to_str().expect("spec path"),
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    (out_dir, stdout_json(&output))
}

fn read_visuals(project: &Path) -> Vec<Value> {
    let page = project.join("LayoutCompiler.Report/definition/pages/ReportSectionOverview/visuals");
    let mut paths = fs::read_dir(page)
        .expect("visual directory")
        .map(|entry| entry.expect("visual entry").path().join("visual.json"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            serde_json::from_slice(&fs::read(path).expect("visual json")).expect("parse visual")
        })
        .collect()
}

fn read_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, files: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(path).expect("read tree") {
            let entry = entry.expect("tree entry");
            let child = entry.path();
            if child.is_dir() {
                visit(root, &child, files);
            } else {
                let relative = child
                    .strip_prefix(root)
                    .expect("relative path")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative, fs::read(child).expect("tree bytes")));
            }
        }
    }
    let mut files = Vec::new();
    visit(root, root, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[test]
fn template_slots_and_headings_compile_to_grid_positions_and_styled_textboxes() {
    let root = tempfile::tempdir().expect("tempdir");
    let (project, response) = build(root.path(), &layout_spec(), "layout");
    assert_eq!(response["compiled"]["counts"]["visuals"], 5);
    assert!(
        response["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning["code"] == "feature_pending")
    );

    let visuals = read_visuals(&project);
    let revenue = visuals
        .iter()
        .find(|visual| {
            visual["name"]
                .as_str()
                .is_some_and(|name| name.contains("VisualContainerRevenue"))
        })
        .expect("revenue card");
    assert_eq!(revenue["position"]["x"], 232.0);
    assert_eq!(revenue["position"]["y"], 96.0);

    let headings = visuals
        .iter()
        .filter(|visual| visual["visual"]["visualType"] == "textbox")
        .collect::<Vec<_>>();
    assert_eq!(headings.len(), 2);
    for heading in headings {
        let run = &heading["visual"]["objects"]["general"][0]["properties"]["paragraphs"][0]["textRuns"]
            [0];
        assert_eq!(run["textStyle"]["fontFamily"], "Aptos Display");
        assert!(run["textStyle"]["fontSize"].as_str().is_some());
        assert!(heading["position"]["y"].as_f64().expect("heading y") < 80.0);
    }
}

#[test]
fn explicit_visual_layout_overrides_a_named_slot() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut spec = layout_spec();
    spec["pages"][0]["visuals"][0]["layout"] = json!({
        "x": 11,
        "y": 22,
        "width": 333,
        "height": 144
    });
    let (project, _) = build(root.path(), &spec, "explicit");
    let revenue = read_visuals(&project)
        .into_iter()
        .find(|visual| {
            visual["name"]
                .as_str()
                .is_some_and(|name| name.contains("VisualContainerRevenue"))
        })
        .expect("revenue card");
    assert_eq!(revenue["position"]["x"], 11.0);
    assert_eq!(revenue["position"]["y"], 22.0);
    assert_eq!(revenue["position"]["width"], 333.0);
    assert_eq!(revenue["position"]["height"], 144.0);
}

#[test]
fn unknown_and_duplicate_slots_are_pointer_rich_refusals() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = write_spec(root.path(), "unknown-slot", {
        let mut spec = layout_spec();
        spec["pages"][0]["visuals"][0]["slot"] = "not-a-slot".into();
        spec
    });
    let output = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        path.to_str().expect("spec path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let error = &stdout_json(&output)["errors"][0];
    assert_eq!(error["code"], "spec.missing_input");
    assert_eq!(error["pointer"], "/pages/0/visuals/0/slot");
    assert!(error["reason"].as_str().expect("reason").contains("kpi.1"));
    assert!(error["hint"].as_str().expect("hint").contains("primary"));

    let duplicate = write_spec(root.path(), "duplicate-slot", {
        let mut spec = layout_spec();
        spec["pages"][0]["visuals"][1]["slot"] = "kpi.1".into();
        spec
    });
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        duplicate.to_str().expect("spec path"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 2, "stdout: {}", output.stdout);
    let error: Value = serde_json::from_str(&output.stderr).expect("stderr json");
    assert_eq!(error["error"]["code"], "invalid_args");
    assert_eq!(error["error"]["pointer"], "/pages/0/visuals/1/slot");
}

#[test]
fn slot_coordinates_are_metamorphic_with_equivalent_explicit_layout() {
    let root = tempfile::tempdir().expect("tempdir");
    // Compare slot resolution with the same visuals expressed entirely as
    // explicit coordinates. Keep generated heading visuals out of both
    // artifacts so this checks only the layout boundary.
    let mut slot = layout_spec();
    slot["pages"][0]
        .as_object_mut()
        .expect("page")
        .remove("heading");
    slot["pages"][0]
        .as_object_mut()
        .expect("page")
        .remove("subtitle");
    let slot_path = write_spec(root.path(), "slot", slot);
    let mut explicit = layout_spec();
    explicit["pages"][0]
        .as_object_mut()
        .expect("page")
        .remove("template");
    explicit["pages"][0]
        .as_object_mut()
        .expect("page")
        .remove("heading");
    explicit["pages"][0]
        .as_object_mut()
        .expect("page")
        .remove("subtitle");
    explicit.as_object_mut().expect("spec").remove("style");
    for visual in explicit["pages"][0]["visuals"]
        .as_array_mut()
        .expect("visuals")
    {
        visual.as_object_mut().expect("visual").remove("slot");
    }
    explicit["pages"][0]["visuals"][0]["layout"] = json!({
        "x": 232.0, "y": 96.0, "width": 192.0, "height": 96.0
    });
    explicit["pages"][0]["visuals"][1]["layout"] = json!({
        "x": 232.0, "y": 208.0, "width": 504.0, "height": 240.0
    });
    explicit["pages"][0]["visuals"][2]["layout"] = json!({
        "x": 232.0, "y": 464.0, "width": 1024.0, "height": 216.0
    });
    let explicit_path = write_spec(root.path(), "explicit-equivalent", explicit);
    let slot_out = root.path().join("slot-project");
    let explicit_out = root.path().join("explicit-equivalent-project");
    for (path, out) in [(&slot_path, &slot_out), (&explicit_path, &explicit_out)] {
        let output = run_powerbi(&[
            "report",
            "build",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            path.to_str().expect("spec path"),
            "--out-dir",
            out.to_str().expect("output path"),
            "--json",
        ]);
        assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    }
    assert_eq!(read_tree(&slot_out), read_tree(&explicit_out));
}

#[test]
fn explain_exposes_template_slot_coordinates_and_generated_heading_operations() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = write_spec(root.path(), "explain-layout", layout_spec());
    let output = run_powerbi(&[
        "report",
        "spec",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        path.to_str().expect("spec path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let page = &value["layout"]["pages"][0];
    assert_eq!(page["template"]["name"], "overview");
    assert_eq!(page["resolvedSlots"][0]["name"], "heading");
    assert_eq!(page["resolvedSlots"][0]["position"]["x"], 232.0);
    assert_eq!(page["headings"].as_array().expect("headings").len(), 2);
    assert!(
        value["plan"]["ops"]
            .as_array()
            .expect("ops")
            .iter()
            .any(|operation| operation["operation"]["visualType"] == "textbox")
    );
}

#[test]
fn every_template_explain_matches_the_grid_snapshots_at_both_page_sizes() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshots: Value =
        serde_json::from_str(include_str!("snapshots/report-layout-all-templates.json"))
            .expect("grid snapshots");
    for (template, sizes) in snapshots.as_object().expect("templates") {
        for (size, expected) in sizes.as_object().expect("sizes") {
            let mut spec = layout_spec();
            spec["pages"][0]["template"] = template.clone().into();
            spec["pages"][0]["size"] = expected["pageSize"].clone();
            spec["pages"][0]["visuals"] = json!([]);
            let path = write_spec(root.path(), &format!("{template}-{size}"), spec);
            let output = run_powerbi(&[
                "report",
                "spec",
                "explain",
                "--schema",
                "examples/sales.schema.json",
                "--spec",
                path.to_str().expect("path"),
                "--json",
            ]);
            assert_eq!(output.code, 0, "{template}/{size}: {}", output.stderr);
            let value = stdout_json(&output);
            assert_eq!(
                value["layout"]["pages"][0]["resolvedSlots"], expected["slots"],
                "{template}/{size}"
            );
        }
    }
}

#[test]
fn template_build_is_deterministic_and_dry_run_preserves_existing_output() {
    let root = tempfile::tempdir().expect("tempdir");
    let spec = layout_spec();
    let (first, _) = build(root.path(), &spec, "first");
    let (second, _) = build(root.path(), &spec, "second");
    assert_eq!(read_tree(&first), read_tree(&second));
    let before = read_tree(root.path());
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        root.path()
            .join("first.dashboard.json")
            .to_str()
            .expect("path"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "{}", output.stderr);
    assert_eq!(read_tree(root.path()), before);
}

#[test]
fn unknown_template_and_missing_template_refuse_with_input_pointers() {
    let root = tempfile::tempdir().expect("tempdir");
    for (name, pointer) in [
        ("unknown", "/pages/0/template"),
        ("missing", "/pages/0/visuals/0/slot"),
    ] {
        let mut spec = layout_spec();
        if name == "unknown" {
            spec["pages"][0]["template"] = "nonexistent".into();
        } else {
            for key in ["template", "heading", "subtitle"] {
                spec["pages"][0].as_object_mut().expect("page").remove(key);
            }
        }
        let path = write_spec(root.path(), name, spec);
        let output = run_powerbi(&[
            "report",
            "build",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            path.to_str().expect("path"),
            "--dry-run",
            "--json",
        ]);
        assert_ne!(output.code, 0);
        let value: Value = serde_json::from_str(&output.stderr).expect("error JSON");
        assert_eq!(value["error"]["pointer"], pointer);
    }
}

#[test]
fn slot_family_mismatch_warns_without_refusing_the_visual() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut spec = layout_spec();
    spec["pages"][0]["visuals"][0]["slot"] = "secondary".into();
    let (_, response) = build(root.path(), &spec, "family");
    assert!(
        response["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning["code"] == "design.slot_family_mismatch"
                && warning["pointer"] == "/pages/0/visuals/0/slot")
    );
}

#[test]
fn invalid_typography_refuses_before_creating_a_project() {
    let root = tempfile::tempdir().expect("tempdir");
    for (index, typography, pointer) in [
        (0, json!({"family": " "}), "/style/tokens/typography/family"),
        (1, json!({"scale": 0}), "/style/tokens/typography/scale"),
        (2, json!({"scale": 1e308}), "/style/tokens/typography/scale"),
        (
            3,
            json!({"scale": 0.000001}),
            "/style/tokens/typography/scale",
        ),
    ] {
        let mut spec = layout_spec();
        spec["style"]["tokens"]["typography"] = typography;
        let path = write_spec(root.path(), &format!("invalid-{index}"), spec);
        let out = root.path().join(format!("invalid-{index}-project"));
        let output = run_powerbi(&[
            "report",
            "build",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            path.to_str().expect("path"),
            "--out-dir",
            out.to_str().expect("out"),
            "--json",
        ]);
        assert_ne!(output.code, 0);
        let value: Value = serde_json::from_str(&output.stderr).expect("error JSON");
        assert_eq!(value["error"]["pointer"], pointer);
        assert!(!out.exists());
    }
}
