//! Integration coverage for operation-kernel parity at the CLI boundary.
//!
//! The direct typed-kernel equivalence test lives beside the private binary
//! modules (`ops::set_object::tests`). This process-level case verifies that
//! the public command remains deterministic when the same typed payload is
//! replayed against equivalent projects.

mod common;

use common::{first_page_name, first_visual_json, run_powerbi, scaffold_sales, stdout_json};
use serde_json::Value;
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
