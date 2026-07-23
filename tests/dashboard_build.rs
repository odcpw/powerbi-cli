use serde_json::{Value, json};
use std::fs;
use std::path::Path;
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

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

#[test]
fn generic_dashboard_build_validates_handoff_and_matches_golden() {
    let temp = tempfile::tempdir().expect("tempdir");
    let profile = temp.path().join("sales.profile.json");
    let profile_arg = path_arg(&profile);
    let project = temp.path().join("generic_sales");
    let project_arg = path_arg(&project);

    let schema = run_powerbi(&["schema", "validate", "examples/sales.schema.json", "--json"]);
    assert_eq!(schema.code, 0, "stderr: {}", schema.stderr);
    assert_eq!(
        stdout_json(&schema)["schema"],
        Value::from("powerbi-cli.schema.validate.v1")
    );

    let infer = run_powerbi(&[
        "profile",
        "infer",
        "--schema",
        "examples/sales.schema.json",
        "--out",
        &profile_arg,
        "--json",
    ]);
    assert_eq!(infer.code, 0, "stderr: {}", infer.stderr);
    let infer_json = stdout_json(&infer);
    assert_eq!(
        infer_json["profile"]["schema"],
        Value::from("powerbi-cli.dataProfile.v1")
    );
    assert!(
        infer_json["profile"]["candidates"]["factTables"]
            .as_array()
            .expect("fact tables")
            .iter()
            .any(|item| item == "FactSales")
    );

    let profile_validate = run_powerbi(&["profile", "validate", &profile_arg, "--json"]);
    assert_eq!(
        profile_validate.code, 0,
        "stderr: {}",
        profile_validate.stderr
    );
    assert_eq!(stdout_json(&profile_validate)["ok"], Value::Bool(true));

    let spec_validate = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        &profile_arg,
        "--spec",
        "examples/sales.dashboard.json",
        "--json",
    ]);
    assert_eq!(spec_validate.code, 0, "stderr: {}", spec_validate.stderr);
    let spec_json = stdout_json(&spec_validate);
    assert_eq!(
        spec_json["schema"],
        Value::from("powerbi-cli.report.spec.validate.v1")
    );
    assert_eq!(spec_json["ok"], Value::Bool(true));
    assert_eq!(spec_json["compiled"]["counts"]["visuals"], Value::from(3));
    assert_eq!(spec_json["compiled"]["counts"]["bindings"], Value::from(6));

    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        &profile_arg,
        "--spec",
        "examples/sales.dashboard.json",
        "--out-dir",
        &project_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);
    let build_json = stdout_json(&build);
    assert_eq!(
        build_json["schema"],
        Value::from("powerbi-cli.report.build.v1")
    );
    assert_eq!(build_json["changed"], Value::Bool(true));
    assert_eq!(build_json["dryRun"], Value::Bool(false));
    assert_eq!(build_json["changes"][0]["kind"], "pbip.project");
    assert_eq!(build_json["changes"][0]["action"], "create");
    assert_eq!(build_json["changes"][0]["path"], build_json["projectDir"]);
    assert_eq!(
        build_json["proof"]["claimedDesktopCompatibility"],
        Value::Bool(false)
    );
    assert!(build_json["inspectCommand"].is_string());
    assert!(build_json["validateCommand"].is_string());
    assert!(build_json["handoffCheckCommand"].is_string());
    assert!(build_json["fixtureNormalizeCommand"].is_string());
    assert!(build_json["desktopOpenCheckCommand"].is_string());

    let validate = run_powerbi(&["validate", "--strict", &project_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));

    let handoff = run_powerbi(&["handoff", "check", &project_arg, "--json"]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(
        stdout_json(&handoff)["safeForOfflineHandoff"],
        Value::Bool(true)
    );

    let verify = run_powerbi(&[
        "fixture",
        "verify",
        &project_arg,
        "--expected",
        "testdata/golden/generic-sales.summary.json",
        "--json",
    ]);
    assert_eq!(verify.code, 0, "stderr: {}", verify.stderr);
    assert_eq!(
        stdout_json(&verify)["verification"]["same"],
        Value::Bool(true)
    );
}

#[test]
fn dashboard_build_emits_declared_visual_interactions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_path = temp.path().join("interactions.dashboard.json");
    let project = temp.path().join("interaction_project");
    let project_arg = path_arg(&project);
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.json").expect("sales spec"),
    )
    .expect("parse sales spec");
    spec["pages"][0]["interactions"] = json!([{
        "source": "customer_detail",
        "target": "revenue_trend",
        "type": "DataFilter"
    }]);
    fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("spec json"),
    )
    .expect("write spec");
    let spec_arg = path_arg(&spec_path);

    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &spec_arg,
        "--out-dir",
        &project_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);

    let page_path = project
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join("ReportSectionOverview")
        .join("page.json");
    let page: Value = serde_json::from_str(&fs::read_to_string(page_path).expect("page json"))
        .expect("parse page");
    assert_eq!(
        page["visualInteractions"],
        json!([{
            "source": "VisualContainerCustomerDetail",
            "target": "VisualContainerRevenueTrend",
            "type": "DataFilter"
        }])
    );

    spec["pages"][0]["interactions"][0]["source"] = Value::from("missing_visual");
    fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("invalid spec json"),
    )
    .expect("write invalid spec");
    let invalid = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &spec_arg,
        "--json",
    ]);
    assert_eq!(invalid.code, 10);
    assert!(
        stdout_json(&invalid)["errors"][0]
            .as_str()
            .unwrap_or_default()
            .contains("source visual missing_visual does not exist")
    );
}

#[test]
fn report_build_requires_exactly_one_output_mode_and_reports_dry_run_changes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ignored_out = temp.path().join("must-not-exist");
    let ignored_out_arg = path_arg(&ignored_out);
    let conflict = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--dry-run",
        "--out-dir",
        &ignored_out_arg,
        "--json",
    ]);
    assert_eq!(conflict.code, 2);
    let conflict_json: Value =
        serde_json::from_str(conflict.stderr.trim()).expect("conflict stderr JSON");
    assert_eq!(conflict_json["error"]["code"], "invalid_args");
    assert_eq!(
        conflict_json["error"]["message"],
        "choose exactly one output mode: --dry-run or --out-dir <dir>"
    );
    assert!(!ignored_out.exists());

    let dry_run = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(dry_json["changed"], Value::Bool(false));
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(dry_json["changes"][0]["kind"], "pbip.project");
    assert_eq!(dry_json["changes"][0]["action"], "create");
    assert!(dry_json["changes"][0]["path"].is_null());
}

#[test]
fn flat_ops_archetype_build_validates_handoff_and_matches_golden() {
    assert_archetype_golden(ArchetypeCase {
        slug: "flat_ops",
        schema: "examples/archetypes/flat-ops.schema.json",
        profile: "examples/archetypes/flat-ops.profile.json",
        spec: "examples/archetypes/flat-ops.dashboard.json",
        golden: "testdata/golden/archetypes/flat-ops.summary.json",
        expected_visuals: 3,
        expected_bindings: 7,
        check_dax: false,
    });
}

#[test]
fn scatter_bubble_archetype_build_validates_handoff_and_matches_golden() {
    assert_archetype_golden(ArchetypeCase {
        slug: "scatter_bubble",
        schema: "examples/archetypes/scatter-bubble.schema.json",
        profile: "examples/archetypes/scatter-bubble.profile.json",
        spec: "examples/archetypes/scatter-bubble.dashboard.json",
        golden: "testdata/golden/archetypes/scatter-bubble.summary.json",
        expected_visuals: 2,
        expected_bindings: 10,
        check_dax: false,
    });
}

#[test]
fn catalog_proof_archetype_build_validates_handoff_and_matches_golden() {
    assert_archetype_golden(ArchetypeCase {
        slug: "catalog_proof",
        schema: "examples/archetypes/catalog-proof.schema.json",
        profile: "examples/archetypes/catalog-proof.profile.json",
        spec: "examples/archetypes/catalog-proof.dashboard.json",
        golden: "testdata/golden/archetypes/catalog-proof.summary.json",
        expected_visuals: 6,
        expected_bindings: 13,
        check_dax: false,
    });
}

#[test]
fn regional_sales_archetype_runs_post_build_chain_and_matches_golden() {
    let temp = tempfile::tempdir().expect("tempdir");
    let built = temp.path().join("regional_sales_build");
    let with_customer_drillthrough = temp.path().join("regional_sales_customer_drillthrough");
    let with_drillthrough = temp.path().join("regional_sales_drillthrough");
    let with_customer_top3 = temp.path().join("regional_sales_customer_top3");
    let project = temp.path().join("regional_sales");
    let built_arg = path_arg(&built);
    let with_customer_drillthrough_arg = path_arg(&with_customer_drillthrough);
    let with_drillthrough_arg = path_arg(&with_drillthrough);
    let with_customer_top3_arg = path_arg(&with_customer_top3);
    let project_arg = path_arg(&project);
    let schema_path = "examples/archetypes/regional-sales.schema.json";
    let profile_path = "examples/archetypes/regional-sales.profile.json";
    let spec_path = "examples/archetypes/regional-sales.dashboard.json";
    let customer_page = "page:ReportSectionCustomerDetail";
    let segment_page = "page:ReportSectionSegmentDetail";
    let customer_table = "visual:ReportSectionCustomerDetail:VisualContainerCustomerDetailTable";
    let segment_table = "visual:ReportSectionSegmentDetail:VisualContainerSegmentDetailTable";

    let schema = run_powerbi(&["schema", "validate", schema_path, "--json"]);
    assert_eq!(schema.code, 0, "stderr: {}", schema.stderr);
    assert_eq!(stdout_json(&schema)["ok"], Value::Bool(true));

    let profile = run_powerbi(&["profile", "validate", profile_path, "--json"]);
    assert_eq!(profile.code, 0, "stderr: {}", profile.stderr);
    assert_eq!(stdout_json(&profile)["ok"], Value::Bool(true));

    let spec = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        schema_path,
        "--profile",
        profile_path,
        "--spec",
        spec_path,
        "--json",
    ]);
    assert_eq!(spec.code, 0, "stderr: {}", spec.stderr);
    let spec_json = stdout_json(&spec);
    assert_eq!(spec_json["ok"], Value::Bool(true));
    assert_eq!(spec_json["compiled"]["counts"]["visuals"], 9);
    assert_eq!(spec_json["compiled"]["counts"]["bindings"], 12);

    // The raw dashboard spec proves slicers span more than one page (multi-page slicers),
    // and that a non-ASCII column feeds one of them directly from the fixture on disk.
    let dashboard: Value = serde_json::from_str(
        &fs::read_to_string(spec_path).expect("read regional-sales dashboard"),
    )
    .expect("parse regional-sales dashboard");
    let pages = dashboard["pages"].as_array().expect("dashboard pages");
    let pages_with_slicers = pages
        .iter()
        .filter(|page| {
            page["visuals"]
                .as_array()
                .expect("page visuals")
                .iter()
                .any(|visual| visual["type"] == "slicer")
        })
        .count();
    assert_eq!(
        pages_with_slicers, 2,
        "expected slicers on two different pages"
    );
    let overview = pages
        .iter()
        .find(|page| page["id"] == "overview")
        .expect("overview page");
    assert!(
        overview["visuals"]
            .as_array()
            .expect("overview visuals")
            .iter()
            .any(|visual| visual["type"] == "slicer"
                && visual["bindings"][0]["field"] == "DimCustomer[Größenklasse]")
    );

    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        schema_path,
        "--profile",
        profile_path,
        "--spec",
        spec_path,
        "--out-dir",
        &built_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);
    assert_eq!(stdout_json(&build)["ok"], Value::Bool(true));

    // Non-ASCII measure and column names round-trip byte-for-byte into generated PBIR.
    let overview_card = fs::read_to_string(
        built
            .join("RegionalSales.Report")
            .join("definition")
            .join("pages")
            .join("ReportSectionOverview")
            .join("visuals")
            .join("VisualContainerUmsatzUebersichtCard")
            .join("visual.json"),
    )
    .expect("read Umsatz Übersicht card visual.json");
    assert!(overview_card.contains("\"Umsatz Übersicht\""));
    let overview_slicer = fs::read_to_string(
        built
            .join("RegionalSales.Report")
            .join("definition")
            .join("pages")
            .join("ReportSectionOverview")
            .join("visuals")
            .join("VisualContainerGroessenklasseSlicer")
            .join("visual.json"),
    )
    .expect("read Größenklasse slicer visual.json");
    assert!(overview_slicer.contains("\"Größenklasse\""));

    let drillthrough_dry = run_powerbi(&[
        "report",
        "drillthrough",
        "set",
        "--project",
        &built_arg,
        "--page",
        customer_page,
        "--target",
        "DimCustomer[Customer]",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        drillthrough_dry.code, 0,
        "stderr: {}",
        drillthrough_dry.stderr
    );
    let drillthrough_dry_json = stdout_json(&drillthrough_dry);
    assert_eq!(drillthrough_dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        drillthrough_dry_json["drillthroughPlan"]["after"]["enabled"],
        Value::Bool(true)
    );

    let drillthrough_write = run_powerbi(&[
        "report",
        "drillthrough",
        "set",
        "--project",
        &built_arg,
        "--page",
        customer_page,
        "--target",
        "DimCustomer[Customer]",
        "--out-dir",
        &with_customer_drillthrough_arg,
        "--json",
    ]);
    assert_eq!(
        drillthrough_write.code, 0,
        "stderr: {}",
        drillthrough_write.stderr
    );
    let drillthrough_write_json = stdout_json(&drillthrough_write);
    assert_eq!(drillthrough_write_json["mode"], "out-dir");
    assert_eq!(
        drillthrough_write_json["validation"]["ok"],
        Value::Bool(true)
    );

    let segment_drillthrough_dry = run_powerbi(&[
        "report",
        "drillthrough",
        "set",
        "--project",
        &with_customer_drillthrough_arg,
        "--page",
        segment_page,
        "--target",
        "DimCustomer[Segment]",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        segment_drillthrough_dry.code, 0,
        "stderr: {}",
        segment_drillthrough_dry.stderr
    );
    let segment_drillthrough_dry_json = stdout_json(&segment_drillthrough_dry);
    assert_eq!(segment_drillthrough_dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        segment_drillthrough_dry_json["drillthroughPlan"]["after"]["enabled"],
        Value::Bool(true)
    );

    let segment_drillthrough_write = run_powerbi(&[
        "report",
        "drillthrough",
        "set",
        "--project",
        &with_customer_drillthrough_arg,
        "--page",
        segment_page,
        "--target",
        "DimCustomer[Segment]",
        "--out-dir",
        &with_drillthrough_arg,
        "--json",
    ]);
    assert_eq!(
        segment_drillthrough_write.code, 0,
        "stderr: {}",
        segment_drillthrough_write.stderr
    );
    assert_eq!(
        stdout_json(&segment_drillthrough_write)["validation"]["ok"],
        Value::Bool(true)
    );

    let top3_dry = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        &with_drillthrough_arg,
        "--scope",
        "visual",
        "--visual",
        customer_table,
        "--target",
        "DimCustomer[Customer]",
        "--top",
        "3",
        "--by",
        "FactSales[Total Revenue]",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(top3_dry.code, 0, "stderr: {}", top3_dry.stderr);
    let top3_dry_json = stdout_json(&top3_dry);
    assert_eq!(top3_dry_json["dryRun"], Value::Bool(true));
    assert_eq!(top3_dry_json["changes"][0]["after"]["filterType"], "TopN");

    let top3_write = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        &with_drillthrough_arg,
        "--scope",
        "visual",
        "--visual",
        customer_table,
        "--target",
        "DimCustomer[Customer]",
        "--top",
        "3",
        "--by",
        "FactSales[Total Revenue]",
        "--out-dir",
        &with_customer_top3_arg,
        "--json",
    ]);
    assert_eq!(top3_write.code, 0, "stderr: {}", top3_write.stderr);
    assert_eq!(
        stdout_json(&top3_write)["validation"]["ok"],
        Value::Bool(true)
    );

    let segment_top2_dry = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        &with_customer_top3_arg,
        "--scope",
        "visual",
        "--visual",
        segment_table,
        "--target",
        "DimCustomer[Segment]",
        "--top",
        "2",
        "--by",
        "FactSales[Total Revenue]",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        segment_top2_dry.code, 0,
        "stderr: {}",
        segment_top2_dry.stderr
    );
    let segment_top2_dry_json = stdout_json(&segment_top2_dry);
    assert_eq!(segment_top2_dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        segment_top2_dry_json["changes"][0]["after"]["filterType"],
        "TopN"
    );

    let segment_top2_write = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        &with_customer_top3_arg,
        "--scope",
        "visual",
        "--visual",
        segment_table,
        "--target",
        "DimCustomer[Segment]",
        "--top",
        "2",
        "--by",
        "FactSales[Total Revenue]",
        "--out-dir",
        &project_arg,
        "--json",
    ]);
    assert_eq!(
        segment_top2_write.code, 0,
        "stderr: {}",
        segment_top2_write.stderr
    );
    assert_eq!(
        stdout_json(&segment_top2_write)["validation"]["ok"],
        Value::Bool(true)
    );

    let drillthrough_show = run_powerbi(&[
        "report",
        "drillthrough",
        "show",
        "--project",
        &project_arg,
        "--page",
        customer_page,
        "--json",
    ]);
    assert_eq!(
        drillthrough_show.code, 0,
        "stderr: {}",
        drillthrough_show.stderr
    );
    let drillthrough_json = stdout_json(&drillthrough_show);
    assert_eq!(drillthrough_json["drillthrough"]["enabled"], true);
    assert_eq!(
        drillthrough_json["drillthrough"]["binding"]["parameters"][0]["target"]["column"],
        "Customer"
    );
    let bound_filter =
        &drillthrough_json["drillthrough"]["binding"]["parameters"][0]["boundFilter"];
    assert_eq!(
        drillthrough_json["drillthrough"]["filters"][0]["name"],
        bound_filter.clone()
    );
    assert_eq!(
        drillthrough_json["drillthrough"]["filters"][0]["hasPersistedFilterDefinition"],
        false
    );

    let segment_drillthrough_show = run_powerbi(&[
        "report",
        "drillthrough",
        "show",
        "--project",
        &project_arg,
        "--page",
        segment_page,
        "--json",
    ]);
    assert_eq!(
        segment_drillthrough_show.code, 0,
        "stderr: {}",
        segment_drillthrough_show.stderr
    );
    let segment_drillthrough_json = stdout_json(&segment_drillthrough_show);
    assert_eq!(segment_drillthrough_json["drillthrough"]["enabled"], true);
    assert_eq!(
        segment_drillthrough_json["drillthrough"]["binding"]["parameters"][0]["target"]["column"],
        "Segment"
    );
    assert_eq!(
        segment_drillthrough_json["drillthrough"]["filters"][0]["hasPersistedFilterDefinition"],
        false
    );

    let filters = run_powerbi(&[
        "report",
        "filters",
        "list",
        "--project",
        &project_arg,
        "--json",
    ]);
    assert_eq!(filters.code, 0, "stderr: {}", filters.stderr);
    let filters_json = stdout_json(&filters);
    assert_eq!(filters_json["counts"]["filters"], 4);
    assert_eq!(filters_json["counts"]["pageFilters"], 2);
    assert_eq!(filters_json["counts"]["visualFilters"], 2);

    let dependencies = run_powerbi(&[
        "model",
        "dax",
        "dependencies",
        "--project",
        &project_arg,
        "--json",
    ]);
    assert_eq!(dependencies.code, 0, "stderr: {}", dependencies.stderr);
    assert!(
        stdout_json(&dependencies)["findings"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    let lint = run_powerbi(&["model", "dax", "lint", "--project", &project_arg, "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    assert!(
        stdout_json(&lint)["findings"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    let validate = run_powerbi(&["validate", "--strict", &project_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));

    let handoff = run_powerbi(&["handoff", "check", &project_arg, "--json"]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(
        stdout_json(&handoff)["safeForOfflineHandoff"],
        Value::Bool(true)
    );

    let audit = run_powerbi(&["report", "audit", "--project", &project_arg, "--json"]);
    assert_eq!(audit.code, 0, "stderr: {}", audit.stderr);
    let audit_json = stdout_json(&audit);
    assert_eq!(audit_json["ok"], Value::Bool(true));
    assert_eq!(audit_json["counts"]["findings"], 2);
    assert!(
        audit_json["findings"]
            .as_array()
            .expect("audit findings")
            .iter()
            .all(
                |finding| finding["ruleId"] == "filter.possible_persisted_values"
                    && finding["severity"] == "warning"
            )
    );

    let verify = run_powerbi(&[
        "fixture",
        "verify",
        &project_arg,
        "--expected",
        "testdata/golden/archetypes/regional-sales.summary.json",
        "--json",
    ]);
    assert_eq!(verify.code, 0, "stderr: {}", verify.stderr);
    assert_eq!(
        stdout_json(&verify)["verification"]["same"],
        Value::Bool(true)
    );
}

#[test]
fn report_spec_fields_lists_agent_safe_bindings_from_schema_and_profile() {
    let output = run_powerbi(&[
        "report",
        "spec",
        "fields",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.report.spec.fields.v1")
    );
    assert_eq!(value["ok"], Value::Bool(true));
    assert!(
        value["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .any(|field| field["reference"] == "FactSales[Total Revenue]"
                && field["kind"] == "measure")
    );
    assert!(
        value["tables"]
            .as_array()
            .expect("tables")
            .iter()
            .flat_map(|table| table["columns"].as_array().into_iter().flatten())
            .any(|column| column["reference"] == "DimCustomer[Segment]"
                && column["structuredBinding"]["table"] == "DimCustomer"
                && column["structuredBinding"]["column"] == "Segment")
    );
    assert!(
        value["next"]
            .as_array()
            .expect("next")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report spec validate"))
    );
}

#[test]
fn report_plan_builds_agent_dashboard_spec_from_schema_profile_and_objective() {
    let temp = tempfile::tempdir().expect("tempdir");
    let planned_spec = temp.path().join("planned-sales.dashboard.json");
    let project = temp.path().join("planned_sales");
    let planned_spec_arg = path_arg(&planned_spec);
    let project_arg = path_arg(&project);

    let plan = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--objective",
        "Executive sales overview with revenue trend, segment comparison, and portfolio scatter",
        "--out",
        &planned_spec_arg,
        "--json",
    ]);
    assert_eq!(plan.code, 0, "stderr: {}", plan.stderr);
    assert!(planned_spec.is_file());
    let value = stdout_json(&plan);
    assert_eq!(value["schema"], Value::from("powerbi-cli.report.plan.v1"));
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(
        value["spec"]["schema"],
        Value::from("powerbi-cli.dashboard.v1")
    );
    assert!(
        value["compiled"]["counts"]["visuals"]
            .as_i64()
            .expect("visual count")
            >= 5
    );
    assert!(
        value["decisions"]
            .as_array()
            .expect("decisions")
            .iter()
            .any(|decision| decision["kind"] == "primary-measure")
    );

    let spec_validate = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--spec",
        &planned_spec_arg,
        "--json",
    ]);
    assert_eq!(spec_validate.code, 0, "stderr: {}", spec_validate.stderr);
    assert_eq!(stdout_json(&spec_validate)["ok"], Value::Bool(true));

    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--spec",
        &planned_spec_arg,
        "--out-dir",
        &project_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);

    let validate = run_powerbi(&["validate", "--strict", &project_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));

    let handoff = run_powerbi(&["handoff", "check", &project_arg, "--json"]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(
        stdout_json(&handoff)["safeForOfflineHandoff"],
        Value::Bool(true)
    );
}

struct ArchetypeCase {
    slug: &'static str,
    schema: &'static str,
    profile: &'static str,
    spec: &'static str,
    golden: &'static str,
    expected_visuals: i64,
    expected_bindings: i64,
    check_dax: bool,
}

fn assert_archetype_golden(case: ArchetypeCase) {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join(case.slug);
    let project_arg = path_arg(&project);

    let schema = run_powerbi(&["schema", "validate", case.schema, "--json"]);
    assert_eq!(schema.code, 0, "stderr: {}", schema.stderr);
    assert_eq!(stdout_json(&schema)["ok"], Value::Bool(true));

    let profile = run_powerbi(&["profile", "validate", case.profile, "--json"]);
    assert_eq!(profile.code, 0, "stderr: {}", profile.stderr);
    assert_eq!(stdout_json(&profile)["ok"], Value::Bool(true));

    let spec = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        case.schema,
        "--profile",
        case.profile,
        "--spec",
        case.spec,
        "--json",
    ]);
    assert_eq!(spec.code, 0, "stderr: {}", spec.stderr);
    let spec_json = stdout_json(&spec);
    assert_eq!(spec_json["ok"], Value::Bool(true));
    assert_eq!(
        spec_json["compiled"]["counts"]["visuals"],
        Value::from(case.expected_visuals)
    );
    assert_eq!(
        spec_json["compiled"]["counts"]["bindings"],
        Value::from(case.expected_bindings)
    );

    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        case.schema,
        "--profile",
        case.profile,
        "--spec",
        case.spec,
        "--out-dir",
        &project_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);
    assert_eq!(stdout_json(&build)["ok"], Value::Bool(true));

    let validate = run_powerbi(&["validate", "--strict", &project_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));

    let handoff = run_powerbi(&["handoff", "check", &project_arg, "--json"]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(
        stdout_json(&handoff)["safeForOfflineHandoff"],
        Value::Bool(true)
    );

    if case.check_dax {
        let dependencies = run_powerbi(&[
            "model",
            "dax",
            "dependencies",
            "--project",
            &project_arg,
            "--json",
        ]);
        assert_eq!(
            dependencies.code, 0,
            "stdout: {}\nstderr: {}",
            dependencies.stdout, dependencies.stderr
        );
        assert!(
            stdout_json(&dependencies)["findings"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "DAX dependency findings: {}",
            dependencies.stdout
        );

        let lint = run_powerbi(&["model", "dax", "lint", "--project", &project_arg, "--json"]);
        assert_eq!(
            lint.code, 0,
            "stdout: {}\nstderr: {}",
            lint.stdout, lint.stderr
        );
        assert!(
            stdout_json(&lint)["findings"]
                .as_array()
                .is_some_and(Vec::is_empty),
            "DAX lint findings: {}",
            lint.stdout
        );
    }

    let verify = run_powerbi(&[
        "fixture",
        "verify",
        &project_arg,
        "--expected",
        case.golden,
        "--json",
    ]);
    assert_eq!(verify.code, 0, "stderr: {}", verify.stderr);
    assert_eq!(
        stdout_json(&verify)["verification"]["same"],
        Value::Bool(true)
    );
}

#[test]
fn dashboard_spec_model_measures_are_available_to_visual_bindings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("model-measure.dashboard.json");
    fs::write(
        &spec,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": {
                "name": "SalesOperations",
                "displayName": "Sales Operations"
            },
            "model": {
                "measures": [
                    {
                        "table": "FactSales",
                        "name": "Average Revenue",
                        "expression": "DIVIDE([Total Revenue], [Total Units])",
                        "formatString": "$#,##0.00"
                    }
                ]
            },
            "pages": [
                {
                    "id": "overview",
                    "displayName": "Overview",
                    "visuals": [
                        {
                            "id": "avg_revenue",
                            "type": "card",
                            "title": "Average Revenue",
                            "bindings": [
                                { "role": "Values", "field": "FactSales[Average Revenue]" }
                            ]
                        }
                    ]
                }
            ]
        }))
        .expect("serialize spec"),
    )
    .expect("write spec");
    let spec_arg = path_arg(&spec);

    let validate = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &spec_arg,
        "--json",
    ]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    let validate_json = stdout_json(&validate);
    assert_eq!(validate_json["ok"], Value::Bool(true));
    assert_eq!(
        validate_json["compiled"]["counts"]["measures"],
        Value::from(3)
    );

    let project = temp.path().join("model_measure_project");
    let project_arg = path_arg(&project);
    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &spec_arg,
        "--out-dir",
        &project_arg,
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);

    let inspect = run_powerbi(&["inspect", "--deep", &project_arg, "--json"]);
    assert_eq!(inspect.code, 0, "stderr: {}", inspect.stderr);
    let inspect_json = stdout_json(&inspect);
    assert!(
        inspect_json["deep"]["model"]["tables"]
            .as_array()
            .expect("tables")
            .iter()
            .any(|table| table["name"] == "FactSales"
                && table["measures"]
                    .as_array()
                    .expect("measures")
                    .iter()
                    .any(|measure| measure["name"] == "Average Revenue")),
        "inspect did not include spec-local measure: {}",
        inspect.stdout
    );
}

#[test]
fn dashboard_spec_validate_enforces_visual_binding_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let card_spec = temp.path().join("bad-card.dashboard.json");
    fs::write(
        &card_spec,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": { "name": "SalesOperations" },
            "pages": [
                {
                    "id": "overview",
                    "visuals": [
                        {
                            "id": "too_many_values",
                            "type": "card",
                            "bindings": [
                                { "role": "Values", "field": "FactSales[Total Revenue]" },
                                { "role": "Values", "field": "FactSales[Total Units]" }
                            ]
                        }
                    ]
                }
            ]
        }))
        .expect("serialize bad card"),
    )
    .expect("write bad card");
    let card_arg = path_arg(&card_spec);
    let card = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &card_arg,
        "--json",
    ]);
    assert_eq!(
        card.code, 10,
        "stdout: {}\nstderr: {}",
        card.stdout, card.stderr
    );
    let card_json = stdout_json(&card);
    assert_eq!(card_json["ok"], Value::Bool(false));
    assert!(
        card_json["errors"][0]
            .as_str()
            .unwrap_or_default()
            .contains("exactly one Values")
    );

    let scatter_spec = temp.path().join("bad-scatter.dashboard.json");
    fs::write(
        &scatter_spec,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": { "name": "SalesOperations" },
            "pages": [
                {
                    "id": "overview",
                    "visuals": [
                        {
                            "id": "missing_y",
                            "type": "scatter",
                            "bindings": [
                                { "role": "X", "field": "FactSales[Total Revenue]" }
                            ]
                        }
                    ]
                }
            ]
        }))
        .expect("serialize bad scatter"),
    )
    .expect("write bad scatter");
    let scatter_arg = path_arg(&scatter_spec);
    let scatter = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        &scatter_arg,
        "--json",
    ]);
    assert_eq!(
        scatter.code, 10,
        "stdout: {}\nstderr: {}",
        scatter.stdout, scatter.stderr
    );
    let scatter_json = stdout_json(&scatter);
    assert_eq!(scatter_json["ok"], Value::Bool(false));
    assert!(
        scatter_json["errors"][0]
            .as_str()
            .unwrap_or_default()
            .contains("exactly one X and exactly one Y")
    );
}

#[test]
fn dashboard_spec_validate_enforces_new_visual_binding_and_mode_contracts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        (
            "pie-two-categories",
            json!({
                "id": "bad_pie",
                "type": "pie",
                "bindings": [
                    { "role": "Category", "field": "CatalogFacts[Category]" },
                    { "role": "Category", "field": "CatalogFacts[Year]" },
                    { "role": "Y", "field": "CatalogFacts[Total Amount]" }
                ]
            }),
            "exactly one Category column binding",
        ),
        (
            "combo-no-line",
            json!({
                "id": "bad_combo",
                "type": "combo",
                "bindings": [
                    { "role": "Category", "field": "CatalogFacts[Category]" },
                    { "role": "Y", "field": "CatalogFacts[Total Amount]" }
                ]
            }),
            "at least one line-axis Y2 measure",
        ),
        (
            "matrix-no-values",
            json!({
                "id": "bad_matrix",
                "type": "matrix",
                "bindings": [
                    { "role": "Rows", "field": "CatalogFacts[Category]" },
                    { "role": "Columns", "field": "CatalogFacts[Year]" }
                ]
            }),
            "at least one Values binding",
        ),
        (
            "slicer-measure",
            json!({
                "id": "bad_slicer_measure",
                "type": "slicer",
                "bindings": [
                    { "role": "Values", "field": "CatalogFacts[Total Amount]" }
                ]
            }),
            "exactly one Values column binding",
        ),
        (
            "slicer-mode",
            json!({
                "id": "bad_slicer_mode",
                "type": "slicer",
                "mode": "relative",
                "bindings": [
                    { "role": "Values", "field": "CatalogFacts[Category]" }
                ]
            }),
            "unsupported slicer mode",
        ),
        (
            "single-select-non-slicer",
            json!({
                "id": "bad_single_select_card",
                "type": "card",
                "singleSelect": true,
                "bindings": [
                    { "role": "Values", "field": "CatalogFacts[Total Amount]" }
                ]
            }),
            "singleSelect is supported only for slicer visuals",
        ),
        (
            "single-select-not-boolean",
            json!({
                "id": "bad_single_select_type",
                "type": "slicer",
                "singleSelect": "yes",
                "bindings": [
                    { "role": "Values", "field": "CatalogFacts[Category]" }
                ]
            }),
            "singleSelect must be a boolean",
        ),
        (
            "slicer-too-short",
            json!({
                "id": "bad_short_slicer",
                "type": "slicer",
                "layout": { "x": 20, "y": 20, "width": 200, "height": 68 },
                "bindings": [
                    { "role": "Values", "field": "CatalogFacts[Category]" }
                ]
            }),
            "height must be at least 76",
        ),
        (
            "between-text-column",
            json!({
                "id": "bad_text_range",
                "type": "slicer",
                "mode": "between",
                "bindings": [
                    { "role": "Values", "field": "CatalogFacts[Category]" }
                ]
            }),
            "requires a numeric or date column",
        ),
        (
            "between-too-short",
            json!({
                "id": "bad_short_range",
                "type": "slicer",
                "mode": "between",
                "layout": { "x": 20, "y": 20, "width": 600, "height": 76 },
                "bindings": [
                    { "role": "Values", "field": "CatalogFacts[Year]" }
                ]
            }),
            "Between slicer height must be at least 104",
        ),
    ];

    for (slug, visual, expected) in cases {
        let spec_path = temp.path().join(format!("{slug}.dashboard.json"));
        fs::write(
            &spec_path,
            serde_json::to_string_pretty(&json!({
                "schema": "powerbi-cli.dashboard.v1",
                "report": { "name": "VisualCatalogProof" },
                "pages": [{ "id": "proof", "visuals": [visual] }]
            }))
            .expect("serialize invalid spec"),
        )
        .expect("write invalid spec");
        let spec_arg = path_arg(&spec_path);
        let result = run_powerbi(&[
            "report",
            "spec",
            "validate",
            "--schema",
            "examples/archetypes/catalog-proof.schema.json",
            "--spec",
            &spec_arg,
            "--json",
        ]);
        assert_eq!(
            result.code, 10,
            "{slug} stdout: {}\nstderr: {}",
            result.stdout, result.stderr
        );
        let result_json = stdout_json(&result);
        assert_eq!(result_json["ok"], Value::Bool(false));
        assert!(
            result_json["errors"]
                .as_array()
                .expect("validation errors")
                .iter()
                .filter_map(Value::as_str)
                .any(|error| error.contains(expected)),
            "{slug} errors: {}",
            result_json["errors"]
        );
    }

    let between_spec = temp.path().join("valid-between.dashboard.json");
    fs::write(
        &between_spec,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": { "name": "VisualCatalogProof" },
            "pages": [{
                "id": "proof",
                "visuals": [{
                    "id": "year_range",
                    "type": "slicer",
                    "mode": "between",
                    "bindings": [
                        { "role": "Values", "field": "CatalogFacts[Year]" }
                    ]
                }]
            }]
        }))
        .expect("serialize valid between spec"),
    )
    .expect("write valid between spec");
    let between_arg = path_arg(&between_spec);
    let between = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/archetypes/catalog-proof.schema.json",
        "--spec",
        &between_arg,
        "--json",
    ]);
    assert_eq!(between.code, 0, "stderr: {}", between.stderr);
    assert_eq!(stdout_json(&between)["ok"], Value::Bool(true));
}

#[test]
fn dashboard_build_emits_single_select_slicer_property() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_path = temp.path().join("single-select.dashboard.json");
    let project = temp.path().join("single_select_project");
    fs::write(
        &spec_path,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": { "name": "VisualCatalogProof" },
            "pages": [{
                "id": "proof",
                "visuals": [{
                    "id": "category",
                    "type": "slicer",
                    "mode": "basic",
                    "singleSelect": true,
                    "bindings": [
                        { "role": "Values", "field": "CatalogFacts[Category]" }
                    ]
                }]
            }]
        }))
        .expect("serialize single-select spec"),
    )
    .expect("write single-select spec");

    let build = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/archetypes/catalog-proof.schema.json",
        "--spec",
        &path_arg(&spec_path),
        "--out-dir",
        &path_arg(&project),
        "--json",
    ]);
    assert_eq!(build.code, 0, "stderr: {}", build.stderr);

    let visual_path = project
        .join("VisualCatalogProof.Report")
        .join("definition")
        .join("pages")
        .join("ReportSectionProof")
        .join("visuals")
        .join("VisualContainerCategory")
        .join("visual.json");
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(visual_path).expect("single-select visual.json"))
            .expect("parse single-select visual.json");
    assert_eq!(
        visual["visual"]["objects"]["selection"][0]["properties"]["singleSelect"]["expr"]["Literal"]
            ["Value"],
        Value::from("true")
    );
}

#[test]
fn report_spec_shape_only_validation_does_not_claim_build_compatibility() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("shape-only.dashboard.json");
    fs::write(
        &spec,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": { "name": "ShapeOnly" },
            "pages": [
                {
                    "id": "overview",
                    "visuals": [
                        {
                            "id": "bad_ref",
                            "type": "card",
                            "bindings": [
                                { "role": "Values", "field": "Nope[Missing]" }
                            ]
                        }
                    ]
                }
            ]
        }))
        .expect("serialize shape-only"),
    )
    .expect("write shape-only spec");
    let spec_arg = path_arg(&spec);

    let output = run_powerbi(&["report", "spec", "validate", "--spec", &spec_arg, "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert!(value["ok"].is_null());
    assert_eq!(value["validationLevel"], Value::from("shape-only"));
    assert!(
        value["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning
                .as_str()
                .unwrap_or_default()
                .contains("cannot prove field references"))
    );
    assert!(
        value["next"]
            .as_array()
            .expect("next")
            .iter()
            .any(|command| command.as_str().unwrap_or_default().contains("--schema"))
    );
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("path").to_string()
}
