mod common;

use common::{archetype_names, assert_json_snapshot, load_archetype, run_powerbi, stdout_json};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;

fn property_sets(value: &Value, sets: &mut BTreeSet<Vec<String>>) {
    let Some(object) = value.as_object() else {
        return;
    };
    if object.get("additionalProperties") == Some(&Value::Bool(false))
        && let Some(properties) = object.get("properties").and_then(Value::as_object)
        && !properties.is_empty()
    {
        let mut keys = properties.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        sets.insert(keys);
    }
    for child in object.values() {
        property_sets(child, sets);
    }
}

fn walker_property_sets(fields: &Value, schema_name: &str) -> BTreeSet<Vec<String>> {
    fields["versionedAllowedFields"]
        .as_array()
        .expect("versioned allowed fields")
        .iter()
        .find(|entry| entry["schema"] == schema_name)
        .and_then(|entry| entry["allowedFields"].as_array())
        .expect("version-specific allowed fields")
        .iter()
        .map(|entry| {
            let mut keys = entry["fields"]
                .as_array()
                .expect("node fields")
                .iter()
                .map(|field| field.as_str().expect("field name").to_string())
                .collect::<Vec<_>>();
            keys.sort();
            keys
        })
        .collect()
}

#[test]
fn schema_properties_are_generated_from_each_versioned_walker_key_table() {
    let fields_output = run_powerbi(&["report", "spec", "fields", "--json"]);
    assert_eq!(fields_output.code, 0, "stderr: {}", fields_output.stderr);
    let fields = stdout_json(&fields_output);
    let schema_output = run_powerbi(&["report", "spec", "schema", "--json"]);
    assert_eq!(schema_output.code, 0, "stderr: {}", schema_output.stderr);
    let schema = stdout_json(&schema_output);

    assert_eq!(
        schema["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert!(schema["oneOf"].is_array());
    for version in ["powerbi-cli.dashboard.v1", "powerbi-cli.dashboard.v2"] {
        let key_sets = walker_property_sets(&fields, version);
        let def_name = if version.ends_with(".v1") { "v1" } else { "v2" };
        let mut schema_sets = BTreeSet::new();
        property_sets(&schema["$defs"][def_name], &mut schema_sets);
        assert_eq!(
            schema_sets, key_sets,
            "schema properties drifted from walker key table for {version}"
        );
    }
}

#[test]
fn schema_accepts_checked_in_v1_and_v2_specs_by_exposing_the_catalog_values() {
    let output = run_powerbi(&["report", "spec", "schema", "--version", "v1", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let v1 = stdout_json(&output);
    assert_eq!(
        v1["properties"]["schema"]["const"],
        "powerbi-cli.dashboard.v1"
    );
    assert!(
        v1["properties"]["pages"]["items"]["properties"]["visuals"]["items"]["properties"]["type"]
            ["enum"]
            .as_array()
            .expect("visual enum")
            .iter()
            .any(|value| value == "donut")
    );

    let output = run_powerbi(&["report", "spec", "schema", "--version", "v2", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let v2 = stdout_json(&output);
    assert_eq!(
        v2["properties"]["schema"]["const"],
        "powerbi-cli.dashboard.v2"
    );
    let format = &v2["properties"]["pages"]["items"]["properties"]["visuals"]["items"]["properties"]
        ["format"]["properties"];
    for key in [
        "labels.show",
        "categoryAxis.show",
        "valueAxis.showAxisTitle",
        "title.text",
    ] {
        assert!(format.get(key).is_some(), "missing format key {key}");
    }
}

#[test]
fn explain_is_deterministic_and_does_not_write_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let args = [
        "report",
        "spec",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--spec",
        "examples/sales.dashboard.json",
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(first.stdout.as_bytes(), second.stdout.as_bytes());
    let value = stdout_json(&first);
    assert_eq!(value["ok"], true);
    assert_eq!(value["plan"]["schema"], "powerbi-cli.ops.v1");
    assert!(value["plan"]["stages"].as_array().is_some());
    assert!(value["handles"]["declared"].as_array().is_some());
    assert_eq!(value["layout"]["available"], true);
    assert!(value["defaults"]["perVisual"].as_array().is_some());
    assert!(value["proofPlan"]["commands"].as_array().is_some());
    assert!(
        fs::read_dir(temp.path())
            .expect("tempdir read")
            .next()
            .is_none()
    );
}

#[test]
fn explain_previews_uncompiled_v2_sections_with_owning_beads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_path = temp.path().join("unsupported.dashboard.json");
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.v2.json").expect("sales v2 spec"),
    )
    .expect("parse sales v2 spec");
    spec["style"] = serde_json::json!({"preset": "neutral"});
    spec["filters"] = serde_json::json!([]);
    fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("serialize unsupported spec"),
    )
    .expect("write unsupported spec");
    let path = spec_path.to_str().expect("spec path");
    let output = run_powerbi(&[
        "report",
        "spec",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        path,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let unsupported = value["unsupportedSections"]
        .as_array()
        .expect("unsupported");
    assert!(unsupported.iter().any(|item| {
        item["pointer"] == "/filters" && item["owningBead"] == "pbi-t3-compiler-completeness-1qi.1"
    }));
    assert!(unsupported.iter().any(|item| {
        item["pointer"] == "/style" && item["owningBead"] == "pbi-t3-compiler-completeness-1qi.6"
    }));
    assert_eq!(fs::read_dir(temp.path()).expect("tempdir read").count(), 1);
}

#[test]
fn explain_orders_model_visual_and_behavior_operations_with_stable_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_path = temp.path().join("with-ops.dashboard.json");
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.json").expect("sales spec"),
    )
    .expect("parse sales spec");
    spec["model"]["measures"] = serde_json::json!([
        {
            "table": "FactSales",
            "name": "Preview Measure",
            "expression": "SUM(FactSales[Revenue])"
        }
    ]);
    spec["pages"][0]["interactions"] = serde_json::json!([
        {"source": "revenue_card", "target": "revenue_trend", "type": "DataFilter"}
    ]);
    fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("serialize spec"),
    )
    .expect("write spec");
    let output = run_powerbi(&[
        "report",
        "spec",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec_path.to_str().expect("spec path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let ops = value["plan"]["ops"].as_array().expect("plan ops");
    assert_eq!(ops.first().expect("measure op")["op"], "addMeasure");
    assert_eq!(ops.last().expect("interaction op")["op"], "setInteraction");
    assert_eq!(ops[0]["stageName"], "model");
    assert_eq!(ops[1]["stageName"], "visual");
    assert_eq!(ops.last().expect("interaction op")["stageName"], "behavior");
    assert!(
        value["handles"]["declared"]
            .as_array()
            .expect("declared handles")
            .iter()
            .any(|item| item["handle"] == "measure:FactSales:Preview Measure")
    );
    assert!(
        value["handles"]["references"]
            .as_array()
            .expect("references")
            .iter()
            .any(|item| item["field"] == "source")
    );
}

#[test]
fn explain_snapshots_cover_every_checked_in_archetype() {
    for name in archetype_names() {
        let fixture = load_archetype(name);
        let output = run_powerbi(&[
            "report",
            "spec",
            "explain",
            "--schema",
            fixture.schema.to_str().expect("schema path"),
            "--profile",
            fixture.profile.to_str().expect("profile path"),
            "--spec",
            fixture.spec.to_str().expect("spec path"),
            "--json",
        ]);
        assert_eq!(output.code, 0, "{name}: {}", output.stderr);
        let value = stdout_json(&output);
        assert_json_snapshot(
            &format!("report-spec-explain-{name}"),
            &serde_json::json!({
                "schema": value["schema"],
                "specVersion": value["specVersion"],
                "plan": value["plan"],
                "handles": value["handles"],
                "layout": value["layout"],
                "defaults": value["defaults"],
                "proofUnavailable": value["proofPlan"]["unavailable"],
                "unsupportedSections": value["unsupportedSections"]
            }),
        );
    }
}
