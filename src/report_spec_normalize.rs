//! Canonical dashboard-spec composition and artifact output.

use crate::json_composition::normalize_spec_file;
use crate::report_spec_schema::validate_known_fields;
use crate::{CliError, CliResult, EXIT_SUCCESS, canonical_display, command_arg};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct Options {
    spec: Option<PathBuf>,
    out: Option<PathBuf>,
}

pub(crate) fn normalize_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    let spec_path = options.spec.ok_or_else(|| {
        CliError::invalid_args("report spec normalize requires <dashboard.json>")
            .with_hint("Run `powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json`.")
            .with_suggested_command(
                "powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json",
            )
    })?;
    let out = options.out.ok_or_else(|| {
        CliError::invalid_args("report spec normalize requires --out <canonical.json>")
            .with_hint("Run `powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json`.")
            .with_suggested_command(
                "powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json",
            )
    })?;
    let normalized = normalize_spec_file(&spec_path)?;
    let version = validate_known_fields(&normalized.value)?;
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::unexpected(format!(
                "create output directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    let mut output = serde_json::to_string_pretty(&normalized.value).map_err(|error| {
        CliError::unexpected(format!("serialize normalized dashboard spec: {error}"))
    })?;
    output.push('\n');
    fs::write(&out, output)
        .map_err(|error| CliError::unexpected(format!("write {}: {error}", out.display())))?;

    Ok(json!({
        "schema": "powerbi-cli.report.spec.normalize.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "specPath": canonical_display(&spec_path),
        "normalizedOut": canonical_display(&out),
        "normalizedFrom": normalized.normalized_from,
        "specVersion": match version {
            crate::report_spec_schema::SpecVersion::V1 => crate::report_spec_schema::DASHBOARD_V1,
            crate::report_spec_schema::SpecVersion::V2 => crate::report_spec_schema::DASHBOARD_V2,
        },
        "next": [
            format!("powerbi-cli report spec validate {} --json", command_arg(&out)),
            format!("powerbi-cli report build --schema <schema.json> --spec {} --out-dir <project-dir> --json", command_arg(&out))
        ]
    }))
}

fn parse_args(args: &[String]) -> CliResult<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--spec" => {
                if options.spec.is_some() {
                    return Err(CliError::invalid_args(
                        "report spec normalize accepts only one --spec path",
                    )
                    .with_suggested_command(
                        "powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json",
                    ));
                }
                options.spec = Some(take_value(args, &mut index, "--spec")?);
            }
            "--out" | "--out-dir" => {
                if options.out.is_some() {
                    return Err(CliError::invalid_args(
                        "report spec normalize accepts only one --out path",
                    )
                    .with_suggested_command(
                        "powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json",
                    ));
                }
                options.out = Some(take_value(args, &mut index, "--out")?);
            }
            value if value.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown report spec normalize flag: {value}"
                ))
                .with_hint("Run `powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json`.")
                .with_suggested_command(
                    "powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json",
                ));
            }
            value => {
                if options.spec.is_some() {
                    return Err(CliError::invalid_args(
                        "report spec normalize accepts exactly one spec path",
                    )
                    .with_suggested_command(
                        "powerbi-cli report spec normalize <dashboard.json> --out <canonical.json> --json",
                    ));
                }
                options.spec = Some(PathBuf::from(value));
                index += 1;
            }
        }
    }
    Ok(options)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<PathBuf> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
    *index += 2;
    Ok(Path::new(value).to_path_buf())
}
