mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;

fn proof_spec(path: &std::path::Path) {
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.v2.json").expect("read v2 dashboard fixture"),
    )
    .expect("parse v2 dashboard fixture");
    spec["proof"] = json!({
        "desktop": {
            "level": "desktop-canvas-refresh",
            "pages": ["overview"],
            "expectValues": [{
                "query": "EVALUATE ROW(\"Value\", 1)",
                "expected": 1
            }]
        },
        "goldens": ["sales"]
    });
    fs::write(
        path,
        serde_json::to_vec_pretty(&spec).expect("serialize proof spec"),
    )
    .expect("write proof spec");
}

#[test]
fn report_build_compiles_proof_plan_and_next_commands_without_running_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("proof.dashboard.v2.json");
    proof_spec(&spec);
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec.to_str().expect("spec path"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["proofPlan"]["requestedLevel"],
        "desktop-canvas-refresh"
    );
    assert_eq!(
        value["proofPlan"]["achievableHere"],
        if cfg!(windows) {
            "desktop-golden-pending"
        } else {
            "schema-golden"
        }
    );
    let commands = value["proofPlan"]["commands"]
        .as_array()
        .expect("proof plan commands");
    assert!(commands.iter().any(|command| {
        command
            .as_str()
            .is_some_and(|command| command.starts_with("powerbi-cli desktop open "))
    }));
    assert!(commands.iter().any(|command| {
        command.as_str().is_some_and(|command| {
            command.contains("model dax execute") && command.contains("EVALUATE")
        })
    }));
    assert!(commands.iter().any(|command| {
        command.as_str().is_some_and(|command| {
            command.contains("fixture verify") && command.contains("sales.summary.json")
        })
    }));
    assert_eq!(
        value["next"].as_array().expect("next").len(),
        1 + commands.len()
    );
    assert!(
        value["executedPrimitives"]
            .as_array()
            .expect("executed")
            .is_empty()
    );
}

#[test]
fn report_build_renders_actual_project_path_in_proof_commands() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("proof.dashboard.v2.json");
    let project = temp.path().join("proof-project");
    proof_spec(&spec);
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec.to_str().expect("spec path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let project_display = value["projectDir"].as_str().expect("projectDir");
    assert!(
        value["proofPlan"]["commands"]
            .as_array()
            .expect("commands")
            .iter()
            .all(|command| {
                let command = command.as_str().expect("command");
                command == "powerbi-cli desktop close --json" || command.contains(project_display)
            })
    );
    assert!(
        project.exists(),
        "proof planning must not prevent the build"
    );
}

#[test]
fn report_build_rejects_invalid_proof_level_with_pointer_and_no_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("invalid-proof.dashboard.v2.json");
    let project = temp.path().join("must-not-exist");
    let mut value: Value = serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.v2.json").expect("read v2 dashboard fixture"),
    )
    .expect("parse v2 dashboard fixture");
    value["proof"] = json!({"desktop": {"level": "not-a-proof-level"}});
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&value).expect("serialize invalid proof"),
    )
    .expect("write invalid proof");
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec.to_str().expect("spec path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 2, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "invalid_args");
    assert_eq!(error["pointer"], "/proof/desktop/level");
    assert!(!project.exists());
}

#[test]
fn report_build_proof_plan_is_byte_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("proof.dashboard.v2.json");
    proof_spec(&spec);
    let spec_path = spec.to_str().expect("spec path");
    let args = [
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec_path,
        "--dry-run",
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn proof_next_commands_match_catalogued_command_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("proof.dashboard.v2.json");
    proof_spec(&spec);
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec.to_str().expect("spec path"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let catalog = stdout_json(&capabilities);
    let paths = catalog["commands"]
        .as_array()
        .expect("catalog commands")
        .iter()
        .filter_map(|command| command["path"].as_str())
        .collect::<Vec<_>>();
    for command in stdout_json(&output)["next"]
        .as_array()
        .expect("next")
        .iter()
        .map(|command| command.as_str().expect("command"))
    {
        let words = command.split_whitespace().collect::<Vec<_>>();
        assert_eq!(words.first().copied(), Some("powerbi-cli"), "{command}");
        assert!(
            paths.iter().any(|path| {
                let path_words = path.split_whitespace().collect::<Vec<_>>();
                words.len() > path_words.len() && words[1..].starts_with(&path_words)
            }),
            "next command is not catalogued: {command}"
        );
    }
}
