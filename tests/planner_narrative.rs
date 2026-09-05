mod common;

use common::{assert_json_snapshot, assert_tree_equal, run_powerbi, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn write(root: &Path, name: &str, value: &Value) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

#[test]
fn narrative_star_and_flat_plans_build_in_order_with_replicated_rail_and_drillthrough() {
    for kind in ["star", "flat"] {
        let root = tempfile::tempdir().unwrap();
        let fixture: Value = serde_json::from_slice(
            &fs::read(format!("testdata/golden/planner-narrative/{kind}.json")).unwrap(),
        )
        .unwrap();
        let schema = write(root.path(), "schema.json", &fixture["schema"]);
        let profile = write(root.path(), "profile.json", &fixture["profile"]);
        // Isolate narrative compilation from the independently proposed TopN
        // guard surface, whose spec compiler remains on its owning bead.
        let intent = write(
            root.path(),
            "intent.json",
            &json!({"questions":["Narrative flow"],"guards":{"threshold":1000}}),
        );
        let args = [
            "report",
            "plan",
            "--schema",
            schema.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--intent",
            intent.to_str().unwrap(),
            "--json",
        ];
        let first = run_powerbi(&args);
        let second = run_powerbi(&args);
        assert_eq!(first.code, 0, "{}", first.stderr);
        assert_eq!(first.stdout, second.stdout);
        let plan = stdout_json(&first);
        assert_eq!(plan["shape"]["kind"], kind);
        assert_json_snapshot(&format!("planner-narrative-{kind}"), &plan["narrativeFlow"]);
        let pages = plan["specV2"]["pages"].as_array().unwrap();
        assert_eq!(pages[0]["id"], "overview");
        assert_eq!(pages[1]["id"], "trend");
        assert_eq!(pages.last().unwrap()["id"], "drillthrough-detail");
        let target = &plan["narrativeFlow"]["drillthrough"]["target"];
        assert_eq!(pages.last().unwrap()["drillthrough"]["target"], *target);
        let source = &plan["narrativeFlow"]["drillthrough"]["sourcePage"];
        assert!(
            pages.iter().find(|page| page["id"] == *source).unwrap()["visuals"]
                .as_array()
                .unwrap()
                .iter()
                .any(|visual| visual["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|binding| binding["role"] == "Category" && binding["field"] == *target))
        );
        let rail_field = &plan["narrativeFlow"]["rail"]["field"];
        let rails = plan["specV2"]["layout"]["rail"]["slicers"]
            .as_array()
            .unwrap();
        assert_eq!(rails.len(), 1);
        assert_eq!(rails[0]["field"], *rail_field);
        let spec = write(root.path(), "spec.json", &plan["specV2"]);
        let outputs = [root.path().join("first"), root.path().join("second")];
        for output in &outputs {
            let built = run_powerbi(&[
                "report",
                "build",
                "--schema",
                schema.to_str().unwrap(),
                "--spec",
                spec.to_str().unwrap(),
                "--out-dir",
                output.to_str().unwrap(),
                "--json",
            ]);
            assert_eq!(built.code, 0, "{}", built.stderr);
        }
        assert_tree_equal(
            &outputs[0],
            &outputs[1],
            "narrative builds are deterministic",
        );
        let report = fixture["schema"]["name"].as_str().unwrap();
        let metadata: Value = serde_json::from_slice(
            &fs::read(outputs[0].join(format!("{report}.Report/definition/pages/pages.json")))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["activePageName"], "ReportSectionOverview");
        assert_eq!(metadata["pageOrder"].as_array().unwrap().len(), pages.len());
        for (name, expected) in metadata["pageOrder"].as_array().unwrap().iter().zip(pages) {
            let page_root = outputs[0].join(format!(
                "{report}.Report/definition/pages/{}",
                name.as_str().unwrap()
            ));
            let saved: Value =
                serde_json::from_slice(&fs::read(page_root.join("page.json")).unwrap()).unwrap();
            assert_eq!(saved["displayName"], expected["displayName"]);
            let slicer_count = fs::read_dir(page_root.join("visuals"))
                .unwrap()
                .filter(|entry| {
                    let visual: Value = serde_json::from_slice(
                        &fs::read(entry.as_ref().unwrap().path().join("visual.json")).unwrap(),
                    )
                    .unwrap();
                    visual["visual"]["visualType"] == "slicer"
                })
                .count();
            assert_eq!(slicer_count, 1, "every saved page has its rail");
        }
        let drill: Value = serde_json::from_slice(
            &fs::read(outputs[0].join(format!(
                "{report}.Report/definition/pages/ReportSectionDrillthroughDetail/page.json"
            )))
            .unwrap(),
        )
        .unwrap();
        assert!(drill.get("pageBinding").is_some());
    }
}

#[test]
fn no_cardinality_evidence_omits_drillthrough() {
    let root = tempfile::tempdir().unwrap();
    let schema = write(
        root.path(),
        "schema.json",
        &json!({"name":"Minimal","tables":[{"name":"Items","columns":[{"name":"Date","dataType":"date"},{"name":"Amount","dataType":"decimal"},{"name":"Category","dataType":"string"}],"measures":[{"name":"Total","expression":"SUM('Items'[Amount])"}]}]}),
    );
    let result = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        schema.to_str().unwrap(),
        "--objective",
        "Summary",
        "--json",
    ]);
    assert_eq!(result.code, 0, "{}", result.stderr);
    let plan = stdout_json(&result);
    assert!(plan["narrativeFlow"]["drillthrough"].is_null());
    assert!(
        !plan["specV2"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|page| page.get("drillthrough").is_some())
    );
}

#[test]
fn unresolved_requested_rail_refuses_before_writing_the_plan() {
    let root = tempfile::tempdir().unwrap();
    let intent = write(
        root.path(),
        "intent.json",
        &json!({"schema":"powerbi-cli.intent.v1","questions":["Summary"],"filterDimensions":["Missing[Column]"]}),
    );
    let out = root.path().join("plan.json");
    let result = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--intent",
        intent.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_ne!(result.code, 0);
    let error: Value = serde_json::from_str(&result.stderr).unwrap();
    assert_eq!(error["error"]["code"], "spec.missing_input");
    assert_eq!(error["error"]["pointer"], "/filterDimensions/0");
    assert!(error["error"]["hint"].is_string());
    assert!(!out.exists());
}
