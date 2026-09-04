//! Deterministic, path-free JSON snapshots for test contracts.

use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Compare JSON with `tests/snapshots/<name>.json`.
///
/// Set `UPDATE_SNAPSHOTS=1` to rewrite the expected file, then review its diff.
pub fn assert_json_snapshot(name: &str, value: &Value) {
    assert!(
        !name.is_empty()
            && name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')),
        "snapshot name must contain only ASCII letters, digits, dot, dash, or underscore"
    );
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.json"));
    assert_json_snapshot_at(&path, value);
}

fn assert_json_snapshot_at(path: &Path, value: &Value) {
    assert_path_free(value, "");
    let normalized = sort_json(value);
    let mut actual = serde_json::to_string_pretty(&normalized).expect("serialize JSON snapshot");
    actual.push('\n');

    if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        fs::create_dir_all(path.parent().expect("snapshot parent"))
            .expect("create snapshot directory");
        fs::write(path, actual).expect("update JSON snapshot");
        eprintln!("updated JSON snapshot: {}", path.display());
        return;
    }

    let expected = fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "read JSON snapshot {}: {error}; run UPDATE_SNAPSHOTS=1 cargo test <filter> to create it",
            path.display()
        )
    });
    assert_eq!(
        actual,
        expected.replace("\r\n", "\n"),
        "JSON snapshot mismatch: {}; review with UPDATE_SNAPSHOTS=1",
        path.display()
    );
}

fn sort_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), sort_json(value)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(sort_json).collect()),
        scalar => scalar.clone(),
    }
}

fn assert_path_free(value: &Value, pointer: &str) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                assert_path_free(value, &format!("{pointer}/{}", escape_pointer(key)));
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                assert_path_free(value, &format!("{pointer}/{index}"));
            }
        }
        Value::String(text) => {
            let windows_absolute = text.len() >= 3
                && text.as_bytes()[0].is_ascii_alphabetic()
                && text.as_bytes()[1] == b':'
                && matches!(text.as_bytes()[2], b'/' | b'\\');
            let field = pointer
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let path_field = field.contains("path")
                || field.ends_with("dir")
                || field.ends_with("directory")
                || field.ends_with("root");
            let manifest_root = env!("CARGO_MANIFEST_DIR");
            assert!(
                !windows_absolute
                    && !text.contains(manifest_root)
                    && !(path_field && Path::new(text).is_absolute()),
                "JSON snapshot contains an absolute path at {pointer}: {text:?}"
            );
        }
        _ => {}
    }
}

fn escape_pointer(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}
