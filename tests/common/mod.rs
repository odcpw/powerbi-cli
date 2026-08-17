//! Shared test harness helpers. Each test binary includes this module via
//! `mod common;`, so helpers unused by a given binary are expected dead code.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct RunOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn run_powerbi(args: &[&str]) -> RunOutput {
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

pub fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

pub fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

pub fn assert_unsupported_feature(stderr: &str, message_fragment: &str) -> Value {
    let value: Value = serde_json::from_str(stderr.trim()).expect("stderr JSON");
    assert_eq!(value["error"]["code"], Value::from("unsupported_feature"));
    assert_eq!(value["error"]["exitCode"], Value::from(2));
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains(message_fragment),
        "expected error message to contain {message_fragment:?}: {value}"
    );
    value
}

pub fn run_powerbi_owned(args: &[String]) -> RunOutput {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_powerbi(&args)
}

pub fn scaffold_sales(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

pub fn build_scatter_bubble(root: &Path) -> PathBuf {
    let out_dir = root.join("scatter_bubble_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/archetypes/scatter-bubble.schema.json",
        "--profile",
        "examples/archetypes/scatter-bubble.profile.json",
        "--spec",
        "examples/archetypes/scatter-bubble.dashboard.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

pub fn report_pages_json(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join("pages.json")
}

pub fn report_json(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("report.json")
}

pub fn first_page_json(project: &Path) -> PathBuf {
    let pages_json: Value =
        serde_json::from_str(&fs::read_to_string(report_pages_json(project)).expect("pages json"))
            .expect("parse pages json");
    let page_name = pages_json["pageOrder"][0]
        .as_str()
        .expect("first page name");
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join(page_name)
        .join("page.json")
}

pub fn first_page_name(project: &Path) -> String {
    first_page_json(project)
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .expect("first page name")
        .to_string()
}

pub fn first_visual_json(project: &Path) -> PathBuf {
    let page_json = first_page_json(project);
    let visuals_dir = page_json.parent().expect("page dir").join("visuals");
    fs::read_dir(visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .find(|entry| entry.file_type().expect("file type").is_dir())
        .expect("first visual")
        .path()
        .join("visual.json")
}

pub fn assert_strict_valid(project: &Path) {
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(
        output.code, 0,
        "strict validation stderr: {}",
        output.stderr
    );
    assert_eq!(stdout_json(&output)["ok"], Value::Bool(true));
}

pub fn first_two_visual_names(project: &Path) -> (String, String) {
    let page_json = first_page_json(project);
    let visuals_dir = page_json.parent().expect("page dir").join("visuals");
    let mut visual_json_paths = fs::read_dir(visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("file type").is_dir())
        .map(|entry| entry.path().join("visual.json"))
        .collect::<Vec<_>>();
    visual_json_paths.sort();
    let names = visual_json_paths
        .iter()
        .take(2)
        .map(|path| {
            let value: Value =
                serde_json::from_str(&fs::read_to_string(path).expect("visual json"))
                    .expect("parse visual json");
            value["name"]
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| {
                    path.parent()
                        .and_then(Path::file_name)
                        .and_then(|name| name.to_str())
                        .map(ToOwned::to_owned)
                })
                .expect("visual name")
        })
        .collect::<Vec<_>>();
    assert!(names.len() >= 2, "sales fixture should contain two visuals");
    (names[0].clone(), names[1].clone())
}

pub fn patch_json(path: &Path, patch: impl FnOnce(&mut Value)) {
    let mut value: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("json text")).expect("parse json");
    patch(&mut value);
    fs::write(
        path,
        serde_json::to_string_pretty(&value).expect("json pretty"),
    )
    .expect("write json");
}

pub fn categorical_filter_fixture(
    name: &str,
    table: &str,
    column: &str,
    values: Vec<Value>,
) -> Value {
    let alias = table
        .chars()
        .find(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase().to_string())
        .unwrap_or_else(|| "t".to_string());
    let pbi_values = values
        .iter()
        .map(|value| json!([{ "Literal": { "Value": pbi_literal_fixture(value) } }]))
        .collect::<Vec<_>>();
    json!({
        "name": name,
        "type": "Categorical",
        "field": {
            "Column": {
                "Expression": { "SourceRef": { "Entity": table } },
                "Property": column
            }
        },
        "filter": {
            "Version": 2,
            "From": [
                { "Name": alias, "Entity": table, "Type": 0 }
            ],
            "Where": [
                {
                    "Condition": {
                        "In": {
                            "Expressions": [
                                {
                                    "Column": {
                                        "Expression": { "SourceRef": { "Source": alias } },
                                        "Property": column
                                    }
                                }
                            ],
                            "Values": pbi_values
                        }
                    }
                }
            ]
        },
        "howCreated": "User"
    })
}

pub fn pbi_literal_fixture(value: &Value) -> String {
    match value {
        Value::String(text) => format!("'{}'", text.replace('\'', "''")),
        Value::Number(number) if number.as_i64().is_some() || number.as_u64().is_some() => {
            format!("{number}L")
        }
        Value::Number(number) => format!("{number}D"),
        Value::Bool(value) => value.to_string(),
        _ => panic!("unsupported test filter literal: {value}"),
    }
}

pub fn install_filter_fixtures(project: &Path) {
    patch_json(&report_json(project), |report| {
        let mut filter = categorical_filter_fixture(
            "ReportRegionFilter",
            "DimRegion",
            "Region",
            vec![Value::from("North")],
        );
        filter["displayName"] = json!("Region");
        report["filterConfig"]["filters"] = json!([filter]);
    });
    patch_json(&first_page_json(project), |page| {
        page["filterConfig"]["filters"] = json!([{
            "name": "PageRevenueFilter",
            "displayName": "Revenue",
            "type": "Advanced",
            "field": {
                "Column": {
                    "Expression": { "SourceRef": { "Entity": "FactSales" } },
                    "Property": "Revenue"
                }
            },
            "filter": {
                "Version": 2,
                "From": [
                    { "Name": "f", "Entity": "FactSales", "Type": 0 }
                ],
                "Where": [{
                    "Condition": {
                        "Comparison": {
                            "ComparisonKind": 2,
                            "Left": {
                                "Column": {
                                    "Expression": { "SourceRef": { "Source": "f" } },
                                    "Property": "Revenue"
                                }
                            },
                            "Right": { "Literal": { "Value": "1000L" } }
                        }
                    }
                }]
            },
            "howCreated": "User"
        }]);
    });
    patch_json(&first_visual_json(project), |visual| {
        visual["filterConfig"]["filters"] = json!([{
            "name": "VisualUnitsFilter",
            "displayName": "Units",
            "type": "NotYetKnownByCli",
            "field": {
                "Column": {
                    "Expression": { "SourceRef": { "Entity": "FactSales" } },
                    "Property": "Units"
                }
            },
            "filter": { "values": [5] },
            "howCreated": "Auto"
        }]);
    });
}

pub fn install_slicer_fixture(project: &Path) {
    patch_json(&first_visual_json(project), |visual| {
        visual["name"] = json!("VisualContainerRegionSlicer");
        visual["annotations"] = json!([{
            "name": "powerbi-cli.placeholderTitle",
            "value": "Region Slicer"
        }]);
        visual["visual"]["visualType"] = json!("slicer");
        visual["visual"]["query"] = json!({
            "queryState": {
                "Values": {
                    "projections": [{
                        "field": {
                            "Column": {
                                "Expression": { "SourceRef": { "Entity": "DimRegion" } },
                                "Property": "Region"
                            }
                        },
                        "queryRef": "DimRegion.Region",
                        "nativeQueryRef": "Region",
                        "displayName": "Region"
                    }]
                }
            }
        });
        let mut filter = categorical_filter_fixture(
            "SlicerRegionSelection",
            "DimRegion",
            "Region",
            vec![Value::from("North")],
        );
        filter["filterExpressionMetadata"] = json!({
            "cachedValueItems": [{
                "valueMap": { "0": "North" },
                "identities": []
            }]
        });
        visual["filterConfig"]["filters"] = json!([filter]);
        visual["visual"]["objects"] = json!({
            "general": [{
                "properties": {
                    "orientation": {
                        "expr": { "Literal": { "Value": "'vertical'" } }
                    }
                }
            }]
        });
    });
}

pub fn install_interaction_fixture(project: &Path) -> (String, String) {
    let (source, target) = first_two_visual_names(project);
    patch_json(&first_page_json(project), |page| {
        page["visualInteractions"] = json!([
            {
                "source": source.clone(),
                "target": target.clone(),
                "type": "NoFilter"
            },
            {
                "source": target.clone(),
                "target": "MissingVisualForInteraction",
                "type": "SurpriseMode"
            }
        ]);
    });
    (source, target)
}
