//! Check the public discovery document against the real CLI dispatcher.

mod common;

use common::{hash_tree, run_powerbi, scaffold_sales, stdout_json};
use serde_json::Value;
use std::collections::BTreeMap;

fn invoke(path: &str, flags: &[&str]) -> Value {
    let args: Vec<_> = path
        .split_ascii_whitespace()
        .chain(flags.iter().copied())
        .chain(["--json"])
        .collect();
    let run = run_powerbi(&args);
    assert_eq!(run.exit, 0, "{args:?}: {}", run.stderr);
    stdout_json(&run)
}

#[test]
fn every_advertised_theme_or_style_path_has_one_owner_and_matches_its_runtime_schema() {
    let root = tempfile::tempdir().unwrap();
    let catalog = invoke("capabilities", &[]);
    let commands = catalog["commands"].as_array().unwrap();
    let target = scaffold_sales(root.path());
    let project = target.to_str().unwrap();
    let before = hash_tree(&target);
    let themed = root.path().join("themed");
    invoke(
        "report themes apply-preset",
        &[
            "--project",
            project,
            "--preset",
            "risk-dashboard",
            "--out-dir",
            themed.to_str().unwrap(),
        ],
    );
    let source = themed.to_str().unwrap();

    let raw_path = root.path().join("raw.json");
    let portable_path = root.path().join("portable.json");
    let raw = raw_path.to_str().unwrap();
    let portable = portable_path.to_str().unwrap();
    invoke(
        "report themes extract",
        &["--project", source, "--out", raw],
    );
    invoke(
        "report style extract",
        &["--project", source, "--out", portable],
    );
    let source_before = hash_tree(&themed);

    let mut owners = BTreeMap::new();
    for command in commands {
        let owner = command["path"].as_str().unwrap();
        let aliases = command["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().unwrap());
        for spelling in std::iter::once(owner).chain(aliases) {
            if !matches!(
                spelling.split(' ').take(2).collect::<Vec<_>>().as_slice(),
                ["report", "theme" | "themes" | "style" | "styles"]
            ) {
                continue;
            }
            assert_eq!(
                owners.insert(spelling, owner),
                None,
                "duplicate owner of {spelling}"
            );
            let args: &[&str] = match owner {
                "report themes presets" => &["list"],
                "report themes apply-preset" => &[
                    "--project",
                    project,
                    "--preset",
                    "risk-dashboard",
                    "--dry-run",
                ],
                "report themes apply" => &["--project", project, "--bundle", raw, "--dry-run"],
                "report style apply" => &["--project", project, "--bundle", portable, "--dry-run"],
                "report style diff" => &[portable, portable],
                _ => &["--project", source],
            };
            let response = invoke(spelling, args);
            assert_eq!(response["schema"], command["outputSchema"], "{spelling}");
            match owner {
                "report themes extract" => assert_eq!(
                    response["bundle"]["schema"],
                    "powerbi-cli.report.theme-bundle.v1"
                ),
                "report style extract" => assert_eq!(
                    response["bundle"]["schema"],
                    "powerbi-cli.report.style-bundle.v1"
                ),
                "report themes apply" | "report themes apply-preset" | "report style apply" => {
                    assert_eq!(response["dryRun"], true)
                }
                _ => {}
            }
        }
    }
    // Historically these spellings were incorrectly advertised for raw report
    // themes. Keep both families discoverable with the right bundle semantics.
    for (spelling, owner) in [
        ("report theme show", "report themes show"),
        ("report theme extract", "report themes extract"),
        ("report theme apply", "report themes apply"),
        ("report style show", "report style inspect"),
        ("report styles show", "report style inspect"),
        ("report styles extract", "report style extract"),
        ("report styles apply", "report style apply"),
    ] {
        assert_eq!(owners.get(spelling), Some(&owner), "{spelling}");
    }
    assert_eq!(hash_tree(&target), before, "dry-run changed the target");
    assert_eq!(
        hash_tree(&themed),
        source_before,
        "inspection changed the source"
    );
}
