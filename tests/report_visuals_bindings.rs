//! Report visual catalog, authoring, binding, positioning, cloning, and deletion integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

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
#[test]
fn report_visual_explicit_sort_refuses_unproven_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_catalog_proof(temp.path());
    let project_arg = project.to_str().expect("project path");
    let base = [
        "report",
        "visuals",
        "add",
        "--project",
        project_arg,
        "--page",
        "page:ReportSectionLineControl",
        "--visual-type",
        "combo",
        "--title",
        "Sort Refusal",
    ];

    let ascending = run_powerbi(
        &base
            .iter()
            .copied()
            .chain([
                "--binding",
                "role=Category,table=CatalogFacts,column=Category",
                "--binding",
                "role=Y,table=CatalogFacts,measure=Total Amount,sort=ascending",
                "--binding",
                "role=Y2,table=CatalogFacts,measure=Cumulative Share",
                "--dry-run",
                "--json",
            ])
            .collect::<Vec<_>>(),
    );
    assert_eq!(ascending.code, 2);
    assert_unsupported_feature(&ascending.stderr, "unsupported visual sort direction");

    let category = run_powerbi(
        &base
            .iter()
            .copied()
            .chain([
                "--binding",
                "role=Category,table=CatalogFacts,column=Category,sort=descending",
                "--binding",
                "role=Y,table=CatalogFacts,measure=Total Amount",
                "--binding",
                "role=Y2,table=CatalogFacts,measure=Cumulative Share",
                "--dry-run",
                "--json",
            ])
            .collect::<Vec<_>>(),
    );
    assert_eq!(category.code, 2);
    assert_unsupported_feature(&category.stderr, "proven only for measures");

    let multi_key = run_powerbi(
        &base
            .iter()
            .copied()
            .chain([
                "--binding",
                "role=Category,table=CatalogFacts,column=Category",
                "--binding",
                "role=Y,table=CatalogFacts,measure=Total Amount,sort=descending",
                "--binding",
                "role=Y2,table=CatalogFacts,measure=Cumulative Share,sort=descending",
                "--dry-run",
                "--json",
            ])
            .collect::<Vec<_>>(),
    );
    assert_eq!(multi_key.code, 2);
    assert_unsupported_feature(&multi_key.stderr, "exactly one explicit sort binding");
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
        Value::from("powerbi-cli.report.visuals.catalog.v2")
    );
    let supported = catalog_json["supportedVisualTypes"]
        .as_array()
        .expect("supported types");
    assert!(supported.iter().any(|value| value == "card"));
    assert!(supported.iter().any(|value| value == "areaChart"));
    assert!(supported.iter().any(|value| value == "barChart"));
    assert!(supported.iter().any(|value| value == "scatterChart"));
    assert!(
        supported
            .iter()
            .any(|value| value == "hundredPercentStackedColumnChart")
    );
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
            json!(["measure", "aggregatedColumn"])
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
            slug: "combo",
            requested_type: "combo",
            canonical_type: "lineClusteredColumnComboChart",
            mode: None,
            add_bindings: &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,measure=Total Amount,sort=descending",
                "role=Y2,table=CatalogFacts,measure=Cumulative Share",
            ],
            replacement_bindings: &[
                "role=Category,table=CatalogFacts,column=Category",
                "role=Y,table=CatalogFacts,measure=Total Amount,sort=descending",
                "role=Y2,table=CatalogFacts,measure=Cumulative Share",
            ],
            roles: &["Category", "Y", "Y2"],
        },
        CatalogVisualCase {
            slug: "matrix",
            requested_type: "matrix",
            canonical_type: "pivotTable",
            mode: None,
            add_bindings: &[
                "role=Rows,table=CatalogFacts,column=Category",
                "role=Rows,table=CatalogFacts,column=Year",
                "role=Columns,table=CatalogFacts,column=Amount",
                "role=Values,table=CatalogFacts,measure=Total Amount",
            ],
            replacement_bindings: &[
                "role=Rows,table=CatalogFacts,column=Category",
                "role=Rows,table=CatalogFacts,column=Year",
                "role=Columns,table=CatalogFacts,column=Amount",
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
        } else if case.canonical_type == "lineClusteredColumnComboChart" {
            assert_eq!(
                raw["visual"]["query"]["sortDefinition"]["sort"][0]["field"]["Measure"]["Property"],
                "Total Amount"
            );
            assert_eq!(
                raw["visual"]["query"]["sortDefinition"]["sort"][0]["direction"],
                "Descending"
            );
            assert!(
                raw["visual"]["query"]["sortDefinition"]
                    .get("isDefaultSort")
                    .is_none()
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
            assert_eq!(
                raw["visual"]["objects"]["rowHeaders"][0]["properties"]["showExpandCollapseButtons"]
                    ["expr"]["Literal"]["Value"],
                "true"
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
        } else if case.canonical_type == "lineClusteredColumnComboChart" {
            assert_eq!(
                bind_json["changes"][0]["after"]["sortDefinition"]["sort"][0]["field"]["Measure"]["Property"],
                "Total Amount"
            );
            assert_eq!(
                bind_json["bindingPlan"]["after"][1]["sortDirection"],
                "Descending"
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
    assert_eq!(
        between_json["changes"][0]["after"]["visual"]["objects"]["slider"][0]["properties"]["show"]
            ["expr"]["Literal"]["Value"],
        "true"
    );

    let between_too_short = run_powerbi(&[
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
        "Clipped range",
        "--binding",
        "role=Values,table=CatalogFacts,column=Year",
        "--height",
        "76",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(between_too_short.code, 2);
    assert!(
        stderr_json(&between_too_short)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Between slicer height 76")
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
