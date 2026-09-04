mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

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

#[test]
fn v2_fixture_builds_byte_identically_to_its_v1_source() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v1 = temp.path().join("v1");
    let v2 = temp.path().join("v2");
    for (spec, out) in [
        ("examples/sales.dashboard.json", &v1),
        ("examples/sales.dashboard.v2.json", &v2),
    ] {
        let output = run_powerbi(&[
            "report",
            "build",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            spec,
            "--out-dir",
            out.to_str().expect("output path"),
            "--json",
        ]);
        assert_eq!(output.code, 0, "{spec}: {}", output.stderr);
    }
    assert_eq!(read_tree(&v1), read_tree(&v2));
}

#[test]
fn spec_fields_catalog_lists_every_v2_node() {
    let output = run_powerbi(&["report", "spec", "fields", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["supportedSpecVersions"],
        json!(["powerbi-cli.dashboard.v1", "powerbi-cli.dashboard.v2"])
    );
    let nodes = value["allowedFields"]
        .as_array()
        .expect("allowedFields")
        .iter()
        .map(|node| node["node"].as_str().expect("node name"))
        .collect::<Vec<_>>();
    for expected in [
        "model.measurePatterns[]",
        "model.calculatedColumns[]",
        "model.relationships[]",
        "model.staticTables[]",
        "style.tokens.semantic",
        "layout.rail.slicers[]",
        "filters[].relative",
        "pages[].slicers[]",
        "pages[].drillthrough",
        "pages[].visuals[].sort",
        "pages[].visuals[].drilldown",
        "pages[].visuals[].topnGuard",
        "pages[].visuals[].format",
        "proof.desktop.expectValues[]",
    ] {
        assert!(nodes.contains(&expected), "missing v2 node {expected}");
    }
    let allowed = value["allowedFields"].as_array().expect("allowedFields");
    for (node, expected_fields) in [
        (
            "root",
            &[
                "schema", "report", "model", "style", "layout", "filters", "pages", "proof",
            ][..],
        ),
        (
            "model",
            &[
                "measures",
                "measurePatterns",
                "calculatedColumns",
                "relationships",
                "staticTables",
                "dateTable",
                "sortBy",
                "formatStrings",
            ][..],
        ),
        ("style", &["preset", "bundle", "tokens", "defaults"][..]),
        ("layout", &["grid", "pageSize", "rail"][..]),
        (
            "pages[]",
            &[
                "id",
                "displayName",
                "size",
                "template",
                "heading",
                "subtitle",
                "filters",
                "slicers",
                "visuals",
                "interactions",
                "drillthrough",
                "tooltipFor",
            ][..],
        ),
        (
            "pages[].visuals[]",
            &[
                "id",
                "type",
                "title",
                "subtitle",
                "bindings",
                "layout",
                "slot",
                "sort",
                "drilldown",
                "topnGuard",
                "filters",
                "format",
                "conditionalFormatting",
            ][..],
        ),
        ("proof", &["desktop", "goldens"][..]),
    ] {
        let entry = allowed
            .iter()
            .find(|entry| entry["node"] == node)
            .unwrap_or_else(|| panic!("missing allowed-field node {node}"));
        let fields = entry["fields"].as_array().expect("node fields");
        for field in expected_fields {
            assert!(
                fields.iter().any(|value| value == field),
                "{node} missing field {field}"
            );
        }
    }
}

#[test]
fn v2_unknown_keys_in_new_sections_are_pointer_rich() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        (
            json!({"layout": {"grdi": {}}}),
            "/layout/grdi",
            Some("grid"),
        ),
        (
            json!({"style": {"tokens": {"semantic": {"danger": "#f00"}}}}),
            "/style/tokens/semantic/danger",
            None,
        ),
        (
            json!({"pages": [{"visuals": [{"format": {"title.colour": "#f00"}}]}]}),
            "/pages/0/visuals/0/format/title.colour",
            None,
        ),
        (
            json!({"proof": {"desktop": {"expectValues": [{"sql": "EVALUATE ROW()"}]}}}),
            "/proof/desktop/expectValues/0/sql",
            None,
        ),
    ];
    for (index, (fragment, pointer, suggestion)) in cases.into_iter().enumerate() {
        let mut spec = json!({
            "schema": "powerbi-cli.dashboard.v2",
            "report": {"name": "StrictV2"},
            "pages": []
        });
        merge_object(&mut spec, fragment);
        let path = temp.path().join(format!("unknown-{index}.json"));
        write_spec(&path, &spec);
        let output = run_powerbi(&[
            "report",
            "spec",
            "validate",
            "--spec",
            path.to_str().expect("spec path"),
            "--json",
        ]);
        assert_eq!(output.code, 10, "{}", output.stderr);
        let error = &stdout_json(&output)["errors"][0];
        assert_eq!(error["code"], "spec.unknown_field");
        assert_eq!(error["pointer"], pointer);
        match suggestion {
            Some(expected) => assert_eq!(error["didYouMean"], expected),
            None => assert!(error.get("didYouMean").is_none()),
        }
    }
}

#[test]
fn every_uncompiled_v2_section_names_its_owning_bead() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        (json!({"filters": []}), "pbi-t3-compiler-completeness-1qi.1"),
        (
            json!({"layout": {"rail": {"side": "left", "slicers": []}}}),
            "pbi-t3-compiler-completeness-1qi.2",
        ),
        (
            json!({"pages": [{"drillthrough": {"target": "DimDate[Date]"}}]}),
            "pbi-t3-compiler-completeness-1qi.3",
        ),
        (
            json!({"pages": [{"visuals": [{"sort": {"field": "FactSales[Total Revenue]", "direction": "Descending"}}]}]}),
            "pbi-t3-compiler-completeness-1qi.4",
        ),
        (
            json!({"model": {"calculatedColumns": []}}),
            "pbi-t3-compiler-completeness-1qi.5",
        ),
        (
            json!({"style": {"preset": "neutral"}}),
            "pbi-t3-compiler-completeness-1qi.6",
        ),
        (
            json!({"pages": [{"template": "overview"}]}),
            "pbi-t3-compiler-completeness-1qi.7",
        ),
        (
            json!({"pages": [{"visuals": [{"format": {"title.show": true}}]}]}),
            "pbi-t3-compiler-completeness-1qi.8",
        ),
        (
            json!({"proof": {"goldens": []}}),
            "pbi-t3-compiler-completeness-1qi.9",
        ),
    ];
    for (index, (fragment, bead)) in cases.into_iter().enumerate() {
        let mut spec = json!({
            "schema": "powerbi-cli.dashboard.v2",
            "report": {"name": "UnsupportedV2"},
            "pages": []
        });
        merge_object(&mut spec, fragment);
        let path = temp.path().join(format!("uncompiled-{index}.json"));
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
        assert_eq!(output.code, 2, "stdout: {}", output.stdout);
        let error = &stderr_json(&output)["error"];
        assert_eq!(error["code"], "unsupported_feature");
        assert!(
            error["message"].as_str().unwrap_or_default().contains(bead)
                || error["hint"].as_str().unwrap_or_default().contains(bead),
            "missing owning bead {bead}: {error}"
        );
    }
}

fn merge_object(target: &mut Value, fragment: Value) {
    for (key, value) in fragment.as_object().expect("fragment object") {
        target[key] = value.clone();
    }
}

fn read_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.expect("walk artifact tree"))
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .map(|path| {
            let relative = path
                .strip_prefix(root)
                .expect("relative artifact path")
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = fs::read(path).expect("read artifact file");
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}
