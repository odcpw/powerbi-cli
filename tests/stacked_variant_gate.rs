mod common;

use common::{hash_tree, run_powerbi, scaffold_sales, stdout_json};
use serde_json::Value;

#[test]
fn stacked_aliases_publish_only_the_recorded_ids_and_role_evidence() {
    for (alias, canonical, evidence) in [
        (
            "stackedBarChart",
            "barChart",
            "docs/desktop-acceptance-everything.md",
        ),
        (
            "stackedColumnChart",
            "columnChart",
            "docs/desktop-acceptance-everything.md",
        ),
        (
            "100percentstackedcolumn",
            "hundredPercentStackedColumnChart",
            "testdata/golden/visual-authoring/hundredPercentStackedColumnChart.visual.json",
        ),
    ] {
        let args = [
            "report",
            "visuals",
            "catalog",
            "--visual-type",
            alias,
            "--json",
        ];
        let first = run_powerbi(&args);
        assert_eq!(first.code, 0, "{}", first.stderr);
        assert_eq!(first.stdout, run_powerbi(&args).stdout);
        let catalog = stdout_json(&first);
        assert_eq!(catalog["supportedVisualTypes"][0], canonical);
        assert_eq!(
            catalog["rules"][0]["required"],
            serde_json::json!(["Category", "Y"])
        );
        assert!(
            catalog["rules"][0]["evidence"]
                .as_array()
                .unwrap()
                .contains(&Value::from(evidence))
        );
    }
}

#[test]
fn unproven_variants_refuse_with_missing_evidence_without_writes_in_every_mode() {
    let temp = tempfile::tempdir().unwrap();
    let project = scaffold_sales(temp.path());
    let before = hash_tree(&project);
    for visual_type in [
        "map",
        "hundredPercentStackedBarChart",
        "100percentstackedbar",
    ] {
        for mode in ["--dry-run", "--in-place", "--out-dir"] {
            let output_dir = temp.path().join("refused-output");
            let mut args = vec![
                "report",
                "visuals",
                "add",
                "--project",
                project.to_str().unwrap(),
                "--page",
                "page:ReportSectionOverview",
                "--visual-type",
                visual_type,
                "--title",
                "Evidence gate",
                mode,
            ];
            if mode == "--out-dir" {
                args.push(output_dir.to_str().unwrap());
            }
            args.push("--json");
            let output = run_powerbi(&args);
            assert_ne!(output.code, 0);
            let error: Value = serde_json::from_str(output.stderr.trim()).unwrap();
            assert_eq!(error["error"]["code"], "unsupported_feature", "{error}");
            assert!(
                error["error"]["suggestedCommands"]
                    .as_array()
                    .unwrap()
                    .contains(&Value::from("powerbi-cli report visuals catalog --json"))
            );
            let message = error["error"]["message"].as_str().unwrap();
            assert!(message.contains("Missing Desktop-authored"), "{error}");
            if visual_type == "map" {
                assert!(message.contains("online geocoding"));
            }
            assert!(
                error["error"]["hint"]
                    .as_str()
                    .unwrap()
                    .contains("reference")
            );
            assert_eq!(hash_tree(&project), before);
            assert!(!output_dir.exists());
        }
    }
}
