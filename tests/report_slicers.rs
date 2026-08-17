//! Report slicer listing, inspection, and state-clearing integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;

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
