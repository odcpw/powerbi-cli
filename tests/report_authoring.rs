use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
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

fn scaffold_sales(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

fn line_chart_handle(project: &Path) -> String {
    let project = project.to_str().expect("project path");
    let output = run_powerbi(&["report", "visuals", "list", "--project", project, "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    value["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == "lineChart")
        .and_then(|visual| visual["handle"].as_str())
        .expect("line chart handle")
        .to_string()
}

#[test]
fn design_plan_layout_and_theme_preset_are_agent_surfaces() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let design = run_powerbi(&["report", "design-plan", "--project", project_arg, "--json"]);
    assert_eq!(design.code, 0, "stderr: {}", design.stderr);
    let design_json = stdout_json(&design);
    assert_eq!(
        design_json["schema"],
        Value::from("powerbi-cli.report.designPlan.v1")
    );
    assert!(
        design_json["opportunities"]
            .as_array()
            .expect("opportunities")
            .iter()
            .any(|item| item["kind"] == "layout")
    );
    assert!(
        design_json["recommendedWorkflow"]
            .as_array()
            .expect("workflow")
            .iter()
            .any(|command| command
                .as_str()
                .unwrap_or_default()
                .contains("report themes apply-preset"))
    );

    let layout_out = temp.path().join("sales_layout");
    let layout_out_arg = layout_out.to_str().expect("layout path");
    let layout = run_powerbi(&[
        "report",
        "layout",
        "auto",
        "--project",
        project_arg,
        "--page",
        "page:ReportSectionOverview",
        "--preset",
        "grid",
        "--out-dir",
        layout_out_arg,
        "--json",
    ]);
    assert_eq!(layout.code, 0, "stderr: {}", layout.stderr);
    let layout_json = stdout_json(&layout);
    assert_eq!(
        layout_json["schema"],
        Value::from("powerbi-cli.report.layout.autoMutation.v1")
    );
    assert_eq!(layout_json["dryRun"], Value::Bool(false));
    assert!(
        layout_json["changes"]
            .as_array()
            .expect("layout changes")
            .len()
            >= 2
    );

    let theme_out = temp.path().join("sales_theme");
    let theme_out_arg = theme_out.to_str().expect("theme path");
    let theme = run_powerbi(&[
        "report",
        "themes",
        "apply-preset",
        "--project",
        project_arg,
        "--preset",
        "risk-dashboard",
        "--out-dir",
        theme_out_arg,
        "--json",
    ]);
    assert_eq!(theme.code, 0, "stderr: {}", theme.stderr);
    let theme_json = stdout_json(&theme);
    assert_eq!(
        theme_json["schema"],
        Value::from("powerbi-cli.report.themes.mutation.v1")
    );
    assert_eq!(
        theme_json["source"]["preset"],
        Value::from("risk-dashboard")
    );
    assert!(
        theme_out
            .join("SalesOperations.Report")
            .join("StaticResources")
            .join("RegisteredResources")
            .join("powerbi-cli-risk-dashboard.json")
            .is_file()
    );
}

#[test]
fn drilldown_set_hierarchy_replaces_category_projections() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let line_handle = line_chart_handle(&project);

    let bind = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &line_handle,
        "--binding",
        "role=Category,table=DimDate,column=Date",
        "--binding",
        "role=Y,table=FactSales,measure=Total Revenue",
        "--in-place",
        "--json",
    ]);
    assert_eq!(bind.code, 0, "stderr: {}", bind.stderr);

    let drilldown = run_powerbi(&[
        "report",
        "drilldown",
        "set-hierarchy",
        "--project",
        project_arg,
        "--handle",
        &line_handle,
        "--field",
        "DimDate[FiscalYear]",
        "--field",
        "DimDate[Month]",
        "--in-place",
        "--json",
    ]);
    assert_eq!(drilldown.code, 0, "stderr: {}", drilldown.stderr);
    let drilldown_json = stdout_json(&drilldown);
    assert_eq!(
        drilldown_json["schema"],
        Value::from("powerbi-cli.report.drilldown.hierarchyMutation.v1")
    );
    assert_eq!(
        drilldown_json["hierarchyPlan"]["after"]["projectionCount"],
        Value::from(2)
    );

    let visual_path = drilldown_json["target"]["path"]
        .as_str()
        .expect("visual path");
    let visual_json: Value =
        serde_json::from_str(&fs::read_to_string(visual_path).expect("read visual json"))
            .expect("parse visual json");
    let projections = visual_json["visual"]["query"]["queryState"]["Category"]["projections"]
        .as_array()
        .expect("category projections");
    assert_eq!(projections.len(), 2);
    assert_eq!(projections[0]["nativeQueryRef"], Value::from("FiscalYear"));
    assert_eq!(projections[1]["nativeQueryRef"], Value::from("Month"));
}
