mod common;

use common::{run_powerbi, scaffold_sales, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::path::Path;

const MAX_ROWS_FILE_BYTES: u64 = 64 * 1024 * 1024;

fn rows_schema() -> Value {
    json!({
        "name": "ProfileRows",
        "displayName": "Profile Rows",
        "tables": [{
            "name": "FactMetrics",
            "columns": [
                {"name": "Id", "dataType": "int64", "isKey": true},
                {"name": "EventDate", "dataType": "date"},
                {"name": "Amount", "dataType": "decimal"},
                {"name": "Category", "dataType": "string"}
            ]
        }]
    })
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(value).expect("serialize JSON fixture")
        ),
    )
    .expect("write JSON fixture");
}

fn write_rows_fixture(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let schema = root.join("profile.schema.json");
    write_json(&schema, &rows_schema());
    let rows = root.join("profile.rows.csv");
    fs::write(
        &rows,
        "Id,EventDate,Amount,Category\n1,2025-01-01,10.50,Synthetic North\n1,2026-02-03,20.00,Synthetic North\n2,,30.00,Synthetic South\n3,2026-04-05,invalid,Synthetic North\n",
    )
    .expect("write CSV fixture");
    (schema, rows)
}

#[test]
fn profile_infer_rows_emits_v2_statistics_and_redacts_literals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (schema, rows) = write_rows_fixture(temp.path());
    let profile_path = temp.path().join("profile.json");
    let output = run_powerbi(&[
        "profile",
        "infer",
        "--schema",
        schema.to_str().expect("schema path"),
        "--rows",
        rows.to_str().expect("rows path"),
        "--out",
        profile_path.to_str().expect("profile path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(output.stderr.is_empty());
    let value = stdout_json(&output);
    assert_eq!(value["schema"], "powerbi-cli.profile.infer.v2");
    let profile = &value["profile"];
    assert_eq!(profile["schema"], "powerbi-cli.dataProfile.v2");
    assert_eq!(profile["dataValues"], false);
    assert_eq!(profile["source"]["kind"], "external-rows");
    assert_eq!(profile["source"]["format"], "csv");
    assert_eq!(profile["source"]["table"], "FactMetrics");
    assert_eq!(profile["source"]["rowCount"], 4);

    let columns = profile["tables"][0]["columns"]
        .as_array()
        .expect("profile columns");
    let column = |name: &str| {
        columns
            .iter()
            .find(|column| column["name"] == name)
            .unwrap_or_else(|| panic!("missing profile column {name}"))
    };
    assert_eq!(column("Id")["nullRate"].as_f64(), Some(0.0));
    assert_eq!(column("Id")["distinctCount"], 3);
    assert_eq!(column("Id")["min"], 1);
    assert_eq!(column("Id")["max"], 3);
    assert_eq!(column("EventDate")["nullCount"], 1);
    assert_eq!(column("EventDate")["timeCoverage"]["start"], "2025-01-01");
    assert_eq!(column("EventDate")["timeCoverage"]["end"], "2026-04-05");
    assert_eq!(column("Amount")["min"], 10.5);
    assert_eq!(column("Amount")["max"], 30.0);
    assert_eq!(column("Amount")["typeCoercion"]["failedCount"], 1);
    assert!(
        profile["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|diagnostic| diagnostic["code"] == "profile.type_coercion_failed")
    );
    assert!(
        profile["grainConflicts"]
            .as_array()
            .expect("grain conflicts")
            .iter()
            .any(|conflict| conflict["code"] == "profile.grain_conflict")
    );
    for top in column("Category")["topValues"]
        .as_array()
        .expect("top values")
    {
        assert_eq!(top["value"], "[REDACTED]");
        assert_eq!(top["redacted"], true);
    }
    assert!(!output.stdout.contains("Synthetic North"));
    assert!(!output.stdout.contains("Synthetic South"));
    assert!(profile_path.is_file());

    let validate = run_powerbi(&[
        "profile",
        "validate",
        profile_path.to_str().expect("profile path"),
        "--json",
    ]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], true);
}

#[test]
fn profile_infer_rows_json_is_byte_deterministic_and_supports_array_headers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (schema, _) = write_rows_fixture(temp.path());
    let rows = temp.path().join("profile.rows.json");
    write_json(
        &rows,
        &json!([
            ["Id", "EventDate", "Amount", "Category"],
            ["1", "2025-01-01", "10", "Synthetic North"],
            ["2", "2025-02-01", "20", "Synthetic South"]
        ]),
    );
    let args = [
        "profile",
        "infer",
        "--schema",
        schema.to_str().expect("schema path"),
        "--rows",
        rows.to_str().expect("rows path"),
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(first.stdout.as_bytes(), second.stdout.as_bytes());
    assert_eq!(first.stderr.as_bytes(), second.stderr.as_bytes());
    let profile = &stdout_json(&first)["profile"];
    assert_eq!(profile["source"]["format"], "json");
    assert_eq!(profile["source"]["rowCount"], 2);
    assert_eq!(profile["tables"][0]["columns"][0]["distinctCount"], 2);
}

#[test]
fn profile_infer_rows_refuses_headers_that_do_not_match_the_schema() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema = temp.path().join("mismatch.schema.json");
    write_json(&schema, &rows_schema());
    let rows = temp.path().join("mismatch.rows.csv");
    fs::write(&rows, "UnknownColumn\nsynthetic-value\n").expect("mismatch rows");
    let profile = temp.path().join("mismatch.profile.json");
    let output = run_powerbi(&[
        "profile",
        "infer",
        "--schema",
        schema.to_str().expect("schema path"),
        "--rows",
        rows.to_str().expect("rows path"),
        "--out",
        profile.to_str().expect("profile path"),
        "--json",
    ]);
    assert_eq!(output.code, 10);
    assert!(output.stdout.is_empty());
    assert!(!profile.exists());
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "validation_failed");
    assert!(error["error"]["hint"].is_string());
    assert!(error["error"]["suggestedCommands"].is_array());
    assert!(!output.stderr.contains("synthetic-value"));
}

#[test]
fn profile_infer_include_data_values_is_explicit_bounded_and_validated() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (schema, rows) = write_rows_fixture(temp.path());
    let output = run_powerbi(&[
        "profile",
        "infer",
        "--schema",
        schema.to_str().expect("schema path"),
        "--rows",
        rows.to_str().expect("rows path"),
        "--include-data-values",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let profile = &stdout_json(&output)["profile"];
    assert_eq!(profile["dataValues"], true);
    let top_values = profile["tables"][0]["columns"][3]["topValues"]
        .as_array()
        .expect("top values");
    assert!(top_values.len() <= 5);
    assert_eq!(top_values[0]["value"], "Synthetic North");
    assert_eq!(top_values[0]["redacted"], false);
    assert_eq!(profile["tables"][0]["columns"][3]["valuesRedacted"], false);

    let profile_path = temp.path().join("included.profile.json");
    write_json(&profile_path, profile);
    let validate = run_powerbi(&[
        "profile",
        "validate",
        profile_path.to_str().expect("profile path"),
        "--json",
    ]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
}

#[test]
fn profile_infer_redact_is_a_noop_alias_with_a_deprecation_note() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (schema, rows) = write_rows_fixture(temp.path());
    let output = run_powerbi(&[
        "profile",
        "infer",
        "--schema",
        schema.to_str().expect("schema path"),
        "--rows",
        rows.to_str().expect("rows path"),
        "--redact",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["profile"]["dataValues"], false);
    assert_eq!(
        value["deprecations"][0]["code"],
        "profile.redact_deprecated"
    );
    assert!(!output.stdout.contains("Synthetic North"));
}

#[test]
fn profile_infer_include_data_values_refuses_credential_and_pii_columns_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema = temp.path().join("unsafe.schema.json");
    write_json(
        &schema,
        &json!({
            "name": "UnsafeProfileRows",
            "tables": [{
                "name": "FactMetrics",
                "columns": [
                    {"name": "Name", "dataType": "string"},
                    {"name": "Password", "dataType": "string"}
                ]
            }]
        }),
    );
    let rows = temp.path().join("unsafe.rows.csv");
    fs::write(&rows, "Name,Password\nTest Person,synthetic-secret\n").expect("unsafe rows");
    let profile = temp.path().join("unsafe.profile.json");
    let output = run_powerbi(&[
        "profile",
        "infer",
        "--schema",
        schema.to_str().expect("schema path"),
        "--rows",
        rows.to_str().expect("rows path"),
        "--include-data-values",
        "--out",
        profile.to_str().expect("profile path"),
        "--json",
    ]);
    assert_eq!(output.code, 10);
    assert!(output.stdout.is_empty());
    assert!(!profile.exists());
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "validation_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("credential/PII scan")
    );
    assert!(!output.stderr.contains("synthetic-secret"));
}

#[test]
fn profile_infer_rows_enforces_input_safety_file_budget_before_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema = temp.path().join("budget.schema.json");
    write_json(&schema, &rows_schema());
    let rows = temp.path().join("oversized.rows.csv");
    let file = File::create(&rows).expect("create oversized rows");
    file.set_len(MAX_ROWS_FILE_BYTES + 1)
        .expect("set oversized rows length");
    let profile = temp.path().join("budget.profile.json");
    let args = [
        "profile",
        "infer",
        "--schema",
        schema.to_str().expect("schema path"),
        "--rows",
        rows.to_str().expect("rows path"),
        "--out",
        profile.to_str().expect("profile path"),
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 10);
    assert_eq!(first.stdout, "");
    assert_eq!(first.stderr, second.stderr);
    assert_eq!(
        stderr_json(&first)["error"]["code"],
        "input_safety_violation"
    );
    assert!(!profile.exists());
}

#[test]
fn data_bearing_profiles_are_reported_by_handoff_and_source_pack() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let profile = project.join("sales.profile.json");
    write_json(
        &profile,
        &json!({
            "schema": "powerbi-cli.dataProfile.v2",
            "dataValues": true,
            "tables": [{"name": "FactSales", "columns": []}]
        }),
    );
    let project_arg = project.to_str().expect("project path");

    let handoff = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(handoff.code, 10, "stderr: {}", handoff.stderr);
    let handoff_json = stdout_json(&handoff);
    assert_eq!(handoff_json["dataBearing"], true);
    assert_eq!(handoff_json["counts"]["dataBearingProfiles"], 1);
    assert!(
        handoff_json["findings"]
            .as_array()
            .expect("handoff findings")
            .iter()
            .any(|finding| finding["message"]
                .as_str()
                .unwrap_or_default()
                .contains("profile is data-bearing"))
    );

    let package = temp.path().join("data-bearing-source.pbit");
    let source_pack = run_powerbi(&[
        "package",
        "source-pack",
        "--project",
        project_arg,
        "--out",
        package.to_str().expect("package path"),
        "--json",
    ]);
    assert_eq!(source_pack.code, 10);
    assert!(!package.exists());
    let error = stderr_json(&source_pack);
    assert_eq!(error["error"]["code"], "validation_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("package error")
            .contains("data-bearing profile")
    );
}
