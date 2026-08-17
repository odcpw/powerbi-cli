//! Report page discovery, mutation, and layout-capability integration tests.

mod common;

use common::*;
use serde_json::Value;
use std::fs;

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
    assert!(paths.contains(&"report visuals set-object"));
    assert!(paths.contains(&"report visuals set-display-name"));
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
    let set_object = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals set-object")
        .expect("set-object command");
    assert_eq!(set_object["mutates"], Value::Bool(true));
    assert_eq!(
        set_object["outputSchema"],
        Value::from("powerbi-cli.report.visuals.objectMutation.v1")
    );
    assert!(
        set_object["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--object <name>")
    );
    let set_display_name = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report visuals set-display-name")
        .expect("set-display-name command");
    assert_eq!(set_display_name["mutates"], Value::Bool(true));
    assert_eq!(
        set_display_name["outputSchema"],
        Value::from("powerbi-cli.report.visuals.displayNameMutation.v1")
    );
    assert!(
        set_display_name["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--role <Values|Category|Series|X|Y|Y2|Size|Rows|Columns|Tooltips>")
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
