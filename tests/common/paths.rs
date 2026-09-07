//! Platform-neutral path helpers for assertions against CLI output.
//!
//! The CLI canonicalizes project paths (symlinks and Windows 8.3 short names
//! resolved) and renders them without the Windows verbatim `\\?\` prefix.
//! Tests that compare emitted `path` fields, or that redact a temporary root
//! before snapshotting, must use the same form on every platform.

use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// Render `path` the way the CLI does: canonical, without a Windows verbatim
/// prefix. Components below the deepest existing ancestor are appended as
/// given, so a path to a not-yet-written or deleted file still renders.
pub fn canonical_display(path: &Path) -> String {
    let mut existing: PathBuf = path.to_path_buf();
    let mut missing: Vec<OsString> = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .unwrap_or_else(|| panic!("no existing ancestor for {}", path.display()))
            .to_os_string();
        missing.push(name);
        existing = existing
            .parent()
            .unwrap_or_else(|| panic!("no existing ancestor for {}", path.display()))
            .to_path_buf();
    }
    let mut canonical = fs::canonicalize(&existing)
        .unwrap_or_else(|error| panic!("canonicalize {}: {error}", existing.display()));
    for name in missing.iter().rev() {
        canonical.push(name);
    }
    strip_verbatim_prefix(&canonical.to_string_lossy())
}

fn strip_verbatim_prefix(value: &str) -> String {
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        value.to_string()
    }
}

/// Replace `from` with `to` inside every string of a JSON value.
///
/// Serialized JSON escapes Windows separators, so replacing on the serialized
/// text misses `C:\...` roots; this walks the values instead.
pub fn replace_in_strings(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::Object(object) => {
            for child in object.values_mut() {
                replace_in_strings(child, from, to);
            }
        }
        Value::Array(values) => {
            for child in values {
                replace_in_strings(child, from, to);
            }
        }
        Value::String(text) if text.contains(from) => {
            *text = text.replace(from, to);
        }
        _ => {}
    }
}

/// In every string that carries `marker` (a placeholder such as `<root>`),
/// use forward slashes so one snapshot serves Linux and Windows.
pub fn forward_slashes_after(value: &mut Value, marker: &str) {
    match value {
        Value::Object(object) => {
            for child in object.values_mut() {
                forward_slashes_after(child, marker);
            }
        }
        Value::Array(values) => {
            for child in values {
                forward_slashes_after(child, marker);
            }
        }
        Value::String(text) if text.contains(marker) && text.contains('\\') => {
            *text = text.replace('\\', "/");
        }
        _ => {}
    }
}
