//! Lossless migration from the v1 dashboard-spec shape to v2.
//!
//! The v1 and v2 shapes intentionally share the compiled subset.  Upgrading
//! therefore validates the complete v1 key surface first, rewrites only the
//! schema marker, and emits a recursively canonicalized JSON document.  No
//! fields are inferred or discarded.

use crate::input_safety::{InputKind, read_utf8};
use crate::project_io::write_json_pretty;
use crate::report_spec_schema::{DASHBOARD_V1, DASHBOARD_V2, SpecVersion, validate_known_fields};
use crate::{CliError, CliResult, EXIT_SUCCESS, canonical_display, command_arg};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct UpgradeOptions {
    spec: Option<PathBuf>,
    out: Option<PathBuf>,
    dry_run: bool,
    force: bool,
}

/// Upgrade one strict v1 dashboard spec to the equivalent v2 document.
pub(crate) fn upgrade_command(args: &[String]) -> CliResult<Value> {
    let options = parse_upgrade_args(args)?;
    let spec_path = options.spec.ok_or_else(|| {
        CliError::invalid_args("report spec upgrade requires --spec <v1.json>")
            .with_hint("Pass a powerbi-cli.dashboard.v1 spec; unknown keys are rejected before any output is written.")
            .with_suggested_command(
                "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
            )
    })?;

    if options.dry_run && options.out.is_some() {
        return Err(CliError::invalid_args(
            "report spec upgrade accepts exactly one output mode: --dry-run or --out <v2.json>",
        )
        .with_suggested_command("powerbi-cli report spec upgrade --spec <v1.json> --dry-run --json")
        .with_suggested_command(
            "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
        ));
    }
    if !options.dry_run && options.out.is_none() {
        return Err(CliError::invalid_args(
            "report spec upgrade requires --out <v2.json> or --dry-run",
        )
        .with_hint("Use --dry-run to inspect the normalized v2 spec without writing a file.")
        .with_suggested_command(
            "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
        ));
    }
    if options.force && options.out.is_none() {
        return Err(CliError::invalid_args(
            "report spec upgrade --force requires --out <v2.json>",
        )
        .with_suggested_command(
            "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --force --json",
        ));
    }

    let source = load_spec(&spec_path)?;
    let version = validate_known_fields(&source)?;
    if version == SpecVersion::V1
        && source
            .get("schema")
            .is_some_and(|schema| !schema.as_str().is_some_and(|schema| schema == DASHBOARD_V1))
    {
        return Err(CliError::invalid_args(
            "report spec upgrade requires schema powerbi-cli.dashboard.v1 when schema is present",
        )
        .with_pointer("/schema")
        .with_hint(format!("Use `{DASHBOARD_V1}` for v1 input."))
        .with_suggested_command("powerbi-cli report spec validate --spec <v1.json> --json"));
    }
    if version != SpecVersion::V1 {
        return Err(CliError::invalid_args(
            "report spec upgrade accepts only powerbi-cli.dashboard.v1 input",
        )
        .with_pointer("/schema")
        .with_hint(format!(
            "The document is already v2; validate it with `powerbi-cli report spec validate --spec {} --json`.",
            command_arg(&spec_path)
        ))
        .with_suggested_command(format!(
            "powerbi-cli report spec validate --spec {} --json",
            command_arg(&spec_path)
        )));
    }

    let (upgraded, changes) = rewrite_schema(source)?;
    // Validate the transformed document as v2 as a final guard.  This keeps
    // the command lossless while proving that every retained key belongs to
    // the target shape before the output is published.
    validate_known_fields(&upgraded)?;

    let normalized = normalize_json(upgraded);
    let out = options.out.as_deref();
    if let Some(out) = out {
        if out.exists() && !options.force {
            return Err(CliError::invalid_args(format!(
                "report spec upgrade output already exists: {}",
                out.display()
            ))
            .with_hint(
                "Pass --force after reviewing the existing file, or choose a new --out path.",
            )
            .with_suggested_command(format!(
                "powerbi-cli report spec upgrade --spec {} --out {} --force --json",
                command_arg(&spec_path),
                command_arg(out)
            )));
        }
        if !options.dry_run {
            write_json_pretty(out, &normalized)?;
        }
    }

    let transformed_pointers = changes
        .iter()
        .filter_map(|change| change.get("pointer").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let upgraded_spec = normalized.clone();
    Ok(json!({
        "schema": "powerbi-cli.report.spec.upgrade.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "changed": out.is_some() && !options.dry_run,
        "dryRun": options.dry_run,
        "specPath": canonical_display(&spec_path),
        "outPath": out.map(canonical_display),
        "sourceVersion": DASHBOARD_V1,
        "targetVersion": DASHBOARD_V2,
        "transformed": transformed_pointers,
        "transformedPointers": transformed_pointers,
        "changes": changes,
        "spec": normalized,
        "upgradedSpec": upgraded_spec,
        "next": next_for_upgrade(&spec_path, out, options.dry_run)
    }))
}

fn load_spec(path: &Path) -> CliResult<Value> {
    let text = read_utf8(path, InputKind::DashboardSpec)?;
    serde_json::from_str(&text).map_err(|error| {
        CliError::invalid_args(format!("parse dashboard spec {}: {error}", path.display()))
    })
}

fn rewrite_schema(mut source: Value) -> CliResult<(Value, Vec<Value>)> {
    let object = source
        .as_object_mut()
        .ok_or_else(|| CliError::invalid_args("dashboard spec root must be an object"))?;
    let prior = object.get("schema").cloned();
    let operation = if prior.is_some() { "rewrite" } else { "insert" };
    object.insert(
        "schema".to_string(),
        Value::String(DASHBOARD_V2.to_string()),
    );
    Ok((
        source,
        vec![json!({
            "pointer": "/schema",
            "operation": operation,
            "from": prior,
            "to": DASHBOARD_V2
        })],
    ))
}

fn normalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries = object.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut normalized = Map::new();
            for (key, value) in entries {
                normalized.insert(key, normalize_json(value));
            }
            Value::Object(normalized)
        }
        Value::Array(values) => Value::Array(values.into_iter().map(normalize_json).collect()),
        other => other,
    }
}

fn next_for_upgrade(spec: &Path, out: Option<&Path>, dry_run: bool) -> Vec<String> {
    if dry_run {
        return vec![format!(
            "powerbi-cli report spec upgrade --spec {} --out <v2.json> --json",
            command_arg(spec)
        )];
    }
    let Some(out) = out else {
        return Vec::new();
    };
    vec![
        format!(
            "powerbi-cli report spec validate --spec {} --json",
            command_arg(out)
        ),
        format!(
            "powerbi-cli report build --schema <schema.json> --spec {} --out-dir <project-dir> --json",
            command_arg(out)
        ),
    ]
}

fn parse_upgrade_args(args: &[String]) -> CliResult<UpgradeOptions> {
    let mut options = UpgradeOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--spec" => {
                options.spec = Some(PathBuf::from(take_value(args, &mut i, "--spec")?));
            }
            "--out" | "--out-file" => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            "--dry-run" => {
                options.dry_run = true;
                i += 1;
            }
            "--force" => {
                options.force = true;
                i += 1;
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown report spec upgrade flag: {other}"
                ))
                .with_suggested_command(
                    "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
                ));
            }
            other => {
                if options.spec.is_some() {
                    return Err(CliError::invalid_args(
                        "report spec upgrade accepts at most one positional spec path",
                    )
                    .with_suggested_command(
                        "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
                    ));
                }
                options.spec = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(options)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value_index = index.saturating_add(1);
    let Some(value) = args.get(value_index) else {
        return Err(CliError::invalid_args(format!("{flag} requires a value"))
            .with_suggested_command(
                "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
            ));
    };
    if value.starts_with('-') {
        return Err(
            CliError::invalid_args(format!("{flag} requires a non-flag value"))
                .with_suggested_command(
                    "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
                ),
        );
    }
    *index = value_index + 1;
    Ok(value.clone())
}

#[cfg(test)]
mod tests {
    use super::{DASHBOARD_V2, normalize_json, rewrite_schema};
    use serde_json::json;

    #[test]
    fn rewrite_changes_only_the_schema_pointer() {
        let source = json!({"pages": [], "schema": "powerbi-cli.dashboard.v1"});
        let (upgraded, changes) = rewrite_schema(source.clone()).expect("rewrite");
        assert_eq!(upgraded["pages"], source["pages"]);
        assert_eq!(upgraded["schema"], DASHBOARD_V2);
        assert_eq!(changes[0]["pointer"], "/schema");
        assert_eq!(changes[0]["operation"], "rewrite");
    }

    #[test]
    fn normalization_sorts_every_object_without_reordering_arrays() {
        let value = json!({"z": {"b": 1, "a": 2}, "a": [{"z": 0, "a": 1}, 3]});
        let normalized = normalize_json(value);
        let text = serde_json::to_string(&normalized).expect("serialize normalized value");
        assert_eq!(text, r#"{"a":[{"a":1,"z":0},3],"z":{"a":2,"b":1}}"#);
    }
}
