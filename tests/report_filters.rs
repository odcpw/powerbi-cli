//! Report filter listing, authoring, mutation, and clearing integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

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
