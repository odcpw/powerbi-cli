//! Structured native validation diagnostics.

mod common;

use common::{patch_json, report_pages_json, run_powerbi, scaffold_sales, stdout_json};
use serde_json::{Value, json};
use std::fs;

fn assert_finding_shape(finding: &Value) {
    for field in ["code", "message", "path", "pointer", "severity"] {
        assert!(
            finding[field].as_str().is_some(),
            "native validation finding is missing string field {field}: {finding}"
        );
    }
    let pointer = finding["pointer"].as_str().expect("pointer");
    assert!(
        is_rfc6901_pointer(pointer),
        "invalid RFC 6901 pointer: {pointer:?}"
    );
}

fn is_rfc6901_pointer(pointer: &str) -> bool {
    if pointer.is_empty() {
        return true;
    }
    if !pointer.starts_with('/') {
        return false;
    }
    pointer.split('/').skip(1).all(|token| {
        let bytes = token.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'~' {
                if !matches!(bytes.get(index + 1), Some(b'0' | b'1')) {
                    return false;
                }
                index += 2;
            } else {
                index += 1;
            }
        }
        true
    })
}

#[test]
fn native_validation_errors_are_structured_registered_and_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let pages_path = report_pages_json(&project);
    patch_json(&pages_path, |pages| {
        let first = pages["pageOrder"][0].clone();
        pages["pageOrder"] = json!([first.clone(), first]);
    });

    let project_arg = project.to_str().expect("project path");
    let first = run_powerbi(&["validate", project_arg, "--json"]);
    assert_eq!(first.code, 10, "stderr: {}", first.stderr);
    let first_json = stdout_json(&first);
    let errors = first_json["errors"].as_array().expect("errors");
    assert!(!errors.is_empty(), "duplicate page must fail validation");
    for finding in errors {
        assert_finding_shape(finding);
        assert_eq!(finding["severity"], "error");
        let explained = run_powerbi(&[
            "lint",
            "--explain",
            finding["code"].as_str().unwrap(),
            "--json",
        ]);
        assert_eq!(explained.code, 0, "stderr: {}", explained.stderr);
        assert_eq!(stdout_json(&explained)["rule"]["id"], finding["code"]);
    }
    let duplicate = errors
        .iter()
        .find(|finding| {
            finding["message"]
                .as_str()
                .unwrap_or_default()
                .contains("duplicate page")
        })
        .expect("duplicate page diagnostic");
    assert_eq!(duplicate["code"], "validation.page_order");
    assert_eq!(duplicate["pointer"], "/pageOrder/1");
    assert_eq!(duplicate["path"], pages_path.to_string_lossy().as_ref());

    let second = run_powerbi(&["validate", project_arg, "--json"]);
    assert_eq!(second.code, 10, "stderr: {}", second.stderr);
    assert_eq!(stdout_json(&second)["errors"], first_json["errors"]);
}

#[test]
fn missing_files_use_a_valid_root_pointer_and_preserve_the_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let report_json = project
        .join("SalesOperations.Report")
        .join("definition")
        .join("report.json");
    fs::remove_file(&report_json).expect("remove report metadata");
    let output = run_powerbi(&[
        "validate",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let finding = value["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .find(|finding| {
            finding["message"]
                .as_str()
                .unwrap_or_default()
                .contains("missing required file")
        })
        .expect("missing report.json finding");
    assert_finding_shape(finding);
    assert_eq!(finding["code"], "validation.missing_file");
    assert_eq!(finding["pointer"], "");
    assert_eq!(finding["path"], report_json.to_string_lossy().as_ref());
}

#[test]
fn malformed_json_is_reported_without_aborting_native_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let report_json = project
        .join("SalesOperations.Report")
        .join("definition")
        .join("report.json");
    fs::write(&report_json, "{\n").expect("write malformed report metadata");
    let output = run_powerbi(&[
        "validate",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let finding = value["errors"]
        .as_array()
        .expect("errors")
        .iter()
        .find(|finding| finding["path"] == report_json.to_string_lossy().as_ref())
        .expect("malformed report JSON finding");
    assert_finding_shape(finding);
    assert_eq!(finding["code"], "validation.invalid_json");
    assert_eq!(finding["pointer"], "");
}

#[test]
fn validation_codes_are_generated_in_capabilities() {
    let output = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert!(
        value["schemaManifest"]["validationFindingCodes"]
            .as_array()
            .expect("validation finding codes")
            .iter()
            .any(|code| code == "validation.missing_file")
    );
    let validate = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "validate")
        .expect("validate capability");
    assert!(
        validate["diagnosticCodes"]
            .as_array()
            .expect("validation diagnostic codes")
            .iter()
            .any(|code| code == "validation.page_order")
    );
}
