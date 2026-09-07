mod common;

use common::{
    assert_json_snapshot, canonical_display, first_page_json, first_visual_json, hash_tree,
    patch_json, run_powerbi, scaffold_sales, stderr_json, stdout_json,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn plan(project: &Path) -> Value {
    let output = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project.to_str().unwrap(),
        "--allow-unmapped",
        "--json",
    ]);
    assert_eq!(output.code, 0, "{}", output.stderr);
    stdout_json(&output)
}

#[test]
fn runbook_embeds_live_scorecard_and_deterministic_unverified_proof_ladder() {
    let temp = tempfile::tempdir().unwrap();
    let project = scaffold_sales(temp.path());
    patch_json(&first_visual_json(&project), |visual| {
        visual["position"]["x"] = json!(-8)
    });
    let before = hash_tree(&project);
    let first = plan(&project);
    assert_eq!(first, plan(&project));
    let triage = run_powerbi(&["triage", project.to_str().unwrap(), "--json"]);
    assert_eq!(triage.code, 0, "{}", triage.stderr);
    assert_eq!(first["scorecard"], stdout_json(&triage)["scorecard"]);
    assert_eq!(first["scorecard"]["schema"], "scorecard.v1");
    assert_eq!(first["scorecard"]["proofLevel"], "unit-smoke");
    assert!(
        first["scorecard"]["designLint"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["ruleId"] == "report.visual_outside_page")
    );
    assert_eq!(first["proofLadder"][0]["status"], "local-checks-passed");
    assert_eq!(first["proofLadder"].as_array().unwrap().len(), 5);
    assert!(
        first["proofLadder"].as_array().unwrap()[1..]
            .iter()
            .all(|row| row["status"] == "not-verified")
    );
    let markdown = first["instructionsMarkdown"].as_str().unwrap();
    let scorecard_json = serde_json::to_string_pretty(&first["scorecard"]).unwrap();
    assert!(markdown.contains(&format!("```json\n{scorecard_json}\n```")));
    assert!(markdown.contains("report.visual_outside_page"));
    assert!(markdown.contains("relayout-template"));
    assert!(markdown.contains("/report/pages/0/visuals/0/position"));
    assert_json_snapshot("handoff-runbook-proof-ladder", &first["proofLadder"]);

    // The full scorecard is asserted against triage above. Replace only that
    // platform-dependent block and the temporary root in the runbook golden.
    let normalized = markdown
        .replace(
            &scorecard_json,
            "<live scorecard.v1 JSON; asserted against triage>",
        )
        .replace(&canonical_display(&project), "<project-dir>")
        // The runbook renders native separators; the golden uses `/` everywhere.
        .replace("<project-dir>\\", "<project-dir>/");
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/golden/handoff-rebind-runbook.md");
    if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        fs::write(&golden, &normalized).unwrap();
    }
    assert_eq!(
        normalized,
        fs::read_to_string(golden).unwrap().replace("\r\n", "\n")
    );
    assert_eq!(before, hash_tree(&project));
}

#[test]
fn runbook_reflects_new_validation_failures_without_promoting_proof() {
    let temp = tempfile::tempdir().unwrap();
    let project = scaffold_sales(temp.path());
    let clean = plan(&project);
    assert_eq!(clean["scorecard"]["validation"]["ok"], true);
    patch_json(&first_page_json(&project), |page| page["width"] = json!(-1));
    let failed = plan(&project);
    assert_eq!(failed["scorecard"]["validation"]["ok"], false);
    assert_eq!(failed["proofLadder"][0]["status"], "local-checks-failed");
    assert!(
        failed["instructionsMarkdown"]
            .as_str()
            .unwrap()
            .contains("local-checks-failed")
    );
    assert_ne!(clean["scorecard"], failed["scorecard"]);
}

#[test]
fn written_runbook_matches_response_and_preserves_overwrite_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let project = scaffold_sales(temp.path());
    let out = temp.path().join("runbook.md");
    let args = [
        "handoff",
        "rebind-plan",
        project.to_str().unwrap(),
        "--allow-unmapped",
        "--out",
        out.to_str().unwrap(),
        "--json",
    ];
    let first = run_powerbi(&args);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let value = stdout_json(&first);
    let bytes = fs::read(&out).unwrap();
    assert_eq!(
        bytes,
        value["instructionsMarkdown"].as_str().unwrap().as_bytes()
    );
    assert_eq!(value["runbookWritten"], true);
    let refused = run_powerbi(&args);
    assert_eq!(refused.code, 2);
    let error = stderr_json(&refused);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["hint"]
            .as_str()
            .is_some_and(|hint| !hint.is_empty())
    );
    assert!(
        error["error"]["suggestedCommands"]
            .as_array()
            .is_some_and(|commands| !commands.is_empty()
                && commands.iter().all(|command| command
                    .as_str()
                    .is_some_and(|command| command.starts_with("powerbi-cli "))))
    );
    assert_eq!(bytes, fs::read(&out).unwrap());
    let mut force = args.to_vec();
    force.push("--force");
    let forced = run_powerbi(&force);
    assert_eq!(forced.code, 0, "{}", forced.stderr);
    assert_eq!(bytes, fs::read(&out).unwrap());
}
