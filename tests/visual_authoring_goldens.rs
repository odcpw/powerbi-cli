use serde_json::Value;
mod common;

use common::cli_command;
use std::path::{Path, PathBuf};
use std::process::Output;

fn run_powerbi(args: &[String]) -> Output {
    cli_command(args).output()
}

fn scaffold_sales(root: &Path) -> PathBuf {
    let project = root.join("sales_project");
    let output = run_powerbi(&[
        "scaffold".to_string(),
        "--schema".to_string(),
        "examples/sales.schema.json".to_string(),
        "--out-dir".to_string(),
        project.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    assert!(
        output.status.success(),
        "scaffold stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    project
}

fn add_visual(
    project: &Path,
    visual_type: &str,
    name: &str,
    title: &str,
    bindings: &[&str],
) -> Output {
    let mut args = vec![
        "report".to_string(),
        "visuals".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--page".to_string(),
        "page:ReportSectionOverview".to_string(),
        "--visual-type".to_string(),
        visual_type.to_string(),
        "--name".to_string(),
        name.to_string(),
        "--title".to_string(),
        title.to_string(),
    ];
    for binding in bindings {
        args.push("--binding".to_string());
        args.push((*binding).to_string());
    }
    args.extend([
        "--x".to_string(),
        "10".to_string(),
        "--y".to_string(),
        "20".to_string(),
        "--width".to_string(),
        "300".to_string(),
        "--height".to_string(),
        "200".to_string(),
        "--z".to_string(),
        "7".to_string(),
        "--tab-order".to_string(),
        "9".to_string(),
        "--dry-run".to_string(),
        "--json".to_string(),
    ]);
    run_powerbi(&args)
}

fn assert_golden(output: Output, expected: &str) {
    assert!(
        output.status.success(),
        "visual add stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual_output: Value = serde_json::from_slice(&output.stdout).expect("visual add JSON");
    let expected: Value = serde_json::from_str(expected).expect("golden visual JSON");
    assert_eq!(actual_output["changes"][0]["after"], expected);
}

#[test]
fn report_visual_add_matches_2026_08_desktop_rendered_goldens() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());

    // These exact shapes replicate Desktop-rendered fixtures captured during the
    // 2026-08 production pilot, satisfying the roadmap's Desktop-authored precondition.
    assert_golden(
        add_visual(
            &project,
            "card",
            "PilotCard",
            "Revenue Card",
            &["role=Values,table=FactSales,measure=Total Revenue"],
        ),
        include_str!("../testdata/golden/visual-authoring/card.visual.json"),
    );
    assert_golden(
        add_visual(
            &project,
            "tableEx",
            "PilotTable",
            "Sales Detail",
            &[
                "role=Values,table=DimCustomer,column=CustomerName",
                "role=Values,table=FactSales,measure=Total Revenue",
                "role=Values,table=FactSales,column=Units",
            ],
        ),
        include_str!("../testdata/golden/visual-authoring/tableEx.visual.json"),
    );
    assert_golden(
        add_visual(
            &project,
            "lineChart",
            "PilotLine",
            "Revenue Trend",
            &[
                "role=Category,table=DimDate,column=Month",
                "role=Series,table=DimCustomer,column=Segment",
                "role=Y,table=FactSales,measure=Total Revenue",
            ],
        ),
        include_str!("../testdata/golden/visual-authoring/lineChart.visual.json"),
    );
    assert_golden(
        add_visual(
            &project,
            "scatterChart",
            "PilotScatter",
            "Revenue vs Units",
            &[
                "role=Category,table=DimCustomer,column=CustomerName",
                "role=X,table=FactSales,column=Revenue",
                "role=Y,table=FactSales,measure=Total Units",
                "role=Size,table=FactSales,column=Units",
            ],
        ),
        include_str!("../testdata/golden/visual-authoring/scatterChart.visual.json"),
    );
    assert_golden(
        add_visual(
            &project,
            "hundredPercentStackedColumnChart",
            "PilotHundredPercent",
            "Revenue Mix",
            &[
                "role=Category,table=DimDate,column=Month",
                "role=Series,table=DimCustomer,column=Segment",
                "role=Y,table=FactSales,column=Revenue",
            ],
        ),
        include_str!(
            "../testdata/golden/visual-authoring/hundredPercentStackedColumnChart.visual.json"
        ),
    );
}

#[test]
fn scatter_details_role_is_rejected_with_category_hint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let output = add_visual(
        &project,
        "scatterChart",
        "RejectedDetails",
        "Rejected Details",
        &[
            "role=Details,table=DimCustomer,column=CustomerName",
            "role=X,table=FactSales,measure=Total Revenue",
            "role=Y,table=FactSales,measure=Total Units",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stderr).expect("error JSON");
    assert_eq!(error["error"]["code"], "unsupported_feature");
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Details"))
    );
    assert!(
        error["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("Category"))
    );
}

#[test]
fn aggregated_scatter_columns_round_trip_through_visual_readback() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let target = temp.path().join("scatter_project");
    let output = run_powerbi(&[
        "report".to_string(),
        "visuals".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--page".to_string(),
        "page:ReportSectionOverview".to_string(),
        "--visual-type".to_string(),
        "scatterChart".to_string(),
        "--name".to_string(),
        "ReadbackScatter".to_string(),
        "--title".to_string(),
        "Readback Scatter".to_string(),
        "--binding".to_string(),
        "role=Category,table=DimCustomer,column=CustomerName".to_string(),
        "--binding".to_string(),
        "role=X,table=FactSales,column=Revenue".to_string(),
        "--binding".to_string(),
        "role=Y,table=FactSales,measure=Total Units".to_string(),
        "--binding".to_string(),
        "role=Size,table=FactSales,column=Units".to_string(),
        "--out-dir".to_string(),
        target.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    assert!(
        output.status.success(),
        "scatter add stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let added: Value = serde_json::from_slice(&output.stdout).expect("add JSON");
    let handle = added["target"]["handle"].as_str().expect("visual handle");
    let show = run_powerbi(&[
        "report".to_string(),
        "visuals".to_string(),
        "show".to_string(),
        "--project".to_string(),
        target.to_string_lossy().into_owned(),
        "--handle".to_string(),
        handle.to_string(),
        "--json".to_string(),
    ]);
    assert!(
        show.status.success(),
        "scatter show stderr: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let shown: Value = serde_json::from_slice(&show.stdout).expect("show JSON");
    let bindings = shown["visual"]["bindings"]
        .as_array()
        .expect("readback bindings");
    for (role, column) in [("X", "Revenue"), ("Size", "Units")] {
        assert!(bindings.iter().any(|binding| {
            binding["role"] == role
                && binding["kind"] == "aggregatedColumn"
                && binding["column"] == column
        }));
    }
}
