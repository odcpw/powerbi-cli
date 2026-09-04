mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn write_spec(root: &Path, name: &str, spec: Value) -> String {
    let path = root.join(format!("{name}.dashboard.json"));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&spec).expect("serialize dashboard spec"),
    )
    .expect("write dashboard spec");
    path.to_str().expect("spec path").to_string()
}

fn assert_missing_input(root: &Path, name: &str, spec: Value, pointer: &str, field: &str) {
    let path = write_spec(root, name, spec);
    let output = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &path,
        "--json",
    ]);
    assert_eq!(output.code, 10, "{name}: {}", output.stderr);
    let error = &stdout_json(&output)["errors"][0];
    assert_eq!(error["code"], "spec.missing_input", "{name}: {error}");
    assert_eq!(error["pointer"], pointer, "{name}: {error}");
    assert_eq!(error["field"], field, "{name}: {error}");
    assert!(error["reason"].as_str().is_some(), "{name}: {error}");
    assert_eq!(
        error["candidatesCommand"], "powerbi-cli report spec fields --schema <schema.json> --json",
        "{name}: {error}"
    );
    assert!(error.get("example").is_some(), "{name}: {error}");
}

fn v2_page(visuals: Value) -> Value {
    json!({
        "schema": "powerbi-cli.dashboard.v2",
        "report": {"name": "MissingInput"},
        "pages": [{"id": "overview", "visuals": visuals}]
    })
}

#[test]
fn topn_without_order_by_is_structured_missing_input() {
    let root = tempfile::tempdir().expect("tempdir");
    assert_missing_input(
        root.path(),
        "topn-order-by",
        v2_page(json!([{
            "type": "lineChart",
            "bindings": [
                {"role": "Category", "field": "DimDate[Date]"},
                {"role": "Y", "field": "FactSales[Total Revenue]"},
                {"role": "Y", "field": "FactSales[Total Units]"}
            ],
            "topnGuard": {"top": 5}
        }])),
        "/pages/0/visuals/0/topnGuard/orderBy",
        "visuals[].topnGuard.orderBy",
    );
}

#[test]
fn drillthrough_without_target_is_structured_missing_input() {
    let root = tempfile::tempdir().expect("tempdir");
    assert_missing_input(
        root.path(),
        "drillthrough-target",
        json!({
            "schema": "powerbi-cli.dashboard.v2",
            "report": {"name": "MissingInput"},
            "pages": [{"id": "detail", "drillthrough": {"hidden": false}}]
        }),
        "/pages/0/drillthrough/target",
        "pages[].drillthrough.target",
    );
}

#[test]
fn slot_outside_page_template_is_structured_missing_input() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut spec = v2_page(json!([{
        "type": "card",
        "slot": "not-a-slot",
        "bindings": [{"role": "Values", "field": "FactSales[Total Revenue]"}]
    }]));
    spec["pages"][0]["template"] = Value::String("overview".to_string());
    assert_missing_input(
        root.path(),
        "slot-template",
        spec,
        "/pages/0/visuals/0/slot",
        "visuals[].slot",
    );
}

#[test]
fn page_slicer_measure_is_structured_missing_input() {
    let root = tempfile::tempdir().expect("tempdir");
    assert_missing_input(
        root.path(),
        "slicer-column",
        json!({
            "schema": "powerbi-cli.dashboard.v2",
            "report": {"name": "MissingInput"},
            "pages": [{
                "id": "overview",
                "slicers": [{"field": "FactSales[Total Revenue]"}]
            }]
        }),
        "/pages/0/slicers/0/field",
        "slicers[].field",
    );
}

#[test]
fn conditional_formatting_without_semantic_token_is_structured_missing_input() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut spec = v2_page(json!([{
        "type": "card",
        "bindings": [{"role": "Values", "field": "FactSales[Total Revenue]"}],
        "conditionalFormatting": [{"color": "semantic.good"}]
    }]));
    spec["style"] = json!({"tokens": {"semantic": {}}});
    assert_missing_input(
        root.path(),
        "semantic-color",
        spec,
        "/style/tokens/semantic/good",
        "style.tokens.semantic",
    );
}

#[test]
fn period_measure_pattern_without_date_is_structured_missing_input() {
    let root = tempfile::tempdir().expect("tempdir");
    assert_missing_input(
        root.path(),
        "measure-pattern-date",
        json!({
            "schema": "powerbi-cli.dashboard.v2",
            "report": {"name": "MissingInput"},
            "model": {"measurePatterns": [{"pattern": "yoy", "base": "FactSales[Total Revenue]"}]},
            "pages": []
        }),
        "/model/measurePatterns/0/date",
        "model.measurePatterns[].date",
    );
}

#[test]
fn visual_without_bindings_or_text_is_structured_missing_input() {
    let root = tempfile::tempdir().expect("tempdir");
    assert_missing_input(
        root.path(),
        "visual-bindings",
        v2_page(json!([{"type": "textbox"}])),
        "/pages/0/visuals/0/bindings",
        "visuals[].bindings",
    );
}

#[test]
fn missing_input_is_catalogued_for_explain_and_capabilities() {
    let explained = run_powerbi(&["lint", "--explain", "spec.missing_input", "--json"]);
    assert_eq!(explained.code, 0, "stderr: {}", explained.stderr);
    let explanation = stdout_json(&explained);
    assert_eq!(explanation["rule"]["id"], "spec.missing_input");
    assert_eq!(explanation["rule"]["family"], "validation");
    assert_eq!(explanation["exampleFinding"]["code"], "spec.missing_input");

    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let capabilities_json = stdout_json(&capabilities);
    let codes = capabilities_json["diagnosticCodes"]
        .as_array()
        .expect("diagnostic codes");
    assert!(
        codes
            .iter()
            .any(|code| code["code"] == "spec.missing_input")
    );

    // The stderr serializer carries the same fields as report spec validate.
    let missing_schema = run_powerbi(&["report", "build", "--dry-run", "--json"]);
    assert_eq!(missing_schema.code, 10);
    let error = &stderr_json(&missing_schema)["error"];
    assert_eq!(error["code"], "spec.missing_input");
    assert_eq!(error["field"], "schema");
    assert!(error["reason"].is_string());
    assert!(error["candidatesCommand"].is_string());
    assert!(error.get("example").is_some());
}

#[test]
fn documented_optional_defaults_are_reported_without_blocking_build() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut spec = json!({
        "schema": "powerbi-cli.dashboard.v1",
        "report": {"name": "Defaults"},
        "pages": [{
            "id": "overview",
            "visuals": [{
                "type": "card",
                "bindings": [{"role": "Values", "field": "FactSales[Total Revenue]"}]
            }]
        }]
    });
    let path = write_spec(root.path(), "defaults", spec.take());
    let output = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &path,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["ok"], true);
    let defaults = value["defaultsApplied"]
        .as_array()
        .expect("defaultsApplied");
    assert!(
        defaults
            .iter()
            .any(|item| item["field"] == "visuals[].layout.x")
    );
    assert!(defaults.iter().all(|item| item["pointer"].is_string()));
}
