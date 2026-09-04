mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn write_spec(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_string_pretty(value).expect("serialize dashboard spec"),
    )
    .expect("write dashboard spec");
}

fn minimal_spec() -> Value {
    json!({
        "schema": "powerbi-cli.dashboard.v1",
        "report": {"name": "StrictSpec"},
        "pages": []
    })
}

#[test]
fn spec_validate_rejects_misplaced_root_measure_with_structured_diagnostic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("bad.dashboard.json");
    let mut spec = minimal_spec();
    spec["measures"] = json!([]);
    write_spec(&path, &spec);

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
    let value = stdout_json(&output);
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["errors"][0]["code"], "spec.unknown_field");
    assert_eq!(value["errors"][0]["pointer"], "/measures");
    assert_eq!(value["errors"][0]["didYouMean"], "model.measures");
}

#[test]
fn shape_only_spec_validation_rejects_nested_unknown_key_with_pointer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("bad-nested.dashboard.json");
    let mut spec = minimal_spec();
    spec["pages"] = json!([{
        "id": "overview",
        "visuals": [
            {"id": "first", "type": "card", "bindings": []},
            {"id": "second", "type": "card", "colour": "red", "bindings": []}
        ]
    }]);
    write_spec(&path, &spec);

    let output = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--spec",
        path.to_str().expect("spec path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["validationLevel"], "shape-only");
    assert_eq!(value["errors"][0]["code"], "spec.unknown_field");
    assert_eq!(value["errors"][0]["pointer"], "/pages/0/visuals/1/colour");
    assert!(value["errors"][0].get("didYouMean").is_none());
}

#[test]
fn report_build_rejects_unknown_key_before_creating_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("bad.dashboard.json");
    let out = temp.path().join("must-not-exist");
    let mut spec = minimal_spec();
    spec["measures"] = json!([]);
    write_spec(&path, &spec);

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
    assert_eq!(output.code, 10, "stdout: {}", output.stdout);
    let value = stderr_json(&output);
    assert_eq!(value["error"]["code"], "spec.unknown_field");
    assert_eq!(value["error"]["pointer"], "/measures");
    assert_eq!(value["error"]["didYouMean"], "model.measures");
    assert!(!out.exists(), "failed build must not create output");
}

#[test]
fn recognized_uncompiled_v1_sections_remain_unsupported_features() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("style.dashboard.json");
    let mut spec = minimal_spec();
    spec["style"] = json!({"preset": "neutral"});
    write_spec(&path, &spec);

    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        path.to_str().expect("spec path"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 2);
    assert_eq!(stderr_json(&output)["error"]["code"], "unsupported_feature");
}

#[test]
fn spec_fields_without_schema_lists_every_v1_node() {
    let output = run_powerbi(&["report", "spec", "fields", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let nodes = value["allowedFields"]
        .as_array()
        .expect("allowedFields")
        .iter()
        .map(|node| node["node"].as_str().expect("node name"))
        .collect::<Vec<_>>();
    for expected in [
        "root",
        "report",
        "model",
        "model.measures[]",
        "pages[]",
        "pages[].size",
        "pages[].visuals[]",
        "pages[].visuals[].layout",
        "pages[].visuals[].bindings[]",
        "pages[].interactions[]",
    ] {
        assert!(nodes.contains(&expected), "missing node {expected}");
    }
}
