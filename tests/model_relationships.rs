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

fn scaffold_custom_project(root: &Path) -> PathBuf {
    let schema = root.join("relationship.schema.json");
    fs::write(
        &schema,
        serde_json::to_string_pretty(&json!({
            "name": "RelationshipHarness",
            "displayName": "Relationship Harness",
            "tables": [
                {
                    "name": "FactWork",
                    "columns": [
                        {"name": "AKey", "dataType": "int64", "isKey": true},
                        {"name": "BKey", "dataType": "int64"},
                        {"name": "Amount", "dataType": "double"}
                    ],
                    "rows": [
                        {"AKey": 1, "BKey": 10, "Amount": 42.0}
                    ]
                },
                {
                    "name": "DimA",
                    "columns": [
                        {"name": "AKey", "dataType": "int64", "isKey": true},
                        {"name": "Name", "dataType": "string"}
                    ],
                    "rows": [
                        {"AKey": 1, "Name": "Alpha"}
                    ]
                },
                {
                    "name": "DimB",
                    "columns": [
                        {"name": "BKey", "dataType": "int64", "isKey": true},
                        {"name": "Name", "dataType": "string"}
                    ],
                    "rows": [
                        {"BKey": 10, "Name": "Beta"}
                    ]
                }
            ],
            "relationships": [
                {
                    "fromTable": "FactWork",
                    "fromColumn": "AKey",
                    "toTable": "DimA",
                    "toColumn": "AKey"
                }
            ],
            "pages": []
        }))
        .expect("schema JSON"),
    )
    .expect("write schema");
    let out_dir = root.join("relationship_project");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        schema.to_str().expect("schema path"),
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

fn relationships_tmdl(project: &Path) -> PathBuf {
    project
        .join("RelationshipHarness.SemanticModel")
        .join("definition")
        .join("relationships.tmdl")
}

#[test]
fn model_relationships_list_show_and_inspect_scaffolded_relationships() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");

    let list = run_powerbi(&["model", "relationships", "list", "--project", out, "--json"]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(
        list_json["schema"],
        Value::from("powerbi-cli.model.relationships.list.v1")
    );
    assert_eq!(list_json["counts"]["relationships"], Value::from(2));
    let relationships = list_json["relationships"]
        .as_array()
        .expect("relationships");
    let date_relationship = relationships
        .iter()
        .find(|relationship| {
            relationship["fromTable"] == "FactSales"
                && relationship["fromColumn"] == "DateKey"
                && relationship["toTable"] == "DimDate"
                && relationship["toColumn"] == "DateKey"
        })
        .expect("date relationship");
    assert_eq!(
        date_relationship["properties"]["crossFilteringBehavior"],
        Value::from("oneDirection")
    );
    assert_eq!(
        date_relationship["properties"]["isActive"],
        Value::Bool(true)
    );
    assert_eq!(
        date_relationship["from"]["columnHandle"],
        Value::from("column:FactSales:DateKey")
    );
    let handle = date_relationship["handle"].as_str().expect("handle");

    let show = run_powerbi(&[
        "model",
        "relationships",
        "show",
        "--project",
        out,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(show_json["relationship"]["handle"], Value::from(handle));
    assert!(
        show_json["block"]
            .as_str()
            .unwrap_or_default()
            .contains("fromColumn: 'FactSales'.'DateKey'")
    );

    let deep = run_powerbi(&["inspect", "--deep", out, "--json"]);
    assert_eq!(deep.code, 0, "stderr: {}", deep.stderr);
    let deep_json = stdout_json(&deep);
    assert_eq!(deep_json["counts"]["relationships"], Value::from(2));
    assert!(
        deep_json["deep"]["model"]["relationships"]
            .as_array()
            .expect("deep relationships")
            .iter()
            .any(|relationship| {
                relationship["handle"] == handle
                    && relationship["from"]["columnHandle"] == "column:FactSales:DateKey"
                    && relationship["to"]["columnHandle"] == "column:DimDate:DateKey"
            })
    );
}

#[test]
fn model_relationships_add_update_delete_round_trip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_custom_project(temp.path());
    let out = project.to_str().expect("project path");
    let relationship_path = relationships_tmdl(&project);
    let before = fs::read_to_string(&relationship_path).expect("relationships.tmdl");

    let dry = run_powerbi(&[
        "model",
        "relationships",
        "add",
        "--project",
        out,
        "--from-table",
        "FactWork",
        "--from-column",
        "BKey",
        "--to-table",
        "DimB",
        "--to-column",
        "BKey",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    assert_eq!(stdout_json(&dry)["dryRun"], Value::Bool(true));
    assert_eq!(
        before,
        fs::read_to_string(&relationship_path).expect("relationships.tmdl after dry-run")
    );

    let copy_dir = temp.path().join("relationship_copy");
    let copy = copy_dir.to_str().expect("copy path");
    let out_dir_add = run_powerbi(&[
        "model",
        "relationships",
        "add",
        "--project",
        out,
        "--from-table",
        "FactWork",
        "--from-column",
        "BKey",
        "--to-table",
        "DimB",
        "--to-column",
        "BKey",
        "--out-dir",
        copy,
        "--json",
    ]);
    assert_eq!(out_dir_add.code, 0, "stderr: {}", out_dir_add.stderr);
    assert_eq!(stdout_json(&out_dir_add)["mode"], Value::from("out-dir"));
    assert_eq!(
        before,
        fs::read_to_string(&relationship_path).expect("relationships.tmdl after --out-dir")
    );
    let copy_validate = run_powerbi(&["validate", "--strict", copy, "--json"]);
    assert_eq!(copy_validate.code, 0, "stderr: {}", copy_validate.stderr);

    let add = run_powerbi(&[
        "model",
        "relationships",
        "add",
        "--project",
        out,
        "--from-table",
        "FactWork",
        "--from-column",
        "BKey",
        "--to-table",
        "DimB",
        "--to-column",
        "BKey",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    let handle = add_json["target"]["handle"].as_str().expect("handle");
    assert!(handle.starts_with("relationship:rel"));
    assert!(
        add_json["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("relationships show")
    );

    let update = run_powerbi(&[
        "model",
        "relationships",
        "update",
        "--project",
        out,
        "--handle",
        handle,
        "--cross-filtering-behavior",
        "bothDirections",
        "--inactive",
        "--in-place",
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);

    let show_updated = run_powerbi(&[
        "model",
        "relationships",
        "show",
        "--project",
        out,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show_updated.code, 0, "stderr: {}", show_updated.stderr);
    let updated_json = stdout_json(&show_updated);
    assert_eq!(
        updated_json["relationship"]["properties"]["crossFilteringBehavior"],
        Value::from("bothDirections")
    );
    assert_eq!(
        updated_json["relationship"]["properties"]["isActive"],
        Value::Bool(false)
    );

    let endpoint_update = run_powerbi(&[
        "model",
        "relationships",
        "update",
        "--project",
        out,
        "--handle",
        handle,
        "--from-column",
        "AKey",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(endpoint_update.code, 2);
    assert!(
        stderr_json(&endpoint_update)["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("delete")
    );

    let delete_without_confirm = run_powerbi(&[
        "model",
        "relationships",
        "delete",
        "--project",
        out,
        "--handle",
        handle,
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
        "relationships",
        "delete",
        "--project",
        out,
        "--handle",
        handle,
        "--in-place",
        "--confirm",
        handle,
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);

    let validate = run_powerbi(&["validate", "--strict", out, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(
        stdout_json(&validate)["counts"]["relationships"],
        Value::from(1)
    );

    let show_deleted = run_powerbi(&[
        "model",
        "relationships",
        "show",
        "--project",
        out,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show_deleted.code, 10);
    assert_eq!(
        stderr_json(&show_deleted)["error"]["code"],
        Value::from("validation_failed")
    );
}

#[test]
fn model_relationships_validate_missing_endpoint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_custom_project(temp.path());
    let out = project.to_str().expect("project path");

    let missing_endpoint = run_powerbi(&[
        "model",
        "relationships",
        "add",
        "--project",
        out,
        "--from-table",
        "FactWork",
        "--from-column",
        "MissingKey",
        "--to-table",
        "DimB",
        "--to-column",
        "BKey",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_endpoint.code, 10);
    assert!(
        stderr_json(&missing_endpoint)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("relationship fromColumn column not found")
    );
}

#[test]
fn model_relationships_update_refuses_lossy_unknown_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_custom_project(temp.path());
    let out = project.to_str().expect("project path");

    let list = run_powerbi(&["model", "relationships", "list", "--project", out, "--json"]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let handle = stdout_json(&list)["relationships"][0]["handle"]
        .as_str()
        .expect("handle")
        .to_string();

    let relationship_path = relationships_tmdl(&project);
    let text = fs::read_to_string(&relationship_path).expect("relationships.tmdl");
    fs::write(
        &relationship_path,
        text.replace(
            "    crossFilteringBehavior: oneDirection\n",
            "    crossFilteringBehavior: oneDirection\n    lineageTag: desktop-authored\n",
        ),
    )
    .expect("write unsupported relationship metadata");

    let update = run_powerbi(&[
        "model",
        "relationships",
        "update",
        "--project",
        out,
        "--handle",
        &handle,
        "--inactive",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(update.code, 2);
    assert_unsupported_feature(&update.stderr, "unsupported TMDL line");
}

#[test]
fn relationship_mutations_reject_confirm_on_add() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_custom_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    let confirm = run_powerbi(&[
        "model",
        "relationships",
        "add",
        "--project",
        project_arg,
        "--from-table",
        "FactWork",
        "--from-column",
        "BKey",
        "--to-table",
        "DimB",
        "--to-column",
        "BKey",
        "--dry-run",
        "--confirm",
        "relationship:unused",
        "--json",
    ]);
    assert_eq!(confirm.code, 2);
    assert!(
        stderr_json(&confirm)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--confirm is only valid for model relationships delete")
    );
}
