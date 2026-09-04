mod common;

use common::{assert_json_snapshot, run_powerbi, stderr_json, stdout_json};
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
fn schema_version_missing_warns_during_the_compatibility_release() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema_path = temp.path().join("legacy.schema.json");
    write_spec(
        &schema_path,
        &json!({
            "name": "LegacySchema",
            "tables": [{"name": "Fact", "columns": [{"name": "Value", "dataType": "int64"}]}]
        }),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        schema_path.to_str().expect("schema path"),
        "--json",
    ]);
    assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
    let response = stdout_json(&output);
    assert_eq!(response["ok"], true);
    assert!(
        response["warnings"][0]
            .as_str()
            .unwrap_or_default()
            .contains("schemaVersion")
    );
}

#[test]
fn schema_version_must_be_a_non_empty_string_when_present() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema_path = temp.path().join("invalid-version.schema.json");
    write_spec(
        &schema_path,
        &json!({
            "schemaVersion": 1,
            "name": "InvalidVersion",
            "tables": [{"name": "Fact", "columns": [{"name": "Value", "dataType": "int64"}]}]
        }),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        schema_path.to_str().expect("schema path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let response = stdout_json(&output);
    assert_eq!(response["ok"], false);
    assert!(
        response["errors"][0]
            .as_str()
            .unwrap_or_default()
            .contains("non-empty string")
    );
}

#[test]
fn schema_and_spec_includes_normalize_deterministically_and_build_with_parity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let parts = temp.path().join("parts");
    fs::create_dir_all(&parts).expect("parts directory");

    let fact = json!({
        "name": "Fact",
        "columns": [
            {"name": "Category", "dataType": "string"},
            {"name": "Value", "dataType": "int64"}
        ],
        "rows": [{"Category": "Synthetic", "Value": 1}]
    });
    write_spec(&parts.join("fact.json"), &fact);
    write_spec(
        &parts.join("schema-part.json"),
        &json!({
            "tables": [{"$include": "fact.json"}],
            "relationships": []
        }),
    );
    let dimension = json!({
        "name": "Dim",
        "columns": [{"name": "Category", "dataType": "string"}],
        "rows": [{"Category": "Synthetic"}]
    });
    let schema = json!({
        "schemaVersion": "1",
        "name": "IncludeParity",
        "displayName": "Include Parity",
        "$include": "parts/schema-part.json",
        "tables": [dimension.clone()],
        "relationships": []
    });
    let inline_schema = json!({
        "schemaVersion": "1",
        "name": "IncludeParity",
        "displayName": "Include Parity",
        "tables": [fact.clone(), dimension],
        "relationships": []
    });
    let schema_path = temp.path().join("include.schema.json");
    let inline_schema_path = temp.path().join("inline.schema.json");
    write_spec(&schema_path, &schema);
    write_spec(&inline_schema_path, &inline_schema);

    let validation = run_powerbi(&[
        "schema",
        "validate",
        schema_path.to_str().expect("schema path"),
        "--json",
    ]);
    assert_eq!(validation.exit, 0, "stderr: {}", validation.stderr);
    let validation_json = stdout_json(&validation);
    assert_eq!(
        validation_json["normalizedFrom"],
        json!(["parts/fact.json", "parts/schema-part.json"])
    );
    assert_eq!(validation_json["counts"]["tables"], 2);

    let normalized_one = temp.path().join("schema.normalized.one.json");
    let normalized_two = temp.path().join("schema.normalized.two.json");
    for output_path in [&normalized_one, &normalized_two] {
        let output = run_powerbi(&[
            "schema",
            "normalize",
            schema_path.to_str().expect("schema path"),
            "--out",
            output_path.to_str().expect("normalized path"),
            "--json",
        ]);
        assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
    }
    assert_eq!(
        fs::read(&normalized_one).expect("normalized one"),
        fs::read(&normalized_two).expect("normalized two")
    );
    let normalized_schema: Value =
        serde_json::from_slice(&fs::read(&normalized_one).expect("read normalized schema"))
            .expect("parse normalized schema");
    assert!(normalized_schema.get("$include").is_none());
    assert_eq!(normalized_schema["tables"][0]["name"], "Fact");
    assert_eq!(normalized_schema["tables"][1]["name"], "Dim");

    let spec_parts = temp.path().join("spec-parts");
    fs::create_dir_all(&spec_parts).expect("spec parts directory");
    write_spec(
        &spec_parts.join("model.json"),
        &json!({
            "measures": [{"table": "Fact", "name": "Total", "expression": "SUM(Fact[Value])"}]
        }),
    );
    write_spec(
        &spec_parts.join("page.json"),
        &json!({"id": "overview", "displayName": "Overview", "visuals": []}),
    );
    write_spec(
        &spec_parts.join("style.json"),
        &json!({"tokens": {"palette": ["#123456"]}}),
    );
    let spec = json!({
        "schema": "powerbi-cli.dashboard.v2",
        "report": {"name": "IncludeSpec", "displayName": "Include Spec"},
        "model": {"$include": "spec-parts/model.json"},
        "pages": [{"$include": "spec-parts/page.json"}],
        "style": {"$include": "spec-parts/style.json"}
    });
    let spec_path = temp.path().join("include.dashboard.json");
    write_spec(&spec_path, &spec);
    let normalized_spec_one = temp.path().join("spec.normalized.one.json");
    let normalized_spec_two = temp.path().join("spec.normalized.two.json");
    for output_path in [&normalized_spec_one, &normalized_spec_two] {
        let output = run_powerbi(&[
            "report",
            "spec",
            "normalize",
            spec_path.to_str().expect("spec path"),
            "--out",
            output_path.to_str().expect("normalized spec path"),
            "--json",
        ]);
        assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
    }
    assert_eq!(
        fs::read(&normalized_spec_one).expect("normalized spec one"),
        fs::read(&normalized_spec_two).expect("normalized spec two")
    );
    let normalized_spec: Value =
        serde_json::from_slice(&fs::read(&normalized_spec_one).expect("read normalized spec"))
            .expect("parse normalized spec");
    assert!(normalized_spec.get("$include").is_none());
    assert_eq!(normalized_spec["model"]["measures"][0]["name"], "Total");
    assert_eq!(normalized_spec["pages"][0]["displayName"], "Overview");
    assert_eq!(normalized_spec["style"]["tokens"]["palette"][0], "#123456");

    let normalized_response = run_powerbi(&[
        "report",
        "spec",
        "normalize",
        spec_path.to_str().expect("spec path"),
        "--out",
        normalized_spec_one.to_str().expect("normalized spec path"),
        "--json",
    ]);
    assert_eq!(normalized_response.exit, 0);
    let response_json = stdout_json(&normalized_response);
    assert_json_snapshot(
        "schema-spec-include-normalize",
        &json!({
            "schema": {
                "normalizedFrom": validation_json["normalizedFrom"],
                "tableNames": normalized_schema["tables"].as_array().expect("tables").iter().map(|table| table["name"].clone()).collect::<Vec<_>>()
            },
            "spec": {
                "normalizedFrom": response_json["normalizedFrom"],
                "specVersion": response_json["specVersion"],
                "measure": normalized_spec["model"]["measures"][0]["name"],
                "page": normalized_spec["pages"][0]["displayName"]
            }
        }),
    );

    let spec_for_build = temp.path().join("build.dashboard.json");
    write_spec(
        &spec_for_build,
        &json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": {"name": "IncludeParity", "displayName": "Include Parity"},
            "pages": []
        }),
    );
    let included_project = temp.path().join("included-project");
    let inline_project = temp.path().join("inline-project");
    for (schema_input, project) in [
        (&schema_path, &included_project),
        (&inline_schema_path, &inline_project),
    ] {
        let output = run_powerbi(&[
            "report",
            "build",
            "--schema",
            schema_input.to_str().expect("build schema path"),
            "--spec",
            spec_for_build.to_str().expect("build spec path"),
            "--out-dir",
            project.to_str().expect("project path"),
            "--json",
        ]);
        assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
    }
    assert_eq!(read_tree(&included_project), read_tree(&inline_project));
}

#[test]
fn include_path_escape_and_unsupported_location_are_refused_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema_path = temp.path().join("escape.schema.json");
    write_spec(
        &schema_path,
        &json!({"schemaVersion": "1", "name": "Escape", "$include": "../outside.json"}),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        schema_path.to_str().expect("schema path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "include.path_escape");
    assert_eq!(error["pointer"], "/$include");
    assert!(error["suggestedCommands"].as_array().is_some());

    let spec_path = temp.path().join("unsupported.dashboard.json");
    write_spec(
        &spec_path,
        &json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": {"name": "Unsupported"},
            "pages": [{"id": "overview", "visuals": [{"$include": "fragment.json"}]}]
        }),
    );
    write_spec(&temp.path().join("fragment.json"), &json!({"id": "visual"}));
    let output = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--spec",
        spec_path.to_str().expect("spec path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stderr: {}", output.stderr);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "include.unsupported_location");
    assert_eq!(error["pointer"], "/pages/0/visuals/0/$include");
}

#[test]
fn include_scalar_fragment_is_refused_instead_of_being_dropped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("scalar.schema.json");
    write_spec(&temp.path().join("scalar.json"), &json!(null));
    write_spec(
        &root,
        &json!({"schemaVersion": "1", "name": "Scalar", "$include": "scalar.json"}),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        root.to_str().expect("root path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "include.invalid");
    assert_eq!(error["pointer"], "/$include");
}

#[test]
fn include_cycle_is_refused_with_the_active_chain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("root.schema.json");
    write_spec(
        &root,
        &json!({"schemaVersion": "1", "name": "Cycle", "$include": "a.json"}),
    );
    write_spec(&temp.path().join("a.json"), &json!({"$include": "b.json"}));
    write_spec(
        &temp.path().join("b.json"),
        &json!({"$include": "root.schema.json"}),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        root.to_str().expect("root path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "include.cycle");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("cycle")
    );
    assert!(error["hint"].is_string());
}

#[test]
fn include_depth_budget_is_refused_before_parsing_more_fragments() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("depth-root.schema.json");
    for index in 0..=9 {
        let path = temp.path().join(format!("depth-{index}.schema.json"));
        let value = if index < 9 {
            json!({"$include": format!("depth-{}.schema.json", index + 1)})
        } else {
            json!({"tables": []})
        };
        write_spec(&path, &value);
    }
    write_spec(
        &root,
        &json!({"schemaVersion": "1", "name": "Depth", "$include": "depth-0.schema.json"}),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        root.to_str().expect("root path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "input_safety_violation");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("depth")
    );
}

#[test]
fn include_fragment_count_budget_is_refused_for_many_siblings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut includes = Vec::new();
    for index in 0..201 {
        let name = format!("fragment-{index:03}.json");
        write_spec(&temp.path().join(&name), &json!({"tables": []}));
        includes.push(name);
    }
    let root = temp.path().join("count.schema.json");
    write_spec(
        &root,
        &json!({"schemaVersion": "1", "name": "Count", "$include": includes}),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        root.to_str().expect("root path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "input_safety_violation");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("count")
    );
}

#[test]
fn include_fragment_size_budget_is_refused_before_json_parse() {
    let temp = tempfile::tempdir().expect("tempdir");
    let large = temp.path().join("large.json");
    fs::write(&large, vec![b' '; 8 * 1024 * 1024 + 1]).expect("large fragment");
    let root = temp.path().join("size.schema.json");
    write_spec(
        &root,
        &json!({"schemaVersion": "1", "name": "Size", "$include": "large.json"}),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        root.to_str().expect("root path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "input_safety_violation");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("maximum")
    );
}

#[cfg(unix)]
#[test]
fn include_symlink_is_refused_before_canonical_containment() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let real = temp.path().join("real.json");
    write_spec(&real, &json!({"tables": []}));
    let link = temp.path().join("link.json");
    symlink(&real, &link).expect("include symlink");
    let root = temp.path().join("symlink.schema.json");
    write_spec(
        &root,
        &json!({"schemaVersion": "1", "name": "Link", "$include": "link.json"}),
    );
    let output = run_powerbi(&[
        "schema",
        "validate",
        root.to_str().expect("root path"),
        "--json",
    ]);
    assert_eq!(output.exit, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "input_safety_violation");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("symlink")
    );
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
        (
            json!({"layout": {"rail": {"side": "left", "slicers": []}}}),
            "pbi-t3-compiler-completeness-1qi.2",
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
