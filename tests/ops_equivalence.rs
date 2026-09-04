//! External artifact checks for the typed operation kernels.
//!
//! The in-crate kernel tests exercise `Transaction` directly (the binary has
//! no public ops/apply command yet). This target keeps the established CLI
//! path in the same deterministic artifact corpus used by those tests.

mod common;

use common::{first_page_name, run_powerbi, scaffold_sales};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let relative = entry.path().strip_prefix(root).expect("relative path");
            (
                relative.to_path_buf(),
                fs::read(entry.path()).expect("artifact bytes"),
            )
        })
        .collect()
}

#[test]
fn add_filter_ops_equivalence_fixture_is_byte_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let source_arg = source.to_str().expect("source path");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
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
