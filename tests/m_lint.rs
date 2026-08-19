mod common;

use common::{run_powerbi, stdout_json};
use std::fs;
use std::path::{Path, PathBuf};

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

fn set_column_data_type(project: &Path, column: &str, data_type: &str) {
    let path = fact_sales_tmdl(project);
    let text = fs::read_to_string(&path).expect("FactSales TMDL");
    let marker = format!("    column {column}\n        dataType: ");
    let start = text.find(&marker).expect("column dataType");
    let type_start = start + marker.len();
    let type_end = type_start + text[type_start..].find('\n').expect("dataType line end");
    let mut updated = String::new();
    updated.push_str(&text[..type_start]);
    updated.push_str(data_type);
    updated.push_str(&text[type_end..]);
    fs::write(&path, updated).expect("patch column dataType");
}

fn expand_source(retype: bool) -> String {
    let typed = if retype {
        ",\n                Typed = Table.TransformColumnTypes(Expanded, {{\"Revenue\", type number}})"
    } else {
        ""
    };
    let result = if retype { "Typed" } else { "Expanded" };
    format!(
        r#"        source =
            let
                Source = #table(type table [DateKey = Int64.Type, CustomerKey = Int64.Type, Revenue = Currency.Type, Units = Int64.Type], {{}}),
                Grouped = Table.Group(Source, {{"DateKey", "CustomerKey", "Units"}}, {{{{"Rows", each _, type table}}}}),
                Expanded = Table.ExpandTableColumn(Grouped, "Rows", {{"Revenue"}}){typed}
            in
                {result}"#
    )
}

fn untyped_expansion_findings(project: &Path) -> Vec<serde_json::Value> {
    let lint = run_powerbi(&["lint", project.to_str().expect("project path"), "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    stdout_json(&lint)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|finding| finding["code"] == "m.untyped_expansion")
        .cloned()
        .collect()
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
    assert_eq!(finding["stepKind"], "tableLiteral");
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
    assert_eq!(finding["stepKind"], "tableLiteral");
    assert_eq!(finding["referenceCount"], 2);
}

#[test]
fn lint_suppresses_function_and_scalar_reuse_but_flags_table_steps() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    replace_partition_source(
        &project,
        r#"        source =
            let
                Normalize = (value) => value,
                Scale = 1.5,
                Shared = #table(type table [DateKey = Int64.Type, CustomerKey = Int64.Type, Revenue = Currency.Type, Units = Int64.Type], {}),
                LeftFn = Normalize,
                RightFn = Normalize,
                LeftScale = Scale,
                RightScale = Scale,
                Left = Table.SelectRows(Shared, each [Units] >= 0),
                Right = Table.SelectRows(Shared, each [Units] < 0),
                Result = Table.Combine({Left, Right})
            in
                Result"#,
    );

    let lint = run_powerbi(&["lint", project.to_str().expect("project path"), "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    let findings = stdout_json(&lint)["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|finding| finding["code"] == "m.unbuffered_reuse")
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(findings.len(), 1, "expected only table reuse: {findings:?}");
    assert_eq!(findings[0]["step"], "Shared");
    assert_eq!(findings[0]["stepKind"], "tableLiteral");
}

#[test]
fn lint_warns_for_untyped_numeric_expansion_without_failing_strict_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    set_column_data_type(&project, "Revenue", "double");
    replace_partition_source(&project, &expand_source(false));
    let project_arg = project.to_str().expect("project path");

    let findings = untyped_expansion_findings(&project);
    assert_eq!(
        findings.len(),
        1,
        "expected one untyped expansion: {findings:?}"
    );
    let finding = &findings[0];
    assert_eq!(finding["severity"], "warning");
    assert_eq!(finding["analysisBoundary"], "heuristic");
    assert_eq!(finding["documentKind"], "partition");
    assert_eq!(finding["step"], "Expanded");
    assert_eq!(finding["stepKind"], "other");
    assert_eq!(finding["column"], "Revenue");
    assert_eq!(finding["handle"], "partition:FactSales:FactSales");
    assert!(
        finding["message"]
            .as_str()
            .expect("message")
            .contains("Table.TransformColumnTypes")
    );

    let strict = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(strict.code, 0, "stderr: {}", strict.stderr);
    assert_eq!(stdout_json(&strict)["ok"], true);
}

#[test]
fn lint_does_not_flag_expanded_column_after_transform_column_types() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    set_column_data_type(&project, "Revenue", "double");
    replace_partition_source(&project, &expand_source(true));

    assert!(untyped_expansion_findings(&project).is_empty());
    let project_arg = project.to_str().expect("project path");
    let strict = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(strict.code, 0, "stderr: {}", strict.stderr);
    assert_eq!(stdout_json(&strict)["ok"], true);
}

#[test]
fn lint_does_not_flag_untyped_expansion_mapped_to_string_column() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    set_column_data_type(&project, "Revenue", "string");
    replace_partition_source(&project, &expand_source(false));

    assert!(untyped_expansion_findings(&project).is_empty());
    let project_arg = project.to_str().expect("project path");
    let strict = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(strict.code, 0, "stderr: {}", strict.stderr);
    assert_eq!(stdout_json(&strict)["ok"], true);
}
