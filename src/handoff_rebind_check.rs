//! Offline verification of work-machine source rebindings.
//!
//! `handoff rebind-check` deliberately stops at the boundary where a source
//! can be inspected without evaluating it.  It checks that a partition no
//! longer carries a generated/placeholder source, validates the closed
//! connector shapes emitted by source templates, and probes only local file
//! system paths.  It never invokes a connector or asks Power BI Desktop to
//! refresh.

use crate::rules;
use crate::safety_scan::{contains_credential_like_text_str, redact_credential_values};
use crate::source_template_paths::{is_placeholder, validate_sharepoint_site_url};
use crate::source_templates::{
    SourceTemplateRecord, SourceTemplateStore, find_template, load_source_template_store,
    source_template_safety_json, source_templates_path,
};
use crate::tmdl::{PartitionRecord, load_table_documents, same_name, table_handle};
use crate::validation::validate_command;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project,
};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct RebindCheckOptions {
    project: Option<PathBuf>,
    table: Option<String>,
    partition: Option<String>,
}

#[derive(Debug, Default)]
struct PartitionCheck {
    state: &'static str,
    source_kind: String,
    findings: Vec<Value>,
    paths: Vec<Value>,
}

#[derive(Debug)]
enum MArgument {
    String(String),
    Other,
}

pub(crate) fn rebind_check(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    let project = options.project.ok_or_else(|| {
        CliError::invalid_args("handoff rebind-check requires <project-dir-or.pbip> or --project")
            .with_hint(
                "Pass the PBIP project directory or .pbip file after `handoff rebind-check`.",
            )
            .with_suggested_command("powerbi-cli handoff rebind-check <project-dir-or.pbip> --json")
    })?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let mut partitions = docs
        .iter()
        .flat_map(|doc| doc.partitions.iter())
        .filter(|partition| {
            options
                .table
                .as_ref()
                .is_none_or(|table| same_name(table, &partition.table))
        })
        .filter(|partition| {
            options
                .partition
                .as_ref()
                .is_none_or(|selector| selector_matches_partition(selector, partition))
        })
        .collect::<Vec<_>>();
    partitions.sort_by_key(|partition| partition.handle());
    if (options.table.is_some() || options.partition.is_some()) && partitions.is_empty() {
        return Err(CliError::validation_failed(
            "handoff rebind-check selector matched no partitions",
        )
        .with_hint("Use `model partitions list` to obtain a current table or partition handle.")
        .with_suggested_command(
            "powerbi-cli model partitions list --project <project-dir-or.pbip> --json",
        ));
    }

    let template_path = source_templates_path(&resolved.project_dir);
    let (store, template_store_error) = match load_source_template_store(&resolved) {
        Ok(store) => (store, None),
        Err(error) => (SourceTemplateStore::default(), Some(error)),
    };
    let mut findings = Vec::new();
    if let Some(error) = template_store_error {
        findings.push(json!({
            "code": rules::HANDOFF_SOURCE_TEMPLATE_STORE_INVALID,
            "severity": "error",
            "message": error.message,
            "handle": Value::Null,
            "path": canonical_display(&template_path)
        }));
    }
    if options.table.is_none() && options.partition.is_none() {
        for doc in &docs {
            if doc.partitions.is_empty() {
                findings.push(json!({
                "code": rules::HANDOFF_TABLE_WITHOUT_PARTITION,
                "severity": "error",
                "message": format!("table has no partition to resolve after rebinding: {}", doc.table),
                "handle": table_handle(&doc.table),
                "path": canonical_display(&doc.path)
            }));
            }
        }
        if docs.is_empty() {
            findings.push(json!({
                "code": rules::HANDOFF_TABLE_WITHOUT_PARTITION,
                "severity": "error",
                "message": "semantic model contains no table definitions to resolve after rebinding",
                "handle": Value::Null,
                "path": canonical_display(&resolved.semantic_model_dir)
            }));
        }
    }

    let mut partition_values = Vec::with_capacity(partitions.len());
    for partition in partitions {
        let mut check = inspect_partition(partition, &resolved.project_dir);
        let template = find_template(&store, &partition.handle());
        // An unsafe sidecar must not be silently ignored just because the
        // already-applied partition happens to be syntactically valid.  Emit
        // only the registered finding; never echo template/source text.
        if let Some(template) = template {
            for template_finding in template_safety_findings(template) {
                let finding = json!({
                    "code": template_finding["code"].clone(),
                    "severity": template_finding["severity"].clone(),
                    "message": template_finding["message"].clone(),
                    "handle": partition.handle(),
                    "path": canonical_display(&partition.path)
                });
                check.findings.push(finding.clone());
            }
        }
        findings.extend(check.findings.iter().cloned());
        let template_summary = template.map(|record| template_summary(record, &template_path));
        let materialized = check.state == "materialized";
        partition_values.push(json!({
            "handle": partition.handle(),
            "table": partition.table,
            "partition": partition.name,
            "state": check.state,
            "materialized": materialized,
            "resolved": materialized,
            "sourceKind": check.source_kind,
            "sourcePath": canonical_display(&partition.path),
            "template": template_summary,
            "paths": check.paths,
            "findings": check.findings
        }));
    }

    // Validation is native strict validation only.  Calling the existing
    // command implementation keeps its output contract in sync while still
    // guaranteeing that no connector or Desktop process is started.
    let validation_args = vec![
        "--strict".to_string(),
        resolved.project_dir.to_string_lossy().into_owned(),
    ];
    let validation = validate_command(&validation_args)?;
    let validation_ok = validation["ok"].as_bool().unwrap_or(false);
    if let Some(errors) = validation["errors"].as_array() {
        for error in errors.iter().filter_map(Value::as_str) {
            findings.push(json!({
                "code": rules::PROJECT_VALIDATION_ERROR,
                "severity": "error",
                "message": error,
                "handle": Value::Null,
                "path": Value::Null
            }));
        }
    }
    let partition_error_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count();
    let partition_warning_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "warning")
        .count();
    let all_materialized = partition_values
        .iter()
        .all(|partition| partition["materialized"] == Value::Bool(true));
    let status = if !validation_ok || partition_error_count > 0 {
        "unsafe"
    } else if partition_warning_count > 0 {
        "review"
    } else {
        "safe"
    };
    let ok = status == "safe" && all_materialized;
    let project_arg = command_arg(&resolved.project_dir);
    let validate_next = format!("powerbi-cli validate --strict {} --json", project_arg);
    let plan_next = format!("powerbi-cli handoff rebind-plan {} --json", project_arg);
    let desktop_next = format!("powerbi-cli desktop open {} --json", project_arg);
    let mut next = if ok {
        vec![validate_next.clone(), desktop_next]
    } else {
        vec![plan_next, validate_next.clone()]
    };
    next.dedup();

    rules::ensure_finding_ids_registered(&findings, "code")?;
    Ok(json!({
        "schema": "powerbi-cli.handoff.rebind-check.v1",
        "ok": ok,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "status": status,
        "offline": true,
        "credentialsEmbedded": false,
        "connectionsOpened": false,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "counts": {
            "partitions": partition_values.len(),
            "materializedPartitions": partition_values.iter().filter(|partition| partition["materialized"] == Value::Bool(true)).count(),
            "placeholderPartitions": partition_values.iter().filter(|partition| partition["state"] == "placeholder").count(),
            "unresolvedPartitions": partition_values.iter().filter(|partition| partition["state"] == "unresolved").count(),
            "findings": findings.len(),
            "errors": partition_error_count,
            "warnings": partition_warning_count,
            "templates": store.templates.len()
        },
        "partitions": partition_values,
        "findings": findings,
        "validation": validation,
        "refresh": {
            "requested": false,
            "performed": false,
            "available": false,
            "status": "not-run",
            "connectionOpened": false,
            "reason": "This check is offline and credential-free; Desktop refresh is intentionally not invoked.",
            "next": [desktop_next_command(&resolved.project_dir)]
        },
        "next": next,
        "instructions": [
            "Rebind-check proves source syntax and local path readability only; it never opens a database, SharePoint, or Power BI Desktop connection.",
            "Use the Desktop command in next[] for the separate work-machine refresh and canvas proof."
        ]
    }))
}

fn parse_args(args: &[String]) -> CliResult<RebindCheckOptions> {
    let mut options = RebindCheckOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                if options.project.is_some() {
                    return Err(one_project_error());
                }
                options.project = Some(PathBuf::from(take_value(args, &mut index, "--project")?));
            }
            "--table" => options.table = Some(take_value(args, &mut index, "--table")?),
            "--partition" | "--handle" => {
                options.partition = Some(take_value(args, &mut index, "--partition")?);
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown handoff rebind-check flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for handoff rebind-check` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli handoff rebind-check <project-dir-or.pbip> --json",
                ));
            }
            other => {
                if options.project.is_some() {
                    return Err(one_project_error());
                }
                options.project = Some(PathBuf::from(other));
                index += 1;
            }
        }
    }
    Ok(options)
}

fn inspect_partition(partition: &PartitionRecord, project_dir: &Path) -> PartitionCheck {
    let mut check = PartitionCheck {
        source_kind: partition.source_kind.clone(),
        ..PartitionCheck::default()
    };
    let Some(source) = partition.source.as_deref() else {
        check.findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_PARTITION_SOURCE_MISSING,
            "error",
            "partition has no source expression to resolve after rebinding",
        ));
        check.state = "unresolved";
        return check;
    };

    let has_placeholder = source_string_literals(source)
        .iter()
        .any(|literal| is_placeholder(literal));
    if partition.source_kind == "dummyMTable" {
        check.findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_PARTITION_PLACEHOLDER,
            "error",
            "partition still uses a generated dummy #table source; apply a materialized work-machine template",
        ));
    }
    if has_placeholder {
        check.findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_SOURCE_PLACEHOLDER,
            "error",
            "partition source still contains unresolved template placeholder values",
        ));
    }
    if partition
        .safety
        .findings
        .iter()
        .any(|finding| finding.code == rules::PARTITION_CREDENTIAL_LIKE_TEXT)
    {
        check.findings.push(partition_finding(
            partition,
            rules::PARTITION_CREDENTIAL_LIKE_TEXT,
            "error",
            "partition source contains credential-like text; rebind-check never reveals or stores it",
        ));
    }

    match partition.source_kind.as_str() {
        "sqlDatabase" => validate_database_source(
            partition,
            source,
            "Sql.Database",
            "SQL Server",
            &mut check.findings,
        ),
        "postgresqlDatabase" => validate_database_source(
            partition,
            source,
            "PostgreSQL.Database",
            "PostgreSQL",
            &mut check.findings,
        ),
        "odbcDataSource" => validate_odbc_source(partition, source, &mut check.findings),
        "sharePointFiles" => validate_sharepoint_source(partition, source, &mut check.findings),
        "externalFile" => validate_external_path_source(
            partition,
            source,
            project_dir,
            &mut check.paths,
            &mut check.findings,
        ),
        _ => check.findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_SOURCE_UNRECOGNIZED,
            "error",
            "partition source is not a supported materialized SQL, PostgreSQL, ODBC, SharePoint, file, or folder source",
        )),
    }

    check.state = if partition.source_kind == "dummyMTable" || has_placeholder {
        "placeholder"
    } else if check
        .findings
        .iter()
        .any(|finding| finding["severity"] == "error")
    {
        "unresolved"
    } else {
        "materialized"
    };
    check
}

fn validate_database_source(
    partition: &PartitionRecord,
    source: &str,
    function: &str,
    label: &str,
    findings: &mut Vec<Value>,
) {
    let arguments = match call_arguments(source, function) {
        Ok(Some(arguments)) => arguments,
        Ok(None) | Err(_) => {
            findings.push(partition_finding(
                partition,
                rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
                "error",
                &format!("{label} source is missing a parseable connector call"),
            ));
            return;
        }
    };
    if arguments.len() < 2
        || !non_empty_materialized_string(arguments.first())
        || !non_empty_materialized_string(arguments.get(1))
    {
        findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
            "error",
            &format!("{label} source requires concrete server and database string arguments"),
        ));
    }
}

fn validate_odbc_source(partition: &PartitionRecord, source: &str, findings: &mut Vec<Value>) {
    let arguments = match call_arguments(source, "Odbc.DataSource") {
        Ok(Some(arguments)) => arguments,
        Ok(None) | Err(_) => {
            findings.push(partition_finding(
                partition,
                rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
                "error",
                "ODBC source is missing a parseable Odbc.DataSource call",
            ));
            return;
        }
    };
    let Some(MArgument::String(connection)) = arguments.first() else {
        findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
            "error",
            "ODBC source requires a concrete DSN string argument",
        ));
        return;
    };
    let dsn = if connection
        .as_bytes()
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"dsn="))
    {
        &connection[4..]
    } else {
        connection
    };
    if dsn.trim().is_empty() || dsn.contains([';', '=']) {
        findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
            "error",
            "ODBC source DSN is empty or contains inline connection attributes",
        ));
    }
}

fn validate_sharepoint_source(
    partition: &PartitionRecord,
    source: &str,
    findings: &mut Vec<Value>,
) {
    let arguments = match call_arguments(source, "SharePoint.Files") {
        Ok(Some(arguments)) => arguments,
        Ok(None) | Err(_) => {
            findings.push(partition_finding(
                partition,
                rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
                "error",
                "SharePoint source is missing a parseable SharePoint.Files call",
            ));
            return;
        }
    };
    let Some(MArgument::String(site_url)) = arguments.first() else {
        findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
            "error",
            "SharePoint source requires a concrete site URL string argument",
        ));
        return;
    };
    if site_url.trim().is_empty() || validate_sharepoint_site_url(site_url).is_err() {
        findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
            "error",
            "SharePoint source requires a credential-free HTTPS *.sharepoint.com site URL",
        ));
    }
}

fn validate_external_path_source(
    partition: &PartitionRecord,
    source: &str,
    project_dir: &Path,
    paths: &mut Vec<Value>,
    findings: &mut Vec<Value>,
) {
    if let Ok(Some(arguments)) = call_arguments(source, "File.Contents") {
        let Some(MArgument::String(path)) = arguments.first() else {
            findings.push(partition_finding(
                partition,
                rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
                "error",
                "file source requires a concrete File.Contents path string",
            ));
            return;
        };
        inspect_local_path(partition, project_dir, path, "file", paths, findings);
        return;
    }
    if let Ok(Some(arguments)) = call_arguments(source, "Folder.Files") {
        let Some(MArgument::String(path)) = arguments.first() else {
            findings.push(partition_finding(
                partition,
                rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
                "error",
                "folder source requires a concrete Folder.Files path string",
            ));
            return;
        };
        inspect_local_path(partition, project_dir, path, "folder", paths, findings);
        return;
    }
    findings.push(partition_finding(
        partition,
        rules::REBIND_CHECK_SOURCE_SYNTAX_INCOMPLETE,
        "error",
        "file/folder source is missing a parseable File.Contents or Folder.Files call",
    ));
}

fn inspect_local_path(
    partition: &PartitionRecord,
    project_dir: &Path,
    raw_path: &str,
    expected: &str,
    paths: &mut Vec<Value>,
    findings: &mut Vec<Value>,
) {
    if is_placeholder(raw_path) {
        return;
    }
    let path = resolve_local_path(project_dir, raw_path);
    let display = if contains_credential_like_text_str(raw_path) {
        "<redacted-local-path>".to_string()
    } else {
        canonical_display(&path)
    };
    let mut check = json!({
        "path": display,
        "kind": expected,
        "exists": false,
        "readable": false,
        "status": "missing"
    });
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => {
            findings.push(partition_finding(
                partition,
                rules::REBIND_CHECK_LOCAL_PATH_MISSING,
                "error",
                &format!("local {expected} source path does not exist"),
            ));
            paths.push(check);
            return;
        }
    };
    check["exists"] = Value::Bool(true);
    let kind_matches =
        (expected == "file" && metadata.is_file()) || (expected == "folder" && metadata.is_dir());
    if !kind_matches {
        check["status"] = Value::String("unreadable".to_string());
        findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_LOCAL_PATH_UNREADABLE,
            "error",
            &format!("local source path is not a readable {expected}"),
        ));
        paths.push(check);
        return;
    }
    let readable = if expected == "file" {
        File::open(&path).is_ok()
    } else {
        fs::read_dir(&path).is_ok()
    };
    if !readable {
        check["status"] = Value::String("unreadable".to_string());
        findings.push(partition_finding(
            partition,
            rules::REBIND_CHECK_LOCAL_PATH_UNREADABLE,
            "error",
            &format!("local source {expected} exists but is not readable"),
        ));
    } else {
        check["readable"] = Value::Bool(true);
        check["status"] = Value::String("ok".to_string());
    }
    paths.push(check);
}

fn template_safety_findings(record: &SourceTemplateRecord) -> Vec<Value> {
    let safety = source_template_safety_json(record);
    safety
        .get("findings")
        .and_then(Value::as_array)
        .map(|findings| findings.to_vec())
        .unwrap_or_default()
}

fn template_summary(record: &SourceTemplateRecord, path: &Path) -> Value {
    json!({
        "handle": redact_credential_values(&record.handle),
        "name": record.name.as_deref().map(redact_credential_values),
        "partitionHandle": redact_credential_values(&record.partition_handle),
        "kind": record.kind,
        "safety": source_template_safety_json(record),
        "path": canonical_display(path)
    })
}

fn partition_finding(
    partition: &PartitionRecord,
    code: &str,
    severity: &str,
    message: &str,
) -> Value {
    json!({
        "code": code,
        "severity": severity,
        "message": message,
        "handle": partition.handle(),
        "path": canonical_display(&partition.path)
    })
}

fn non_empty_materialized_string(argument: Option<&MArgument>) -> bool {
    matches!(argument, Some(MArgument::String(value)) if !value.trim().is_empty() && !is_placeholder(value))
}

fn source_string_literals(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut literals = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(next) = skip_m_comment(bytes, index) {
            index = next;
            continue;
        }
        if bytes[index] == b'"' {
            if let Some((literal, next)) = parse_m_string(bytes, index) {
                literals.push(literal);
                index = next;
                continue;
            }
        }
        index += 1;
    }
    literals
}

fn call_arguments(source: &str, function: &str) -> Result<Option<Vec<MArgument>>, String> {
    let bytes = source.as_bytes();
    let function_bytes = function.as_bytes();
    let mut index = 0;
    while index + function_bytes.len() <= bytes.len() {
        if let Some(next) = skip_m_comment(bytes, index) {
            index = next;
            continue;
        }
        if bytes[index] == b'"' {
            index = parse_m_string(bytes, index)
                .map(|(_, next)| next)
                .unwrap_or(bytes.len());
            continue;
        }
        if bytes[index..index + function_bytes.len()].eq_ignore_ascii_case(function_bytes)
            && (index == 0 || !is_identifier_byte(bytes[index - 1]))
            && (index + function_bytes.len() == bytes.len()
                || !is_identifier_byte(bytes[index + function_bytes.len()]))
        {
            let mut open = index + function_bytes.len();
            while open < bytes.len() && bytes[open].is_ascii_whitespace() {
                open += 1;
            }
            if bytes.get(open) != Some(&b'(') {
                index += function_bytes.len();
                continue;
            }
            let close = matching_paren(bytes, open)?;
            let body = &source[open + 1..close];
            let arguments = split_m_arguments(body)
                .into_iter()
                .map(|argument| {
                    let argument = argument.trim();
                    parse_m_string(argument.as_bytes(), 0)
                        .filter(|(_, next)| *next == argument.len())
                        .map(|(literal, _)| MArgument::String(literal))
                        .unwrap_or(MArgument::Other)
                })
                .collect();
            return Ok(Some(arguments));
        }
        index += 1;
    }
    Ok(None)
}

fn matching_paren(bytes: &[u8], open: usize) -> Result<usize, String> {
    let mut depth = 1_usize;
    let mut index = open + 1;
    while index < bytes.len() {
        if let Some(next) = skip_m_comment(bytes, index) {
            index = next;
            continue;
        }
        if bytes[index] == b'"' {
            index = parse_m_string(bytes, index)
                .map(|(_, next)| next)
                .ok_or_else(|| "unterminated M string literal".to_string())?;
            continue;
        }
        match bytes[index] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    Err("unterminated connector call".to_string())
}

fn split_m_arguments(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut values = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut parens = 0_usize;
    let mut brackets = 0_usize;
    let mut braces = 0_usize;
    while index < bytes.len() {
        if let Some(next) = skip_m_comment(bytes, index) {
            index = next;
            continue;
        }
        if bytes[index] == b'"' {
            index = parse_m_string(bytes, index)
                .map(|(_, next)| next)
                .unwrap_or(bytes.len());
            continue;
        }
        match bytes[index] {
            b'(' => parens += 1,
            b')' => parens = parens.saturating_sub(1),
            b'[' => brackets += 1,
            b']' => brackets = brackets.saturating_sub(1),
            b'{' => braces += 1,
            b'}' => braces = braces.saturating_sub(1),
            b',' if parens == 0 && brackets == 0 && braces == 0 => {
                values.push(&body[start..index]);
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    values.push(&body[start..]);
    values
}

fn parse_m_string(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut value = Vec::new();
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            if bytes.get(index + 1) == Some(&b'"') {
                value.push(b'"');
                index += 2;
                continue;
            }
            return Some((String::from_utf8_lossy(&value).into_owned(), index + 1));
        }
        value.push(bytes[index]);
        index += 1;
    }
    None
}

fn skip_m_comment(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'/') {
        return None;
    }
    if bytes.get(start + 1) == Some(&b'/') {
        let mut index = start + 2;
        while index < bytes.len() && bytes[index] != b'\n' {
            index += 1;
        }
        return Some(index);
    }
    if bytes.get(start + 1) == Some(&b'*') {
        let mut index = start + 2;
        while index + 1 < bytes.len() {
            if bytes[index] == b'*' && bytes[index + 1] == b'/' {
                return Some(index + 2);
            }
            index += 1;
        }
        return Some(bytes.len());
    }
    None
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.'
}

fn resolve_local_path(project_dir: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    }
}

fn selector_matches_partition(selector: &str, partition: &PartitionRecord) -> bool {
    if selector.starts_with("partition:") {
        selector == partition.handle()
    } else {
        same_name(selector, &partition.name)
    }
}

fn desktop_next_command(project: &Path) -> String {
    format!("powerbi-cli desktop open {} --json", command_arg(project))
}

fn one_project_error() -> CliError {
    CliError::invalid_args("handoff rebind-check accepts exactly one project")
        .with_hint("Pass either a positional project path or --project, not both.")
        .with_suggested_command("powerbi-cli handoff rebind-check <project-dir-or.pbip> --json")
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for handoff rebind-check` for exact usage.",
            )
            .with_suggested_command("powerbi-cli handoff rebind-check <project-dir-or.pbip> --json")
    })?;
    *index += 2;
    Ok(value.clone())
}

#[cfg(test)]
mod tests {
    use super::{MArgument, call_arguments, source_string_literals};

    #[test]
    fn connector_argument_parser_handles_nested_m_and_escaped_quotes() {
        let args = call_arguments(
            r#"let Source = Sql.Database("server", "db", [Options = "a""b"]), X = "Sql.Database(\"fake\")" in Source"#,
            "Sql.Database",
        )
        .expect("parse")
        .expect("call");
        assert!(matches!(args.first(), Some(MArgument::String(value)) if value == "server"));
        assert!(matches!(args.get(1), Some(MArgument::String(value)) if value == "db"));
        assert!(
            source_string_literals("// \"ignored\"\nSource = \"<server>\"")
                .iter()
                .any(|value| value == "<server>")
        );
        assert!(
            source_string_literals("/* \"ignored <placeholder>\" */ Source = \"server\"")
                .iter()
                .all(|value| value == "server")
        );
    }

    #[test]
    fn malformed_connector_call_is_reported_without_panicking() {
        assert!(call_arguments("Sql.Database(\"server\",", "Sql.Database").is_err());
    }
}
