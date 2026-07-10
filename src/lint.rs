use crate::inspect::deep_inspect;
use crate::model_dax::{add_cycle_findings, analyze_dax};
use crate::tmdl::load_table_documents;
use crate::{
    CliError, CliResult, ResolvedProject, ValidationReport, canonical_display, command_arg,
    read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

const DESKTOP_ROUND_TRIP_REPORT_VERSION: &str = "2.0.0";

pub(crate) fn lint_command(args: &[String]) -> CliResult<Value> {
    let path = parse_lint_args(args)?;
    let resolved = resolve_project(&path)?;
    let validation = validate_project(&resolved)?;
    lint_project(&resolved, &validation)
}

pub(crate) fn lint_project(
    resolved: &ResolvedProject,
    validation: &ValidationReport,
) -> CliResult<Value> {
    let deep = deep_inspect(resolved, validation)?;
    let mut findings = Vec::new();
    add_validation_findings(validation, &mut findings);
    add_pbir_metadata_findings(resolved, &mut findings)?;
    add_report_findings(&deep, &mut findings);
    add_model_findings(&deep, &mut findings);
    add_dax_findings(resolved, &mut findings)?;

    let error_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count();
    let warning_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "warning")
        .count();
    let info_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "info")
        .count();

    Ok(json!({
        "schema": "powerbi-cli.lint.v1",
        "ok": error_count == 0,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "counts": {
            "errors": error_count,
            "warnings": warning_count,
            "info": info_count,
            "findings": findings.len()
        },
        "findings": findings,
        "next": [
            format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn parse_lint_args(args: &[String]) -> CliResult<PathBuf> {
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown lint flag: {other}"))
                        .with_hint("Run `powerbi-cli lint <project-dir-or.pbip> --json`.")
                        .with_suggested_command("powerbi-cli lint <project-dir-or.pbip> --json"),
                );
            }
            other => {
                if path.is_some() {
                    return Err(CliError::invalid_args("lint accepts exactly one path")
                        .with_hint("Run `powerbi-cli lint <project-dir-or.pbip> --json`.")
                        .with_suggested_command("powerbi-cli lint <project-dir-or.pbip> --json"));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    path.ok_or_else(|| {
        CliError::invalid_args("lint requires a path")
            .with_hint("Run `powerbi-cli lint <project-dir-or.pbip> --json`.")
            .with_suggested_command("powerbi-cli lint <project-dir-or.pbip> --json")
    })
}

fn add_validation_findings(validation: &ValidationReport, findings: &mut Vec<Value>) {
    for message in &validation.errors {
        findings.push(finding(
            "validation.structure",
            "error",
            message,
            None,
            None,
        ));
    }
    for message in &validation.warnings {
        findings.push(finding(
            "validation.warning",
            "warning",
            message,
            None,
            None,
        ));
    }
}

fn add_pbir_metadata_findings(
    resolved: &ResolvedProject,
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    let version_path = resolved.report_dir.join("definition").join("version.json");
    if !version_path.is_file() {
        return Ok(());
    }
    let version_json = read_json_value(&version_path)?;
    let version = version_json["version"].as_str().unwrap_or_default();
    if version != DESKTOP_ROUND_TRIP_REPORT_VERSION {
        let path = canonical_display(&version_path);
        findings.push(finding(
            "pbir.report_definition_version",
            "error",
            &format!(
                "PBIR report definition version {version:?} is not Desktop round-trip proven; expected {DESKTOP_ROUND_TRIP_REPORT_VERSION}"
            ),
            None,
            Some(path.as_str()),
        ));
    }
    Ok(())
}

fn add_report_findings(deep: &Value, findings: &mut Vec<Value>) {
    if let Some(pages) = deep["report"]["pages"].as_array() {
        let mut page_title_counts = BTreeMap::<String, usize>::new();
        for page in pages {
            let title = normalized_label(page["displayName"].as_str().unwrap_or_default());
            if !title.is_empty() {
                *page_title_counts.entry(title).or_default() += 1;
            }
        }
        for page in pages {
            let page_handle = page["handle"].as_str();
            let page_name = page["displayName"].as_str().unwrap_or("page");
            let normalized_page_name = normalized_label(page_name);
            if !normalized_page_name.is_empty()
                && page_title_counts
                    .get(&normalized_page_name)
                    .copied()
                    .unwrap_or_default()
                    > 1
            {
                findings.push(finding(
                    "bpa.report.duplicate_page_title",
                    "warning",
                    &format!("multiple pages share display name: {page_name}"),
                    page_handle,
                    page["path"].as_str(),
                ));
            }
            let page_width = page["width"].as_f64().unwrap_or(0.0);
            let page_height = page["height"].as_f64().unwrap_or(0.0);
            let visuals = page["visuals"].as_array().cloned().unwrap_or_default();
            let mut visual_title_counts = BTreeMap::<String, usize>::new();
            for visual in &visuals {
                let title = normalized_label(visual["title"].as_str().unwrap_or_default());
                if !title.is_empty() {
                    *visual_title_counts.entry(title).or_default() += 1;
                }
            }
            if visuals.is_empty() {
                findings.push(finding(
                    "report.page_empty",
                    "warning",
                    &format!("page has no visuals: {page_name}"),
                    page_handle,
                    None,
                ));
            }
            for visual in visuals {
                let visual_handle = visual["handle"].as_str();
                let title = visual["title"].as_str().unwrap_or_default();
                if title.trim().is_empty() {
                    findings.push(finding(
                        "report.visual_missing_title",
                        "warning",
                        "visual is missing a title",
                        visual_handle,
                        visual["path"].as_str(),
                    ));
                } else if visual_title_counts
                    .get(&normalized_label(title))
                    .copied()
                    .unwrap_or_default()
                    > 1
                {
                    findings.push(finding(
                        "bpa.report.duplicate_visual_title",
                        "warning",
                        &format!("multiple visuals on page `{page_name}` share title: {title}"),
                        visual_handle,
                        visual["path"].as_str(),
                    ));
                }
                if visual["bindings"].as_array().is_some_and(Vec::is_empty) {
                    findings.push(finding(
                        "report.visual_unbound",
                        "info",
                        &format!("visual has no field bindings: {title}"),
                        visual_handle,
                        visual["path"].as_str(),
                    ));
                }
                if visual_missing_alt_text(&visual) {
                    findings.push(finding(
                        "bpa.report.visual_missing_alt_text",
                        "warning",
                        &format!("visual is missing alt text: {title}"),
                        visual_handle,
                        visual["path"].as_str(),
                    ));
                }
                if visual_outside_page(&visual, page_width, page_height) {
                    findings.push(finding(
                        "report.visual_outside_page",
                        "warning",
                        &format!("visual is outside page bounds: {title}"),
                        visual_handle,
                        visual["path"].as_str(),
                    ));
                }
            }
        }
    }
}

fn add_model_findings(deep: &Value, findings: &mut Vec<Value>) {
    if let Some(tables) = deep["model"]["tables"].as_array() {
        for table in tables {
            let table_handle = table["handle"].as_str();
            let table_name = table["name"].as_str().unwrap_or("table");
            let path = table["path"].as_str();
            if table["columns"].as_array().is_some_and(Vec::is_empty) {
                findings.push(finding(
                    "model.table_without_columns",
                    "error",
                    &format!("table has no columns: {table_name}"),
                    table_handle,
                    path,
                ));
            }
            if table["partitions"].as_array().is_some_and(Vec::is_empty) {
                findings.push(finding(
                    "model.table_without_partition",
                    "warning",
                    &format!("table has no partition: {table_name}"),
                    table_handle,
                    path,
                ));
            }
        }
    }
}

fn add_dax_findings(resolved: &ResolvedProject, findings: &mut Vec<Value>) -> CliResult<()> {
    let docs = match load_table_documents(resolved) {
        Ok(docs) => docs,
        Err(err) if err.code == "file_not_found" => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut analysis = analyze_dax(&docs);
    add_cycle_findings(&mut analysis);
    for finding in analysis.findings {
        findings.push(finding);
    }
    Ok(())
}

fn visual_outside_page(visual: &Value, page_width: f64, page_height: f64) -> bool {
    let position = &visual["position"];
    let x = position["x"].as_f64().unwrap_or(0.0);
    let y = position["y"].as_f64().unwrap_or(0.0);
    let width = position["width"].as_f64().unwrap_or(0.0);
    let height = position["height"].as_f64().unwrap_or(0.0);
    x < 0.0 || y < 0.0 || x + width > page_width || y + height > page_height
}

fn visual_missing_alt_text(visual: &Value) -> bool {
    let Some(path) = visual["path"].as_str() else {
        return false;
    };
    let Ok(raw) = read_json_value(PathBuf::from(path).as_path()) else {
        return false;
    };
    let value = raw
        .pointer("/visual/objects/general/0/properties/altText/expr/Literal/Value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    value.trim().is_empty()
}

fn normalized_label(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn finding(
    code: &str,
    severity: &str,
    message: &str,
    handle: Option<&str>,
    path: Option<&str>,
) -> Value {
    json!({
        "code": code,
        "severity": severity,
        "message": message,
        "handle": handle,
        "path": path
    })
}
