mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use std::fs;
use std::path::Path;

fn copy_documentation(root: &Path) {
    fs::create_dir_all(root.join("skills/powerbi-cli")).expect("create skill directory");
    fs::copy("README.md", root.join("README.md")).expect("copy README");
    fs::copy(
        "skills/powerbi-cli/SKILL.md",
        root.join("skills/powerbi-cli/SKILL.md"),
    )
    .expect("copy skill");
}

fn generated_region<'a>(text: &'a str, section: &str) -> &'a str {
    let start = format!("<!-- powerbi-cli:{section}:start -->");
    let end = format!("<!-- powerbi-cli:{section}:end -->");
    let start_end = text.find(&start).expect("generated start marker") + start.len();
    let end_start = text[start_end..]
        .find(&end)
        .map(|offset| start_end + offset)
        .expect("generated end marker");
    &text[start_end..end_start]
}

fn outside_generated_region(text: &str, section: &str) -> String {
    let start = format!("<!-- powerbi-cli:{section}:start -->");
    let end = format!("<!-- powerbi-cli:{section}:end -->");
    let start_end = text.find(&start).expect("generated start marker") + start.len();
    let end_start = text[start_end..]
        .find(&end)
        .map(|offset| offset + start_end)
        .expect("generated end marker");
    let mut outside = String::with_capacity(text.len());
    outside.push_str(&text[..start_end]);
    outside.push_str(&text[end_start..]);
    outside
}

fn root_args(root: &Path, section: &str, check: bool) -> Vec<String> {
    let mut args = vec![
        "robot-docs".to_string(),
        "render".to_string(),
        "--root".to_string(),
        root.to_string_lossy().into_owned(),
        "--section".to_string(),
        section.to_string(),
        "--json".to_string(),
    ];
    if check {
        args.insert(args.len() - 1, "--check".to_string());
    }
    args
}

#[test]
fn render_check_is_deterministic_and_regions_match_between_docs() {
    let first = run_powerbi(&["robot-docs", "render", "--check", "--json"]);
    let second = run_powerbi(&["robot-docs", "render", "--check", "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "render check must be deterministic"
    );
    let value = stdout_json(&first);
    assert_eq!(value["schema"], "powerbi-cli.robot-docs.render.v1");
    assert_eq!(value["check"], true);
    assert_eq!(
        value["sections"],
        serde_json::json!(["commands", "limits", "features"])
    );
    assert!(value["files"].as_array().is_some_and(|files| {
        files.len() == 2 && files.iter().all(|file| file["changed"] == false)
    }));

    let readme = fs::read_to_string("README.md").expect("README");
    let skill = fs::read_to_string("skills/powerbi-cli/SKILL.md").expect("SKILL");
    for section in ["commands", "limits", "features"] {
        let readme_region = generated_region(&readme, section);
        let skill_region = generated_region(&skill, section);
        assert!(
            !readme_region.trim().is_empty(),
            "empty README {section} region"
        );
        assert_eq!(
            readme_region, skill_region,
            "README and SKILL {section} regions must be identical"
        );
    }
}

#[test]
fn render_check_reports_drift_and_render_rewrites_only_selected_regions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    copy_documentation(&root);
    let initial_args = root_args(&root, "commands", false);
    let initial_args = initial_args.iter().map(String::as_str).collect::<Vec<_>>();
    let rendered = run_powerbi(&initial_args);
    assert_eq!(rendered.code, 0, "stderr: {}", rendered.stderr);

    let readme_path = root.join("README.md");
    let skill_path = root.join("skills/powerbi-cli/SKILL.md");
    let canonical_skill = fs::read_to_string(&skill_path).expect("rendered SKILL");
    let canonical_limits = generated_region(&canonical_skill, "limits").to_string();
    let before_drift = fs::read_to_string(&readme_path).expect("rendered README");
    let mutated = before_drift.replacen(
        "### Commands (generated from `capabilities --json`)",
        "### Commands (drifted)",
        1,
    );
    assert_ne!(mutated, before_drift);
    fs::write(&readme_path, mutated).expect("write drift");
    let drift_args = root_args(&root, "commands", true);
    let drift_args = drift_args.iter().map(String::as_str).collect::<Vec<_>>();
    let drift = run_powerbi(&drift_args);
    assert_eq!(drift.code, 1, "drift check must exit one: {}", drift.stderr);
    let error = stderr_json(&drift);
    assert_eq!(error["error"]["code"], "docs_drift");
    assert_eq!(error["error"]["exitCode"], 1);
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("README.md") && message.contains("drifted"))
    );
    assert_eq!(
        fs::read_to_string(&skill_path).expect("SKILL after check"),
        canonical_skill,
        "--check must not write the other documentation file"
    );

    let check_args = root_args(&root, "limits", true);
    let check_args = check_args.iter().map(String::as_str).collect::<Vec<_>>();
    let limits_check = run_powerbi(&check_args);
    assert_eq!(
        limits_check.code, 0,
        "unselected section should remain clean"
    );
    assert_eq!(
        generated_region(&canonical_skill, "limits"),
        canonical_limits
    );

    let repair_args = root_args(&root, "commands", false);
    let repair_args = repair_args.iter().map(String::as_str).collect::<Vec<_>>();
    let repair = run_powerbi(&repair_args);
    assert_eq!(repair.code, 0, "stderr: {}", repair.stderr);
    let repaired = fs::read_to_string(&readme_path).expect("repaired README");
    assert_eq!(
        outside_generated_region(&repaired, "commands"),
        outside_generated_region(&before_drift, "commands"),
        "render must preserve README text outside selected markers"
    );
    let repaired_check = run_powerbi(&drift_args);
    assert_eq!(repaired_check.code, 0, "repaired docs should pass check");
    assert_eq!(
        generated_region(&repaired, "commands"),
        generated_region(&canonical_skill, "commands")
    );
}

#[test]
fn render_rejects_unknown_sections_and_catalog_paths_are_in_skill_region() {
    let invalid = run_powerbi(&["robot-docs", "render", "--section", "unknown", "--json"]);
    assert_eq!(invalid.code, 2);
    let error = stderr_json(&invalid);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("commands") && hint.contains("features"))
    );

    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let capabilities_value = stdout_json(&capabilities);
    let paths = capabilities_value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    let skill = fs::read_to_string("skills/powerbi-cli/SKILL.md").expect("SKILL");
    let discovery = generated_region(&skill, "commands");
    for path in paths {
        assert!(
            discovery.contains(path),
            "generated SKILL discovery is missing catalog path {path}"
        );
    }
}
