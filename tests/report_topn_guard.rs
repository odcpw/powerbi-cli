//! Visual TopN guard create/update integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn first_visual_target(project: &Path) -> (String, std::path::PathBuf) {
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let visual = &stdout_json(&output)["visuals"][0];
    let handle = visual["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let path = visual["path"].as_str().expect("visual path").to_string();
    (handle, std::path::PathBuf::from(path))
}

fn expected_guard(name: &str, display_name: &str, top: u64, direction: u64) -> Value {
    json!({
        "name": name,
        "type": "TopN",
        "field": {
            "Column": {
                "Expression": { "SourceRef": { "Entity": "DimCustomer" } },
                "Property": "CustomerName"
            }
        },
        "filter": {
            "Version": 2,
            "From": [
                {
                    "Name": "topn",
                    "Expression": {
                        "Subquery": {
                            "Query": {
                                "Version": 2,
                                "From": [
                                    { "Name": "t", "Entity": "DimCustomer", "Type": 0 },
                                    { "Name": "m", "Entity": "FactSales", "Type": 0 }
                                ],
                                "Select": [{
                                    "Column": {
                                        "Expression": { "SourceRef": { "Source": "t" } },
                                        "Property": "CustomerName"
                                    },
                                    "Name": "field"
                                }],
                                "OrderBy": [{
                                    "Direction": direction,
                                    "Expression": {
                                        "Measure": {
                                            "Expression": { "SourceRef": { "Source": "m" } },
                                            "Property": "Total Revenue"
                                        }
                                    }
                                }],
                                "Top": top
                            }
                        }
                    },
                    "Type": 2
                },
                { "Name": "d", "Entity": "DimCustomer", "Type": 0 }
            ],
            "Where": [{
                "Condition": {
                    "In": {
                        "Expressions": [{
                            "Column": {
                                "Expression": { "SourceRef": { "Source": "d" } },
                                "Property": "CustomerName"
                            }
                        }],
                        "Table": { "SourceRef": { "Source": "topn" } }
                    }
                }
            }]
        },
        "howCreated": "User",
        "displayName": display_name
    })
}

fn read_visual_filters(path: &Path) -> Vec<Value> {
    let visual: Value = serde_json::from_str(&fs::read_to_string(path).expect("visual json"))
        .expect("parse visual");
    visual
        .pointer("/filterConfig/filters")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn set_guard_args(
    project: &str,
    handle: &str,
    top: &str,
    extra: &[&str],
    mode: &[&str],
) -> Vec<String> {
    let mut args = vec![
        "report".to_string(),
        "visuals".to_string(),
        "set-topn-guard".to_string(),
        "--project".to_string(),
        project.to_string(),
        "--handle".to_string(),
        handle.to_string(),
        "--field".to_string(),
        "DimCustomer.CustomerName".to_string(),
        "--order-by".to_string(),
        "FactSales.Total Revenue".to_string(),
        "--top".to_string(),
        top.to_string(),
    ];
    args.extend(extra.iter().map(|arg| (*arg).to_string()));
    args.extend(mode.iter().map(|arg| (*arg).to_string()));
    args.push("--json".to_string());
    args
}

#[test]
fn set_topn_guard_creates_canonical_filter_on_empty_visual() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (handle, visual_path) = first_visual_target(&project);
    let visual_before: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("visual json"))
            .expect("parse visual");
    assert!(
        visual_before.get("filterConfig").is_none(),
        "scaffold visual should start without filterConfig: {visual_before}"
    );

    let out_dir = temp.path().join("guarded");
    let output = run_powerbi_owned(&set_guard_args(
        project.to_str().expect("project path"),
        &handle,
        "251",
        &["--name", "TopNGuard", "--display-name", "Top 251"],
        &["--out-dir", out_dir.to_str().expect("out dir")],
    ));
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["action"], Value::from("create"));
    assert_eq!(value["changes"][0]["before"], Value::Null);
    assert_eq!(value["changes"][0]["after"]["top"], Value::from(251));
    assert_eq!(
        value["changes"][0]["after"]["orderBy"],
        Value::from("FactSales[Total Revenue]")
    );
    assert_eq!(value["validation"]["ok"], Value::Bool(true));
    assert_strict_valid(&out_dir);

    let written = Path::new(value["changes"][0]["path"].as_str().expect("written path"));
    let filters = read_visual_filters(written);
    assert_eq!(filters.len(), 1);
    assert_eq!(
        filters[0],
        expected_guard("TopNGuard", "Top 251", 251, 2),
        "written TopN guard must match the existing filter writer shape"
    );
    assert!(filters[0]["name"].as_str().expect("name").len() <= 50);
}

#[test]
fn set_topn_guard_updates_existing_guard_top_and_keeps_name() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (handle, _) = first_visual_target(&project);
    let project_arg = project.to_str().expect("project path");
    let created = temp.path().join("created");
    let create = run_powerbi_owned(&set_guard_args(
        project_arg,
        &handle,
        "28",
        &["--name", "TopNGuard"],
        &["--out-dir", created.to_str().expect("created")],
    ));
    assert_eq!(create.code, 0, "create stderr: {}", create.stderr);
    let created_json = stdout_json(&create);
    assert_eq!(created_json["action"], Value::from("create"));
    assert_eq!(created_json["changes"][0]["after"]["top"], Value::from(28));

    let update = run_powerbi_owned(&set_guard_args(
        created.to_str().expect("created path"),
        &handle,
        "251",
        &[],
        &["--in-place"],
    ));
    assert_eq!(update.code, 0, "update stderr: {}", update.stderr);
    let update_json = stdout_json(&update);
    assert_eq!(update_json["action"], Value::from("update"));
    assert_eq!(update_json["guard"]["name"], Value::from("TopNGuard"));
    assert_eq!(update_json["changes"][0]["before"]["top"], Value::from(28));
    assert_eq!(update_json["changes"][0]["after"]["top"], Value::from(251));
    assert_eq!(
        update_json["changes"][0]["before"]["orderBy"],
        Value::from("FactSales[Total Revenue]")
    );
    assert_eq!(
        update_json["changes"][0]["after"]["orderBy"],
        Value::from("FactSales[Total Revenue]")
    );

    let written = Path::new(
        update_json["changes"][0]["path"]
            .as_str()
            .expect("written path"),
    );
    let filters = read_visual_filters(written);
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0]["name"], Value::from("TopNGuard"));
    assert_eq!(
        filters[0]["field"]["Column"]["Property"],
        Value::from("CustomerName")
    );
    assert_eq!(
        filters[0]["filter"]["From"][0]["Expression"]["Subquery"]["Query"]["Top"],
        Value::from(251)
    );
    assert_eq!(filters[0]["displayName"], Value::from("Top 251"));
}

#[test]
fn set_topn_guard_writes_ascending_direction() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (handle, _) = first_visual_target(&project);
    let out_dir = temp.path().join("asc");
    let output = run_powerbi_owned(&set_guard_args(
        project.to_str().expect("project path"),
        &handle,
        "5",
        &[
            "--name",
            "BottomGuard",
            "--direction",
            "asc",
            "--display-name",
            "Bottom 5",
        ],
        &["--out-dir", out_dir.to_str().expect("out dir")],
    ));
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert_eq!(
        stdout_json(&output)["changes"][0]["after"]["direction"],
        Value::from("asc")
    );
    let output_json = stdout_json(&output);
    let written = Path::new(
        output_json["changes"][0]["path"]
            .as_str()
            .expect("written path"),
    );
    let filters = read_visual_filters(written);
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0], expected_guard("BottomGuard", "Bottom 5", 5, 1));
    assert_eq!(
        filters[0]["filter"]["From"][0]["Expression"]["Subquery"]["Query"]["OrderBy"][0]["Direction"],
        Value::from(1)
    );
}

#[test]
fn set_topn_guard_refuses_name_over_50_chars() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (handle, _) = first_visual_target(&project);
    let long_name = "T".repeat(51);
    let output = run_powerbi_owned(&set_guard_args(
        project.to_str().expect("project path"),
        &handle,
        "10",
        &["--name", &long_name],
        &["--dry-run"],
    ));
    assert_eq!(output.code, 2, "stderr: {}", output.stderr);
    assert!(
        output.stdout.trim().is_empty(),
        "stdout must stay empty on invalid_args: {}",
        output.stdout
    );
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], Value::from("invalid_args"));
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("50 characters or fewer"),
        "unexpected error: {error}"
    );
}

#[test]
fn set_topn_guard_coexists_with_unrelated_categorical_filter() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (handle, _) = first_visual_target(&project);
    let project_arg = project.to_str().expect("project path");
    let with_categorical = temp.path().join("categorical");
    let add = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--visual",
        &handle,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--name",
        "SegmentCat",
        "--out-dir",
        with_categorical.to_str().expect("categorical"),
        "--json",
    ]);
    assert_eq!(add.code, 0, "add categorical stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    let written = Path::new(
        add_json["changes"][0]["path"]
            .as_str()
            .expect("categorical path"),
    );
    let before = read_visual_filters(written);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0]["type"], Value::from("Categorical"));
    assert_eq!(before[0]["name"], Value::from("SegmentCat"));
    let first_before = before[0].clone();

    let guard = run_powerbi_owned(&set_guard_args(
        with_categorical.to_str().expect("categorical path"),
        &handle,
        "28",
        &["--name", "TopNGuard"],
        &["--in-place"],
    ));
    assert_eq!(guard.code, 0, "guard stderr: {}", guard.stderr);
    let after = read_visual_filters(written);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0], first_before);
    assert_eq!(after[1]["type"], Value::from("TopN"));
    assert_eq!(after[1]["name"], Value::from("TopNGuard"));
}
