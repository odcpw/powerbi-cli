mod common;

use common::{run_powerbi, run_powerbi_owned, scaffold_sales, stdout_json};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn run_for_project(command: &[&str], project: &Path) -> common::RunOutput {
    let mut args = command
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    args.extend([
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    run_powerbi_owned(&args)
}

fn lint(project: &Path) -> Value {
    let output = run_powerbi(&["lint", project.to_str().expect("project path"), "--json"]);
    assert_eq!(output.code, 0, "lint stderr: {}", output.stderr);
    stdout_json(&output)
}

fn dax_lint(project: &Path) -> Value {
    let output = run_powerbi(&[
        "model",
        "dax",
        "lint",
        "--project",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "DAX lint stderr: {}", output.stderr);
    stdout_json(&output)
}

fn finding_codes(value: &Value) -> BTreeSet<String> {
    value["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter_map(|finding| finding["code"].as_str())
        .map(ToOwned::to_owned)
        .collect()
}

fn findings_for<'a>(value: &'a Value, code: &str) -> Vec<&'a Value> {
    value["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|finding| finding["code"] == code)
        .collect()
}

fn table_path(project: &Path, table: &str) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join(format!("{table}.tmdl"))
}

fn relationships_path(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("relationships.tmdl")
}

fn add_measure(project: &Path, name: &str, format_string: Option<&str>) {
    let mut args = vec![
        "model".to_string(),
        "measures".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--table".to_string(),
        "FactSales".to_string(),
        "--name".to_string(),
        name.to_string(),
        "--expression".to_string(),
        "SUM('FactSales'[Revenue])".to_string(),
        "--in-place".to_string(),
        "--json".to_string(),
    ];
    if let Some(format_string) = format_string {
        args.splice(
            args.len() - 2..args.len() - 2,
            ["--format-string".to_string(), format_string.to_string()],
        );
    }
    let output = run_powerbi_owned(&args);
    assert_eq!(output.code, 0, "measure add stderr: {}", output.stderr);
}

fn hide_relationship_keys(project: &Path) {
    for table in ["DimDate", "DimCustomer"] {
        let path = table_path(project, table);
        let text = fs::read_to_string(&path).expect("read key table");
        assert!(text.contains("        isKey\n"), "key fixture in {path:?}");
        fs::write(
            path,
            text.replace("        isKey\n", "        isKey\n        isHidden\n"),
        )
        .expect("hide relationship key");
    }
}

fn set_relationship_direction(project: &Path, behavior: &str) {
    let path = relationships_path(project);
    let text = fs::read_to_string(&path).expect("read relationships");
    assert!(text.contains("crossFilteringBehavior: oneDirection"));
    fs::write(
        path,
        text.replace(
            "crossFilteringBehavior: oneDirection",
            &format!("crossFilteringBehavior: {behavior}"),
        ),
    )
    .expect("write relationship direction fixture");
}

#[test]
fn dax_format_rules_flag_missing_and_malformed_static_formats_but_not_valid_formats() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());

    let baseline = dax_lint(&project);
    assert!(!finding_codes(&baseline).contains("dax.format_missing"));
    assert!(!finding_codes(&baseline).contains("dax.format_invalid"));

    add_measure(&project, "Unformatted", None);
    let missing = dax_lint(&project);
    let missing_finding = findings_for(&missing, "dax.format_missing")
        .into_iter()
        .find(|finding| finding["handle"] == "measure:FactSales:Unformatted")
        .expect("missing format finding");
    assert_eq!(missing_finding["severity"], "warning");
    assert!(
        missing_finding["message"]
            .as_str()
            .unwrap_or_default()
            .contains("no static")
    );
    assert!(
        missing_finding["hint"]
            .as_str()
            .is_some_and(|hint| !hint.is_empty())
    );
    assert!(missing_finding["path"].as_str().is_some());
    let combined_missing = lint(&project);
    assert!(
        findings_for(&combined_missing, "dax.format_missing")
            .iter()
            .any(|finding| finding["handle"] == "measure:FactSales:Unformatted")
    );

    add_measure(&project, "Malformed", Some("0.0["));
    let malformed = dax_lint(&project);
    let invalid_finding = findings_for(&malformed, "dax.format_invalid")
        .into_iter()
        .find(|finding| finding["handle"] == "measure:FactSales:Malformed")
        .expect("invalid format finding");
    assert_eq!(invalid_finding["severity"], "warning");
    assert!(
        invalid_finding["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unclosed")
    );
    assert!(
        invalid_finding["hint"]
            .as_str()
            .is_some_and(|hint| !hint.is_empty())
    );
    assert!(
        !findings_for(&malformed, "dax.format_invalid")
            .iter()
            .any(|finding| finding["handle"] == "measure:FactSales:Total Revenue")
    );
}

#[test]
fn model_key_visibility_rule_flags_visible_relationship_keys_and_accepts_hidden_keys() {
    let temp = tempfile::tempdir().expect("tempdir");
    let visible_root = temp.path().join("visible");
    fs::create_dir_all(&visible_root).expect("visible root");
    let visible_project = scaffold_sales(&visible_root);
    let visible = lint(&visible_project);
    let visible_findings = findings_for(&visible, "model.key_not_hidden");
    assert_eq!(visible_findings.len(), 2);
    assert!(visible_findings.iter().all(|finding| {
        finding["severity"] == "warning"
            && finding["handle"]
                .as_str()
                .is_some_and(|handle| handle.starts_with("column:"))
            && finding["hint"]
                .as_str()
                .is_some_and(|hint| !hint.is_empty())
    }));

    let hidden_root = temp.path().join("hidden");
    fs::create_dir_all(&hidden_root).expect("hidden root");
    let hidden_project = scaffold_sales(&hidden_root);
    hide_relationship_keys(&hidden_project);
    let hidden = lint(&hidden_project);
    assert!(findings_for(&hidden, "model.key_not_hidden").is_empty());
}

#[test]
fn relationship_direction_rule_flags_both_directions_only_for_fact_to_dimension() {
    let temp = tempfile::tempdir().expect("tempdir");
    let one_root = temp.path().join("one");
    fs::create_dir_all(&one_root).expect("one root");
    let one_direction_project = scaffold_sales(&one_root);
    let one_direction = lint(&one_direction_project);
    assert!(findings_for(&one_direction, "model.relationship_direction_suspect").is_empty());

    let both_root = temp.path().join("both");
    fs::create_dir_all(&both_root).expect("both root");
    let both_direction_project = scaffold_sales(&both_root);
    set_relationship_direction(&both_direction_project, "bothDirections");
    let both_direction = lint(&both_direction_project);
    let findings = findings_for(&both_direction, "model.relationship_direction_suspect");
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|finding| {
        finding["severity"] == "warning"
            && finding["handle"]
                .as_str()
                .is_some_and(|handle| handle.starts_with("relationship:"))
            && finding["message"]
                .as_str()
                .is_some_and(|message| message.contains("both directions"))
            && finding["hint"]
                .as_str()
                .is_some_and(|hint| !hint.is_empty())
    }));
}

#[test]
fn unused_column_rule_tracks_visual_measure_and_relationship_references() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let add = run_for_project(
        &[
            "model",
            "calculated-columns",
            "add",
            "--table",
            "FactSales",
            "--name",
            "UnusedProbe",
            "--expression",
            "1",
            "--data-type",
            "int64",
            "--in-place",
        ],
        &project,
    );
    assert_eq!(add.code, 0, "calculated column add stderr: {}", add.stderr);

    let value = lint(&project);
    let unused = findings_for(&value, "model.column_unused");
    let probe = unused
        .iter()
        .find(|finding| finding["handle"] == "column:FactSales:UnusedProbe")
        .expect("planted unused column finding");
    assert_eq!(probe["severity"], "warning");
    assert!(
        probe["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not referenced")
    );
    assert!(probe["hint"].as_str().is_some_and(|hint| !hint.is_empty()));
    assert!(
        !unused
            .iter()
            .any(|finding| finding["handle"] == "column:FactSales:Revenue")
    );
    assert!(
        !unused
            .iter()
            .any(|finding| finding["handle"] == "column:FactSales:Units")
    );
    assert!(
        !unused
            .iter()
            .any(|finding| finding["handle"] == "column:DimCustomer:CustomerName")
    );
    assert!(
        !unused
            .iter()
            .any(|finding| finding["handle"] == "column:DimDate:FiscalYear")
    );
}

#[test]
fn completeness_findings_flow_through_triage_and_fixture_scorecard_deterministically() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let first = run_powerbi(&["triage", project_arg, "--json"]);
    assert_eq!(first.code, 0, "triage stderr: {}", first.stderr);
    let second = run_powerbi(&["triage", project_arg, "--json"]);
    assert_eq!(second.code, 0, "triage stderr: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout, "triage must be deterministic");
    let triage = stdout_json(&first);
    let triage_codes = finding_codes(&triage["lint"]);
    assert!(triage_codes.contains("model.key_not_hidden"));
    assert!(triage_codes.contains("model.column_unused"));
    assert!(triage["topFindings"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|finding| finding["code"] == "model.column_unused")
    }));

    let first_fixture = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(
        first_fixture.code, 0,
        "fixture stderr: {}",
        first_fixture.stderr
    );
    let second_fixture = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(
        second_fixture.code, 0,
        "fixture stderr: {}",
        second_fixture.stderr
    );
    assert_eq!(
        first_fixture.stdout, second_fixture.stdout,
        "fixture scorecard must be deterministic"
    );
    let fixture = stdout_json(&first_fixture);
    let scorecard_codes = fixture["lint"]["findings"]
        .as_array()
        .expect("fixture lint findings")
        .iter()
        .filter_map(|finding| finding["code"].as_str())
        .collect::<BTreeSet<_>>();
    assert!(scorecard_codes.contains("model.key_not_hidden"));
    assert!(scorecard_codes.contains("model.column_unused"));
}
