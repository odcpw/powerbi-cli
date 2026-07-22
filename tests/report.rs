mod common;

use common::assert_unsupported_feature;
use serde_json::{Value, json};
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

fn run_powerbi_owned(args: &[String]) -> RunOutput {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_powerbi(&args)
}

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

fn scaffold_sales(root: &Path) -> PathBuf {
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

fn build_catalog_proof(root: &Path) -> PathBuf {
    let out_dir = root.join("catalog_proof_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/archetypes/catalog-proof.schema.json",
        "--profile",
        "examples/archetypes/catalog-proof.profile.json",
        "--spec",
        "examples/archetypes/catalog-proof.dashboard.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

fn build_scatter_bubble(root: &Path) -> PathBuf {
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

fn install_registered_theme(project: &Path, theme_name: &str, colors: &[&str]) -> PathBuf {
    let report_dir = project.join("SalesOperations.Report");
    let resource_dir = report_dir
        .join("StaticResources")
        .join("RegisteredResources");
    fs::create_dir_all(&resource_dir).expect("theme resource dir");
    let theme_path = resource_dir.join("CorpTheme.json");
    fs::write(
        &theme_path,
        serde_json::to_string_pretty(&json!({
            "name": theme_name,
            "dataColors": colors,
            "background": "#FFFFFF",
            "foreground": "#222222",
            "tableAccent": colors.first().copied().unwrap_or("#4472C4")
        }))
        .expect("theme json"),
    )
    .expect("write theme resource");

    let report_json_path = report_dir.join("definition").join("report.json");
    let mut report_json: Value =
        serde_json::from_str(&fs::read_to_string(&report_json_path).expect("report json"))
            .expect("parse report json");
    report_json["themeCollection"] = json!({
        "customTheme": {
            "name": theme_name,
            "resource": "StaticResources/RegisteredResources/CorpTheme.json"
        }
    });
    fs::write(
        &report_json_path,
        serde_json::to_string_pretty(&report_json).expect("report json text"),
    )
    .expect("write report json");
    theme_path
}

fn report_pages_json(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join("pages.json")
}

fn report_json(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("report.json")
}

fn first_page_json(project: &Path) -> PathBuf {
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

fn first_page_name(project: &Path) -> String {
    first_page_json(project)
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .expect("first page name")
        .to_string()
}

fn first_visual_json(project: &Path) -> PathBuf {
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

fn first_visual_handle(project: &Path) -> String {
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
    stdout_json(&output)["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string()
}

fn assert_strict_valid(project: &Path) {
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(
        output.code, 0,
        "strict validation stderr: {}",
        output.stderr
    );
    assert_eq!(stdout_json(&output)["ok"], Value::Bool(true));
}

fn first_visual_json_by_type(project: &Path, visual_type: &str) -> PathBuf {
    let page_json = first_page_json(project);
    let visuals_dir = page_json.parent().expect("page dir").join("visuals");
    let mut visual_json_paths = fs::read_dir(visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("file type").is_dir())
        .map(|entry| entry.path().join("visual.json"))
        .collect::<Vec<_>>();
    visual_json_paths.sort();
    visual_json_paths
        .into_iter()
        .find(|path| {
            let value: Value =
                serde_json::from_str(&fs::read_to_string(path).expect("visual json"))
                    .expect("parse visual json");
            value["visual"]["visualType"].as_str() == Some(visual_type)
                || value["visualType"].as_str() == Some(visual_type)
        })
        .unwrap_or_else(|| panic!("visual type not found: {visual_type}"))
}

fn first_two_visual_names(project: &Path) -> (String, String) {
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

fn patch_json(path: &Path, patch: impl FnOnce(&mut Value)) {
    let mut value: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("json text")).expect("parse json");
    patch(&mut value);
    fs::write(
        path,
        serde_json::to_string_pretty(&value).expect("json pretty"),
    )
    .expect("write json");
}

fn categorical_filter_fixture(name: &str, table: &str, column: &str, values: Vec<Value>) -> Value {
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

fn pbi_literal_fixture(value: &Value) -> String {
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

fn install_filter_fixtures(project: &Path) {
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

fn exercise_authored_filter_lifecycle<F>(
    root: &Path,
    scope: &str,
    target: &str,
    kind_args: &[&str],
    expected_type: &str,
    updated_display_name: &str,
    assert_shape: F,
) where
    F: Fn(&Value),
{
    fs::create_dir_all(root).expect("lifecycle root");
    let project = scaffold_sales(root);
    let project_arg = project.to_str().expect("project path");
    let owner_args = match scope {
        "report" => vec!["--scope".to_string(), "report".to_string()],
        "page" => vec![
            "--scope".to_string(),
            "page".to_string(),
            "--page".to_string(),
            first_page_name(&project),
        ],
        "visual" => vec![
            "--scope".to_string(),
            "visual".to_string(),
            "--visual".to_string(),
            first_visual_handle(&project),
        ],
        other => panic!("unsupported lifecycle scope: {other}"),
    };

    let add_args = |project: &str, mode: &[String], include_raw: bool| {
        let mut args = vec![
            "report".to_string(),
            "filters".to_string(),
            "add".to_string(),
            "--project".to_string(),
            project.to_string(),
        ];
        args.extend(owner_args.clone());
        args.extend(["--target".to_string(), target.to_string()]);
        args.extend(kind_args.iter().map(|arg| (*arg).to_string()));
        args.extend(mode.iter().cloned());
        if include_raw {
            args.push("--include-raw".to_string());
        }
        args.push("--json".to_string());
        args
    };

    let dry = run_powerbi_owned(&add_args(project_arg, &["--dry-run".to_string()], true));
    assert_eq!(dry.code, 0, "add dry-run stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        dry_json["filterPlan"]["rawAfterIncluded"],
        Value::Bool(true)
    );
    assert_eq!(dry_json["changes"][0]["after"]["type"], expected_type);
    assert_shape(&dry_json["changes"][0]["after"]);

    let added = root.join("added");
    let added_arg = added.to_str().expect("added path");
    let add = run_powerbi_owned(&add_args(
        project_arg,
        &["--out-dir".to_string(), added_arg.to_string()],
        false,
    ));
    assert_eq!(add.code, 0, "add stderr: {}", add.stderr);
    assert_eq!(stdout_json(&add)["validation"]["ok"], Value::Bool(true));
    assert_strict_valid(&added);

    let list = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        added_arg,
        "--scope",
        scope,
        "--json",
    ]);
    assert_eq!(list.code, 0, "list stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(list_json["counts"]["filters"], Value::from(1));
    assert_eq!(list_json["filters"][0]["filterType"], expected_type);
    let handle = list_json["filters"][0]["handle"]
        .as_str()
        .expect("filter handle")
        .to_string();

    let show = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        added_arg,
        "--handle",
        &handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "show stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_shape(&show_json["filter"]["raw"]);

    let update_dry = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        added_arg,
        "--handle",
        &handle,
        "--display-name",
        updated_display_name,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        update_dry.code, 0,
        "update dry-run stderr: {}",
        update_dry.stderr
    );
    let update_dry_json = stdout_json(&update_dry);
    assert_eq!(
        update_dry_json["schema"],
        Value::from("powerbi-cli.report.filters.updateMutation.v1")
    );
    assert_eq!(
        update_dry_json["filterPlan"]["rawIncluded"],
        Value::Bool(true)
    );
    assert_eq!(
        update_dry_json["filterPlan"]["before"]["type"],
        expected_type
    );
    assert_eq!(
        update_dry_json["filterPlan"]["after"]["displayName"],
        updated_display_name
    );

    let updated = root.join("updated");
    let updated_arg = updated.to_str().expect("updated path");
    let update = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        added_arg,
        "--handle",
        &handle,
        "--display-name",
        updated_display_name,
        "--out-dir",
        updated_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "update stderr: {}", update.stderr);
    let update_json = stdout_json(&update);
    assert_eq!(update_json["validation"]["ok"], Value::Bool(true));
    assert_eq!(update_json["filterPlan"]["rawIncluded"], Value::Bool(false));
    assert!(update_json["filterPlan"]["before"].get("filter").is_none());
    assert!(update_json["filterPlan"]["after"].get("filter").is_none());
    assert_strict_valid(&updated);

    let updated_show = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        updated_arg,
        "--handle",
        &handle,
        "--json",
    ]);
    assert_eq!(
        updated_show.code, 0,
        "updated show stderr: {}",
        updated_show.stderr
    );
    let updated_show_json = stdout_json(&updated_show);
    assert_eq!(
        updated_show_json["filter"]["displayName"],
        updated_display_name
    );
    assert_shape(&updated_show_json["filter"]["raw"]);

    let delete_dry = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        updated_arg,
        "--handle",
        &handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        delete_dry.code, 0,
        "delete dry-run stderr: {}",
        delete_dry.stderr
    );

    let deleted = root.join("deleted");
    let deleted_arg = deleted.to_str().expect("deleted path");
    let delete = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        updated_arg,
        "--handle",
        &handle,
        "--out-dir",
        deleted_arg,
        "--json",
    ]);
    assert_eq!(delete.code, 0, "delete stderr: {}", delete.stderr);
    assert_eq!(stdout_json(&delete)["validation"]["ok"], Value::Bool(true));
    assert_strict_valid(&deleted);

    let after_delete = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        deleted_arg,
        "--scope",
        scope,
        "--json",
    ]);
    assert_eq!(
        after_delete.code, 0,
        "deleted list stderr: {}",
        after_delete.stderr
    );
    assert_eq!(
        stdout_json(&after_delete)["counts"]["filters"],
        Value::from(0)
    );
}

fn assert_numeric_range_shape(filter: &Value, comparisons: &[(i64, &str)]) {
    assert_eq!(filter["type"], Value::from("Advanced"));
    assert_eq!(filter["filter"]["Version"], Value::from(2));
    let alias = filter["filter"]["From"][0]["Name"]
        .as_str()
        .expect("range source alias");
    let condition = &filter["filter"]["Where"][0]["Condition"];
    let actual = if let Some(comparison) = condition.get("Comparison") {
        vec![comparison]
    } else {
        vec![
            &condition["And"]["Left"]["Comparison"],
            &condition["And"]["Right"]["Comparison"],
        ]
    };
    assert_eq!(actual.len(), comparisons.len());
    for (comparison, (kind, literal)) in actual.into_iter().zip(comparisons) {
        assert_eq!(comparison["ComparisonKind"], Value::from(*kind));
        assert_eq!(
            comparison["Left"]["Column"]["Expression"]["SourceRef"]["Source"],
            Value::from(alias)
        );
        assert_eq!(
            comparison["Right"]["Literal"]["Value"],
            Value::from(*literal)
        );
    }
}

fn assert_topn_shape(filter: &Value, direction: i64) {
    assert_eq!(filter["type"], Value::from("TopN"));
    assert_eq!(filter["filter"]["Version"], Value::from(2));
    assert_eq!(filter["filter"]["From"][0]["Name"], Value::from("topn"));
    assert_eq!(filter["filter"]["From"][0]["Type"], Value::from(2));
    let query = &filter["filter"]["From"][0]["Expression"]["Subquery"]["Query"];
    assert_eq!(query["Version"], Value::from(2));
    assert_eq!(query["Top"], Value::from(5));
    assert_eq!(
        query["Select"][0]["Column"]["Property"],
        Value::from("CustomerName")
    );
    assert_eq!(
        query["Select"][0]["Column"]["Expression"]["SourceRef"]["Source"],
        Value::from("t")
    );
    assert_eq!(query["OrderBy"][0]["Direction"], Value::from(direction));
    assert_eq!(
        query["OrderBy"][0]["Expression"]["Measure"]["Property"],
        Value::from("Total Revenue")
    );
    assert_eq!(
        query["OrderBy"][0]["Expression"]["Measure"]["Expression"]["SourceRef"]["Source"],
        Value::from("m")
    );
    assert!(query["From"].as_array().is_some_and(|from| {
        from.iter()
            .any(|source| source["Name"] == "t" && source["Entity"] == "DimCustomer")
            && from
                .iter()
                .any(|source| source["Name"] == "m" && source["Entity"] == "FactSales")
    }));
    assert_eq!(
        filter["filter"]["Where"][0]["Condition"]["In"]["Table"]["SourceRef"]["Source"],
        Value::from("topn")
    );
    assert_eq!(
        filter["filter"]["Where"][0]["Condition"]["In"]["Expressions"][0]["Column"]["Expression"]["SourceRef"]
            ["Source"],
        filter["filter"]["From"][1]["Name"]
    );
}

fn assert_relative_date_shape(filter: &Value, lower: &Value, upper: &Value) {
    assert_eq!(filter["type"], Value::from("RelativeDate"));
    assert_eq!(filter["filter"]["Version"], Value::from(2));
    let alias = filter["filter"]["From"][0]["Name"]
        .as_str()
        .expect("relative-date source alias");
    let between = &filter["filter"]["Where"][0]["Condition"]["Between"];
    assert_eq!(
        between["Expression"]["Column"]["Expression"]["SourceRef"]["Source"],
        Value::from(alias)
    );
    assert_eq!(&between["LowerBound"], lower);
    assert_eq!(&between["UpperBound"], upper);
}

fn assert_error(output: &RunOutput, code: &str, message: &str) {
    assert_ne!(output.code, 0, "command unexpectedly succeeded");
    let error = stderr_json(output);
    assert_eq!(error["error"]["code"], Value::from(code));
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("error message")
            .contains(message),
        "unexpected error: {error}"
    );
}

fn install_slicer_fixture(project: &Path) {
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

fn install_visual_formatting_fixture(project: &Path) {
    patch_json(&first_visual_json_by_type(project, "card"), |visual| {
        visual["visual"]["visualContainerObjects"]
            .as_object_mut()
            .expect("visual container objects")
            .remove("general");
        visual["visual"]["objects"] = json!({
            "general": [{
                "properties": {
                    "orientation": {
                        "expr": { "Literal": { "Value": "'vertical'" } }
                    },
                    "altText": {
                        "expr": { "Literal": { "Value": "'Executive revenue chart'" } }
                    }
                }
            }],
            "dataPoint": [{
                "selector": {
                    "data": [{ "dataViewWildcard": { "matchingOption": 0 } }]
                },
                "properties": {
                    "fill": {
                        "solid": {
                            "color": {
                                "expr": { "Literal": { "Value": "'#123456'" } }
                            }
                        }
                    }
                }
            }],
            "title": [{
                "properties": {
                    "show": {
                        "expr": { "Literal": { "Value": "true" } }
                    },
                    "text": {
                        "expr": { "Literal": { "Value": "'Revenue Overview'" } }
                    },
                    "fontColor": {
                        "solid": {
                            "color": {
                                "expr": { "Literal": { "Value": "'#654321'" } }
                            }
                        }
                    }
                }
            }]
        });
    });
}

fn install_interaction_fixture(project: &Path) -> (String, String) {
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

fn install_bookmark_fixtures(project: &Path) {
    let report_dir = project.join("SalesOperations.Report");
    let bookmarks_dir = report_dir.join("definition").join("bookmarks");
    fs::create_dir_all(&bookmarks_dir).expect("bookmarks dir");

    let page_path = first_page_json(project);
    let page_name = page_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .expect("page name")
        .to_string();
    let visual_path = first_visual_json(project);
    let visual_name = visual_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .expect("visual name")
        .to_string();

    fs::write(
        bookmarks_dir.join("bookmarks.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmarksMetadata/1.0.0/schema.json",
            "items": [
                { "name": "BookmarkExecutive" },
                {
                    "name": "OperationsGroup",
                    "displayName": "Operations",
                    "children": ["BookmarkVisualFocus"]
                }
            ]
        }))
        .expect("bookmarks metadata"),
    )
    .expect("write bookmarks metadata");

    let mut visual_containers = serde_json::Map::new();
    visual_containers.insert(
        visual_name.clone(),
        json!({
            "filters": {
                "byType": [{
                    "name": "VisualUnitsFilter",
                    "filterExpressionMetadata": {
                        "expressions": [],
                        "cachedValueItems": [{
                            "identities": [],
                            "valueMap": { "0": "North" }
                        }]
                    }
                }]
            },
            "singleVisual": {
                "display": { "mode": "spotlight" }
            },
            "highlight": {
                "selection": [{
                    "metadata": ["DimRegion.Region"],
                    "id": "North"
                }]
            }
        }),
    );
    let mut sections = serde_json::Map::new();
    sections.insert(
        page_name.clone(),
        json!({
            "filters": {
                "byName": {
                    "PageRegionFilter": {
                        "name": "PageRegionFilter",
                        "filter": { "values": ["North"] }
                    }
                }
            },
            "visualContainers": Value::Object(visual_containers)
        }),
    );

    fs::write(
        bookmarks_dir.join("BookmarkExecutive.bookmark.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/2.1.0/schema.json",
            "displayName": "Executive View",
            "name": "BookmarkExecutive",
            "options": {
                "suppressDisplay": false
            },
            "explorationState": {
                "version": "1.3",
                "activeSection": page_name,
                "filters": {
                    "byExpr": [{
                        "name": "ReportRegionFilter",
                        "filter": {
                            "Version": 2,
                            "Where": [{
                                "Condition": {
                                    "In": {
                                        "Expressions": [{
                                            "Column": {
                                                "Expression": { "SourceRef": { "Entity": "DimRegion" } },
                                                "Property": "Region"
                                            }
                                        }],
                                        "Values": [[{ "Literal": { "Value": "'North'" } }]]
                                    }
                                }
                            }]
                        }
                    }]
                },
                "sections": Value::Object(sections)
            }
        }))
        .expect("bookmark json"),
    )
    .expect("write executive bookmark");

    let mut visual_focus_sections = serde_json::Map::new();
    visual_focus_sections.insert(
        page_name.clone(),
        json!({
            "visualContainers": {}
        }),
    );
    fs::write(
        bookmarks_dir.join("BookmarkVisualFocus.bookmark.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/2.1.0/schema.json",
            "displayName": "Visual Focus",
            "name": "BookmarkVisualFocus",
            "options": {
                "applyOnlyToTargetVisuals": true,
                "targetVisualNames": [visual_name],
                "suppressData": true
            },
            "explorationState": {
                "version": "1.3",
                "activeSection": page_name,
                "sections": Value::Object(visual_focus_sections)
            }
        }))
        .expect("bookmark json"),
    )
    .expect("write visual focus bookmark");
}

#[test]
fn validate_accepts_desktop_field_well_filter_placeholders() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    patch_json(&first_visual_json(&project), |visual| {
        visual["filterConfig"]["filters"] = json!([
            {
                "name": "desktopCategoryPlaceholder",
                "field": {
                    "Column": {
                        "Expression": { "SourceRef": { "Entity": "DimDate" } },
                        "Property": "Month"
                    }
                },
                "type": "Categorical"
            },
            {
                "name": "desktopMeasurePlaceholder",
                "field": {
                    "Measure": {
                        "Expression": { "SourceRef": { "Entity": "FactSales" } },
                        "Property": "Total Revenue"
                    }
                },
                "type": "Advanced"
            }
        ]);
    });

    assert_strict_valid(&project);
}

#[test]
fn report_object_tree_find_cat_and_query_expose_stable_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    install_slicer_fixture(&project);
    install_interaction_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let tree = run_powerbi(&["report", "tree", "--project", project_arg, "--json"]);
    assert_eq!(tree.code, 0, "stderr: {}", tree.stderr);
    let tree_json = stdout_json(&tree);
    assert_eq!(
        tree_json["schema"],
        Value::from("powerbi-cli.report.objects.tree.v1")
    );
    assert!(tree_json["counts"]["page"].as_u64().unwrap_or_default() > 0);
    assert!(tree_json["counts"]["visual"].as_u64().unwrap_or_default() > 0);
    assert!(tree_json["counts"]["binding"].as_u64().unwrap_or_default() > 0);
    assert!(tree_json["counts"]["filter"].as_u64().unwrap_or_default() > 0);
    assert!(
        tree_json["counts"]["interaction"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let binding_handle = tree_json["objects"]
        .as_array()
        .expect("objects")
        .iter()
        .find(|object| object["kind"] == "binding")
        .and_then(|object| object["handle"].as_str())
        .expect("binding handle")
        .to_string();

    let find = run_powerbi(&[
        "report",
        "find",
        "--project",
        project_arg,
        "--kind",
        "visual",
        "--json",
    ]);
    assert_eq!(find.code, 0, "stderr: {}", find.stderr);
    let find_json = stdout_json(&find);
    assert!(find_json["counts"]["matched"].as_u64().unwrap_or_default() > 0);

    let cat = run_powerbi(&[
        "report",
        "cat",
        "--project",
        project_arg,
        "--handle",
        &binding_handle,
        "--json",
    ]);
    assert_eq!(cat.code, 0, "stderr: {}", cat.stderr);
    let cat_json = stdout_json(&cat);
    assert_eq!(
        cat_json["schema"],
        Value::from("powerbi-cli.report.objects.cat.v1")
    );
    assert_eq!(cat_json["object"]["kind"], Value::from("binding"));
    assert_eq!(cat_json["rawIncluded"], Value::Bool(false));
    assert_eq!(cat_json["raw"], Value::Null);

    let query = run_powerbi(&[
        "report",
        "query",
        "--project",
        project_arg,
        "--selector",
        "kind:binding",
        "--json",
    ]);
    assert_eq!(query.code, 0, "stderr: {}", query.stderr);
    let query_json = stdout_json(&query);
    assert!(query_json["counts"]["matched"].as_u64().unwrap_or_default() > 0);
}

#[test]
fn report_audit_and_sanitize_clear_filter_and_slicer_state_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    install_slicer_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let audit = run_powerbi(&["report", "audit", "--project", project_arg, "--json"]);
    assert_eq!(audit.code, 0, "stderr: {}", audit.stderr);
    let audit_json = stdout_json(&audit);
    assert_eq!(
        audit_json["schema"],
        Value::from("powerbi-cli.report.audit.v1")
    );
    assert!(
        audit_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["ruleId"] == "filter.possible_persisted_values")
    );
    assert!(
        audit_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["ruleId"] == "slicer.possible_persisted_values")
    );

    let plan = run_powerbi(&[
        "report",
        "sanitize",
        "plan",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(plan.code, 0, "stderr: {}", plan.stderr);
    let plan_json = stdout_json(&plan);
    assert_eq!(
        plan_json["schema"],
        Value::from("powerbi-cli.report.sanitize.plan.v1")
    );
    assert!(
        plan_json["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .any(|action| action["kind"] == "clear-filter-values")
    );
    assert!(
        plan_json["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .any(|action| action["kind"] == "clear-slicer-selections")
    );

    let dry_run = run_powerbi(&[
        "report",
        "sanitize",
        "apply",
        "--project",
        project_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert!(dry_json["changes"].as_array().expect("changes").len() >= 3);

    let original_filters = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(
        original_filters.code, 0,
        "stderr: {}",
        original_filters.stderr
    );
    assert!(
        stdout_json(&original_filters)["counts"]["filters"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let sanitized = temp.path().join("sanitized");
    let sanitized_arg = sanitized.to_str().expect("sanitized path");
    let apply = run_powerbi(&[
        "report",
        "sanitize",
        "apply",
        "--project",
        project_arg,
        "--out-dir",
        sanitized_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["mode"], Value::from("out-dir"));
    assert_eq!(apply_json["dryRun"], Value::Bool(false));

    let sanitized_filters = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        sanitized_arg,
        "--json",
    ]);
    assert_eq!(
        sanitized_filters.code, 0,
        "stderr: {}",
        sanitized_filters.stderr
    );
    assert_eq!(
        stdout_json(&sanitized_filters)["counts"]["filters"],
        Value::from(0)
    );

    let still_original = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(still_original.code, 0, "stderr: {}", still_original.stderr);
    assert!(
        stdout_json(&still_original)["counts"]["filters"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "source project must not be changed by --out-dir"
    );
}

#[test]
fn report_sanitize_in_place_requires_exact_confirm_token() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");

    let rejected = run_powerbi(&[
        "report",
        "sanitize",
        "apply",
        "--project",
        project_arg,
        "--in-place",
        "--confirm",
        "sanitize:not-the-plan",
        "--json",
    ]);
    assert_eq!(rejected.code, 2);
    let error = stderr_json(&rejected);
    assert_eq!(error["error"]["code"], Value::from("invalid_args"));
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --confirm sanitize:fnv64:")
    );
}

#[test]
fn report_pages_and_visuals_are_readable_by_handle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let pages_json = stdout_json(&pages);
    assert_eq!(
        pages_json["schema"],
        Value::from("powerbi-cli.report.pages.list.v1")
    );
    assert_eq!(pages_json["counts"]["pages"], Value::from(1));
    let page_handle = pages_json["pages"][0]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();

    let page = run_powerbi(&[
        "report",
        "pages",
        "show",
        "--project",
        project_arg,
        "--handle",
        &page_handle,
        "--json",
    ]);
    assert_eq!(page.code, 0, "stderr: {}", page.stderr);
    let page_json = stdout_json(&page);
    assert_eq!(
        page_json["schema"],
        Value::from("powerbi-cli.report.pages.show.v1")
    );
    assert_eq!(
        page_json["page"]["handle"],
        Value::from(page_handle.clone())
    );
    assert_eq!(
        page_json["page"]["visuals"]
            .as_array()
            .expect("page visuals")
            .len(),
        3
    );

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    assert_eq!(
        visuals_json["schema"],
        Value::from("powerbi-cli.report.visuals.list.v1")
    );
    assert_eq!(visuals_json["counts"]["visuals"], Value::from(3));
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();

    let visual = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--json",
    ]);
    assert_eq!(visual.code, 0, "stderr: {}", visual.stderr);
    let visual_json = stdout_json(&visual);
    assert_eq!(
        visual_json["schema"],
        Value::from("powerbi-cli.report.visuals.show.v1")
    );
    assert_eq!(
        visual_json["visual"]["handle"],
        Value::from(visual_handle.clone())
    );
    assert!(visual_json["visual"]["position"].is_object());
    assert!(visual_json["visual"]["bindings"].is_array());
}

#[test]
fn report_visuals_formatting_list_and_show_summarize_objects_without_raw() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_visual_formatting_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let list = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(
        list_json["schema"],
        Value::from("powerbi-cli.report.visuals.formatting.list.v1")
    );
    assert_eq!(list_json["counts"]["visuals"], Value::from(3));
    assert_eq!(list_json["counts"]["visualsWithFormatting"], Value::from(3));
    assert_eq!(
        list_json["counts"]["formatObjectContainers"],
        Value::from(6)
    );
    assert_eq!(list_json["counts"]["formatProperties"], Value::from(12));
    assert_eq!(list_json["rawIncluded"], Value::Bool(false));
    assert!(
        !list.stdout.contains("#123456"),
        "raw color literal should be omitted by default"
    );
    assert!(
        !list.stdout.contains("'Revenue Overview'"),
        "raw title literal should be omitted by default"
    );

    let formatted_visual = list_json["visuals"]
        .as_array()
        .expect("visual rows")
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .expect("formatted visual");
    let handle = formatted_visual["handle"].as_str().expect("visual handle");
    let object_names = formatted_visual["formatting"]["objectNames"]
        .as_array()
        .expect("object names");
    assert!(object_names.iter().any(|name| name == "title"));
    assert!(object_names.iter().any(|name| name == "dataPoint"));
    assert!(object_names.iter().any(|name| name == "general"));

    let show = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "show",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["schema"],
        Value::from("powerbi-cli.report.visuals.formatting.show.v1")
    );
    assert_eq!(show_json["visual"]["handle"], Value::from(handle));
    assert_eq!(
        show_json["formatting"]["formatPropertyCount"],
        Value::from(8)
    );
    assert_eq!(show_json["formatting"]["rawIncluded"], Value::Bool(false));
    let title_container = show_json["formatting"]["containers"]
        .as_array()
        .expect("containers")
        .iter()
        .find(|container| {
            container["source"] == "visual.objects" && container["objectName"] == "title"
        })
        .expect("title container");
    assert_eq!(title_container["propertyCount"], Value::from(3));
    assert!(title_container.get("raw").is_none());

    let raw_show = run_powerbi(&[
        "report",
        "visuals",
        "format",
        "show",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(raw_show.code, 0, "stderr: {}", raw_show.stderr);
    let raw_json = stdout_json(&raw_show);
    assert_eq!(raw_json["formatting"]["rawIncluded"], Value::Bool(true));
    assert!(
        raw_show.stdout.contains("#123456"),
        "raw opt-in should include formatting literal values"
    );
    assert!(
        raw_json["formatting"]["containers"]
            .as_array()
            .expect("raw containers")
            .iter()
            .any(|container| container.get("raw").is_some())
    );
}

#[test]
fn report_visuals_formatting_extract_and_apply_round_trip_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_visual_formatting_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_rows = visuals_json["visuals"].as_array().expect("visual rows");
    let source_visual = visual_rows
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .expect("source card visual");
    let non_card_visual = visual_rows
        .iter()
        .find(|visual| visual["visualType"] != "card")
        .expect("non-card visual");
    let source_handle = source_visual["handle"]
        .as_str()
        .expect("source visual handle")
        .to_string();
    let non_card_handle = non_card_visual["handle"]
        .as_str()
        .expect("non-card handle")
        .to_string();
    let source_path = PathBuf::from(source_visual["path"].as_str().expect("source visual path"));
    let source_before = fs::read_to_string(&source_path).expect("source visual before");

    let bundle_path = temp.path().join("visual-formatting-bundle.json");
    let bundle_arg = bundle_path.to_str().expect("bundle path");
    let extract = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "extract",
        "--project",
        project_arg,
        "--handle",
        &source_handle,
        "--out",
        bundle_arg,
        "--json",
    ]);
    assert_eq!(extract.code, 0, "stderr: {}", extract.stderr);
    let extract_json = stdout_json(&extract);
    assert_eq!(
        extract_json["schema"],
        Value::from("powerbi-cli.report.visuals.formatting.extract.v1")
    );
    assert!(bundle_path.is_file(), "formatting bundle was not written");
    assert_eq!(
        extract_json["bundle"]["schema"],
        Value::from("powerbi-cli.report.visuals.formatting-bundle.v1")
    );
    assert_eq!(
        extract_json["bundle"]["summary"]["formatObjectContainerCount"],
        Value::from(3)
    );
    assert_eq!(
        extract_json["bundle"]["formatting"]["visualObjects"]["title"][0]["properties"]["fontColor"]
            ["solid"]["color"]["expr"]["Literal"]["Value"],
        Value::from("'#654321'")
    );
    assert_eq!(
        extract_json["bundle"]["safety"]["containsLiteralText"],
        Value::Bool(true)
    );
    assert_eq!(
        extract_json["bundle"]["safety"]["containsColors"],
        Value::Bool(true)
    );
    assert_eq!(
        extract_json["bundle"]["safety"]["containsDataSelectors"],
        Value::Bool(false)
    );

    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let page_handle = stdout_json(&pages)["pages"][0]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();

    let target_project = temp.path().join("sales_project_target_card");
    let target_arg = target_project.to_str().expect("target project path");
    let add_target = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--title",
        "Styled Target",
        "--visual-type",
        "card",
        "--binding",
        "role=Values,table=FactSales,measure=Total Revenue",
        "--out-dir",
        target_arg,
        "--json",
    ]);
    assert_eq!(add_target.code, 0, "stderr: {}", add_target.stderr);
    let add_target_json = stdout_json(&add_target);
    let target_handle = add_target_json["target"]["handle"]
        .as_str()
        .expect("target visual handle")
        .to_string();
    let target_path = PathBuf::from(
        add_target_json["target"]["path"]
            .as_str()
            .expect("target visual path"),
    );
    let target_before_text = fs::read_to_string(&target_path).expect("target visual before");
    let target_before_json: Value =
        serde_json::from_str(&target_before_text).expect("target visual json");

    let literal_rejected = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "apply",
        "--project",
        target_arg,
        "--handle",
        &target_handle,
        "--bundle",
        bundle_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(literal_rejected.code, 2);
    assert!(
        stderr_json(&literal_rejected)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("literal text")
    );

    let mismatch = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "apply",
        "--project",
        target_arg,
        "--handle",
        &non_card_handle,
        "--bundle",
        bundle_arg,
        "--allow-literal-text",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(mismatch.code, 2);
    assert!(
        stderr_json(&mismatch)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("visualType")
    );

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "apply",
        "--project",
        target_arg,
        "--handle",
        &target_handle,
        "--bundle",
        bundle_arg,
        "--allow-literal-text",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.visuals.formatting.mutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["mode"], Value::from("dry-run"));
    assert_eq!(
        dry_json["formattingPlan"]["after"]["formatObjectContainerCount"],
        Value::from(3)
    );
    assert!(
        dry_json["changes"][0]["jsonPointers"]
            .as_array()
            .expect("json pointers")
            .iter()
            .any(|pointer| pointer == "/visual/objects")
    );
    assert_eq!(
        fs::read_to_string(&target_path).expect("target visual after dry-run"),
        target_before_text
    );

    let styled_project = temp.path().join("sales_project_styled");
    let styled_arg = styled_project.to_str().expect("styled project path");
    let apply = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "apply",
        "--project",
        target_arg,
        "--handle",
        &target_handle,
        "--bundle",
        bundle_arg,
        "--allow-literal-text",
        "--include-raw",
        "--out-dir",
        styled_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["ok"], Value::Bool(true));
    assert_eq!(apply_json["mode"], Value::from("out-dir"));
    assert_eq!(apply_json["validation"]["ok"], Value::Bool(true));
    assert_eq!(
        fs::read_to_string(&source_path).expect("source visual after apply"),
        source_before
    );
    assert_eq!(
        fs::read_to_string(&target_path).expect("target project visual after out-dir"),
        target_before_text
    );

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "show",
        "--project",
        styled_arg,
        "--handle",
        &target_handle,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(
        readback_json["formatting"]["formatObjectContainerCount"],
        Value::from(4)
    );
    assert!(
        readback.stdout.contains("#123456"),
        "styled readback should contain copied color"
    );
    assert!(
        readback.stdout.contains("Revenue Overview"),
        "styled readback should contain opted-in copied literal text"
    );

    let styled_visual_path = PathBuf::from(
        readback_json["visual"]["path"]
            .as_str()
            .expect("styled visual path"),
    );
    let styled_visual_json: Value =
        serde_json::from_str(&fs::read_to_string(styled_visual_path).expect("styled visual json"))
            .expect("parse styled visual json");
    assert_eq!(
        styled_visual_json["position"],
        target_before_json["position"]
    );
    assert_eq!(styled_visual_json["name"], target_before_json["name"]);
    assert_eq!(
        styled_visual_json["visual"]["visualType"],
        target_before_json["visual"]["visualType"]
    );
    assert_eq!(
        styled_visual_json["visual"]["query"],
        target_before_json["visual"]["query"]
    );

    let validate = run_powerbi(&["validate", "--strict", styled_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
}

#[test]
fn report_visuals_formatting_set_text_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_visual_formatting_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let source_visual = visuals_json["visuals"]
        .as_array()
        .expect("visual rows")
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .expect("card visual");
    let handle = source_visual["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let source_path = PathBuf::from(source_visual["path"].as_str().expect("visual path"));
    let source_before = fs::read_to_string(&source_path).expect("source visual before");

    let legacy_lint = run_powerbi(&["lint", project_arg, "--json"]);
    assert_eq!(legacy_lint.code, 0, "stderr: {}", legacy_lint.stderr);
    let legacy_lint_json = stdout_json(&legacy_lint);
    let legacy_finding = legacy_lint_json["findings"]
        .as_array()
        .expect("lint findings")
        .iter()
        .find(|finding| finding["code"] == "pbir.visual_alt_text_legacy_location")
        .expect("legacy alt text should produce an actionable lint finding");
    assert!(
        legacy_finding["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--clear-alt-text")
    );

    let rejected_alt_text = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-text",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--alt-text",
        "Updated executive KPI",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(rejected_alt_text.code, 2);
    let rejected_json = stderr_json(&rejected_alt_text);
    assert_eq!(rejected_json["error"]["code"], "unsupported_feature");
    assert!(
        rejected_json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("PBIR_FORMATTING_PROP_UNKNOWN")
    );
    assert_eq!(
        fs::read_to_string(&source_path).expect("source visual after refused alt text"),
        source_before
    );

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-text",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--title",
        "Updated Revenue",
        "--include-raw",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.visuals.formatting.textMutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        dry_json["textPlan"]["requested"]["autoShowTitle"],
        Value::Bool(true)
    );
    assert_eq!(
        dry_json["textPlan"]["after"]["title"],
        Value::from("Updated Revenue")
    );
    assert_eq!(
        dry_json["textPlan"]["after"]["showTitle"],
        Value::Bool(true)
    );
    assert_eq!(
        dry_json["textPlan"]["after"]["altText"],
        Value::from("Executive revenue chart")
    );
    assert_eq!(
        dry_json["textPlan"]["before"]["altTextSource"],
        Value::from("legacyVisualObjects")
    );
    assert_eq!(
        dry_json["textPlan"]["after"]["altTextSource"],
        Value::from("legacyVisualObjects")
    );
    let dry_pointers = dry_json["changes"][0]["jsonPointers"]
        .as_array()
        .expect("json pointers");
    assert!(
        dry_pointers
            .iter()
            .any(|pointer| pointer == "/visual/objects/title/0/properties/text/expr/Literal/Value")
    );
    assert!(dry_pointers.iter().any(|pointer| {
        pointer == "/visual/visualContainerObjects/title/0/properties/text/expr/Literal/Value"
    }));
    assert!(
        dry_pointers
            .iter()
            .any(|pointer| pointer == "/annotations/0/value")
    );
    assert!(
        dry_pointers
            .iter()
            .all(|pointer| !pointer.as_str().unwrap_or_default().contains("altText"))
    );
    assert_eq!(
        fs::read_to_string(&source_path).expect("source visual after dry-run"),
        source_before
    );

    let styled_project = temp.path().join("sales_project_text");
    let styled_arg = styled_project.to_str().expect("styled project path");
    let apply = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-text",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--title",
        "Updated Revenue",
        "--show-title",
        "false",
        "--out-dir",
        styled_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["mode"], Value::from("out-dir"));
    assert_eq!(apply_json["validation"]["ok"], Value::Bool(true));
    assert_eq!(
        fs::read_to_string(&source_path).expect("source visual after out-dir"),
        source_before
    );
    let styled_visual_path = PathBuf::from(
        apply_json["target"]["path"]
            .as_str()
            .expect("styled visual path"),
    );
    let styled_visual_json: Value =
        serde_json::from_str(&fs::read_to_string(&styled_visual_path).expect("styled visual json"))
            .expect("parse styled visual json");
    assert_eq!(
        styled_visual_json["visual"]["objects"]["title"][0]["properties"]["text"]["expr"]["Literal"]
            ["Value"],
        Value::from("'Updated Revenue'")
    );
    assert_eq!(
        styled_visual_json["visual"]["objects"]["title"][0]["properties"]["show"]["expr"]["Literal"]
            ["Value"],
        Value::from("false")
    );
    assert_eq!(
        styled_visual_json["visual"]["visualContainerObjects"]["title"][0]["properties"]["text"]["expr"]
            ["Literal"]["Value"],
        Value::from("'Updated Revenue'")
    );
    assert_eq!(
        styled_visual_json["visual"]["visualContainerObjects"]["title"][0]["properties"]["show"]["expr"]
            ["Literal"]["Value"],
        Value::from("false")
    );
    assert_eq!(
        styled_visual_json["annotations"][0]["value"],
        Value::from("Updated Revenue")
    );
    assert_eq!(
        styled_visual_json["visual"]["objects"]["general"][0]["properties"]["orientation"]["expr"]
            ["Literal"]["Value"],
        Value::from("'vertical'"),
        "title mutation must preserve sibling formatting properties"
    );
    assert_eq!(
        styled_visual_json["visual"]["objects"]["general"][0]["properties"]["altText"]["expr"]["Literal"]
            ["Value"],
        Value::from("'Executive revenue chart'"),
        "title-only mutation must not silently rewrite existing invalid metadata"
    );
    assert_eq!(
        styled_visual_json["visual"]["objects"]["title"][0]["properties"]["fontColor"]["solid"]["color"]
            ["expr"]["Literal"]["Value"],
        Value::from("'#654321'")
    );

    let styled_lint = run_powerbi(&["lint", styled_arg, "--json"]);
    assert_eq!(styled_lint.code, 0, "stderr: {}", styled_lint.stderr);
    assert!(
        stdout_json(&styled_lint)["findings"]
            .as_array()
            .expect("lint findings")
            .iter()
            .any(|finding| finding["code"] == "pbir.visual_alt_text_legacy_location"),
        "title-only mutation should leave rejected alt text visible to lint"
    );

    let visual_show = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        styled_arg,
        "--handle",
        &handle,
        "--json",
    ]);
    assert_eq!(visual_show.code, 0, "stderr: {}", visual_show.stderr);
    assert_eq!(
        stdout_json(&visual_show)["visual"]["title"],
        Value::from("Updated Revenue")
    );

    patch_json(&styled_visual_path, |visual| {
        visual["visual"]["visualContainerObjects"]["general"] = json!([{
            "properties": {
                "altText": {
                    "expr": { "Literal": { "Value": "'Rejected shared alt text'" } }
                }
            }
        }]);
    });
    let container_lint = run_powerbi(&["lint", styled_arg, "--json"]);
    assert_eq!(container_lint.code, 0, "stderr: {}", container_lint.stderr);
    let container_lint_json = stdout_json(&container_lint);
    let container_finding = container_lint_json["findings"]
        .as_array()
        .expect("lint findings")
        .iter()
        .find(|finding| finding["code"] == "pbir.visual_alt_text_unsupported_location")
        .expect("visual-container alt text should produce an actionable lint finding");
    assert!(
        container_finding["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--clear-alt-text")
    );
    let styled_before_clear =
        fs::read_to_string(&styled_visual_path).expect("styled visual before clear");

    let cleared_project = temp.path().join("sales_project_text_cleared");
    let cleared_arg = cleared_project.to_str().expect("cleared project path");
    let clear = run_powerbi(&[
        "report",
        "visuals",
        "format",
        "title",
        "--project",
        styled_arg,
        "--handle",
        &handle,
        "--clear-alt-text",
        "--out-dir",
        cleared_arg,
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    let clear_json = stdout_json(&clear);
    let cleared_visual_path = PathBuf::from(
        clear_json["target"]["path"]
            .as_str()
            .expect("cleared visual path"),
    );
    let cleared_visual_json: Value = serde_json::from_str(
        &fs::read_to_string(cleared_visual_path).expect("cleared visual json"),
    )
    .expect("parse cleared visual json");
    assert!(
        cleared_visual_json
            .pointer("/visual/visualContainerObjects/general/0/properties/altText")
            .is_none()
    );
    assert!(
        cleared_visual_json
            .pointer("/visual/objects/general/0/properties/altText")
            .is_none()
    );
    assert_eq!(
        cleared_visual_json["visual"]["objects"]["general"][0]["properties"]["orientation"]["expr"]
            ["Literal"]["Value"],
        Value::from("'vertical'"),
        "clear must preserve sibling formatting properties"
    );
    assert_eq!(
        fs::read_to_string(&styled_visual_path).expect("styled source after out-dir clear"),
        styled_before_clear,
        "out-dir clear should not mutate the styled source project"
    );
    let cleared_lint = run_powerbi(&["lint", cleared_arg, "--json"]);
    assert_eq!(cleared_lint.code, 0, "stderr: {}", cleared_lint.stderr);
    assert!(
        stdout_json(&cleared_lint)["findings"]
            .as_array()
            .expect("lint findings")
            .iter()
            .all(|finding| !finding["code"]
                .as_str()
                .unwrap_or_default()
                .contains("alt_text")),
        "cleared project should contain no rejected alt-text lint finding"
    );
}

#[test]
fn report_visuals_formatting_set_text_creates_missing_cards_with_page_visual_selector() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual = &visuals_json["visuals"][0];
    let page_handle = visual["page"]["handle"].as_str().expect("page handle");
    let visual_name = visual["name"].as_str().expect("visual name");

    let out_project = temp.path().join("sales_project_created_text");
    let out_arg = out_project.to_str().expect("out project path");
    let update = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-text",
        "--project",
        project_arg,
        "--page",
        page_handle,
        "--visual",
        visual_name,
        "--title",
        "Generated Title",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);
    let update_json = stdout_json(&update);
    assert_eq!(
        update_json["textPlan"]["after"]["title"],
        Value::from("Generated Title")
    );
    assert_eq!(
        update_json["textPlan"]["after"]["showTitle"],
        Value::Bool(true)
    );
    let visual_path = PathBuf::from(
        update_json["target"]["path"]
            .as_str()
            .expect("updated visual path"),
    );
    let visual_json: Value =
        serde_json::from_str(&fs::read_to_string(visual_path).expect("updated visual json"))
            .expect("parse updated visual json");
    assert_eq!(
        visual_json["visual"]["visualContainerObjects"]["title"][0]["properties"]["text"]["expr"]["Literal"]
            ["Value"],
        Value::from("'Generated Title'")
    );
    assert_eq!(
        visual_json["visual"]["visualContainerObjects"]["title"][0]["properties"]["show"]["expr"]["Literal"]
            ["Value"],
        Value::from("true")
    );
    assert!(visual_json["visual"]["objects"].get("title").is_none());
    assert_eq!(
        visual_json["annotations"][0]["value"],
        Value::from("Generated Title")
    );
    assert!(
        visual_json
            .pointer("/visual/visualContainerObjects/general/0/properties/altText")
            .is_none()
    );
}

#[test]
fn report_visuals_formatting_set_text_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let handle = stdout_json(&visuals)["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();

    let no_mode = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-text",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--title",
        "No Mode",
        "--json",
    ]);
    assert_eq!(no_mode.code, 2);
    assert!(
        stderr_json(&no_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--dry-run")
    );

    let no_fields = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-text",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(no_fields.code, 2);
    assert!(
        stderr_json(&no_fields)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --title")
    );

    let unsupported_alt = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-text",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--alt-text",
        "Replacement",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unsupported_alt.code, 2);
    let unsupported_alt_json = stderr_json(&unsupported_alt);
    assert_eq!(
        unsupported_alt_json["error"]["code"],
        Value::from("unsupported_feature")
    );
    assert!(
        unsupported_alt_json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("PBIR_FORMATTING_PROP_UNKNOWN")
    );
    assert!(
        unsupported_alt_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("--clear-alt-text"))
    );
}

#[test]
fn report_visuals_formatting_set_color_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_visual_formatting_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let card = visuals_json["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .expect("card visual");
    let handle = card["handle"].as_str().expect("card handle").to_string();
    let source_path = PathBuf::from(card["path"].as_str().expect("card path"));
    let source_before = fs::read_to_string(&source_path).expect("source visual before");

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--slot",
        "title.fontColor",
        "--color",
        "#abcdef",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.visuals.formatting.colorMutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        dry_json["colorPlan"]["requested"]["slot"],
        "title.fontColor"
    );
    assert_eq!(dry_json["colorPlan"]["requested"]["color"], "#ABCDEF");
    assert_eq!(
        dry_json["colorPlan"]["before"]["titleFontColor"],
        Value::from("#654321")
    );
    assert_eq!(
        dry_json["colorPlan"]["after"]["titleFontColor"],
        Value::from("#ABCDEF")
    );
    assert_eq!(
        dry_json["colorPlan"]["after"]["dataPointFill"],
        Value::from("#123456")
    );
    assert!(
        dry_json["changes"][0]["jsonPointers"]
            .as_array()
            .expect("json pointers")
            .iter()
            .any(|pointer| pointer
                == "/visual/objects/title/0/properties/fontColor/solid/color/expr/Literal/Value")
    );
    assert_eq!(
        fs::read_to_string(&source_path).expect("source visual after dry-run"),
        source_before
    );

    let colored_project = temp.path().join("sales_project_color");
    let colored_arg = colored_project.to_str().expect("colored project path");
    let apply = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--data-point-fill",
        "112233",
        "--out-dir",
        colored_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["mode"], Value::from("out-dir"));
    assert_eq!(apply_json["validation"]["ok"], Value::Bool(true));
    assert_eq!(
        apply_json["colorPlan"]["after"]["dataPointFill"],
        Value::from("#112233")
    );
    assert_eq!(
        fs::read_to_string(&source_path).expect("source visual after out-dir"),
        source_before
    );
    let colored_visual_path = PathBuf::from(
        apply_json["target"]["path"]
            .as_str()
            .expect("colored visual path"),
    );
    let colored_visual_json: Value = serde_json::from_str(
        &fs::read_to_string(&colored_visual_path).expect("colored visual json"),
    )
    .expect("parse colored visual json");
    assert_eq!(
        colored_visual_json["visual"]["objects"]["dataPoint"][0]["properties"]["fill"]["solid"]["color"]
            ["expr"]["Literal"]["Value"],
        Value::from("'#112233'")
    );
    assert_eq!(
        colored_visual_json["visual"]["objects"]["title"][0]["properties"]["fontColor"]["solid"]["color"]
            ["expr"]["Literal"]["Value"],
        Value::from("'#654321'")
    );
}

#[test]
fn report_visuals_formatting_set_color_creates_missing_title_card_with_page_visual_selector() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual = &visuals_json["visuals"][0];
    let page_handle = visual["page"]["handle"].as_str().expect("page handle");
    let visual_name = visual["name"].as_str().expect("visual name");

    let out_project = temp.path().join("sales_project_created_color");
    let out_arg = out_project.to_str().expect("out project path");
    let update = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-colour",
        "--project",
        project_arg,
        "--page",
        page_handle,
        "--visual",
        visual_name,
        "--title-font-colour",
        "445566",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);
    let update_json = stdout_json(&update);
    assert_eq!(
        update_json["colorPlan"]["after"]["titleFontColor"],
        Value::from("#445566")
    );
    let visual_path = PathBuf::from(
        update_json["target"]["path"]
            .as_str()
            .expect("updated visual path"),
    );
    let visual_json: Value =
        serde_json::from_str(&fs::read_to_string(visual_path).expect("updated visual json"))
            .expect("parse updated visual json");
    assert_eq!(
        visual_json["visual"]["objects"]["title"][0]["properties"]["fontColor"]["solid"]["color"]["expr"]
            ["Literal"]["Value"],
        Value::from("'#445566'")
    );
}

#[test]
fn report_visuals_formatting_set_color_creates_numeric_data_view_wildcard() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let handle = stdout_json(&visuals)["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .expect("card visual")["handle"]
        .as_str()
        .expect("card handle")
        .to_string();

    let output = temp.path().join("numeric_wildcard");
    let output_arg = output.to_str().expect("output path");
    let update = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--slot",
        "dataPoint.fill",
        "--color",
        "#AABBCC",
        "--out-dir",
        output_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);
    let update_json = stdout_json(&update);
    let visual_path = PathBuf::from(
        update_json["target"]["path"]
            .as_str()
            .expect("updated visual path"),
    );
    let visual_json: Value =
        serde_json::from_str(&fs::read_to_string(visual_path).expect("updated visual json"))
            .expect("parse updated visual json");
    assert_eq!(
        visual_json["visual"]["objects"]["dataPoint"][0]["selector"]["data"][0]["dataViewWildcard"]
            ["matchingOption"],
        Value::from(0)
    );
}

#[test]
fn report_visuals_formatting_set_color_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_visual_formatting_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let handle = stdout_json(&visuals)["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .expect("card visual")["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();

    let no_mode = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--slot",
        "title.fontColor",
        "--color",
        "#123456",
        "--json",
    ]);
    assert_eq!(no_mode.code, 2);
    assert!(
        stderr_json(&no_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--dry-run")
    );

    let no_fields = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(no_fields.code, 2);
    assert!(
        stderr_json(&no_fields)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --slot")
    );

    let unsupported_slot = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--slot",
        "legend.color",
        "--color",
        "#123456",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unsupported_slot.code, 2);
    assert_unsupported_feature(
        &unsupported_slot.stderr,
        "unsupported visual formatting color slot",
    );

    let bad_color = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--slot",
        "title.fontColor",
        "--color",
        "blue",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(bad_color.code, 2);
    assert!(
        stderr_json(&bad_color)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid color literal")
    );

    patch_json(&first_visual_json_by_type(&project, "card"), |visual| {
        visual["visual"]["objects"]["dataPoint"][0]["selector"] = json!({
            "data": [{ "identityIndex": 0 }]
        });
    });
    let data_bound = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "set-color",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--slot",
        "dataPoint.fill",
        "--color",
        "#010203",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(data_bound.code, 2);
    assert!(
        stderr_json(&data_bound)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("data-bound selectors")
    );
}

#[test]
fn report_pages_mutations_round_trip_through_out_dirs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let pages_path = report_pages_json(&project);
    let source_pages_before = fs::read_to_string(&pages_path).expect("source pages before");

    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let pages_json = stdout_json(&pages);
    let original_handle = pages_json["pages"][0]["handle"]
        .as_str()
        .expect("original page handle")
        .to_string();
    assert_eq!(pages_json["pages"][0]["isActive"], Value::Bool(true));

    let dry_run = run_powerbi(&[
        "report",
        "pages",
        "add",
        "--project",
        project_arg,
        "--display-name",
        "Executive Summary",
        "--width",
        "1366",
        "--height",
        "768",
        "--after",
        &original_handle,
        "--set-active",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.pages.mutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        fs::read_to_string(&pages_path).expect("source pages after dry-run"),
        source_pages_before
    );

    let added = temp.path().join("added_project");
    let added_arg = added.to_str().expect("added path");
    let add = run_powerbi(&[
        "report",
        "pages",
        "add",
        "--project",
        project_arg,
        "--display-name",
        "Executive Summary",
        "--width",
        "1366",
        "--height",
        "768",
        "--after",
        &original_handle,
        "--set-active",
        "--out-dir",
        added_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    assert_eq!(add_json["validation"]["ok"], Value::Bool(true));
    let new_handle = add_json["target"]["handle"]
        .as_str()
        .expect("new page handle")
        .to_string();
    let new_name = add_json["target"]["name"]
        .as_str()
        .expect("new page name")
        .to_string();
    assert_eq!(
        fs::read_to_string(&pages_path).expect("source pages after out-dir add"),
        source_pages_before
    );

    let added_pages = run_powerbi(&["report", "pages", "list", "--project", added_arg, "--json"]);
    assert_eq!(added_pages.code, 0, "stderr: {}", added_pages.stderr);
    let added_pages_json = stdout_json(&added_pages);
    assert_eq!(added_pages_json["counts"]["pages"], Value::from(2));
    let active_added = added_pages_json["pages"]
        .as_array()
        .expect("added pages")
        .iter()
        .find(|page| page["handle"] == new_handle)
        .expect("new page in list");
    assert_eq!(active_added["isActive"], Value::Bool(true));

    let updated = temp.path().join("updated_project");
    let updated_arg = updated.to_str().expect("updated path");
    let update = run_powerbi(&[
        "report",
        "pages",
        "update",
        "--project",
        added_arg,
        "--handle",
        &new_handle,
        "--display-name",
        "Executive Board",
        "--width",
        "1400",
        "--height",
        "800",
        "--display-option",
        "FitToWidth",
        "--out-dir",
        updated_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);
    let show_updated = run_powerbi(&[
        "report",
        "pages",
        "show",
        "--project",
        updated_arg,
        "--handle",
        &new_handle,
        "--json",
    ]);
    assert_eq!(show_updated.code, 0, "stderr: {}", show_updated.stderr);
    let show_updated_json = stdout_json(&show_updated);
    assert_eq!(
        show_updated_json["page"]["displayName"],
        Value::from("Executive Board")
    );
    assert_eq!(show_updated_json["page"]["width"], Value::from(1400.0));
    assert_eq!(
        show_updated_json["page"]["displayOption"],
        Value::from("FitToWidth")
    );

    let reordered = temp.path().join("reordered_project");
    let reordered_arg = reordered.to_str().expect("reordered path");
    let order = format!("{new_handle},{original_handle}");
    let reorder = run_powerbi(&[
        "report",
        "pages",
        "reorder",
        "--project",
        updated_arg,
        "--order",
        &order,
        "--out-dir",
        reordered_arg,
        "--json",
    ]);
    assert_eq!(reorder.code, 0, "stderr: {}", reorder.stderr);
    let reordered_pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        reordered_arg,
        "--json",
    ]);
    assert_eq!(
        reordered_pages.code, 0,
        "stderr: {}",
        reordered_pages.stderr
    );
    let reordered_json = stdout_json(&reordered_pages);
    assert_eq!(
        reordered_json["pages"][0]["handle"],
        Value::from(new_handle.clone())
    );
    assert_eq!(
        reordered_json["pages"][1]["handle"],
        Value::from(original_handle.clone())
    );

    let activated = temp.path().join("activated_project");
    let activated_arg = activated.to_str().expect("activated path");
    let set_active = run_powerbi(&[
        "report",
        "pages",
        "set-active",
        "--project",
        reordered_arg,
        "--handle",
        &original_handle,
        "--out-dir",
        activated_arg,
        "--json",
    ]);
    assert_eq!(set_active.code, 0, "stderr: {}", set_active.stderr);

    let deleted = temp.path().join("deleted_project");
    let deleted_arg = deleted.to_str().expect("deleted path");
    let delete = run_powerbi(&[
        "report",
        "pages",
        "delete-empty",
        "--project",
        activated_arg,
        "--handle",
        &new_handle,
        "--out-dir",
        deleted_arg,
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    let deleted_pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        deleted_arg,
        "--json",
    ]);
    assert_eq!(deleted_pages.code, 0, "stderr: {}", deleted_pages.stderr);
    let deleted_json = stdout_json(&deleted_pages);
    assert_eq!(deleted_json["counts"]["pages"], Value::from(1));
    assert_eq!(
        deleted_json["pages"][0]["handle"],
        Value::from(original_handle)
    );
    assert_eq!(deleted_json["pages"][0]["isActive"], Value::Bool(true));
    assert!(
        !deleted
            .join("SalesOperations.Report")
            .join("definition")
            .join("pages")
            .join(new_name)
            .exists()
    );
}

#[test]
fn report_filters_list_empty_scaffold_returns_zero_filters() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.report.filters.list.v1")
    );
    assert_eq!(value["counts"]["filters"], Value::from(0));
    assert_eq!(value["filters"].as_array().expect("filters").len(), 0);
}

#[test]
fn report_filters_list_and_show_report_page_visual_filters_by_handle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["counts"]["filters"], Value::from(3));
    assert_eq!(value["counts"]["reportFilters"], Value::from(1));
    assert_eq!(value["counts"]["pageFilters"], Value::from(1));
    assert_eq!(value["counts"]["visualFilters"], Value::from(1));
    assert_eq!(value["counts"]["unsupported"], Value::from(1));
    assert!(
        value["filters"]
            .as_array()
            .expect("filters")
            .iter()
            .all(|filter| filter.get("raw").is_none()),
        "list should not include raw filter JSON by default"
    );

    let report_filter = value["filters"]
        .as_array()
        .expect("filters")
        .iter()
        .find(|filter| filter["scope"] == "report")
        .expect("report filter");
    assert_eq!(report_filter["target"]["table"], Value::from("DimRegion"));
    assert_eq!(report_filter["target"]["column"], Value::from("Region"));
    assert_eq!(
        report_filter["safety"]["mayContainDataValues"],
        Value::Bool(true)
    );
    let handle = report_filter["handle"].as_str().expect("filter handle");
    assert_eq!(handle, "filter:report:main:ReportRegionFilter");
    assert_eq!(report_filter["handleIdentity"], Value::from("name"));
    assert_eq!(report_filter["handleAmbiguous"], Value::Bool(false));
    assert_eq!(report_filter["arrayOrigin"], Value::from("filterConfig"));

    let show = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["schema"],
        Value::from("powerbi-cli.report.filters.show.v1")
    );
    assert_eq!(show_json["filter"]["handle"], Value::from(handle));
    assert_eq!(
        show_json["filter"]["raw"]["name"],
        Value::from("ReportRegionFilter")
    );
    assert_eq!(
        show_json["filter"]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
    assert!(
        show_json["readbackCommand"]
            .as_str()
            .expect("readback command")
            .contains("report wireframe export")
    );

    let visual_only = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--scope",
        "visual",
        "--include-raw",
        "--json",
    ]);
    assert_eq!(visual_only.code, 0, "stderr: {}", visual_only.stderr);
    let visual_json = stdout_json(&visual_only);
    assert_eq!(visual_json["counts"]["filters"], Value::from(1));
    assert_eq!(visual_json["filters"][0]["scope"], Value::from("visual"));
    assert_eq!(visual_json["filters"][0]["unsupported"], Value::Bool(true));
    assert_eq!(
        visual_json["filters"][0]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
}

#[test]
fn report_filters_show_rejects_missing_or_unknown_handle_with_suggested_list_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let missing = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(missing.code, 2);
    let missing_json = stderr_json(&missing);
    assert!(
        missing_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report filters list"))
    );

    let unknown = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        project_arg,
        "--handle",
        "filter:report:nope",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    let unknown_json = stderr_json(&unknown);
    assert!(
        unknown_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report filters list"))
    );

    install_filter_fixtures(&project);
    let legacy_ordinal = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        project_arg,
        "--handle",
        "filter:report:0",
        "--json",
    ]);
    assert_eq!(legacy_ordinal.code, 2);
    let legacy_ordinal_json = stderr_json(&legacy_ordinal);
    assert_eq!(
        legacy_ordinal_json["error"]["code"],
        Value::from("invalid_args")
    );
    assert!(
        legacy_ordinal_json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("legacy ordinal filter handle")
    );
    assert!(
        legacy_ordinal_json["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("Re-list filters")
    );
}

#[test]
fn report_filters_add_report_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let before_report = fs::read_to_string(report_json(&project)).expect("report json");

    let dry = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--dry-run",
        "--include-raw",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.filters.addMutation.v1")
    );
    assert_eq!(dry_json["action"], Value::from("add"));
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["mode"], Value::from("dry-run"));
    assert_eq!(dry_json["target"]["scope"], Value::from("report"));
    assert_eq!(
        dry_json["target"]["target"]["table"],
        Value::from("DimCustomer")
    );
    assert_eq!(
        dry_json["target"]["target"]["column"],
        Value::from("Segment")
    );
    assert_eq!(
        dry_json["target"]["safety"]["mayContainDataValues"],
        Value::Bool(true)
    );
    assert_eq!(dry_json["owner"]["kind"], Value::from("report"));
    assert_eq!(dry_json["filterPlan"]["beforeCount"], Value::from(0));
    assert_eq!(dry_json["filterPlan"]["afterCount"], Value::from(1));
    assert_eq!(
        dry_json["changes"][0]["jsonPointer"],
        Value::from("/filterConfig/filters/0")
    );
    assert_eq!(
        dry_json["changes"][0]["after"]["name"],
        Value::from("PowerBICliReportDimSegmCatIf74b6f21C19a017e7Filter")
    );
    assert_eq!(
        dry_json["target"]["handle"],
        Value::from("filter:report:main:PowerBICliReportDimSegmCatIf74b6f21C19a017e7Filter")
    );
    assert_eq!(dry_json["target"]["handleIdentity"], "name");
    assert_eq!(dry_json["target"]["arrayOrigin"], "filterConfig");
    assert!(
        dry_json["rawReviewCommand"]
            .as_str()
            .expect("raw review command")
            .contains("--include-raw")
    );
    assert!(dry_json["filterReadbackCommand"].is_null());
    assert!(
        !dry_json["next"]
            .as_array()
            .expect("next commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report filters show")),
        "dry-run must not return a show command for an unwritten planned filter"
    );
    assert_eq!(
        fs::read_to_string(report_json(&project)).expect("report json"),
        before_report,
        "dry-run must not mutate the source report"
    );

    let out_dir = temp.path().join("sales_project_filter_added");
    let out_arg = out_dir.to_str().expect("out dir");
    let add = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    assert_eq!(add_json["ok"], Value::Bool(true));
    assert_eq!(add_json["mode"], Value::from("out-dir"));
    assert_eq!(add_json["validation"]["ok"], Value::Bool(true));
    assert!(add_json["rawReviewCommand"].is_null());
    assert!(
        add_json["filterReadbackCommand"]
            .as_str()
            .expect("filter readback command")
            .contains("report filters show")
    );

    let after = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        out_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(after.code, 0, "stderr: {}", after.stderr);
    let after_json = stdout_json(&after);
    assert_eq!(after_json["counts"]["filters"], Value::from(1));
    assert_eq!(after_json["counts"]["reportFilters"], Value::from(1));
    assert_eq!(
        after_json["filters"][0]["target"]["table"],
        Value::from("DimCustomer")
    );
    assert_eq!(
        after_json["filters"][0]["target"]["column"],
        Value::from("Segment")
    );
    assert!(after_json["filters"][0].get("raw").is_none());
    assert_eq!(
        after_json["filters"][0]["safety"]["mayContainDataValues"],
        Value::Bool(true)
    );

    let handle = after_json["filters"][0]["handle"]
        .as_str()
        .expect("filter handle")
        .to_string();
    let show = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        out_arg,
        "--handle",
        &handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["filter"]["raw"]["filter"]["Where"][0]["Condition"]["In"]["Values"][0][0]["Literal"]
            ["Value"],
        Value::from("'Enterprise'")
    );

    let original = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(original.code, 0, "stderr: {}", original.stderr);
    assert_eq!(stdout_json(&original)["counts"]["filters"], Value::from(0));
}

#[test]
fn report_filters_add_supports_page_and_visual_selectors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(&project);

    let page_filter = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--page",
        &page,
        "--table",
        "DimDate",
        "--column",
        "FiscalYear",
        "--value-json",
        "2026",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(page_filter.code, 0, "stderr: {}", page_filter.stderr);
    let page_json = stdout_json(&page_filter);
    assert_eq!(page_json["target"]["scope"], Value::from("page"));
    assert_eq!(page_json["owner"]["kind"], Value::from("page"));
    assert_eq!(
        page_json["target"]["target"]["table"],
        Value::from("DimDate")
    );
    assert_eq!(
        page_json["target"]["target"]["column"],
        Value::from("FiscalYear")
    );
    assert_eq!(page_json["filterPlan"]["afterCount"], Value::from(1));
    assert!(
        page_json["readbackCommand"]
            .as_str()
            .expect("readback command")
            .contains("--scope page")
    );

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();

    let visual_filter = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--visual",
        &visual_handle,
        "--target",
        "FactSales.Units",
        "--value-json",
        "42",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(visual_filter.code, 0, "stderr: {}", visual_filter.stderr);
    let visual_json = stdout_json(&visual_filter);
    assert_eq!(visual_json["target"]["scope"], Value::from("visual"));
    assert_eq!(visual_json["owner"]["kind"], Value::from("visual"));
    assert_eq!(visual_json["owner"]["handle"], Value::from(visual_handle));
    assert_eq!(
        visual_json["target"]["target"]["table"],
        Value::from("FactSales")
    );
    assert_eq!(
        visual_json["target"]["target"]["column"],
        Value::from("Units")
    );
    assert!(
        visual_json["ownerReadbackCommand"]
            .as_str()
            .expect("owner readback command")
            .contains("report visuals show")
    );
}

#[test]
fn report_filters_add_rejects_unsafe_or_invalid_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let missing_mode = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --dry-run")
    );

    let missing_value = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer[Segment]",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_value.code, 2);
    assert!(
        stderr_json(&missing_value)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires at least one")
    );

    let bad_target = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer",
        "--value",
        "Enterprise",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(bad_target.code, 2);
    assert_error(&bad_target, "invalid_args", "invalid filter target syntax");

    let unknown_target = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "MissingTable[Segment]",
        "--value",
        "Enterprise",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unknown_target.code, 10);
    let unknown_json = stderr_json(&unknown_target);
    assert_eq!(
        unknown_json["error"]["code"],
        Value::from("validation_failed")
    );
    assert!(
        unknown_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("inspect --deep"))
    );

    let scope_all = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "all",
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(scope_all.code, 2);
    assert!(
        stderr_json(&scope_all)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("cannot use --scope all")
    );

    let missing_page = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "page",
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_page.code, 2);
    assert!(
        stderr_json(&missing_page)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --page")
    );

    let mixed_target = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer[Segment]",
        "--table",
        "DimCustomer",
        "--value",
        "Enterprise",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(mixed_target.code, 2);
    assert!(
        stderr_json(&mixed_target)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("either --target or --table plus --column")
    );

    let invalid_name = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--name",
        "Bad Name",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(invalid_name.code, 2);
    assert!(
        stderr_json(&invalid_name)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--name must be non-empty")
    );

    let nested_value = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer[Segment]",
        "--values-json",
        "[{}]",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(nested_value.code, 2);
    assert!(
        stderr_json(&nested_value)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("supports only scalar non-null")
    );

    let wrong_type = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "FactSales[Units]",
        "--value",
        "forty-two",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(wrong_type.code, 2);
    assert!(
        stderr_json(&wrong_type)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("is not compatible")
    );

    let duplicate_dir = temp.path().join("sales_project_filter_duplicate_base");
    let duplicate_arg = duplicate_dir.to_str().expect("duplicate path");
    let first_add = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--out-dir",
        duplicate_arg,
        "--json",
    ]);
    assert_eq!(first_add.code, 0, "stderr: {}", first_add.stderr);
    let second_dir = temp.path().join("sales_project_filter_second_condition");
    let second_arg = second_dir.to_str().expect("second filter path");
    let second_add = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        duplicate_arg,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "SMB",
        "--out-dir",
        second_arg,
        "--json",
    ]);
    assert_eq!(second_add.code, 0, "stderr: {}", second_add.stderr);
    let first_name = stdout_json(&first_add)["target"]["name"]
        .as_str()
        .expect("first generated name")
        .to_string();
    let second_name = stdout_json(&second_add)["target"]["name"]
        .as_str()
        .expect("second generated name")
        .to_string();
    assert_ne!(first_name, second_name);
    assert!(first_name.contains("If74b6f21C19a017e7"));
    assert!(second_name.contains("If74b6f21C00a05e45"));
    assert!(first_name.len() <= 50);
    assert!(second_name.len() <= 50);

    let listed = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        second_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    let listed_json = stdout_json(&listed);
    assert_eq!(listed_json["counts"]["filters"], Value::from(2));
    assert_ne!(
        listed_json["filters"][0]["handle"],
        listed_json["filters"][1]["handle"]
    );

    let duplicate = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        second_arg,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "SMB",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(duplicate.code, 2);
    assert!(
        stderr_json(&duplicate)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("filter name already exists")
    );
}

#[test]
fn report_filters_numeric_range_full_lifecycle_all_scopes() {
    let temp = tempfile::tempdir().expect("tempdir");

    exercise_authored_filter_lifecycle(
        &temp.path().join("report_range"),
        "report",
        "FactSales[Revenue]",
        &[
            "--condition-type",
            "range",
            "--min",
            "1000",
            "--max",
            "5000",
        ],
        "Advanced",
        "Revenue from 1k to 5k",
        |filter| assert_numeric_range_shape(filter, &[(2, "1000L"), (4, "5000L")]),
    );
    exercise_authored_filter_lifecycle(
        &temp.path().join("page_range"),
        "page",
        "FactSales[Revenue]",
        &["--min", "1250.5"],
        "Advanced",
        "Revenue at least 1250.5",
        |filter| assert_numeric_range_shape(filter, &[(2, "1250.5D")]),
    );
    exercise_authored_filter_lifecycle(
        &temp.path().join("visual_range"),
        "visual",
        "FactSales[Units]",
        &["--max", "42"],
        "Advanced",
        "Units at most 42",
        |filter| assert_numeric_range_shape(filter, &[(4, "42L")]),
    );
}

#[test]
fn report_filters_topn_full_lifecycle_visual_scope() {
    let temp = tempfile::tempdir().expect("tempdir");

    exercise_authored_filter_lifecycle(
        &temp.path().join("top"),
        "visual",
        "DimCustomer[CustomerName]",
        &["--top", "5", "--by", "Total Revenue"],
        "TopN",
        "Top five customers",
        |filter| assert_topn_shape(filter, 2),
    );
    exercise_authored_filter_lifecycle(
        &temp.path().join("bottom"),
        "visual",
        "DimCustomer[CustomerName]",
        &["--bottom", "5", "--by", "FactSales[Total Revenue]"],
        "TopN",
        "Bottom five customers",
        |filter| assert_topn_shape(filter, 1),
    );
}

#[test]
fn report_filters_relative_date_full_lifecycle_all_scopes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let now = || json!({ "Now": {} });

    let report_lower = json!({
        "DateSpan": {
            "Expression": {
                "DateAdd": {
                    "Expression": {
                        "DateAdd": { "Expression": now(), "Amount": 1, "TimeUnit": 0 }
                    },
                    "Amount": -12,
                    "TimeUnit": 2
                }
            },
            "TimeUnit": 0
        }
    });
    let report_upper = json!({ "DateSpan": { "Expression": now(), "TimeUnit": 0 } });
    exercise_authored_filter_lifecycle(
        &temp.path().join("report_relative"),
        "report",
        "DimDate[Date]",
        &["--relative", "last", "--unit", "months", "--span", "12"],
        "RelativeDate",
        "Last 12 months",
        |filter| assert_relative_date_shape(filter, &report_lower, &report_upper),
    );

    let page_lower = json!({ "DateSpan": { "Expression": now(), "TimeUnit": 0 } });
    let page_upper = json!({
        "DateSpan": {
            "Expression": {
                "DateAdd": {
                    "Expression": {
                        "DateAdd": { "Expression": now(), "Amount": -1, "TimeUnit": 0 }
                    },
                    "Amount": 7,
                    "TimeUnit": 0
                }
            },
            "TimeUnit": 0
        }
    });
    exercise_authored_filter_lifecycle(
        &temp.path().join("page_relative"),
        "page",
        "DimDate[Date]",
        &["--relative", "next", "--unit", "days", "--span", "7"],
        "RelativeDate",
        "Next seven days",
        |filter| assert_relative_date_shape(filter, &page_lower, &page_upper),
    );

    let visual_lower = json!({
        "DateSpan": { "Expression": now(), "TimeUnit": 3 }
    });
    let visual_upper = json!({
        "DateSpan": {
            "Expression": {
                "DateAdd": {
                    "Expression": {
                        "DateAdd": {
                            "Expression": visual_lower.clone(),
                            "Amount": 1,
                            "TimeUnit": 3
                        }
                    },
                    "Amount": -1,
                    "TimeUnit": 0
                }
            },
            "TimeUnit": 0
        }
    });
    exercise_authored_filter_lifecycle(
        &temp.path().join("visual_relative"),
        "visual",
        "DimDate[Date]",
        &[
            "--relative",
            "this",
            "--unit",
            "calendar-years",
            "--span",
            "1",
        ],
        "RelativeDate",
        "This calendar year",
        |filter| assert_relative_date_shape(filter, &visual_lower, &visual_upper),
    );
}

#[test]
fn report_filters_update_categorical_values_full_lifecycle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let added = temp.path().join("categorical_added");
    let added_arg = added.to_str().expect("added path");
    let add = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--out-dir",
        added_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "add stderr: {}", add.stderr);
    assert_strict_valid(&added);

    let list = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        added_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(list.code, 0, "list stderr: {}", list.stderr);
    let handle = stdout_json(&list)["filters"][0]["handle"]
        .as_str()
        .expect("filter handle")
        .to_string();

    let dry = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        added_arg,
        "--handle",
        &handle,
        "--condition-type",
        "categorical",
        "--values-json",
        "[\"SMB\",\"Mid-Market\"]",
        "--display-name",
        "Customer segment",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "update dry-run stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(dry_json["filterPlan"]["rawIncluded"], Value::Bool(true));
    assert_eq!(
        dry_json["filterPlan"]["before"]["filter"]["Where"][0]["Condition"]["In"]["Values"][0][0]["Literal"]
            ["Value"],
        Value::from("'Enterprise'")
    );
    assert_eq!(
        dry_json["filterPlan"]["after"]["filter"]["Where"][0]["Condition"]["In"]["Values"][1][0]["Literal"]
            ["Value"],
        Value::from("'Mid-Market'")
    );

    let unchanged = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        added_arg,
        "--handle",
        &handle,
        "--json",
    ]);
    assert_eq!(
        unchanged.code, 0,
        "unchanged show stderr: {}",
        unchanged.stderr
    );
    assert_eq!(
        stdout_json(&unchanged)["filter"]["raw"]["filter"]["Where"][0]["Condition"]["In"]["Values"]
            [0][0]["Literal"]["Value"],
        Value::from("'Enterprise'")
    );

    let updated = temp.path().join("categorical_updated");
    let updated_arg = updated.to_str().expect("updated path");
    let update = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        added_arg,
        "--handle",
        &handle,
        "--values-json",
        "[\"SMB\",\"Mid-Market\"]",
        "--display-name",
        "Customer segment",
        "--out-dir",
        updated_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "update stderr: {}", update.stderr);
    assert_strict_valid(&updated);
    let show = run_powerbi(&[
        "report",
        "filters",
        "show",
        "--project",
        updated_arg,
        "--handle",
        &handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "show stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["filter"]["displayName"],
        Value::from("Customer segment")
    );
    assert_eq!(
        show_json["filter"]["raw"]["filter"]["Where"][0]["Condition"]["In"]["Values"]
            .as_array()
            .expect("updated values")
            .len(),
        2
    );

    let deleted = temp.path().join("categorical_deleted");
    let deleted_arg = deleted.to_str().expect("deleted path");
    let delete = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        updated_arg,
        "--handle",
        &handle,
        "--out-dir",
        deleted_arg,
        "--json",
    ]);
    assert_eq!(delete.code, 0, "delete stderr: {}", delete.stderr);
    assert_strict_valid(&deleted);
}

#[test]
fn report_filters_numeric_range_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let missing_mode = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "FactSales[Revenue]",
        "--min",
        "100",
        "--json",
    ]);
    assert_error(&missing_mode, "invalid_args", "requires --dry-run");

    let wrong_column_type = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimCustomer[Segment]",
        "--min",
        "100",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &wrong_column_type,
        "invalid_args",
        "must have a numeric TMDL dataType",
    );

    let bad_number = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "FactSales[Revenue]",
        "--min",
        "nope",
        "--dry-run",
        "--json",
    ]);
    assert_error(&bad_number, "invalid_args", "parse --min");

    let reversed = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "FactSales[Revenue]",
        "--min",
        "500",
        "--max",
        "100",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &reversed,
        "invalid_args",
        "--min must be less than or equal",
    );

    let long_name = "R".repeat(51);
    let long_name_args = vec![
        "report".to_string(),
        "filters".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project_arg.to_string(),
        "--target".to_string(),
        "FactSales[Revenue]".to_string(),
        "--max".to_string(),
        "500".to_string(),
        "--name".to_string(),
        long_name,
        "--dry-run".to_string(),
        "--json".to_string(),
    ];
    assert_error(
        &run_powerbi_owned(&long_name_args),
        "invalid_args",
        "50 characters or fewer",
    );

    let unknown_flag = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "FactSales[Revenue]",
        "--min",
        "100",
        "--mystery-range",
        "true",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &unknown_flag,
        "invalid_args",
        "unknown report filters add flag",
    );
}

#[test]
fn report_filters_topn_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visual = first_visual_handle(&project);

    let missing_mode = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--visual",
        &visual,
        "--target",
        "DimCustomer[CustomerName]",
        "--top",
        "5",
        "--by",
        "Total Revenue",
        "--json",
    ]);
    assert_error(&missing_mode, "invalid_args", "requires --dry-run");

    let wrong_reference_type = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--visual",
        &visual,
        "--target",
        "DimCustomer[CustomerName]",
        "--top",
        "5",
        "--by",
        "FactSales[Revenue]",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &wrong_reference_type,
        "validation_failed",
        "measure not found for TopN --by",
    );

    let zero = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--visual",
        &visual,
        "--target",
        "DimCustomer[CustomerName]",
        "--top",
        "0",
        "--by",
        "Total Revenue",
        "--dry-run",
        "--json",
    ]);
    assert_error(&zero, "invalid_args", "--top must be between 1");

    let wrong_scope = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--target",
        "DimCustomer[CustomerName]",
        "--top",
        "5",
        "--by",
        "Total Revenue",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &wrong_scope,
        "unsupported_feature",
        "supported only for visual-owned",
    );

    let long_name = "T".repeat(51);
    let long_name_args = vec![
        "report".to_string(),
        "filters".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project_arg.to_string(),
        "--visual".to_string(),
        visual.clone(),
        "--target".to_string(),
        "DimCustomer[CustomerName]".to_string(),
        "--top".to_string(),
        "5".to_string(),
        "--by".to_string(),
        "Total Revenue".to_string(),
        "--name".to_string(),
        long_name,
        "--dry-run".to_string(),
        "--json".to_string(),
    ];
    assert_error(
        &run_powerbi_owned(&long_name_args),
        "invalid_args",
        "50 characters or fewer",
    );

    let unknown_flag = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--visual",
        &visual,
        "--target",
        "DimCustomer[CustomerName]",
        "--top",
        "5",
        "--by",
        "Total Revenue",
        "--rank-mode",
        "dense",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &unknown_flag,
        "invalid_args",
        "unknown report filters add flag",
    );
}

#[test]
fn report_filters_relative_date_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let missing_mode = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimDate[Date]",
        "--relative",
        "last",
        "--unit",
        "months",
        "--span",
        "12",
        "--json",
    ]);
    assert_error(&missing_mode, "invalid_args", "requires --dry-run");

    let wrong_column_type = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "FactSales[Revenue]",
        "--relative",
        "last",
        "--unit",
        "months",
        "--span",
        "12",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &wrong_column_type,
        "invalid_args",
        "must have a date-typed TMDL dataType",
    );

    let zero_span = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimDate[Date]",
        "--relative",
        "last",
        "--unit",
        "months",
        "--span",
        "0",
        "--dry-run",
        "--json",
    ]);
    assert_error(&zero_span, "invalid_args", "--span must be between 1");

    let bad_unit = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimDate[Date]",
        "--relative",
        "last",
        "--unit",
        "fortnights",
        "--span",
        "2",
        "--dry-run",
        "--json",
    ]);
    assert_unsupported_feature(&bad_unit.stderr, "unsupported relative-date unit");

    let bad_operator = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimDate[Date]",
        "--relative",
        "previous",
        "--unit",
        "months",
        "--span",
        "2",
        "--dry-run",
        "--json",
    ]);
    assert_unsupported_feature(&bad_operator.stderr, "unsupported --relative operator");

    let bad_this_span = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimDate[Date]",
        "--relative",
        "this",
        "--unit",
        "calendar-years",
        "--span",
        "2",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &bad_this_span,
        "invalid_args",
        "--relative this requires --span 1",
    );

    let long_name = "D".repeat(51);
    let long_name_args = vec![
        "report".to_string(),
        "filters".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project_arg.to_string(),
        "--target".to_string(),
        "DimDate[Date]".to_string(),
        "--relative".to_string(),
        "next".to_string(),
        "--unit".to_string(),
        "years".to_string(),
        "--span".to_string(),
        "1".to_string(),
        "--name".to_string(),
        long_name,
        "--dry-run".to_string(),
        "--json".to_string(),
    ];
    assert_error(
        &run_powerbi_owned(&long_name_args),
        "invalid_args",
        "50 characters or fewer",
    );

    let unknown_flag = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--target",
        "DimDate[Date]",
        "--relative",
        "last",
        "--unit",
        "months",
        "--span",
        "12",
        "--timezone",
        "UTC",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &unknown_flag,
        "invalid_args",
        "unknown report filters add flag",
    );
}

#[test]
fn report_filters_update_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");
    let list = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "list stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let report_handle = list_json["filters"]
        .as_array()
        .expect("filters")
        .iter()
        .find(|filter| filter["scope"] == "report")
        .and_then(|filter| filter["handle"].as_str())
        .expect("report filter handle")
        .to_string();
    let page_handle = list_json["filters"]
        .as_array()
        .expect("filters")
        .iter()
        .find(|filter| filter["scope"] == "page")
        .and_then(|filter| filter["handle"].as_str())
        .expect("page filter handle")
        .to_string();

    let retry_out = temp.path().join("filter-update-retry");
    let retry_out_arg = retry_out.to_str().expect("retry output path");
    let invalid_out_dir = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        "filter:missing",
        "--display-name",
        "Changed",
        "--out-dir",
        retry_out_arg,
        "--json",
    ]);
    assert_error(&invalid_out_dir, "invalid_args", "filter not found");
    assert!(
        !retry_out.exists(),
        "invalid source plan must not materialize --out-dir"
    );

    let retry = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        &report_handle,
        "--display-name",
        "Changed",
        "--out-dir",
        retry_out_arg,
        "--json",
    ]);
    assert_eq!(retry.code, 0, "retry stderr: {}", retry.stderr);
    assert!(retry_out.is_dir(), "valid retry must create --out-dir");

    let missing_mode = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        &report_handle,
        "--display-name",
        "Changed",
        "--json",
    ]);
    assert_error(&missing_mode, "invalid_args", "requires --dry-run");

    let type_change = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        &report_handle,
        "--condition-type",
        "range",
        "--display-name",
        "Changed",
        "--dry-run",
        "--json",
    ]);
    assert_error(&type_change, "unsupported_feature", "refuses type change");

    let range_values = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        &page_handle,
        "--value-json",
        "2000",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &range_values,
        "unsupported_feature",
        "cannot replace values on Advanced filters",
    );

    let condition_edit = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        &page_handle,
        "--min",
        "2000",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &condition_edit,
        "unsupported_feature",
        "does not change filter conditions with --min",
    );

    let empty_values = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        &report_handle,
        "--values-json",
        "[]",
        "--dry-run",
        "--json",
    ]);
    assert_error(&empty_values, "invalid_args", "must not be empty");

    let unknown_flag = run_powerbi(&[
        "report",
        "filters",
        "update",
        "--project",
        project_arg,
        "--handle",
        &report_handle,
        "--display-name",
        "Changed",
        "--rename-type",
        "no",
        "--dry-run",
        "--json",
    ]);
    assert_error(
        &unknown_flag,
        "invalid_args",
        "unknown report filters update flag",
    );
}

#[test]
fn report_filters_delete_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");

    let list = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let handle = list_json["filters"]
        .as_array()
        .expect("filters")
        .iter()
        .find(|filter| filter["scope"] == "page")
        .expect("page filter")["handle"]
        .as_str()
        .expect("filter handle")
        .to_string();

    let dry = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.filters.deleteMutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["action"], Value::from("delete"));
    assert_eq!(dry_json["target"]["handle"], Value::from(handle.clone()));
    assert!(dry_json["target"].get("raw").is_none());
    assert!(dry_json["changes"][0]["before"].get("raw").is_none());
    assert_eq!(
        dry_json["filterPlan"]["rawBeforeIncluded"],
        Value::Bool(false)
    );
    assert_eq!(dry_json["filterPlan"]["arrayBeforeCount"], Value::from(1));
    assert_eq!(dry_json["filterPlan"]["arrayAfterCount"], Value::from(0));
    assert!(dry_json["changes"][0]["after"].is_null());
    assert!(
        dry_json["readbackCommand"]
            .as_str()
            .expect("readback command")
            .contains("--scope page")
    );
    assert!(
        dry_json["rawReviewCommand"]
            .as_str()
            .expect("raw review command")
            .contains("--include-raw")
    );

    let dry_raw = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--dry-run",
        "--include-raw",
        "--json",
    ]);
    assert_eq!(dry_raw.code, 0, "stderr: {}", dry_raw.stderr);
    let dry_raw_json = stdout_json(&dry_raw);
    assert_eq!(
        dry_raw_json["filterPlan"]["rawBeforeIncluded"],
        Value::Bool(true)
    );
    assert_eq!(
        dry_raw_json["target"]["raw"]["name"],
        Value::from("PageRevenueFilter")
    );

    let out_dir = temp.path().join("sales_project_filter_deleted");
    let out_arg = out_dir.to_str().expect("out dir");
    let delete = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    let delete_json = stdout_json(&delete);
    assert_eq!(delete_json["ok"], Value::Bool(true));
    assert_eq!(delete_json["mode"], Value::from("out-dir"));
    assert_eq!(delete_json["validation"]["ok"], Value::Bool(true));
    assert!(delete_json["rawReviewCommand"].is_null());

    let after = run_powerbi(&["report", "filters", "list", "--project", out_arg, "--json"]);
    assert_eq!(after.code, 0, "stderr: {}", after.stderr);
    let after_json = stdout_json(&after);
    assert_eq!(after_json["counts"]["filters"], Value::from(2));
    assert!(
        !after_json["filters"]
            .as_array()
            .expect("filters")
            .iter()
            .any(|filter| filter["handle"] == handle)
    );

    let original = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(original.code, 0, "stderr: {}", original.stderr);
    assert_eq!(stdout_json(&original)["counts"]["filters"], Value::from(3));
}

#[test]
fn report_filters_delete_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");
    let listed = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    let listed_json = stdout_json(&listed);
    let handle_owned = listed_json["filters"][0]["handle"]
        .as_str()
        .expect("report filter handle")
        .to_string();
    let handle = handle_owned.as_str();

    let missing_mode = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --dry-run")
    );

    let missing_confirm = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--in-place",
        "--json",
    ]);
    assert_eq!(missing_confirm.code, 2);
    assert!(
        stderr_json(&missing_confirm)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--confirm")
    );

    let unknown = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "filter:report:nope",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    assert!(
        stderr_json(&unknown)["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report filters list"))
    );

    let old_ordinal = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "filter:report:0",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(old_ordinal.code, 2);
    assert!(
        stderr_json(&old_ordinal)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("legacy ordinal filter handle")
    );

    patch_json(&report_json(&project), |report| {
        report["filters"] = json!([{
            "name": "ReportRegionFilter",
            "type": "Categorical",
            "field": {
                "Column": {
                    "Expression": { "SourceRef": { "Entity": "DimRegion" } },
                    "Property": "Region"
                }
            }
        }]);
    });
    let origins = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(origins.code, 0, "stderr: {}", origins.stderr);
    let origins_json = stdout_json(&origins);
    assert_eq!(origins_json["counts"]["filters"], Value::from(2));
    assert_eq!(
        origins_json["filters"][0]["handle"],
        Value::from("filter:report:main:ReportRegionFilter")
    );
    assert_eq!(
        origins_json["filters"][1]["handle"],
        Value::from("filter:report:main:ReportRegionFilter#legacy")
    );
    assert_eq!(origins_json["filters"][1]["arrayOrigin"], "legacy");

    let legacy_delete = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        origins_json["filters"][1]["handle"]
            .as_str()
            .expect("legacy handle"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(legacy_delete.code, 0, "stderr: {}", legacy_delete.stderr);
    assert_eq!(
        stdout_json(&legacy_delete)["target"]["arrayOrigin"],
        "legacy"
    );
}

#[test]
fn report_filter_name_handles_survive_earlier_deletion_without_retargeting() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    patch_json(&report_json(&project), |report| {
        report["filterConfig"]["filters"] = json!([
            categorical_filter_fixture(
                "FirstRegionFilter",
                "DimRegion",
                "Region",
                vec![Value::from("North")],
            ),
            categorical_filter_fixture(
                "SecondRegionFilter",
                "DimRegion",
                "Region",
                vec![Value::from("South")],
            )
        ]);
    });
    let project_arg = project.to_str().expect("project path");

    let before = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(before.code, 0, "stderr: {}", before.stderr);
    let before_json = stdout_json(&before);
    let first_handle = before_json["filters"][0]["handle"]
        .as_str()
        .expect("first handle")
        .to_string();
    let cached_second_handle = before_json["filters"][1]["handle"]
        .as_str()
        .expect("second handle")
        .to_string();
    assert_eq!(
        cached_second_handle,
        "filter:report:main:SecondRegionFilter"
    );

    let after_first_dir = temp.path().join("after_first_filter_delete");
    let after_first_arg = after_first_dir.to_str().expect("after first path");
    let delete_first = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &first_handle,
        "--out-dir",
        after_first_arg,
        "--json",
    ]);
    assert_eq!(delete_first.code, 0, "stderr: {}", delete_first.stderr);

    let after = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        after_first_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(after.code, 0, "stderr: {}", after.stderr);
    let after_json = stdout_json(&after);
    assert_eq!(after_json["counts"]["filters"], Value::from(1));
    assert_eq!(
        after_json["filters"][0]["handle"],
        Value::from(cached_second_handle.clone())
    );
    assert_eq!(after_json["filters"][0]["ordinal"], Value::from(0));

    let delete_cached_second = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        after_first_arg,
        "--handle",
        &cached_second_handle,
        "--dry-run",
        "--include-raw",
        "--json",
    ]);
    assert_eq!(
        delete_cached_second.code, 0,
        "stderr: {}",
        delete_cached_second.stderr
    );
    assert_eq!(
        stdout_json(&delete_cached_second)["target"]["raw"]["name"],
        "SecondRegionFilter"
    );

    let stale_ordinal = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        after_first_arg,
        "--handle",
        "filter:report:1",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(stale_ordinal.code, 2);
    assert!(
        stderr_json(&stale_ordinal)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("legacy ordinal filter handle")
    );
}

#[test]
fn report_filter_duplicate_identities_are_unique_but_mutation_ambiguous() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    patch_json(&report_json(&project), |report| {
        report["filterConfig"]["filters"] = json!([
            categorical_filter_fixture(
                "DuplicateRegionFilter",
                "DimRegion",
                "Region",
                vec![Value::from("North")],
            ),
            categorical_filter_fixture(
                "DuplicateRegionFilter",
                "DimRegion",
                "Region",
                vec![Value::from("South")],
            )
        ]);
    });
    let project_arg = project.to_str().expect("project path");

    let listed = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    let listed_json = stdout_json(&listed);
    assert_eq!(
        listed_json["filters"][0]["handle"],
        "filter:report:main:DuplicateRegionFilter~1"
    );
    assert_eq!(
        listed_json["filters"][1]["handle"],
        "filter:report:main:DuplicateRegionFilter~2"
    );
    assert_eq!(listed_json["filters"][0]["handleAmbiguous"], true);
    assert_eq!(listed_json["filters"][1]["handleAmbiguous"], true);

    let ambiguous = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        listed_json["filters"][0]["handle"]
            .as_str()
            .expect("ambiguous handle"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(ambiguous.code, 2);
    let error = stderr_json(&ambiguous);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ambiguous and cannot be mutated safely")
    );
    assert!(
        error["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("unique names")
    );
}

#[test]
fn report_filter_nameless_entries_use_fingerprint_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    patch_json(&report_json(&project), |report| {
        let mut filter = categorical_filter_fixture(
            "TemporaryName",
            "DimRegion",
            "Region",
            vec![Value::from("North")],
        );
        filter
            .as_object_mut()
            .expect("filter object")
            .remove("name");
        report["filters"] = json!([filter]);
    });
    let project_arg = project.to_str().expect("project path");

    let listed = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    let listed_json = stdout_json(&listed);
    let filter = &listed_json["filters"][0];
    let handle = filter["handle"].as_str().expect("fingerprint handle");
    assert!(handle.starts_with("filter:report:main:@"));
    assert!(handle.ends_with("#legacy"));
    assert_eq!(filter["handleIdentity"], "fingerprint");
    assert_eq!(filter["arrayOrigin"], "legacy");
    assert_eq!(filter["handleAmbiguous"], false);

    let delete = run_powerbi(&[
        "report",
        "filters",
        "delete",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    assert_eq!(stdout_json(&delete)["target"]["handle"], handle);
}

#[test]
fn report_filters_clear_page_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(&project);

    let dry = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--page",
        &page,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.filters.clearMutation.v1")
    );
    assert_eq!(dry_json["action"], Value::from("clear"));
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["selector"]["kind"], Value::from("page"));
    assert_eq!(dry_json["counts"]["matchedFilters"], Value::from(1));
    assert_eq!(dry_json["counts"]["pageFilters"], Value::from(1));
    assert_eq!(dry_json["counts"]["visualFilters"], Value::from(0));
    assert_eq!(
        dry_json["filterPlan"]["rawBeforeIncluded"],
        Value::Bool(false)
    );
    assert!(dry_json["targets"][0].get("raw").is_none());
    assert!(dry_json["changes"][0]["after"].is_null());
    assert!(
        dry_json["readbackCommand"]
            .as_str()
            .expect("readback command")
            .contains("--scope page")
    );
    assert!(
        dry_json["rawReviewCommand"]
            .as_str()
            .expect("raw review command")
            .contains("--include-raw")
    );

    let out_dir = temp.path().join("sales_project_filters_page_cleared");
    let out_arg = out_dir.to_str().expect("out dir");
    let clear = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--page",
        &page,
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    let clear_json = stdout_json(&clear);
    assert_eq!(clear_json["ok"], Value::Bool(true));
    assert_eq!(clear_json["mode"], Value::from("out-dir"));
    assert_eq!(clear_json["validation"]["ok"], Value::Bool(true));
    assert!(clear_json["rawReviewCommand"].is_null());

    let after = run_powerbi(&["report", "filters", "list", "--project", out_arg, "--json"]);
    assert_eq!(after.code, 0, "stderr: {}", after.stderr);
    let after_json = stdout_json(&after);
    assert_eq!(after_json["counts"]["filters"], Value::from(2));
    assert_eq!(after_json["counts"]["reportFilters"], Value::from(1));
    assert_eq!(after_json["counts"]["pageFilters"], Value::from(0));
    assert_eq!(after_json["counts"]["visualFilters"], Value::from(1));

    let original = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(original.code, 0, "stderr: {}", original.stderr);
    assert_eq!(stdout_json(&original)["counts"]["filters"], Value::from(3));
}

#[test]
fn report_filters_clear_visual_supports_full_handle_and_page_visual_selector() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");

    let list = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        project_arg,
        "--scope",
        "visual",
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let visual_handle = list_json["filters"][0]["visual"]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let page_handle = list_json["filters"][0]["page"]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();
    let visual_name = list_json["filters"][0]["visual"]["name"]
        .as_str()
        .expect("visual name")
        .to_string();

    let dry = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--visual",
        &visual_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(dry_json["selector"]["kind"], Value::from("visual"));
    assert_eq!(
        dry_json["selector"]["visualHandle"],
        Value::from(visual_handle.clone())
    );
    assert_eq!(dry_json["counts"]["visualFilters"], Value::from(1));

    let named = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual",
        &visual_name,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(named.code, 0, "stderr: {}", named.stderr);
    let named_json = stdout_json(&named);
    assert_eq!(
        named_json["selector"]["visualHandle"],
        Value::from(visual_handle)
    );
    assert_eq!(named_json["counts"]["matchedFilters"], Value::from(1));

    let missing_page = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--visual",
        &visual_name,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_page.code, 2);
    assert!(
        stderr_json(&missing_page)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --page")
    );
}

#[test]
fn report_filters_clear_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    let project_arg = project.to_str().expect("project path");

    let missing_selector = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_selector.code, 2);
    assert!(
        stderr_json(&missing_selector)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --handle")
    );

    let scope_all = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--scope",
        "all",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(scope_all.code, 2);
    assert!(
        stderr_json(&scope_all)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--all")
    );

    let mixed_all = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--all",
        "--page",
        &first_page_name(&project),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(mixed_all.code, 2);
    assert!(
        stderr_json(&mixed_all)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("cannot be combined")
    );

    let missing_mode = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --dry-run")
    );

    let missing_confirm = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--in-place",
        "--json",
    ]);
    assert_eq!(missing_confirm.code, 2);
    let missing_confirm_json = stderr_json(&missing_confirm);
    assert!(
        missing_confirm_json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--confirm clear:filters:report:report:main:1")
    );
    assert!(
        missing_confirm_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("--confirm clear:filters:report:report:main:1"))
    );
}

#[test]
fn report_filters_clear_groups_filter_config_and_legacy_arrays() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_filter_fixtures(&project);
    patch_json(&report_json(&project), |report| {
        report["filters"] = json!([{
            "name": "ReportRegionFilter",
            "type": "Categorical",
            "field": {
                "Column": {
                    "Expression": { "SourceRef": { "Entity": "DimRegion" } },
                    "Property": "Region"
                }
            },
            "filter": { "values": ["South"] }
        }]);
    });
    let project_arg = project.to_str().expect("project path");

    let dry = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--dry-run",
        "--include-raw",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(dry_json["counts"]["matchedFilters"], Value::from(2));
    assert_eq!(dry_json["counts"]["arrayEdits"], Value::from(2));
    assert_eq!(
        dry_json["targets"][0]["handle"],
        Value::from("filter:report:main:ReportRegionFilter")
    );
    assert_eq!(
        dry_json["targets"][1]["handle"],
        Value::from("filter:report:main:ReportRegionFilter#legacy")
    );
    assert_eq!(dry_json["targets"][0]["handleAmbiguous"], false);
    assert_eq!(dry_json["targets"][1]["handleAmbiguous"], false);
    assert_eq!(dry_json["targets"][1]["arrayOrigin"], "legacy");
    assert_eq!(
        dry_json["filterPlan"]["rawBeforeIncluded"],
        Value::Bool(true)
    );
    assert_eq!(
        dry_json["targets"][0]["raw"]["name"],
        Value::from("ReportRegionFilter")
    );

    let out_dir = temp.path().join("sales_project_report_filters_cleared");
    let out_arg = out_dir.to_str().expect("out dir");
    let clear = run_powerbi(&[
        "report",
        "filters",
        "clear",
        "--project",
        project_arg,
        "--scope",
        "report",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);

    let after = run_powerbi(&["report", "filters", "list", "--project", out_arg, "--json"]);
    assert_eq!(after.code, 0, "stderr: {}", after.stderr);
    let after_json = stdout_json(&after);
    assert_eq!(after_json["counts"]["reportFilters"], Value::from(0));
    assert_eq!(after_json["counts"]["pageFilters"], Value::from(1));
    assert_eq!(after_json["counts"]["visualFilters"], Value::from(1));
}

#[test]
fn report_slicers_list_empty_scaffold_returns_zero_slicers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.report.slicers.list.v1")
    );
    assert_eq!(value["counts"]["slicers"], Value::from(0));
    assert_eq!(value["slicers"].as_array().expect("slicers").len(), 0);
}

#[test]
fn report_slicers_list_and_show_raw_slicer_by_handle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_slicer_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["counts"]["slicers"], Value::from(1));
    assert_eq!(value["counts"]["boundSlicers"], Value::from(1));
    assert_eq!(value["counts"]["possibleDataValueSlicers"], Value::from(1));
    assert!(
        value["slicers"]
            .as_array()
            .expect("slicers")
            .iter()
            .all(|slicer| slicer.get("raw").is_none()),
        "list should not include raw slicer visual JSON by default"
    );

    let slicer = &value["slicers"][0];
    assert_eq!(slicer["title"], Value::from("Region Slicer"));
    assert_eq!(slicer["visualType"], Value::from("slicer"));
    assert_eq!(slicer["target"]["table"], Value::from("DimRegion"));
    assert_eq!(slicer["target"]["column"], Value::from("Region"));
    assert_eq!(slicer["state"]["fieldCount"], Value::from(1));
    assert_eq!(slicer["state"]["filterConfigFilters"], Value::from(1));
    assert_eq!(slicer["state"]["hasSelectionState"], Value::Bool(true));
    assert_eq!(slicer["state"]["hasCachedDisplayState"], Value::Bool(true));
    assert_eq!(slicer["safety"]["mayContainDataValues"], Value::Bool(true));
    let handle = slicer["handle"].as_str().expect("slicer handle");
    let visual_handle = slicer["visualHandle"].as_str().expect("visual handle");
    assert!(handle.starts_with("slicer:"));
    assert!(visual_handle.starts_with("visual:"));
    assert!(
        slicer["state"]["queryRoles"]
            .as_array()
            .expect("query roles")
            .iter()
            .any(|role| role == "Values")
    );

    let show = run_powerbi(&[
        "report",
        "slicers",
        "show",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["schema"],
        Value::from("powerbi-cli.report.slicers.show.v1")
    );
    assert_eq!(show_json["slicer"]["handle"], Value::from(handle));
    assert_eq!(
        show_json["slicer"]["raw"]["visual"]["visualType"],
        Value::from("slicer")
    );
    assert_eq!(
        show_json["slicer"]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
    assert!(
        show_json["visualReadbackCommand"]
            .as_str()
            .expect("visual readback command")
            .contains("report visuals show")
    );

    let include_raw = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(include_raw.code, 0, "stderr: {}", include_raw.stderr);
    let include_raw_json = stdout_json(&include_raw);
    assert_eq!(
        include_raw_json["slicers"][0]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
    assert_eq!(
        include_raw_json["slicers"][0]["raw"]["filterConfig"]["filters"][0]["name"],
        Value::from("SlicerRegionSelection")
    );
}

#[test]
fn report_slicers_show_accepts_visual_handle_and_rejects_missing_or_unknown_handle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_slicer_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let list = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let visual_handle = list_json["slicers"][0]["visualHandle"]
        .as_str()
        .expect("visual handle");

    let show_by_visual = run_powerbi(&[
        "report",
        "slicers",
        "show",
        "--project",
        project_arg,
        "--handle",
        visual_handle,
        "--no-raw",
        "--json",
    ]);
    assert_eq!(show_by_visual.code, 0, "stderr: {}", show_by_visual.stderr);
    let show_json = stdout_json(&show_by_visual);
    assert_eq!(
        show_json["slicer"]["visualHandle"],
        Value::from(visual_handle)
    );
    assert!(show_json["slicer"].get("raw").is_none());
    assert_eq!(
        show_json["slicer"]["safety"]["rawIncluded"],
        Value::Bool(false)
    );

    let missing = run_powerbi(&[
        "report",
        "slicers",
        "show",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(missing.code, 2);
    let missing_json = stderr_json(&missing);
    assert!(
        missing_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report slicers list"))
    );

    let unknown = run_powerbi(&[
        "report",
        "slicers",
        "show",
        "--project",
        project_arg,
        "--handle",
        "slicer:nope",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    let unknown_json = stderr_json(&unknown);
    assert!(
        unknown_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report slicers list"))
    );
}

#[test]
fn report_slicers_clear_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_slicer_fixture(&project);
    let project_arg = project.to_str().expect("project path");
    let visual_path = first_visual_json(&project);
    let source_before = fs::read_to_string(&visual_path).expect("source visual before clear");

    let list = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let handle = list_json["slicers"][0]["handle"]
        .as_str()
        .expect("slicer handle")
        .to_string();

    let dry = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--dry-run",
        "--include-raw",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.slicers.clearMutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["mode"], Value::from("dry-run"));
    assert_eq!(dry_json["target"]["handle"], Value::from(handle.clone()));
    assert_eq!(dry_json["counts"]["clearedFilterEntries"], Value::from(1));
    assert_eq!(dry_json["counts"]["filterConfigFilters"], Value::from(1));
    assert_eq!(dry_json["counts"]["legacyFilters"], Value::from(0));
    assert_eq!(
        dry_json["slicerPlan"]["beforeState"]["filterConfigFilters"],
        Value::from(1)
    );
    assert_eq!(
        dry_json["slicerPlan"]["afterState"]["filterConfigFilters"],
        Value::from(0)
    );
    assert_eq!(
        dry_json["changes"][0]["jsonPointer"],
        Value::from("/filterConfig/filters/0")
    );
    assert_eq!(
        dry_json["changes"][0]["parentJsonPointer"],
        Value::from("/filterConfig/filters")
    );
    assert_eq!(
        dry_json["changes"][0]["before"]["name"],
        Value::from("SlicerRegionSelection")
    );
    assert!(
        dry_json["rawReviewCommand"]
            .as_str()
            .expect("raw review command")
            .contains("--include-raw")
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after dry-run"),
        source_before,
        "dry-run must not mutate the source project"
    );

    let out_dir = temp.path().join("sales_project_slicer_cleared");
    let out_arg = out_dir.to_str().expect("out dir");
    let clear = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    let clear_json = stdout_json(&clear);
    assert_eq!(clear_json["mode"], Value::from("out-dir"));
    assert_eq!(clear_json["validation"]["ok"], Value::Bool(true));
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after out-dir"),
        source_before,
        "out-dir clear must not mutate the source project"
    );

    let after = run_powerbi(&["report", "slicers", "list", "--project", out_arg, "--json"]);
    assert_eq!(after.code, 0, "stderr: {}", after.stderr);
    let after_json = stdout_json(&after);
    let after_slicer = &after_json["slicers"][0];
    assert_eq!(after_slicer["state"]["filterConfigFilters"], Value::from(0));
    assert_eq!(after_slicer["state"]["legacyFilters"], Value::from(0));
    assert_eq!(
        after_slicer["state"]["hasSelectionState"],
        Value::Bool(false)
    );
    assert_eq!(
        after_slicer["state"]["hasCachedDisplayState"],
        Value::Bool(false)
    );
    assert_eq!(after_slicer["target"]["table"], Value::from("DimRegion"));
    assert_eq!(after_slicer["target"]["column"], Value::from("Region"));

    let show_after = run_powerbi(&[
        "report",
        "slicers",
        "show",
        "--project",
        out_arg,
        "--handle",
        &handle,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(show_after.code, 0, "stderr: {}", show_after.stderr);
    let show_after_json = stdout_json(&show_after);
    assert_eq!(
        show_after_json["slicer"]["raw"]["filterConfig"]["filters"]
            .as_array()
            .expect("cleared filters")
            .len(),
        0
    );
    assert_eq!(
        show_after_json["slicer"]["raw"]["visual"]["query"]["queryState"]["Values"]["projections"]
            .as_array()
            .expect("slicer projections")
            .len(),
        1
    );
    assert_eq!(
        show_after_json["slicer"]["raw"]["visual"]["objects"]["general"][0]["properties"]["orientation"]
            ["expr"]["Literal"]["Value"],
        Value::from("'vertical'")
    );

    let original = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(original.code, 0, "stderr: {}", original.stderr);
    assert_eq!(
        stdout_json(&original)["slicers"][0]["state"]["filterConfigFilters"],
        Value::from(1)
    );
}

#[test]
fn report_slicers_clear_accepts_visual_selectors_and_rejects_non_slicer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_slicer_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let list = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    let slicer = &list_json["slicers"][0];
    let handle = slicer["handle"].as_str().expect("slicer handle");
    let visual_handle = slicer["visualHandle"].as_str().expect("visual handle");
    let page_handle = slicer["page"]["handle"].as_str().expect("page handle");
    let title = slicer["title"].as_str().expect("slicer title");

    let by_visual_handle = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        visual_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        by_visual_handle.code, 0,
        "stderr: {}",
        by_visual_handle.stderr
    );
    assert_eq!(
        stdout_json(&by_visual_handle)["target"]["handle"],
        Value::from(handle)
    );

    let by_page_title = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--page",
        page_handle,
        "--visual",
        title,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(by_page_title.code, 0, "stderr: {}", by_page_title.stderr);
    assert_eq!(
        stdout_json(&by_page_title)["target"]["visualHandle"],
        Value::from(visual_handle)
    );

    let by_page_handle = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--page",
        page_handle,
        "--visual",
        visual_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(by_page_handle.code, 0, "stderr: {}", by_page_handle.stderr);
    assert_eq!(
        stdout_json(&by_page_handle)["target"]["visualHandle"],
        Value::from(visual_handle)
    );

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let non_slicer_handle = visuals_json["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] != "slicer")
        .and_then(|visual| visual["handle"].as_str())
        .expect("non-slicer visual handle");

    let non_slicer = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        non_slicer_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(non_slicer.code, 2);
    assert!(
        stderr_json(&non_slicer)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("not a slicer")
    );
}

#[test]
fn report_slicers_clear_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_slicer_fixture(&project);
    let project_arg = project.to_str().expect("project path");
    let list = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let handle = stdout_json(&list)["slicers"][0]["handle"]
        .as_str()
        .expect("slicer handle")
        .to_string();

    let missing_selector = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_selector.code, 2);
    assert!(
        stderr_json(&missing_selector)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --handle")
    );

    let visual_without_page = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--visual",
        "Region Slicer",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(visual_without_page.code, 2);
    assert!(
        stderr_json(&visual_without_page)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --page")
    );

    let mixed_selector = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--page",
        &first_page_name(&project),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(mixed_selector.code, 2);
    assert!(
        stderr_json(&mixed_selector)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("cannot be combined")
    );

    let missing_mode = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --dry-run")
    );

    let missing_confirm = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--in-place",
        "--json",
    ]);
    assert_eq!(missing_confirm.code, 2);
    let missing_confirm_json = stderr_json(&missing_confirm);
    assert!(
        missing_confirm_json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--confirm clear:slicer:")
    );
    assert!(
        missing_confirm_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("--confirm clear:slicer:"))
    );
}

#[test]
fn report_slicers_clear_handles_legacy_array_and_preserves_unmatched_filters() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_slicer_fixture(&project);
    patch_json(&first_visual_json(&project), |visual| {
        visual["filterConfig"]["filters"]
            .as_array_mut()
            .expect("slicer filterConfig filters")
            .push(categorical_filter_fixture(
                "UnrelatedProductFilter",
                "DimProduct",
                "Category",
                vec![Value::from("Tools")],
            ));
        visual["filters"] = json!([{
            "name": "LegacySlicerRegionSelection",
            "type": "Categorical",
            "field": {
                "Column": {
                    "Expression": { "SourceRef": { "Entity": "DimRegion" } },
                    "Property": "Region"
                }
            },
            "filter": { "values": ["South"] }
        }]);
    });
    let project_arg = project.to_str().expect("project path");
    let list = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let handle = stdout_json(&list)["slicers"][0]["handle"]
        .as_str()
        .expect("slicer handle")
        .to_string();

    let out_dir = temp.path().join("sales_project_slicer_target_clear");
    let out_arg = out_dir.to_str().expect("out dir");
    let clear = run_powerbi(&[
        "report",
        "slicers",
        "clear",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    let clear_json = stdout_json(&clear);
    assert_eq!(clear_json["counts"]["clearedFilterEntries"], Value::from(2));
    assert_eq!(clear_json["counts"]["filterConfigFilters"], Value::from(1));
    assert_eq!(clear_json["counts"]["legacyFilters"], Value::from(1));
    let pointers = clear_json["changes"]
        .as_array()
        .expect("changes")
        .iter()
        .map(|change| change["jsonPointer"].as_str().expect("pointer"))
        .collect::<Vec<_>>();
    assert!(pointers.contains(&"/filterConfig/filters/0"));
    assert!(pointers.contains(&"/filters/0"));

    let show = run_powerbi(&[
        "report",
        "slicers",
        "show",
        "--project",
        out_arg,
        "--handle",
        &handle,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    let filter_config_filters = show_json["slicer"]["raw"]["filterConfig"]["filters"]
        .as_array()
        .expect("filterConfig filters");
    assert_eq!(filter_config_filters.len(), 1);
    assert_eq!(
        filter_config_filters[0]["name"],
        Value::from("UnrelatedProductFilter")
    );
    assert_eq!(
        show_json["slicer"]["raw"]["filters"]
            .as_array()
            .expect("legacy filters")
            .len(),
        0
    );
}

#[test]
fn report_interactions_list_empty_scaffold_returns_zero_interactions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "interactions",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.report.interactions.list.v1")
    );
    assert_eq!(value["counts"]["interactions"], Value::from(0));
    assert_eq!(
        value["interactions"]
            .as_array()
            .expect("interactions")
            .len(),
        0
    );
    assert_eq!(
        value["semantics"]["mode"],
        Value::from("explicit-overrides")
    );
    assert!(
        value["semantics"]["missingRowsMean"]
            .as_str()
            .unwrap_or_default()
            .contains("default interaction behavior")
    );
}

#[test]
fn report_interactions_list_and_show_page_visual_interactions_by_handle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (source, target) = install_interaction_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "interactions",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["counts"]["interactions"], Value::from(2));
    assert_eq!(
        value["counts"]["pagesWithExplicitInteractions"],
        Value::from(1)
    );
    assert_eq!(value["counts"]["unsupported"], Value::from(1));
    assert_eq!(value["counts"]["staleVisualReferences"], Value::from(1));
    assert_eq!(value["counts"]["byType"]["NoFilter"], Value::from(1));
    assert_eq!(value["counts"]["byType"]["SurpriseMode"], Value::from(1));
    assert!(
        value["interactions"]
            .as_array()
            .expect("interactions")
            .iter()
            .all(|interaction| interaction.get("raw").is_none()),
        "list should not include raw interaction JSON by default"
    );

    let first = &value["interactions"][0];
    assert_eq!(first["interactionType"], Value::from("NoFilter"));
    assert_eq!(first["sourceName"], Value::from(source.as_str()));
    assert_eq!(first["targetName"], Value::from(target.as_str()));
    assert_eq!(first["source"]["found"], Value::Bool(true));
    assert_eq!(first["target"]["found"], Value::Bool(true));
    assert_eq!(first["unsupported"], Value::Bool(false));
    assert_eq!(first["safety"]["mayContainDataValues"], Value::Bool(false));
    let handle = first["handle"].as_str().expect("interaction handle");
    assert!(handle.starts_with("interaction:ReportSectionOverview:"));

    let show = run_powerbi(&[
        "report",
        "interactions",
        "show",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["schema"],
        Value::from("powerbi-cli.report.interactions.show.v1")
    );
    assert_eq!(show_json["interaction"]["handle"], Value::from(handle));
    assert_eq!(
        show_json["interaction"]["raw"]["type"],
        Value::from("NoFilter")
    );
    assert_eq!(
        show_json["interaction"]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
    assert!(
        show_json["sourceVisualReadbackCommand"]
            .as_str()
            .expect("source visual readback")
            .contains("report visuals show")
    );
    assert!(
        show_json["targetVisualReadbackCommand"]
            .as_str()
            .expect("target visual readback")
            .contains("report visuals show")
    );

    let include_raw = run_powerbi(&[
        "report",
        "interactions",
        "list",
        "--project",
        project_arg,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(include_raw.code, 0, "stderr: {}", include_raw.stderr);
    let include_raw_json = stdout_json(&include_raw);
    assert_eq!(
        include_raw_json["interactions"][0]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
    assert_eq!(
        include_raw_json["interactions"][1]["target"]["found"],
        Value::Bool(false)
    );
}

#[test]
fn report_interactions_show_accepts_endpoint_selector_and_rejects_bad_selectors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (source, target) = install_interaction_fixture(&project);
    let project_arg = project.to_str().expect("project path");

    let by_endpoints = run_powerbi(&[
        "report",
        "interactions",
        "show",
        "--project",
        project_arg,
        "--page",
        "page:ReportSectionOverview",
        "--source",
        &source,
        "--target",
        &target,
        "--no-raw",
        "--json",
    ]);
    assert_eq!(by_endpoints.code, 0, "stderr: {}", by_endpoints.stderr);
    let by_endpoints_json = stdout_json(&by_endpoints);
    assert_eq!(
        by_endpoints_json["interaction"]["interactionType"],
        Value::from("NoFilter")
    );
    assert!(by_endpoints_json["interaction"].get("raw").is_none());
    assert_eq!(
        by_endpoints_json["interaction"]["safety"]["rawIncluded"],
        Value::Bool(false)
    );

    let filtered = run_powerbi(&[
        "report",
        "interactions",
        "list",
        "--project",
        project_arg,
        "--type",
        "no-filter",
        "--source",
        &source,
        "--json",
    ]);
    assert_eq!(filtered.code, 0, "stderr: {}", filtered.stderr);
    let filtered_json = stdout_json(&filtered);
    assert_eq!(filtered_json["counts"]["interactions"], Value::from(1));
    assert_eq!(
        filtered_json["interactions"][0]["interactionType"],
        Value::from("NoFilter")
    );

    let missing = run_powerbi(&[
        "report",
        "interactions",
        "show",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(missing.code, 2);
    let missing_json = stderr_json(&missing);
    assert!(
        missing_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report interactions list"))
    );

    let unknown = run_powerbi(&[
        "report",
        "interactions",
        "show",
        "--project",
        project_arg,
        "--handle",
        "interaction:nope",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    let unknown_json = stderr_json(&unknown);
    assert!(
        unknown_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report interactions list"))
    );
}

#[test]
fn report_interactions_disable_dry_run_and_out_dir_upsert_no_filter() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project arg");
    let page_name = first_page_name(&project);
    let page_handle = format!("page:{page_name}");
    let (source, target) = first_two_visual_names(&project);
    let source_handle = format!("visual:{page_name}:{source}");
    let target_handle = format!("visual:{page_name}:{target}");
    let page_path = first_page_json(&project);
    let before_page = fs::read_to_string(&page_path).expect("page json before");

    let dry_run = run_powerbi(&[
        "report",
        "interactions",
        "disable",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.interactions.mutation.v1")
    );
    assert_eq!(dry_json["action"], Value::from("disable"));
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        dry_json["target"]["interactionType"],
        Value::from("NoFilter")
    );
    assert_eq!(dry_json["interactionPlan"]["existed"], Value::Bool(false));
    assert_eq!(
        fs::read_to_string(&page_path).expect("page json after dry-run"),
        before_page,
        "dry-run must not mutate source page.json"
    );

    let out_dir = temp.path().join("sales_disabled");
    let out_arg = out_dir.to_str().expect("out dir");
    let written = run_powerbi(&[
        "report",
        "interactions",
        "disable",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(written.code, 0, "stderr: {}", written.stderr);
    let written_json = stdout_json(&written);
    assert_eq!(written_json["ok"], Value::Bool(true));
    assert_eq!(written_json["mode"], Value::from("out-dir"));
    assert_eq!(
        written_json["validation"]["ok"],
        Value::Bool(true),
        "out-dir writes should validate"
    );
    assert_eq!(
        fs::read_to_string(&page_path).expect("source page after out-dir"),
        before_page,
        "out-dir mutation must leave source project unchanged"
    );

    let show = run_powerbi(&[
        "report",
        "interactions",
        "show",
        "--project",
        out_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--no-raw",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["interaction"]["interactionType"],
        Value::from("NoFilter")
    );
    assert!(show_json["interaction"].get("raw").is_none());
}

#[test]
fn report_interactions_set_updates_existing_row_without_duplicates_and_supports_in_place() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project arg");
    let page_name = first_page_name(&project);
    let page_handle = format!("page:{page_name}");
    let (source, target) = install_interaction_fixture(&project);
    let source_handle = format!("visual:{page_name}:{source}");
    let target_handle = format!("visual:{page_name}:{target}");

    let out_dir = temp.path().join("sales_highlight");
    let out_arg = out_dir.to_str().expect("out dir");
    let update = run_powerbi(&[
        "report",
        "interactions",
        "set",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--type",
        "HighlightFilter",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(update.code, 0, "stderr: {}", update.stderr);
    let update_json = stdout_json(&update);
    assert_eq!(update_json["interactionPlan"]["existed"], Value::Bool(true));
    assert_eq!(
        update_json["changes"][0]["action"],
        Value::from("update"),
        "existing interaction should update, not append"
    );

    let list = run_powerbi(&[
        "report",
        "interactions",
        "list",
        "--project",
        out_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(list_json["counts"]["interactions"], Value::from(2));
    assert_eq!(
        list_json["interactions"][0]["interactionType"],
        Value::from("HighlightFilter")
    );

    let in_place = run_powerbi(&[
        "report",
        "interactions",
        "set",
        "--project",
        out_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--type",
        "DataFilter",
        "--in-place",
        "--json",
    ]);
    assert_eq!(in_place.code, 0, "stderr: {}", in_place.stderr);
    let in_place_json = stdout_json(&in_place);
    assert_eq!(in_place_json["mode"], Value::from("in-place"));
    assert_eq!(in_place_json["validation"]["ok"], Value::Bool(true));

    let show = run_powerbi(&[
        "report",
        "interactions",
        "show",
        "--project",
        out_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--no-raw",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["interaction"]["interactionType"],
        Value::from("DataFilter")
    );
}

#[test]
fn report_interactions_mutations_reject_unsafe_or_unproven_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project arg");
    let page_name = first_page_name(&project);
    let page_handle = format!("page:{page_name}");
    let (source, target) = first_two_visual_names(&project);
    let source_handle = format!("visual:{page_name}:{source}");
    let target_handle = format!("visual:{page_name}:{target}");

    let default = run_powerbi(&[
        "report",
        "interactions",
        "set",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--type",
        "Default",
        "--dry-run",
        "--json",
    ]);
    assert_ne!(default.code, 0);
    let default_json = stderr_json(&default);
    assert_eq!(
        default_json["error"]["code"],
        Value::from("unsupported_feature")
    );
    assert!(
        default_json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("report.interaction-default-reset")
    );

    let missing_mode = run_powerbi(&[
        "report",
        "interactions",
        "disable",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--json",
    ]);
    assert_ne!(missing_mode.code, 0);
    let missing_mode_json = stderr_json(&missing_mode);
    assert!(
        missing_mode_json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires --dry-run")
    );

    patch_json(&first_page_json(&project), |page| {
        page["visualInteractions"] = json!([
            {
                "source": source.clone(),
                "target": target.clone(),
                "type": "NoFilter"
            },
            {
                "source": source.clone(),
                "target": target.clone(),
                "type": "DataFilter"
            }
        ]);
    });
    let duplicate = run_powerbi(&[
        "report",
        "interactions",
        "disable",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--source",
        &source_handle,
        "--target",
        &target_handle,
        "--dry-run",
        "--json",
    ]);
    assert_ne!(duplicate.code, 0);
    let duplicate_json = stderr_json(&duplicate);
    assert!(
        duplicate_json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("duplicate visualInteractions")
    );
}

#[test]
fn report_drillthrough_set_show_clear_round_trips_through_out_dirs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page_name = first_page_name(&project);
    let page_handle = format!("page:{page_name}");

    let dry = run_powerbi(&[
        "report",
        "drillthrough",
        "set",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--target",
        "DimCustomer[Segment]",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "stderr: {}", dry.stderr);
    let dry_json = stdout_json(&dry);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.drillthrough.setMutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["target"]["table"], Value::from("DimCustomer"));
    assert_eq!(dry_json["target"]["column"], Value::from("Segment"));
    assert_eq!(
        dry_json["drillthroughPlan"]["after"]["enabled"],
        Value::Bool(true)
    );
    assert_eq!(
        dry_json["drillthroughPlan"]["after"]["binding"]["type"],
        Value::from("Drillthrough")
    );
    assert_eq!(
        dry_json["drillthroughPlan"]["after"]["filters"]
            .as_array()
            .expect("filters")
            .len(),
        1
    );
    let original_page: Value =
        serde_json::from_str(&fs::read_to_string(first_page_json(&project)).expect("page json"))
            .expect("parse page json");
    assert!(original_page.get("pageBinding").is_none());

    let with_drill = temp.path().join("sales_with_drillthrough");
    let with_drill_arg = with_drill.to_str().expect("with drill path");
    let write = run_powerbi(&[
        "report",
        "drillthrough",
        "set",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--target",
        "DimCustomer[Segment]",
        "--out-dir",
        with_drill_arg,
        "--json",
    ]);
    assert_eq!(write.code, 0, "stderr: {}", write.stderr);
    let write_json = stdout_json(&write);
    assert_eq!(write_json["ok"], Value::Bool(true));
    assert_eq!(write_json["mode"], Value::from("out-dir"));
    assert_eq!(write_json["validation"]["ok"], Value::Bool(true));
    let written_page: Value =
        serde_json::from_str(&fs::read_to_string(first_page_json(&with_drill)).expect("page json"))
            .expect("parse page json");
    assert_eq!(written_page["type"], Value::from("Drillthrough"));
    assert_eq!(written_page["visibility"], Value::from("HiddenInViewMode"));
    assert_eq!(
        written_page["pageBinding"]["type"],
        Value::from("Drillthrough")
    );
    assert_eq!(
        written_page["pageBinding"]["referenceScope"],
        Value::from("Default")
    );
    let parameter = &written_page["pageBinding"]["parameters"][0];
    let bound_filter = parameter["boundFilter"]
        .as_str()
        .expect("bound drillthrough filter");
    assert!(bound_filter.starts_with("DrillthroughFilter_"));
    assert_eq!(
        write_json["drillthroughPlan"]["filterName"],
        Value::from(bound_filter)
    );
    assert_eq!(
        parameter["fieldExpr"]["Column"]["Expression"]["SourceRef"]["Entity"],
        Value::from("DimCustomer")
    );
    assert_eq!(
        parameter["fieldExpr"]["Column"]["Property"],
        Value::from("Segment")
    );
    let paired_filters = written_page["filterConfig"]["filters"]
        .as_array()
        .expect("paired drillthrough filters");
    assert_eq!(paired_filters.len(), 1);
    let paired_filter = &paired_filters[0];
    assert_eq!(paired_filter["name"], Value::from(bound_filter));
    assert_eq!(paired_filter["howCreated"], Value::from("Drillthrough"));
    assert_eq!(paired_filter["type"], Value::from("Categorical"));
    assert_eq!(paired_filter["field"], parameter["fieldExpr"]);
    assert!(
        paired_filter.get("filter").is_none(),
        "Desktop-authored Drillthrough filters have no persisted filter body"
    );

    let show = run_powerbi(&[
        "report",
        "drillthrough",
        "show",
        "--project",
        with_drill_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(show_json["drillthrough"]["enabled"], Value::Bool(true));
    assert_eq!(
        show_json["drillthrough"]["binding"]["parameters"][0]["target"]["table"],
        Value::from("DimCustomer")
    );
    assert_eq!(
        show_json["drillthrough"]["binding"]["parameters"][0]["target"]["column"],
        Value::from("Segment")
    );
    assert_eq!(
        show_json["drillthrough"]["binding"]["parameters"][0]["boundFilter"],
        Value::from(bound_filter)
    );
    assert_eq!(
        show_json["drillthrough"]["binding"]["parameters"][0]["fieldExpr"],
        parameter["fieldExpr"]
    );
    assert_eq!(
        show_json["drillthrough"]["filters"][0]["name"],
        Value::from(bound_filter)
    );
    assert_eq!(
        show_json["drillthrough"]["filters"][0]["hasPersistedFilterDefinition"],
        Value::Bool(false)
    );

    let normalized = run_powerbi(&["fixture", "normalize", with_drill_arg, "--json"]);
    assert_eq!(normalized.code, 0, "stderr: {}", normalized.stderr);
    let normalized_json = stdout_json(&normalized);
    let drillthrough = &normalized_json["report"]["pages"][0]["drillthrough"];
    assert_eq!(drillthrough["enabled"], Value::Bool(true));
    assert_eq!(
        drillthrough["binding"]["parameters"][0]["target"]["table"],
        Value::from("DimCustomer")
    );
    assert_eq!(
        drillthrough["binding"]["parameters"][0]["boundFilter"],
        Value::from(bound_filter)
    );
    let normalized_filter = normalized_json["pbir"]["filters"]["items"]
        .as_array()
        .expect("normalized filters")
        .iter()
        .find(|filter| filter["name"].as_str() == Some(bound_filter))
        .expect("normalized paired drillthrough filter");
    assert_eq!(normalized_filter["scope"], Value::from("page"));
    assert_eq!(normalized_filter["filterType"], Value::from("Categorical"));
    assert_eq!(
        normalized_filter["target"]["table"],
        Value::from("DimCustomer")
    );
    assert_eq!(
        normalized_filter["target"]["column"],
        Value::from("Segment")
    );
    assert_eq!(normalized_filter["literalCount"], Value::from(0));

    let validate = run_powerbi(&["validate", "--strict", with_drill_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));

    let cleared = temp.path().join("sales_drillthrough_cleared");
    let cleared_arg = cleared.to_str().expect("cleared path");
    let clear = run_powerbi(&[
        "report",
        "drillthrough",
        "clear",
        "--project",
        with_drill_arg,
        "--page",
        &page_handle,
        "--out-dir",
        cleared_arg,
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    let clear_json = stdout_json(&clear);
    assert_eq!(clear_json["ok"], Value::Bool(true));
    assert_eq!(
        clear_json["drillthroughPlan"]["after"]["enabled"],
        Value::Bool(false)
    );
    assert_eq!(
        clear_json["drillthroughPlan"]["removedFilters"],
        Value::from(1)
    );
    let cleared_page: Value =
        serde_json::from_str(&fs::read_to_string(first_page_json(&cleared)).expect("page json"))
            .expect("parse page json");
    assert!(cleared_page.get("type").is_none());
    assert!(cleared_page.get("pageBinding").is_none());
    assert!(
        cleared_page["filterConfig"]["filters"]
            .as_array()
            .expect("cleared page filters")
            .is_empty(),
        "clear must remove the filter paired with the pageBinding parameter"
    );
    assert_eq!(
        cleared_page["visibility"],
        Value::from("HiddenInViewMode"),
        "clear does not infer whether hidden pages should become visible"
    );
}

#[test]
fn report_drillthrough_rejects_unproven_variants() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page_name = first_page_name(&project);
    let page_handle = format!("page:{page_name}");

    for args in [
        vec![
            "report",
            "drillthrough",
            "set",
            "--project",
            project_arg,
            "--page",
            &page_handle,
            "--target",
            "DimCustomer[Segment]",
            "--cross-report",
            "--dry-run",
            "--json",
        ],
        vec![
            "report",
            "drillthrough",
            "set",
            "--project",
            project_arg,
            "--page",
            &page_handle,
            "--target",
            "DimCustomer[Segment]",
            "--visual",
            "visual:source",
            "--dry-run",
            "--json",
        ],
        vec![
            "report",
            "drillthrough",
            "set",
            "--project",
            project_arg,
            "--page",
            &page_handle,
            "--target",
            "DimCustomer[Segment]",
            "--filter-name",
            "DesktopSpecificFilter",
            "--dry-run",
            "--json",
        ],
    ] {
        let output = run_powerbi(&args);
        assert_eq!(output.code, 2, "args: {args:?}; stderr: {}", output.stderr);
        let error = stderr_json(&output);
        assert_eq!(
            error["error"]["code"],
            Value::from("unsupported_feature"),
            "args: {args:?}"
        );
    }
}

#[test]
fn known_unimplemented_report_features_return_structured_refusals() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["report", "tooltips", "add", "--json"],
        vec!["report", "bookmarks", "add", "--json"],
        vec!["report", "slicers", "add", "--json"],
        vec!["report", "slicers", "sync", "--json"],
        vec!["report", "interactions", "reset", "--json"],
    ];

    for args in cases {
        let output = run_powerbi(&args);
        assert_eq!(output.code, 2, "args: {args:?}; stderr: {}", output.stderr);
        assert!(output.stdout.trim().is_empty(), "args: {args:?}");
        let error = stderr_json(&output);
        assert_eq!(
            error["error"]["code"],
            Value::from("unsupported_feature"),
            "args: {args:?}"
        );
        assert!(
            !error["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("unknown"),
            "args: {args:?}; error: {error}"
        );
        assert!(
            error["error"]["suggestedCommands"]
                .as_array()
                .expect("suggestedCommands")
                .iter()
                .any(|command| command
                    .as_str()
                    .unwrap_or_default()
                    .contains("features list")),
            "args: {args:?}; error: {error}"
        );
    }
}

#[test]
fn report_bookmarks_list_empty_scaffold_returns_zero_bookmarks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "bookmarks",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.report.bookmarks.list.v1")
    );
    assert_eq!(value["counts"]["bookmarks"], Value::from(0));
    let bookmarks_dir = value["bookmarksDir"].as_str().expect("bookmarks dir");
    assert!(
        bookmarks_dir.ends_with("definition\\bookmarks")
            || bookmarks_dir.ends_with("definition/bookmarks")
    );
    assert_eq!(value["bookmarks"].as_array().expect("bookmarks").len(), 0);
}

#[test]
fn report_bookmarks_list_and_show_raw_bookmarks_by_handle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_bookmark_fixtures(&project);
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "bookmarks",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["counts"]["bookmarks"], Value::from(2));
    assert_eq!(value["counts"]["groups"], Value::from(1));
    assert_eq!(
        value["counts"]["possibleDataValueBookmarks"],
        Value::from(1)
    );
    assert_eq!(value["counts"]["targetVisualBookmarks"], Value::from(1));
    assert_eq!(value["bookmarksMetadata"]["items"], Value::from(2));
    assert_eq!(value["bookmarksMetadata"]["groups"], Value::from(1));
    assert_eq!(
        value["bookmarkDiagnostics"]
            .as_array()
            .expect("bookmark diagnostics")
            .len(),
        0
    );
    assert!(
        value["bookmarks"]
            .as_array()
            .expect("bookmarks")
            .iter()
            .all(|bookmark| bookmark.get("raw").is_none()),
        "list should not include raw bookmark JSON by default"
    );

    let first = &value["bookmarks"][0];
    assert_eq!(first["handle"], Value::from("bookmark:BookmarkExecutive"));
    assert_eq!(first["displayName"], Value::from("Executive View"));
    assert_eq!(first["schemaVersion"], Value::from("2.1.0"));
    assert_eq!(first["state"]["reportFilterStates"], Value::from(1));
    assert_eq!(first["state"]["pageFilterStates"], Value::from(1));
    assert_eq!(first["state"]["visualFilterStates"], Value::from(1));
    assert_eq!(first["state"]["highlightStates"], Value::from(1));
    assert_eq!(
        first["state"]["displayModeCounts"]["spotlight"],
        Value::from(1)
    );
    assert_eq!(first["safety"]["mayContainDataValues"], Value::Bool(true));
    let handle = first["handle"].as_str().expect("bookmark handle");

    let show = run_powerbi(&[
        "report",
        "bookmarks",
        "show",
        "--project",
        project_arg,
        "--handle",
        handle,
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["schema"],
        Value::from("powerbi-cli.report.bookmarks.show.v1")
    );
    assert_eq!(show_json["bookmark"]["handle"], Value::from(handle));
    assert_eq!(
        show_json["bookmark"]["raw"]["name"],
        Value::from("BookmarkExecutive")
    );
    assert_eq!(
        show_json["bookmark"]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
    assert!(
        show_json["readbackCommand"]
            .as_str()
            .expect("readback command")
            .contains("report bookmarks list")
    );

    let include_raw = run_powerbi(&[
        "report",
        "bookmarks",
        "list",
        "--project",
        project_arg,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(include_raw.code, 0, "stderr: {}", include_raw.stderr);
    let include_raw_json = stdout_json(&include_raw);
    assert_eq!(
        include_raw_json["bookmarks"][0]["safety"]["rawIncluded"],
        Value::Bool(true)
    );
    assert_eq!(
        include_raw_json["bookmarks"][1]["group"]["displayName"],
        Value::from("Operations")
    );
    assert_eq!(
        include_raw_json["bookmarks"][1]["options"]["targetVisualCount"],
        Value::from(1)
    );
}

#[test]
fn report_bookmarks_show_rejects_missing_or_unknown_handle_with_suggested_list_command() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let missing = run_powerbi(&[
        "report",
        "bookmarks",
        "show",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(missing.code, 2);
    let missing_json = stderr_json(&missing);
    assert!(
        missing_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report bookmarks list"))
    );

    let unknown = run_powerbi(&[
        "report",
        "bookmarks",
        "show",
        "--project",
        project_arg,
        "--handle",
        "bookmark:nope",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    let unknown_json = stderr_json(&unknown);
    assert!(
        unknown_json["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report bookmarks list"))
    );
}

#[test]
fn report_bookmarks_list_reports_metadata_and_file_diagnostics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let bookmarks_dir = project
        .join("SalesOperations.Report")
        .join("definition")
        .join("bookmarks");
    fs::create_dir_all(&bookmarks_dir).expect("bookmarks dir");
    fs::write(
        bookmarks_dir.join("bookmarks.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmarksMetadata/1.0.0/schema.json",
            "items": [{ "name": "MissingBookmark" }]
        }))
        .expect("bookmarks metadata"),
    )
    .expect("write bookmarks metadata");
    fs::write(
        bookmarks_dir.join("FileNameBookmark.bookmark.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/2.1.0/schema.json",
            "displayName": "Actual Bookmark",
            "name": "ActualBookmark",
            "explorationState": {
                "version": "1.3",
                "activeSection": "ReportSectionOverview",
                "sections": {
                    "ReportSectionOverview": {
                        "visualContainers": {}
                    }
                }
            }
        }))
        .expect("bookmark json"),
    )
    .expect("write bookmark");
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&[
        "report",
        "bookmarks",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let codes = value["bookmarkDiagnostics"]
        .as_array()
        .expect("bookmark diagnostics")
        .iter()
        .map(|item| item["code"].as_str().expect("diagnostic code"))
        .collect::<Vec<_>>();
    assert!(codes.contains(&"bookmark.metadata_missing_file"));
    assert!(codes.contains(&"bookmark.file_not_in_metadata"));
    assert!(codes.contains(&"bookmark.name_file_mismatch"));
    assert_eq!(value["bookmarks"][0]["handle"], "bookmark:ActualBookmark");
}

#[test]
fn report_pages_mutations_reject_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let pages_json = stdout_json(&pages);
    let original_handle = pages_json["pages"][0]["handle"]
        .as_str()
        .expect("original page handle")
        .to_string();

    let missing_mode = run_powerbi(&[
        "report",
        "pages",
        "add",
        "--project",
        project_arg,
        "--display-name",
        "Scratch",
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --dry-run")
    );

    let empty_update = run_powerbi(&[
        "report",
        "pages",
        "update",
        "--project",
        project_arg,
        "--handle",
        &original_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(empty_update.code, 2);
    assert!(
        stderr_json(&empty_update)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires at least one")
    );

    let non_empty_delete = run_powerbi(&[
        "report",
        "pages",
        "delete-empty",
        "--project",
        project_arg,
        "--handle",
        &original_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(non_empty_delete.code, 2);
    assert!(
        stderr_json(&non_empty_delete)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("refuses pages that contain visuals")
    );

    let added = temp.path().join("added_project");
    let added_arg = added.to_str().expect("added path");
    let add = run_powerbi(&[
        "report",
        "pages",
        "add",
        "--project",
        project_arg,
        "--display-name",
        "Scratch",
        "--out-dir",
        added_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    let scratch_handle = add_json["target"]["handle"]
        .as_str()
        .expect("scratch handle")
        .to_string();
    let scratch_name = add_json["target"]["name"]
        .as_str()
        .expect("scratch name")
        .to_string();

    let incomplete_reorder = run_powerbi(&[
        "report",
        "pages",
        "reorder",
        "--project",
        added_arg,
        "--order",
        &original_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(incomplete_reorder.code, 2);
    assert!(
        stderr_json(&incomplete_reorder)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("every page exactly once")
    );

    fs::write(
        added
            .join("SalesOperations.Report")
            .join("definition")
            .join("pages")
            .join(&scratch_name)
            .join("metadata.json"),
        "{}",
    )
    .expect("write unknown page file");
    let unsafe_delete = run_powerbi(&[
        "report",
        "pages",
        "delete-empty",
        "--project",
        added_arg,
        "--handle",
        &scratch_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unsafe_delete.code, 2);
    assert!(
        stderr_json(&unsafe_delete)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown files")
    );
}

#[test]
fn report_visual_set_position_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let visual_path = PathBuf::from(
        visuals_json["visuals"][0]["path"]
            .as_str()
            .expect("visual path"),
    );
    let source_before = fs::read_to_string(&visual_path).expect("source visual before");

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "set-position",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--x",
        "80",
        "--y",
        "90",
        "--width",
        "300",
        "--height",
        "210",
        "--tab-order",
        "4",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_run_json = stdout_json(&dry_run);
    assert_eq!(
        dry_run_json["schema"],
        Value::from("powerbi-cli.report.visuals.positionMutation.v1")
    );
    assert_eq!(dry_run_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_run_json["changes"][0]["after"]["x"], Value::from(80.0));
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after dry-run"),
        source_before
    );

    let moved_project = temp.path().join("sales_project_moved");
    let moved_arg = moved_project.to_str().expect("moved project path");
    let mutation = run_powerbi(&[
        "report",
        "visuals",
        "set-position",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--x",
        "120",
        "--y",
        "140",
        "--width",
        "360",
        "--height",
        "220",
        "--z",
        "5",
        "--out-dir",
        moved_arg,
        "--json",
    ]);
    assert_eq!(mutation.code, 0, "stderr: {}", mutation.stderr);
    let mutation_json = stdout_json(&mutation);
    assert_eq!(mutation_json["mode"], Value::from("out-dir"));
    assert_eq!(mutation_json["ok"], Value::Bool(true));
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after out-dir"),
        source_before
    );

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        moved_arg,
        "--handle",
        &visual_handle,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(readback_json["visual"]["position"]["x"], Value::from(120.0));
    assert_eq!(readback_json["visual"]["position"]["y"], Value::from(140.0));
    assert_eq!(
        readback_json["visual"]["position"]["width"],
        Value::from(360.0)
    );
    assert_eq!(readback_json["visual"]["position"]["z"], Value::from(5));
}

#[test]
fn report_visual_set_position_rejects_unsafe_geometry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();

    let missing_mode = run_powerbi(&[
        "report",
        "visuals",
        "set-position",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--x",
        "10",
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --dry-run")
    );

    let negative = run_powerbi(&[
        "report",
        "visuals",
        "set-position",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--x",
        "-1",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(negative.code, 2);
    assert!(
        stderr_json(&negative)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("nonnegative")
    );

    let oversized = run_powerbi(&[
        "report",
        "visuals",
        "set-position",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--x",
        "0",
        "--y",
        "0",
        "--width",
        "10000",
        "--height",
        "10000",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(oversized.code, 2);
    assert!(
        stderr_json(&oversized)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("outside page bounds")
    );
}

#[test]
fn report_visuals_catalog_advertises_generated_types_roles_and_limits() {
    let catalog = run_powerbi(&["report", "visuals", "catalog", "--json"]);
    assert_eq!(catalog.code, 0, "stderr: {}", catalog.stderr);
    let catalog_json = stdout_json(&catalog);
    assert_eq!(
        catalog_json["schema"],
        Value::from("powerbi-cli.report.visuals.catalog.v1")
    );
    let supported = catalog_json["supportedVisualTypes"]
        .as_array()
        .expect("supported types");
    assert!(supported.iter().any(|value| value == "card"));
    assert!(supported.iter().any(|value| value == "areaChart"));
    assert!(supported.iter().any(|value| value == "barChart"));
    assert!(supported.iter().any(|value| value == "scatterChart"));
    assert!(supported.iter().any(|value| value == "pieChart"));
    assert!(supported.iter().any(|value| value == "donutChart"));
    assert!(supported.iter().any(|value| value == "pivotTable"));
    assert!(supported.iter().any(|value| value == "slicer"));
    assert!(
        catalog_json["templateOnlyVisualTypes"]
            .as_array()
            .expect("template only")
            .iter()
            .all(|value| value["visualType"] != "slicer")
    );
    assert!(
        catalog_json["plannedVisualTypes"]
            .as_array()
            .expect("planned")
            .iter()
            .all(|value| !matches!(
                value["visualType"].as_str(),
                Some("pieChart" | "donutChart" | "matrix" | "pivotTable" | "slicer")
            ))
    );

    let line = run_powerbi(&["report", "visuals", "types", "--type", "line", "--json"]);
    assert_eq!(line.code, 0, "stderr: {}", line.stderr);
    let line_json = stdout_json(&line);
    assert_eq!(line_json["generatedVisualTypeCount"], Value::from(1));
    assert_eq!(
        line_json["visualTypes"][0]["visualType"],
        Value::from("lineChart")
    );
    let roles = line_json["visualTypes"][0]["roles"]
        .as_array()
        .expect("roles");
    assert!(roles.iter().any(|role| role["role"] == "Category"));
    assert!(roles.iter().any(|role| role["role"] == "Y"));
    assert!(roles.iter().any(|role| role["role"] == "Series"));
    assert!(roles.iter().any(|role| role["role"] == "Tooltips"));
    assert_eq!(
        roles
            .iter()
            .find(|role| role["role"] == "Y")
            .expect("line Y role")["fieldKinds"],
        json!(["measure"])
    );

    let scatter = run_powerbi(&["report", "visuals", "types", "--type", "bubble", "--json"]);
    assert_eq!(scatter.code, 0, "stderr: {}", scatter.stderr);
    let scatter_json = stdout_json(&scatter);
    assert_eq!(
        scatter_json["visualTypes"][0]["visualType"],
        Value::from("scatterChart")
    );
    let scatter_roles = scatter_json["visualTypes"][0]["roles"]
        .as_array()
        .expect("scatter roles");
    assert!(scatter_roles.iter().any(|role| role["role"] == "X"));
    assert!(scatter_roles.iter().any(|role| role["role"] == "Y"));
    assert!(scatter_roles.iter().any(|role| role["role"] == "Size"));
    assert!(scatter_roles.iter().any(|role| role["role"] == "Series"));
    assert!(scatter_roles.iter().all(|role| role["role"] != "Legend"));
    assert!(scatter_roles.iter().any(|role| role["role"] == "Tooltips"));
    for role_name in ["X", "Y", "Size"] {
        assert_eq!(
            scatter_roles
                .iter()
                .find(|role| role["role"] == role_name)
                .expect("scatter value role")["fieldKinds"],
            json!(["measure"])
        );
    }
    assert!(
        catalog_json["plannedVisualTypes"]
            .as_array()
            .expect("planned")
            .iter()
            .all(|value| value["visualType"] != "scatterChart")
    );

    let pie = run_powerbi(&["report", "visuals", "catalog", "--type", "pie", "--json"]);
    assert_eq!(pie.code, 0, "stderr: {}", pie.stderr);
    let pie_json = stdout_json(&pie);
    assert_eq!(pie_json["visualTypes"][0]["visualType"], "pieChart");
    assert_eq!(pie_json["visualTypes"][0]["bindingFamily"], "categoryShare");
    assert_eq!(
        pie_json["visualTypes"][0]["proofLevel"],
        "desktop-golden-pending"
    );
    assert_eq!(
        pie_json["visualTypes"][0]["bindingProofLevel"],
        "manual-desktop-canvas-refresh"
    );
    let pie_roles = pie_json["visualTypes"][0]["roles"]
        .as_array()
        .expect("pie roles");
    assert_eq!(pie_roles.len(), 2);
    assert!(
        pie_roles
            .iter()
            .any(|role| { role["role"] == "Category" && role["min"] == 1 && role["max"] == 1 })
    );
    assert_eq!(
        pie_roles
            .iter()
            .find(|role| role["role"] == "Y")
            .expect("pie Y role")["fieldKinds"],
        json!(["measure"])
    );

    let matrix = run_powerbi(&["report", "visuals", "catalog", "--type", "matrix", "--json"]);
    assert_eq!(matrix.code, 0, "stderr: {}", matrix.stderr);
    let matrix_json = stdout_json(&matrix);
    assert_eq!(matrix_json["visualTypes"][0]["visualType"], "pivotTable");
    assert_eq!(
        matrix_json["visualTypes"][0]["proofLevel"],
        "desktop-golden-pending"
    );
    assert_eq!(
        matrix_json["visualTypes"][0]["bindingProofLevel"],
        "manual-desktop-canvas-refresh"
    );
    let matrix_roles = matrix_json["visualTypes"][0]["roles"]
        .as_array()
        .expect("matrix roles");
    assert!(matrix_roles.iter().any(|role| role["role"] == "Rows"));
    assert!(matrix_roles.iter().any(|role| role["role"] == "Columns"));
    assert!(matrix_roles.iter().any(|role| role["role"] == "Values"));
    assert_eq!(
        matrix_roles
            .iter()
            .find(|role| role["role"] == "Values")
            .expect("matrix Values role")["fieldKinds"],
        json!(["measure"])
    );

    let slicer = run_powerbi(&["report", "visuals", "catalog", "--type", "slicer", "--json"]);
    assert_eq!(slicer.code, 0, "stderr: {}", slicer.stderr);
    let slicer_json = stdout_json(&slicer);
    assert_eq!(
        slicer_json["visualTypes"][0]["bindingFamily"],
        "slicerField"
    );
    assert_eq!(
        slicer_json["visualTypes"][0]["proofLevel"],
        "desktop-golden-pending"
    );
    assert_eq!(
        slicer_json["visualTypes"][0]["bindingProofLevel"],
        "manual-desktop-canvas-refresh"
    );
    assert_eq!(
        slicer_json["visualTypes"][0]["modes"],
        json!(["Basic", "Dropdown", "Between"])
    );
    assert_eq!(slicer_json["visualTypes"][0]["roles"][0]["max"], 1);
    assert_eq!(
        slicer_json["visualTypes"][0]["roles"][0]["fieldKinds"],
        json!(["column"])
    );

    let unsupported = run_powerbi(&[
        "report",
        "visuals",
        "catalog",
        "--visual-type",
        "map",
        "--json",
    ]);
    assert_eq!(unsupported.code, 2);
    let error = stderr_json(&unsupported);
    assert_eq!(error["error"]["code"], Value::from("unsupported_feature"));
    assert!(
        error["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("report visuals catalog")
    );
}

#[test]
fn report_visual_add_supports_series_and_scatter_bubble_roles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let page_handle = stdout_json(&pages)["pages"][0]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();

    let line_dry_run = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "line",
        "--title",
        "Revenue by Segment",
        "--binding",
        "role=Category,table=DimDate,column=Month",
        "--binding",
        "role=legend,table=DimCustomer,column=Segment",
        "--binding",
        "role=Y,table=FactSales,measure=Total Revenue",
        "--binding",
        "role=tooltip,table=FactSales,measure=Total Units",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(line_dry_run.code, 0, "stderr: {}", line_dry_run.stderr);
    let line_json = stdout_json(&line_dry_run);
    assert!(
        line_json["bindingPlan"]["after"]
            .as_array()
            .expect("line bindings")
            .iter()
            .any(|binding| binding["role"] == "Series")
    );
    assert!(
        line_json["bindingPlan"]["after"]
            .as_array()
            .expect("line bindings")
            .iter()
            .any(|binding| binding["role"] == "Tooltips")
    );

    let scatter_base = build_scatter_bubble(temp.path());
    let scatter_base_arg = scatter_base.to_str().expect("scatter base path");
    let scatter_pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        scatter_base_arg,
        "--json",
    ]);
    assert_eq!(scatter_pages.code, 0, "stderr: {}", scatter_pages.stderr);
    let scatter_page_handle = stdout_json(&scatter_pages)["pages"][0]["handle"]
        .as_str()
        .expect("scatter page handle")
        .to_string();
    let scatter_project = temp.path().join("scatter_project_added_visual");
    let scatter_arg = scatter_project.to_str().expect("scatter project path");
    let scatter = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        scatter_base_arg,
        "--page",
        &scatter_page_handle,
        "--visual-type",
        "bubble",
        "--title",
        "Revenue vs Units by Segment",
        "--binding",
        "role=Category,table=Facilities,column=Facility",
        "--binding",
        "role=X,table=Facilities,measure=Average Risk Score",
        "--binding",
        "role=Y,table=Facilities,measure=Average Incident Rate",
        "--binding",
        "role=Size,table=Facilities,measure=Total Exposure Hours",
        "--binding",
        "role=legend,table=Facilities,column=Region",
        "--binding",
        "role=Tooltips,table=Facilities,column=RiskScore",
        "--x",
        "40",
        "--y",
        "420",
        "--width",
        "500",
        "--height",
        "260",
        "--out-dir",
        scatter_arg,
        "--json",
    ]);
    assert_eq!(scatter.code, 0, "stderr: {}", scatter.stderr);
    let scatter_json = stdout_json(&scatter);
    assert_eq!(
        scatter_json["target"]["visualType"],
        Value::from("scatterChart")
    );
    let scatter_handle = scatter_json["target"]["handle"]
        .as_str()
        .expect("scatter handle")
        .to_string();

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        scatter_arg,
        "--handle",
        &scatter_handle,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(
        readback_json["visual"]["visualType"],
        Value::from("scatterChart")
    );
    let binding_roles = readback_json["visual"]["bindings"]
        .as_array()
        .expect("scatter bindings")
        .iter()
        .map(|binding| binding["role"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(binding_roles.contains(&"Category".to_string()));
    assert!(binding_roles.contains(&"X".to_string()));
    assert!(binding_roles.contains(&"Y".to_string()));
    assert!(binding_roles.contains(&"Size".to_string()));
    assert!(binding_roles.contains(&"Series".to_string()));
    assert!(!binding_roles.contains(&"Legend".to_string()));
    assert!(binding_roles.contains(&"Tooltips".to_string()));

    let visual_json_path = PathBuf::from(
        scatter_json["target"]["path"]
            .as_str()
            .expect("scatter target path"),
    );
    let visual_json: Value =
        serde_json::from_str(&fs::read_to_string(visual_json_path).expect("visual json"))
            .expect("parse visual json");
    assert!(visual_json["visual"]["query"]["queryState"]["X"].is_object());
    assert!(visual_json["visual"]["query"]["queryState"]["Y"].is_object());
    assert!(visual_json["visual"]["query"]["queryState"]["Size"].is_object());
    assert!(visual_json["visual"]["query"]["queryState"]["Series"].is_object());
    assert!(visual_json["visual"]["query"]["queryState"]["Legend"].is_null());
}

#[test]
fn validate_rejects_stale_scatter_legend_role_with_series_repair() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_scatter_bubble(temp.path());
    let listed = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    let listed_json = stdout_json(&listed);
    let visual_path = PathBuf::from(
        listed_json["visuals"]
            .as_array()
            .expect("visuals")
            .iter()
            .find(|visual| visual["visualType"] == "scatterChart")
            .and_then(|visual| visual["path"].as_str())
            .expect("scatter path"),
    );
    patch_json(&visual_path, |visual| {
        let series = visual["visual"]["query"]["queryState"]
            .as_object_mut()
            .expect("queryState")
            .remove("Series")
            .expect("Series role");
        visual["visual"]["query"]["queryState"]["Legend"] = series;
    });

    let output = run_powerbi(&[
        "validate",
        "--strict",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let output_json = stdout_json(&output);
    assert!(
        output_json["errors"]
            .as_array()
            .expect("errors")
            .iter()
            .any(|error| error.as_str().is_some_and(|message| {
                message.contains("queryState role `Legend`") && message.contains("use `Series`")
            }))
    );
}

#[test]
fn validate_reports_empty_visual_directory_with_repair_hint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let empty_visual = first_page_json(&project)
        .parent()
        .expect("page dir")
        .join("visuals")
        .join("deleted_visual_leftover");
    fs::create_dir_all(&empty_visual).expect("empty visual dir");

    let output = run_powerbi(&[
        "validate",
        "--strict",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    assert!(
        stdout_json(&output)["errors"]
            .as_array()
            .expect("errors")
            .iter()
            .any(|error| error.as_str().is_some_and(|message| {
                message.contains("visual directory is missing visual.json")
                    && message.contains("Remove the empty visual directory")
            }))
    );
}

#[test]
fn report_visual_add_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let page_handle = stdout_json(&pages)["pages"][0]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();
    let source_visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(source_visuals.code, 0, "stderr: {}", source_visuals.stderr);
    assert_eq!(
        stdout_json(&source_visuals)["counts"]["visuals"],
        Value::from(3)
    );

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "card",
        "--title",
        "Margin KPI",
        "--binding",
        "role=Values,table=FactSales,measure=Total Revenue",
        "--x",
        "40",
        "--y",
        "560",
        "--width",
        "260",
        "--height",
        "120",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.visuals.mutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["visualPlan"]["nameGenerated"], Value::Bool(true));
    assert_eq!(
        dry_json["bindingPlan"]["after"][0]["measure"],
        Value::from("Total Revenue")
    );
    let dry_path = PathBuf::from(
        dry_json["target"]["path"]
            .as_str()
            .expect("dry target path"),
    );
    assert!(
        !dry_path.exists(),
        "dry-run should not create {}",
        dry_path.display()
    );

    let added_project = temp.path().join("sales_project_visual_added");
    let added_arg = added_project.to_str().expect("added project path");
    let add = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "card",
        "--title",
        "Margin KPI",
        "--binding",
        "role=Values,table=FactSales,measure=Total Revenue",
        "--x",
        "40",
        "--y",
        "560",
        "--width",
        "260",
        "--height",
        "120",
        "--out-dir",
        added_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    assert_eq!(add_json["ok"], Value::Bool(true));
    assert_eq!(add_json["mode"], Value::from("out-dir"));
    let added_visual_path = PathBuf::from(
        add_json["target"]["path"]
            .as_str()
            .expect("added visual path"),
    );
    let added_visual_json: Value =
        serde_json::from_str(&fs::read_to_string(&added_visual_path).expect("added visual json"))
            .expect("parse added visual json");
    assert_eq!(
        added_visual_json["visual"]["visualContainerObjects"]["title"][0]["properties"]["text"]["expr"]
            ["Literal"]["Value"],
        "'Margin KPI'"
    );
    assert_eq!(
        added_visual_json["visual"]["visualContainerObjects"]["title"][0]["properties"]["show"]["expr"]
            ["Literal"]["Value"],
        "true"
    );
    assert!(added_visual_json.get("visualContainerObjects").is_none());
    assert!(added_visual_json.get("objects").is_none());
    let new_handle = add_json["target"]["handle"]
        .as_str()
        .expect("new visual handle")
        .to_string();
    let source_after = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(source_after.code, 0, "stderr: {}", source_after.stderr);
    assert_eq!(
        stdout_json(&source_after)["counts"]["visuals"],
        Value::from(3)
    );

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        added_arg,
        "--handle",
        &new_handle,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(readback_json["visual"]["title"], Value::from("Margin KPI"));
    assert_eq!(readback_json["visual"]["visualType"], Value::from("card"));
    assert_eq!(
        readback_json["visual"]["bindings"][0]["measure"],
        Value::from("Total Revenue")
    );

    let added_visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        added_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(added_visuals.code, 0, "stderr: {}", added_visuals.stderr);
    let added_visuals_json = stdout_json(&added_visuals);
    assert_eq!(added_visuals_json["counts"]["visuals"], Value::from(4));
    assert_eq!(added_visuals_json["counts"]["boundVisuals"], Value::from(4));

    let validate = run_powerbi(&["validate", "--strict", added_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    let validate_json = stdout_json(&validate);
    assert_eq!(validate_json["counts"]["visuals"], Value::from(4));
    assert_eq!(validate_json["counts"]["boundVisuals"], Value::from(4));
}

#[test]
fn report_visual_add_defaults_require_a_binding_and_create_alias_is_readable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let page_handle = stdout_json(&pages)["pages"][0]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();

    let created_project = temp.path().join("sales_project_visual_created");
    let created_arg = created_project.to_str().expect("created project path");
    let create = run_powerbi(&[
        "report",
        "visuals",
        "create",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--title",
        "Scratch Card",
        "--binding",
        "role=Values,table=FactSales,measure=Total Revenue",
        "--out-dir",
        created_arg,
        "--json",
    ]);
    assert_eq!(create.code, 0, "stderr: {}", create.stderr);
    let create_json = stdout_json(&create);
    assert_eq!(
        create_json["schema"],
        Value::from("powerbi-cli.report.visuals.mutation.v1")
    );
    assert_eq!(create_json["target"]["visualType"], Value::from("card"));
    assert_eq!(
        create_json["target"]["position"]["width"],
        Value::from(320.0)
    );
    assert_eq!(
        create_json["target"]["position"]["height"],
        Value::from(180.0)
    );
    assert_eq!(create_json["target"]["bindingCount"], Value::from(1));
    assert_eq!(
        create_json["target"]["bindings"]
            .as_array()
            .expect("bindings")
            .len(),
        1
    );

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        created_arg,
        "--handle",
        create_json["target"]["handle"].as_str().expect("handle"),
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(
        readback_json["visual"]["title"],
        Value::from("Scratch Card")
    );
    assert_eq!(readback_json["visual"]["visualType"], Value::from("card"));
}

#[test]
fn report_visual_add_supports_catalog_chart_aliases() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let page_handle = stdout_json(&pages)["pages"][0]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();

    let chart_project = temp.path().join("sales_project_stacked_chart");
    let chart_arg = chart_project.to_str().expect("chart project path");
    let add = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "stackedbar",
        "--title",
        "Revenue by Segment",
        "--binding",
        "role=axis,table=DimCustomer,column=Segment",
        "--binding",
        "role=values,table=FactSales,measure=Total Revenue",
        "--x",
        "680",
        "--y",
        "32",
        "--width",
        "500",
        "--height",
        "280",
        "--out-dir",
        chart_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    assert_eq!(add_json["target"]["visualType"], Value::from("barChart"));
    assert_eq!(
        add_json["bindingPlan"]["after"][0]["role"],
        Value::from("Category")
    );
    assert_eq!(
        add_json["bindingPlan"]["after"][1]["role"],
        Value::from("Y")
    );
    let new_handle = add_json["target"]["handle"]
        .as_str()
        .expect("new visual handle")
        .to_string();

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        chart_arg,
        "--handle",
        &new_handle,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(
        readback_json["visual"]["visualType"],
        Value::from("barChart")
    );
    assert_eq!(
        readback_json["visual"]["bindings"][0]["column"],
        Value::from("Segment")
    );
    assert_eq!(
        readback_json["visual"]["bindings"][1]["measure"],
        Value::from("Total Revenue")
    );

    let validate = run_powerbi(&["validate", "--strict", chart_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["counts"]["visuals"], Value::from(4));
}

#[test]
fn report_visual_new_families_round_trip_add_format_bind_clone_and_delete() {
    struct CatalogVisualCase {
        slug: &'static str,
        requested_type: &'static str,
        canonical_type: &'static str,
        mode: Option<&'static str>,
        add_bindings: &'static [&'static str],
        replacement_bindings: &'static [&'static str],
        roles: &'static [&'static str],
    }

    let cases = [
        CatalogVisualCase {
            slug: "pie",
            requested_type: "pie",
            canonical_type: "pieChart",
            mode: None,
            add_bindings: &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,measure=Total Amount",
            ],
            replacement_bindings: &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,measure=Total Amount",
            ],
            roles: &["Category", "Y"],
        },
        CatalogVisualCase {
            slug: "donut",
            requested_type: "donut",
            canonical_type: "donutChart",
            mode: None,
            add_bindings: &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,measure=Total Amount",
            ],
            replacement_bindings: &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,measure=Total Amount",
            ],
            roles: &["Category", "Y"],
        },
        CatalogVisualCase {
            slug: "matrix",
            requested_type: "matrix",
            canonical_type: "pivotTable",
            mode: None,
            add_bindings: &[
                "role=Rows,table=CatalogFacts,column=Category",
                "role=Columns,table=CatalogFacts,column=Year",
                "role=Values,table=CatalogFacts,measure=Total Amount",
            ],
            replacement_bindings: &[
                "role=Rows,table=CatalogFacts,column=Category",
                "role=Columns,table=CatalogFacts,column=Year",
                "role=Values,table=CatalogFacts,measure=Total Amount",
            ],
            roles: &["Rows", "Columns", "Values"],
        },
        CatalogVisualCase {
            slug: "slicer",
            requested_type: "slicer",
            canonical_type: "slicer",
            mode: Some("dropdown"),
            add_bindings: &["role=Values,table=CatalogFacts,column=Category"],
            replacement_bindings: &["role=Values,table=CatalogFacts,column=Year"],
            roles: &["Values"],
        },
    ];

    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_catalog_proof(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page_handle = "page:ReportSectionLineControl";

    for case in cases {
        let added_project = temp.path().join(format!("{}_added", case.slug));
        let added_arg = added_project.to_str().expect("added path");
        let mut add_args = vec![
            "report".to_string(),
            "visuals".to_string(),
            "add".to_string(),
            "--project".to_string(),
            project_arg.to_string(),
            "--page".to_string(),
            page_handle.to_string(),
            "--visual-type".to_string(),
            case.requested_type.to_string(),
            "--title".to_string(),
            format!("{} Lifecycle", case.slug),
        ];
        if let Some(mode) = case.mode {
            add_args.extend(["--mode".to_string(), mode.to_string()]);
        }
        for binding in case.add_bindings {
            add_args.extend(["--binding".to_string(), (*binding).to_string()]);
        }
        add_args.extend([
            "--x".to_string(),
            "440".to_string(),
            "--y".to_string(),
            "300".to_string(),
            "--width".to_string(),
            "320".to_string(),
            "--height".to_string(),
            "160".to_string(),
            "--out-dir".to_string(),
            added_arg.to_string(),
            "--json".to_string(),
        ]);
        let add = run_powerbi_owned(&add_args);
        assert_eq!(add.code, 0, "{} add stderr: {}", case.slug, add.stderr);
        let add_json = stdout_json(&add);
        assert_eq!(add_json["target"]["visualType"], case.canonical_type);
        assert_eq!(
            add_json["target"]["mode"],
            case.mode
                .map(|mode| if mode == "dropdown" {
                    "Dropdown"
                } else {
                    "Basic"
                })
                .map(Value::from)
                .unwrap_or(Value::Null)
        );
        let handle = add_json["target"]["handle"]
            .as_str()
            .expect("added handle")
            .to_string();
        let visual_path = PathBuf::from(
            add_json["target"]["path"]
                .as_str()
                .expect("added visual path"),
        );
        let raw: Value = serde_json::from_str(
            &fs::read_to_string(&visual_path).expect("read added visual json"),
        )
        .expect("parse added visual json");
        assert!(
            raw.get("objects").is_none(),
            "{} emitted forbidden root-level objects",
            case.slug
        );
        assert!(
            raw.pointer("/visual/visualContainerObjects/general/0/properties/altText")
                .is_none(),
            "{} emitted validator-rejected general.altText",
            case.slug
        );
        for role in case.roles {
            assert!(
                raw["visual"]["query"]["queryState"][*role].is_object(),
                "{} missing role {role}",
                case.slug
            );
        }
        if matches!(case.canonical_type, "pieChart" | "donutChart") {
            assert_eq!(
                raw["visual"]["query"]["queryState"]["Category"]["projections"][0]["active"],
                Value::Bool(true)
            );
            assert_eq!(
                raw["visual"]["query"]["sortDefinition"]["sort"][0]["field"]["Measure"]["Property"],
                "Total Amount"
            );
            assert_eq!(
                raw["visual"]["query"]["sortDefinition"]["sort"][0]["direction"],
                "Descending"
            );
            assert_eq!(
                raw["visual"]["query"]["sortDefinition"]["isDefaultSort"],
                Value::Bool(true)
            );
        } else if case.canonical_type == "pivotTable" {
            assert_eq!(
                raw["visual"]["query"]["queryState"]["Rows"]["projections"][0]["active"],
                Value::Bool(true)
            );
            assert_eq!(
                raw["visual"]["query"]["queryState"]["Columns"]["projections"][0]["active"],
                Value::Bool(true)
            );
        } else {
            assert_eq!(
                raw["visual"]["query"]["queryState"]["Values"]["projections"][0]["active"],
                Value::Bool(true)
            );
            assert_eq!(
                raw["visual"]["objects"]["data"][0]["properties"]["mode"]["expr"]["Literal"]["Value"],
                "'Dropdown'"
            );
            assert!(
                raw["visual"]["objects"]["general"][0]["properties"]
                    .get("filter")
                    .is_none()
            );
            assert!(raw.get("filterConfig").is_none());
            assert!(raw.get("filters").is_none());
        }

        let show = run_powerbi(&[
            "report",
            "visuals",
            "show",
            "--project",
            added_arg,
            "--handle",
            &handle,
            "--json",
        ]);
        assert_eq!(show.code, 0, "{} show stderr: {}", case.slug, show.stderr);
        let show_json = stdout_json(&show);
        assert_eq!(show_json["visual"]["visualType"], case.canonical_type);
        assert_eq!(
            show_json["visual"]["bindings"]
                .as_array()
                .expect("added bindings")
                .len(),
            case.add_bindings.len()
        );

        let formatting_list = run_powerbi(&[
            "report",
            "visuals",
            "formatting",
            "list",
            "--project",
            added_arg,
            "--json",
        ]);
        assert_eq!(
            formatting_list.code, 0,
            "{} formatting list stderr: {}",
            case.slug, formatting_list.stderr
        );
        assert!(
            stdout_json(&formatting_list)["visuals"]
                .as_array()
                .expect("formatting visuals")
                .iter()
                .any(|visual| visual["handle"] == handle)
        );
        let formatting_show = run_powerbi(&[
            "report",
            "visuals",
            "formatting",
            "show",
            "--project",
            added_arg,
            "--handle",
            &handle,
            "--json",
        ]);
        assert_eq!(
            formatting_show.code, 0,
            "{} formatting show stderr: {}",
            case.slug, formatting_show.stderr
        );
        let object_names = stdout_json(&formatting_show)["formatting"]["objectNames"]
            .as_array()
            .expect("formatting object names")
            .clone();
        if case.canonical_type == "slicer" {
            assert!(object_names.iter().any(|name| name == "data"));
        }

        let bound_project = temp.path().join(format!("{}_bound", case.slug));
        let bound_arg = bound_project.to_str().expect("bound path");
        let mut bind_args = vec![
            "report".to_string(),
            "visuals".to_string(),
            "set-bindings".to_string(),
            "--project".to_string(),
            added_arg.to_string(),
            "--handle".to_string(),
            handle.clone(),
        ];
        for binding in case.replacement_bindings {
            bind_args.extend(["--binding".to_string(), (*binding).to_string()]);
        }
        bind_args.extend([
            "--out-dir".to_string(),
            bound_arg.to_string(),
            "--json".to_string(),
        ]);
        let bind = run_powerbi_owned(&bind_args);
        assert_eq!(bind.code, 0, "{} bind stderr: {}", case.slug, bind.stderr);
        let bind_json = stdout_json(&bind);
        assert_eq!(
            bind_json["bindingPlan"]["after"]
                .as_array()
                .expect("replacement bindings")
                .len(),
            case.replacement_bindings.len()
        );
        if matches!(case.canonical_type, "pieChart" | "donutChart") {
            assert_eq!(
                bind_json["changes"][0]["after"]["sortDefinition"]["sort"][0]["field"]["Measure"]["Property"],
                "Total Amount"
            );
        }

        let cloned_project = temp.path().join(format!("{}_cloned", case.slug));
        let cloned_arg = cloned_project.to_str().expect("cloned path");
        let clone = run_powerbi(&[
            "report",
            "visuals",
            "clone",
            "--project",
            bound_arg,
            "--handle",
            &handle,
            "--title",
            &format!("{} Clone", case.slug),
            "--x",
            "40",
            "--y",
            "300",
            "--width",
            "320",
            "--height",
            "160",
            "--out-dir",
            cloned_arg,
            "--json",
        ]);
        assert_eq!(
            clone.code, 0,
            "{} clone stderr: {}",
            case.slug, clone.stderr
        );
        let clone_json = stdout_json(&clone);
        let clone_handle = clone_json["target"]["handle"]
            .as_str()
            .expect("clone handle")
            .to_string();
        assert_eq!(clone_json["target"]["visualType"], case.canonical_type);

        let clone_show = run_powerbi(&[
            "report",
            "visuals",
            "show",
            "--project",
            cloned_arg,
            "--handle",
            &clone_handle,
            "--json",
        ]);
        assert_eq!(
            clone_show.code, 0,
            "{} clone show stderr: {}",
            case.slug, clone_show.stderr
        );
        assert_eq!(
            stdout_json(&clone_show)["visual"]["bindings"]
                .as_array()
                .expect("clone bindings")
                .len(),
            case.replacement_bindings.len()
        );

        if case.canonical_type == "slicer" {
            let slicers = run_powerbi(&[
                "report",
                "slicers",
                "list",
                "--project",
                cloned_arg,
                "--json",
            ]);
            assert_eq!(slicers.code, 0, "slicer list stderr: {}", slicers.stderr);
            assert_eq!(
                stdout_json(&slicers)["counts"]["possibleDataValueSlicers"],
                0
            );
            let audit = run_powerbi(&["report", "audit", "--project", cloned_arg, "--json"]);
            assert_eq!(audit.code, 0, "slicer audit stderr: {}", audit.stderr);
            assert!(
                stdout_json(&audit)["findings"]
                    .as_array()
                    .expect("audit findings")
                    .iter()
                    .all(|finding| finding["ruleId"] != "slicer.possible_persisted_values")
            );
        }

        let deleted_project = temp.path().join(format!("{}_deleted", case.slug));
        let deleted_arg = deleted_project.to_str().expect("deleted path");
        let delete = run_powerbi(&[
            "report",
            "visuals",
            "delete",
            "--project",
            cloned_arg,
            "--handle",
            &clone_handle,
            "--out-dir",
            deleted_arg,
            "--json",
        ]);
        assert_eq!(
            delete.code, 0,
            "{} delete stderr: {}",
            case.slug, delete.stderr
        );
        let list_after = run_powerbi(&[
            "report",
            "visuals",
            "list",
            "--project",
            deleted_arg,
            "--json",
        ]);
        assert_eq!(
            list_after.code, 0,
            "{} list after delete stderr: {}",
            case.slug, list_after.stderr
        );
        assert!(
            stdout_json(&list_after)["visuals"]
                .as_array()
                .expect("visuals after delete")
                .iter()
                .all(|visual| visual["handle"] != clone_handle)
        );
        assert_strict_valid(&deleted_project);
    }
}

#[test]
fn report_visual_clone_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let source_visual = visuals_json["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .expect("card visual");
    let source_handle = source_visual["handle"]
        .as_str()
        .expect("source handle")
        .to_string();
    let page_handle = source_visual["page"]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();
    let source_path = PathBuf::from(source_visual["path"].as_str().expect("source path"));
    let source_before = fs::read_to_string(&source_path).expect("source before");

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "clone",
        "--project",
        project_arg,
        "--handle",
        &source_handle,
        "--title",
        "Revenue Clone",
        "--x",
        "420",
        "--y",
        "40",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.visuals.cloneMutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["target"]["title"], Value::from("Revenue Clone"));
    assert_eq!(dry_json["target"]["visualType"], Value::from("card"));
    assert_eq!(dry_json["clonePlan"]["copiedSidecars"], Value::Bool(false));
    let dry_path = PathBuf::from(dry_json["target"]["path"].as_str().expect("dry clone path"));
    assert!(
        !dry_path.exists(),
        "dry-run should not create {}",
        dry_path.display()
    );

    let cloned_project = temp.path().join("sales_project_cloned_visual");
    let cloned_arg = cloned_project.to_str().expect("cloned project path");
    let clone = run_powerbi(&[
        "report",
        "visuals",
        "duplicate",
        "--project",
        project_arg,
        "--handle",
        &source_handle,
        "--target-page",
        &page_handle,
        "--title",
        "Revenue Clone",
        "--x",
        "420",
        "--y",
        "40",
        "--out-dir",
        cloned_arg,
        "--json",
    ]);
    assert_eq!(clone.code, 0, "stderr: {}", clone.stderr);
    let clone_json = stdout_json(&clone);
    assert_eq!(clone_json["ok"], Value::Bool(true));
    assert_eq!(clone_json["mode"], Value::from("out-dir"));
    let clone_handle = clone_json["target"]["handle"]
        .as_str()
        .expect("clone handle")
        .to_string();
    assert_ne!(clone_handle, source_handle);
    let cloned_visual_path =
        PathBuf::from(clone_json["target"]["path"].as_str().expect("clone path"));
    let cloned_visual_json: Value =
        serde_json::from_str(&fs::read_to_string(&cloned_visual_path).expect("cloned visual.json"))
            .expect("parse cloned visual.json");
    assert_eq!(
        cloned_visual_json
            .pointer("/visual/visualContainerObjects/title/0/properties/text/expr/Literal/Value"),
        Some(&Value::from("'Revenue Clone'")),
        "--title must update the visible Power BI title"
    );
    assert_eq!(
        cloned_visual_json
            .pointer("/visual/visualContainerObjects/title/0/properties/show/expr/Literal/Value"),
        Some(&Value::from("true")),
        "a cloned title must be visible"
    );
    assert!(
        cloned_visual_json["annotations"]
            .as_array()
            .expect("clone annotations")
            .iter()
            .any(|annotation| {
                annotation["name"] == "powerbi-cli.placeholderTitle"
                    && annotation["value"] == "Revenue Clone"
            }),
        "--title must keep the title annotation in sync"
    );
    assert_eq!(
        fs::read_to_string(&source_path).expect("source after clone"),
        source_before,
        "out-dir clone must not modify source project"
    );

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        cloned_arg,
        "--handle",
        &clone_handle,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(
        readback_json["visual"]["title"],
        Value::from("Revenue Clone")
    );
    assert_eq!(readback_json["visual"]["visualType"], Value::from("card"));
    assert_eq!(readback_json["visual"]["position"]["x"], Value::from(420.0));
    assert_eq!(readback_json["visual"]["position"]["y"], Value::from(40.0));
    assert_eq!(readback_json["visual"]["position"]["z"], Value::from(3));
    assert_eq!(
        readback_json["visual"]["position"]["tabOrder"],
        Value::from(3)
    );

    let cloned_visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        cloned_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(cloned_visuals.code, 0, "stderr: {}", cloned_visuals.stderr);
    assert_eq!(
        stdout_json(&cloned_visuals)["counts"]["visuals"],
        Value::from(4)
    );

    let validate = run_powerbi(&["validate", "--strict", cloned_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["counts"]["visuals"], Value::from(4));
}

#[test]
fn report_visual_clone_preserves_desktop_authored_slicer_template_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_slicer_fixture(&project);
    let project_arg = project.to_str().expect("project path");
    let slicers = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(slicers.code, 0, "stderr: {}", slicers.stderr);
    let slicers_json = stdout_json(&slicers);
    assert_eq!(slicers_json["counts"]["slicers"], Value::from(1));
    let source_visual_handle = slicers_json["slicers"][0]["visualHandle"]
        .as_str()
        .expect("source slicer visual handle")
        .to_string();

    let cloned_project = temp.path().join("sales_project_cloned_slicer");
    let cloned_arg = cloned_project.to_str().expect("cloned project path");
    let clone = run_powerbi(&[
        "report",
        "visuals",
        "clone",
        "--project",
        project_arg,
        "--handle",
        &source_visual_handle,
        "--title",
        "Region Slicer Copy",
        "--name",
        "VisualContainerRegionSlicerCopy",
        "--x",
        "20",
        "--y",
        "300",
        "--out-dir",
        cloned_arg,
        "--json",
    ]);
    assert_eq!(clone.code, 0, "stderr: {}", clone.stderr);
    let clone_json = stdout_json(&clone);
    let slicer_readback = clone_json["slicerReadbackCommand"]
        .as_str()
        .expect("slicer readback");
    assert!(slicer_readback.contains("report slicers show"));
    assert!(
        slicer_readback.contains("slicer:ReportSectionOverview:VisualContainerRegionSlicerCopy")
    );

    let cloned_slicers = run_powerbi(&[
        "report",
        "slicers",
        "list",
        "--project",
        cloned_arg,
        "--json",
    ]);
    assert_eq!(cloned_slicers.code, 0, "stderr: {}", cloned_slicers.stderr);
    let cloned_slicers_json = stdout_json(&cloned_slicers);
    assert_eq!(cloned_slicers_json["counts"]["slicers"], Value::from(2));
    let cloned_slicer = cloned_slicers_json["slicers"]
        .as_array()
        .expect("slicers")
        .iter()
        .find(|slicer| slicer["name"] == "VisualContainerRegionSlicerCopy")
        .expect("cloned slicer");
    assert_eq!(cloned_slicer["title"], Value::from("Region Slicer Copy"));
    assert_eq!(cloned_slicer["visualType"], Value::from("slicer"));
    assert_eq!(cloned_slicer["target"]["table"], Value::from("DimRegion"));
    assert_eq!(cloned_slicer["target"]["column"], Value::from("Region"));

    let validate = run_powerbi(&["validate", "--strict", cloned_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["counts"]["visuals"], Value::from(4));
}

#[test]
fn report_visual_clone_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let source_visual = &visuals_json["visuals"].as_array().expect("visuals")[0];
    let source_handle = source_visual["handle"]
        .as_str()
        .expect("source handle")
        .to_string();
    let source_name = source_visual["name"]
        .as_str()
        .expect("source name")
        .to_string();

    let missing_mode = run_powerbi(&[
        "report",
        "visuals",
        "clone",
        "--project",
        project_arg,
        "--handle",
        &source_handle,
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --dry-run")
    );

    let duplicate_name = run_powerbi(&[
        "report",
        "visuals",
        "clone",
        "--project",
        project_arg,
        "--handle",
        &source_handle,
        "--name",
        &source_name,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(duplicate_name.code, 2);
    assert!(
        stderr_json(&duplicate_name)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("visual already exists")
    );

    let source_path = PathBuf::from(source_visual["path"].as_str().expect("source path"));
    fs::write(
        source_path
            .parent()
            .expect("visual dir")
            .join("sidecar.json"),
        "{}",
    )
    .expect("write sidecar");
    let sidecar = run_powerbi(&[
        "report",
        "visuals",
        "clone",
        "--project",
        project_arg,
        "--handle",
        &source_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(sidecar.code, 2);
    assert_unsupported_feature(&sidecar.stderr, "simple visual containers only");
}

#[test]
fn report_visual_add_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let pages = run_powerbi(&[
        "report",
        "pages",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(pages.code, 0, "stderr: {}", pages.stderr);
    let page_handle = stdout_json(&pages)["pages"][0]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();
    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let existing_visual_name = stdout_json(&visuals)["visuals"][0]["name"]
        .as_str()
        .expect("existing visual name")
        .to_string();

    let missing_mode = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "card",
        "--title",
        "Unsafe",
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --dry-run")
    );

    let missing_page = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        "page:MissingPage",
        "--visual-type",
        "card",
        "--title",
        "Missing Page",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_page.code, 2);
    let missing_page_error = stderr_json(&missing_page);
    assert!(
        missing_page_error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("page not found")
    );
    let suggested = missing_page_error["error"]["suggestedCommands"][0]
        .as_str()
        .expect("suggested command");
    assert!(suggested.contains("--page <page-handle>"));
    assert!(!suggested.contains("--handle <page-handle>"));

    let unsupported_type = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "map",
        "--title",
        "Unsupported",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unsupported_type.code, 2);
    let unsupported_type_json = stderr_json(&unsupported_type);
    assert_eq!(
        unsupported_type_json["error"]["code"],
        Value::from("unsupported_feature")
    );
    assert!(
        unsupported_type_json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unsupported visual type")
    );

    let bad_role = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "card",
        "--title",
        "Bad Role",
        "--binding",
        "role=Category,table=DimCustomer,column=Segment",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(bad_role.code, 2);
    assert_unsupported_feature(&bad_role.stderr, "unsupported role");

    let outside_page = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "card",
        "--title",
        "Too Far",
        "--x",
        "2000",
        "--y",
        "40",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(outside_page.code, 2);
    assert!(
        stderr_json(&outside_page)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("outside page bounds")
    );

    let duplicate_name = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "card",
        "--title",
        "Duplicate",
        "--name",
        &existing_visual_name,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(duplicate_name.code, 2);
    assert!(
        stderr_json(&duplicate_name)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("visual already exists")
    );

    let unsafe_name = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        &page_handle,
        "--visual-type",
        "card",
        "--title",
        "Unsafe Name",
        "--name",
        "../BadVisual",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unsafe_name.code, 2);
    assert!(
        stderr_json(&unsafe_name)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unsafe visual name")
    );
}

#[test]
fn report_visual_new_families_reject_invalid_bindings_and_slicer_modes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_catalog_proof(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = "page:ReportSectionLineControl";

    let pie_categories = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "pie",
        "--title",
        "Invalid Pie",
        "--binding",
        "role=Category,table=CatalogFacts,column=Category",
        "--binding",
        "role=Category,table=CatalogFacts,column=Year",
        "--binding",
        "role=Y,table=CatalogFacts,measure=Total Amount",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(pie_categories.code, 2);
    assert!(
        stderr_json(&pie_categories)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("exactly one Category column binding")
    );

    let matrix_values = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "matrix",
        "--title",
        "Invalid Matrix",
        "--binding",
        "role=Rows,table=CatalogFacts,column=Category",
        "--binding",
        "role=Columns,table=CatalogFacts,column=Year",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(matrix_values.code, 2);
    assert!(
        stderr_json(&matrix_values)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("at least one Values binding")
    );

    let slicer_measure = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "slicer",
        "--title",
        "Invalid Slicer",
        "--binding",
        "role=Values,table=CatalogFacts,measure=Total Amount",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(slicer_measure.code, 2);
    assert!(
        stderr_json(&slicer_measure)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("exactly one Values column binding")
    );

    let between_mode = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "slicer",
        "--mode",
        "between",
        "--name",
        "BetweenYearSlicer",
        "--title",
        "Year range",
        "--binding",
        "role=Values,table=CatalogFacts,column=Year",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(between_mode.code, 0, "stderr: {}", between_mode.stderr);
    let between_json = stdout_json(&between_mode);
    assert_eq!(between_json["target"]["mode"], "Between");
    assert_eq!(
        between_json["changes"][0]["after"]["visual"]["objects"]["data"][0]["properties"]["mode"]["expr"]
            ["Literal"]["Value"],
        "'Between'"
    );

    let between_text = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "slicer",
        "--mode",
        "between",
        "--title",
        "Invalid text range",
        "--binding",
        "role=Values,table=CatalogFacts,column=Category",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(between_text.code, 2);
    let between_text_error = stderr_json(&between_text);
    assert_eq!(between_text_error["error"]["code"], "unsupported_feature");
    assert!(
        between_text_error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires a numeric or date column")
    );

    let slicer_mode = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "slicer",
        "--mode",
        "relative",
        "--title",
        "Unsupported Mode",
        "--binding",
        "role=Values,table=CatalogFacts,column=Category",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(slicer_mode.code, 2);
    let mode_error = stderr_json(&slicer_mode);
    assert_eq!(mode_error["error"]["code"], "unsupported_feature");
    assert!(
        mode_error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unsupported slicer mode")
    );

    let non_slicer_mode = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "pie",
        "--mode",
        "basic",
        "--title",
        "Wrong Mode Surface",
        "--binding",
        "role=Category,table=CatalogFacts,column=Category",
        "--binding",
        "role=Y,table=CatalogFacts,measure=Total Amount",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(non_slicer_mode.code, 2);
    assert!(
        stderr_json(&non_slicer_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("only when --visual-type is slicer")
    );

    let default_mode = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "slicer",
        "--title",
        "Default Basic Slicer",
        "--binding",
        "role=Values,table=CatalogFacts,column=Category",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(default_mode.code, 0, "stderr: {}", default_mode.stderr);
    let default_json = stdout_json(&default_mode);
    assert_eq!(default_json["target"]["mode"], "Basic");
    assert_eq!(
        default_json["changes"][0]["after"]["visual"]["objects"]["data"][0]["properties"]["mode"]["expr"]
            ["Literal"]["Value"],
        "'Basic'"
    );
    assert!(default_json["changes"][0]["after"].get("objects").is_none());
}

#[test]
fn report_visuals_reject_unproven_value_columns_and_duplicate_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_catalog_proof(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = "page:ReportSectionLineControl";

    let cases: &[(&str, &[&str])] = &[
        ("card", &["role=Values,table=CatalogFacts,column=Amount"]),
        (
            "line",
            &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,column=Amount",
            ],
        ),
        (
            "pie",
            &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,column=Amount",
            ],
        ),
        (
            "matrix",
            &[
                "role=Rows,table=CatalogFacts,column=Category",
                "role=Values,table=CatalogFacts,column=Amount",
            ],
        ),
        ("scatter", &["role=X,table=CatalogFacts,column=Amount"]),
    ];
    for (visual_type, bindings) in cases {
        let mut args = vec![
            "report".to_string(),
            "visuals".to_string(),
            "add".to_string(),
            "--project".to_string(),
            project_arg.to_string(),
            "--page".to_string(),
            page.to_string(),
            "--visual-type".to_string(),
            (*visual_type).to_string(),
            "--title".to_string(),
            format!("Rejected {visual_type}"),
        ];
        for binding in *bindings {
            args.extend(["--binding".to_string(), (*binding).to_string()]);
        }
        args.extend(["--dry-run".to_string(), "--json".to_string()]);
        let output = run_powerbi_owned(&args);
        assert_eq!(output.code, 2, "{visual_type} stderr: {}", output.stderr);
        let error = stderr_json(&output);
        assert_eq!(error["error"]["code"], "unsupported_feature");
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("raw column bindings are not Desktop-proven"),
            "{visual_type}: {error}"
        );
        assert!(
            error["error"]["hint"]
                .as_str()
                .unwrap_or_default()
                .contains("Define a measure")
        );
    }

    let proven_table_column = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "table",
        "--title",
        "Proven Detail Column",
        "--binding",
        "role=Values,table=CatalogFacts,column=Amount",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        proven_table_column.code, 0,
        "stderr: {}",
        proven_table_column.stderr
    );
    assert_eq!(
        stdout_json(&proven_table_column)["bindingPlan"]["after"][0]["kind"],
        "column"
    );

    let duplicate = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "scatter",
        "--title",
        "Duplicate Measure",
        "--binding",
        "role=X,table=CatalogFacts,measure=Total Amount",
        "--binding",
        "role=Y,table=CatalogFacts,measure=Total Amount",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(duplicate.code, 2);
    let duplicate_error = stderr_json(&duplicate);
    assert_eq!(
        duplicate_error["error"]["code"],
        Value::from("unsupported_feature")
    );
    assert!(
        duplicate_error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("duplicate visual field usage is not Desktop-proven")
    );
    assert!(
        duplicate_error["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("duplicate queryRef/nativeQueryRef numbering")
    );
}

#[test]
fn report_visual_set_bindings_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let visual_path = PathBuf::from(
        visuals_json["visuals"][0]["path"]
            .as_str()
            .expect("visual path"),
    );
    let source_before = fs::read_to_string(&visual_path).expect("source visual before");

    let bindings_json = serde_json::to_string(&json!([
        {
            "role": "Values",
            "table": "FactSales",
            "measure": "Total Revenue",
            "displayName": "Revenue KPI"
        }
    ]))
    .expect("bindings json");

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--bindings-json",
        &bindings_json,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.visuals.bindingMutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        dry_json["bindingPlan"]["after"][0]["measure"],
        Value::from("Total Revenue")
    );
    assert_eq!(
        dry_json["changes"][0]["after"]["queryState"]["Values"]["projections"][0]["field"]["Measure"]
            ["Property"],
        Value::from("Total Revenue")
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after dry-run"),
        source_before
    );

    let bound_project = temp.path().join("sales_project_bound");
    let bound_arg = bound_project.to_str().expect("bound project path");
    let mutation = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--bindings-json",
        &bindings_json,
        "--out-dir",
        bound_arg,
        "--json",
    ]);
    assert_eq!(mutation.code, 0, "stderr: {}", mutation.stderr);
    let mutation_json = stdout_json(&mutation);
    assert_eq!(mutation_json["ok"], Value::Bool(true));
    assert_eq!(mutation_json["mode"], Value::from("out-dir"));
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after out-dir"),
        source_before
    );

    let readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        bound_arg,
        "--handle",
        &visual_handle,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(
        readback_json["visual"]["bindings"][0]["table"],
        Value::from("FactSales")
    );
    assert_eq!(
        readback_json["visual"]["bindings"][0]["measure"],
        Value::from("Total Revenue")
    );
    assert_eq!(
        readback_json["visual"]["bindings"][0]["kind"],
        Value::from("measure")
    );

    let validate = run_powerbi(&["validate", "--strict", bound_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    let validate_json = stdout_json(&validate);
    assert_eq!(validate_json["counts"]["boundVisuals"], Value::from(3));

    let cleared_project = temp.path().join("sales_project_cleared");
    let cleared_arg = cleared_project.to_str().expect("cleared project path");
    let clear = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        bound_arg,
        "--handle",
        &visual_handle,
        "--clear-bindings",
        "--out-dir",
        cleared_arg,
        "--json",
    ]);
    assert_eq!(clear.code, 0, "stderr: {}", clear.stderr);
    let clear_json = stdout_json(&clear);
    assert_eq!(clear_json["action"], Value::from("clear-bindings"));
    assert!(clear_json["changes"][0]["after"].is_null());

    let cleared_readback = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        cleared_arg,
        "--handle",
        &visual_handle,
        "--json",
    ]);
    assert_eq!(
        cleared_readback.code, 0,
        "stderr: {}",
        cleared_readback.stderr
    );
    let cleared_json = stdout_json(&cleared_readback);
    assert_eq!(
        cleared_json["visual"]["bindings"]
            .as_array()
            .expect("bindings")
            .len(),
        0
    );
}

#[test]
fn report_visual_set_bindings_rejects_bad_specs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let card_handle = visuals_json["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == "card")
        .and_then(|visual| visual["handle"].as_str())
        .expect("card visual handle")
        .to_string();

    let bad_shape = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--binding",
        "role=Values,table=FactSales,column=Revenue,measure=Total Revenue",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(bad_shape.code, 2);
    assert!(
        stderr_json(&bad_shape)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("either column or measure")
    );

    let unknown_measure = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--binding",
        "role=Values,table=FactSales,measure=Missing Measure",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unknown_measure.code, 10);
    assert!(
        stderr_json(&unknown_measure)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("measure not found")
    );

    let bad_cardinality = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &card_handle,
        "--binding",
        "role=Values,table=FactSales,measure=Total Revenue",
        "--binding",
        "role=Values,table=FactSales,measure=Total Units",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(bad_cardinality.code, 2);
    assert!(
        stderr_json(&bad_cardinality)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("single-value visuals accept exactly one Values binding")
    );
}

#[test]
fn report_visual_set_bindings_preserves_between_slicer_type_safety() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_catalog_proof(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = "page:ReportSectionLineControl";

    let add = run_powerbi(&[
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        page,
        "--visual-type",
        "slicer",
        "--mode",
        "between",
        "--name",
        "BetweenRebindProof",
        "--title",
        "Year range",
        "--binding",
        "role=Values,table=CatalogFacts,column=Year",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let handle = stdout_json(&add)["target"]["handle"]
        .as_str()
        .expect("Between slicer handle")
        .to_string();

    let text_binding = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--binding",
        "role=Values,table=CatalogFacts,column=Category",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(text_binding.code, 2);
    assert_eq!(
        stderr_json(&text_binding)["error"]["code"],
        "unsupported_feature"
    );
    assert!(
        stderr_json(&text_binding)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires a numeric or date column")
    );

    let numeric_binding = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &handle,
        "--binding",
        "role=Values,table=CatalogFacts,column=Amount",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        numeric_binding.code, 0,
        "stderr: {}",
        numeric_binding.stderr
    );
}

#[test]
fn report_visual_delete_round_trips_through_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_count = visuals_json["counts"]["visuals"]
        .as_u64()
        .expect("visual count");
    let visual = &visuals_json["visuals"][0];
    let visual_handle = visual["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let page_handle = visual["page"]["handle"]
        .as_str()
        .expect("page handle")
        .to_string();
    let visual_path = PathBuf::from(visual["path"].as_str().expect("visual path"));
    let source_before = fs::read_to_string(&visual_path).expect("source visual before");

    let dry_run = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.visuals.deleteMutation.v1")
    );
    assert_eq!(dry_json["action"], Value::from("delete"));
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        dry_json["target"]["handle"],
        Value::from(visual_handle.clone())
    );
    assert!(dry_json["deletePlan"]["after"].is_null());
    assert!(
        dry_json["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("report visuals list")
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after dry-run"),
        source_before
    );

    let deleted_project = temp.path().join("sales_project_deleted_visual");
    let deleted_arg = deleted_project.to_str().expect("deleted project path");
    let delete = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--out-dir",
        deleted_arg,
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    let delete_json = stdout_json(&delete);
    assert_eq!(delete_json["ok"], Value::Bool(true));
    assert_eq!(delete_json["mode"], Value::from("out-dir"));
    assert_eq!(
        delete_json["validation"]["counts"]["visuals"],
        Value::from(visual_count - 1)
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("source visual after out-dir"),
        source_before
    );
    let deleted_path = PathBuf::from(
        delete_json["changes"][0]["path"]
            .as_str()
            .expect("deleted path"),
    );
    assert!(!deleted_path.exists(), "deleted visual file still exists");
    assert!(
        !deleted_path
            .parent()
            .expect("deleted visual parent")
            .exists(),
        "deleted visual directory still exists"
    );

    let deleted_visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        deleted_arg,
        "--page",
        &page_handle,
        "--json",
    ]);
    assert_eq!(
        deleted_visuals.code, 0,
        "stderr: {}",
        deleted_visuals.stderr
    );
    let deleted_visuals_json = stdout_json(&deleted_visuals);
    assert_eq!(
        deleted_visuals_json["counts"]["visuals"],
        Value::from(visual_count - 1)
    );
    assert!(
        !deleted_visuals_json["visuals"]
            .as_array()
            .expect("visuals")
            .iter()
            .any(|item| item["handle"] == visual_handle)
    );

    let show_deleted = run_powerbi(&[
        "report",
        "visuals",
        "show",
        "--project",
        deleted_arg,
        "--handle",
        &visual_handle,
        "--json",
    ]);
    assert_eq!(show_deleted.code, 2);
    assert!(
        stderr_json(&show_deleted)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("visual not found")
    );

    let validate = run_powerbi(&["validate", "--strict", deleted_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(
        stdout_json(&validate)["counts"]["visuals"],
        Value::from(visual_count - 1)
    );
}

#[cfg(windows)]
#[test]
fn report_visual_delete_handles_read_only_visual_directories_on_windows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let visual_path = PathBuf::from(
        visuals_json["visuals"][0]["path"]
            .as_str()
            .expect("visual path"),
    );
    let visual_dir = visual_path.parent().expect("visual directory");
    let mut permissions = fs::metadata(visual_dir)
        .expect("visual directory metadata")
        .permissions();
    permissions.set_readonly(true);
    fs::set_permissions(visual_dir, permissions).expect("mark visual directory read-only");

    let delete = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--in-place",
        "--confirm",
        &visual_handle,
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    assert!(!visual_path.exists(), "deleted visual file still exists");
    assert!(
        !visual_dir.exists(),
        "deleted visual directory still exists"
    );
}

#[test]
fn report_visual_delete_rejects_unsafe_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let visuals_json = stdout_json(&visuals);
    let visual_handle = visuals_json["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let visual_path = PathBuf::from(
        visuals_json["visuals"][0]["path"]
            .as_str()
            .expect("visual path"),
    );

    let missing_mode = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --dry-run")
    );

    let missing_selector = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_selector.code, 2);
    assert!(
        stderr_json(&missing_selector)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --handle")
    );

    let unknown = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        "visual:Missing:Nope",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    assert!(
        stderr_json(&unknown)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("visual not found")
    );

    let multiple_modes_project = temp.path().join("multiple_modes");
    let multiple_modes_arg = multiple_modes_project
        .to_str()
        .expect("multiple modes path");
    let multiple_modes = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--dry-run",
        "--out-dir",
        multiple_modes_arg,
        "--json",
    ]);
    assert_eq!(multiple_modes.code, 2);
    assert!(
        stderr_json(&multiple_modes)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("choose exactly one output mode")
    );

    let in_place_without_confirm = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--in-place",
        "--json",
    ]);
    assert_eq!(in_place_without_confirm.code, 2);
    assert!(
        stderr_json(&in_place_without_confirm)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --confirm")
    );

    let in_place_wrong_confirm = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--in-place",
        "--confirm",
        "visual:Wrong:Handle",
        "--json",
    ]);
    assert_eq!(in_place_wrong_confirm.code, 2);
    assert!(
        stderr_json(&in_place_wrong_confirm)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --confirm")
    );

    let visual_dir = visual_path.parent().expect("visual dir");
    fs::write(visual_dir.join("metadata.json"), "{}").expect("write extra visual file");
    let extra_file = run_powerbi(&[
        "report",
        "visuals",
        "delete",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(extra_file.code, 2);
    assert!(
        stderr_json(&extra_file)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown files")
    );
}

#[test]
fn report_themes_extract_and_apply_raw_bundle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(&temp.path().join("source"));
    let target = scaffold_sales(&temp.path().join("target"));
    let source_theme = install_registered_theme(
        &source,
        "Corporate Safety",
        &["#004B87", "#E87722", "#4A7729"],
    );
    let source_arg = source.to_str().expect("source path");
    let target_arg = target.to_str().expect("target path");

    let show_empty = run_powerbi(&[
        "report",
        "themes",
        "show",
        "--project",
        target_arg,
        "--json",
    ]);
    assert_eq!(show_empty.code, 0, "stderr: {}", show_empty.stderr);
    let show_empty_json = stdout_json(&show_empty);
    assert_eq!(
        show_empty_json["schema"],
        Value::from("powerbi-cli.report.themes.show.v1")
    );
    assert_eq!(show_empty_json["theme"]["state"], Value::from("none"));

    let bundle_path = temp.path().join("theme-bundle.json");
    let bundle_arg = bundle_path.to_str().expect("bundle path");
    let extract = run_powerbi(&[
        "report",
        "themes",
        "extract",
        "--project",
        source_arg,
        "--out",
        bundle_arg,
        "--json",
    ]);
    assert_eq!(extract.code, 0, "stderr: {}", extract.stderr);
    let extract_json = stdout_json(&extract);
    assert_eq!(
        extract_json["bundle"]["schema"],
        Value::from("powerbi-cli.report.theme-bundle.v1")
    );
    assert!(bundle_path.is_file());
    assert!(
        extract_json["bundle"]["sourceFingerprint"]
            .as_str()
            .unwrap_or_default()
            .starts_with("fnv64:")
    );
    assert!(
        extract_json["bundle"]["registeredThemes"][0]["relativePath"]
            .as_str()
            .unwrap()
            .contains("StaticResources/RegisteredResources/CorpTheme.json")
    );

    let target_report_json = target
        .join("SalesOperations.Report")
        .join("definition")
        .join("report.json");
    let target_before = fs::read_to_string(&target_report_json).expect("target before");
    let dry_run = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        bundle_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.themes.mutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        fs::read_to_string(&target_report_json).expect("target after dry-run"),
        target_before
    );

    let themed = temp.path().join("target-themed");
    let themed_arg = themed.to_str().expect("themed path");
    let apply = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        bundle_arg,
        "--out-dir",
        themed_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["ok"], Value::Bool(true));
    assert_eq!(apply_json["mode"], Value::from("out-dir"));
    assert_eq!(
        fs::read_to_string(&target_report_json).expect("target after out-dir"),
        target_before
    );

    let readback = run_powerbi(&[
        "report",
        "themes",
        "show",
        "--project",
        themed_arg,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(readback_json["theme"]["state"], Value::from("referenced"));
    assert_eq!(
        readback_json["theme"]["registeredThemes"][0]["name"],
        Value::from("CorpTheme.json")
    );
    let copied_theme = themed
        .join("SalesOperations.Report")
        .join("StaticResources")
        .join("RegisteredResources")
        .join("CorpTheme.json");
    let copied_theme_json: Value =
        serde_json::from_str(&fs::read_to_string(&copied_theme).expect("copied theme"))
            .expect("copied theme json");
    let source_theme_json: Value =
        serde_json::from_str(&fs::read_to_string(&source_theme).expect("source theme"))
            .expect("source theme json");
    assert_eq!(copied_theme_json["name"], Value::from("CorpTheme.json"));
    assert_eq!(
        copied_theme_json["dataColors"], source_theme_json["dataColors"],
        "theme application should normalize only the host-managed name"
    );
}

#[test]
fn report_theme_preset_uses_schema_three_version_object() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let source_arg = source.to_str().expect("source path");
    let source_report = report_json(&source);
    let mut report: Value =
        serde_json::from_str(&fs::read_to_string(&source_report).expect("source report JSON"))
            .expect("parse source report JSON");
    report["$schema"] = Value::from(
        "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.3.0/schema.json",
    );
    fs::write(
        &source_report,
        serde_json::to_string_pretty(&report).expect("schema-three report JSON"),
    )
    .expect("write schema-three report JSON");

    let themed = temp.path().join("schema-three-themed");
    let themed_arg = themed.to_str().expect("themed path");
    let apply = run_powerbi(&[
        "report",
        "themes",
        "apply-preset",
        "--project",
        source_arg,
        "--preset",
        "risk-dashboard",
        "--out-dir",
        themed_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);

    let themed_report: Value = serde_json::from_str(
        &fs::read_to_string(report_json(&themed)).expect("themed report JSON"),
    )
    .expect("parse themed report JSON");
    assert_eq!(
        themed_report["themeCollection"]["customTheme"]["reportVersionAtImport"],
        json!({
            "visual": "2.10.0",
            "page": "2.3.1",
            "report": "3.4.0"
        })
    );

    let validation = run_powerbi(&["validate", themed_arg, "--strict", "--json"]);
    assert_eq!(validation.code, 0, "stderr: {}", validation.stderr);

    let mut malformed = themed_report;
    malformed["themeCollection"]["customTheme"]["reportVersionAtImport"] = json!({
        "visual": "2.10.0",
        "report": "not-a-version"
    });
    fs::write(
        report_json(&themed),
        serde_json::to_string_pretty(&malformed).expect("malformed report JSON"),
    )
    .expect("write malformed report JSON");
    let rejected = run_powerbi(&["validate", themed_arg, "--json"]);
    assert_eq!(rejected.code, 10);
    assert!(
        stdout_json(&rejected)["errors"]
            .as_array()
            .expect("validation errors")
            .iter()
            .any(|error| error
                .as_str()
                .unwrap_or_default()
                .contains("reportVersionAtImport must match"))
    );
}

#[test]
fn report_themes_apply_rejects_unsafe_or_wrong_bundle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = scaffold_sales(temp.path());
    let target_arg = target.to_str().expect("target path");
    let unsafe_bundle = temp.path().join("unsafe-theme-bundle.json");
    fs::write(
        &unsafe_bundle,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.report.theme-bundle.v1",
            "themeCollection": {
                "customTheme": {
                    "name": "Unsafe",
                    "note": "https://example.invalid/theme.json"
                }
            },
            "registeredThemes": []
        }))
        .expect("unsafe bundle json"),
    )
    .expect("write unsafe bundle");
    let unsafe_arg = unsafe_bundle.to_str().expect("unsafe bundle path");
    let rejected = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        unsafe_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(rejected.code, 10);
    assert!(
        stderr_json(&rejected)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("external URI")
    );

    let safe_bundle = temp.path().join("safe-theme-bundle.json");
    fs::write(
        &safe_bundle,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.report.theme-bundle.v1",
            "themeCollection": {},
            "registeredThemes": []
        }))
        .expect("safe bundle json"),
    )
    .expect("write safe bundle");
    let safe_arg = safe_bundle.to_str().expect("safe bundle path");
    let missing_mode = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        safe_arg,
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --dry-run")
    );
}

#[test]
fn capabilities_advertise_report_layout_commands() {
    let full_contract = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(full_contract.code, 0, "stderr: {}", full_contract.stderr);
    let full_contract_value = stdout_json(&full_contract);

    let output = run_powerbi(&["capabilities", "--json", "--for", "report"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let paths = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(paths.contains(&"report pages list"));
    assert!(paths.contains(&"report pages show"));
    assert!(paths.contains(&"report pages add"));
    assert!(paths.contains(&"report pages update"));
    assert!(paths.contains(&"report pages reorder"));
    assert!(paths.contains(&"report pages set-active"));
    assert!(paths.contains(&"report pages delete-empty"));
    assert!(paths.contains(&"report bookmarks list"));
    assert!(paths.contains(&"report bookmarks show"));
    assert!(paths.contains(&"report filters list"));
    assert!(paths.contains(&"report filters show"));
    assert!(paths.contains(&"report filters add"));
    assert!(paths.contains(&"report filters delete"));
    assert!(paths.contains(&"report filters clear"));
    assert!(paths.contains(&"report slicers list"));
    assert!(paths.contains(&"report slicers show"));
    assert!(paths.contains(&"report slicers clear"));
    assert!(paths.contains(&"report interactions list"));
    assert!(paths.contains(&"report interactions show"));
    assert!(paths.contains(&"report interactions set"));
    assert!(paths.contains(&"report interactions disable"));
    assert!(paths.contains(&"report themes show"));
    assert!(paths.contains(&"report themes extract"));
    assert!(paths.contains(&"report themes apply"));
    assert!(paths.contains(&"report visuals list"));
    assert!(paths.contains(&"report visuals show"));
    assert!(paths.contains(&"report visuals formatting list"));
    assert!(paths.contains(&"report visuals formatting show"));
    assert!(paths.contains(&"report visuals formatting extract"));
    assert!(paths.contains(&"report visuals formatting apply"));
    assert!(paths.contains(&"report visuals formatting set-text"));
    assert!(paths.contains(&"report visuals formatting set-color"));
    assert!(paths.contains(&"report visuals add"));
    assert!(paths.contains(&"report visuals clone"));
    assert!(paths.contains(&"report visuals delete"));
    assert!(paths.contains(&"report visuals set-position"));
    assert!(paths.contains(&"report visuals set-bindings"));
    let set_position = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals set-position")
        .expect("set-position command");
    assert_eq!(set_position["mutates"], Value::Bool(true));
    assert_eq!(
        set_position["outputSchema"],
        Value::from("powerbi-cli.report.visuals.positionMutation.v1")
    );
    assert!(
        set_position["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--out-dir <dir>")
    );
    let visual_formatting = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals formatting show")
        .expect("visual formatting show command");
    assert_eq!(visual_formatting["readOnly"], Value::Bool(true));
    assert_eq!(visual_formatting["mutates"], Value::Bool(false));
    assert_eq!(
        visual_formatting["outputSchema"],
        Value::from("powerbi-cli.report.visuals.formatting.show.v1")
    );
    assert!(
        visual_formatting["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--include-raw")
    );
    let visual_formatting_extract = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals formatting extract")
        .expect("visual formatting extract command");
    assert_eq!(visual_formatting_extract["readOnly"], Value::Bool(false));
    assert_eq!(visual_formatting_extract["mutates"], Value::Bool(true));
    assert_eq!(
        visual_formatting_extract["mutatesProject"],
        Value::Bool(false)
    );
    assert_eq!(
        visual_formatting_extract["outputSchema"],
        Value::from("powerbi-cli.report.visuals.formatting.extract.v1")
    );
    assert!(
        visual_formatting_extract["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--out <formatting-bundle.json>")
    );
    let visual_formatting_apply = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals formatting apply")
        .expect("visual formatting apply command");
    assert_eq!(visual_formatting_apply["mutates"], Value::Bool(true));
    assert_eq!(visual_formatting_apply["requiresOutput"], Value::Bool(true));
    assert_eq!(
        visual_formatting_apply["writesDataCache"],
        Value::Bool(false)
    );
    assert_eq!(
        visual_formatting_apply["outputSchema"],
        Value::from("powerbi-cli.report.visuals.formatting.mutation.v1")
    );
    for expected_flag in [
        "--bundle <formatting-bundle.json>",
        "--allow-literal-text",
        "--dry-run",
        "--in-place",
        "--out-dir <dir>",
    ] {
        assert!(
            visual_formatting_apply["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing visual formatting apply flag {expected_flag}"
        );
    }
    let visual_formatting_set_text = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals formatting set-text")
        .expect("visual formatting set-text command");
    assert_eq!(visual_formatting_set_text["mutates"], Value::Bool(true));
    assert_eq!(
        visual_formatting_set_text["requiresOutput"],
        Value::Bool(true)
    );
    assert_eq!(
        visual_formatting_set_text["writesDataCache"],
        Value::Bool(false)
    );
    assert_eq!(
        visual_formatting_set_text["outputSchema"],
        Value::from("powerbi-cli.report.visuals.formatting.textMutation.v1")
    );
    for expected_flag in [
        "--title <text>",
        "--show-title true|false",
        "--clear-alt-text",
        "--dry-run",
        "--out-dir <dir>",
    ] {
        assert!(
            visual_formatting_set_text["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing visual formatting set-text flag {expected_flag}"
        );
    }
    assert!(
        !visual_formatting_set_text["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--alt-text <text>"),
        "capabilities must not advertise validator-rejected alt-text authoring"
    );
    let visual_formatting_set_color = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals formatting set-color")
        .expect("visual formatting set-color command");
    assert_eq!(visual_formatting_set_color["mutates"], Value::Bool(true));
    assert_eq!(
        visual_formatting_set_color["requiresOutput"],
        Value::Bool(true)
    );
    assert_eq!(
        visual_formatting_set_color["writesDataCache"],
        Value::Bool(false)
    );
    assert_eq!(
        visual_formatting_set_color["outputSchema"],
        Value::from("powerbi-cli.report.visuals.formatting.colorMutation.v1")
    );
    for expected_flag in [
        "--slot title.fontColor|dataPoint.fill",
        "--color <hex>",
        "--title-font-color <hex>",
        "--data-point-fill <hex>",
        "--dry-run",
        "--out-dir <dir>",
    ] {
        assert!(
            visual_formatting_set_color["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing visual formatting set-color flag {expected_flag}"
        );
    }
    let add_visual = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals add")
        .expect("add visual command");
    assert_eq!(add_visual["mutates"], Value::Bool(true));
    assert_eq!(add_visual["requiresOutput"], Value::Bool(true));
    assert_eq!(add_visual["writesDataCache"], Value::Bool(false));
    assert_eq!(
        add_visual["outputSchema"],
        Value::from("powerbi-cli.report.visuals.mutation.v1")
    );
    for expected_flag in [
        "--page <page-name-or-handle>",
        "--title <title>",
        "--mode basic|dropdown|between",
        "--binding <key=value,...>",
        "--bindings-json <json>",
        "--dry-run",
        "--out-dir <dir>",
    ] {
        assert!(
            add_visual["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing visual add flag {expected_flag}"
        );
    }
    for expected_type in ["pieChart", "donutChart", "pivotTable", "slicer"] {
        assert!(
            add_visual["supportedVisualTypes"]
                .as_array()
                .expect("supported visual types")
                .iter()
                .any(|visual_type| visual_type == expected_type),
            "missing generated visual type {expected_type}"
        );
    }
    assert!(
        add_visual["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "visualPlan.after")
    );
    let clone_visual = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals clone")
        .expect("clone visual command");
    assert_eq!(clone_visual["mutates"], Value::Bool(true));
    assert_eq!(clone_visual["requiresOutput"], Value::Bool(true));
    assert_eq!(clone_visual["writesDataCache"], Value::Bool(false));
    assert_eq!(
        clone_visual["outputSchema"],
        Value::from("powerbi-cli.report.visuals.cloneMutation.v1")
    );
    for expected_flag in [
        "--handle <source-visual-handle>",
        "--target-page <page-name-or-handle>",
        "--title <title>",
        "--dry-run",
        "--out-dir <dir>",
    ] {
        assert!(
            clone_visual["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing visual clone flag {expected_flag}"
        );
    }
    assert!(
        clone_visual["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "clonePlan.targetPath")
    );
    let bookmark_list = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report bookmarks list")
        .expect("bookmark list command");
    assert_eq!(bookmark_list["readOnly"], Value::Bool(true));
    assert_eq!(bookmark_list["mutates"], Value::Bool(false));
    assert_eq!(
        bookmark_list["outputSchema"],
        Value::from("powerbi-cli.report.bookmarks.list.v1")
    );
    assert!(
        bookmark_list["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--include-raw")
    );
    let bookmark_show = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report bookmarks show")
        .expect("bookmark show command");
    assert_eq!(bookmark_show["readOnly"], Value::Bool(true));
    assert_eq!(
        bookmark_show["outputSchema"],
        Value::from("powerbi-cli.report.bookmarks.show.v1")
    );
    let filter_list = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report filters list")
        .expect("filter list command");
    assert_eq!(filter_list["readOnly"], Value::Bool(true));
    assert_eq!(filter_list["mutates"], Value::Bool(false));
    assert_eq!(
        filter_list["outputSchema"],
        Value::from("powerbi-cli.report.filters.list.v1")
    );
    assert!(
        filter_list["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--include-raw")
    );
    let filter_show = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report filters show")
        .expect("filter show command");
    assert_eq!(filter_show["readOnly"], Value::Bool(true));
    assert_eq!(
        filter_show["outputSchema"],
        Value::from("powerbi-cli.report.filters.show.v1")
    );
    let filter_add = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report filters add")
        .expect("filter add command");
    assert_eq!(filter_add["mutates"], Value::Bool(true));
    assert_eq!(filter_add["requiresOutput"], Value::Bool(true));
    assert_eq!(filter_add["writesDataCache"], Value::Bool(false));
    assert_eq!(
        filter_add["outputSchema"],
        Value::from("powerbi-cli.report.filters.addMutation.v1")
    );
    for expected_flag in [
        "--target <table[column]>",
        "--table <table>",
        "--column <column>",
        "--value <text>",
        "--value-json <json>",
        "--values-json <json-array>",
        "--dry-run",
        "--out-dir <dir>",
        "--include-raw",
    ] {
        assert!(
            filter_add["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing filter add flag {expected_flag}"
        );
    }
    let filter_delete = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report filters delete")
        .expect("filter delete command");
    assert_eq!(filter_delete["mutates"], Value::Bool(true));
    assert_eq!(filter_delete["requiresOutput"], Value::Bool(true));
    assert_eq!(filter_delete["writesDataCache"], Value::Bool(false));
    assert_eq!(
        filter_delete["confirmRequiredForInPlace"],
        Value::Bool(true)
    );
    assert_eq!(
        filter_delete["outputSchema"],
        Value::from("powerbi-cli.report.filters.deleteMutation.v1")
    );
    for expected_flag in [
        "--handle <filter-handle>",
        "--dry-run",
        "--in-place",
        "--confirm <filter-handle>",
        "--out-dir <dir>",
        "--include-raw",
    ] {
        assert!(
            filter_delete["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing filter delete flag {expected_flag}"
        );
    }
    let filter_clear = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report filters clear")
        .expect("filter clear command");
    assert_eq!(filter_clear["mutates"], Value::Bool(true));
    assert_eq!(filter_clear["requiresOutput"], Value::Bool(true));
    assert_eq!(filter_clear["writesDataCache"], Value::Bool(false));
    assert_eq!(filter_clear["confirmRequiredForInPlace"], Value::Bool(true));
    assert_eq!(
        filter_clear["outputSchema"],
        Value::from("powerbi-cli.report.filters.clearMutation.v1")
    );
    for expected_flag in [
        "--scope report|page|visual",
        "--page <page-name-or-handle>",
        "--visual <visual-name-or-handle>",
        "--all",
        "--confirm <confirm-token>",
        "--out-dir <dir>",
    ] {
        assert!(
            filter_clear["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing filter clear flag {expected_flag}"
        );
    }
    let slicer_list = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report slicers list")
        .expect("slicer list command");
    assert_eq!(slicer_list["readOnly"], Value::Bool(true));
    assert_eq!(slicer_list["mutates"], Value::Bool(false));
    assert_eq!(
        slicer_list["outputSchema"],
        Value::from("powerbi-cli.report.slicers.list.v1")
    );
    assert!(
        slicer_list["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--include-raw")
    );
    let slicer_show = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report slicers show")
        .expect("slicer show command");
    assert_eq!(slicer_show["readOnly"], Value::Bool(true));
    assert_eq!(
        slicer_show["outputSchema"],
        Value::from("powerbi-cli.report.slicers.show.v1")
    );
    assert!(
        slicer_show["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--no-raw")
    );
    let slicer_clear = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report slicers clear")
        .expect("slicer clear command");
    assert_eq!(slicer_clear["readOnly"], Value::Bool(false));
    assert_eq!(slicer_clear["mutates"], Value::Bool(true));
    assert_eq!(slicer_clear["requiresOutput"], Value::Bool(true));
    assert_eq!(slicer_clear["writesDataCache"], Value::Bool(false));
    assert_eq!(slicer_clear["confirmRequiredForInPlace"], Value::Bool(true));
    assert_eq!(
        slicer_clear["outputSchema"],
        Value::from("powerbi-cli.report.slicers.clearMutation.v1")
    );
    for expected_flag in [
        "--handle <slicer-or-visual-handle>",
        "--page <page-name-or-handle>",
        "--visual <visual-name-or-handle>",
        "--confirm <confirm-token>",
        "--out-dir <dir>",
    ] {
        assert!(
            slicer_clear["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing slicer clear flag {expected_flag}"
        );
    }
    let interaction_list = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report interactions list")
        .expect("interaction list command");
    assert_eq!(interaction_list["readOnly"], Value::Bool(true));
    assert_eq!(interaction_list["mutates"], Value::Bool(false));
    assert_eq!(
        interaction_list["outputSchema"],
        Value::from("powerbi-cli.report.interactions.list.v1")
    );
    assert!(
        interaction_list["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--type Default|DataFilter|HighlightFilter|NoFilter")
    );
    let interaction_show = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report interactions show")
        .expect("interaction show command");
    assert_eq!(interaction_show["readOnly"], Value::Bool(true));
    assert_eq!(
        interaction_show["outputSchema"],
        Value::from("powerbi-cli.report.interactions.show.v1")
    );
    assert!(
        interaction_show["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "interaction.semantics")
    );
    let interaction_set = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report interactions set")
        .expect("interaction set command");
    assert_eq!(interaction_set["mutates"], Value::Bool(true));
    assert_eq!(interaction_set["requiresOutput"], Value::Bool(true));
    assert_eq!(interaction_set["writesDataCache"], Value::Bool(false));
    assert_eq!(
        interaction_set["outputSchema"],
        Value::from("powerbi-cli.report.interactions.mutation.v1")
    );
    assert!(
        interaction_set["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--type DataFilter|HighlightFilter|NoFilter")
    );
    assert!(
        interaction_set["summary"]
            .as_str()
            .expect("summary")
            .contains("Default authoring")
    );
    let interaction_disable = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report interactions disable")
        .expect("interaction disable command");
    assert_eq!(interaction_disable["mutates"], Value::Bool(true));
    assert_eq!(interaction_disable["requiresOutput"], Value::Bool(true));
    assert_eq!(
        interaction_disable["outputSchema"],
        Value::from("powerbi-cli.report.interactions.mutation.v1")
    );
    assert!(
        interaction_disable["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "interactionPlan.after.type")
    );
    let delete_visual = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals delete")
        .expect("delete visual command");
    assert_eq!(delete_visual["mutates"], Value::Bool(true));
    assert_eq!(delete_visual["requiresOutput"], Value::Bool(true));
    assert_eq!(delete_visual["writesDataCache"], Value::Bool(false));
    assert_eq!(
        delete_visual["outputSchema"],
        Value::from("powerbi-cli.report.visuals.deleteMutation.v1")
    );
    assert_eq!(
        delete_visual["confirmRequiredForInPlace"],
        Value::Bool(true)
    );
    for expected_flag in [
        "--handle <visual-handle>",
        "--page <page-name-or-handle>",
        "--visual <visual-name-or-handle>",
        "--dry-run",
        "--in-place",
        "--confirm <visual-handle>",
        "--out-dir <dir>",
    ] {
        assert!(
            delete_visual["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == expected_flag),
            "missing visual delete flag {expected_flag}"
        );
    }
    assert!(
        delete_visual["followUpFields"]
            .as_array()
            .expect("followUpFields")
            .iter()
            .any(|field| field == "deletePlan.after")
    );
    let set_bindings = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals set-bindings")
        .expect("set-bindings command");
    assert_eq!(set_bindings["mutates"], Value::Bool(true));
    assert_eq!(set_bindings["requiresOutput"], Value::Bool(true));
    assert!(
        set_bindings["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--bindings-json <json>")
    );
    let apply_theme = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report themes apply")
        .expect("apply theme command");
    assert_eq!(apply_theme["mutates"], Value::Bool(true));
    assert_eq!(apply_theme["requiresOutput"], Value::Bool(true));
    assert_eq!(apply_theme["writesDataCache"], Value::Bool(false));
    assert!(
        full_contract_value["schemaManifest"]["visualMutationFields"]
            .as_array()
            .expect("visual mutation fields")
            .iter()
            .any(|field| field == "visualPlan.after")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualDeleteMutationFields"]
            .as_array()
            .expect("visual delete mutation fields")
            .iter()
            .any(|field| field == "deletePlan.after")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualCloneMutationFields"]
            .as_array()
            .expect("visual clone mutation fields")
            .iter()
            .any(|field| field == "clonePlan.targetPath")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualFormattingFields"]
            .as_array()
            .expect("visual formatting fields")
            .iter()
            .any(|field| field == "containers")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualFormattingContainerFields"]
            .as_array()
            .expect("visual formatting container fields")
            .iter()
            .any(|field| field == "propertyNames")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualFormattingBundleFields"]
            .as_array()
            .expect("visual formatting bundle fields")
            .iter()
            .any(|field| field == "formatting.visualObjects")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualFormattingMutationFields"]
            .as_array()
            .expect("visual formatting mutation fields")
            .iter()
            .any(|field| field == "formattingPlan.after")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualFormattingTextMutationFields"]
            .as_array()
            .expect("visual formatting text mutation fields")
            .iter()
            .any(|field| field == "textPlan.after")
    );
    assert!(
        full_contract_value["schemaManifest"]["visualFormattingColorMutationFields"]
            .as_array()
            .expect("visual formatting color mutation fields")
            .iter()
            .any(|field| field == "colorPlan.after")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportBookmarkFields"]
            .as_array()
            .expect("report bookmark fields")
            .iter()
            .any(|field| field == "safety")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportBookmarkSafetyFields"]
            .as_array()
            .expect("report bookmark safety fields")
            .iter()
            .any(|field| field == "literalCountInBookmarkState")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportFilterFields"]
            .as_array()
            .expect("report filter fields")
            .iter()
            .any(|field| field == "safety")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportFilterMutationFields"]
            .as_array()
            .expect("report filter mutation fields")
            .iter()
            .any(|field| field == "filterPlan.after")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportFilterAddMutationFields"]
            .as_array()
            .expect("report filter add mutation fields")
            .iter()
            .any(|field| field == "filterPlan.afterCount")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportFilterClearMutationFields"]
            .as_array()
            .expect("report filter clear mutation fields")
            .iter()
            .any(|field| field == "confirmToken")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportFilterClearMutationFields"]
            .as_array()
            .expect("report filter clear mutation fields")
            .iter()
            .any(|field| field == "filterPlan.arrayEdits")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportSlicerFields"]
            .as_array()
            .expect("report slicer fields")
            .iter()
            .any(|field| field == "target")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportSlicerSafetyFields"]
            .as_array()
            .expect("report slicer safety fields")
            .iter()
            .any(|field| field == "literalCountInSlicerState")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportSlicerClearMutationFields"]
            .as_array()
            .expect("report slicer clear mutation fields")
            .iter()
            .any(|field| field == "confirmToken")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportSlicerClearMutationFields"]
            .as_array()
            .expect("report slicer clear mutation fields")
            .iter()
            .any(|field| field == "slicerPlan.arrayEdits")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportInteractionFields"]
            .as_array()
            .expect("report interaction fields")
            .iter()
            .any(|field| field == "semantics")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportInteractionSemanticsFields"]
            .as_array()
            .expect("report interaction semantics fields")
            .iter()
            .any(|field| field == "missingRowsMean")
    );
    assert!(
        full_contract_value["schemaManifest"]["reportInteractionMutationFields"]
            .as_array()
            .expect("report interaction mutation fields")
            .iter()
            .any(|field| field == "interactionPlan.after")
    );
    let filter_capabilities = run_powerbi(&["capabilities", "--json", "--for", "filter"]);
    assert_eq!(
        filter_capabilities.code, 0,
        "stderr: {}",
        filter_capabilities.stderr
    );
    let filter_value = stdout_json(&filter_capabilities);
    let filter_paths = filter_value["commands"]
        .as_array()
        .expect("filter commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(filter_paths.contains(&"report filters list"));
    assert!(filter_paths.contains(&"report filters show"));
    assert!(filter_paths.contains(&"report filters add"));
    assert!(filter_paths.contains(&"report filters delete"));
    assert!(filter_paths.contains(&"report filters clear"));
    let bookmark_capabilities = run_powerbi(&["capabilities", "--json", "--for", "bookmark"]);
    assert_eq!(
        bookmark_capabilities.code, 0,
        "stderr: {}",
        bookmark_capabilities.stderr
    );
    let bookmark_value = stdout_json(&bookmark_capabilities);
    let bookmark_paths = bookmark_value["commands"]
        .as_array()
        .expect("bookmark commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(bookmark_paths.contains(&"report bookmarks list"));
    assert!(bookmark_paths.contains(&"report bookmarks show"));
    let slicer_capabilities = run_powerbi(&["capabilities", "--json", "--for", "slicer"]);
    assert_eq!(
        slicer_capabilities.code, 0,
        "stderr: {}",
        slicer_capabilities.stderr
    );
    let slicer_value = stdout_json(&slicer_capabilities);
    let slicer_paths = slicer_value["commands"]
        .as_array()
        .expect("slicer commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(slicer_paths.contains(&"report slicers list"));
    assert!(slicer_paths.contains(&"report slicers show"));
    assert!(slicer_paths.contains(&"report slicers clear"));
    assert!(slicer_paths.contains(&"report visuals catalog"));
    assert!(slicer_paths.contains(&"report visuals add"));
    let interaction_capabilities = run_powerbi(&["capabilities", "--json", "--for", "interaction"]);
    assert_eq!(
        interaction_capabilities.code, 0,
        "stderr: {}",
        interaction_capabilities.stderr
    );
    let interaction_value = stdout_json(&interaction_capabilities);
    let interaction_paths = interaction_value["commands"]
        .as_array()
        .expect("interaction commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(interaction_paths.contains(&"report interactions list"));
    assert!(interaction_paths.contains(&"report interactions show"));
    assert!(interaction_paths.contains(&"report interactions set"));
    assert!(interaction_paths.contains(&"report interactions disable"));
    for path in [
        "report pages add",
        "report pages update",
        "report pages reorder",
        "report pages set-active",
        "report pages delete-empty",
    ] {
        let command = value["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .find(|command| command["path"] == path)
            .expect("page mutation command");
        assert_eq!(command["mutates"], Value::Bool(true));
        assert_eq!(command["requiresOutput"], Value::Bool(true));
        assert_eq!(command["writesDataCache"], Value::Bool(false));
        assert_eq!(
            command["outputSchema"],
            Value::from("powerbi-cli.report.pages.mutation.v1")
        );
        assert!(
            command["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == "--dry-run")
        );
        assert!(
            command["flags"]
                .as_array()
                .expect("flags")
                .iter()
                .any(|flag| flag == "--out-dir <dir>")
        );
    }
}
