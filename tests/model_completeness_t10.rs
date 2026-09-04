mod common;

use common::{assert_json_snapshot, run_powerbi, scaffold_sales, stderr_json, stdout_json};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

fn table_path(project: &Path, table: &str) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join(format!("{table}.tmdl"))
}

fn expressions_path(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("expressions.tmdl")
}

#[test]
fn calculated_table_add_is_deterministic_and_supports_all_output_modes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let args = [
        "model",
        "tables",
        "add-calculated",
        "--project",
        project_arg,
        "--table",
        "SalesAbovePlan",
        "--expression",
        "FILTER('FactSales', 'FactSales'[Revenue] > 0)",
        "--dry-run",
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout, "dry-run must be deterministic");
    let value = stdout_json(&first);
    assert_eq!(value["action"], "add-calculated");
    assert_eq!(value["target"]["partitionKind"], "calculated");
    assert!(!table_path(&project, "SalesAbovePlan").exists());
    assert_json_snapshot(
        "model-calculated-table-dry-run",
        &json!({
            "schema": value["schema"],
            "action": value["action"],
            "dryRun": value["dryRun"],
            "mode": value["mode"],
            "target": {"handle": value["target"]["handle"], "partitionKind": value["target"]["partitionKind"], "table": value["target"]["table"]},
            "changes": [{"kind": value["changes"][0]["kind"], "action": value["changes"][0]["action"], "before": value["changes"][0]["before"], "after": value["changes"][0]["after"]}]
        }),
    );

    let out_dir = temp.path().join("calculated-out");
    let out_arg = out_dir.to_str().expect("out dir");
    let out = run_powerbi(&[
        "model",
        "tables",
        "add-calculated",
        "--project",
        project_arg,
        "--table",
        "SalesAbovePlan",
        "--expression",
        "FILTER('FactSales', 'FactSales'[Revenue] > 0)",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(stdout_json(&out)["projectModified"], true);
    assert!(table_path(&out_dir, "SalesAbovePlan").is_file());
    assert!(!table_path(&project, "SalesAbovePlan").exists());

    let in_place = run_powerbi(&[
        "model",
        "tables",
        "add-calculated",
        "--project",
        project_arg,
        "--table",
        "SalesAbovePlan",
        "--expression",
        "FILTER('FactSales', 'FactSales'[Revenue] > 0)",
        "--in-place",
        "--json",
    ]);
    assert_eq!(in_place.code, 0, "stderr: {}", in_place.stderr);
    let path = table_path(&project, "SalesAbovePlan");
    let text = fs::read_to_string(&path).expect("calculated table");
    assert!(text.contains("partition SalesAbovePlan = calculated"));
    assert!(text.contains("source = FILTER('FactSales'"));

    let strict = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(
        strict.code, 0,
        "calculated-table schema is deferred to Desktop; stderr: {}",
        strict.stderr
    );

    let dependencies = run_powerbi(&[
        "model",
        "dax",
        "dependencies",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(dependencies.code, 0, "stderr: {}", dependencies.stderr);
    let dependencies = stdout_json(&dependencies);
    assert!(dependencies["expressions"].as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item["kind"] == "calculated-table"
                && item["handle"] == "partition:SalesAbovePlan:SalesAbovePlan"
                && item["references"]["tableColumns"][0]["resolved"] == true
        })
    }));

    let delete = run_powerbi(&[
        "model",
        "tables",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "table:SalesAbovePlan",
        "--in-place",
        "--confirm",
        "table:SalesAbovePlan",
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    assert!(!path.exists());
}

#[test]
fn calculated_table_refusals_are_offline_safe_and_non_mutating() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let empty = run_powerbi(&[
        "model",
        "tables",
        "add-calculated",
        "--project",
        project_arg,
        "--table",
        "EmptyCalc",
        "--expression",
        "   ",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(empty.code, 2);
    assert_eq!(stderr_json(&empty)["error"]["code"], "invalid_args");
    assert!(!table_path(&project, "EmptyCalc").exists());

    let credentials = run_powerbi(&[
        "model",
        "tables",
        "add-calculated",
        "--project",
        project_arg,
        "--table",
        "CredentialCalc",
        "--expression",
        "Web.Contents(\"https://user:password@example.test\")",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(credentials.code, 2);
    let error = stderr_json(&credentials);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("credential-like")
    );
    assert!(!table_path(&project, "CredentialCalc").exists());
}

#[test]
fn named_expression_crud_preserves_newlines_and_m_lint_finds_duplicate_steps() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let add_args = [
        "model",
        "expressions",
        "add",
        "--project",
        project_arg,
        "--name",
        "Transient:Expression",
        "--expression",
        "#table(type table [Value = Int64.Type], {{1}})",
        "--dry-run",
        "--json",
    ];
    let first = run_powerbi(&add_args);
    let second = run_powerbi(&add_args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout, "dry-run must be deterministic");
    let dry = stdout_json(&first);
    assert_eq!(dry["target"]["handle"], "expression:Transient%3AExpression");
    assert!(!expressions_path(&project).exists());
    assert_json_snapshot(
        "model-named-expression-dry-run",
        &json!({
            "schema": dry["schema"],
            "action": dry["action"],
            "dryRun": dry["dryRun"],
            "mode": dry["mode"],
            "target": {"handle": dry["target"]["handle"], "name": dry["target"]["name"]},
            "changes": [{"kind": dry["changes"][0]["kind"], "action": dry["changes"][0]["action"], "before": dry["changes"][0]["before"], "after": dry["changes"][0]["after"]}]
        }),
    );

    let out_dir = temp.path().join("expression-out");
    let out = run_powerbi(&[
        "model",
        "expressions",
        "add",
        "--project",
        project_arg,
        "--name",
        "Transient:Expression",
        "--expression",
        "#table(type table [Value = Int64.Type], {{1}})",
        "--out-dir",
        out_dir.to_str().expect("out dir"),
        "--json",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(expressions_path(&out_dir).is_file());
    assert!(!expressions_path(&project).exists());

    let add = run_powerbi(&[
        "model",
        "expressions",
        "add",
        "--project",
        project_arg,
        "--name",
        "Transient:Expression",
        "--expression",
        "#table(type table [Value = Int64.Type], {{1}})",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let path = expressions_path(&project);
    let before_update = fs::read_to_string(&path).expect("expressions");
    assert!(before_update.contains("expression 'Transient:Expression'"));

    let update = run_powerbi(&[
        "model",
        "expressions",
        "update",
        "--project",
        project_arg,
        "--handle",
        "expression:Transient%3AExpression",
        "--expression",
        "let\n    Source = #table(type table [Value = Int64.Type], {{2}}),\n    Result = Source\nin\n    Result",
        "--in-place",
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);
    let updated = fs::read_to_string(&path).expect("updated expressions");
    assert!(updated.contains("{{2}}"));
    assert!(updated.contains("expression 'Transient:Expression'"));

    let duplicate = run_powerbi(&[
        "model",
        "expressions",
        "add",
        "--project",
        project_arg,
        "--name",
        "DuplicateSteps",
        "--expression",
        "let\n    Source = #table(type table [Value = Int64.Type], {{1}}),\n    Source = Source\nin\n    Source",
        "--in-place",
        "--json",
    ]);
    assert_eq!(duplicate.code, 0, "stderr: {}", duplicate.stderr);
    let lint = run_powerbi(&["lint", project_arg, "--json"]);
    let lint_json = stdout_json(&lint);
    assert!(lint_json["findings"].as_array().is_some_and(|findings| {
        findings.iter().any(|finding| {
            finding["code"] == "m.duplicate_step_name"
                && finding["handle"] == "expression:DuplicateSteps"
        })
    }));

    let delete_duplicate = run_powerbi(&[
        "model",
        "expressions",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "expression:DuplicateSteps",
        "--in-place",
        "--confirm",
        "expression:DuplicateSteps",
        "--json",
    ]);
    assert_eq!(
        delete_duplicate.code, 0,
        "stderr: {}",
        delete_duplicate.stderr
    );
    let delete = run_powerbi(&[
        "model",
        "expressions",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "expression:Transient%3AExpression",
        "--in-place",
        "--confirm",
        "expression:Transient%3AExpression",
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    assert!(
        !fs::read_to_string(&path)
            .expect("expressions after delete")
            .contains("Transient:Expression")
    );
}

#[test]
fn named_expression_unknown_metadata_refuses_without_changing_bytes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let path = expressions_path(&project);
    let original = "expression Protected = #table(type table [Value = Int64.Type], {{1}})\r\n    annotation DesktopOnly = \"keep\"\r\n\r\nexpression Other = #table(type table [Value = Int64.Type], {{2}})\r\n";
    fs::write(&path, original).expect("write annotated expressions");
    let project_arg = project.to_str().expect("project path");
    let update = run_powerbi(&[
        "model",
        "expressions",
        "update",
        "--project",
        project_arg,
        "--handle",
        "expression:Protected",
        "--expression",
        "#table(type table [Value = Int64.Type], {{3}})",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(update.code, 2);
    let error = stderr_json(&update);
    assert_eq!(error["error"]["code"], "unsupported_feature");
    assert!(
        error["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("Desktop-authored")
    );
    assert_eq!(
        fs::read(&path).expect("bytes after refusal"),
        original.as_bytes()
    );

    let list = run_powerbi(&[
        "model",
        "expressions",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let records = list_json["records"].as_array().expect("records");
    assert!(records.iter().any(|record| {
        record["handle"] == "expression:Protected" && record["lineRange"]["start"] == 1
    }));
}
