mod common;

use common::{run_powerbi, scaffold_sales, stdout_json};
use serde_json::Value;
use std::fs;

#[test]
fn triage_is_ok_and_byte_deterministic_on_clean_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let first = run_powerbi(&["triage", project_arg, "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let value = stdout_json(&first);
    assert_eq!(value["ok"], Value::Bool(true));
    assert_eq!(value["schema"], Value::from("triageResult.v1"));
    assert_eq!(value["validation"]["ok"], Value::Bool(true));
    assert!(value["lint"]["counts"]["findings"].is_number());
    let next = value["next"].as_array().expect("next commands");
    assert!(!next.is_empty());
    assert!(
        next.iter()
            .all(|cmd| cmd.as_str().is_some_and(|c| c.starts_with("powerbi-cli ")))
    );

    let second = run_powerbi(&["triage", project_arg, "--json"]);
    assert_eq!(first.stdout, second.stdout, "triage output must be byte-deterministic");
}

#[test]
fn triage_reports_validation_failure_with_exit_ten() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let pages = project
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join("pages.json");
    fs::write(&pages, "{ not json").expect("corrupt pages.json");

    let output = run_powerbi(&["triage", project.to_str().expect("project path"), "--json"]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["ok"], Value::Bool(false));
    assert!(!value["next"].as_array().expect("next").is_empty());
}

#[test]
fn guid_returns_requested_count_of_v4_guids() {
    let output = run_powerbi(&["guid", "--count", "5", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["count"], Value::from(5));
    let guids = value["guids"].as_array().expect("guids");
    assert_eq!(guids.len(), 5);
    let mut seen = std::collections::BTreeSet::new();
    for guid in guids {
        let guid = guid.as_str().expect("guid string");
        assert_eq!(guid.len(), 36);
        assert_eq!(guid, guid.to_lowercase());
        assert_eq!(guid.as_bytes()[14], b'4', "v4 version nibble: {guid}");
        assert!(seen.insert(guid.to_string()), "guids must be unique");
    }
}

#[test]
fn guid_rejects_out_of_range_counts() {
    for count in ["0", "101"] {
        let output = run_powerbi(&["guid", "--count", count, "--json"]);
        assert_eq!(output.code, 2, "count {count} must be rejected");
        assert!(output.stdout.trim().is_empty());
    }
}

#[test]
fn version_carries_build_identity() {
    let output = run_powerbi(&["version", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let sha = value["gitSha"].as_str().expect("gitSha");
    assert!(
        sha == "unknown" || (sha.len() == 12 && sha.chars().all(|c| c.is_ascii_hexdigit())),
        "gitSha shape: {sha}"
    );
    assert!(value["buildEpoch"].as_u64().is_some_and(|epoch| epoch > 0));
}
