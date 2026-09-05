//! Report drillthrough lifecycle and unsupported-command integration tests.

mod common;

use common::*;
use serde_json::Value;
use std::fs;

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
