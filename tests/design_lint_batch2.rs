mod common;
use common::{archetype_names, load_archetype, patch_json, run_powerbi, stdout_json};
use serde_json::{Value, json};
use std::fs;

#[test]
fn every_archetype_schema_builds_clean_with_slots_and_resolved_tokens() {
    let temp = tempfile::tempdir().unwrap();
    for name in archetype_names() {
        let fixture = load_archetype(name);
        let schema: Value = serde_json::from_slice(&fs::read(&fixture.schema).unwrap()).unwrap();
        let table = schema["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|table| {
                table["measures"]
                    .as_array()
                    .is_some_and(|measures| !measures.is_empty())
            })
            .unwrap();
        let table_name = table["name"].as_str().unwrap();
        let measure = table["measures"][0]["name"].as_str().unwrap();
        let column = table["columns"][0]["name"].as_str().unwrap();
        let spec = json!({"schema":"powerbi-cli.dashboard.v2", "report":{"name":"Clean"},
        "style":{"tokens":{"preset":"corporate-neutral"}},
        "pages":[{"id":"overview", "template":"time-series", "heading":"Overview", "visuals":[{
            "id":"trend", "type":"lineChart", "title":"Trend", "slot":"primary",
            "bindings":[{"role":"Category", "field":format!("{table_name}[{column}]")}, {"role":"Y", "field":format!("{table_name}[{measure}]")}]
        }]}]});
        let spec_path = temp.path().join(format!("{name}.json"));
        fs::write(&spec_path, serde_json::to_vec_pretty(&spec).unwrap()).unwrap();
        let out = temp.path().join(name);
        let built = run_powerbi(&[
            "report",
            "build",
            "--schema",
            fixture.schema.to_str().unwrap(),
            "--spec",
            spec_path.to_str().unwrap(),
            "--out-dir",
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(built.code, 0, "{name}: {}", built.stderr);
        let value = stdout_json(&built);
        let design = &value["scorecard"]["designLint"];
        assert_eq!(design["findings"], json!([]), "{name}: {design}");
        assert_eq!(design["deferredRules"], json!([]), "{name}: {design}");
        assert_eq!(design["evaluatedRules"].as_array().unwrap().len(), 20);
        let args = [
            "report",
            "audit",
            "--project",
            out.to_str().unwrap(),
            "--rules",
            "design",
            "--json",
        ];
        let first = run_powerbi(&args);
        let second = run_powerbi(&args);
        assert_eq!(first.code, 0, "{}", first.stderr);
        assert_eq!(first.stdout, second.stdout);
        assert_eq!(stdout_json(&first)["findings"], json!([]));
        assert_eq!(stdout_json(&first)["deferredRules"], json!([]));

        // Actual persisted property readback, not just synthetic predicate tests.
        let visual_path = out
            .join("Clean.Report/definition/pages/ReportSectionOverview/visuals/VisualContainerTrend/visual.json");
        patch_json(&visual_path, |raw| {
            raw["visual"]["visualContainerObjects"]["title"] = json!([{"properties":{
                "text":{"expr":{"Literal":{"Value":"'Planted title'"}}},
                "fontSize":{"expr":{"Literal":{"Value":"5D"}}}
            }}]);
        });
        let planted = run_powerbi(&args);
        assert!(
            stdout_json(&planted)["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["ruleId"] == "design.font_below_minimum")
        );
        // A familiar preset filename does not establish the policy after edits.
        let themes = out.join("Clean.Report/StaticResources/RegisteredResources");
        let theme = fs::read_dir(themes)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "json"))
            .unwrap();
        patch_json(&theme, |raw| raw["foreground"] = json!("#123456"));
        let audit = stdout_json(&run_powerbi(&args));
        assert_eq!(audit["deferredRules"].as_array().unwrap().len(), 4);
        let triage = run_powerbi(&["triage", out.to_str().unwrap(), "--json"]);
        let triage = stdout_json(&triage);
        let deferred = &triage["scorecard"]["designLint"]["deferredRules"];
        assert_eq!(deferred.as_array().unwrap().len(), 4, "{triage}");
        assert!(deferred.as_array().unwrap().iter().all(|rule| {
            rule["status"] == "not-evaluated"
                && rule["reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty())
        }));
    }
}
