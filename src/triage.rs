use crate::cli_support::shell_arg;
use crate::lint::lint_project;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, ValidationReport,
    canonical_display, command_arg, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(crate) fn triage_command(args: &[String]) -> CliResult<Value> {
    let path = parse_triage_args(args)?;
    let resolved = resolve_project(&path)?;
    let validation = match validate_project(&resolved) {
        Ok(report) => report,
        // A project too broken to parse still deserves a triage document: fold the
        // hard failure into the validation errors instead of aborting the readback.
        Err(error) if error.exit_code == EXIT_VALIDATION_FAILED => ValidationReport {
            errors: vec![error.message],
            warnings: Vec::new(),
            json_files_checked: 0,
            pages: 0,
            visuals: 0,
            bound_visuals: 0,
            tables: 0,
            measures: 0,
            relationships: 0,
        },
        Err(error) => return Err(error),
    };
    let mut lint = match lint_project(&resolved, &validation) {
        Ok(lint) => lint,
        Err(_) if !validation.errors.is_empty() => {
            json!({ "ok": false, "counts": { "findings": 0 }, "findings": [] })
        }
        Err(error) => return Err(error),
    };
    if let Some(findings) = lint.get_mut("findings").and_then(Value::as_array_mut) {
        sort_findings(findings);
    }
    let validation_ok = validation.errors.is_empty();
    let lint_ok = lint["ok"].as_bool().unwrap_or(false);
    let ok = validation_ok && lint_ok;
    let findings = lint["findings"].as_array().cloned().unwrap_or_default();
    let top_findings = top_findings(&findings);
    let next = next_commands(&resolved, &validation, &findings, ok);
    Ok(json!({
        "schema": "triageResult.v1",
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "validation": {
            "ok": validation_ok,
            "strict": true,
            "counts": {
                "jsonFilesChecked": validation.json_files_checked,
                "pages": validation.pages,
                "visuals": validation.visuals,
                "boundVisuals": validation.bound_visuals,
                "tables": validation.tables,
                "measures": validation.measures,
                "relationships": validation.relationships
            },
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "lint": {
            "ok": lint_ok,
            "counts": lint["counts"],
            "findings": findings
        },
        "topFindings": top_findings,
        "next": next
    }))
}

fn parse_triage_args(args: &[String]) -> CliResult<PathBuf> {
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown triage flag: {other}"))
                        .with_hint("Run `powerbi-cli triage <project-dir-or.pbip> --json`.")
                        .with_suggested_command("powerbi-cli triage <project-dir-or.pbip> --json"),
                );
            }
            other => {
                if path.is_some() {
                    return Err(CliError::invalid_args("triage accepts exactly one path")
                        .with_hint("Run `powerbi-cli triage <project-dir-or.pbip> --json`.")
                        .with_suggested_command(
                            "powerbi-cli triage <project-dir-or.pbip> --json",
                        ));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    path.ok_or_else(|| {
        CliError::invalid_args("triage requires a path")
            .with_hint("Run `powerbi-cli triage <project-dir-or.pbip> --json`.")
            .with_suggested_command("powerbi-cli triage <project-dir-or.pbip> --json")
    })
}

fn sort_findings(findings: &mut [Value]) {
    findings.sort_by(|left, right| {
        severity_rank(left)
            .cmp(&severity_rank(right))
            .then_with(|| finding_path(left).cmp(finding_path(right)))
            .then_with(|| {
                left["code"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["code"].as_str().unwrap_or_default())
            })
            .then_with(|| {
                left["message"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["message"].as_str().unwrap_or_default())
            })
    });
}

fn severity_rank(finding: &Value) -> u8 {
    match finding["severity"].as_str() {
        Some("error") => 0,
        Some("warning") => 1,
        Some("info") => 2,
        _ => 3,
    }
}

fn finding_path(finding: &Value) -> &str {
    finding["path"].as_str().unwrap_or_default()
}

fn top_findings(findings: &[Value]) -> Vec<Value> {
    findings
        .iter()
        .filter(|finding| {
            matches!(
                finding["severity"].as_str(),
                Some("error") | Some("warning")
            ) && is_table_producing_finding(finding)
        })
        .take(10)
        .cloned()
        .collect()
}

fn is_table_producing_finding(finding: &Value) -> bool {
    match finding["stepKind"].as_str() {
        Some("other" | "tableLiteral" | "navigation") => true,
        Some(_) => false,
        None => true,
    }
}

fn next_commands(
    resolved: &ResolvedProject,
    validation: &ValidationReport,
    findings: &[Value],
    all_green: bool,
) -> Vec<String> {
    let project = command_arg(&resolved.project_dir);
    if let Some(error) = validation.errors.first() {
        return validation_next(&project, error);
    }
    if findings.iter().any(|finding| {
        matches!(
            finding["severity"].as_str(),
            Some("error") | Some("warning")
        )
    }) {
        return lint_next(&project, findings);
    }
    if all_green {
        return vec![
            format!("powerbi-cli inspect --deep {project} --json"),
            format!("powerbi-cli report design-plan --project {project} --json"),
            format!("powerbi-cli desktop open {project} --json"),
        ];
    }
    vec![format!("powerbi-cli inspect --deep {project} --json")]
}

fn validation_next(project: &str, error: &str) -> Vec<String> {
    let mut next = Vec::new();
    if let Some((page, visual)) = extract_visual_names(error) {
        next.push(format!(
            "powerbi-cli report visuals show --project {project} --handle {} --json",
            shell_arg(&format!("visual:{page}:{visual}"))
        ));
    } else if let Some(page) = extract_page_name(error) {
        next.push(format!(
            "powerbi-cli report pages show --project {project} --handle {} --json",
            shell_arg(&format!("page:{page}"))
        ));
    }
    next.push(format!("powerbi-cli inspect --deep {project} --json"));
    next.push(format!("powerbi-cli validate --strict {project} --json"));
    next
}

fn lint_next(project: &str, findings: &[Value]) -> Vec<String> {
    let mut next = vec![format!("powerbi-cli lint {project} --json")];
    if let Some(finding) = findings.iter().find(|finding| {
        matches!(
            finding["severity"].as_str(),
            Some("error") | Some("warning")
        )
    }) && let Some(handle) = finding["handle"].as_str()
        && let Some(command) = handle_readback(project, handle)
    {
        next.push(command);
    }
    next.push(format!("powerbi-cli inspect --deep {project} --json"));
    next
}

fn handle_readback(project: &str, handle: &str) -> Option<String> {
    let handle = shell_arg(handle);
    if handle.starts_with("visual:") {
        return Some(format!(
            "powerbi-cli report visuals show --project {project} --handle {handle} --json"
        ));
    }
    if handle.starts_with("page:") {
        return Some(format!(
            "powerbi-cli report pages show --project {project} --handle {handle} --json"
        ));
    }
    if handle.starts_with("partition:") {
        return Some(format!(
            "powerbi-cli model partitions show --project {project} --handle {handle} --include-source --json"
        ));
    }
    None
}

fn extract_visual_names(error: &str) -> Option<(String, String)> {
    let normalized = error.replace('\\', "/");
    let marker = "/pages/";
    let pages = normalized.find(marker)?;
    let after_pages = &normalized[pages + marker.len()..];
    let visuals_marker = "/visuals/";
    let visuals = after_pages.find(visuals_marker)?;
    let page = &after_pages[..visuals];
    let after_visuals = &after_pages[visuals + visuals_marker.len()..];
    let visual = after_visuals.split('/').next().unwrap_or_default();
    if page.is_empty() || visual.is_empty() {
        return None;
    }
    Some((page.to_string(), visual.to_string()))
}

fn extract_page_name(error: &str) -> Option<String> {
    let normalized = error.replace('\\', "/");
    let marker = "/pages/";
    let pages = normalized.find(marker)?;
    let after_pages = &normalized[pages + marker.len()..];
    let segment = after_pages.split('/').next().unwrap_or_default();
    // Error text may continue after the path (": key must be a string …");
    // keep only the path segment so prose never leaks into a suggested command.
    let page = segment
        .split([':', ' '])
        .next()
        .unwrap_or_default()
        .trim_end_matches(|c: char| !c.is_alphanumeric());
    if page.is_empty() || page == "pages" || segment.starts_with("pages.json") {
        return None;
    }
    Some(page.to_string())
}
