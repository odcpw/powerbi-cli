//! Deterministic repository documentation generated from the live CLI catalog.
//!
//! `robot-docs guide` remains the concise in-tool guide. This module owns the
//! opt-in repository writer used by `robot-docs render`; the generated regions
//! are deliberately marker-delimited so surrounding prose stays hand-owned.

use crate::cli_error::EXIT_DOCS_DRIFT;
use crate::contract::capabilities;
use crate::feature_catalog::features_command;
use crate::project_io::write_text_atomic;
use crate::{CliError, CliResult, EXIT_SUCCESS};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const README_RELATIVE: &str = "README.md";
const SKILL_RELATIVE: &str = "skills/powerbi-cli/SKILL.md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Section {
    Commands,
    Limits,
    Features,
}

impl Section {
    fn parse(value: &str) -> CliResult<Self> {
        match value.to_ascii_lowercase().as_str() {
            "commands" | "command" => Ok(Self::Commands),
            "limits" | "limit" => Ok(Self::Limits),
            "features" | "feature" => Ok(Self::Features),
            _ => Err(CliError::invalid_args(format!(
                "robot-docs render does not support section `{value}`"
            ))
            .with_hint("Choose commands, limits, or features; omit --section to render all three.")
            .with_suggested_command(
                "powerbi-cli robot-docs render --section commands --check --json",
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Limits => "limits",
            Self::Features => "features",
        }
    }

    fn start_marker(self) -> String {
        format!("<!-- powerbi-cli:{}:start -->", self.as_str())
    }

    fn end_marker(self) -> String {
        format!("<!-- powerbi-cli:{}:end -->", self.as_str())
    }
}

#[derive(Debug)]
struct Options {
    sections: Vec<Section>,
    check: bool,
    root: PathBuf,
}

#[derive(Debug)]
struct FileResult {
    path: PathBuf,
    changed: bool,
    drift: Option<String>,
}

/// Render selected live-catalog sections into the repository documentation.
///
/// Without `--check`, both README and the Power BI CLI skill are updated using
/// atomic text writes. `--check` performs the same render in memory and fails
/// with exit code 1 when either generated region differs.
pub(crate) fn render_robot_docs(args: &[String]) -> CliResult<Value> {
    let options = parse_options(args)?;
    let capabilities_value = capabilities(&[])?;
    let features_value = features_command(&["list".to_string()])?;
    let sections = options.sections;
    let mut file_results = Vec::new();

    for relative in [README_RELATIVE, SKILL_RELATIVE] {
        let path = options.root.join(relative);
        let original = fs::read_to_string(&path).map_err(|err| {
            CliError::file_not_found(format!("read documentation file {}: {err}", path.display()))
                .with_hint("Run this command from the repository root or pass --root <repo-dir>.")
                .with_suggested_command("powerbi-cli robot-docs render --check --json")
        })?;
        let mut rendered = original.clone();
        for section in &sections {
            let block = render_section(*section, &capabilities_value, &features_value)?;
            rendered = replace_marked_region(&rendered, &path, *section, &block)?;
        }
        let changed = rendered != original;
        let drift = if options.check && changed {
            Some(render_diff(&path, &original, &rendered, &sections))
        } else {
            None
        };
        if !options.check && changed {
            write_text_atomic(&path, &rendered)?;
        }
        file_results.push(FileResult {
            path,
            changed,
            drift,
        });
    }

    if options.check {
        let diffs = file_results
            .iter()
            .filter_map(|result| result.drift.as_deref())
            .collect::<Vec<_>>();
        if !diffs.is_empty() {
            return Err(CliError::new(
                "docs_drift",
                EXIT_DOCS_DRIFT,
                format!(
                    "generated documentation drift detected\n{}",
                    diffs.join("\n")
                ),
            )
            .with_hint(
                "Regenerate the marker-delimited regions with `powerbi-cli robot-docs render`.",
            )
            .with_suggested_command("powerbi-cli robot-docs render --check --json")
            .with_suggested_command("powerbi-cli robot-docs render --json"));
        }
    }

    let files = file_results
        .iter()
        .map(|result| {
            json!({
                "path": result.path.to_string_lossy(),
                "changed": result.changed,
                "drift": result.drift.is_some()
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.robot-docs.render.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "check": options.check,
        "sections": sections.iter().map(|section| section.as_str()).collect::<Vec<_>>(),
        "root": options.root.to_string_lossy(),
        "files": files,
        "sources": {
            "commands": "capabilities --json",
            "limits": "capabilities.limits",
            "features": "features list --json"
        },
        "next": ["powerbi-cli robot-docs render --check --json"]
    }))
}

fn parse_options(args: &[String]) -> CliResult<Options> {
    let mut selected = BTreeSet::new();
    let mut check = false;
    let mut root = PathBuf::from(".");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--section" => {
                let value = args.get(i + 1).ok_or_else(|| {
                    CliError::invalid_args("robot-docs render --section requires a value")
                        .with_hint("Choose commands, limits, or features.")
                        .with_suggested_command(
                            "powerbi-cli robot-docs render --section commands --check --json",
                        )
                })?;
                selected.insert(Section::parse(value)?);
                i += 2;
            }
            "--check" => {
                check = true;
                i += 1;
            }
            "--root" => {
                let value = args.get(i + 1).ok_or_else(|| {
                    CliError::invalid_args("robot-docs render --root requires a directory")
                        .with_hint("Pass the repository root containing README.md and skills/powerbi-cli/SKILL.md.")
                })?;
                root = PathBuf::from(value);
                i += 2;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown robot-docs render flag: {other}"
                ))
                .with_hint("Use --section commands|limits|features, --check, or --root <repo-dir>.")
                .with_suggested_command(
                    "powerbi-cli robot-docs render --section commands --check --json",
                ));
            }
        }
    }
    let sections = if selected.is_empty() {
        vec![Section::Commands, Section::Limits, Section::Features]
    } else {
        selected.into_iter().collect()
    };
    Ok(Options {
        sections,
        check,
        root,
    })
}

fn render_section(
    section: Section,
    capabilities_value: &Value,
    features_value: &Value,
) -> CliResult<String> {
    match section {
        Section::Commands => render_commands(capabilities_value),
        Section::Limits => render_limits(capabilities_value),
        Section::Features => render_features(features_value),
    }
}

fn render_commands(capabilities_value: &Value) -> CliResult<String> {
    let mut commands = capabilities_value["commands"]
        .as_array()
        .ok_or_else(|| CliError::unexpected("capabilities catalog has no commands array"))?
        .clone();
    commands.sort_by(|left, right| {
        left["path"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["path"].as_str().unwrap_or_default())
    });
    let mut markdown = String::from("### Commands (generated from `capabilities --json`)\n\n");
    markdown.push_str(
        "This list is generated; edit the live command catalog in `src/contract/` rather than this region.\n\n",
    );
    for command in commands {
        let usage = command["usage"].as_str().unwrap_or_default();
        let summary = command["summary"].as_str().unwrap_or_default();
        let proof = command["proofLevel"].as_str().unwrap_or("unit-smoke");
        markdown.push_str(&format!(
            "- `{}` — {} _(proof: `{}`)_\n",
            escape_inline(usage),
            escape_inline(summary),
            escape_inline(proof)
        ));
    }
    Ok(markdown.trim_end().to_string())
}

fn render_limits(capabilities_value: &Value) -> CliResult<String> {
    let limits = capabilities_value
        .get("limits")
        .ok_or_else(|| CliError::unexpected("capabilities catalog has no limits object"))?;
    let canonical = canonical_json(limits);
    let encoded = serde_json::to_string_pretty(&canonical)
        .map_err(|err| CliError::unexpected(format!("serialize capabilities limits: {err}")))?;
    Ok(format!(
        "### Input-safety limits (generated from `capabilities.limits`)\n\nThe exact bounded input contract is live in the capabilities payload.\n\n```json\n{encoded}\n```"
    ))
}

fn render_features(features_value: &Value) -> CliResult<String> {
    let mut features = features_value["features"]
        .as_array()
        .ok_or_else(|| CliError::unexpected("features catalog has no features array"))?
        .clone();
    features.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["id"].as_str().unwrap_or_default())
    });
    let mut markdown =
        String::from("### Feature catalog (generated from `features list --json`)\n\n");
    markdown.push_str(
        "Each feature carries its live support status and proof level; update `src/feature_catalog.rs` rather than this region.\n\n",
    );
    for feature in features {
        let id = feature["id"].as_str().unwrap_or_default();
        let title = feature["title"].as_str().unwrap_or_default();
        let status = feature["status"].as_str().unwrap_or_default();
        let support = feature["support"].as_str().unwrap_or_default();
        let proof = feature["proofLevel"].as_str().unwrap_or_default();
        let commands = feature["commands"]
            .as_array()
            .map(|commands| {
                commands
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|command| format!("`{}`", escape_inline(command)))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        markdown.push_str(&format!(
            "- `{}` — **{}**, {}, proof `{}`: {}.{}\n",
            escape_inline(id),
            escape_inline(status),
            escape_inline(support),
            escape_inline(proof),
            escape_inline(title),
            if commands.is_empty() {
                String::new()
            } else {
                format!(" Commands: {commands}.")
            }
        ));
    }
    Ok(markdown.trim_end().to_string())
}

fn escape_inline(value: &str) -> String {
    value.replace('`', "\\`").replace('\n', " ")
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut sorted = Map::new();
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                sorted.insert(key.clone(), canonical_json(&object[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        scalar => scalar.clone(),
    }
}

fn replace_marked_region(
    original: &str,
    path: &Path,
    section: Section,
    block: &str,
) -> CliResult<String> {
    let start = section.start_marker();
    let end = section.end_marker();
    let start_end = original.find(&start).ok_or_else(|| {
        CliError::validation_failed(format!(
            "{} is missing generated-document marker {start}",
            path.display()
        ))
        .with_hint("Add the marker pair to README.md and SKILL.md, then rerun the renderer.")
    })? + start.len();
    let end_start = original[start_end..]
        .find(&end)
        .map(|offset| start_end + offset)
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} is missing generated-document marker {end}",
                path.display()
            ))
            .with_hint("Add the marker pair to README.md and SKILL.md, then rerun the renderer.")
        })?;
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let block = block.replace('\n', newline);
    let replacement = format!("{newline}{block}{newline}");
    Ok(format!(
        "{}{}{}",
        &original[..start_end],
        replacement,
        &original[end_start..]
    ))
}

fn render_diff(path: &Path, original: &str, rendered: &str, sections: &[Section]) -> String {
    let mut output = format!(
        "--- {} (committed)\n+++ {} (rendered)\n",
        path.display(),
        path.display()
    );
    for section in sections {
        let current = marked_region(original, *section).unwrap_or_default();
        let expected = marked_region(rendered, *section).unwrap_or_default();
        if current != expected {
            output.push_str(&format!("@@ {} @@\n", section.as_str()));
            for line in current.lines() {
                output.push('-');
                output.push_str(line);
                output.push('\n');
            }
            for line in expected.lines() {
                output.push('+');
                output.push_str(line);
                output.push('\n');
            }
        }
    }
    output.trim_end().to_string()
}

fn marked_region(text: &str, section: Section) -> Option<&str> {
    let start = section.start_marker();
    let end = section.end_marker();
    let start_end = text.find(&start)? + start.len();
    let end_start = text[start_end..]
        .find(&end)
        .map(|offset| start_end + offset)?;
    Some(&text[start_end..end_start])
}
