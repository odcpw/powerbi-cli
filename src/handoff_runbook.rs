//! Live quality evidence for the work-machine rebind runbook.

use crate::desktop_proof::ProofLevel;
use serde_json::{Value, json};

/// Planning never executes a golden comparison or a Desktop oracle. The only
/// evidence available here is the shared scorecard's local inspection result.
pub(crate) fn proof_ladder(scorecard: &Value) -> Value {
    let local_ok = scorecard["validation"]["ok"] == true
        && scorecard["lint"]["ok"] == true
        && scorecard["designLint"]["ok"] == true;
    Value::Array(
        [
            (ProofLevel::UnitSmoke, "Native validation and local lint checks; warnings remain in the scorecard."),
            (ProofLevel::SchemaGolden, "Compare the current project with an approved schema golden."),
            (ProofLevel::DesktopGoldenPending, "Provide a matching Desktop-authored reference; canvas and refresh proof is still pending."),
            (ProofLevel::ManualDesktopCanvasRefresh, "Record a manual Desktop canvas and refresh review for the current project."),
            (ProofLevel::DesktopCanvasRefresh, "Record automated Desktop canvas and refresh evidence for the current project."),
        ]
        .into_iter()
        .map(|(level, evidence)| {
            json!({
                "level": level.as_str(),
                "status": if level == ProofLevel::UnitSmoke {
                    if local_ok { "local-checks-passed" } else { "local-checks-failed" }
                } else { "not-verified" },
                "evidenceRequired": evidence
            })
        })
        .collect(),
    )
}

pub(crate) fn markdown(scorecard: &Value, ladder: &Value) -> String {
    let mut out = String::from("## Design scorecard and proof status\n\n");
    out.push_str("This scorecard was computed from the current project before writing this runbook. Re-run triage after rebinding or editing the project. Source-template completeness does not establish report quality.\n\n");
    out.push_str(&format!(
        "Local proof level: `{}`. Generating this runbook does not verify a schema golden, open Desktop, refresh data, or inspect a canvas. No external proof record is imported by this command.\n\n",
        scorecard["proofLevel"].as_str().unwrap_or("unavailable")
    ));
    out.push_str("| Proof level | Status | Evidence required |\n| --- | --- | --- |\n");
    if let Some(rows) = ladder.as_array() {
        for row in rows {
            out.push_str(&format!(
                "| `{}` | {} | {} |\n",
                row["level"].as_str().unwrap_or_default(),
                row["status"].as_str().unwrap_or_default(),
                row["evidenceRequired"].as_str().unwrap_or_default()
            ));
        }
    }
    out.push_str("\nThe full `scorecard.v1` below includes native validation, Microsoft-validator availability, general lint, design findings with pointers and suggested actions, offline handoff safety, and follow-up commands.\n\n```json\n");
    out.push_str(&serde_json::to_string_pretty(scorecard).expect("serialize scorecard value"));
    out.push_str("\n```\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_failure_never_becomes_a_pass_or_desktop_evidence() {
        for scorecard in [json!({}), json!({"validation": {"ok": false}})] {
            let ladder = proof_ladder(&scorecard);
            assert_eq!(ladder[0]["status"], "local-checks-failed");
            assert!(
                ladder.as_array().unwrap()[1..]
                    .iter()
                    .all(|row| row["status"] == "not-verified")
            );
        }
    }
}
