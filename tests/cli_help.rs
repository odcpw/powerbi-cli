//! Catalog-rendered help at every dispatch level, plus did-you-mean recovery.

mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::Value;

#[test]
fn top_level_help_exits_zero_on_stdout() {
    for args in [&["--help"][..], &["-h"], &["help"]] {
        let output = run_powerbi(args);
        assert_eq!(output.code, 0, "{args:?} stderr: {}", output.stderr);
        assert!(
            output.stderr.trim().is_empty(),
            "{args:?} leaked diagnostics: {}",
            output.stderr
        );
        assert!(
            output.stdout.contains("powerbi-cli helps agents"),
            "{args:?} stdout: {}",
            output.stdout
        );
    }

    let json_help = run_powerbi(&["--help", "--json"]);
    assert_eq!(json_help.code, 0, "stderr: {}", json_help.stderr);
    let value = stdout_json(&json_help);
    assert_eq!(value["tool"], Value::from("powerbi-cli"));
    assert!(
        value["commands"]
            .as_array()
            .is_some_and(|commands| !commands.is_empty())
    );
}

#[test]
fn family_help_lists_report_pages_clone() {
    let output = run_powerbi(&["report", "pages", "--help"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(output.stderr.trim().is_empty(), "stderr: {}", output.stderr);
    assert!(
        output.stdout.contains("report pages clone"),
        "family help missing clone: {}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .contains("powerbi-cli --json capabilities --for \"report pages\""),
        "family help missing capabilities footer: {}",
        output.stdout
    );
}

#[test]
fn leaf_help_contains_catalog_usage() {
    let capabilities = run_powerbi(&["--json", "capabilities", "--for", "report pages list"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let usage = catalog_usage(&stdout_json(&capabilities), "report pages list");

    let output = run_powerbi(&["report", "pages", "list", "--help"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(output.stderr.trim().is_empty(), "stderr: {}", output.stderr);
    assert!(
        output.stdout.contains(&usage),
        "leaf help missing catalog usage {usage:?}: {}",
        output.stdout
    );
}

#[test]
fn help_prefix_matches_suffix_help() {
    let prefix = run_powerbi(&["help", "report", "pages"]);
    let suffix = run_powerbi(&["report", "pages", "--help"]);
    assert_eq!(prefix.code, 0, "prefix stderr: {}", prefix.stderr);
    assert_eq!(suffix.code, 0, "suffix stderr: {}", suffix.stderr);
    assert_eq!(prefix.stdout, suffix.stdout);
}

#[test]
fn help_json_returns_catalog_usage() {
    let capabilities = run_powerbi(&["--json", "capabilities", "--for", "report pages list"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let usage = catalog_usage(&stdout_json(&capabilities), "report pages list");

    let output = run_powerbi(&["report", "pages", "list", "--help", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert!(output.stderr.trim().is_empty(), "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["help"]["usage"], Value::from(usage));
}

#[test]
fn family_help_json_includes_child_usage() {
    let output = run_powerbi(&["report", "pages", "--help", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["help"]["path"], Value::from("report pages"));
    let commands = value["help"]["commands"]
        .as_array()
        .expect("family commands");
    assert!(
        commands.iter().any(|command| {
            command["path"] == "report pages clone"
                && command["usage"]
                    .as_str()
                    .is_some_and(|usage| usage.contains("report pages clone"))
        }),
        "family JSON missing clone usage: {value}"
    );
}

#[test]
fn unknown_subcommand_did_you_mean_list() {
    let output = run_powerbi(&["report", "pages", "lst", "--json"]);
    assert_eq!(output.code, 2);
    assert!(output.stdout.trim().is_empty(), "stdout: {}", output.stdout);
    let value = stderr_json(&output);
    assert_eq!(value["error"]["code"], Value::from("invalid_args"));
    assert_eq!(value["error"]["exitCode"], Value::from(2));
    let hint = value["error"]["hint"].as_str().unwrap_or_default();
    assert!(
        hint.starts_with("Did you mean `list`?"),
        "hint should start with did-you-mean: {hint}"
    );
    assert!(
        hint.contains("capabilities --for \"report pages\""),
        "existing redirect should remain: {hint}"
    );
}

#[test]
fn unknown_flag_did_you_mean_strict() {
    let output = run_powerbi(&["validate", "--strick", "--json"]);
    assert_eq!(output.code, 2);
    assert!(output.stdout.trim().is_empty(), "stdout: {}", output.stdout);
    let value = stderr_json(&output);
    assert_eq!(value["error"]["code"], Value::from("invalid_args"));
    let hint = value["error"]["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("Did you mean `--strict`?"),
        "hint should name --strict: {hint}"
    );
}

#[test]
fn help_stays_on_stdout_and_errors_stay_on_stderr() {
    let help = run_powerbi(&["report", "pages", "list", "--help"]);
    assert_eq!(help.code, 0, "stderr: {}", help.stderr);
    assert!(!help.stdout.trim().is_empty());
    assert!(help.stderr.trim().is_empty());

    let error = run_powerbi(&["report", "pages", "lst", "--json"]);
    assert_eq!(error.code, 2);
    assert!(error.stdout.trim().is_empty());
    let value = stderr_json(&error);
    assert!(value["error"]["code"].is_string());
    assert!(value["error"]["message"].is_string());
}

fn catalog_usage(capabilities: &Value, path: &str) -> String {
    capabilities["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == path)
        .and_then(|command| command["usage"].as_str())
        .unwrap_or_else(|| panic!("missing catalog usage for {path}"))
        .to_string()
}
