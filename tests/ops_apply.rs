mod common;

use common::{
    assert_tree_equal, first_page_name, first_visual_json, run_powerbi, scaffold_sales, stdout_json,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn handle(project: &Path) -> String {
    let visual: Value =
        serde_json::from_slice(&fs::read(first_visual_json(project)).unwrap()).unwrap();
    format!(
        "visual:{}:{}",
        first_page_name(project),
        visual["name"].as_str().unwrap()
    )
}

fn plan(path: &Path, operations: Value) {
    fs::write(
        path,
        serde_json::to_vec(&json!({"schema":"powerbi-cli.ops.v1", "ops":operations})).unwrap(),
    )
    .unwrap();
}

fn apply(source: &Path, plan: &Path, mode: &[&str]) -> common::CliRun {
    let mut args = vec![
        "ops",
        "apply",
        "--project",
        source.to_str().unwrap(),
        "--ops",
        plan.to_str().unwrap(),
        "--json",
    ];
    args.extend_from_slice(mode);
    run_powerbi(&args)
}

#[test]
fn public_replay_matches_sequential_cli_artifacts_and_is_deterministic_in_all_modes() {
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(&temp.path().join("source with spaces"));
    let sequential = scaffold_sales(&temp.path().join("sequential"));
    let visual = handle(&source);
    let ops = temp.path().join("ops.json");
    plan(
        &ops,
        json!([
            {"op":"setPosition", "visual":visual, "x":120.0},
            {"op":"setObject", "visual":visual, "object":"categoryLabels", "property":"fontSize", "value":{"expr":{"Literal":{"Value":"20D"}}}}
        ]),
    );
    let preview = apply(&source, &ops, &["--dry-run"]);
    assert_eq!(preview.exit, 0, "{}", preview.stderr);
    assert_eq!(preview.stdout, apply(&source, &ops, &["--dry-run"]).stdout);
    assert_tree_equal(&source, &sequential, "dry-run leaves source untouched");
    let value = stdout_json(&preview);
    assert_eq!(value["schema"], "powerbi-cli.ops.apply.v1");
    assert_eq!(value["operationCount"], 2);
    assert_eq!(value["scorecard"]["schema"], "scorecard.v1");
    assert!(value["readback"][&visual].is_array());
    assert!(
        !preview.stdout.contains("powerbi-cli-ops-"),
        "staging path leaked: {}",
        preview.stdout
    );
    for (command, flags) in [
        ("set-position", vec!["--x", "120"]),
        (
            "set-object",
            vec![
                "--object",
                "categoryLabels",
                "--property",
                "fontSize",
                "--value",
                "20",
            ],
        ),
    ] {
        let mut args = vec![
            "report",
            "visuals",
            command,
            "--project",
            sequential.to_str().unwrap(),
            "--handle",
            &visual,
            "--in-place",
            "--json",
        ];
        args.extend(flags);
        let result = run_powerbi(&args);
        assert_eq!(result.exit, 0, "{}", result.stderr);
    }
    for name in ["out one", "out two"] {
        let out = temp.path().join(name);
        let result = apply(&source, &ops, &["--out-dir", out.to_str().unwrap()]);
        assert_eq!(result.exit, 0, "{}", result.stderr);
        assert_tree_equal(&out, &sequential, "N operations equals N CLI commands");
    }
    let result = apply(&source, &ops, &["--in-place"]);
    assert_eq!(result.exit, 0, "{}", result.stderr);
    assert!(Path::new(stdout_json(&result)["snapshotDir"].as_str().unwrap()).is_dir());
    assert_tree_equal(&source, &sequential, "in-place parity");
}

#[test]
fn failure_at_second_operation_never_publishes_any_mode() {
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(&temp.path().join("source"));
    let baseline = scaffold_sales(&temp.path().join("baseline"));
    let visual = handle(&source);
    let ops = temp.path().join("ops.json");
    plan(
        &ops,
        json!([
            {"op":"setPosition", "visual":visual, "x":120.0},
            {"op":"setObject", "visual":visual, "object":"unproven", "property":"bogus", "value":true}
        ]),
    );
    let out = temp.path().join("unpublished");
    for mode in [
        vec!["--dry-run"],
        vec!["--in-place"],
        vec!["--out-dir", out.to_str().unwrap()],
    ] {
        let result = apply(&source, &ops, &mode);
        assert_ne!(result.exit, 0);
        let error: Value = serde_json::from_str(&result.stderr).unwrap();
        assert_eq!(error["error"]["code"], "unsupported_feature");
        assert!(
            error["error"]["pointer"]
                .as_str()
                .unwrap()
                .starts_with("/ops/1")
        );
        assert_tree_equal(&source, &baseline, "failure atomicity");
        assert!(!out.exists());
    }
}

#[test]
fn malformed_plans_refuse_with_stable_codes_and_pointers_before_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(temp.path());
    let visual = handle(&source);
    let ops = temp.path().join("ops.json");
    let out = temp.path().join("never-created");
    for (operations, code, pointer) in [
        (
            json!([{"op":"rawPatch"}]),
            "unsupported_feature",
            "/ops/0/op",
        ),
        (
            json!([{"op":"setPosition", "visual":visual, "x":1, "out-dir":"evil"}]),
            "ops.invalid_plan",
            "/ops/0/out-dir",
        ),
        (
            json!([{"op":"setPosition", "kind":"setObject", "visual":visual}]),
            "ops.invalid_plan",
            "/ops/0/kind",
        ),
        (
            json!([{"op":"setPosition", "visual":visual, "x":1, "typo":true}]),
            "ops.invalid_plan",
            "/ops/0/typo",
        ),
        (
            json!([{"op":"setPosition", "visual":"visual:absent:absent", "x":1}]),
            "ops.dangling_handle",
            "/ops/0/visual",
        ),
    ] {
        plan(&ops, operations);
        let result = apply(&source, &ops, &["--out-dir", out.to_str().unwrap()]);
        assert_ne!(result.exit, 0);
        let error: Value = serde_json::from_str(&result.stderr).unwrap();
        assert_eq!(error["error"]["code"], code, "{error}");
        assert_eq!(error["error"]["pointer"], pointer, "{error}");
        assert!(!out.exists());
    }
}

#[test]
fn replay_refuses_unproven_object_encodings_and_transport_controls() {
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(temp.path());
    let visual = handle(&source);
    let ops = temp.path().join("ops.json");
    for value in [
        json!(20),
        json!({"expr":{"Literal":{"Value":"NaND"}}}),
        json!({"expr":{"Literal":{"Value":"true"}}}),
        json!({"expr":{"Literal":{"Value":"20D"}},"extra":true}),
    ] {
        plan(
            &ops,
            json!([{"op":"setObject", "visual":visual, "object":"categoryLabels", "property":"fontSize", "value":value}]),
        );
        let result = apply(&source, &ops, &["--dry-run"]);
        assert_ne!(result.exit, 0);
        let error: Value = serde_json::from_str(&result.stderr).unwrap();
        assert_eq!(error["error"]["code"], "unsupported_feature");
        assert_eq!(error["error"]["pointer"], "/ops/0/value");
    }
    for key in [
        "project",
        "outDir",
        "dryRun",
        "inPlace",
        "_",
        "snapshotDir",
        "out-dir",
    ] {
        let mut operation = json!({"op":"updatePage", "page":"page:Main"});
        operation[key] = json!("external");
        plan(&ops, json!([operation]));
        let result = apply(&source, &ops, &["--dry-run"]);
        let error: Value = serde_json::from_str(&result.stderr).unwrap();
        assert_eq!(error["error"]["code"], "ops.invalid_plan", "{error}");
        assert_eq!(error["error"]["pointer"], format!("/ops/0/{key}"));
    }
}

#[test]
fn replay_accepts_catalog_boolean_and_escaped_string_literals() {
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(temp.path());
    let visual = handle(&source);
    let ops = temp.path().join("ops.json");
    plan(
        &ops,
        json!([
            {"op":"setObject", "visual":visual, "object":"categoryLabels", "property":"show", "value":{"expr":{"Literal":{"Value":"false"}}}},
            {"op":"setObject", "visual":visual, "object":"title", "property":"text", "value":{"expr":{"Literal":{"Value":"'Revenue''s title'"}}}}
        ]),
    );
    let result = apply(&source, &ops, &["--dry-run"]);
    assert_eq!(result.exit, 0, "{}", result.stderr);
    assert_eq!(stdout_json(&result)["operationCount"], 2);
}

#[test]
fn replay_requires_exactly_one_mode_and_refuses_existing_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(temp.path());
    let ops = temp.path().join("ops.json");
    plan(&ops, json!([]));
    for mode in [
        vec![],
        vec!["--dry-run", "--in-place"],
        vec!["--out-dir", source.to_str().unwrap()],
    ] {
        let result = apply(&source, &ops, &mode);
        assert_ne!(result.exit, 0);
        let error: Value = serde_json::from_str(&result.stderr).unwrap();
        assert_eq!(error["error"]["code"], "invalid_args", "{error}");
    }
    let result = apply(&source, &ops, &["--dry-run"]);
    assert_eq!(result.exit, 0, "{}", result.stderr);
    assert_eq!(stdout_json(&result)["changed"], false);
}

#[test]
fn public_replay_discovery_is_bounded_and_plan_order_is_validated() {
    for args in [
        vec!["capabilities", "--for", "ops apply", "--json"],
        vec!["capabilities", "--for", "ops apply", "--compact", "--json"],
    ] {
        let result = run_powerbi(&args);
        assert_eq!(result.exit, 0, "{}", result.stderr);
        assert!(result.stdout.len() < 20_000);
        assert!(result.stdout.contains("ops apply"));
    }
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(temp.path());
    let visual = handle(&source);
    let ops = temp.path().join("ops.json");
    let position = json!({"op":"setPosition", "visual":visual, "x":120});
    let object = json!({"op":"setObject", "visual":visual, "object":"categoryLabels", "property":"fontSize", "value":{"expr":{"Literal":{"Value":"20D"}}}});
    for (operations, code) in [
        (json!([position, position]), "ops.duplicate_operation"),
        (json!([object, position]), "ops.stage_order"),
    ] {
        plan(&ops, operations);
        let result = apply(&source, &ops, &["--dry-run"]);
        assert_ne!(result.exit, 0);
        let error: Value = serde_json::from_str(&result.stderr).unwrap();
        assert_eq!(error["error"]["code"], code, "{error}");
        assert!(
            error["error"]["pointer"]
                .as_str()
                .unwrap()
                .starts_with("/ops/1")
        );
    }
}

#[test]
fn mixed_model_and_legacy_page_plan_matches_individual_commands() {
    let temp = tempfile::tempdir().unwrap();
    let source = scaffold_sales(&temp.path().join("source"));
    let sequential = scaffold_sales(&temp.path().join("sequential"));
    let page = format!("page:{}", first_page_name(&source));
    let ops = temp.path().join("ops.json");
    plan(
        &ops,
        json!([
            {"op":"addMeasure", "handle":"measure:FactSales:Replay Revenue", "table":"FactSales", "name":"Replay Revenue", "expression":"SUM('FactSales'[Revenue])"},
            {"op":"updatePage", "handle":page, "displayName":"Replay Page"}
        ]),
    );
    for command in [
        vec![
            "model",
            "measures",
            "add",
            "--table",
            "FactSales",
            "--name",
            "Replay Revenue",
            "--expression",
            "SUM('FactSales'[Revenue])",
        ],
        vec![
            "report",
            "pages",
            "update",
            "--handle",
            &page,
            "--display-name",
            "Replay Page",
        ],
    ] {
        let mut args = command;
        args.extend([
            "--project",
            sequential.to_str().unwrap(),
            "--in-place",
            "--json",
        ]);
        let result = run_powerbi(&args);
        assert_eq!(result.exit, 0, "{}", result.stderr);
    }
    let out = temp.path().join("out");
    let result = apply(&source, &ops, &["--out-dir", out.to_str().unwrap()]);
    assert_eq!(result.exit, 0, "{}", result.stderr);
    assert_tree_equal(&out, &sequential, "mixed kernel plan equivalence");
    let result = stdout_json(&result);
    assert!(result["readback"]["measure:FactSales:Replay Revenue"].is_array());
    assert!(result["readback"][&page].is_array());
}
