use crate::input_safety::{InputKind, read_utf8};
use crate::inspect::deep_inspect;
use crate::model_dax::{add_cycle_findings, add_model_completeness_findings, analyze_dax};
use crate::rules;
use crate::tmdl::load_table_documents;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, ResolvedProject, ValidationReport, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[path = "m_lint.rs"]
mod m_lint;

const DESKTOP_ROUND_TRIP_REPORT_VERSION: &str = "2.0.0";

pub(crate) fn lint_command(args: &[String]) -> CliResult<Value> {
    rules::validate_registry().map_err(CliError::unexpected)?;
    match parse_lint_args(args)? {
        LintRequest::Project(path) => {
            let resolved = resolve_project(&path)?;
            let validation = validate_project(&resolved)?;
            lint_project(&resolved, &validation)
        }
        LintRequest::Rules => Ok(json!({
            "schema": "powerbi-cli.lint.rules.v1",
            "ok": true,
            "exitCode": EXIT_SUCCESS,
            "count": rules::all_rules().len(),
            "families": rules::rule_family_names(),
            "rules": rules::rule_definitions_json(),
            "next": ["powerbi-cli lint --explain <rule-id> --json"]
        })),
        LintRequest::Explain(id) => {
            let rule = rules::find_rule(&id).ok_or_else(|| {
                CliError::invalid_args(format!("unknown lint rule id: {id}"))
                    .with_hint("Run `powerbi-cli lint --rules --json` to list registered rule ids.")
                    .with_suggested_command("powerbi-cli lint --rules --json")
            })?;
            Ok(json!({
                "schema": "powerbi-cli.lint.ruleExplanation.v1",
                "ok": true,
                "exitCode": EXIT_SUCCESS,
                "rule": rule.to_json(),
                "exampleFinding": rule.example_finding(),
                "next": ["powerbi-cli lint --rules --json"]
            }))
        }
    }
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
    add_dax_findings(resolved, &deep, &mut findings)?;
    findings.extend(m_lint::buffer_reuse_findings(resolved)?);
    add_desktop_compat_findings(resolved, &mut findings)?;
    rules::ensure_finding_ids_registered(&findings, "code")?;

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

#[derive(Debug, PartialEq, Eq)]
enum LintRequest {
    Project(PathBuf),
    Rules,
    Explain(String),
}

fn parse_lint_args(args: &[String]) -> CliResult<LintRequest> {
    let mut path = None;
    let mut request = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--rules" => {
                if request.is_some() {
                    return Err(lint_mode_conflict());
                }
                request = Some(LintRequest::Rules);
                index += 1;
            }
            "--explain" => {
                if request.is_some() {
                    return Err(lint_mode_conflict());
                }
                let id = args.get(index + 1).ok_or_else(|| {
                    CliError::invalid_args("--explain requires a rule id")
                        .with_hint(
                            "Run `powerbi-cli lint --rules --json` to list registered rule ids.",
                        )
                        .with_suggested_command("powerbi-cli lint --rules --json")
                })?;
                if id.starts_with('-') {
                    return Err(CliError::invalid_args("--explain requires a rule id")
                        .with_hint(
                            "Run `powerbi-cli lint --rules --json` to list registered rule ids.",
                        )
                        .with_suggested_command("powerbi-cli lint --rules --json"));
                }
                request = Some(LintRequest::Explain(id.clone()));
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown lint flag: {other}"))
                        .with_hint(
                            "Run `powerbi-cli --json capabilities --for lint` for exact usage.",
                        )
                        .with_suggested_command("powerbi-cli --json capabilities --for lint"),
                );
            }
            other => {
                if path.is_some() || request.is_some() {
                    return Err(CliError::invalid_args("lint accepts exactly one path")
                        .with_hint(
                            "Pass one project path, or use one of --rules and --explain <rule-id>.",
                        )
                        .with_suggested_command("powerbi-cli --json capabilities --for lint"));
                }
                path = Some(PathBuf::from(other));
                index += 1;
            }
        }
    }
    match (path, request) {
        (Some(path), None) => Ok(LintRequest::Project(path)),
        (None, Some(request)) => Ok(request),
        (Some(_), Some(_)) => Err(lint_mode_conflict()),
        (None, None) => Err(CliError::invalid_args(
            "lint requires a path, --rules, or --explain <rule-id>",
        )
        .with_hint("Run `powerbi-cli --json capabilities --for lint` for exact usage.")
        .with_suggested_command("powerbi-cli --json capabilities --for lint")),
    }
}

fn lint_mode_conflict() -> CliError {
    CliError::invalid_args("lint accepts one mode: a project path, --rules, or --explain <rule-id>")
        .with_hint("Choose exactly one lint operation.")
        .with_suggested_command("powerbi-cli --json capabilities --for lint")
}

fn add_validation_findings(validation: &ValidationReport, findings: &mut Vec<Value>) {
    for diagnostic in &validation.errors {
        findings.push(json!({
            "code": diagnostic.code,
            "severity": diagnostic.severity,
            "message": diagnostic.message,
            "handle": Value::Null,
            "path": diagnostic.path,
            "pointer": diagnostic.pointer
        }));
    }
    for diagnostic in &validation.warnings {
        findings.push(json!({
            "code": diagnostic.code,
            "severity": diagnostic.severity,
            "message": diagnostic.message,
            "handle": Value::Null,
            "path": diagnostic.path,
            "pointer": diagnostic.pointer
        }));
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
            rules::PBIR_REPORT_DEFINITION_VERSION,
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
                    rules::BPA_REPORT_DUPLICATE_PAGE_TITLE,
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
                    rules::REPORT_PAGE_EMPTY,
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
                        rules::REPORT_VISUAL_MISSING_TITLE,
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
                        rules::BPA_REPORT_DUPLICATE_VISUAL_TITLE,
                        "warning",
                        &format!("multiple visuals on page `{page_name}` share title: {title}"),
                        visual_handle,
                        visual["path"].as_str(),
                    ));
                }
                if visual["bindings"].as_array().is_some_and(Vec::is_empty) {
                    findings.push(finding(
                        rules::REPORT_VISUAL_UNBOUND,
                        "info",
                        &format!("visual has no field bindings: {title}"),
                        visual_handle,
                        visual["path"].as_str(),
                    ));
                }
                match visual_alt_text_status(&visual) {
                    // Microsoft powerbi-report-authoring-cli v0.1.4 rejects both
                    // known general.altText placements. Absence is valid PBIR.
                    VisualAltTextStatus::Missing => {}
                    VisualAltTextStatus::VisualObjects => findings.push(finding(
                        rules::PBIR_VISUAL_ALT_TEXT_LEGACY_LOCATION,
                        "warning",
                        &format!(
                            "visual contains validator-rejected alt text under visual.objects.general: {title}; remove it with report visuals formatting set-text --clear-alt-text"
                        ),
                        visual_handle,
                        visual["path"].as_str(),
                    )),
                    VisualAltTextStatus::VisualContainerObjects | VisualAltTextStatus::Both => {
                        findings.push(finding(
                            rules::PBIR_VISUAL_ALT_TEXT_UNSUPPORTED_LOCATION,
                            "warning",
                            &format!(
                                "visual contains validator-rejected alt text under visual.visualContainerObjects.general: {title}; remove it with report visuals formatting set-text --clear-alt-text"
                            ),
                            visual_handle,
                            visual["path"].as_str(),
                        ));
                    }
                }
                if visual_outside_page(&visual, page_width, page_height) {
                    findings.push(finding(
                        rules::REPORT_VISUAL_OUTSIDE_PAGE,
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
            // A calculated-table partition can be authored with only its DAX
            // source. Desktop materializes the resulting columns on refresh,
            // so defer the generic no-columns error for this supported shape.
            let schema_deferred_to_desktop =
                table["partitions"].as_array().is_some_and(|partitions| {
                    partitions.iter().any(|partition| {
                        partition["expressionKind"]
                            .as_str()
                            .is_some_and(|kind| kind.eq_ignore_ascii_case("calculated"))
                    })
                });
            if table["columns"].as_array().is_some_and(Vec::is_empty) && !schema_deferred_to_desktop
            {
                findings.push(finding(
                    rules::MODEL_TABLE_WITHOUT_COLUMNS,
                    "error",
                    &format!("table has no columns: {table_name}"),
                    table_handle,
                    path,
                ));
            }
            if table["partitions"].as_array().is_some_and(Vec::is_empty) {
                findings.push(finding(
                    rules::MODEL_TABLE_WITHOUT_PARTITION,
                    "warning",
                    &format!("table has no partition: {table_name}"),
                    table_handle,
                    path,
                ));
            }
        }
    }
}

fn add_dax_findings(
    resolved: &ResolvedProject,
    deep: &Value,
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    let docs = match load_table_documents(resolved) {
        Ok(docs) => docs,
        Err(err) if err.code == "file_not_found" => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut analysis = analyze_dax(&docs);
    add_cycle_findings(&mut analysis);
    // Model completeness uses the same static DAX graph so columns referenced
    // by a measure are not reported as unused. DAX-only lint remains scoped to
    // the analysis findings returned by `model dax lint`.
    add_model_completeness_findings(deep, &analysis, findings);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualAltTextStatus {
    VisualContainerObjects,
    VisualObjects,
    Both,
    Missing,
}

fn visual_alt_text_status(visual: &Value) -> VisualAltTextStatus {
    let Some(path) = visual["path"].as_str() else {
        return VisualAltTextStatus::Missing;
    };
    let Ok(raw) = read_json_value(PathBuf::from(path).as_path()) else {
        return VisualAltTextStatus::Missing;
    };
    let visual_container_objects = raw
        .pointer("/visual/visualContainerObjects/general/0/properties/altText")
        .is_some();
    let visual_objects = raw
        .pointer("/visual/objects/general/0/properties/altText")
        .is_some();
    match (visual_container_objects, visual_objects) {
        (true, true) => VisualAltTextStatus::Both,
        (true, false) => VisualAltTextStatus::VisualContainerObjects,
        (false, true) => VisualAltTextStatus::VisualObjects,
        (false, false) => VisualAltTextStatus::Missing,
    }
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

/// Desktop Store 2.156 refuses to open a PBIP whose TMDL carries a comment
/// directly above a `relationship` declaration: `///` doc comments compile to
/// a `description` property that relationships do not have in TOM, and plain
/// `//` comments fail the same way. The dialog reads "Property 'description'
/// is unknown and is not expected in the situation it appears" and Desktop
/// falls back to an empty Untitled session. Newer Desktop builds tolerate the
/// comment, which makes this a silent cross-version trap the oracle only
/// reveals on the older machine.
fn add_desktop_compat_findings(
    resolved: &ResolvedProject,
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    let definition_dir = resolved.semantic_model_dir.join("definition");
    let mut tmdl_paths = vec![
        definition_dir.join("model.tmdl"),
        definition_dir.join("relationships.tmdl"),
    ];
    let tables_dir = definition_dir.join("tables");
    if tables_dir.is_dir() {
        let mut table_paths = fs::read_dir(&tables_dir)
            .map_err(|err| CliError::unexpected(format!("read {}: {err}", tables_dir.display())))?
            .map(|entry| crate::read_dir_entry(&tables_dir, entry, "lint table TMDL"))
            .collect::<CliResult<Vec<_>>>()?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("tmdl"))
            .collect::<Vec<_>>();
        table_paths.sort();
        tmdl_paths.extend(table_paths);
    }
    for path in tmdl_paths {
        if !path.is_file() {
            continue;
        }
        let text = read_utf8(&path, InputKind::ProjectText)?;
        findings.extend(relationship_comment_findings(
            &text,
            &canonical_display(&path),
        ));
    }
    for platform_path in [
        resolved.report_dir.join(".platform"),
        resolved.semantic_model_dir.join(".platform"),
    ] {
        if !platform_path.is_file() {
            continue;
        }
        let value = read_json_value(&platform_path)?;
        findings.extend(platform_metadata_findings(
            &value,
            &canonical_display(&platform_path),
        ));
    }
    Ok(())
}

fn relationship_comment_findings(text: &str, path: &str) -> Vec<Value> {
    let lines: Vec<&str> = text.lines().collect();
    let mut findings = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(rest) = line.trim_start().strip_prefix("relationship ") else {
            continue;
        };
        let name = rest.trim();
        // Property assignments inside M or DAX bodies can start with the same
        // word; a declaration's remainder is a bare object name.
        if name.is_empty() || name.contains('=') || name.contains(':') {
            continue;
        }
        let Some(previous) = lines[..index]
            .iter()
            .rev()
            .find(|prior| !prior.trim().is_empty())
        else {
            continue;
        };
        if previous.trim_start().starts_with("//") {
            findings.push(finding(
                rules::MODEL_RELATIONSHIP_COMMENT_UNSUPPORTED,
                "error",
                &format!(
                    "comment above relationship '{name}': relationships have no description in TOM, so Power BI Desktop 2.156 refuses to open the project (\"Property 'description' is unknown and is not expected in the situation it appears\"); delete the comment lines and keep the prose in the commit message"
                ),
                Some(&format!("relationship:{name}")),
                Some(path),
            ));
        }
    }
    findings
}

fn platform_metadata_findings(value: &Value, path: &str) -> Vec<Value> {
    const KNOWN_METADATA_PROPERTIES: [&str; 2] = ["type", "displayName"];
    let mut findings = Vec::new();
    if let Some(metadata) = value["metadata"].as_object() {
        for key in metadata.keys() {
            if !KNOWN_METADATA_PROPERTIES.contains(&key.as_str()) {
                findings.push(finding(
                    rules::PLATFORM_UNKNOWN_METADATA_PROPERTY,
                    "warning",
                    &format!(
                        "unknown .platform metadata property '{key}': the Fabric platformProperties 2.0.0 schema defines only type and displayName, and unknown properties risk Desktop-version rejection"
                    ),
                    None,
                    Some(path),
                ));
            }
        }
    }
    findings
}

#[cfg(test)]
mod desktop_compat_tests {
    use super::{platform_metadata_findings, relationship_comment_findings};
    use serde_json::json;

    #[test]
    fn doc_comment_above_relationship_is_an_error() {
        let text = "/// Many-to-many on the canton code.\nrelationship relAgenturKanton\n\tfromColumn: A.K\n\ttoColumn: B.K\n";
        let findings = relationship_comment_findings(text, "relationships.tmdl");
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0]["code"],
            "model.relationship_comment_unsupported"
        );
        assert_eq!(findings[0]["severity"], "error");
        assert_eq!(findings[0]["handle"], "relationship:relAgenturKanton");
    }

    #[test]
    fn plain_comment_above_relationship_is_flagged_even_across_a_blank_line() {
        let text =
            "// explains the join\n\nrelationship relX\n\tfromColumn: A.K\n\ttoColumn: B.K\n";
        let findings = relationship_comment_findings(text, "relationships.tmdl");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn comments_on_supported_objects_and_property_lines_pass() {
        let text = "/// Table docs are legal TOM descriptions.\ntable DimJahr\n\n\tcolumn Jahr\n\t\tdataType: int64\n\nrelationship relClean\n\tfromColumn: A.K\n\ttoColumn: B.K\n\npartition p = m\n\tsource =\n\t\tlet\n\t\t\trelationship = 1\n\t\tin\n\t\t\trelationship\n";
        assert!(relationship_comment_findings(text, "tables/DimJahr.tmdl").is_empty());
    }

    #[test]
    fn platform_description_is_a_warning_and_known_keys_pass() {
        let with_description = json!({
            "metadata": {"type": "Report", "displayName": "Contoso", "description": "x"}
        });
        let findings = platform_metadata_findings(&with_description, ".platform");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["code"], "platform.unknown_metadata_property");
        assert_eq!(findings[0]["severity"], "warning");

        let clean = json!({"metadata": {"type": "Report", "displayName": "Contoso"}});
        assert!(platform_metadata_findings(&clean, ".platform").is_empty());
    }
}
