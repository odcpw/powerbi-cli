//! Report visual formatting inspection, extraction, and mutation integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

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
