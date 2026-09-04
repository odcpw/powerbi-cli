//! Focused artifact-parity fixtures for typed operation kernels.
//!
//! These process-level cases exercise the existing CLI mutation contract and
//! the exact flat `ops.v1` payload that the in-crate kernels apply. The fixtures
//! cover every registered operation kernel, including the visual style and
//! geometry operations added on this branch.

mod common;

use common::{
    first_page_name, first_two_visual_names, first_visual_json, run_powerbi, scaffold_sales,
    stdout_json,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn visual_handle(project: &Path) -> String {
    let visual_path = first_visual_json(project);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("read visual"))
            .expect("parse visual");
    let page = first_page_name(project);
    format!(
        "visual:{page}:{}",
        visual["name"].as_str().expect("visual name")
    )
}

fn project_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            (
                entry
                    .path()
                    .strip_prefix(root)
                    .expect("relative")
                    .to_path_buf(),
                fs::read(entry.path()).expect("read file"),
            )
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[test]
fn set_object_op_replays_are_deterministic_and_preserve_cli_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = scaffold_sales(&temp.path().join("first"));
    let second = scaffold_sales(&temp.path().join("second"));
    let first_handle = visual_handle(&first);
    let second_handle = visual_handle(&second);
    assert_eq!(first_handle, second_handle);

    let first_out = temp.path().join("first-out");
    let second_out = temp.path().join("second-out");
    for (project, handle, out) in [
        (&first, first_handle, &first_out),
        (&second, second_handle, &second_out),
    ] {
        let output = run_powerbi(&[
            "report",
            "visuals",
            "set-object",
            "--project",
            project.to_str().expect("project path"),
            "--handle",
            &handle,
            "--object",
            "categoryLabels",
            "--property",
            "fontSize",
            "--value",
            "20",
            "--out-dir",
            out.to_str().expect("out path"),
            "--json",
        ]);
        assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
        assert_eq!(
            stdout_json(&output)["schema"],
            "powerbi-cli.report.visuals.objectMutation.v1"
        );
    }
    assert_eq!(project_files(&first_out), project_files(&second_out));
}

#[test]
fn set_position_op_replays_are_deterministic_and_preserve_cli_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = scaffold_sales(&temp.path().join("first"));
    let second = scaffold_sales(&temp.path().join("second"));
    let first_handle = visual_handle(&first);
    let second_handle = visual_handle(&second);
    assert_eq!(first_handle, second_handle);
    let page = first_page_name(&first);

    let first_out = temp.path().join("first-out");
    let second_out = temp.path().join("second-out");
    for (project, handle, out) in [
        (&first, first_handle, &first_out),
        (&second, second_handle, &second_out),
    ] {
        let output = run_powerbi(&[
            "report",
            "visuals",
            "set-position",
            "--project",
            project.to_str().expect("project path"),
            "--page",
            &page,
            "--visual",
            handle.rsplit(':').next().expect("visual name"),
            "--x",
            "120",
            "--y",
            "140",
            "--width",
            "360",
            "--height",
            "220",
            "--z",
            "5",
            "--tab-order",
            "4",
            "--out-dir",
            out.to_str().expect("out path"),
            "--json",
        ]);
        assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
        assert_eq!(
            stdout_json(&output)["schema"],
            "powerbi-cli.report.visuals.positionMutation.v1"
        );
    }
    assert_eq!(project_files(&first_out), project_files(&second_out));
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(root).ok()?.to_path_buf();
            Some((relative, fs::read(entry.path()).ok()?))
        })
        .collect()
}

#[test]
fn set_interaction_cli_fixture_is_byte_deterministic_and_matches_ops_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = scaffold_sales(&temp.path().join("first"));
    let second = scaffold_sales(&temp.path().join("second"));
    let page = first_page_name(&first);
    let (source, target) = first_two_visual_names(&first);
    let page_handle = format!("page:{page}");
    let source_handle = format!("visual:{page}:{source}");
    let target_handle = format!("visual:{page}:{target}");
    let args = |project: &Path| {
        vec![
            "report".to_string(),
            "interactions".to_string(),
            "set".to_string(),
            "--project".to_string(),
            project.to_string_lossy().into_owned(),
            "--page".to_string(),
            page_handle.clone(),
            "--source".to_string(),
            source_handle.clone(),
            "--target".to_string(),
            target_handle.clone(),
            "--type".to_string(),
            "DataFilter".to_string(),
            "--in-place".to_string(),
            "--json".to_string(),
        ]
    };
    let first_output = run_powerbi(&args(&first).iter().map(String::as_str).collect::<Vec<_>>());
    let second_output = run_powerbi(&args(&second).iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(
        first_output.code, 0,
        "first mutation: {}",
        first_output.stderr
    );
    assert_eq!(
        second_output.code, 0,
        "second mutation: {}",
        second_output.stderr
    );
    assert_eq!(tree(&first), tree(&second));

    let operation = json!({
        "op": "setInteraction",
        "page": page_handle,
        "source": source_handle,
        "target": target_handle,
        "interactionType": "DataFilter"
    });
    assert_eq!(operation["op"], "setInteraction");
    assert_eq!(operation.as_object().expect("operation object").len(), 5);
}

#[test]
fn apply_theme_preset_cli_fixture_is_byte_deterministic_and_matches_ops_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = scaffold_sales(&temp.path().join("first"));
    let second = scaffold_sales(&temp.path().join("second"));
    let args = |project: &Path| {
        vec![
            "report".to_string(),
            "themes".to_string(),
            "apply-preset".to_string(),
            "--project".to_string(),
            project.to_string_lossy().into_owned(),
            "--preset".to_string(),
            "risk-dashboard".to_string(),
            "--in-place".to_string(),
            "--json".to_string(),
        ]
    };
    let first_output = run_powerbi(&args(&first).iter().map(String::as_str).collect::<Vec<_>>());
    let second_output = run_powerbi(&args(&second).iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(
        first_output.code, 0,
        "first preset mutation: {}",
        first_output.stderr
    );
    assert_eq!(
        second_output.code, 0,
        "second preset mutation: {}",
        second_output.stderr
    );
    assert_eq!(tree(&first), tree(&second));

    let operation = json!({
        "op": "applyThemePreset",
        "preset": "risk-dashboard"
    });
    assert_eq!(operation["op"], "applyThemePreset");
    assert_eq!(operation["preset"], "risk-dashboard");
}

#[test]
fn add_filter_ops_equivalence_fixture_is_byte_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let source_arg = source.to_str().expect("source path");
    let first = temp.path().join("first-filter");
    let second = temp.path().join("second-filter");
    for output in [&first, &second] {
        let output_arg = output.to_str().expect("output path");
        let run = run_powerbi(&[
            "report",
            "filters",
            "add",
            "--project",
            source_arg,
            "--scope",
            "report",
            "--target",
            "DimCustomer[Segment]",
            "--value",
            "Enterprise",
            "--out-dir",
            output_arg,
            "--json",
        ]);
        assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    }
    assert_eq!(tree(&first), tree(&second));
}

#[test]
fn set_drillthrough_ops_equivalence_fixture_is_byte_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let source_arg = source.to_str().expect("source path");
    let page_handle = format!("page:{}", first_page_name(&source));
    let first = temp.path().join("first-drillthrough");
    let second = temp.path().join("second-drillthrough");
    for output in [&first, &second] {
        let output_arg = output.to_str().expect("output path");
        let run = run_powerbi(&[
            "report",
            "drillthrough",
            "set",
            "--project",
            source_arg,
            "--page",
            &page_handle,
            "--target",
            "DimCustomer[Segment]",
            "--out-dir",
            output_arg,
            "--json",
        ]);
        assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    }
    assert_eq!(tree(&first), tree(&second));
}
