use crate::partitions::partition_summary_json;
use crate::rebind_plan::rebind_plan;
use crate::safety_scan::{contains_credential_like_text_str, contains_pii_suspect_text};
use crate::source_templates::{
    load_source_template_store, source_template_findings, source_templates_path,
};
use crate::tmdl::{PartitionRecord, load_table_documents, table_handle};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

pub(crate) fn handoff_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args("handoff requires a subcommand: check, rebind-plan")
                .with_hint("Run `powerbi-cli handoff check <project-dir-or.pbip> --json`.")
                .with_suggested_command("powerbi-cli handoff check <project-dir-or.pbip> --json"),
        );
    };

    match action.as_str() {
        "check" => check_handoff(rest),
        "rebind" | "rebind-plan" => rebind_plan(rest),
        _ => Err(
            CliError::invalid_args(format!("unknown handoff command: {action}"))
                .with_hint("Run `powerbi-cli handoff check <project-dir-or.pbip> --json` or `powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json`.")
                .with_suggested_command("powerbi-cli handoff check <project-dir-or.pbip> --json")
                .with_suggested_command("powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json"),
        ),
    }
}

#[derive(Debug, Default)]
struct CheckOptions {
    project: Option<PathBuf>,
}

fn check_handoff(args: &[String]) -> CliResult<Value> {
    let options = parse_check_args(args)?;
    let project = options.project.ok_or_else(|| {
        CliError::invalid_args("handoff check requires <project-dir-or.pbip> or --project")
            .with_hint("Pass the PBIP project directory or the .pbip file to check.")
            .with_suggested_command("powerbi-cli handoff check <project-dir-or.pbip> --json")
    })?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let docs = load_table_documents(&resolved)?;
    let partitions = docs
        .iter()
        .flat_map(|doc| doc.partitions.iter())
        .collect::<Vec<_>>();
    let mut findings = Vec::new();
    for error in &validation.errors {
        findings.push(project_finding("error", error));
    }
    for warning in &validation.warnings {
        findings.push(json!({
            "code": "project.validation_warning",
            "severity": "warning",
            "message": warning,
            "handle": Value::Null,
            "path": Value::Null
        }));
    }
    add_project_file_hazards(&resolved.project_dir, &mut findings)?;
    for doc in &docs {
        if doc.partitions.is_empty() {
            findings.push(json!({
                "code": "handoff.table_without_partition",
                "severity": "error",
                "message": format!("table has no partition to rebind safely: {}", doc.table),
                "handle": table_handle(&doc.table),
                "path": canonical_display(&doc.path)
            }));
        }
    }
    for partition in &partitions {
        add_partition_findings(partition, &mut findings);
    }
    let source_template_path = source_templates_path(&resolved.project_dir);
    match load_source_template_store(&resolved) {
        Ok(store) => {
            for template in &store.templates {
                add_source_template_findings(template, &source_template_path, &mut findings);
            }
        }
        Err(err) => {
            findings.push(json!({
                "code": "handoff.source_template_store_invalid",
                "severity": "error",
                "message": err.message,
                "handle": Value::Null,
                "path": canonical_display(&source_template_path)
            }));
        }
    }

    let error_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count();
    let review_partition_count = partitions
        .iter()
        .filter(|partition| partition.safety.status != "safe")
        .count();
    let review_finding_count = findings
        .iter()
        .filter(|finding| {
            finding["severity"] == "warning"
                && finding["code"]
                    .as_str()
                    .is_some_and(|code| code.contains("pii_suspect"))
        })
        .count();
    let status = if error_count > 0 || !validation.errors.is_empty() {
        "unsafe"
    } else if review_partition_count > 0 || review_finding_count > 0 {
        "review"
    } else {
        "safe"
    };
    let ok = status == "safe";
    let project_arg = command_arg(&resolved.project_dir);

    Ok(json!({
        "schema": "powerbi-cli.handoff.check.v1",
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "safeForOfflineHandoff": ok,
        "status": status,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "counts": {
            "tables": docs.len(),
            "partitions": partitions.len(),
            "safePartitions": partitions.iter().filter(|partition| partition.safety.status == "safe").count(),
            "reviewPartitions": review_partition_count,
            "reviewFindings": review_finding_count,
            "sourceTemplates": load_source_template_store(&resolved).map(|store| store.templates.len()).unwrap_or(0),
            "findings": findings.len(),
            "errors": error_count
        },
        "partitions": partitions.iter().map(|partition| partition_summary_json(partition)).collect::<Vec<_>>(),
        "findings": findings,
        "next": if ok {
            vec![
                format!("powerbi-cli validate --strict {} --json", project_arg)
            ]
        } else {
            vec![
                format!("powerbi-cli model partitions list --project {} --json", project_arg),
                format!("powerbi-cli validate --strict {} --json", project_arg)
            ]
        },
        "instructions": if ok {
            vec![format!("Open {} in Power BI Desktop at work and rebind dummy #table partitions to corporate sources.", command_arg(&resolved.pbip_path))]
        } else {
            Vec::<String>::new()
        }
    }))
}

fn parse_check_args(args: &[String]) -> CliResult<CheckOptions> {
    let mut options = CheckOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                if options.project.is_some() {
                    return Err(CliError::invalid_args(
                        "handoff check accepts exactly one project",
                    )
                    .with_hint("Pass either a positional project path or --project, not both.")
                    .with_suggested_command(
                        "powerbi-cli handoff check <project-dir-or.pbip> --json",
                    ));
                }
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown handoff check flag: {other}"))
                        .with_hint("Run `powerbi-cli handoff check <project-dir-or.pbip> --json`.")
                        .with_suggested_command(
                            "powerbi-cli handoff check <project-dir-or.pbip> --json",
                        ),
                );
            }
            other => {
                if options.project.is_some() {
                    return Err(CliError::invalid_args(
                        "handoff check accepts exactly one project",
                    )
                    .with_hint("Pass either a positional project path or --project, not both.")
                    .with_suggested_command(
                        "powerbi-cli handoff check <project-dir-or.pbip> --json",
                    ));
                }
                options.project = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(options)
}

fn add_partition_findings(partition: &PartitionRecord, findings: &mut Vec<Value>) {
    for finding in &partition.safety.findings {
        findings.push(json!({
            "code": finding.code,
            "severity": finding.severity,
            "message": finding.message,
            "handle": partition.handle(),
            "path": canonical_display(&partition.path)
        }));
    }
    if partition.source_kind != "dummyMTable" {
        findings.push(json!({
            "code": "handoff.partition_not_dummy",
            "severity": "error",
            "message": format!("handoff requires dummy #table partitions; {} uses {}", partition.handle(), partition.source_kind),
            "handle": partition.handle(),
            "path": canonical_display(&partition.path)
        }));
    }
}

fn add_source_template_findings(
    template: &crate::source_templates::SourceTemplateRecord,
    path: &std::path::Path,
    findings: &mut Vec<Value>,
) {
    for finding in source_template_findings(template) {
        findings.push(json!({
            "code": finding.code,
            "severity": finding.severity,
            "message": finding.message,
            "handle": template.handle,
            "path": canonical_display(path)
        }));
    }
}

fn add_project_file_hazards(
    project_dir: &std::path::Path,
    findings: &mut Vec<Value>,
) -> CliResult<()> {
    for entry in WalkDir::new(project_dir) {
        let entry = crate::walkdir_entry(project_dir, entry, "walk handoff safety inputs")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(project_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let normalized = relative.to_ascii_lowercase();
        let code = if normalized.contains("/.pbi/") || normalized.starts_with(".pbi/") {
            Some("handoff.powerbi_cache_folder")
        } else if normalized.ends_with(".abf") {
            Some("handoff.analysis_services_cache")
        } else if normalized.ends_with(".pbix") || normalized.ends_with(".pbit") {
            Some("handoff.binary_powerbi_file")
        } else if normalized.ends_with("localsettings.json") {
            Some("handoff.local_settings_file")
        } else if normalized.ends_with(".csv")
            || normalized.ends_with(".xlsx")
            || normalized.ends_with(".parquet")
            || normalized.ends_with(".duckdb")
            || normalized.ends_with(".sqlite")
            || normalized.ends_with(".sqlite3")
        {
            Some("handoff.embedded_data_file")
        } else {
            None
        };
        if let Some(code) = code {
            findings.push(json!({
                "code": code,
                "severity": "error",
                "message": format!("offline handoff project contains unsafe file: {relative}"),
                "handle": Value::Null,
                "path": canonical_display(path)
            }));
        }
        if is_handoff_text_file(&relative) {
            match fs::read_to_string(path) {
                Ok(text) => {
                    if contains_credential_like_text_str(&text) {
                        findings.push(json!({
                            "code": "handoff.credential_like_text",
                            "severity": "error",
                            "message": format!("offline handoff text file contains credential-like content: {relative}"),
                            "handle": Value::Null,
                            "path": canonical_display(path)
                        }));
                    }
                    if contains_pii_suspect_text(&text) {
                        findings.push(json!({
                            "code": "handoff.pii_suspect_text",
                            "severity": "warning",
                            "message": format!("offline handoff text file contains PII-suspect row literals requiring review: {relative}"),
                            "handle": Value::Null,
                            "path": canonical_display(path)
                        }));
                    }
                }
                Err(err) => findings.push(json!({
                    "code": "handoff.text_scan_failed",
                    "severity": "error",
                    "message": format!("could not read handoff text file {relative}: {err}"),
                    "handle": Value::Null,
                    "path": canonical_display(path)
                })),
            }
        }
    }
    Ok(())
}

fn is_handoff_text_file(relative: &str) -> bool {
    let path = std::path::Path::new(relative);
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("tmdl" | "m" | "json" | "md" | "pbip" | "pbir" | "pbism")
    ) || path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(".platform"))
}

fn project_finding(severity: &str, message: &str) -> Value {
    let code = if message.contains("offline-unsafe") {
        "handoff.offline_unsafe_file"
    } else {
        "project.validation_error"
    };
    json!({
        "code": code,
        "severity": severity,
        "message": message,
        "handle": Value::Null,
        "path": Value::Null
    })
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli handoff check <project-dir-or.pbip> --json`.")
            .with_suggested_command("powerbi-cli handoff check <project-dir-or.pbip> --json")
    })?;
    *index += 2;
    Ok(value.clone())
}
