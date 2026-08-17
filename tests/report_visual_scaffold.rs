//! Desktop-proven card, slicer, and textbox scaffold command tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn existing_visual_names(project: &Path) -> Vec<String> {
    let visuals_dir = first_page_json(project)
        .parent()
        .expect("page dir")
        .join("visuals");
    let mut names = fs::read_dir(&visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("file type").is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn page_max_stack(project: &Path) -> u64 {
    let visuals_dir = first_page_json(project)
        .parent()
        .expect("page dir")
        .join("visuals");
    fs::read_dir(visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("file type").is_dir())
        .map(|entry| {
            let visual: Value = serde_json::from_str(
                &fs::read_to_string(entry.path().join("visual.json")).expect("visual json"),
            )
            .expect("parse visual json");
            json_u64(&visual["position"]["z"]).max(json_u64(&visual["position"]["tabOrder"]))
        })
        .max()
        .unwrap_or(0)
}

fn json_u64(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| value.as_f64().and_then(|n| (n >= 0.0).then_some(n as u64)))
        .unwrap_or(0)
}

fn page_visual_count(project: &Path) -> usize {
    existing_visual_names(project).len()
}

fn assert_missing_mode_refused(project: &Path, action: &str, extra: &[&str]) {
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(project);
    let mut args = vec![
        "report",
        "visuals",
        action,
        "--project",
        project_arg,
        "--page",
        &page,
        "--title",
        "No Mode",
        "--x",
        "900",
        "--y",
        "20",
        "--width",
        "160",
        "--height",
        "80",
        "--json",
    ];
    args.extend_from_slice(extra);
    let before = page_visual_count(project);
    let output = run_powerbi(&args);
    assert_eq!(output.code, 2, "stderr: {}", output.stderr);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], Value::from("invalid_args"));
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--dry-run"),
        "expected mutation-mode refusal: {error}"
    );
    assert_eq!(page_visual_count(project), before);
}

#[test]
fn add_card_dry_run_plans_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(&project);
    let before = page_visual_count(&project);
    let output = run_powerbi(&[
        "report",
        "visuals",
        "add-card",
        "--project",
        project_arg,
        "--page",
        &page,
        "--measure",
        "FactSales.Total Revenue",
        "--title",
        "Units KPI",
        "--x",
        "900",
        "--y",
        "20",
        "--width",
        "160",
        "--height",
        "80",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["dryRun"], Value::Bool(true));
    assert_eq!(value["action"], Value::from("add-card"));
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["changes"][0]["before"], Value::Null);
    assert_eq!(
        value["changes"][0]["after"]["visual"]["visualType"],
        Value::from("card")
    );
    assert!(
        value["readbackCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("report visuals show")
    );
    assert!(
        value["validateCommand"]
            .as_str()
            .unwrap_or_default()
            .contains("validate --strict")
    );
    assert_eq!(page_visual_count(&project), before);
    let planned_path = value["target"]["path"].as_str().expect("planned path");
    assert!(!Path::new(planned_path).exists());
}

#[test]
fn add_card_in_place_writes_desktop_template_and_increments_stack() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(&project);
    let expected_stack = page_max_stack(&project) + 1;
    let output = run_powerbi(&[
        "report",
        "visuals",
        "add-card",
        "--project",
        project_arg,
        "--page",
        &page,
        "--measure",
        "FactSales.Total Revenue",
        "--title",
        "O'Reilly KPI",
        "--x",
        "900",
        "--y",
        "20",
        "--width",
        "160",
        "--height",
        "80",
        "--value-font-size",
        "20",
        "--category-font-size",
        "9",
        "--word-wrap",
        "--in-place",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["dryRun"], Value::Bool(false));
    assert_eq!(value["ok"], Value::Bool(true));
    let after = &value["changes"][0]["after"];
    assert_eq!(
        after["$schema"],
        Value::from(
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json"
        )
    );
    assert_eq!(after["howCreated"], Value::from("DraggedToFieldWell"));
    assert_eq!(after["visual"]["visualType"], Value::from("card"));
    assert_eq!(
        after["visual"]["objects"]["labels"][0]["properties"]["fontSize"]["expr"]["Literal"]["Value"],
        Value::from("20D")
    );
    assert_eq!(
        after["visual"]["objects"]["categoryLabels"][0]["properties"]["show"]["expr"]["Literal"]["Value"],
        Value::from("true")
    );
    assert_eq!(
        after["visual"]["objects"]["categoryLabels"][0]["properties"]["wordWrap"]["expr"]["Literal"]
            ["Value"],
        Value::from("true")
    );
    assert_eq!(
        after["visual"]["visualContainerObjects"]["title"][0]["properties"]["text"]["expr"]["Literal"]
            ["Value"],
        Value::from("'O''Reilly KPI'")
    );
    assert_eq!(after["position"]["tabOrder"], json!(expected_stack));
    assert_eq!(after["position"]["z"], json!(expected_stack));
    let written_path = Path::new(value["target"]["path"].as_str().expect("path"));
    let written: Value =
        serde_json::from_str(&fs::read_to_string(written_path).expect("written visual"))
            .expect("parse written visual");
    assert_eq!(written, *after);
    assert_eq!(
        value["target"]["name"],
        Value::from("VisualContainerOReillyKpi")
    );
    assert_strict_valid(&project);
}

#[test]
fn add_slicer_mode_and_single_select_literals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(&project);

    let dropdown = run_powerbi(&[
        "report",
        "visuals",
        "add-slicer",
        "--project",
        project_arg,
        "--page",
        &page,
        "--field",
        "DimCustomer.Segment",
        "--title",
        "Segment Dropdown",
        "--x",
        "900",
        "--y",
        "20",
        "--width",
        "200",
        "--height",
        "80",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dropdown.code, 0, "stderr: {}", dropdown.stderr);
    let dropdown_after = stdout_json(&dropdown)["changes"][0]["after"].clone();
    assert_eq!(
        dropdown_after["visual"]["objects"]["data"][0]["properties"]["mode"]["expr"]["Literal"]["Value"],
        Value::from("'Dropdown'")
    );
    assert!(dropdown_after["visual"]["objects"]["selection"].is_null());
    assert_eq!(
        dropdown_after["visual"]["query"]["queryState"]["Values"]["projections"][0]["active"],
        Value::Bool(true)
    );
    assert_eq!(
        dropdown_after["visual"]["query"]["queryState"]["Values"]["projections"][0]["field"]["Column"]
            ["Property"],
        Value::from("Segment")
    );

    let basic = run_powerbi(&[
        "report",
        "visuals",
        "add-slicer",
        "--project",
        project_arg,
        "--page",
        &page,
        "--field",
        "DimCustomer.Segment",
        "--title",
        "Segment Basic",
        "--mode",
        "Basic",
        "--single-select",
        "--x",
        "900",
        "--y",
        "110",
        "--width",
        "200",
        "--height",
        "80",
        "--in-place",
        "--json",
    ]);
    assert_eq!(basic.code, 0, "stderr: {}", basic.stderr);
    let basic_after = stdout_json(&basic)["changes"][0]["after"].clone();
    assert_eq!(
        basic_after["visual"]["objects"]["data"][0]["properties"]["mode"]["expr"]["Literal"]["Value"],
        Value::from("'Basic'")
    );
    assert_eq!(
        basic_after["visual"]["objects"]["selection"][0]["properties"]["singleSelect"]["expr"]["Literal"]
            ["Value"],
        Value::from("true")
    );
    assert_strict_valid(&project);
}

#[test]
fn add_textbox_from_three_line_file_preserves_utf8() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(&project);
    let paragraphs = temp.path().join("guide.txt");
    fs::write(
        &paragraphs,
        "Lesen Sie hier\nZweite Zeile mit Umlauten: äöüß\nDritte Zeile\n\n",
    )
    .expect("write paragraphs");
    let output = run_powerbi(&[
        "report",
        "visuals",
        "add-textbox",
        "--project",
        project_arg,
        "--page",
        &page,
        "--title",
        "Reading Guide",
        "--paragraphs-file",
        paragraphs.to_str().expect("paragraphs path"),
        "--x",
        "40",
        "--y",
        "540",
        "--width",
        "400",
        "--height",
        "140",
        "--in-place",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let after = stdout_json(&output)["changes"][0]["after"].clone();
    assert_eq!(after["howCreated"], Value::from("InsertVisualButton"));
    assert_eq!(after["visual"]["visualType"], Value::from("textbox"));
    assert!(after["visual"].get("query").is_none());
    let annotations = after["annotations"].as_array().expect("annotations");
    assert_eq!(annotations.len(), 1);
    assert_eq!(
        annotations[0]["name"],
        Value::from("powerbi-cli.placeholderTitle")
    );
    let paragraphs_json = after["visual"]["objects"]["general"][0]["properties"]["paragraphs"]
        .as_array()
        .expect("paragraphs");
    assert_eq!(paragraphs_json.len(), 3);
    assert_eq!(
        paragraphs_json[0]["textRuns"][0]["value"],
        Value::from("Lesen Sie hier")
    );
    assert_eq!(
        paragraphs_json[0]["textRuns"][0]["textStyle"],
        json!({"fontWeight": "bold", "fontSize": "12pt"})
    );
    assert_eq!(
        paragraphs_json[1]["textRuns"][0]["value"],
        Value::from("Zweite Zeile mit Umlauten: äöüß")
    );
    assert_eq!(
        paragraphs_json[1]["textRuns"][0]["textStyle"],
        json!({"fontSize": "10pt"})
    );
    assert_eq!(
        paragraphs_json[2]["textRuns"][0]["textStyle"],
        json!({"fontSize": "10pt"})
    );
    let result = stdout_json(&output);
    let written_path = result["target"]["path"].as_str().expect("written path");
    let written = fs::read(written_path).expect("written bytes");
    let umlauts = "äöüß".as_bytes();
    assert!(
        written
            .windows(umlauts.len())
            .any(|window| window == umlauts),
        "umlauts must survive byte-exact in visual.json"
    );
    assert_strict_valid(&project);
}

#[test]
fn explicit_name_collision_refuses_and_auto_name_is_unique() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let page = first_page_name(&project);
    let existing = existing_visual_names(&project);
    let taken = existing.first().expect("existing visual");
    let before = existing.len();
    let collision = run_powerbi(&[
        "report",
        "visuals",
        "add-card",
        "--project",
        project_arg,
        "--page",
        &page,
        "--measure",
        "FactSales.Total Units",
        "--title",
        "Collision",
        "--name",
        taken,
        "--x",
        "900",
        "--y",
        "20",
        "--width",
        "160",
        "--height",
        "80",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(collision.code, 2, "stderr: {}", collision.stderr);
    let error = stderr_json(&collision);
    assert_eq!(error["error"]["code"], Value::from("invalid_args"));
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("already exists"),
        "expected name collision: {error}"
    );
    assert_eq!(page_visual_count(&project), before);

    let first = run_powerbi(&[
        "report",
        "visuals",
        "add-card",
        "--project",
        project_arg,
        "--page",
        &page,
        "--measure",
        "FactSales.Total Units",
        "--title",
        "Shared Title",
        "--x",
        "900",
        "--y",
        "20",
        "--width",
        "160",
        "--height",
        "80",
        "--in-place",
        "--json",
    ]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let first_name = stdout_json(&first)["target"]["name"]
        .as_str()
        .expect("first name")
        .to_string();
    assert_eq!(first_name, "VisualContainerSharedTitle");

    let second = run_powerbi(&[
        "report",
        "visuals",
        "add-card",
        "--project",
        project_arg,
        "--page",
        &page,
        "--measure",
        "FactSales.Total Units",
        "--title",
        "Shared Title",
        "--x",
        "900",
        "--y",
        "110",
        "--width",
        "160",
        "--height",
        "80",
        "--in-place",
        "--json",
    ]);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    let second_name = stdout_json(&second)["target"]["name"]
        .as_str()
        .expect("second name")
        .to_string();
    assert_eq!(second_name, "VisualContainerSharedTitle2");
    assert_ne!(first_name, second_name);
}

#[test]
fn scaffold_commands_refuse_without_mutation_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    assert_missing_mode_refused(
        &project,
        "add-card",
        &["--measure", "FactSales.Total Revenue"],
    );
    assert_missing_mode_refused(&project, "add-slicer", &["--field", "DimCustomer.Segment"]);
    assert_missing_mode_refused(&project, "add-textbox", &["--text", "Hello"]);
}
