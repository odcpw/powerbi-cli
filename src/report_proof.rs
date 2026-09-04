//! Compile dashboard-spec proof requirements into deterministic follow-up commands.
//!
//! Proof planning is deliberately side-effect free.  The planner describes the
//! Desktop, DAX, and fixture commands an agent can run later; it never launches
//! Desktop, reads model data, or writes an evidence artifact.

use crate::cli_support::shell_arg;
use crate::desktop_proof::ProofLevel;
use crate::{CliError, CliResult, command_arg};
use serde_json::{Map, Value, json};
use std::path::Path;

const WINDOWS_ORACLE_INSTRUCTION: &str =
    "Windows with Power BI Desktop installed and POWERBI_DESKTOP_ORACLE=1; run the command there.";
const WINDOWS_T9_INSTRUCTION: &str =
    "Windows with Power BI Desktop and the T9.1/T9.2 Desktop oracle commands available.";
const DESKTOP_REFERENCE_INSTRUCTION: &str =
    "Windows with a matching Desktop-authored proof record under testdata/desktop-proof/.";

/// The fully rendered proof plan returned by the report compiler.
#[derive(Debug, Clone)]
pub(crate) struct ProofPlan {
    pub(crate) value: Value,
    pub(crate) next: Vec<String>,
}

/// Compile an optional `proof` block from a dashboard spec.
///
/// `project_dir` is intentionally optional: dry-run/spec-validation callers use
/// `<project-dir>` while a writing build can render the exact output path.  Both
/// forms are executable command templates and are deterministic for the same
/// input.
pub(crate) fn compile_proof_plan(
    spec: Option<&Value>,
    project_dir: Option<&Path>,
) -> CliResult<Option<ProofPlan>> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let Some(raw_proof) = spec.get("proof") else {
        return Ok(None);
    };
    let proof = raw_proof.as_object().ok_or_else(|| {
        CliError::invalid_args("dashboard spec proof must be an object").with_pointer("/proof")
    })?;

    let desktop = match proof.get("desktop") {
        Some(value) => Some(value.as_object().ok_or_else(|| {
            CliError::invalid_args("dashboard spec proof.desktop must be an object")
                .with_pointer("/proof/desktop")
        })?),
        None => None,
    };
    let requested_level = requested_level(proof, desktop)?;
    // The planner itself never executes Desktop, so Linux/macOS are capped at
    // schema-golden. Windows can host the pending Desktop/reference step, but
    // stronger canvas/refresh levels still require the T9 oracle commands.
    let achievable_here = if cfg!(windows) {
        ProofLevel::DesktopGoldenPending
    } else {
        ProofLevel::SchemaGolden
    };
    let project = project_dir
        .map(command_arg)
        .unwrap_or_else(|| "<project-dir>".to_string());

    let pages = desktop
        .and_then(|value| value.get("pages"))
        .map(parse_pages)
        .transpose()?;
    let expect_values = desktop
        .and_then(|value| value.get("expectValues"))
        .map(parse_expect_values)
        .transpose()?;
    let goldens = proof
        .get("goldens")
        .map(parse_goldens)
        .transpose()?
        .unwrap_or_default();

    let mut commands = Vec::new();
    if requested_level >= ProofLevel::DesktopGoldenPending {
        commands.push(format!(
            "powerbi-cli desktop open {project} --preflight strict --json"
        ));
    }
    if requested_level >= ProofLevel::ManualDesktopCanvasRefresh {
        commands.push(format!(
            "powerbi-cli desktop refresh-check {project} --json"
        ));
        if let Some(pages) = pages.as_ref() {
            for page in pages {
                commands.push(format!(
                    "powerbi-cli desktop canvas-check {project} --page {} --expect <values.json> --json",
                    shell_arg(page)
                ));
            }
        }
    }
    if let Some(expect_values) = expect_values.as_ref() {
        for expectation in expect_values {
            commands.push(format!(
                "powerbi-cli model dax execute --project {project} --query {} --allow-data-read --enable-oracle --max-rows 10 --json",
                shell_arg(&expectation.query)
            ));
        }
    }
    for golden in &goldens {
        commands.push(format!(
            "powerbi-cli fixture verify {project} --expected {} --json",
            shell_arg(golden)
        ));
    }
    if requested_level >= ProofLevel::DesktopGoldenPending {
        commands.push("powerbi-cli desktop close --json".to_string());
    }

    let mut unavailable = Vec::new();
    let desktop_commands = commands
        .iter()
        .filter(|command| is_desktop_dependent(command))
        .cloned()
        .collect::<Vec<_>>();
    for command in desktop_commands {
        let (why, where_it_works) = unavailable_reason(&command);
        unavailable.push(json!({
            "what": command,
            "why": why,
            "whereItWorks": where_it_works
        }));
    }
    if requested_level > achievable_here {
        let (why, where_it_works) = requested_level_reason();
        unavailable.push(json!({
            "what": requested_level.as_str(),
            "why": why,
            "whereItWorks": where_it_works
        }));
    }
    if requested_level >= ProofLevel::DesktopGoldenPending && goldens.is_empty() {
        unavailable.push(json!({
            "what": "desktop reference",
            "why": "missing_reference",
            "whereItWorks": DESKTOP_REFERENCE_INSTRUCTION
        }));
    }
    if requested_level >= ProofLevel::ManualDesktopCanvasRefresh
        && pages.as_ref().is_none_or(Vec::is_empty)
    {
        unavailable.push(json!({
            "what": "desktop canvas page",
            "why": "missing_reference",
            "whereItWorks": "Add proof.desktop.pages[] page identifiers from the Desktop-authored report."
        }));
    }

    let value = json!({
        "requestedLevel": requested_level.as_str(),
        "achievableHere": achievable_here.as_str(),
        "commands": commands,
        "unavailable": unavailable
    });
    let next = value["commands"]
        .as_array()
        .expect("proof plan commands array")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    Ok(Some(ProofPlan { value, next }))
}

fn requested_level(
    proof: &Map<String, Value>,
    desktop: Option<&Map<String, Value>>,
) -> CliResult<ProofLevel> {
    let raw = desktop
        .and_then(|value| value.get("level"))
        .or_else(|| proof.get("required"));
    let Some(raw) = raw else {
        return Ok(ProofLevel::SchemaGolden);
    };
    let value = raw.as_str().ok_or_else(|| {
        CliError::invalid_args("dashboard spec proof level must be a string").with_pointer(
            if desktop.is_some() {
                "/proof/desktop/level"
            } else {
                "/proof/required"
            },
        )
    })?;
    ProofLevel::parse(value).ok_or_else(|| {
        CliError::invalid_args(format!("unsupported dashboard proof level: {value}"))
            .with_pointer(if desktop.is_some() {
                "/proof/desktop/level"
            } else {
                "/proof/required"
            })
            .with_hint("Use unit-smoke, schema-golden, desktop-golden-pending, manual-desktop-canvas-refresh, or desktop-canvas-refresh.")
    })
}

fn parse_pages(value: &Value) -> CliResult<Vec<String>> {
    let pages = value.as_array().ok_or_else(|| {
        CliError::invalid_args("proof.desktop.pages must be an array")
            .with_pointer("/proof/desktop/pages")
    })?;
    pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let page = page.as_str().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "proof.desktop.pages[{index}] must be a non-empty string"
                ))
                .with_pointer(format!("/proof/desktop/pages/{index}"))
            })?;
            if page.trim().is_empty() {
                return Err(CliError::invalid_args(format!(
                    "proof.desktop.pages[{index}] must be a non-empty string"
                ))
                .with_pointer(format!("/proof/desktop/pages/{index}")));
            }
            Ok(page.to_string())
        })
        .collect()
}

#[derive(Debug)]
struct ExpectValue {
    query: String,
    #[allow(dead_code)]
    expected: Value,
}

fn parse_expect_values(value: &Value) -> CliResult<Vec<ExpectValue>> {
    let values = value.as_array().ok_or_else(|| {
        CliError::invalid_args("proof.desktop.expectValues must be an array")
            .with_pointer("/proof/desktop/expectValues")
    })?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let object = value.as_object().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "proof.desktop.expectValues[{index}] must be an object"
                ))
                .with_pointer(format!("/proof/desktop/expectValues/{index}"))
            })?;
            let query = object
                .get("query")
                .and_then(Value::as_str)
                .filter(|query| !query.trim().is_empty())
                .ok_or_else(|| {
                    CliError::invalid_args(format!(
                        "proof.desktop.expectValues[{index}].query must be a non-empty string"
                    ))
                    .with_pointer(format!("/proof/desktop/expectValues/{index}/query"))
                })?;
            let expected = object.get("expected").cloned().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "proof.desktop.expectValues[{index}] requires expected"
                ))
                .with_pointer(format!("/proof/desktop/expectValues/{index}/expected"))
            })?;
            Ok(ExpectValue {
                query: query.to_string(),
                expected,
            })
        })
        .collect()
}

fn parse_goldens(value: &Value) -> CliResult<Vec<String>> {
    let values = value.as_array().ok_or_else(|| {
        CliError::invalid_args("proof.goldens must be an array").with_pointer("/proof/goldens")
    })?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let (name, pointer) = match value {
                Value::String(name) => (name.as_str(), format!("/proof/goldens/{index}")),
                Value::Object(object) => {
                    let field = ["path", "expected", "name"]
                        .into_iter()
                        .find_map(|field| object.get(field).and_then(Value::as_str))
                        .ok_or_else(|| {
                            CliError::invalid_args(format!(
                                "proof.goldens[{index}] requires path, expected, or name"
                            ))
                            .with_pointer(format!("/proof/goldens/{index}"))
                        })?;
                    (field, format!("/proof/goldens/{index}"))
                }
                _ => {
                    return Err(CliError::invalid_args(format!(
                        "proof.goldens[{index}] must be a string or object"
                    ))
                    .with_pointer(format!("/proof/goldens/{index}")));
                }
            };
            if name.trim().is_empty() {
                return Err(CliError::invalid_args(format!(
                    "proof.goldens[{index}] must not be empty"
                ))
                .with_pointer(pointer));
            }
            Ok(golden_path(name))
        })
        .collect()
}

fn golden_path(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.ends_with(".summary.json") || trimmed.ends_with(".json") {
        trimmed.to_string()
    } else {
        format!("testdata/golden/{trimmed}.summary.json")
    }
}

fn is_desktop_dependent(command: &str) -> bool {
    command.starts_with("powerbi-cli desktop ")
        || command.starts_with("powerbi-cli model dax execute ")
}

fn unavailable_reason(command: &str) -> (&'static str, &'static str) {
    if !cfg!(windows) {
        return ("platform_non_windows", WINDOWS_ORACLE_INSTRUCTION);
    }
    if command.contains("refresh-check") || command.contains("canvas-check") {
        return ("missing_desktop_oracle_command", WINDOWS_T9_INSTRUCTION);
    }
    ("missing_desktop", WINDOWS_ORACLE_INSTRUCTION)
}

fn requested_level_reason() -> (&'static str, &'static str) {
    if !cfg!(windows) {
        ("platform_non_windows", WINDOWS_ORACLE_INSTRUCTION)
    } else {
        ("missing_desktop", WINDOWS_ORACLE_INSTRUCTION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan(spec: Value) -> ProofPlan {
        compile_proof_plan(Some(&spec), None)
            .expect("proof plan")
            .expect("proof block")
    }

    #[test]
    fn proof_plan_expands_expectations_and_goldens_without_execution() {
        let plan = plan(json!({
            "proof": {
                "desktop": {
                    "level": "desktop-canvas-refresh",
                    "pages": ["overview"],
                    "expectValues": [{"query": "EVALUATE ROW(\"x\", 1)", "expected": 1}]
                },
                "goldens": ["sales"]
            }
        }));
        assert_eq!(plan.value["requestedLevel"], "desktop-canvas-refresh");
        assert_eq!(
            plan.value["achievableHere"],
            if cfg!(windows) {
                "desktop-golden-pending"
            } else {
                "schema-golden"
            }
        );
        let commands = plan.value["commands"].as_array().expect("commands");
        assert!(commands.iter().any(|command| {
            command
                .as_str()
                .is_some_and(|command| command.contains("model dax execute"))
        }));
        assert!(commands.iter().any(|command| {
            command.as_str().is_some_and(|command| {
                command.contains("fixture verify")
                    && command.contains("testdata/golden/sales.summary.json")
            })
        }));
        assert_eq!(plan.next.len(), commands.len());
        assert!(
            plan.value["unavailable"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
    }

    #[test]
    fn proof_plan_rejects_unknown_level_with_pointer() {
        let error = compile_proof_plan(
            Some(&json!({"proof": {"desktop": {"level": "imaginary"}}})),
            None,
        )
        .expect_err("unknown level must fail");
        assert_eq!(error.code, "invalid_args");
        assert_eq!(error.pointer(), Some("/proof/desktop/level"));
    }

    #[test]
    fn proof_plan_rejects_malformed_inputs_with_precise_pointers() {
        let cases = [
            (json!({"proof": true}), "/proof"),
            (
                json!({"proof": {"desktop": {"pages": [""]}}}),
                "/proof/desktop/pages/0",
            ),
            (
                json!({"proof": {"desktop": {"expectValues": [{"expected": 1}]}}}),
                "/proof/desktop/expectValues/0/query",
            ),
            (json!({"proof": {"goldens": [{}]}}), "/proof/goldens/0"),
        ];
        for (spec, pointer) in cases {
            let error =
                compile_proof_plan(Some(&spec), None).expect_err("malformed proof input must fail");
            assert_eq!(error.code, "invalid_args");
            assert_eq!(error.pointer(), Some(pointer));
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_snapshot_marks_desktop_commands_as_platform_unavailable() {
        let plan = plan(json!({
            "proof": {"desktop": {"level": "desktop-golden-pending"}}
        }));
        assert_eq!(plan.value["achievableHere"], "schema-golden");
        assert!(
            plan.value["unavailable"]
                .as_array()
                .expect("unavailable")
                .iter()
                .any(|item| { item["why"] == "platform_non_windows" })
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_snapshot_marks_desktop_runtime_as_unavailable_until_explicitly_run() {
        let plan = plan(json!({
            "proof": {"desktop": {"level": "desktop-golden-pending"}}
        }));
        assert_eq!(plan.value["achievableHere"], "desktop-golden-pending");
        assert!(
            plan.value["unavailable"]
                .as_array()
                .expect("unavailable")
                .iter()
                .any(|item| { item["why"] == "missing_desktop" })
        );
    }
}
