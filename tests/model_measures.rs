mod common;

use common::assert_unsupported_feature;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct RunOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_powerbi(args: &[&str]) -> RunOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
        .output()
        .expect("run powerbi-cli binary");
    RunOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

fn scaffold_sales_project(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "--json",
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert_eq!(stdout_json(&output)["ok"], Value::Bool(true));
    out_dir
}

fn fact_sales_tmdl(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("FactSales.tmdl")
}

#[test]
fn model_measures_list_and_show_scaffolded_dax() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");

    let list = run_powerbi(&["model", "measures", "list", "--project", out, "--json"]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(
        list_json["schema"],
        Value::from("powerbi-cli.model.measures.list.v1")
    );
    assert_eq!(list_json["counts"]["measures"], Value::from(2));
    assert!(
        list_json["measures"]
            .as_array()
            .expect("measures")
            .iter()
            .any(|measure| measure["handle"] == "measure:FactSales:Total Revenue")
    );

    let show = run_powerbi(&[
        "model",
        "measures",
        "show",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Total Revenue",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(show_json["measure"]["table"], Value::from("FactSales"));
    assert_eq!(
        show_json["measure"]["expression"],
        Value::from("SUM('FactSales'[Revenue])")
    );
    assert!(
        show_json["block"]
            .as_str()
            .unwrap_or_default()
            .contains("measure")
    );
}

#[test]
fn model_dax_bridge_plan_reports_inventory_and_validation_boundary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");

    let bridge = run_powerbi(&[
        "model",
        "dax",
        "bridge-plan",
        "--project",
        out,
        "--engine",
        "desktop",
        "--json",
    ]);
    assert_eq!(bridge.code, 0, "stderr: {}", bridge.stderr);
    let bridge_json = stdout_json(&bridge);
    assert_eq!(
        bridge_json["schema"],
        Value::from("powerbi-cli.model.dax.bridgePlan.v1")
    );
    assert_eq!(bridge_json["counts"]["measures"], Value::from(2));
    assert_eq!(
        bridge_json["bridge"]["requestedEngine"],
        Value::from("desktop")
    );
    assert_eq!(bridge_json["bridge"]["noFakeFallbacks"], Value::Bool(true));
    assert_eq!(
        bridge_json["validationBridge"]["offlineDaxParser"]["available"],
        Value::Bool(false)
    );
    assert!(
        bridge_json["daxInventory"]["measures"]
            .as_array()
            .expect("measures")
            .iter()
            .any(|measure| measure["handle"] == "measure:FactSales:Total Revenue")
    );

    let invalid_engine = run_powerbi(&[
        "model",
        "dax",
        "bridge-plan",
        "--project",
        out,
        "--engine",
        "sparkle",
        "--json",
    ]);
    assert_eq!(invalid_engine.code, 2);
    let invalid_error = stderr_json(&invalid_engine);
    assert_eq!(invalid_error["error"]["code"], "invalid_args");
    assert_eq!(
        invalid_error["error"]["message"],
        "invalid DAX bridge engine: sparkle"
    );
}

#[test]
fn model_dax_execute_requires_data_and_oracle_opt_ins_without_echoing_query() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");
    let query = "EVALUATE ROW(\"PrivateLabel\", 42)";

    let missing_data_opt_in = run_powerbi(&[
        "model",
        "dax",
        "execute",
        "--project",
        out,
        "--query",
        query,
        "--json",
    ]);
    assert_eq!(missing_data_opt_in.code, 2);
    assert_eq!(
        stderr_json(&missing_data_opt_in)["error"]["message"],
        "model dax execute requires --allow-data-read"
    );

    let oracle_disabled = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args([
            "model",
            "dax",
            "execute",
            "--project",
            out,
            "--query",
            query,
            "--allow-data-read",
            "--json",
        ])
        .env_remove("POWERBI_DESKTOP_ORACLE")
        .output()
        .expect("run oracle-disabled DAX execute");
    assert_eq!(oracle_disabled.status.code(), Some(30));
    let stdout = String::from_utf8_lossy(&oracle_disabled.stdout);
    let value: Value = serde_json::from_str(stdout.trim()).expect("stdout JSON");
    assert_eq!(
        value["stage"],
        if cfg!(windows) {
            "oracle-opt-in"
        } else {
            "platform"
        }
    );
    assert_eq!(value["query"]["textReturned"], Value::Bool(false));
    assert!(!stdout.contains("PrivateLabel"));

    let mutation_payload = run_powerbi(&[
        "model",
        "dax",
        "execute",
        "--project",
        out,
        "--query",
        "CREATE TABLE Nope",
        "--allow-data-read",
        "--json",
    ]);
    assert_eq!(mutation_payload.code, 2);
    assert!(
        stderr_json(&mutation_payload)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("only DAX query forms")
    );
}

#[test]
fn model_measures_add_update_delete_round_trip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");
    let table_path = fact_sales_tmdl(&project);
    let before = fs::read_to_string(&table_path).expect("FactSales.tmdl");

    let dry = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Average Revenue",
        "--expression",
        "DIVIDE([Total Revenue], [Total Units])",
        "--format-string",
        "$#,0.00",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        before,
        fs::read_to_string(&table_path).expect("FactSales.tmdl after dry-run")
    );

    let copy_dir = temp.path().join("sales_copy");
    let copy = copy_dir.to_str().expect("copy path");
    let out_dir_add = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Average Revenue",
        "--expression",
        "DIVIDE([Total Revenue], [Total Units])",
        "--out-dir",
        copy,
        "--json",
    ]);
    assert_eq!(out_dir_add.code, 0, "stderr: {}", out_dir_add.stderr);
    assert_eq!(stdout_json(&out_dir_add)["mode"], Value::from("out-dir"));
    assert_eq!(
        before,
        fs::read_to_string(&table_path).expect("FactSales.tmdl after --out-dir")
    );
    let copy_validate = run_powerbi(&["validate", "--strict", copy, "--json"]);
    assert_eq!(copy_validate.code, 0, "stderr: {}", copy_validate.stderr);

    let add = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Average Revenue",
        "--expression",
        "DIVIDE([Total Revenue], [Total Units])",
        "--format-string",
        "$#,0.00",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    assert_eq!(add_json["dryRun"], Value::Bool(false));
    assert!(
        add_json["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("show")
    );

    let show_added = run_powerbi(&[
        "model",
        "measures",
        "show",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Average Revenue",
        "--json",
    ]);
    assert_eq!(show_added.code, 0, "stderr: {}", show_added.stderr);
    assert_eq!(
        stdout_json(&show_added)["measure"]["expression"],
        Value::from("DIVIDE([Total Revenue], [Total Units])")
    );

    let dax_file = temp.path().join("average-revenue.dax");
    fs::write(&dax_file, "DIVIDE(\n    [Total Revenue],\n    1000\n)\n").expect("write dax file");
    let dax_path = dax_file.to_str().expect("dax path");
    let update = run_powerbi(&[
        "model",
        "measures",
        "update",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Average Revenue",
        "--expression-file",
        dax_path,
        "--description",
        "Revenue in thousands",
        "--in-place",
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);

    let show_updated = run_powerbi(&[
        "model",
        "measures",
        "show",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Average Revenue",
        "--json",
    ]);
    assert_eq!(show_updated.code, 0, "stderr: {}", show_updated.stderr);
    let updated_json = stdout_json(&show_updated);
    assert!(
        updated_json["measure"]["expression"]
            .as_str()
            .unwrap_or_default()
            .contains("[Total Revenue]")
    );
    assert_eq!(
        updated_json["measure"]["properties"]["description"],
        Value::from("Revenue in thousands")
    );

    let delete_without_confirm = run_powerbi(&[
        "model",
        "measures",
        "delete",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Average Revenue",
        "--in-place",
        "--json",
    ]);
    assert_eq!(delete_without_confirm.code, 2);
    assert_eq!(
        stderr_json(&delete_without_confirm)["error"]["code"],
        Value::from("invalid_args")
    );

    let delete = run_powerbi(&[
        "model",
        "measures",
        "delete",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Average Revenue",
        "--in-place",
        "--confirm",
        "measure:FactSales:Average Revenue",
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    let delete_json = stdout_json(&delete);
    assert!(
        delete_json["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("list")
    );

    let validate = run_powerbi(&["validate", "--strict", out, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["counts"]["measures"], Value::from(2));

    let show_deleted = run_powerbi(&[
        "model",
        "measures",
        "show",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Average Revenue",
        "--json",
    ]);
    assert_eq!(show_deleted.code, 10);
    assert_eq!(
        stderr_json(&show_deleted)["error"]["code"],
        Value::from("validation_failed")
    );
}

#[test]
fn model_measures_update_refuses_lossy_unknown_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");
    let table_path = fact_sales_tmdl(&project);
    let text = fs::read_to_string(&table_path).expect("FactSales.tmdl");
    let marker = "    measure 'Total Revenue' = SUM('FactSales'[Revenue])\n";
    let with_metadata = text.replace(
        marker,
        &format!("{marker}        annotation PBI_FormatHint = \"Desktop-authored\"\n"),
    );
    assert_ne!(text, with_metadata, "test fixture marker should be present");
    fs::write(&table_path, &with_metadata).expect("write unsupported measure metadata");

    let update = run_powerbi(&[
        "model",
        "measures",
        "update",
        "--project",
        out,
        "--handle",
        "measure:FactSales:Total Revenue",
        "--expression",
        "SUM('FactSales'[Revenue])",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(update.code, 2);
    let error = assert_unsupported_feature(
        &update.stderr,
        "measure update would drop unsupported TMDL line",
    );
    assert!(update.stdout.trim().is_empty());
    assert!(
        error["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("model measures show"))
    );
    assert_eq!(
        with_metadata,
        fs::read_to_string(&table_path).expect("FactSales.tmdl after refused dry-run")
    );
}

#[test]
fn model_measure_mutations_require_explicit_output_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "No Mode",
        "--expression",
        "1",
        "--json",
    ]);
    assert_eq!(output.code, 2);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], Value::from("invalid_args"));
    assert!(
        error["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("--dry-run")
    );
}

#[test]
fn scaffolded_multiline_measure_round_trips_through_tmdl_reader() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema = temp.path().join("multiline.schema.json");
    fs::write(
        &schema,
        serde_json::to_string_pretty(&json!({
            "name": "MultilineModel",
            "tables": [{
                "name": "Facts",
                "columns": [{"name": "Value", "dataType": "int64"}],
                "measures": [{
                    "name": "Multiline Measure",
                    "expression": "VAR x = 1\nRETURN x"
                }]
            }],
            "pages": [{"name": "ReportSectionMain", "displayName": "Main"}]
        }))
        .expect("schema JSON"),
    )
    .expect("write schema");
    let project = temp.path().join("multiline_project");
    let scaffold = run_powerbi(&[
        "scaffold",
        "--schema",
        schema.to_str().expect("schema path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(scaffold.code, 0, "stderr: {}", scaffold.stderr);

    let table = project
        .join("MultilineModel.SemanticModel")
        .join("definition")
        .join("tables")
        .join("Facts.tmdl");
    let tmdl = fs::read_to_string(table).expect("Facts.tmdl");
    assert!(tmdl.contains(
        "    measure 'Multiline Measure' =\n            VAR x = 1\n            RETURN x\n        lineageTag:"
    ));

    let show = run_powerbi(&[
        "model",
        "measures",
        "show",
        "--project",
        project.to_str().expect("project path"),
        "--handle",
        "measure:Facts:Multiline Measure",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    assert_eq!(
        stdout_json(&show)["measure"]["expression"],
        Value::from("VAR x = 1\nRETURN x")
    );
}

#[test]
fn measure_mutations_reject_confirm_on_add() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    let confirm = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "Confirmed Add",
        "--expression",
        "1",
        "--dry-run",
        "--confirm",
        "measure:FactSales:Confirmed Add",
        "--json",
    ]);
    assert_eq!(confirm.code, 2);
    assert!(
        stderr_json(&confirm)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--confirm is only valid for model measures delete")
    );
}
