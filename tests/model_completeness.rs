mod common;

use common::{run_powerbi, stdout_json};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn scaffold(root: &Path) -> PathBuf {
    let project = root.join("sales");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    project
}

fn table_path(project: &Path, table: &str) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join(format!("{table}.tmdl"))
}

#[test]
fn table_and_column_list_show_expose_stable_handles_and_raw_blocks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(temp.path());
    let project_arg = project.to_str().expect("project path");

    let tables = run_powerbi(&[
        "model",
        "tables",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(tables.code, 0, "stderr: {}", tables.stderr);
    let tables_json = stdout_json(&tables);
    assert_eq!(tables_json["schema"], "powerbi-cli.model.tables.list.v1");
    assert!(
        tables_json["tables"]
            .as_array()
            .expect("tables")
            .iter()
            .any(|table| {
                table["handle"] == "table:FactSales" && table["counts"]["columns"] == 4
            })
    );

    let columns = run_powerbi(&[
        "model",
        "columns",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(columns.code, 0, "stderr: {}", columns.stderr);
    let columns_json = stdout_json(&columns);
    assert_eq!(columns_json["schema"], "powerbi-cli.model.columns.list.v1");
    assert!(
        columns_json["columns"]
            .as_array()
            .expect("columns")
            .iter()
            .any(|column| {
                column["handle"] == "column:FactSales:Revenue"
                    && column["properties"]["dataType"] == "decimal"
            })
    );

    let show = run_powerbi(&[
        "model",
        "columns",
        "show",
        "--project",
        project_arg,
        "--handle",
        "column:FactSales:Revenue",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(show_json["column"]["handle"], "column:FactSales:Revenue");
    assert!(
        show_json["block"]
            .as_str()
            .expect("raw block")
            .contains("sourceColumn: Revenue")
    );
}

#[test]
fn table_show_refuses_over_limit_tmdl_with_input_safety_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(temp.path());
    let path = table_path(&project, "FactSales");
    fs::write(&path, vec![b'x'; 16 * 1024 * 1024 + 1]).expect("oversized table TMDL");
    let output = run_powerbi(&[
        "model",
        "tables",
        "show",
        "--project",
        project.to_str().expect("project path"),
        "--handle",
        "table:FactSales",
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let error: Value = serde_json::from_str(output.stderr.trim()).expect("error JSON");
    assert_eq!(error["error"]["code"], "input_safety_violation");
}

#[test]
fn table_rename_refuses_over_limit_reference_file_before_mutation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(temp.path());
    let relationships = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("relationships.tmdl");
    fs::write(&relationships, vec![b'x'; 16 * 1024 * 1024 + 1])
        .expect("oversized relationships TMDL");
    let output = run_powerbi(&[
        "model",
        "tables",
        "rename",
        "--project",
        project.to_str().expect("project path"),
        "--handle",
        "table:DimDate",
        "--new-name",
        "Calendar",
        "--rename-references",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let error: Value = serde_json::from_str(output.stderr.trim()).expect("error JSON");
    assert_eq!(error["error"]["code"], "input_safety_violation");
    assert!(!table_path(&project, "Calendar").exists());
}

#[test]
fn table_and_column_add_support_dry_run_out_dir_and_deterministic_plans() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(temp.path());
    let project_arg = project.to_str().expect("project path");
    // Table/file names deliberately reject `:`; the stable-handle encoding is
    // covered by the column-name test below.  Use a portable table name here.
    let table_args = [
        "model",
        "tables",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimNew",
        "--column",
        "Code",
        "--data-type",
        "string",
        "--dry-run",
        "--json",
    ];
    let first = run_powerbi(&table_args);
    let second = run_powerbi(&table_args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "same input must yield byte-identical JSON plan"
    );
    assert!(!table_path(&project, "DimNew").exists());

    let out_dir = temp.path().join("sales-out");
    let output = run_powerbi(&[
        "model",
        "tables",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimNew",
        "--column",
        "Code",
        "--data-type",
        "string",
        "--out-dir",
        out_dir.to_str().expect("out dir"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert_eq!(stdout_json(&output)["projectModified"], true);
    assert!(table_path(&out_dir, "DimNew").is_file());
    assert!(!table_path(&project, "DimNew").exists());

    let column = run_powerbi(&[
        "model",
        "columns",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "Margin:Net",
        "--data-type",
        "decimal",
        "--in-place",
        "--json",
    ]);
    assert_eq!(column.code, 0, "stderr: {}", column.stderr);
    let column_json = stdout_json(&column);
    assert_eq!(
        column_json["target"]["handle"],
        "column:FactSales:Margin%3ANet"
    );
    assert!(
        fs::read_to_string(table_path(&project, "FactSales"))
            .expect("FactSales")
            .contains("column 'Margin:Net'")
    );
}

#[test]
fn column_update_refuses_unknown_desktop_metadata_and_delete_requires_confirmation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(temp.path());
    let path = table_path(&project, "FactSales");
    let original = fs::read_to_string(&path).expect("FactSales");
    let annotated = original.replacen(
        "        sourceColumn: Revenue\n",
        "        sourceColumn: Revenue\n        extendedProperty DesktopOnly = keep\n",
        1,
    );
    fs::write(&path, &annotated).expect("annotated FactSales");
    let project_arg = project.to_str().expect("project path");

    let update = run_powerbi(&[
        "model",
        "columns",
        "update",
        "--project",
        project_arg,
        "--handle",
        "column:FactSales:Revenue",
        "--format-string",
        "USD",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(update.code, 2);
    let error: Value = serde_json::from_str(update.stderr.trim()).expect("error JSON");
    assert_eq!(error["error"]["code"], "unsupported_feature");
    assert!(
        error["error"]["hint"]
            .as_str()
            .expect("hint")
            .contains("Desktop-authored")
    );
    assert!(
        error["error"]["suggestedCommands"][0]
            .as_str()
            .expect("suggestion")
            .contains("model columns show")
    );
    assert_eq!(
        fs::read_to_string(&path).expect("FactSales after refusal"),
        annotated
    );

    // A mutation of a different block must retain Desktop-authored metadata
    // byte-for-byte; only a mutation of the annotated block is refused.
    let untouched = run_powerbi(&[
        "model",
        "columns",
        "update",
        "--project",
        project_arg,
        "--handle",
        "column:FactSales:Units",
        "--format-string",
        "Units",
        "--in-place",
        "--json",
    ]);
    assert_eq!(untouched.code, 0, "stderr: {}", untouched.stderr);
    assert!(
        fs::read_to_string(&path)
            .expect("FactSales after unrelated update")
            .contains("extendedProperty DesktopOnly = keep")
    );

    let delete = run_powerbi(&[
        "model",
        "columns",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "column:FactSales:Units",
        "--in-place",
        "--json",
    ]);
    assert_eq!(delete.code, 2);
    assert!(delete.stderr.contains("--confirm column:FactSales:Units"));
    let confirmed = run_powerbi(&[
        "model",
        "columns",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "column:FactSales:Units",
        "--in-place",
        "--confirm",
        "column:FactSales:Units",
        "--json",
    ]);
    assert_eq!(confirmed.code, 0, "stderr: {}", confirmed.stderr);
    assert!(
        !fs::read_to_string(&path)
            .expect("FactSales after delete")
            .contains("column Units")
    );
}

#[test]
fn table_rename_refuses_references_then_rewrites_relationships_and_partition_name() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(temp.path());
    let project_arg = project.to_str().expect("project path");
    let date_path = table_path(&project, "DimDate");
    let date_original = fs::read_to_string(&date_path).expect("DimDate");
    let date_with_measure = date_original.replacen(
        "    partition DimDate = m\n",
        "    measure 'Date Key Count' = COUNT('DimDate'[DateKey])\n        lineageTag: 00000000-0000-4000-a000-000000000001\n\n    partition DimDate = m\n",
        1,
    );
    fs::write(&date_path, date_with_measure).expect("self-referencing DimDate measure");

    let refused = run_powerbi(&[
        "model",
        "tables",
        "rename",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--new-name",
        "Calendar",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(refused.code, 10);
    assert!(refused.stderr.contains("--rename-references"));
    assert!(refused.stderr.contains("relationships.tmdl"));

    let renamed = run_powerbi(&[
        "model",
        "tables",
        "rename",
        "--project",
        project_arg,
        "--handle",
        "table:DimDate",
        "--new-name",
        "Calendar",
        "--rename-references",
        "--in-place",
        "--json",
    ]);
    assert_eq!(renamed.code, 0, "stderr: {}", renamed.stderr);
    let renamed_json = stdout_json(&renamed);
    assert_eq!(renamed_json["target"]["handle"], "table:Calendar");
    assert_eq!(renamed_json["target"]["referencesUpdated"], true);
    assert!(table_path(&project, "Calendar").is_file());
    assert!(!table_path(&project, "DimDate").exists());
    let calendar = fs::read_to_string(table_path(&project, "Calendar")).expect("Calendar");
    assert!(calendar.contains("partition Calendar = m"));
    assert!(calendar.contains("COUNT('Calendar'[DateKey])"));
    let relationships = fs::read_to_string(
        project
            .join("SalesOperations.SemanticModel")
            .join("definition")
            .join("relationships.tmdl"),
    )
    .expect("relationships");
    assert!(relationships.contains("'Calendar'.'DateKey'"));
    assert!(!relationships.contains("'DimDate'.'DateKey'"));
}

#[test]
fn table_and_column_diff_scopes_report_semantic_additions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let before = scaffold(&temp.path().join("before"));
    let after = scaffold(&temp.path().join("after"));
    let after_arg = after.to_str().expect("after path");
    let added = run_powerbi(&[
        "model",
        "tables",
        "add",
        "--project",
        after_arg,
        "--table",
        "DimExtra",
        "--column",
        "Code",
        "--in-place",
        "--json",
    ]);
    assert_eq!(added.code, 0, "stderr: {}", added.stderr);
    let column = run_powerbi(&[
        "model",
        "columns",
        "add",
        "--project",
        after_arg,
        "--table",
        "FactSales",
        "--name",
        "Margin",
        "--data-type",
        "decimal",
        "--in-place",
        "--json",
    ]);
    assert_eq!(column.code, 0, "stderr: {}", column.stderr);

    let before_arg = before.to_str().expect("before path");
    let tables = run_powerbi(&[
        "diff",
        before_arg,
        after_arg,
        "--scope",
        "model.tables",
        "--json",
    ]);
    assert_eq!(tables.code, 0, "stderr: {}", tables.stderr);
    let tables_json = stdout_json(&tables);
    assert_eq!(tables_json["scope"], "model.tables");
    assert_eq!(tables_json["summary"]["added"], 1);
    let columns = run_powerbi(&[
        "diff",
        before_arg,
        after_arg,
        "--scope",
        "model.columns",
        "--json",
    ]);
    assert_eq!(columns.code, 0, "stderr: {}", columns.stderr);
    let columns_json = stdout_json(&columns);
    assert_eq!(columns_json["scope"], "model.columns");
    assert_eq!(columns_json["summary"]["added"], 2);
}
