//! Shared validation scorecard for build and triage responses.
//!
//! The scorecard deliberately keeps the individual proof surfaces separate:
//! native validation, the optional Microsoft validator, lint, design lint,
//! handoff safety, and the achieved proof level.  Build and triage call the
//! same projection so an agent can compare a freshly written project with a
//! later triage run without translating command-specific response shapes.

use crate::handoff::handoff_command;
use crate::lint::lint_project;
use crate::{
    Finding, ResolvedProject, ValidationReport, canonical_display, command_arg, validate_project,
};
use serde_json::{Value, json};

const DESIGN_LINT_UNAVAILABLE_REASON: &str = "design lint lands in t5-3";

/// Build a scorecard from a project path.  The helper owns the read-only
/// validation/lint/handoff calls so callers that do not already have those
/// reports (for example report build after scaffolding) can still use the
/// exact same projection as triage.
pub(crate) fn project_scorecard(resolved: &ResolvedProject, proof_level: &str) -> Value {
    let validation = match validate_project(resolved) {
        Ok(report) => report,
        Err(error) => {
            return unavailable_scorecard(
                Some(json!({
                    "ok": false,
                    "errors": [json!({
                        "code": "validation.unavailable",
                        "message": error.message,
                        "path": canonical_display(&resolved.project_dir),
                        "pointer": "",
                        "severity": "error"
                    })],
                    "warnings": []
                })),
                proof_level,
                format!(
                    "native validation could not inspect {}",
                    resolved.project_dir.display()
                ),
            );
        }
    };
    let lint = match lint_project(resolved, &validation) {
        Ok(value) => value,
        Err(error) => json!({
            "ok": false,
            "counts": {"errors": 1, "warnings": 0, "info": 0, "findings": 1},
            "findings": [json!({
                "code": "lint.unavailable",
                "severity": "error",
                "message": error.message,
                "handle": Value::Null,
                "path": canonical_display(&resolved.project_dir)
            })]
        }),
    };
    scorecard_from_parts(resolved, &validation, &lint, proof_level)
}

/// Project an already-computed validation and lint result into the shared
/// scorecard.  Triage uses this path to avoid running its expensive deep
/// inspection twice while retaining byte-identical fields with build.
pub(crate) fn scorecard_from_parts(
    resolved: &ResolvedProject,
    validation: &ValidationReport,
    lint: &Value,
    proof_level: &str,
) -> Value {
    let validation_value = json!({
        "ok": validation.errors.is_empty(),
        "errors": findings_to_values(&validation.errors),
        "warnings": findings_to_values(&validation.warnings)
    });
    let lint_value = lint_scorecard(lint);
    let handoff = handoff_scorecard(resolved);
    let all_green = validation.errors.is_empty()
        && lint_value["ok"].as_bool().unwrap_or(false)
        && handoff["safeForOfflineHandoff"] == Value::Bool(true);
    let next = scorecard_next(resolved, all_green);

    json!({
        "schema": "scorecard.v1",
        "validation": validation_value,
        "microsoftValidator": microsoft_validator_scorecard(),
        "lint": lint_value,
        "designLint": {
            "status": "unavailable",
            "reason": DESIGN_LINT_UNAVAILABLE_REASON,
            "findings": []
        },
        "handoff": handoff,
        "proofLevel": proof_level,
        "next": next
    })
}

/// Shape used when a build is a dry run and therefore has no project tree on
/// which native validation, lint, or handoff can operate yet.
pub(crate) fn dry_run_scorecard(proof_level: &str, next: Vec<String>) -> Value {
    json!({
        "schema": "scorecard.v1",
        "validation": {
            "ok": Value::Null,
            "status": "unavailable",
            "reason": "dry-run does not create a project tree",
            "errors": [],
            "warnings": []
        },
        "microsoftValidator": microsoft_validator_scorecard(),
        "lint": {
            "ok": Value::Null,
            "status": "unavailable",
            "reason": "dry-run does not create a project tree",
            "counts": {"errors": 0, "warnings": 0, "info": 0, "findings": 0},
            "findings": {"error": [], "warning": [], "info": []},
            "findingsList": []
        },
        "designLint": {
            "status": "unavailable",
            "reason": DESIGN_LINT_UNAVAILABLE_REASON,
            "findings": []
        },
        "handoff": {
            "status": "unavailable",
            "safeForOfflineHandoff": false,
            "reason": "dry-run does not create a project tree"
        },
        "proofLevel": proof_level,
        "next": next
    })
}

fn unavailable_scorecard(validation: Option<Value>, proof_level: &str, reason: String) -> Value {
    json!({
        "schema": "scorecard.v1",
        "validation": validation.unwrap_or_else(|| json!({
            "ok": false,
            "status": "unavailable",
            "reason": reason,
            "errors": [],
            "warnings": []
        })),
        "microsoftValidator": microsoft_validator_scorecard(),
        "lint": {
            "ok": false,
            "status": "unavailable",
            "reason": reason,
            "counts": {"errors": 0, "warnings": 0, "info": 0, "findings": 0},
            "findings": {"error": [], "warning": [], "info": []},
            "findingsList": []
        },
        "designLint": {
            "status": "unavailable",
            "reason": DESIGN_LINT_UNAVAILABLE_REASON,
            "findings": []
        },
        "handoff": {
            "status": "unavailable",
            "safeForOfflineHandoff": false,
            "reason": reason
        },
        "proofLevel": proof_level,
        "next": []
    })
}

fn findings_to_values(findings: &[Finding]) -> Vec<Value> {
    findings
        .iter()
        .map(|finding| serde_json::to_value(finding).expect("Finding serializes"))
        .collect()
}

fn lint_scorecard(lint: &Value) -> Value {
    let mut findings = lint["findings"].as_array().cloned().unwrap_or_default();
    sort_findings(&mut findings);
    let mut by_severity = serde_json::Map::new();
    for severity in ["error", "warning", "info"] {
        by_severity.insert(
            severity.to_string(),
            Value::Array(
                findings
                    .iter()
                    .filter(|finding| finding["severity"].as_str() == Some(severity))
                    .cloned()
                    .collect(),
            ),
        );
    }
    let error_count = by_severity["error"].as_array().map_or(0, Vec::len);
    let warning_count = by_severity["warning"].as_array().map_or(0, Vec::len);
    let info_count = by_severity["info"].as_array().map_or(0, Vec::len);
    json!({
        "ok": lint["ok"].as_bool().unwrap_or(error_count == 0),
        "counts": {
            "errors": error_count,
            "warnings": warning_count,
            "info": info_count,
            "findings": findings.len()
        },
        "findings": by_severity,
        "findingsList": findings
    })
}

fn sort_findings(findings: &mut [Value]) {
    findings.sort_by(|left, right| {
        severity_rank(left)
            .cmp(&severity_rank(right))
            .then_with(|| finding_path(left).cmp(finding_path(right)))
            .then_with(|| {
                left["code"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["code"].as_str().unwrap_or_default())
            })
            .then_with(|| {
                left["message"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["message"].as_str().unwrap_or_default())
            })
    });
}

fn severity_rank(finding: &Value) -> u8 {
    match finding["severity"].as_str() {
        Some("error") => 0,
        Some("warning") => 1,
        Some("info") => 2,
        _ => 3,
    }
}

fn finding_path(finding: &Value) -> &str {
    finding["path"].as_str().unwrap_or_default()
}

fn handoff_scorecard(resolved: &ResolvedProject) -> Value {
    let args = vec![
        "check".to_string(),
        "--project".to_string(),
        resolved.project_dir.to_string_lossy().into_owned(),
        "--target".to_string(),
        "offline".to_string(),
    ];
    match handoff_command(&args) {
        Ok(value) => json!({
            "status": value
                .get("status")
                .cloned()
                .unwrap_or_else(|| Value::String("unavailable".into())),
            "safeForOfflineHandoff": value["safeForOfflineHandoff"].as_bool().unwrap_or(false),
            "target": "offline",
            "findings": value
                .get("findings")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()))
        }),
        Err(error) => json!({
            "status": "unavailable",
            "safeForOfflineHandoff": false,
            "target": "offline",
            "reason": error.message,
            "findings": []
        }),
    }
}

fn microsoft_validator_scorecard() -> Value {
    if cfg!(windows) {
        json!({
            "status": "not-installed",
            "reason": "Microsoft Report Validator availability is checked on the Windows work machine",
            "next": ["powerbi-cli integrations status --json"]
        })
    } else {
        json!({
            "status": "unsupported-platform",
            "reason": "Microsoft Report Validator runs on Windows; local native validation remains available"
        })
    }
}

fn scorecard_next(resolved: &ResolvedProject, all_green: bool) -> Vec<String> {
    let project = command_arg(&resolved.project_dir);
    let mut next = vec![
        format!("powerbi-cli inspect --deep {project} --json"),
        format!("powerbi-cli validate --strict {project} --json"),
        format!("powerbi-cli handoff check {project} --json"),
    ];
    if all_green {
        next.push(format!("powerbi-cli desktop open {project} --json"));
    }
    next
}
