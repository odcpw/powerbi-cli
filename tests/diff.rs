mod common;

use common::assert_unsupported_feature;
use serde_json::Value;
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

#[test]
fn diff_identical_projects_returns_empty_change_set() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let out = project.to_str().expect("project path");

    let diff = run_powerbi(&["diff", out, out, "--json"]);
    assert_eq!(diff.code, 0, "stderr: {}", diff.stderr);
    let value = stdout_json(&diff);
    assert_eq!(value["schema"], Value::from("powerbi-cli.diff.v1"));
    assert_eq!(value["mode"], Value::from("semantic"));
    assert_eq!(value["scope"], Value::from("model.measures"));
    assert_eq!(value["same"], Value::Bool(true));
    assert_eq!(value["summary"]["changes"], Value::from(0));
    assert!(value.get("options").is_none());
}

#[test]
fn diff_rejects_removed_volatile_flags() {
    for flag in ["--ignore-volatile", "--include-volatile"] {
        let diff = run_powerbi(&["diff", "before", "after", flag, "--json"]);
        assert_eq!(diff.code, 2, "stderr: {}", diff.stderr);
        let value: Value = serde_json::from_str(diff.stderr.trim()).expect("stderr JSON");
        assert_eq!(value["error"]["code"], Value::from("invalid_args"));
        assert_eq!(
            value["error"]["message"],
            Value::from(format!("unknown diff flag: {flag}"))
        );
    }
}

#[test]
fn diff_rejects_unavailable_semantic_scope_as_unsupported_feature() {
    let diff = run_powerbi(&[
        "diff",
        "before",
        "after",
        "--scope",
        "model.tables",
        "--json",
    ]);
    assert_eq!(diff.code, 2, "stderr: {}", diff.stderr);
    assert_unsupported_feature(&diff.stderr, "unsupported diff scope: model.tables");
}

#[test]
fn diff_reports_measure_expression_changes_by_stable_handle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let before = scaffold_sales_project(temp.path());
    let before_arg = before.to_str().expect("before path");
    let after = temp.path().join("sales_after");
    let after_arg = after.to_str().expect("after path");

    let update = run_powerbi(&[
        "model",
        "measures",
        "update",
        "--project",
        before_arg,
        "--handle",
        "measure:FactSales:Total Revenue",
        "--expression",
        "SUMX('FactSales', 'FactSales'[Revenue])",
        "--out-dir",
        after_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);

    let diff = run_powerbi(&["diff", before_arg, after_arg, "--json"]);
    assert_eq!(diff.code, 0, "stderr: {}", diff.stderr);
    let value = stdout_json(&diff);
    assert_eq!(value["same"], Value::Bool(false));
    assert_eq!(value["summary"]["modified"], Value::from(1));
    let changes = value["changes"].as_array().expect("changes");
    assert!(changes.iter().any(|change| {
        change["kind"] == "model.measure"
            && change["op"] == "modified"
            && change["handle"] == "measure:FactSales:Total Revenue"
            && change["fieldsChanged"] == serde_json::json!(["expression"])
            && change["before"]["expression"] == "SUM('FactSales'[Revenue])"
            && change["after"]["expression"] == "SUMX('FactSales', 'FactSales'[Revenue])"
    }));
}

#[test]
fn diff_reports_added_and_removed_measures() {
    let temp = tempfile::tempdir().expect("tempdir");
    let before = scaffold_sales_project(temp.path());
    let before_arg = before.to_str().expect("before path");
    let after = temp.path().join("sales_after");
    let after_arg = after.to_str().expect("after path");

    let add = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        before_arg,
        "--table",
        "FactSales",
        "--name",
        "Average Revenue",
        "--expression",
        "DIVIDE([Total Revenue], [Total Units])",
        "--format-string",
        "$#,0.00",
        "--out-dir",
        after_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let added = run_powerbi(&["diff", before_arg, after_arg, "--json"]);
    assert_eq!(added.code, 0, "stderr: {}", added.stderr);
    let added_json = stdout_json(&added);
    assert_eq!(added_json["summary"]["added"], Value::from(1));
    assert_eq!(
        added_json["changes"][0]["handle"],
        Value::from("measure:FactSales:Average Revenue")
    );
    assert_eq!(added_json["changes"][0]["op"], Value::from("added"));
    assert_eq!(
        added_json["changes"][0]["after"]["properties"]["formatString"],
        Value::from("$#,0.00")
    );

    let removed = run_powerbi(&["diff", after_arg, before_arg, "--json"]);
    assert_eq!(removed.code, 0, "stderr: {}", removed.stderr);
    let removed_json = stdout_json(&removed);
    assert_eq!(removed_json["summary"]["removed"], Value::from(1));
    assert_eq!(
        removed_json["changes"][0]["handle"],
        Value::from("measure:FactSales:Average Revenue")
    );
    assert_eq!(removed_json["changes"][0]["op"], Value::from("removed"));
}

#[test]
fn diff_output_is_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let before = scaffold_sales_project(temp.path());
    let before_arg = before.to_str().expect("before path");
    let after = temp.path().join("sales_after");
    let after_arg = after.to_str().expect("after path");

    let add = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        before_arg,
        "--table",
        "FactSales",
        "--name",
        "Average Revenue",
        "--expression",
        "DIVIDE([Total Revenue], [Total Units])",
        "--out-dir",
        after_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let first = run_powerbi(&["diff", before_arg, after_arg, "--json"]);
    let second = run_powerbi(&["diff", before_arg, after_arg, "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn diff_reports_calculated_column_changes_by_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let before = scaffold_sales_project(temp.path());
    let before_arg = before.to_str().expect("before path");
    let after = temp.path().join("sales_after");
    let after_arg = after.to_str().expect("after path");

    let add = run_powerbi(&[
        "model",
        "calculated-columns",
        "add",
        "--project",
        before_arg,
        "--table",
        "FactSales",
        "--name",
        "Revenue Band",
        "--expression",
        "IF('FactSales'[Revenue] >= 10000, \"High\", \"Standard\")",
        "--data-type",
        "string",
        "--out-dir",
        after_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let added = run_powerbi(&[
        "diff",
        before_arg,
        after_arg,
        "--scope",
        "model.calculatedColumns",
        "--json",
    ]);
    assert_eq!(added.code, 0, "stderr: {}", added.stderr);
    let added_json = stdout_json(&added);
    assert_eq!(added_json["scope"], Value::from("model.calculatedColumns"));
    assert_eq!(added_json["summary"]["added"], Value::from(1));
    assert_eq!(
        added_json["changes"][0]["kind"],
        Value::from("model.calculatedColumn")
    );
    assert_eq!(
        added_json["changes"][0]["handle"],
        Value::from("column:FactSales:Revenue Band")
    );
    assert!(
        added_json["next"][0]
            .as_str()
            .unwrap_or_default()
            .contains("model calculated-columns show")
    );

    let updated = temp.path().join("sales_updated");
    let updated_arg = updated.to_str().expect("updated path");
    let update = run_powerbi(&[
        "model",
        "calculated-columns",
        "update",
        "--project",
        after_arg,
        "--handle",
        "column:FactSales:Revenue Band",
        "--expression",
        "IF('FactSales'[Revenue] >= 5000, \"High\", \"Standard\")",
        "--out-dir",
        updated_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);

    let modified = run_powerbi(&[
        "diff",
        after_arg,
        updated_arg,
        "--scope",
        "model.calculatedColumns",
        "--json",
    ]);
    assert_eq!(modified.code, 0, "stderr: {}", modified.stderr);
    let modified_json = stdout_json(&modified);
    assert_eq!(modified_json["summary"]["modified"], Value::from(1));
    assert_eq!(
        modified_json["changes"][0]["fieldsChanged"],
        serde_json::json!(["expression"])
    );

    let removed = run_powerbi(&[
        "diff",
        after_arg,
        before_arg,
        "--scope",
        "model.calculatedColumns",
        "--json",
    ]);
    assert_eq!(removed.code, 0, "stderr: {}", removed.stderr);
    let removed_json = stdout_json(&removed);
    assert_eq!(removed_json["summary"]["removed"], Value::from(1));
    assert_eq!(removed_json["changes"][0]["op"], Value::from("removed"));
}

#[test]
fn diff_reports_relationship_changes_by_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let before = scaffold_sales_project(temp.path());
    let before_arg = before.to_str().expect("before path");

    let list = run_powerbi(&[
        "model",
        "relationships",
        "list",
        "--project",
        before_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let handle = list_json["relationships"][0]["handle"]
        .as_str()
        .expect("relationship handle");

    let after = temp.path().join("sales_after");
    let after_arg = after.to_str().expect("after path");
    let update = run_powerbi(&[
        "model",
        "relationships",
        "update",
        "--project",
        before_arg,
        "--handle",
        handle,
        "--cross-filtering-behavior",
        "bothDirections",
        "--inactive",
        "--out-dir",
        after_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);

    let diff = run_powerbi(&[
        "diff",
        before_arg,
        after_arg,
        "--scope",
        "model.relationships",
        "--json",
    ]);
    assert_eq!(diff.code, 0, "stderr: {}", diff.stderr);
    let diff_json = stdout_json(&diff);
    assert_eq!(diff_json["scope"], Value::from("model.relationships"));
    assert_eq!(diff_json["summary"]["modified"], Value::from(1));
    assert_eq!(
        diff_json["changes"][0]["kind"],
        Value::from("model.relationship")
    );
    assert_eq!(diff_json["changes"][0]["handle"], Value::from(handle));
    assert_eq!(
        diff_json["changes"][0]["fieldsChanged"],
        serde_json::json!(["properties.crossFilteringBehavior", "properties.isActive"])
    );
    assert!(
        diff_json["next"][0]
            .as_str()
            .unwrap_or_default()
            .contains("model relationships show")
    );
}
