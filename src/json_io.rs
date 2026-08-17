//! Shared JSON file input.

use crate::{CliError, CliResult};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub(crate) fn read_json_value(path: &Path) -> CliResult<Value> {
    let text = fs::read_to_string(path)
        .map_err(|err| CliError::file_not_found(format!("read {}: {err}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::validation_failed(format!("parse JSON {}: {err}", path.display())))
}
