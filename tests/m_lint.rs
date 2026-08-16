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

fn fact_sales_tmdl(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("FactSales.tmdl")
}

fn replace_partition_source(project: &Path, source: &str) {
    let path = fact_sales_tmdl(project);
    let text = fs::read_to_string(&path).expect("FactSales TMDL");
    let source_start = text.find("        source =").expect("source block");
    fs::write(&path, format!("{}{source}\n\n", &text[..source_start]))
        .expect("replace partition source");
}

fn m_source(buffered: bool) -> String {
    let shared = if buffered { "Buffered" } else { "Source" };
    let buffer_step = if buffered {
        "                Buffered = Table.Buffer(Source),\n"
    } else {
        ""
    };
    format!(
        r#"        source =
            let
                Source = #table(type table [DateKey = Int64.Type, CustomerKey = Int64.Type, Revenue = Currency.Type, Units = Int64.Type], {{}}),
{buffer_step}                Left = Table.SelectRows({shared}, each [Units] >= 0),
                Right = Table.SelectRows({shared}, each [Units] < 0),
                Result = Table.Combine({{Left, Right}})
            in
                Result"#
    )
}

#[test]
fn lint_warns_for_unbuffered_reused_m_step_without_failing_strict_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    replace_partition_source(&project, &m_source(false));
    let project_arg = project.to_str().expect("project path");

    let lint = run_powerbi(&["lint", project_arg, "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    let lint_json = stdout_json(&lint);
    let finding = lint_json["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|finding| finding["code"] == "m.unbuffered_reuse")
        .unwrap_or_else(|| panic!("M buffer warning missing: {lint_json}"));
    assert_eq!(finding["severity"], "warning");
    assert_eq!(finding["step"], "Source");
    assert_eq!(finding["referenceCount"], 2);
    assert!(
        finding["message"]
            .as_str()
            .expect("message")
            .contains("Source")
    );

    let strict = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(strict.code, 0, "stderr: {}", strict.stderr);
    assert_eq!(stdout_json(&strict)["ok"], true);
}

#[test]
fn lint_does_not_flag_reuse_of_a_table_buffer_step() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    replace_partition_source(&project, &m_source(true));

    let lint = run_powerbi(&["lint", project.to_str().expect("project path"), "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    assert!(
        stdout_json(&lint)["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .all(|finding| finding["code"] != "m.unbuffered_reuse")
    );
}

#[test]
fn lint_analyzes_named_m_expression_documents() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let expressions = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("expressions.tmdl");
    fs::write(
        expressions,
        "expression SharedQuery =\n    let\n        Source = #table(type table [Value = Int64.Type], {{1}}),\n        Left = Table.FirstN(Source, 1),\n        Right = Table.LastN(Source, 1)\n    in\n        Left\n",
    )
    .expect("named M expression");

    let lint = run_powerbi(&["lint", project.to_str().expect("project path"), "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    let finding = stdout_json(&lint)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|finding| {
            finding["code"] == "m.unbuffered_reuse" && finding["handle"] == "expression:SharedQuery"
        })
        .cloned()
        .expect("named-expression buffer warning");
    assert_eq!(finding["documentKind"], "expression");
    assert_eq!(finding["step"], "Source");
    assert_eq!(finding["referenceCount"], 2);
}
