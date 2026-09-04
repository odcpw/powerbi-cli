mod common;

use common::{assert_json_snapshot, run_powerbi, scaffold_sales, stdout_json};
use serde_json::Value;
use std::fs;

fn preview_page(value: &Value) -> Value {
    let mut page = value["preview"]["pages"][0].clone();
    // Page summaries intentionally carry absolute readback paths.  The grid
    // preview itself is path-free, so omit that summary from the golden.
    page.as_object_mut()
        .expect("preview page object")
        .remove("page");
    page
}

#[test]
fn named_template_dry_run_returns_svg_free_slots_and_does_not_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let visual_path = project
        .join("SalesOperations.Report/definition/pages/ReportSectionOverview/visuals")
        .read_dir()
        .expect("visual directory")
        .next()
        .expect("visual")
        .expect("visual entry")
        .path()
        .join("visual.json");
    let before = fs::read_to_string(&visual_path).expect("read visual");
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["schema"], "powerbi-cli.report.layout.autoMutation.v1");
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["preview"]["svg"], false);
    assert_eq!(value["preview"]["pages"][0]["template"]["name"], "overview");
    assert_eq!(value["preview"]["pages"][0]["grid"]["columns"], 12);
    assert!(
        value["preview"]["pages"][0]["slots"]
            .as_array()
            .expect("slots")
            .iter()
            .any(|slot| slot["name"] == "rail")
    );
    assert_eq!(
        fs::read_to_string(&visual_path).expect("read visual"),
        before
    );
}

#[test]
fn named_template_out_dir_is_deterministic_and_keeps_source_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let out_dir = temp.path().join("laid_out");
    let out_arg = out_dir.to_str().expect("out path");
    let first = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "kpi-strip-trend-breakdown",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let first_json = stdout_json(&first);
    assert_eq!(first_json["mode"], "out-dir");
    assert_eq!(first_json["dryRun"], false);
    assert!(out_dir.is_dir());
    let source_again = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "kpi-strip-trend-breakdown",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(source_again.code, 0, "stderr: {}", source_again.stderr);
    assert_eq!(
        preview_page(&first_json),
        preview_page(&stdout_json(&source_again)),
        "out-dir and source dry-run must resolve identical grid coordinates"
    );
}

#[test]
fn unknown_template_and_conflicting_layout_selectors_are_pointer_rich_refusals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let unknown = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "not-a-template",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unknown.code, 2);
    let unknown_error: Value = serde_json::from_str(unknown.stderr.trim()).expect("error JSON");
    assert_eq!(unknown_error["error"]["code"], "invalid_args");
    assert_eq!(unknown_error["error"]["pointer"], "/template");
    assert!(
        unknown_error["error"]["hint"]
            .as_str()
            .expect("hint")
            .contains("named templates")
    );

    let conflict = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--preset",
        "grid",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(conflict.code, 2);
    let conflict_error: Value = serde_json::from_str(conflict.stderr.trim()).expect("error JSON");
    assert_eq!(conflict_error["error"]["pointer"], "/template");
}

#[test]
fn family_mismatch_is_a_structured_warning_and_grid_overrides_are_echoed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let output = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "time-series",
        "--grid",
        "columns=12,gutter=20,margin=32,rowUnit=8",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["layoutPlan"]["grid"]["margin"], Value::from(32.0));
    let warnings = value["warnings"].as_array().expect("warnings");
    assert!(warnings.iter().any(|warning| {
        warning["code"] == "design.slot_family_mismatch"
            && warning["pointer"]
                .as_str()
                .is_some_and(|pointer| pointer.starts_with("/pages/0/visuals/"))
    }));
}

#[test]
fn three_named_templates_have_stable_golden_slot_layouts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    for template in ["kpi-strip-trend-breakdown", "overview", "time-series"] {
        let output = run_powerbi(&[
            "report",
            "layout",
            "auto",
            "--project",
            project_arg,
            "--template",
            template,
            "--dry-run",
            "--json",
        ]);
        assert_eq!(output.code, 0, "{template} stderr: {}", output.stderr);
        let value = stdout_json(&output);
        assert_json_snapshot(&format!("report-layout-{template}"), &preview_page(&value));
    }
}

#[test]
fn every_named_template_has_standard_and_wide_golden_coordinates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let add_page = run_powerbi(&[
        "report",
        "pages",
        "add",
        "--project",
        project_arg,
        "--name",
        "LayoutCanvas",
        "--display-name",
        "Layout Canvas",
        "--width",
        "1280",
        "--height",
        "720",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add_page.code, 0, "stderr: {}", add_page.stderr);

    let mut golden = serde_json::Map::new();
    for template in [
        "kpi-strip-trend-breakdown",
        "overview",
        "time-series",
        "ranking",
        "distribution",
        "comparison",
        "detail-table",
        "drillthrough-detail",
        "exception-list",
        "matrix-focus",
        "scatter-focus",
    ] {
        let mut sizes = serde_json::Map::new();
        for (label, size) in [("1280x720", "1280x720"), ("1920x1080", "1920x1080")] {
            let output = run_powerbi(&[
                "report",
                "layout",
                "auto",
                "--project",
                project_arg,
                "--page",
                "LayoutCanvas",
                "--template",
                template,
                "--page-size",
                size,
                "--dry-run",
                "--json",
            ]);
            assert_eq!(output.code, 0, "{template} {size}: {}", output.stderr);
            sizes.insert(label.to_string(), preview_page(&stdout_json(&output)));
        }
        golden.insert(template.to_string(), Value::Object(sizes));
    }
    assert_json_snapshot("report-layout-all-templates", &Value::Object(golden));
}
