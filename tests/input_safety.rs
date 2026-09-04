mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::Value;
use std::fs::{self, File};

const MAX_DASHBOARD_SPEC_BYTES: u64 = 8 * 1024 * 1024;

#[test]
fn capabilities_document_every_input_surface_limit() {
    for args in [
        vec!["capabilities", "--json"],
        vec!["capabilities", "--for", "report spec", "--json"],
    ] {
        let output = run_powerbi(&args);
        assert_eq!(output.code, 0, "stderr: {}", output.stderr);
        let value = stdout_json(&output);
        assert_eq!(value["limits"]["errorCode"], "input_safety_violation");
        assert_eq!(
            value["limits"]["schema"]["maxBytes"],
            Value::from(8 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["profile"]["maxBytes"],
            Value::from(8 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["dashboardSpec"]["maxBytes"],
            Value::from(8 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["jsonArtifact"]["maxBytes"],
            Value::from(16 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["projectText"]["maxBytesPerFile"],
            Value::from(16 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["sourceText"]["maxBytes"],
            Value::from(2 * 1024 * 1024)
        );
        assert_eq!(value["limits"]["include"]["maxDepth"], Value::from(8));
        assert_eq!(
            value["limits"]["include"]["maxResolvedFragments"],
            Value::from(200)
        );
        assert_eq!(
            value["limits"]["include"]["maxFragmentBytes"],
            Value::from(8 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["rows"]["maxFileBytes"],
            Value::from(64 * 1024 * 1024)
        );
        assert_eq!(value["limits"]["rows"]["maxRows"], Value::from(100_000));
        assert_eq!(value["limits"]["rows"]["maxColumns"], Value::from(512));
        assert_eq!(
            value["limits"]["intent"]["maxBytes"],
            Value::from(1024 * 1024)
        );
        assert_eq!(
            value["limits"]["intent"]["includeAndExecDirectives"],
            "refused"
        );
        assert_eq!(value["limits"]["images"]["formats"][0], "png");
        assert_eq!(
            value["limits"]["images"]["maxBytes"],
            Value::from(16 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["ops"]["maxBytes"],
            Value::from(8 * 1024 * 1024)
        );
        assert_eq!(value["limits"]["ops"]["unknownOpKinds"], "refused");
        assert_eq!(
            value["limits"]["snapshots"]["maxFiles"],
            Value::from(10_000)
        );
        assert_eq!(
            value["limits"]["snapshots"]["maxTotalBytes"],
            Value::from(512 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["harvestedFragments"]["maxBytes"],
            Value::from(4 * 1024 * 1024)
        );
        assert_eq!(
            value["limits"]["harvestedFragments"]["silentStripping"],
            false
        );
        assert!(
            value["diagnosticCodes"]
                .as_array()
                .expect("diagnostic codes")
                .iter()
                .any(|item| item["code"] == "input_safety_violation" && item["exitCode"] == 10)
        );
    }
}

#[test]
fn over_limit_dashboard_spec_is_refused_uniformly_and_deterministically() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("oversized.dashboard.json");
    let file = File::create(&spec).expect("create spec");
    file.set_len(MAX_DASHBOARD_SPEC_BYTES + 1)
        .expect("size spec");
    let spec_arg = spec.to_str().expect("spec path");
    let args = ["report", "spec", "validate", "--spec", spec_arg, "--json"];

    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 10);
    assert!(first.stdout.is_empty());
    assert_eq!(first.stderr, second.stderr);
    let error = stderr_json(&first);
    assert_eq!(error["error"]["code"], "input_safety_violation");
    assert_eq!(error["error"]["exitCode"], 10);
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("maximum is 8388608 bytes")
    );
    assert!(error["error"]["hint"].is_string());
    assert_eq!(
        error["error"]["suggestedCommands"][0],
        "powerbi-cli --json capabilities"
    );
}

#[test]
fn intent_files_cannot_smuggle_include_or_exec_directives() {
    let temp = tempfile::tempdir().expect("tempdir");
    let intent = temp.path().join("intent.md");
    fs::write(&intent, "Executive overview\n$include secrets.md\n").expect("intent");
    let intent_arg = intent.to_str().expect("intent path");
    let output = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--intent",
        intent_arg,
        "--json",
    ]);
    assert_eq!(output.code, 10);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "input_safety_violation");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("line 2")
    );
}

#[cfg(unix)]
#[test]
fn dashboard_spec_symlink_is_refused_before_parsing() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("target.dashboard.json");
    let link = temp.path().join("linked.dashboard.json");
    fs::write(
        &target,
        r#"{"schema":"powerbi-cli.dashboard.v1","report":{"name":"Safe"},"pages":[]}"#,
    )
    .expect("target spec");
    symlink(&target, &link).expect("spec symlink");
    let link_arg = link.to_str().expect("link path");
    let output = run_powerbi(&["report", "spec", "validate", "--spec", link_arg, "--json"]);
    assert_eq!(output.code, 10);
    assert_eq!(
        stderr_json(&output)["error"]["code"],
        "input_safety_violation"
    );
}
