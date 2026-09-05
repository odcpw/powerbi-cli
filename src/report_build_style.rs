//! Compile dashboard-spec style declarations into final-stage operations.

use crate::ops::{MutationPayload, Op};
use crate::{CliError, CliResult};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) fn compile_style_operations(
    spec: &Map<String, Value>,
) -> CliResult<(Vec<Op>, Vec<String>)> {
    let Some(style) = spec.get("style") else {
        return Ok((Vec::new(), Vec::new()));
    };
    let style = style.as_object().ok_or_else(|| {
        CliError::invalid_args("dashboard spec style must be an object").with_pointer("/style")
    })?;
    refuse_deferred_style(style)?;
    if style.contains_key("preset") && style.contains_key("bundle") {
        return Err(CliError::invalid_args(
            "dashboard spec style must choose either preset or bundle",
        )
        .with_pointer("/style")
        .with_hint("Remove either style.preset or style.bundle."));
    }
    let allow_literal_text = match style.get("allowLiteralText") {
        Some(Value::Bool(value)) => *value,
        Some(_) => {
            return Err(
                CliError::invalid_args("style.allowLiteralText must be a boolean")
                    .with_pointer("/style/allowLiteralText"),
            );
        }
        None => false,
    };
    if allow_literal_text && !style.contains_key("bundle") {
        return Err(CliError::invalid_args(
            "style.allowLiteralText is only valid with style.bundle",
        )
        .with_pointer("/style/allowLiteralText"));
    }
    if let Some(preset) = style.get("preset") {
        return compile_preset(preset);
    }
    if let Some(bundle) = style.get("bundle") {
        return compile_bundle(bundle, allow_literal_text);
    }
    Ok((Vec::new(), Vec::new()))
}

fn compile_preset(value: &Value) -> CliResult<(Vec<Op>, Vec<String>)> {
    let preset = value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CliError::invalid_args("style.preset must be a non-empty string")
                .with_pointer("/style/preset")
        })?;
    let (preset, _) = crate::report_themes::parse_apply_theme_preset_operation(&[
        "--preset".to_string(),
        preset.to_string(),
        "--dry-run".to_string(),
    ])
    .map_err(|error| error.with_pointer("/style/preset"))?;
    Ok((
        vec![Op::ApplyThemePreset(preset)],
        vec!["/style/preset".to_string()],
    ))
}

fn compile_bundle(value: &Value, allow_literal_text: bool) -> CliResult<(Vec<Op>, Vec<String>)> {
    let bundle = value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            CliError::invalid_args("style.bundle must be a non-empty path string")
                .with_pointer("/style/bundle")
        })?;
    crate::report_style::validate_compiler_bundle(Path::new(bundle), allow_literal_text)?;
    let mut fields = BTreeMap::from([("bundle".to_string(), Value::String(bundle.to_string()))]);
    if allow_literal_text {
        fields.insert("allowLiteralText".to_string(), Value::Bool(true));
    }
    Ok((
        vec![Op::ApplyStyleBundle(MutationPayload { fields })],
        vec!["/style/bundle".to_string()],
    ))
}

fn refuse_deferred_style(style: &Map<String, Value>) -> CliResult<()> {
    if style.contains_key("defaults") {
        return Err(uncompiled_style_error("style.defaults", "/style/defaults"));
    }
    Ok(())
}

fn uncompiled_style_error(section: &str, pointer: &str) -> CliError {
    const BEAD: &str = "pbi-t3-compiler-completeness-1qi.13";
    CliError::unsupported_feature(format!(
        "dashboard spec section `{section}` is recognized but not compiled; owning bead: {BEAD}"
    ))
    .with_pointer(pointer)
    .with_hint(format!(
        "Keep the section for future compilation or remove it before build. Owning bead: {BEAD}."
    ))
    .with_suggested_command(
        "powerbi-cli report themes apply-preset --project <project-dir> --preset <preset> --dry-run --json",
    )
}
