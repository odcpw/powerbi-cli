mod common;
use common::{assert_json_snapshot, run_powerbi, stderr_json, stdout_json};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::Path};

fn args(out: &Path) -> Vec<String> {
    [
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--objective",
        "Executive overview with trends and category comparison",
        "--out",
        out.to_str().unwrap(),
        "--variants",
        "3",
        "--json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
fn run(args: &[String]) -> common::CliRun {
    run_powerbi(&args.iter().map(String::as_str).collect::<Vec<_>>())
}

#[test]
fn variants_are_structurally_distinct_score_ordered_deterministic_and_compiled_valid() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("plan.json");
    let mut arguments = args(&out);
    let first = run(&arguments);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let value = stdout_json(&first);
    let variants = value["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 3);
    let mut hashes = BTreeSet::new();
    let mut bytes = Vec::new();
    let mut previous = i64::MAX;
    for (offset, variant) in variants.iter().enumerate() {
        assert_eq!(variant["index"], offset + 1);
        let path = variant["path"].as_str().unwrap();
        assert!(path.ends_with(&format!("plan.json.variant-{}.json", offset + 1)));
        assert!(hashes.insert(variant["structureHash"].as_str().unwrap()));
        let score = variant["score"].as_i64().unwrap();
        assert!(score <= previous);
        previous = score;
        assert!(!variant["decisionDiff"].as_array().unwrap().is_empty());
        let data = fs::read(path).unwrap();
        let spec: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(spec["schema"], "powerbi-cli.dashboard.v2");
        assert_eq!(
            spec["pages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|page| page["id"].clone())
                .collect::<Vec<_>>(),
            value["narrativeFlow"]["pageOrder"]
                .as_array()
                .unwrap()
                .clone()
        );
        assert_eq!(spec["pages"][0]["id"], "overview");
        assert_eq!(spec["layout"]["rail"], value["specV2"]["layout"]["rail"]);
        let validation = run_powerbi(&[
            "report",
            "spec",
            "validate",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            path,
            "--json",
        ]);
        assert_eq!(validation.code, 0, "{}", validation.stderr);
        assert_eq!(stdout_json(&validation)["ok"], true);
        bytes.push(data);
    }
    arguments.push("--force".into());
    let second = run(&arguments);
    assert_eq!(second.code, 0, "{}", second.stderr);
    assert_eq!(first.stdout, second.stdout);
    for (variant, data) in variants.iter().zip(bytes) {
        assert_eq!(fs::read(variant["path"].as_str().unwrap()).unwrap(), data);
    }
    let mut snapshot = value["variants"].clone();
    for item in snapshot.as_array_mut().unwrap() {
        item["path"] = json!("<variant-path>");
        item["validateCommand"] = json!("<compiled-validation-command>");
        item["performance"]["kernelAvailable"] = json!("<registration-dependent>");
    }
    assert_json_snapshot("planner_variants", &snapshot);
}

#[test]
fn variants_refuse_invalid_counts_or_missing_output_before_writes() {
    for count in ["0", "-1", "nine", "9"] {
        let output = run_powerbi(&[
            "report",
            "plan",
            "--variants",
            count,
            "--out",
            "unused-plan.json",
            "--json",
        ]);
        assert_ne!(output.code, 0);
        let error = stderr_json(&output);
        assert_eq!(error["error"]["code"], "invalid_args");
        assert!(error["error"]["hint"].is_string());
    }
    let output = run_powerbi(&["report", "plan", "--variants", "1", "--json"]);
    assert_eq!(stderr_json(&output)["error"]["code"], "invalid_args");
}

#[test]
fn variants_preserve_weak_evidence_refusals_without_creating_or_replacing_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let schema_path = temp.path().join("schema.json");
    let out = temp.path().join("plan.json");
    for weak_signal in ["date", "measure", "fact", "intent"] {
        let mut schema = json!({"name":"Evidence","tables":[{"name":"Events","columns":[
            {"name":"Date","dataType":"date"}, {"name":"Amount","dataType":"decimal"}, {"name":"Category","dataType":"string"}
        ],"measures":[{"name":"Total","expression":"SUM('Events'[Amount])"}]}]});
        match weak_signal {
            "date" => schema["tables"][0]["columns"][0]["dataType"] = json!("string"),
            "measure" => schema["tables"][0]["measures"] = json!([]),
            "fact" => {
                let mut other = schema["tables"][0].clone();
                other["name"] = json!("OtherEvents");
                schema["tables"].as_array_mut().unwrap().push(other);
            }
            _ => {}
        }
        fs::write(&schema_path, serde_json::to_vec(&schema).unwrap()).unwrap();
        let mut arguments = vec![
            "report",
            "plan",
            "--schema",
            schema_path.to_str().unwrap(),
            "--variants",
            "3",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ];
        if weak_signal != "intent" {
            arguments.extend(["--objective", "Overview"]);
        }
        for force in [false, true] {
            if force {
                fs::write(&out, b"preserve").unwrap();
                arguments.push("--force");
            }
            let first = run_powerbi(&arguments);
            let second = run_powerbi(&arguments);
            assert_eq!(first.code, 10, "{}", first.stderr);
            assert_eq!(first.stderr, second.stderr);
            let error = stderr_json(&first);
            assert_eq!(error["error"]["code"], "plan.missing_input");
            assert!(error["error"]["hint"].is_string());
            assert!(
                error["error"]["suggestedCommands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|command| command.as_str().unwrap().starts_with("powerbi-cli "))
            );
            assert!(!temp.path().join("plan.json.variant-1.json").exists());
            if force {
                assert_eq!(fs::read(&out).unwrap(), b"preserve");
                fs::remove_file(&out).unwrap();
            } else {
                assert!(!out.exists());
            }
        }
    }
}

#[test]
fn variants_preflight_all_collisions_and_force_only_regular_files() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("plan.json");
    let collision = temp.path().join("plan.json.variant-2.json");
    fs::write(&collision, b"preserve").unwrap();
    let mut arguments = args(&out);
    let result = run(&arguments);
    assert_eq!(stderr_json(&result)["error"]["code"], "invalid_args");
    assert!(!out.exists());
    assert!(!temp.path().join("plan.json.variant-1.json").exists());
    assert_eq!(fs::read(&collision).unwrap(), b"preserve");
    arguments.push("--force".into());
    let result = run(&arguments);
    assert_eq!(result.code, 0, "{}", result.stderr);
}

#[test]
fn variants_keep_high_cardinality_guard_proposals_separate_from_validated_specs() {
    let temp = tempfile::tempdir().unwrap();
    let mut profile: serde_json::Value =
        serde_json::from_str(include_str!("../examples/sales.profile.json")).unwrap();
    profile["schema"] = json!("powerbi-cli.dataProfile.v2");
    profile["dataValues"] = json!(false);
    for table in profile["tables"].as_array_mut().unwrap() {
        table["rowCount"] = json!(201);
        for column in table["columns"].as_array_mut().unwrap() {
            column["distinctCount"] = json!(201);
            column["nullRate"] = json!(0.0);
            column["topValues"] = json!([]);
            column.as_object_mut().unwrap().remove("sampleValues");
        }
    }
    let profile_path = temp.path().join("profile.json");
    fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let mut arguments = args(&temp.path().join("plan.json"));
    let position = arguments.iter().position(|arg| arg == "--profile").unwrap();
    arguments[position + 1] = profile_path.to_str().unwrap().into();
    let output = run(&arguments);
    assert_eq!(output.code, 0, "{}", output.stderr);
    for variant in stdout_json(&output)["variants"].as_array().unwrap() {
        assert!(
            !variant["performance"]["ops"]["ops"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!variant["guardDecisions"].as_array().unwrap().is_empty());
        assert_eq!(
            variant["performance"]["target"],
            format!("variant:{}", variant["index"])
        );
        let spec: serde_json::Value =
            serde_json::from_slice(&fs::read(variant["path"].as_str().unwrap()).unwrap()).unwrap();
        assert!(
            spec["pages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|page| page["visuals"].as_array().unwrap())
                .all(|visual| visual.get("topnGuard").is_none())
        );
    }
}

#[cfg(unix)]
#[test]
fn variants_refuse_symlink_even_with_force_without_touching_target() {
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("plan.json");
    let target = temp.path().join("target.json");
    fs::write(&target, b"preserve").unwrap();
    std::os::unix::fs::symlink(&target, temp.path().join("plan.json.variant-1.json")).unwrap();
    let mut arguments = args(&out);
    arguments.push("--force".into());
    let result = run(&arguments);
    assert_eq!(stderr_json(&result)["error"]["code"], "invalid_args");
    assert!(!out.exists());
    assert_eq!(fs::read(&target).unwrap(), b"preserve");
}

#[test]
fn variants_preserve_narrative_star_and_flat_drillthrough_and_shared_rails() {
    for kind in ["star", "flat"] {
        let root = tempfile::tempdir().unwrap();
        let fixture: serde_json::Value = serde_json::from_slice(
            &fs::read(format!("testdata/golden/planner-narrative/{kind}.json")).unwrap(),
        )
        .unwrap();
        let schema = root.path().join("schema.json");
        let profile = root.path().join("profile.json");
        let out = root.path().join("plan.json");
        fs::write(&schema, serde_json::to_vec(&fixture["schema"]).unwrap()).unwrap();
        fs::write(&profile, serde_json::to_vec(&fixture["profile"]).unwrap()).unwrap();
        let output = run_powerbi(&[
            "report",
            "plan",
            "--schema",
            schema.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--objective",
            "Narrative flow",
            "--variants",
            "3",
            "--out",
            out.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(output.code, 0, "{}", output.stderr);
        let plan = stdout_json(&output);
        let primary = plan["specV2"]["pages"].as_array().unwrap();
        assert_eq!(primary.last().unwrap()["id"], "drillthrough-detail");
        for variant in plan["variants"].as_array().unwrap() {
            let path = variant["path"].as_str().unwrap();
            let spec: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            let pages = spec["pages"].as_array().unwrap();
            assert_eq!(pages.len(), primary.len());
            for (page, expected) in pages.iter().zip(primary) {
                assert_eq!(page["id"], expected["id"]);
                assert_eq!(page["drillthrough"], expected["drillthrough"]);
                assert_eq!(
                    page["visuals"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|visual| &visual["bindings"])
                        .collect::<Vec<_>>(),
                    expected["visuals"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|visual| &visual["bindings"])
                        .collect::<Vec<_>>()
                );
            }
            assert_eq!(spec["layout"]["rail"], plan["specV2"]["layout"]["rail"]);
            let validation = run_powerbi(&[
                "report",
                "spec",
                "validate",
                "--schema",
                schema.to_str().unwrap(),
                "--spec",
                path,
                "--json",
            ]);
            assert_eq!(validation.code, 0, "{}", validation.stderr);
            assert_eq!(stdout_json(&validation)["ok"], true);
        }
    }
}
