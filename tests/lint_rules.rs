mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::Value;
use std::collections::BTreeSet;

fn string_set(values: &Value, field: Option<&str>) -> BTreeSet<String> {
    values
        .as_array()
        .expect("array")
        .iter()
        .map(|value| {
            field
                .map(|field| value[field].as_str().expect("string field"))
                .unwrap_or_else(|| value.as_str().expect("string value"))
                .to_string()
        })
        .collect()
}

#[test]
fn lint_rules_are_complete_documented_and_generated_into_capabilities() {
    let listed = run_powerbi(&["lint", "--rules", "--json"]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    assert!(listed.stderr.trim().is_empty());
    let listed_json = stdout_json(&listed);
    assert_eq!(listed_json["schema"], "powerbi-cli.lint.rules.v1");
    assert_eq!(listed_json["ok"], true);
    assert_eq!(listed_json["exitCode"], 0);
    assert!(
        listed_json["families"]
            .as_array()
            .expect("families")
            .iter()
            .any(|family| family == "design")
    );
    let rules = listed_json["rules"].as_array().expect("rules");
    assert_eq!(listed_json["count"], rules.len());
    assert_eq!(rules.len(), 77);
    for rule in rules {
        for field in [
            "id",
            "family",
            "severity",
            "summary",
            "remediation",
            "since",
        ] {
            assert!(
                rule[field]
                    .as_str()
                    .is_some_and(|value| !value.trim().is_empty()),
                "rule field {field} must document {rule}"
            );
        }
        assert!(
            rule.get("sanitizeAction").is_some(),
            "missing optional field: {rule}"
        );
    }
    let listed_ids = string_set(&listed_json["rules"], Some("id"));
    assert_eq!(listed_ids.len(), rules.len(), "rule ids must be unique");

    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let capabilities_json = stdout_json(&capabilities);
    assert_eq!(
        string_set(
            &capabilities_json["schemaManifest"]["lintFindingCodes"],
            None
        ),
        listed_ids
    );
    assert_eq!(
        capabilities_json["schemaManifest"]["lintRuleFields"],
        serde_json::json!([
            "id",
            "family",
            "severity",
            "summary",
            "remediation",
            "sanitizeAction",
            "since"
        ])
    );
    let commands = capabilities_json["commands"].as_array().expect("commands");
    for path in ["lint", "report audit"] {
        let command = commands
            .iter()
            .find(|command| command["path"] == path)
            .unwrap_or_else(|| panic!("missing command {path}"));
        assert_eq!(string_set(&command["diagnosticCodes"], None), listed_ids);
    }
    let lint_command = commands
        .iter()
        .find(|command| command["path"] == "lint")
        .expect("lint command");
    assert_eq!(
        lint_command["outputSchemas"]["explain"],
        "powerbi-cli.lint.ruleExplanation.v1"
    );
    let dax = commands
        .iter()
        .find(|command| command["path"] == "model dax lint")
        .expect("DAX lint command");
    let dax_ids = string_set(&dax["diagnosticCodes"], None);
    assert_eq!(dax_ids.len(), 6);
    assert!(dax_ids.iter().all(|id| id.starts_with("dax.")));
    assert!(
        capabilities_json["contractNotes"]["explainFlagDiscipline"]
            .as_str()
            .expect("contract note")
            .contains("--explain <id>")
    );

    let features = run_powerbi(&[
        "features",
        "list",
        "--for",
        "quality.lint-rule-registry",
        "--json",
    ]);
    assert_eq!(features.code, 0, "stderr: {}", features.stderr);
    let features_json = stdout_json(&features);
    assert_eq!(features_json["matchedFeatures"], 1);
    assert_eq!(features_json["features"][0]["status"], "supported");
}

#[test]
fn lint_explain_works_for_every_registered_rule_and_is_deterministic() {
    let listed = run_powerbi(&["lint", "--rules", "--json"]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    let listed_json = stdout_json(&listed);
    for rule in listed_json["rules"].as_array().expect("rules") {
        let id = rule["id"].as_str().expect("rule id");
        let explained = run_powerbi(&["lint", "--explain", id, "--json"]);
        assert_eq!(explained.code, 0, "rule {id}, stderr: {}", explained.stderr);
        assert!(explained.stderr.trim().is_empty(), "rule {id}");
        let explained_json = stdout_json(&explained);
        assert_eq!(
            explained_json["schema"],
            "powerbi-cli.lint.ruleExplanation.v1"
        );
        assert_eq!(explained_json["rule"], *rule);
        assert_eq!(explained_json["exampleFinding"]["code"], id);
        assert_eq!(explained_json["exampleFinding"]["ruleId"], id);
        assert_eq!(
            explained_json["exampleFinding"]["severity"],
            rule["severity"]
        );
    }

    let first = run_powerbi(&["lint", "--explain", "dax.reference_self", "--json"]);
    let second = run_powerbi(&["lint", "--explain", "dax.reference_self", "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(first.stdout.as_bytes(), second.stdout.as_bytes());
    assert_eq!(first.stderr.as_bytes(), second.stderr.as_bytes());
}

#[test]
fn lint_registry_refusals_are_structured_and_suggest_executable_recovery() {
    for args in [
        vec!["lint", "--explain", "unknown.rule", "--json"],
        vec!["lint", "--explain", "--json"],
        vec![
            "lint",
            "--rules",
            "--explain",
            "dax.reference_self",
            "--json",
        ],
        vec!["lint", "build/sales", "--rules", "--json"],
    ] {
        let output = run_powerbi(&args);
        assert_eq!(output.code, 2, "args: {args:?}, stdout: {}", output.stdout);
        assert!(output.stdout.trim().is_empty(), "args: {args:?}");
        let error = stderr_json(&output);
        assert_eq!(error["error"]["code"], "invalid_args", "args: {args:?}");
        assert_eq!(error["error"]["exitCode"], 2, "args: {args:?}");
        assert!(
            error["error"]["hint"]
                .as_str()
                .is_some_and(|hint| !hint.is_empty()),
            "args: {args:?}"
        );
        let suggestions = error["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands");
        assert!(!suggestions.is_empty(), "args: {args:?}");
        assert!(suggestions.iter().all(|command| {
            command
                .as_str()
                .is_some_and(|command| command.starts_with("powerbi-cli "))
        }));
    }
}
