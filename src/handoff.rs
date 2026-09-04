use crate::handoff_rebind_check::rebind_check;
use crate::input_safety::{INPUT_SAFETY_ERROR_CODE, InputKind, read_utf8};
use crate::partitions::partition_summary_json;
use crate::rebind_plan::rebind_plan;
use crate::rules;
use crate::safety_scan::{contains_credential_like_text_str, contains_pii_suspect_text};
use crate::source_templates::{
    load_source_template_store, source_template_findings, source_templates_path,
};
use crate::tmdl::{
    PartitionRecord, load_table_documents, partition_source_kind_is_external, table_handle,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use walkdir::WalkDir;

pub(crate) fn handoff_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "handoff requires a subcommand: check, rebind-plan, rebind-check",
        )
        .with_hint("Run `powerbi-cli handoff check <project-dir-or.pbip> --json`.")
        .with_suggested_command("powerbi-cli handoff check <project-dir-or.pbip> --json"));
    };

    match action.as_str() {
        "check" => check_handoff(rest),
        "rebind" | "rebind-plan" => rebind_plan(rest),
        "rebind-check" | "rebindCheck" => rebind_check(rest),
        _ => Err(
            CliError::invalid_args(format!("unknown handoff command: {action}"))
                .with_hint("Run `powerbi-cli handoff check <project-dir-or.pbip> --json`, `powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json`, or `powerbi-cli handoff rebind-check <project-dir-or.pbip> --json`.")
                .with_suggested_command("powerbi-cli handoff check <project-dir-or.pbip> --json")
                .with_suggested_command("powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json")
                .with_suggested_command("powerbi-cli handoff rebind-check <project-dir-or.pbip> --json"),
        ),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum HandoffTarget {
    #[default]
    Offline,
    Work,
}

impl HandoffTarget {
    fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Work => "work",
        }
    }
}

#[derive(Debug, Default)]
struct CheckOptions {
    project: Option<PathBuf>,
    target: HandoffTarget,
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
            "code": rules::PROJECT_VALIDATION_WARNING,
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
                "code": rules::HANDOFF_TABLE_WITHOUT_PARTITION,
                "severity": "error",
                "message": format!("table has no partition to rebind safely: {}", doc.table),
                "handle": table_handle(&doc.table),
                "path": canonical_display(&doc.path)
            }));
        }
    }
    for partition in &partitions {
        add_partition_findings(partition, options.target, &mut findings);
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
                "code": rules::HANDOFF_SOURCE_TEMPLATE_STORE_INVALID,
                "severity": "error",
                "message": err.message,
                "handle": Value::Null,
                "path": canonical_display(&source_template_path)
            }));
        }
    }
    rules::ensure_finding_ids_registered(&findings, "code")?;

    let error_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count();
    let review_partition_count = partitions
        .iter()
        .filter(|partition| !partition_safe_for_target(partition, options.target))
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
    let has_live_sources = partitions
        .iter()
        .any(|partition| is_recognized_live_source(&partition.source_kind));
    let has_dummy_sources = partitions
        .iter()
        .any(|partition| partition.source_kind == "dummyMTable");
    let has_model_derived_sources = partitions
        .iter()
        .any(|partition| partition.source_kind == "modelDerived");
    let source_mode = match (
        has_live_sources,
        has_dummy_sources,
        has_model_derived_sources,
    ) {
        (true, false, false) => "live",
        (false, true, false) => "dummy",
        (false, false, true) => "modelDerived",
        (false, false, false) => "unknown",
        _ => "mixed",
    };
    let project_arg = command_arg(&resolved.project_dir);

    Ok(json!({
        "schema": "powerbi-cli.handoff.check.v1",
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "target": options.target.as_str(),
        "sourceMode": source_mode,
        "safeForOfflineHandoff": ok && options.target == HandoffTarget::Offline,
        "safeForWorkHandoff": ok && options.target == HandoffTarget::Work,
        "status": status,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "counts": {
            "tables": docs.len(),
            "partitions": partitions.len(),
            "safePartitions": partitions.iter().filter(|partition| partition.safety.status == "safe").count(),
            "acceptedLivePartitions": partitions.iter().filter(|partition| {
                options.target == HandoffTarget::Work
                    && is_recognized_live_source(&partition.source_kind)
                    && partition_safe_for_target(partition, options.target)
            }).count(),
            "safeForTargetPartitions": partitions.iter().filter(|partition| partition_safe_for_target(partition, options.target)).count(),
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
        "instructions": if ok && options.target == HandoffTarget::Offline {
            vec![format!("Open {} in Power BI Desktop at work and rebind dummy #table partitions to corporate sources.", command_arg(&resolved.pbip_path))]
        } else if ok {
            vec![format!("Open {} in Power BI Desktop on the target work network, configure credentials if prompted, refresh, and verify the report.", command_arg(&resolved.pbip_path))]
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
            "--target" => {
                let value = take_value(args, &mut i, "--target")?;
                options.target = match value.as_str() {
                    "offline" => HandoffTarget::Offline,
                    "work" => HandoffTarget::Work,
                    _ => {
                        return Err(CliError::invalid_args(format!(
                            "handoff check --target must be offline or work, got: {value}"
                        ))
                        .with_suggested_command(
                            "powerbi-cli handoff check <project-dir-or.pbip> --target work --json",
                        ));
                    }
                };
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

fn add_partition_findings(
    partition: &PartitionRecord,
    target: HandoffTarget,
    findings: &mut Vec<Value>,
) {
    for finding in &partition.safety.findings {
        let accepted_live_source =
            target == HandoffTarget::Work && finding.code.starts_with("partition.real_connector.");
        let accepted_model_derived = target == HandoffTarget::Work
            && partition.source_kind == "modelDerived"
            && finding.code == rules::PARTITION_MODEL_DERIVED;
        findings.push(json!({
            "code": finding.code,
            "severity": if accepted_live_source || accepted_model_derived { "info" } else { finding.severity.as_str() },
            "message": if accepted_live_source {
                format!("recognized live connector accepted for work target: {}", partition.source_kind)
            } else if accepted_model_derived {
                "model-derived partition accepted for work target".to_string()
            } else {
                finding.message.clone()
            },
            "handle": partition.handle(),
            "path": canonical_display(&partition.path)
        }));
    }
    if target == HandoffTarget::Offline && partition.source_kind != "dummyMTable" {
        findings.push(json!({
            "code": rules::HANDOFF_PARTITION_NOT_DUMMY,
            "severity": "error",
            "message": format!("handoff requires dummy #table partitions; {} uses {}", partition.handle(), partition.source_kind),
            "handle": partition.handle(),
            "path": canonical_display(&partition.path)
        }));
    } else if target == HandoffTarget::Work
        && partition.source_kind != "dummyMTable"
        && partition.source_kind != "modelDerived"
        && !is_recognized_live_source(&partition.source_kind)
    {
        findings.push(json!({
            "code": rules::HANDOFF_PARTITION_SOURCE_UNRECOGNIZED,
            "severity": "error",
            "message": format!("work handoff requires a dummy table or recognized connector; {} uses {}", partition.handle(), partition.source_kind),
            "handle": partition.handle(),
            "path": canonical_display(&partition.path)
        }));
    }
}

fn is_recognized_live_source(source_kind: &str) -> bool {
    partition_source_kind_is_external(source_kind)
}

pub(crate) fn partition_is_safe_materialized_work_source(partition: &PartitionRecord) -> bool {
    is_recognized_live_source(&partition.source_kind)
        && partition_safe_for_target(partition, HandoffTarget::Work)
}

fn partition_safe_for_target(partition: &PartitionRecord, target: HandoffTarget) -> bool {
    match target {
        HandoffTarget::Offline => partition.safety.status == "safe",
        HandoffTarget::Work if partition.source_kind == "dummyMTable" => {
            partition.safety.status == "safe"
        }
        HandoffTarget::Work if is_recognized_live_source(&partition.source_kind) => {
            !partition.safety.findings.iter().any(|finding| {
                finding.severity == "error"
                    && !finding.code.starts_with("partition.real_connector.")
            })
        }
        HandoffTarget::Work if partition.source_kind == "modelDerived" => !partition
            .safety
            .findings
            .iter()
            .any(|finding| finding.severity == "error"),
        HandoffTarget::Work => false,
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
            Some(rules::HANDOFF_POWERBI_CACHE_FOLDER)
        } else if normalized.ends_with(".abf") {
            Some(rules::HANDOFF_ANALYSIS_SERVICES_CACHE)
        } else if normalized.ends_with(".pbix") || normalized.ends_with(".pbit") {
            Some(rules::HANDOFF_BINARY_POWERBI_FILE)
        } else if normalized.ends_with("localsettings.json") {
            Some(rules::HANDOFF_LOCAL_SETTINGS_FILE)
        } else if normalized.ends_with(".csv")
            || normalized.ends_with(".xlsx")
            || normalized.ends_with(".parquet")
            || normalized.ends_with(".duckdb")
            || normalized.ends_with(".sqlite")
            || normalized.ends_with(".sqlite3")
        {
            Some(rules::HANDOFF_EMBEDDED_DATA_FILE)
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
            match read_utf8(path, InputKind::ProjectText) {
                Ok(text) => {
                    if contains_credential_like_text_str(&text) {
                        findings.push(json!({
                            "code": rules::HANDOFF_CREDENTIAL_LIKE_TEXT,
                            "severity": "error",
                            "message": format!("offline handoff text file contains credential-like content: {relative}"),
                            "handle": Value::Null,
                            "path": canonical_display(path)
                        }));
                    }
                    if contains_pii_suspect_text(&text) {
                        findings.push(json!({
                            "code": rules::HANDOFF_PII_SUSPECT_TEXT,
                            "severity": "warning",
                            "message": format!("offline handoff text file contains PII-suspect row literals requiring review: {relative}"),
                            "handle": Value::Null,
                            "path": canonical_display(path)
                        }));
                    }
                }
                Err(err) if err.code == INPUT_SAFETY_ERROR_CODE => return Err(err),
                Err(err) => findings.push(json!({
                    "code": rules::HANDOFF_TEXT_SCAN_FAILED,
                    "severity": "error",
                    "message": format!("could not read handoff text file {relative}: {}", err.message),
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
        rules::HANDOFF_OFFLINE_UNSAFE_FILE
    } else {
        rules::PROJECT_VALIDATION_ERROR
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
