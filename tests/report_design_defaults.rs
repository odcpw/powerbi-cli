mod common;

use common::{assert_json_snapshot, assert_tree_equal, run_powerbi, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

#[test]
fn report_build_without_design_defaults_preserves_scaffold_artifacts_byte_for_byte() {
    let temp = tempfile::tempdir().expect("tempdir");
    let scaffold = temp.path().join("scaffold");
    let build = temp.path().join("build");
    for (command, output) in [("scaffold", &scaffold), ("report-build", &build)] {
        let output_path = output.to_str().expect("output path");
        let result = if command == "scaffold" {
            run_powerbi(&[
                "scaffold",
                "--schema",
                "examples/sales.schema.json",
                "--out-dir",
                output_path,
                "--json",
            ])
        } else {
            run_powerbi(&[
                "report",
                "build",
                "--schema",
                "examples/sales.schema.json",
                "--out-dir",
                output_path,
                "--json",
            ])
        };
        assert_eq!(result.code, 0, "{command}: {}", result.stderr);
    }
    assert_tree_equal(
        &scaffold,
        &build,
        "defaults-disabled report build must preserve legacy scaffold output",
    );
}

#[test]
fn report_build_with_design_defaults_is_deterministic_and_matches_pbir_golden() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for output in [&first, &second] {
        let result = run_powerbi(&[
            "report",
            "build",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            "examples/sales.dashboard.v2.json",
            "--design-defaults",
            "--out-dir",
            output.to_str().expect("output path"),
            "--json",
        ]);
        assert_eq!(result.code, 0, "stderr: {}", result.stderr);
        let response = stdout_json(&result);
        assert_eq!(response["schema"], "powerbi-cli.report.build.v1");
        assert_eq!(response["scope"]["mode"], "out-dir");
        assert_eq!(
            response["defaultsApplied"].as_array().map(Vec::len),
            Some(3)
        );
    }
    assert_tree_equal(
        &first,
        &second,
        "defaults-enabled builds must be byte-identical",
    );
    assert_json_snapshot(
        "report-design-defaults-enabled",
        &json!({"visuals": visual_formatting_golden(&first)}),
    );
}

#[test]
fn design_defaults_show_and_spec_explain_publish_override_precedence_deterministically() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_path = temp.path().join("styled.dashboard.json");
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.v2.json").expect("sales spec"),
    )
    .expect("parse sales spec");
    spec["style"] = json!({
        "tokens": {"formatting": {"labels.show": false}},
        "defaults": {"labels.show": true}
    });
    spec["pages"][0]["visuals"][0]["format"] = json!({"labels.show": false});
    fs::write(
        &spec_path,
        serde_json::to_vec_pretty(&spec).expect("serialize styled spec"),
    )
    .expect("write styled spec");
    let spec_arg = spec_path.to_str().expect("spec path");

    let args = [
        "report", "design", "defaults", "show", "--spec", spec_arg, "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "show output must be deterministic"
    );
    let shown = stdout_json(&first);
    assert_eq!(shown["schema"], "powerbi-cli.report.design.defaults.v1");
    assert_eq!(shown["defaultsEnabled"], true);
    assert_eq!(
        shown["mergeOrder"],
        json!([
            "catalog",
            "style.tokens",
            "style.defaults",
            "visuals[].format"
        ])
    );
    let card_labels = shown["visuals"][0]["defaults"]
        .as_array()
        .expect("card defaults")
        .iter()
        .find(|value| value["catalogKey"] == "labels.show")
        .expect("labels.show");
    assert_eq!(card_labels["value"], false);
    assert_eq!(card_labels["source"], "visuals[].format");
    let line_labels = shown["visuals"][1]["defaults"]
        .as_array()
        .expect("line defaults")
        .iter()
        .find(|value| value["catalogKey"] == "labels.show")
        .expect("labels.show");
    assert_eq!(line_labels["value"], true);
    assert_eq!(line_labels["source"], "style.defaults");

    let explain = run_powerbi(&[
        "report",
        "spec",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec_arg,
        "--json",
    ]);
    assert_eq!(explain.code, 0, "stderr: {}", explain.stderr);
    let explained = stdout_json(&explain);
    assert_eq!(
        explained["defaults"]["perVisual"][0]["designDefaultsEnabled"],
        true
    );
    assert!(
        explained["defaults"]["perVisual"][1]["designDefaults"]
            .as_array()
            .is_some_and(|defaults| defaults.iter().any(|value| value["inert"] == true)),
        "spec explain must expose inert defaults and their owning T4 bead"
    );
}

#[test]
fn token_theme_and_design_defaults_compile_together_with_visual_precedence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut spec: Value =
        serde_json::from_str(include_str!("../examples/sales.dashboard.v2.json")).expect("spec");
    spec["style"] = json!({
        "tokens": {"preset": "dark", "formatting": {"labels.show": false}},
        "defaults": {"labels.show": true}
    });
    spec["pages"][0]["visuals"][0]["format"] = json!({"labels.show": false});
    let path = temp.path().join("combined.json");
    fs::write(&path, serde_json::to_vec_pretty(&spec).expect("serialize")).expect("write");
    let project = temp.path().join("project");
    let run = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        path.to_str().expect("path"),
        "--out-dir",
        project.to_str().expect("project"),
        "--json",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(stdout_json(&run)["styleTokens"]["id"], "dark");
    assert!(project.join("SalesOperations.Report/StaticResources/RegisteredResources/powerbi-cli-tokens-dark.json").is_file());
    let visuals = visual_formatting_golden(&project);
    let card = visuals
        .iter()
        .find(|v| v["visualType"] == "card")
        .expect("card");
    let line = visuals
        .iter()
        .find(|v| v["visualType"] == "lineChart")
        .expect("line");
    assert_eq!(
        card["objects"]["labels"][0]["properties"]["show"]["expr"]["Literal"]["Value"],
        "false"
    );
    assert_eq!(
        line["objects"]["labels"][0]["properties"]["show"]["expr"]["Literal"]["Value"],
        "true"
    );
}

fn visual_formatting_golden(project: &Path) -> Vec<Value> {
    let mut visuals = WalkDir::new(project)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() == "visual.json")
        .map(|entry| {
            let value: Value =
                serde_json::from_str(&fs::read_to_string(entry.path()).expect("read visual.json"))
                    .expect("parse visual.json");
            json!({
                "name": value["name"],
                "visualType": value["visual"]["visualType"],
                "objects": value["visual"]["objects"],
                "visualContainerObjects": value["visual"]["visualContainerObjects"]
            })
        })
        .collect::<Vec<_>>();
    visuals.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    visuals
}
