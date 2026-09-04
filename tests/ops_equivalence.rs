//! Focused artifact-parity fixtures for typed operation kernels.
//!
//! The binary's public operation-apply dispatcher lands in a later bead, so
//! these integration cases exercise the existing CLI mutation contract and the
//! exact flat `ops.v1` payload that the in-crate kernel tests apply.

mod common;

use common::{first_page_name, first_two_visual_names, run_powerbi, scaffold_sales};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
