mod common;

use common::{assert_json_snapshot, run_powerbi, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;

fn schema() -> Value {
    json!({"name":"Evidence", "tables":[{
        "name":"Events", "columns":[
            {"name":"Date", "dataType":"date"},
            {"name":"Amount", "dataType":"decimal"},
            {"name":"Category", "dataType":"string"}
        ], "measures":[{"name":"Total", "expression":"SUM('Events'[Amount])"}]
    }]})
}

#[test]
fn weak_evidence_refuses_without_writing_and_reports_recovery_fields_deterministically() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("schema.json");
    let out = temp.path().join("dashboard.json");
    for (case, pointer, field) in [
        (
            "date",
            "/schema/tables",
            "schema.tables[].columns[].type=date",
        ),
        (
            "measure",
            "/intent/kpis/0/measure",
            "intent.kpis[0].measure",
        ),
        ("fact", "/intent/model/factTable", "intent.model.factTable"),
    ] {
        let mut value = schema();
        match case {
            "date" => value["tables"][0]["columns"][0]["dataType"] = json!("string"),
            "measure" => value["tables"][0]["measures"] = json!([]),
            _ => {
                let mut second = value["tables"][0].clone();
                second["name"] = json!("OtherEvents");
                value["tables"].as_array_mut().unwrap().push(second);
            }
        }
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        for force in [false, true] {
            if force {
                fs::write(&out, b"preserve existing output").unwrap();
            }
            let mut args = vec![
                "report",
                "plan",
                "--schema",
                path.to_str().unwrap(),
                "--objective",
                "Overview",
                "--out",
                out.to_str().unwrap(),
                "--json",
            ];
            if force {
                args.push("--force");
            }
            let first = run_powerbi(&args);
            let second = run_powerbi(&args);
            assert_eq!(first.code, 10, "{}", first.stderr);
            assert_eq!(first.stderr, second.stderr);
            assert!(first.stdout.is_empty());
            let mut error = stderr_json(&first)["error"].clone();
            assert_eq!(error["code"], "plan.missing_input");
            assert_eq!(error["pointer"], pointer);
            assert_eq!(error["field"], field);
            assert!(error["hint"].is_string());
            assert!(error["reason"].is_string());
            assert!(error["candidates"].is_array());
            for command in error["suggestedCommands"].as_array().unwrap() {
                assert!(command.as_str().unwrap().starts_with("powerbi-cli "));
                assert!(command.as_str().unwrap().ends_with("--json"));
            }
            // Keep the complete public error shape, normalizing only the fixture path.
            error = serde_json::from_str(
                &serde_json::to_string(&error)
                    .unwrap()
                    .replace(path.to_str().unwrap(), "<schema.json>"),
            )
            .unwrap();
            assert_json_snapshot(&format!("report-plan-missing-{case}"), &error);
            if force {
                assert_eq!(fs::read(&out).unwrap(), b"preserve existing output");
            } else {
                assert!(!out.exists());
            }
        }
        fs::remove_file(&out).unwrap();
    }
}

#[test]
fn sufficient_schema_and_explicit_fact_override_produce_deterministic_plans() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("schema.json");
    let mut value = schema();
    let mut second = value["tables"][0].clone();
    second["name"] = json!("OtherEvents");
    value["tables"].as_array_mut().unwrap().push(second);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let intent = r#"{"questions":["Overview"],"model":{"factTable":"Events"}}"#;
    let args = [
        "report",
        "plan",
        "--schema",
        path.to_str().unwrap(),
        "--intent",
        intent,
        "--json",
    ];
    let first = run_powerbi(&args);
    assert_eq!(first.code, 0, "{}", first.stderr);
    assert_eq!(first.stdout, run_powerbi(&args).stdout);
    assert_json_snapshot(
        "report-plan-intent-fact-override",
        &stdout_json(&first)["intent"],
    );
    assert_eq!(
        stdout_json(&first)["intent"]["model"]["factTable"],
        "Events"
    );
    let invalid = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        path.to_str().unwrap(),
        "--intent",
        r#"{"questions":["Overview"],"model":{"factTable":"Missing"}}"#,
        "--json",
    ]);
    assert_eq!(invalid.code, 10);
    assert_eq!(
        stderr_json(&invalid)["error"]["pointer"],
        "/intent/model/factTable"
    );
}

#[test]
fn empty_intent_refuses_with_the_exact_question_field() {
    let output = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--intent",
        "{}",
        "--json",
    ]);
    assert_eq!(output.code, 10, "{}", output.stderr);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "plan.missing_input");
    assert_eq!(error["pointer"], "/intent/questions");
    assert_json_snapshot("report-plan-missing-intent", &error);
}

#[test]
fn explicit_missing_kpi_measure_never_falls_back_to_a_matching_display_name() {
    let output = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--intent",
        r#"{"kpis":[{"name":"Total Revenue","measure":"Missing"}]}"#,
        "--json",
    ]);
    assert_eq!(output.code, 10, "{}", output.stderr);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "plan.missing_input");
    assert_eq!(error["pointer"], "/intent/kpis/0/measure");
    assert!(
        error["candidates"]
            .as_array()
            .unwrap()
            .contains(&json!("FactSales[Total Revenue]"))
    );
    assert_json_snapshot("report-plan-missing-kpi", &error);
    let rule = run_powerbi(&["lint", "--explain", "plan.missing_input", "--json"]);
    assert_eq!(rule.code, 0, "{}", rule.stderr);
    assert_eq!(stdout_json(&rule)["rule"]["id"], "plan.missing_input");
}
