//! Report validation, object inspection, audit, and sanitization integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

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
            .any(|error| error["message"].as_str().is_some_and(|message| {
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
            .any(|error| error["message"].as_str().is_some_and(|message| {
                message.contains("visual directory is missing visual.json")
                    && message.contains("Remove the empty visual directory")
            }))
    );
}
