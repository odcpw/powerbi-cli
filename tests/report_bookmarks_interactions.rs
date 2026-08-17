//! Report bookmark and visual-interaction inspection and mutation integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

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
