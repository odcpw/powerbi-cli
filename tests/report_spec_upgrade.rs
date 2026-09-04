mod common;

use common::{run_powerbi_owned, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const V1: &str = "powerbi-cli.dashboard.v1";
const V2: &str = "powerbi-cli.dashboard.v2";

#[test]
fn every_v1_example_upgrades_and_builds_byte_identically() {
    let fixtures = v1_examples();
    assert!(
        !fixtures.is_empty(),
        "repository must contain v1 dashboard examples"
    );
    let temp = tempfile::tempdir().expect("tempdir");

    for (index, spec_path) in fixtures.iter().enumerate() {
        let schema_path = schema_for(spec_path);
        assert!(
            schema_path.is_file(),
            "v1 example {} must have a sibling schema {}",
            spec_path.display(),
            schema_path.display()
        );
        let upgraded_path = temp.path().join(format!("upgraded-{index}.dashboard.json"));
        let second_upgraded_path = temp
            .path()
            .join(format!("upgraded-{index}.second.dashboard.json"));
        let upgrade = run_owned(&[
            "report",
            "spec",
            "upgrade",
            "--spec",
            &path_arg(spec_path),
            "--out",
            &path_arg(&upgraded_path),
            "--json",
        ]);
        assert_eq!(
            upgrade.code,
            0,
            "upgrade failed for {}: {}",
            spec_path.display(),
            upgrade.stderr
        );
        let response = stdout_json(&upgrade);
        assert_eq!(response["ok"], true);
        assert_eq!(response["sourceVersion"], V1);
        assert_eq!(response["targetVersion"], V2);
        assert_eq!(response["transformedPointers"], json!(["/schema"]));
        assert_eq!(response["spec"]["schema"], V2);
        let mut expected = serde_json::from_str::<Value>(
            &fs::read_to_string(spec_path).expect("read source spec"),
        )
        .expect("parse source spec");
        expected["schema"] = Value::String(V2.to_string());
        assert_eq!(response["spec"], expected);
        assert!(upgraded_path.is_file());
        let upgraded: Value =
            serde_json::from_str(&fs::read_to_string(&upgraded_path).expect("read upgraded spec"))
                .expect("parse upgraded spec");
        assert_eq!(upgraded["schema"], V2);

        let second_upgrade = run_owned(&[
            "report",
            "spec",
            "upgrade",
            "--spec",
            &path_arg(spec_path),
            "--out",
            &path_arg(&second_upgraded_path),
            "--json",
        ]);
        assert_eq!(second_upgrade.code, 0, "{}", second_upgrade.stderr);
        assert_eq!(
            fs::read(&upgraded_path).expect("first upgraded bytes"),
            fs::read(&second_upgraded_path).expect("second upgraded bytes"),
            "upgrade must be deterministic for {}",
            spec_path.display()
        );

        let v1_project = temp.path().join(format!("v1-project-{index}"));
        let v2_project = temp.path().join(format!("v2-project-{index}"));
        let v1_build = run_owned(&[
            "report",
            "build",
            "--schema",
            &path_arg(&schema_path),
            "--spec",
            &path_arg(spec_path),
            "--out-dir",
            &path_arg(&v1_project),
            "--json",
        ]);
        assert_eq!(
            v1_build.code,
            0,
            "v1 build failed for {}: {}",
            spec_path.display(),
            v1_build.stderr
        );
        let v2_build = run_owned(&[
            "report",
            "build",
            "--schema",
            &path_arg(&schema_path),
            "--spec",
            &path_arg(&upgraded_path),
            "--out-dir",
            &path_arg(&v2_project),
            "--json",
        ]);
        assert_eq!(
            v2_build.code,
            0,
            "upgraded v2 build failed for {}: {}",
            spec_path.display(),
            v2_build.stderr
        );
        assert_eq!(
            read_tree(&v1_project),
            read_tree(&v2_project),
            "v1 and upgraded v2 artifacts differ for {}",
            spec_path.display()
        );
    }
}

#[test]
fn unknown_v1_keys_fail_before_upgrade_output_is_written() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        (
            json!({
                "schema": V1,
                "report": {"name": "UnknownRoot", "colour": "red"},
                "pages": []
            }),
            "/report/colour",
        ),
        (
            json!({
                "schema": V1,
                "report": {"name": "UnknownVisual"},
                "pages": [{"visuals": [{"id": "card", "type": "card", "bindings": [], "colour": "red"}]}]
            }),
            "/pages/0/visuals/0/colour",
        ),
    ];
    for (index, (spec, pointer)) in cases.into_iter().enumerate() {
        let source = temp.path().join(format!("unknown-{index}.dashboard.json"));
        let out = temp.path().join(format!("unknown-{index}.v2.json"));
        fs::write(
            &source,
            serde_json::to_string_pretty(&spec).expect("serialize unknown spec"),
        )
        .expect("write unknown spec");
        let output = run_owned(&[
            "report",
            "spec",
            "upgrade",
            "--spec",
            &path_arg(&source),
            "--out",
            &path_arg(&out),
            "--json",
        ]);
        assert_eq!(output.code, 10, "stdout: {}", output.stdout);
        assert!(output.stdout.trim().is_empty());
        let error = stderr_json(&output);
        assert_eq!(error["error"]["code"], "spec.unknown_field");
        assert_eq!(error["error"]["exitCode"], 10);
        assert_eq!(error["error"]["pointer"], pointer);
        assert!(!out.exists(), "unknown input must not create output");
    }
}

#[test]
fn upgrade_dry_run_is_lossless_and_does_not_write_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("must-not-exist.json");
    let output = run_owned(&[
        "report",
        "spec",
        "upgrade",
        "--spec",
        "examples/sales.dashboard.json",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["dryRun"], true);
    assert_eq!(value["changed"], false);
    assert_eq!(value["spec"]["schema"], V2);
    assert!(value["next"].as_array().expect("next").iter().any(|next| {
        next.as_str()
            .is_some_and(|command| command.contains("--out <v2.json>"))
    }));
    assert!(!out.exists());
}

#[test]
fn upgrade_rejects_v2_input_instead_of_rewriting_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("v2-again.json");
    let output = run_owned(&[
        "report",
        "spec",
        "upgrade",
        "--spec",
        "examples/sales.dashboard.v2.json",
        "--out",
        &path_arg(&out),
        "--json",
    ]);
    assert_eq!(output.code, 2, "stdout: {}", output.stdout);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert_eq!(error["error"]["pointer"], "/schema");
    assert!(!out.exists());
}

#[test]
fn existing_upgrade_output_requires_force() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("existing-v2.json");
    fs::write(&out, "keep this file").expect("write sentinel");
    let output = run_owned(&[
        "report",
        "spec",
        "upgrade",
        "--spec",
        "examples/sales.dashboard.json",
        "--out",
        &path_arg(&out),
        "--json",
    ]);
    assert_eq!(output.code, 2, "stdout: {}", output.stdout);
    assert_eq!(stderr_json(&output)["error"]["code"], "invalid_args");
    assert_eq!(
        fs::read_to_string(&out).expect("read sentinel"),
        "keep this file"
    );

    let forced = run_owned(&[
        "report",
        "spec",
        "upgrade",
        "--spec",
        "examples/sales.dashboard.json",
        "--out",
        &path_arg(&out),
        "--force",
        "--json",
    ]);
    assert_eq!(forced.code, 0, "stderr: {}", forced.stderr);
    let upgraded: Value =
        serde_json::from_str(&fs::read_to_string(&out).expect("read forced output"))
            .expect("parse forced output");
    assert_eq!(upgraded["schema"], V2);
}

fn v1_examples() -> Vec<PathBuf> {
    let mut paths = WalkDir::new("examples")
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let path = entry.into_path();
            let is_dashboard = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".dashboard.json"));
            if !is_dashboard {
                return None;
            }
            let value: Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
            (value["schema"] == V1).then_some(path)
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn schema_for(spec: &Path) -> PathBuf {
    let file_name = spec
        .file_name()
        .and_then(|name| name.to_str())
        .expect("dashboard spec filename");
    let base = file_name
        .strip_suffix(".dashboard.json")
        .expect("dashboard spec suffix");
    spec.with_file_name(format!("{base}.schema.json"))
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("UTF-8 path").to_string()
}

fn run_owned(args: &[&str]) -> common::RunOutput {
    run_powerbi_owned(
        &args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>(),
    )
}

fn read_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.expect("walk artifact tree"))
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let path = entry.into_path();
            let relative = path
                .strip_prefix(root)
                .expect("relative artifact path")
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = fs::read(path).expect("read artifact file");
            (relative, bytes)
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}
