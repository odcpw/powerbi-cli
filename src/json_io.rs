//! Shared JSON file input.

use crate::input_safety::{InputKind, read_utf8};
use crate::{CliError, CliResult};
use serde_json::Value;
use std::path::Path;

pub(crate) fn read_json_value(path: &Path) -> CliResult<Value> {
    let text = read_utf8(path, InputKind::JsonArtifact)?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::validation_failed(format!("parse JSON {}: {err}", path.display())))
}
