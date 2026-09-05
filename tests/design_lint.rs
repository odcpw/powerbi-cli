mod common;

use common::{
    assert_json_snapshot, first_visual_json, patch_json, run_powerbi, scaffold_sales, stderr_json,
    stdout_json,
};
use serde_json::{Value, json};

fn finding_projection(value: &Value) -> Value {
    Value::Array(
        value["findings"]
            .as_array()
            .expect("design findings")
            .iter()
            .map(|finding| {
                json!({
                    "ruleId": finding["ruleId"],
                    "severity": finding["severity"],
                    "pointer": finding["pointer"],
                    "sanitizeAction": finding["sanitizeAction"]
                })
            })
            .collect(),
    )
}

#[test]
fn design_geometry_is_opt_in_for_audit_and_separate_from_default_lint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let clean = run_powerbi(&["lint", project_arg, "--json"]);
    assert_eq!(clean.code, 0, "stderr: {}", clean.stderr);
    assert!(
        !stdout_json(&clean)["findings"]
            .as_array()
            .expect("clean findings")
            .iter()
            .any(|finding| finding["ruleId"] == "report.visual_outside_page")
    );

    patch_json(&first_visual_json(&project), |visual| {
        visual["position"]["x"] = Value::from(-8.0);
    });

    let first = run_powerbi(&["lint", project_arg, "--json"]);
    let second = run_powerbi(&["lint", project_arg, "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(first.stdout.as_bytes(), second.stdout.as_bytes());
    assert_eq!(first.stderr.as_bytes(), second.stderr.as_bytes());
    let lint = stdout_json(&first);
    assert!(lint.get("designLint").is_none());
    assert_eq!(lint["counts"], stdout_json(&clean)["counts"]);
    assert!(lint["findings"].as_array().unwrap().iter().all(|finding| {
        !finding["code"]
            .as_str()
            .unwrap_or_default()
            .starts_with("design.")
    }));
    let triage_output = run_powerbi(&["triage", project_arg, "--json"]);
    let scorecard = stdout_json(&triage_output);
    let design = &scorecard["scorecard"]["designLint"];
    assert_eq!(design["schema"], "powerbi-cli.design.lint.v1");
    assert_eq!(design["status"], "available");
    assert_eq!(design["proofLevel"], "unit-smoke");
    assert_eq!(design["ruleIds"].as_array().expect("rule ids").len(), 20);
    assert_eq!(design["evaluatedRules"].as_array().unwrap().len(), 16);
    assert_eq!(design["deferredRules"].as_array().unwrap().len(), 4);
    assert!(
        design["deferredRules"]
            .as_array()
            .unwrap()
            .iter()
            .all(|rule| rule["status"] == "not-evaluated" && rule["reason"].is_string())
    );
    let outside = design["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|finding| finding["ruleId"] == "report.visual_outside_page")
        .expect("outside-page finding");
    assert!(
        outside["pointer"]
            .as_str()
            .is_some_and(|pointer| pointer.starts_with('/'))
    );
    assert_eq!(outside["sanitizeAction"], "relayout-template");

    let audit = run_powerbi(&[
        "report",
        "audit",
        "--project",
        project_arg,
        "--rules",
        "design",
        "--json",
    ]);
    assert_eq!(audit.code, 0, "stderr: {}", audit.stderr);
    let audit = stdout_json(&audit);
    assert_eq!(audit["rules"], "design");
    assert!(
        audit["findings"]
            .as_array()
            .expect("audit findings")
            .iter()
            .all(|finding| {
                finding["jsonPointer"]
                    .as_str()
                    .is_some_and(|pointer| pointer.starts_with('/'))
                    && finding["pointer"] == finding["jsonPointer"]
                    && finding["ruleId"].as_str().is_some_and(|id| {
                        id.starts_with("design.") || id == "report.visual_outside_page"
                    })
            })
    );
    assert!(
        audit["unsupportedActions"]
            .as_array()
            .expect("actions")
            .iter()
            .any(|action| {
                action["kind"] == "relayout-template"
                    && action["sourceRuleIds"]
                        .as_array()
                        .is_some_and(|ids| ids.iter().any(|id| id == "report.visual_outside_page"))
            })
    );

    let triage = run_powerbi(&["triage", project_arg, "--json"]);
    assert_eq!(triage.code, 0, "stderr: {}", triage.stderr);
    assert_eq!(stdout_json(&triage)["scorecard"]["designLint"], *design);

    assert_json_snapshot(
        "design-lint-batch2",
        &json!({
            "schema": design["schema"],
            "status": design["status"],
            "proofLevel": design["proofLevel"],
            "ruleIds": design["ruleIds"],
            "evaluatedRules": design["evaluatedRules"],
            "deferredRules": design["deferredRules"],
            "findings": finding_projection(design)
        }),
    );
}

#[test]
fn report_audit_rejects_unknown_design_rule_set_with_executable_recovery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let output = run_powerbi(&[
        "report",
        "audit",
        "--project",
        project.to_str().expect("project path"),
        "--rules",
        "typography",
        "--json",
    ]);
    assert_eq!(output.code, 2);
    assert!(output.stdout.trim().is_empty());
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["hint"]
            .as_str()
            .is_some_and(|hint| !hint.is_empty())
    );
    assert!(
        error["error"]["suggestedCommands"]
            .as_array()
            .expect("suggestions")
            .iter()
            .all(|command| command
                .as_str()
                .is_some_and(|command| command.starts_with("powerbi-cli ")))
    );
}

#[test]
fn design_rules_are_explainable_and_advertised_by_capabilities_and_features() {
    let listed = run_powerbi(&["lint", "--rules", "--json"]);
    assert_eq!(listed.code, 0, "stderr: {}", listed.stderr);
    let listed = stdout_json(&listed);
    let design_rules = listed["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .filter(|rule| rule["family"] == "design")
        .collect::<Vec<_>>();
    assert_eq!(design_rules.len(), 20);
    for rule in design_rules {
        let id = rule["id"].as_str().expect("rule id");
        let explained = run_powerbi(&["lint", "--explain", id, "--json"]);
        assert_eq!(explained.code, 0, "{id}: {}", explained.stderr);
        let explained = stdout_json(&explained);
        assert_eq!(explained["rule"], *rule);
        assert_eq!(explained["exampleFinding"]["ruleId"], id);
    }

    let capabilities = run_powerbi(&["capabilities", "--for", "report audit", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let capabilities = stdout_json(&capabilities);
    let audit = capabilities["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report audit")
        .expect("report audit capability");
    assert!(
        audit["usage"]
            .as_str()
            .is_some_and(|usage| usage.contains("--rules design"))
    );
    assert_eq!(
        audit["diagnosticCodes"]
            .as_array()
            .expect("diagnostic codes")
            .iter()
            .filter(|code| {
                code.as_str().is_some_and(|code| {
                    code.starts_with("design.") || code == "report.visual_outside_page"
                })
            })
            .count(),
        20
    );

    let feature = run_powerbi(&["features", "list", "--for", "quality.design-lint", "--json"]);
    assert_eq!(feature.code, 0, "stderr: {}", feature.stderr);
    let feature = stdout_json(&feature);
    assert_eq!(feature["matchedFeatures"], 1);
    assert_eq!(feature["features"][0]["status"], "supported");
    assert_eq!(feature["features"][0]["proofLevel"], "unit-smoke");
}
