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

fn scaffold_sales_project(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out_dir.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

fn dim_date_tmdl(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("DimDate.tmdl")
}

#[test]
fn model_columns_set_sort_by_dry_run_set_and_clear_are_byte_preserving() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let table_path = dim_date_tmdl(&project);
    let original = fs::read_to_string(&table_path).expect("DimDate TMDL");

    let dry_run = run_powerbi(&[
        "model",
        "columns",
        "set-sort-by",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--column",
        "Month",
        "--by",
        "DateKey",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(dry_json["target"]["sortByColumn"], "DateKey");
    assert!(
        dry_json["changes"][0]["after"]
            .as_str()
            .expect("after block")
            .contains("sortByColumn: DateKey")
    );
    assert_eq!(
        fs::read_to_string(&table_path).expect("TMDL after dry-run"),
        original
    );

    let set = run_powerbi(&[
        "model",
        "columns",
        "set-sort-by",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--column",
        "Month",
        "--by",
        "DateKey",
        "--in-place",
        "--json",
    ]);
    assert_eq!(set.code, 0, "stderr: {}", set.stderr);
    let set_text = fs::read_to_string(&table_path).expect("TMDL after set");
    assert!(set_text.contains("        sortByColumn: DateKey"));

    let inspect = run_powerbi(&["inspect", "--deep", project_arg, "--json"]);
    assert_eq!(inspect.code, 0, "stderr: {}", inspect.stderr);
    let inspect_json = stdout_json(&inspect);
    let month = inspect_json["deep"]["model"]["tables"]
        .as_array()
        .expect("tables")
        .iter()
        .find(|table| table["name"] == "DimDate")
        .and_then(|table| table["columns"].as_array())
        .and_then(|columns| columns.iter().find(|column| column["name"] == "Month"))
        .expect("Month column");
    assert_eq!(month["properties"]["sortByColumn"], "DateKey");

    let clear = run_powerbi(&[
        "model",
        "columns",
        "set-sort-by",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--column",
        "Month",
        "--clear",
        "--in-place",
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    assert_eq!(
        fs::read_to_string(&table_path).expect("TMDL after clear"),
        original
    );
}

#[test]
fn model_columns_set_sort_by_checks_same_table_columns_and_rejects_self_sort() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    for by in ["Missing", "Month"] {
        let output = run_powerbi(&[
            "model",
            "columns",
            "set-sort-by",
            "--project",
            project_arg,
            "--table",
            "DimDate",
            "--column",
            "Month",
            "--by",
            by,
            "--dry-run",
            "--json",
        ]);
        assert_ne!(output.code, 0, "{by} unexpectedly succeeded");
        assert!(
            output.stderr.contains(if by == "Missing" {
                "column not found"
            } else {
                "cannot sort by itself"
            }),
            "stderr: {}",
            output.stderr
        );
    }
}

#[test]
fn model_columns_set_sort_by_is_advertised() {
    let output = run_powerbi(&["capabilities", "--for", "model columns", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(
        stdout_json(&output)["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .any(|command| command["path"] == "model columns set-sort-by")
    );
}
