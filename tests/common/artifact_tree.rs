//! Deterministic, path-normalized fingerprints for generated project trees.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// The stable artifact-tree checksum contract used by parity and equivalence
/// tests. Relative paths are normalized to / on every host.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct TreeFingerprint {
    pub files: usize,
    pub bytes: u64,
    pub sha256: String,
}

/// Hash every ordinary file below root using artifact-tree v1.
pub fn hash_tree(root: &Path) -> TreeFingerprint {
    hash_tree_with_ignored(root, &[])
}

/// Hash every ordinary file below root except normalized relative paths listed
/// in ignored. This is useful for generated trees that carry an immutable
/// source-manifest sidecar alongside the compiled PBIP artifacts.
pub fn hash_tree_with_ignored(root: &Path, ignored: &[&str]) -> TreeFingerprint {
    let files = file_paths(root)
        .into_iter()
        .filter(|path| !ignored.contains(&normalized_relative(root, path).as_str()))
        .collect::<Vec<_>>();
    let mut hasher = Sha256::new();
    hasher.update(b"powerbi-cli.artifact-tree.v1\0");
    let mut total_bytes = 0_u64;
    for path in &files {
        let relative = normalized_relative(root, path);
        let bytes = fs::read(path)
            .unwrap_or_else(|error| panic!("read generated artifact {}: {error}", path.display()));
        let relative_bytes = relative.as_bytes();
        hasher.update((relative_bytes.len() as u64).to_le_bytes());
        hasher.update(relative_bytes);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        total_bytes += bytes.len() as u64;
    }

    TreeFingerprint {
        files: files.len(),
        bytes: total_bytes,
        sha256: format!("{:x}", hasher.finalize()),
    }
}

/// Assert byte-identical trees and include both fingerprints and a per-file
/// first difference in the panic. File descriptions contain only paths,
/// lengths, and digests, never generated row contents.
pub fn assert_tree_equal(left: &Path, right: &Path, label: &str) {
    assert_tree_equal_with_ignored(left, right, label, &[]);
}

/// Assert byte-identical trees while excluding explicitly named metadata files.
/// The panic still includes both filtered trees and their first differing file.
pub fn assert_tree_equal_with_ignored(left: &Path, right: &Path, label: &str, ignored: &[&str]) {
    let left_fingerprint = hash_tree_with_ignored(left, ignored);
    let right_fingerprint = hash_tree_with_ignored(right, ignored);
    if left_fingerprint == right_fingerprint {
        return;
    }
    let difference = first_difference_with_ignored(left, right, ignored)
        .unwrap_or_else(|| "fingerprints differ but no file-level difference was found".into());
    panic!(
        "{label}: artifact trees differ\nleft: {} => {left_fingerprint:?}\nright: {} => {right_fingerprint:?}\nfirst differing file: {difference}\nleft files:\n{}\nright files:\n{}",
        left.display(),
        right.display(),
        describe_tree(left, ignored),
        describe_tree(right, ignored),
    );
}

/// Return the first lexicographically ordered file difference between trees.
pub fn first_difference(left: &Path, right: &Path) -> Option<String> {
    first_difference_with_ignored(left, right, &[])
}

/// Return the first difference after excluding explicit metadata files.
pub fn first_difference_with_ignored(
    left: &Path,
    right: &Path,
    ignored: &[&str],
) -> Option<String> {
    let left_files = tree_files_with_ignored(left, ignored);
    let right_files = tree_files_with_ignored(right, ignored);
    let mut paths = left_files
        .keys()
        .chain(right_files.keys())
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    for path in paths {
        match (left_files.get(&path), right_files.get(&path)) {
            (Some(left_bytes), Some(right_bytes)) if left_bytes == right_bytes => {}
            (Some(left_bytes), Some(right_bytes)) => {
                return Some(format!(
                    "{path}: bytes differ (left {} bytes, sha256 {}; right {} bytes, sha256 {})",
                    left_bytes.len(),
                    digest(left_bytes),
                    right_bytes.len(),
                    digest(right_bytes),
                ));
            }
            (Some(left_bytes), None) => {
                return Some(format!(
                    "{path}: present only on left ({} bytes, sha256 {})",
                    left_bytes.len(),
                    digest(left_bytes),
                ));
            }
            (None, Some(right_bytes)) => {
                return Some(format!(
                    "{path}: present only on right ({} bytes, sha256 {})",
                    right_bytes.len(),
                    digest(right_bytes),
                ));
            }
            (None, None) => unreachable!("path came from one of the two maps"),
        }
    }
    None
}

/// Collect normalized relative paths and bytes for diagnostics or exact
/// byte-level comparisons.
pub fn tree_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    tree_files_with_ignored(root, &[])
}

/// Collect tree files after excluding normalized relative paths.
pub fn tree_files_with_ignored(root: &Path, ignored: &[&str]) -> BTreeMap<String, Vec<u8>> {
    file_paths(root)
        .into_iter()
        .filter(|path| !ignored.contains(&normalized_relative(root, path).as_str()))
        .map(|path| {
            let relative = normalized_relative(root, &path);
            let bytes = fs::read(&path).unwrap_or_else(|error| {
                panic!("read generated artifact {}: {error}", path.display())
            });
            (relative, bytes)
        })
        .collect()
}

fn file_paths(root: &Path) -> Vec<PathBuf> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.expect("walk generated artifact tree"))
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| normalized_relative(root, path));
    files
}

fn normalized_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("artifact under tree root")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn describe_tree(root: &Path, ignored: &[&str]) -> String {
    tree_files_with_ignored(root, ignored)
        .into_iter()
        .map(|(path, bytes)| format!("{path} ({} bytes, sha256 {})", bytes.len(), digest(&bytes)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
