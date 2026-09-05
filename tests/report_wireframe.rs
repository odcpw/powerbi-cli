mod common;

use common::{
    archetype_names, assert_json_snapshot, load_archetype, run_powerbi, scaffold_sales,
    stderr_json, stdout_json,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;

fn svg_content(value: &Value, index: usize) -> &str {
    value["artifacts"][index]["content"]
        .as_str()
        .expect("SVG artifact content")
}

fn snapshot_projection(value: &Value) -> Value {
    json!({
        "schema": value["schema"],
        "format": value["format"],
        "template": value["template"],
        "grid": value["grid"],
        "counts": value["counts"],
        "pages": value["pages"],
        "artifacts": value["artifacts"]
    })
}

#[test]
fn svg_dry_run_is_deterministic_and_contains_grid_slots_geometry_and_bindings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let args = [
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "svg",
        "--dry-run",
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    let first_json = stdout_json(&first);
    let second_json = stdout_json(&second);
    assert_eq!(first_json["schema"], "powerbi-cli.report.wireframe.v2");
    assert_eq!(first_json["format"], "svg");
    assert_eq!(first_json["dryRun"], true);
    assert_eq!(first_json["mode"], "dry-run");
    assert_eq!(first_json["counts"]["pages"], 1);
    assert_eq!(first_json["counts"]["visuals"], 3);
    assert_eq!(first_json["counts"]["slots"], 9);
    assert_eq!(svg_content(&first_json, 0), svg_content(&second_json, 0));
    let svg = svg_content(&first_json, 0);
    for marker in [
        "<g class=\"grid\"",
        "data-template=\"overview\"",
        "data-slot=\"heading\"",
        "data-col-span=\"10\"",
        "data-visual-type=\"lineChart\"",
        "Revenue Trend",
        "FactSales.Total Revenue",
    ] {
        assert!(svg.contains(marker), "SVG missing marker {marker:?}");
    }
    assert!(
        !svg.contains("<image"),
        "SVG must not embed external images"
    );
    assert!(!svg.contains("url("), "SVG must not use external CSS URLs");
    assert!(
        !temp.path().join("preview").exists(),
        "dry-run must not create an output artifact"
    );
}

#[test]
fn svg_out_writes_external_pages_and_is_byte_stable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let out_dir = temp.path().join("wireframe");
    let out_arg = out_dir.to_str().expect("wireframe output");
    let args = [
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "svg",
        "--out",
        out_arg,
        "--json",
    ];
    let first = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let first_json = stdout_json(&first);
    let artifact = out_dir.join("ReportSectionOverview.svg");
    assert!(artifact.is_file(), "SVG page artifact was not written");
    let first_bytes = fs::read(&artifact).expect("first SVG bytes");
    let second = run_powerbi(&args);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    let second_json = stdout_json(&second);
    let second_bytes = fs::read(&artifact).expect("second SVG bytes");
    assert_eq!(first_bytes, second_bytes, "SVG output must be byte-stable");
    assert_eq!(first_json["artifacts"][0]["bytes"], first_bytes.len());
    assert_eq!(second_json["artifacts"][0]["bytes"], second_bytes.len());
    assert_eq!(first_json["projectDir"], second_json["projectDir"]);
    assert!(
        !project.join("wireframe").exists(),
        "external wireframe output must not enter the project"
    );
}

#[test]
fn html_dry_run_wraps_every_page_with_a_stable_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = load_archetype("regional-sales").build_into(&temp.path().join("regional"));
    assert_eq!(project.code, 0, "stderr: {}", project.stderr);
    let project_dir = temp.path().join("regional");
    let project_arg = project_dir.to_str().expect("project path");
    let output = run_powerbi(&[
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "html",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["format"], "html");
    assert_eq!(value["dryRun"], true);
    assert_eq!(
        value["artifacts"].as_array().expect("HTML artifacts").len(),
        1
    );
    let html = value["artifacts"][0]["content"]
        .as_str()
        .expect("HTML content");
    assert!(html.contains("<nav>"));
    assert_eq!(html.matches("<section id=\"page-").count(), 3);
    assert_eq!(
        html.matches("<svg xmlns=\"http://www.w3.org/2000/svg\"")
            .count(),
        3
    );
    assert!(html.contains("href=\"#page-ReportSectionOverview-0\""));
    assert!(html.contains("ReportSectionCustomerDetail"));
    assert!(!html.contains("<image"));
    assert!(!html.contains("url("));
}

#[test]
fn wireframe_geometry_matches_the_applied_layout_plan() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let layout = run_powerbi(&[
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
    assert_eq!(layout.code, 0, "stderr: {}", layout.stderr);
    let layout_json = stdout_json(&layout);
    let assignments = layout_json["preview"]["pages"][0]["assignments"]
        .as_array()
        .expect("layout assignments")
        .iter()
        .map(|assignment| {
            (
                assignment["visual"].as_str().expect("assignment visual"),
                assignment["position"].clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let apply = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--template",
        "overview",
        "--in-place",
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let wireframe = run_powerbi(&[
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "svg",
        "--template",
        "overview",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(wireframe.code, 0, "stderr: {}", wireframe.stderr);
    let wireframe_json = stdout_json(&wireframe);
    for visual in wireframe_json["pages"][0]["visuals"]
        .as_array()
        .expect("wireframe visuals")
    {
        let handle = visual["handle"].as_str().expect("visual handle");
        let expected = assignments.get(handle).expect("layout visual assignment");
        for field in ["x", "y", "width", "height"] {
            let actual_number = visual["position"][field]
                .as_f64()
                .expect("wireframe position number");
            let expected_number = expected[field].as_f64().expect("layout position number");
            assert_eq!(
                actual_number, expected_number,
                "geometry differs for {handle} {field}"
            );
        }
    }
}

#[test]
fn every_archetype_has_a_byte_stable_svg_golden() {
    for name in archetype_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = load_archetype(name).build_into(&temp.path().join("project"));
        assert_eq!(project.code, 0, "{name} stderr: {}", project.stderr);
        let project_dir = temp.path().join("project");
        let project_arg = project_dir.to_str().expect("project path");
        let args = [
            "report",
            "wireframe",
            "export",
            project_arg,
            "--format",
            "svg",
            "--dry-run",
            "--json",
        ];
        let first = run_powerbi(&args);
        let second = run_powerbi(&args);
        assert_eq!(first.code, 0, "{name} stderr: {}", first.stderr);
        assert_eq!(second.code, 0, "{name} stderr: {}", second.stderr);
        let first_json = stdout_json(&first);
        let second_json = stdout_json(&second);
        assert_eq!(
            first_json["artifacts"], second_json["artifacts"],
            "{name} SVG must be deterministic"
        );
        assert_json_snapshot(
            &format!("report-wireframe-{name}"),
            &snapshot_projection(&first_json),
        );
    }
}

#[test]
fn three_named_templates_render_fixed_svg_golden_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    for template in ["overview", "time-series", "kpi-strip-trend-breakdown"] {
        let output = run_powerbi(&[
            "report",
            "wireframe",
            "export",
            project_arg,
            "--format",
            "svg",
            "--template",
            template,
            "--dry-run",
            "--json",
        ]);
        assert_eq!(output.code, 0, "{template} stderr: {}", output.stderr);
        let value = stdout_json(&output);
        assert_eq!(value["template"], template);
        assert!(svg_content(&value, 0).contains(&format!("data-template=\"{template}\"")));
        assert_json_snapshot(
            &format!("report-wireframe-template-{template}"),
            &snapshot_projection(&value),
        );
    }
}

#[test]
fn wireframe_refusals_are_pointer_rich_and_never_write_the_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let unsupported = run_powerbi(&[
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "png",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(unsupported.code, 2);
    let unsupported_error = stderr_json(&unsupported);
    assert_eq!(unsupported_error["error"]["code"], "invalid_args");
    assert_eq!(unsupported_error["error"]["pointer"], "/format");
    assert!(
        unsupported_error["error"]["hint"]
            .as_str()
            .expect("format hint")
            .contains("json, svg, or html")
    );
    assert!(
        unsupported_error["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("--format svg"))
    );

    let missing_mode = run_powerbi(&[
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "html",
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .expect("mode message")
            .contains("requires --dry-run or --out")
    );

    let inside = project.join("wireframe.svg");
    let inside_arg = inside.to_str().expect("inside path");
    let unsafe_output = run_powerbi(&[
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "svg",
        "--out",
        inside_arg,
        "--json",
    ]);
    assert_eq!(unsafe_output.code, 10);
    assert_eq!(
        stderr_json(&unsafe_output)["error"]["code"],
        "input_safety_violation"
    );
    assert!(!inside.exists(), "unsafe output must not be written");
}

#[test]
fn single_page_svg_file_output_and_html_file_output_are_supported() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let svg = temp.path().join("preview.svg");
    let html = temp.path().join("preview.html");
    let svg_arg = svg.to_str().expect("svg output");
    let html_arg = html.to_str().expect("html output");
    let svg_output = run_powerbi(&[
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "svg",
        "--out",
        svg_arg,
        "--json",
    ]);
    assert_eq!(svg_output.code, 0, "stderr: {}", svg_output.stderr);
    assert!(svg.is_file());
    let html_output = run_powerbi(&[
        "report",
        "wireframe",
        "export",
        project_arg,
        "--format",
        "html",
        "--out",
        html_arg,
        "--json",
    ]);
    assert_eq!(html_output.code, 0, "stderr: {}", html_output.stderr);
    assert!(html.is_file());
    assert!(
        fs::read_to_string(&svg)
            .expect("SVG file")
            .starts_with("<?xml")
    );
    assert!(
        fs::read_to_string(&html)
            .expect("HTML file")
            .starts_with("<!doctype html>")
    );
}
