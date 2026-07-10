mod common;

use common::assert_unsupported_feature;
use serde_json::Value;
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
fn model_calculated_columns_list_starts_empty() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");

    let list = run_powerbi(&[
        "model",
        "calculated-columns",
        "list",
        "--project",
        out,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(
        list_json["schema"],
        Value::from("powerbi-cli.model.calculatedColumns.list.v1")
    );
    assert_eq!(list_json["counts"]["calculatedColumns"], Value::from(0));
    assert_eq!(
        list_json["calculatedColumns"]
            .as_array()
            .expect("calculated columns")
            .len(),
        0
    );
}

#[test]
fn in_place_delete_rolls_back_when_a_relationship_depends_on_the_column() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");
    let column_handle = "column:FactSales:CustomerKey Calc";

    let add_column = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "CustomerKey Calc",
        "--expression",
        "'FactSales'[CustomerKey]",
        "--data-type",
        "int64",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add_column.code, 0, "stderr: {}", add_column.stderr);

    let add_relationship = run_powerbi(&[
        "model",
        "relationships",
        "add",
        "--project",
        out,
        "--from-table",
        "FactSales",
        "--from-column",
        "CustomerKey Calc",
        "--to-table",
        "DimCustomer",
        "--to-column",
        "CustomerKey",
        "--in-place",
        "--json",
    ]);
    assert_eq!(
        add_relationship.code, 0,
        "stderr: {}",
        add_relationship.stderr
    );

    let table_path = fact_sales_tmdl(&project);
    let before_delete = fs::read_to_string(&table_path).expect("table before rejected delete");
    let delete = run_powerbi(&[
        "model",
        "calculated-columns",
        "delete",
        "--project",
        out,
        "--handle",
        column_handle,
        "--in-place",
        "--confirm",
        column_handle,
        "--json",
    ]);
    assert_eq!(delete.code, 10, "stderr: {}", delete.stderr);
    let delete_json = stdout_json(&delete);
    assert_eq!(delete_json["ok"], Value::Bool(false));
    assert_eq!(delete_json["projectModified"], Value::Bool(false));
    assert_eq!(delete_json["rollback"]["performed"], Value::Bool(true));
    assert!(
        delete_json["validation"]["errors"]
            .as_array()
            .expect("validation errors")
            .iter()
            .filter_map(Value::as_str)
            .any(|message| message.contains("CustomerKey Calc")),
        "validation error must explain the dependent relationship: {delete_json:?}"
    );
    assert_eq!(
        fs::read_to_string(&table_path).expect("table after rollback"),
        before_delete,
        "failed in-place mutation must restore the original table document"
    );

    let validate = run_powerbi(&["validate", "--strict", out, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
}

#[test]
fn model_calculated_columns_add_update_delete_round_trip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");
    let table_path = fact_sales_tmdl(&project);
    let before = fs::read_to_string(&table_path).expect("FactSales.tmdl");

    let dry = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Revenue Band",
        "--expression",
        "IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")",
        "--data-type",
        "string",
        "--description",
        "Synthetic revenue band",
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
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Revenue Band",
        "--expression",
        "IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")",
        "--data-type",
        "string",
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
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Revenue Band",
        "--expression",
        "IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")",
        "--data-type",
        "string",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    assert_eq!(add_json["dryRun"], Value::Bool(false));
    assert_eq!(
        add_json["target"]["handle"],
        Value::from("column:FactSales:Revenue Band")
    );
    assert!(
        add_json["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("calculated-columns show")
    );

    let show_added = run_powerbi(&[
        "model",
        "calculated-columns",
        "show",
        "--project",
        out,
        "--handle",
        "column:FactSales:Revenue Band",
        "--json",
    ]);
    assert_eq!(show_added.code, 0, "stderr: {}", show_added.stderr);
    let added_json = stdout_json(&show_added);
    assert_eq!(
        added_json["calculatedColumn"]["expression"],
        Value::from("IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")")
    );
    assert_eq!(
        added_json["calculatedColumn"]["properties"]["dataType"],
        Value::from("string")
    );
    assert!(
        added_json["block"]
            .as_str()
            .unwrap_or_default()
            .contains("column 'Revenue Band' =")
    );

    let dax_file = temp.path().join("revenue-band.dax");
    fs::write(
        &dax_file,
        "IF(\n    'FactSales'[Revenue] >= 5000,\n    \"High\",\n    \"Standard\"\n)\n",
    )
    .expect("write dax file");
    let dax_path = dax_file.to_str().expect("dax path");
    let update = run_powerbi(&[
        "model",
        "calculated-columns",
        "update",
        "--project",
        out,
        "--handle",
        "column:FactSales:Revenue Band",
        "--expression-file",
        dax_path,
        "--description",
        "Updated revenue band",
        "--hidden",
        "--in-place",
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);

    let show_updated = run_powerbi(&[
        "model",
        "calculated-columns",
        "show",
        "--project",
        out,
        "--handle",
        "column:FactSales:Revenue Band",
        "--json",
    ]);
    assert_eq!(show_updated.code, 0, "stderr: {}", show_updated.stderr);
    let updated_json = stdout_json(&show_updated);
    assert!(
        updated_json["calculatedColumn"]["expression"]
            .as_str()
            .unwrap_or_default()
            .contains("'FactSales'[Revenue] >= 5000")
    );
    assert_eq!(
        updated_json["calculatedColumn"]["properties"]["description"],
        Value::from("Updated revenue band")
    );
    let updated_block = updated_json["block"].as_str().expect("updated block");
    assert!(updated_block.contains("/// Updated revenue band"));
    assert!(!updated_block.contains("description:"));
    assert_eq!(
        updated_json["calculatedColumn"]["properties"]["isHidden"],
        Value::Bool(true)
    );

    let deep = run_powerbi(&["inspect", "--deep", out, "--json"]);
    assert_eq!(deep.code, 0, "stderr: {}", deep.stderr);
    let deep_json = stdout_json(&deep);
    assert!(
        deep_json["deep"]["model"]["tables"]
            .as_array()
            .expect("tables")
            .iter()
            .flat_map(|table| table["columns"].as_array().into_iter().flatten())
            .any(|column| {
                column["handle"] == "column:FactSales:Revenue Band"
                    && column["isCalculated"] == Value::Bool(true)
                    && column["expression"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("'FactSales'[Revenue] >= 5000")
            })
    );

    let delete_without_confirm = run_powerbi(&[
        "model",
        "calculated-columns",
        "delete",
        "--project",
        out,
        "--handle",
        "column:FactSales:Revenue Band",
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
        "calculated-columns",
        "delete",
        "--project",
        out,
        "--handle",
        "column:FactSales:Revenue Band",
        "--in-place",
        "--confirm",
        "column:FactSales:Revenue Band",
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    let delete_json = stdout_json(&delete);
    assert!(
        delete_json["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("calculated-columns list")
    );

    let validate = run_powerbi(&["validate", "--strict", out, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);

    let show_deleted = run_powerbi(&[
        "model",
        "calculated-columns",
        "show",
        "--project",
        out,
        "--handle",
        "column:FactSales:Revenue Band",
        "--json",
    ]);
    assert_eq!(show_deleted.code, 10);
    assert_eq!(
        stderr_json(&show_deleted)["error"]["code"],
        Value::from("validation_failed")
    );
}

#[test]
fn model_calculated_columns_update_refuses_lossy_unknown_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");
    let table_path = fact_sales_tmdl(&project);

    let add = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Revenue Band",
        "--expression",
        "IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")",
        "--data-type",
        "string",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let text = fs::read_to_string(&table_path).expect("FactSales.tmdl");
    let marker =
        "    column 'Revenue Band' = IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")\n";
    let with_metadata = text.replace(
        marker,
        &format!("{marker}        annotation PBI_FormatHint = \"Desktop-authored\"\n"),
    );
    assert_ne!(text, with_metadata, "test fixture marker should be present");
    fs::write(&table_path, &with_metadata).expect("write unsupported column metadata");

    let update = run_powerbi(&[
        "model",
        "calculated-columns",
        "update",
        "--project",
        out,
        "--handle",
        "column:FactSales:Revenue Band",
        "--expression",
        "IF('FactSales'[Revenue] >= 5000, \"High\", \"Standard\")",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(update.code, 2);
    let error = assert_unsupported_feature(
        &update.stderr,
        "calculated column update would drop unsupported TMDL line",
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
                .contains("model calculated-columns show"))
    );
    assert_eq!(
        with_metadata,
        fs::read_to_string(&table_path).expect("FactSales.tmdl after refused dry-run")
    );
}

#[test]
fn model_calculated_columns_require_explicit_output_mode_and_data_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");

    let no_mode = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "No Mode",
        "--expression",
        "1",
        "--data-type",
        "int64",
        "--json",
    ]);
    assert_eq!(no_mode.code, 2);
    assert!(
        stderr_json(&no_mode)["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("--dry-run")
    );

    let no_type = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "No Type",
        "--expression",
        "1",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(no_type.code, 2);
    assert!(
        stderr_json(&no_type)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--data-type")
    );

    let duplicate_base_column = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        out,
        "--table",
        "FactSales",
        "--name",
        "Revenue",
        "--expression",
        "1",
        "--data-type",
        "int64",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(duplicate_base_column.code, 2);
    assert!(
        stderr_json(&duplicate_base_column)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("column already exists")
    );
}

#[test]
fn calculated_column_date_matches_scaffold_date_tmdl_and_rejects_unknown_types() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    let add = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--name",
        "Next Date",
        "--expression",
        "'DimDate'[Date] + 1",
        "--data-type",
        "date",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let show = run_powerbi(&[
        "model",
        "calculated-columns",
        "show",
        "--project",
        project_arg,
        "--handle",
        "column:DimDate:Next Date",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["calculatedColumn"]["properties"]["dataType"],
        "dateTime"
    );
    assert_eq!(
        show_json["calculatedColumn"]["properties"]["formatString"],
        "Short Date"
    );
    let dim_date = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("DimDate.tmdl");
    let tmdl = fs::read_to_string(dim_date).expect("DimDate.tmdl");
    assert!(tmdl.contains("        dataType: dateTime"));
    assert!(tmdl.contains("        formatString: \"Short Date\""));
    assert!(!tmdl.contains("        dataType: date\n"));

    let unsupported = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--name",
        "Geo",
        "--expression",
        "1",
        "--data-type",
        "geography",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unsupported.code, 2);
    assert_unsupported_feature(
        &unsupported.stderr,
        "unsupported calculated column data type: geography",
    );
}

#[test]
fn calculated_column_mutations_reject_confirm_on_update() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    let confirm = run_powerbi(&[
        "model",
        "calculated-columns",
        "update",
        "--project",
        project_arg,
        "--handle",
        "column:FactSales:Revenue",
        "--expression",
        "1",
        "--dry-run",
        "--confirm",
        "column:FactSales:Revenue",
        "--json",
    ]);
    assert_eq!(confirm.code, 2);
    assert!(
        stderr_json(&confirm)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--confirm is only valid for model calculated-columns delete")
    );
}
