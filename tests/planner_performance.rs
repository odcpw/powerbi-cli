mod common;

use common::{assert_json_snapshot, run_powerbi, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn profile(root: &Path, count: u64) -> std::path::PathBuf {
    let mut value: Value =
        serde_json::from_str(include_str!("../examples/sales.profile.json")).unwrap();
    value["schema"] = json!("powerbi-cli.dataProfile.v2");
    value["dataValues"] = json!(false);
    for table in value["tables"].as_array_mut().unwrap() {
        table["rowCount"] = json!(count);
        for column in table["columns"].as_array_mut().unwrap() {
            column["distinctCount"] = json!(count);
            column["nullRate"] = json!(0.0);
            column["topValues"] = json!([]);
            column.as_object_mut().unwrap().remove("sampleValues");
        }
    }
    let path = root.join("profile.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    path
}

fn plan(profile: &Path, extra: &[&str]) -> Value {
    let mut args = vec![
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        profile.to_str().unwrap(),
        "--json",
    ];
    if !extra.contains(&"--intent") {
        args.extend(["--objective", "Executive overview"]);
    }
    args.extend_from_slice(extra);
    let first = run_powerbi(&args);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let second = run_powerbi(&args);
    assert_eq!(first.stdout, second.stdout, "deterministic plan");
    stdout_json(&first)
}

#[test]
fn planner_guards_only_above_threshold_and_records_numeric_evidence() {
    let temp = tempfile::tempdir().unwrap();
    for count in [199, 200] {
        let value = plan(&profile(temp.path(), count), &[]);
        assert_eq!(value["performance"]["ops"]["ops"], json!([]));
    }
    let value = plan(&profile(temp.path(), 201), &[]);
    let ops = value["performance"]["ops"]["ops"].as_array().unwrap();
    assert!(!ops.is_empty());
    for op in ops {
        assert_eq!(op["op"], "setTopNGuard");
        assert_eq!(op["top"], 50);
        assert!(op["orderBy"].is_string());
    }
    let decisions: Vec<_> = value["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "performance-guard")
        .cloned()
        .collect();
    assert_eq!(decisions.len(), ops.len());
    assert!(
        decisions
            .iter()
            .all(|item| item["distinctCount"] == 201 && item["threshold"] == 200)
    );
    let guards: Vec<_> = value["specV2"]["pages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|page| page["visuals"].as_array().unwrap())
        .filter_map(|visual| visual.get("topnGuard"))
        .cloned()
        .collect();
    assert_eq!(guards.len(), ops.len());
    assert_eq!(
        value["performance"]["kernelAvailable"],
        json!(powerbi_cli::test_support::registered_kernel_tags().contains(&"setTopNGuard"))
    );
    let mut performance = value["performance"].clone();
    // Kernel registration is an additive integration capability, not a golden constant.
    performance
        .as_object_mut()
        .unwrap()
        .remove("kernelAvailable");
    assert_json_snapshot(
        "planner-performance-high",
        &json!({
            "performance": performance, "decisions": decisions, "guards": guards
        }),
    );
}

#[test]
fn intent_guard_overrides_apply_and_invalid_values_refuse_before_output() {
    let temp = tempfile::tempdir().unwrap();
    let profile = profile(temp.path(), 201);
    let intent = temp.path().join("intent.json");
    fs::write(&intent, r#"{"guards":{"threshold":300,"top":7}}"#).unwrap();
    let value = plan(&profile, &["--intent", intent.to_str().unwrap()]);
    assert_eq!(value["performance"]["ops"]["ops"], json!([]));
    fs::write(&intent, r#"{"guards":{"threshold":100,"top":7}}"#).unwrap();
    let value = plan(&profile, &["--intent", intent.to_str().unwrap()]);
    assert!(
        value["performance"]["ops"]["ops"]
            .as_array()
            .unwrap()
            .iter()
            .all(|op| op["top"] == 7)
    );
    let out = temp.path().join("must-not-exist.json");
    for guards in [
        json!({"top":0}),
        json!({"threshold":-1}),
        json!({"top":2.5}),
        json!({"unexpected":1}),
    ] {
        fs::write(
            &intent,
            serde_json::to_vec(&json!({"guards":guards})).unwrap(),
        )
        .unwrap();
        let run = run_powerbi(&[
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
        assert_eq!(run.code, 2);
        let error = stderr_json(&run);
        assert_eq!(error["error"]["code"], "invalid_args");
        assert!(error["error"]["hint"].is_string());
        assert!(
            !error["error"]["suggestedCommands"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!out.exists());
    }
}

#[test]
fn proposed_guard_payload_writes_real_pbir_in_every_mutation_mode() {
    let temp = tempfile::tempdir().unwrap();
    let value = plan(&profile(temp.path(), 201), &[]);
    let op = &value["performance"]["ops"]["ops"][0];
    let mut page = value["specV2"]["pages"][1].clone();
    for key in ["template", "heading", "subtitle"] {
        page.as_object_mut().unwrap().remove(key);
    }
    for visual in page["visuals"].as_array_mut().unwrap() {
        for key in ["slot", "topnGuard"] {
            visual.as_object_mut().unwrap().remove(key);
        }
    }
    let spec = json!({"schema":"powerbi-cli.dashboard.v1",
        "report": value["spec"]["report"], "pages":[page]});
    let spec_path = temp.path().join("spec.json");
    fs::write(&spec_path, serde_json::to_vec(&spec).unwrap()).unwrap();
    let project = temp.path().join("project");
    let run = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec_path.to_str().unwrap(),
        "--out-dir",
        project.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let out = temp.path().join("guarded");
    let top = op["top"].to_string();
    let base = [
        "report",
        "visuals",
        "set-topn-guard",
        "--project",
        project.to_str().unwrap(),
        "--handle",
        op["handle"].as_str().unwrap(),
        "--field",
        op["field"].as_str().unwrap(),
        "--order-by",
        op["orderBy"].as_str().unwrap(),
        "--top",
        &top,
        "--json",
    ];
    for mode in [
        vec!["--dry-run"],
        vec!["--out-dir", out.to_str().unwrap()],
        vec!["--in-place"],
    ] {
        let mut args = base.to_vec();
        args.extend(mode);
        let run = run_powerbi(&args);
        assert_eq!(run.code, 0, "{}", run.stderr);
        assert_eq!(stdout_json(&run)["guard"]["top"], 50);
    }
    let handles: Vec<_> = op["handle"].as_str().unwrap().split(':').collect();
    let relative = format!(
        "SalesOperations.Report/definition/pages/{}/visuals/{}/visual.json",
        handles[1], handles[2]
    );
    let bytes = fs::read(project.join(&relative)).unwrap();
    assert_eq!(bytes, fs::read(out.join(&relative)).unwrap());
    let visual: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(visual["filterConfig"]["filters"][0]["type"], "TopN");
}

#[test]
fn buffer_recommendations_reuse_analyzer_without_rewriting_partition() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let run = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        project.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let path = project.join("SalesOperations.SemanticModel/definition/tables/FactSales.tmdl");
    let original = fs::read_to_string(&path).unwrap();
    let start = original.find("        source =").unwrap();
    let profile = profile(temp.path(), 200);
    for buffered in [false, true] {
        let source = if buffered {
            "Table.Buffer(#table(type table [Units = Int64.Type], {}))"
        } else {
            "#table(type table [Units = Int64.Type], {})"
        };
        let text = format!(
            "{}        source =\n            let\n                Source = {source},\n                Left = Table.SelectRows(Source, each [Units] > 0),\n                Right = Table.SelectRows(Source, each [Units] <= 0),\n                Result = Table.Combine({{Left, Right}})\n            in\n                Result\n",
            &original[..start]
        );
        fs::write(&path, &text).unwrap();
        let value = plan(&profile, &["--project", project.to_str().unwrap()]);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        let findings = value["performance"]["findings"].as_array().unwrap();
        assert_eq!(findings.len(), usize::from(!buffered));
        if !buffered {
            assert_eq!(findings[0]["code"], "m.unbuffered_reuse");
            assert_eq!(findings[0]["referenceCount"], 2);
            let decisions: Vec<_> = value["decisions"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["kind"] == "performance-recommendation")
                .cloned()
                .collect();
            assert_json_snapshot("planner-performance-buffer", &json!(decisions));
        }
    }
}
