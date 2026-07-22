use crate::cli_support::{required_project, take_value};
use crate::desktop_target::{ResolvedDesktopTarget, resolve_desktop_target};
use crate::{
    CliError, CliResult, EXIT_ORACLE_UNAVAILABLE, EXIT_VALIDATION_FAILED, canonical_display,
    validate_desktop_runtime_project,
};
#[cfg(windows)]
use crate::{EXIT_ORACLE_FAILED, EXIT_SUCCESS};
#[cfg(windows)]
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use crate::desktop::{Timed, run_command_with_timeout};
#[cfg(windows)]
use std::process::{Command, Stdio};

const DEFAULT_MAX_ROWS: u64 = 1_000;
const DEFAULT_MAX_CELL_CHARS: u64 = 4_096;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_ROWS_LIMIT: u64 = 100_000;
const MAX_CELL_CHARS_LIMIT: u64 = 1_000_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
const MAX_QUERY_BYTES: usize = 1_000_000;
#[cfg(windows)]
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct ExecuteOptions {
    project: Option<PathBuf>,
    query: Option<String>,
    query_file: Option<PathBuf>,
    allow_data_read: bool,
    max_rows: u64,
    max_cell_chars: u64,
    timeout_ms: u64,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            project: None,
            query: None,
            query_file: None,
            allow_data_read: false,
            max_rows: DEFAULT_MAX_ROWS,
            max_cell_chars: DEFAULT_MAX_CELL_CHARS,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeColumn {
    ordinal: u64,
    name: String,
    data_type: String,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
struct BridgeRow {
    #[serde(default)]
    values: Vec<Value>,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BridgeResult {
    ok: bool,
    stage: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    desktop_process_id: Option<u64>,
    #[serde(default)]
    model_process_id: Option<u64>,
    #[serde(default)]
    port: Option<u64>,
    #[serde(default)]
    desktop_version: Option<String>,
    #[serde(default)]
    columns: Vec<BridgeColumn>,
    #[serde(default)]
    rows: Vec<BridgeRow>,
    #[serde(default)]
    row_count: u64,
    #[serde(default)]
    column_count: u64,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    truncated_cells: u64,
}

pub(crate) fn execute(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    if !options.allow_data_read {
        return Err(CliError::invalid_args(
            "model dax execute requires --allow-data-read",
        )
        .with_hint(
            "The query can return model data. Review the DAX and opt in explicitly; results are row- and cell-bounded.",
        )
        .with_suggested_command(
            "powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> --query-file <query.dax> --allow-data-read --json",
        ));
    }

    let project = required_project(options.project.clone(), "model dax execute")?;
    let target = resolve_desktop_target(&project)?;
    target.require_live_model()?;
    let validation_json = match target.project() {
        Some(resolved) => {
            let validation = validate_desktop_runtime_project(resolved)?;
            json!({
                "kind": "pbip-runtime",
                "ok": validation.errors.is_empty(),
                "warnings": validation.warnings,
                "errors": validation.errors
            })
        }
        None => json!({
            "kind": "pbix-archive",
            "ok": true,
            "warnings": [],
            "errors": [],
            "archive": target.pbix.as_ref().map(|info| json!({
                "entries": info.entries,
                "hasDataModel": info.has_data_model,
                "hasReportDefinition": info.has_report_definition,
                "hasLegacyReportLayout": info.has_legacy_report_layout
            }))
        }),
    };
    let (query, query_source) = load_query(&options)?;
    validate_query_shape(&query)?;
    let query_metadata = json!({
        "source": query_source,
        "lengthBytes": query.len(),
        "lengthChars": query.chars().count(),
        "fingerprint": format!("fnv64:{}", fingerprint_hex(&query)),
        "textReturned": false
    });
    let base = BasePayloadContext {
        target: &target,
        options: &options,
        query_metadata: &query_metadata,
        validation: &validation_json,
    };

    if validation_json["ok"] != Value::Bool(true) {
        return Ok(base.render(
            false,
            EXIT_VALIDATION_FAILED,
            "document-validation",
            json!({
                "code": "document_validation_failed",
                "message": "Local document validation failed before the Desktop DAX bridge was contacted."
            }),
        ));
    }

    if !cfg!(windows) {
        return Ok(base.render(
            false,
            EXIT_ORACLE_UNAVAILABLE,
            "platform",
            json!({
                "code": "desktop_platform_unsupported",
                "message": format!("Desktop DAX execution is Windows-only; current platform is {}.", std::env::consts::OS)
            }),
        ));
    }

    if !oracle_enabled() {
        return Ok(base.render(
            false,
            EXIT_ORACLE_UNAVAILABLE,
            "oracle-opt-in",
            json!({
                "code": "oracle_disabled",
                "message": "Set POWERBI_DESKTOP_ORACLE=1 to opt in to the local Desktop bridge."
            }),
        ));
    }

    #[cfg(windows)]
    {
        return execute_windows(&target, &query, &query_metadata, &validation_json, &options);
    }

    #[allow(unreachable_code)]
    Err(CliError::unexpected(
        "Desktop DAX execution platform dispatch failed",
    ))
}

struct BasePayloadContext<'a> {
    target: &'a ResolvedDesktopTarget,
    options: &'a ExecuteOptions,
    query_metadata: &'a Value,
    validation: &'a Value,
}

impl BasePayloadContext<'_> {
    fn render(&self, ok: bool, exit_code: i32, stage: &str, diagnostic: Value) -> Value {
        json!({
            "schema": "powerbi-cli.model.dax.execute.v2",
            "ok": ok,
            "exitCode": exit_code,
            "document": self.target.artifact_json(),
            "query": self.query_metadata,
            "safety": {
                "readOnlyQueryFormsOnly": true,
                "allowDataRead": self.options.allow_data_read,
                "desktopOracleEnabled": oracle_enabled(),
                "exactOpenProjectMatchRequired": true,
                "autoLaunch": false,
                "modelWrites": false,
                "queryTextReturned": false,
                "resultDataMayBeSensitive": true
            },
            "limits": {
                "maxRows": self.options.max_rows,
                "maxCellChars": self.options.max_cell_chars,
                "timeoutMs": self.options.timeout_ms,
                "maxQueryBytes": MAX_QUERY_BYTES
            },
            "stage": stage,
            "diagnostics": [diagnostic],
            "validation": self.validation,
            "next": [
                "Keep the exact PBIP/PBIX document open in Power BI Desktop and rerun with both explicit opt-ins.",
                "Use --max-rows, --max-cell-chars, and --timeout-ms to tighten result bounds."
            ]
        })
    }
}

#[cfg(windows)]
fn execute_windows(
    target: &ResolvedDesktopTarget,
    query: &str,
    query_metadata: &Value,
    validation: &Value,
    options: &ExecuteOptions,
) -> CliResult<Value> {
    let base = BasePayloadContext {
        target,
        options,
        query_metadata,
        validation,
    };
    let mut runtime = RuntimeDir::create()?;
    let script_path = runtime.path.join("execute-dax.ps1");
    let query_path = runtime.path.join("query.dax");
    let adomd_copy_path = runtime.path.join("Microsoft.PowerBI.AdomdClient.dll");
    fs::write(&script_path, DAX_EXECUTE_SCRIPT).map_err(|err| {
        CliError::unexpected(format!("write temporary Desktop DAX bridge script: {err}"))
    })?;
    fs::write(&query_path, query.as_bytes())
        .map_err(|err| CliError::unexpected(format!("write temporary Desktop DAX query: {err}")))?;

    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .arg("-DocumentPath")
        .arg(windows_argument_path(&target.artifact_path))
        .arg("-QueryPath")
        .arg(&query_path)
        .arg("-AdomdCopyPath")
        .arg(&adomd_copy_path)
        .arg("-MaxRows")
        .arg(options.max_rows.to_string())
        .arg("-MaxCellChars")
        .arg(options.max_cell_chars.to_string())
        .arg("-TimeoutMs")
        .arg(options.timeout_ms.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let execution = run_command_with_timeout(command, Duration::from_millis(options.timeout_ms));
    let (bridge, process_diagnostic) = match execution {
        Ok(Timed::Completed(output)) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let text = text.trim().trim_start_matches('\u{feff}');
            match serde_json::from_str::<BridgeResult>(text) {
                Ok(result) => (Some(result), None),
                Err(err) => (
                    None,
                    Some(json!({
                        "code": "bridge_output_invalid",
                        "message": format!("Desktop DAX bridge returned invalid JSON: {err}")
                    })),
                ),
            }
        }
        Ok(Timed::Completed(output)) => (
            None,
            Some(json!({
                "code": "bridge_process_failed",
                "message": format!(
                    "Desktop DAX bridge process failed: {}",
                    bounded_message(&String::from_utf8_lossy(&output.stderr))
                )
            })),
        ),
        Ok(Timed::TimedOut) => (
            None,
            Some(json!({
                "code": "bridge_timeout",
                "message": format!("Desktop DAX bridge exceeded the {} ms watchdog.", options.timeout_ms)
            })),
        ),
        Err(err) => (
            None,
            Some(json!({
                "code": "bridge_process_error",
                "message": format!("Desktop DAX bridge could not run: {err}")
            })),
        ),
    };
    let cleanup_succeeded = runtime.cleanup();

    if let Some(diagnostic) = process_diagnostic {
        let mut payload = base.render(false, EXIT_ORACLE_FAILED, "bridge-process", diagnostic);
        payload["runtime"] = json!({"temporaryFilesRemoved": cleanup_succeeded});
        return Ok(payload);
    }

    let bridge = bridge.expect("bridge result or process diagnostic");
    if !bridge.ok {
        let query_failure = bridge.stage == "query-execution";
        let engine = bridge_engine_json(&bridge);
        let exit_code = if query_failure {
            EXIT_VALIDATION_FAILED
        } else {
            EXIT_ORACLE_UNAVAILABLE
        };
        let mut payload = base.render(
            false,
            exit_code,
            &bridge.stage,
            json!({
                "code": if query_failure { "dax_query_failed" } else { "desktop_model_unavailable" },
                "errorType": bridge.error_type,
                "message": bridge.message.unwrap_or_else(|| "Desktop DAX execution failed without a message.".to_string())
            }),
        );
        payload["engine"] = engine;
        payload["runtime"] = json!({"temporaryFilesRemoved": cleanup_succeeded});
        return Ok(payload);
    }

    let engine = bridge_engine_json(&bridge);
    let rows = bridge
        .rows
        .into_iter()
        .map(|row| Value::Array(row.values))
        .collect::<Vec<_>>();
    let columns = bridge
        .columns
        .iter()
        .map(|column| {
            json!({
                "ordinal": column.ordinal,
                "name": column.name,
                "dataType": column.data_type
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.model.dax.execute.v2",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "document": target.artifact_json(),
        "query": query_metadata,
        "safety": {
            "readOnlyQueryFormsOnly": true,
            "allowDataRead": options.allow_data_read,
            "desktopOracleEnabled": true,
            "exactOpenProjectMatchRequired": true,
            "autoLaunch": false,
            "modelWrites": false,
            "queryTextReturned": false,
            "resultDataMayBeSensitive": true
        },
        "limits": {
            "maxRows": options.max_rows,
            "maxCellChars": options.max_cell_chars,
            "timeoutMs": options.timeout_ms,
            "maxQueryBytes": MAX_QUERY_BYTES
        },
        "stage": bridge.stage,
        "engine": engine,
        "columns": columns,
        "rows": rows,
        "counts": {
            "rows": bridge.row_count,
            "columns": bridge.column_count,
            "truncatedCells": bridge.truncated_cells
        },
        "truncation": {
            "rows": bridge.truncated,
            "cells": bridge.truncated_cells > 0
        },
        "runtime": {
            "temporaryFilesRemoved": cleanup_succeeded
        },
        "diagnostics": [],
        "validation": validation,
        "next": [
            "Treat returned rows as potentially sensitive model data.",
            "Use model dax lint for offline static checks and execute for live engine proof."
        ]
    }))
}

#[cfg(windows)]
fn bridge_engine_json(bridge: &BridgeResult) -> Value {
    json!({
        "kind": "power-bi-desktop-local-analysis-services",
        "desktopProcessId": bridge.desktop_process_id,
        "modelProcessId": bridge.model_process_id,
        "port": bridge.port,
        "desktopVersion": bridge.desktop_version
    })
}

fn parse_args(args: &[String]) -> CliResult<ExecuteOptions> {
    let mut options = ExecuteOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                if options.project.is_some() {
                    return Err(duplicate_flag("--project"));
                }
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--query" => {
                if options.query.is_some() {
                    return Err(duplicate_flag("--query"));
                }
                options.query = Some(take_value(args, &mut i, "--query")?);
            }
            "--query-file" => {
                if options.query_file.is_some() {
                    return Err(duplicate_flag("--query-file"));
                }
                options.query_file = Some(PathBuf::from(take_value(args, &mut i, "--query-file")?));
            }
            "--allow-data-read" => {
                options.allow_data_read = true;
                i += 1;
            }
            "--max-rows" => {
                let value = take_value(args, &mut i, "--max-rows")?;
                options.max_rows = bounded_u64(&value, "--max-rows", 1, MAX_ROWS_LIMIT)?;
            }
            "--max-cell-chars" => {
                let value = take_value(args, &mut i, "--max-cell-chars")?;
                options.max_cell_chars =
                    bounded_u64(&value, "--max-cell-chars", 1, MAX_CELL_CHARS_LIMIT)?;
            }
            "--timeout-ms" => {
                let value = take_value(args, &mut i, "--timeout-ms")?;
                options.timeout_ms = bounded_u64(&value, "--timeout-ms", 1_000, MAX_TIMEOUT_MS)?;
            }
            other if !other.starts_with('-') && options.project.is_none() => {
                options.project = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model dax execute flag: {other}"
                ))
                .with_hint("Run capabilities for the exact Desktop DAX execution contract.")
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"model dax execute\"",
                ));
            }
        }
    }
    if options.query.is_some() == options.query_file.is_some() {
        return Err(CliError::invalid_args(
            "model dax execute requires exactly one of --query or --query-file",
        )
        .with_hint("Use --query for a short expression or --query-file <path|-> for file/stdin input.")
        .with_suggested_command(
            "powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> --query-file <query.dax> --allow-data-read --json",
        ));
    }
    Ok(options)
}

fn duplicate_flag(flag: &str) -> CliError {
    CliError::invalid_args(format!("{flag} may be provided only once"))
        .with_hint("Pass one explicit project and one explicit DAX query source.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"model dax execute\"")
}

fn bounded_u64(value: &str, flag: &str, min: u64, max: u64) -> CliResult<u64> {
    let parsed = value.parse::<u64>().map_err(|_| {
        CliError::invalid_args(format!("{flag} requires an integer from {min} to {max}"))
    })?;
    if !(min..=max).contains(&parsed) {
        return Err(CliError::invalid_args(format!(
            "{flag} must be from {min} to {max}; got {parsed}"
        )));
    }
    Ok(parsed)
}

fn load_query(options: &ExecuteOptions) -> CliResult<(String, Value)> {
    let (query, source) = if let Some(query) = &options.query {
        (query.clone(), json!({"kind": "inline"}))
    } else if let Some(path) = &options.query_file {
        if path == Path::new("-") {
            let mut query = String::new();
            io::stdin().read_to_string(&mut query).map_err(|err| {
                CliError::unexpected(format!("read DAX query from standard input: {err}"))
            })?;
            (query, json!({"kind": "stdin"}))
        } else {
            let query = fs::read_to_string(path).map_err(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    CliError::file_not_found(format!(
                        "DAX query file not found: {}",
                        path.display()
                    ))
                } else {
                    CliError::unexpected(format!("read DAX query file {}: {err}", path.display()))
                }
            })?;
            (
                query,
                json!({"kind": "file", "path": canonical_display(path)}),
            )
        }
    } else {
        unreachable!("query source checked during argument parsing")
    };
    if query.len() > MAX_QUERY_BYTES {
        return Err(CliError::invalid_args(format!(
            "DAX query exceeds the {MAX_QUERY_BYTES}-byte safety limit"
        ))
        .with_hint(
            "Narrow the query or split independent EVALUATE statements into separate calls.",
        ));
    }
    Ok((query, source))
}

fn validate_query_shape(query: &str) -> CliResult<()> {
    if query.trim().is_empty() {
        return Err(CliError::invalid_args("DAX query must not be empty"));
    }
    if query.contains('\0') {
        return Err(CliError::invalid_args(
            "DAX query must not contain NUL bytes",
        ));
    }
    let masked = mask_dax_literals_and_comments(query);
    let tokens = masked
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>();
    match tokens.first().map(String::as_str) {
        Some("EVALUATE") => Ok(()),
        Some("DEFINE") if tokens.iter().any(|token| token == "EVALUATE") => Ok(()),
        _ => Err(CliError::invalid_args(
            "model dax execute accepts only DAX query forms beginning with EVALUATE or DEFINE ... EVALUATE",
        )
        .with_hint(
            "Model mutation/XMLA payloads are refused. DEFINE declarations are query-scoped and must be followed by EVALUATE.",
        )
        .with_suggested_command(
            "powerbi-cli model dax execute --project <project-dir-or.pbip-or.pbix> --query \"EVALUATE ROW(\\\"Value\\\", 1)\" --allow-data-read --json",
        )),
    }
}

fn mask_dax_literals_and_comments(query: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        DoubleQuoted,
        SingleQuoted,
        Bracketed,
        LineComment,
        BlockComment,
    }

    let chars = query.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(query.len());
    let mut state = State::Code;
    let mut i = 0;
    while i < chars.len() {
        let current = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            State::Code => match (current, next) {
                ('/', Some('/')) | ('-', Some('-')) => {
                    result.push(' ');
                    result.push(' ');
                    state = State::LineComment;
                    i += 2;
                    continue;
                }
                ('/', Some('*')) => {
                    result.push(' ');
                    result.push(' ');
                    state = State::BlockComment;
                    i += 2;
                    continue;
                }
                ('"', _) => {
                    result.push(' ');
                    state = State::DoubleQuoted;
                }
                ('\'', _) => {
                    result.push(' ');
                    state = State::SingleQuoted;
                }
                ('[', _) => {
                    result.push(' ');
                    state = State::Bracketed;
                }
                _ => result.push(current),
            },
            State::DoubleQuoted => {
                result.push(if current == '\n' { '\n' } else { ' ' });
                if current == '"' {
                    if next == Some('"') {
                        result.push(' ');
                        i += 2;
                        continue;
                    }
                    state = State::Code;
                }
            }
            State::SingleQuoted => {
                result.push(if current == '\n' { '\n' } else { ' ' });
                if current == '\'' {
                    if next == Some('\'') {
                        result.push(' ');
                        i += 2;
                        continue;
                    }
                    state = State::Code;
                }
            }
            State::Bracketed => {
                result.push(if current == '\n' { '\n' } else { ' ' });
                if current == ']' {
                    if next == Some(']') {
                        result.push(' ');
                        i += 2;
                        continue;
                    }
                    state = State::Code;
                }
            }
            State::LineComment => {
                if current == '\n' {
                    result.push('\n');
                    state = State::Code;
                } else {
                    result.push(' ');
                }
            }
            State::BlockComment => {
                result.push(if current == '\n' { '\n' } else { ' ' });
                if current == '*' && next == Some('/') {
                    result.push(' ');
                    state = State::Code;
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    result
}

fn oracle_enabled() -> bool {
    std::env::var("POWERBI_DESKTOP_ORACLE")
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn fingerprint_hex(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(windows)]
fn bounded_message(message: &str) -> String {
    let trimmed = message.trim();
    let bounded = trimmed.chars().take(2_000).collect::<String>();
    if bounded.is_empty() {
        "no stderr was returned".to_string()
    } else if trimmed.chars().count() > 2_000 {
        format!("{bounded}…")
    } else {
        bounded
    }
}

#[cfg(windows)]
fn windows_argument_path(path: &Path) -> String {
    let value = path.as_os_str().to_string_lossy();
    if let Some(stripped) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{stripped}")
    } else if let Some(stripped) = value.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        value.into_owned()
    }
}

#[cfg(windows)]
struct RuntimeDir {
    path: PathBuf,
    cleaned: bool,
}

#[cfg(windows)]
impl RuntimeDir {
    fn create() -> CliResult<Self> {
        let root = std::env::temp_dir();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..64 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "powerbi-cli-dax-{}-{now}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        cleaned: false,
                    });
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => {
                    return Err(CliError::unexpected(format!(
                        "create temporary Desktop DAX bridge directory: {err}"
                    )));
                }
            }
        }
        Err(CliError::unexpected(
            "could not allocate a unique temporary Desktop DAX bridge directory",
        ))
    }

    fn cleanup(&mut self) -> bool {
        if self.cleaned {
            return true;
        }
        match fs::remove_dir_all(&self.path) {
            Ok(()) => {
                self.cleaned = true;
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(windows)]
impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(windows)]
const DAX_EXECUTE_SCRIPT: &str = r#"
param(
    [Parameter(Mandatory = $true)][string]$DocumentPath,
    [Parameter(Mandatory = $true)][string]$QueryPath,
    [Parameter(Mandatory = $true)][string]$AdomdCopyPath,
    [Parameter(Mandatory = $true)][int]$MaxRows,
    [Parameter(Mandatory = $true)][int]$MaxCellChars,
    [Parameter(Mandatory = $true)][int]$TimeoutMs
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$stage = 'desktop-discovery'
$desktop = $null
$model = $null
$port = $null
$desktopVersion = $null
$connection = $null
$reader = $null
try {
    $documentFull = [IO.Path]::GetFullPath($DocumentPath)
    $documentExtension = [IO.Path]::GetExtension($documentFull)
    if (
        -not [string]::Equals($documentExtension, '.pbip', [StringComparison]::OrdinalIgnoreCase) -and
        -not [string]::Equals($documentExtension, '.pbix', [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw "Unsupported Desktop document extension: $documentExtension"
    }
    $processes = @(Get-CimInstance Win32_Process -ErrorAction Stop)
    $desktopMatches = [System.Collections.Generic.List[object]]::new()
    foreach ($candidate in @($processes | Where-Object { $_.Name -like 'PBIDesktop*.exe' })) {
        $tokens = [System.Collections.Generic.List[string]]::new()
        foreach ($match in [regex]::Matches([string]$candidate.CommandLine, '"(?<value>[^\"]+)"')) {
            [void]$tokens.Add([string]$match.Groups['value'].Value)
        }
        foreach ($token in ([string]$candidate.CommandLine -split '\s+')) {
            [void]$tokens.Add($token.Trim('"'))
        }
        $matched = $false
        foreach ($token in $tokens) {
            if (-not $token.EndsWith($documentExtension, [StringComparison]::OrdinalIgnoreCase)) {
                continue
            }
            try {
                $candidateDocument = [IO.Path]::GetFullPath($token)
                if ([string]::Equals($candidateDocument, $documentFull, [StringComparison]::OrdinalIgnoreCase)) {
                    $matched = $true
                    break
                }
            } catch {}
        }
        if ($matched) {
            [void]$desktopMatches.Add($candidate)
        }
    }
    if ($desktopMatches.Count -eq 0) {
        throw "No running Power BI Desktop process has the exact document open: $documentFull"
    }
    if ($desktopMatches.Count -ne 1) {
        throw "Expected one Power BI Desktop process for the exact document, found $($desktopMatches.Count)."
    }
    $desktop = $desktopMatches[0]

    $stage = 'model-discovery'
    $descendantIds = [System.Collections.Generic.HashSet[int]]::new()
    [void]$descendantIds.Add([int]$desktop.ProcessId)
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($candidate in $processes) {
            if (
                $descendantIds.Contains([int]$candidate.ParentProcessId) -and
                -not $descendantIds.Contains([int]$candidate.ProcessId)
            ) {
                [void]$descendantIds.Add([int]$candidate.ProcessId)
                $changed = $true
            }
        }
    }
    $models = [System.Collections.Generic.List[object]]::new()
    foreach ($candidate in @($processes | Where-Object { $_.Name -eq 'msmdsrv.exe' })) {
        if (-not $descendantIds.Contains([int]$candidate.ProcessId)) {
            continue
        }
        $workspaceMatch = [regex]::Match(
            [string]$candidate.CommandLine,
            '(?:^|\s)-s\s+(?:"(?<quoted>[^\"]+)"|(?<bare>\S+))',
            [Text.RegularExpressions.RegexOptions]::IgnoreCase
        )
        if (-not $workspaceMatch.Success) {
            continue
        }
        $workspace = if ($workspaceMatch.Groups['quoted'].Success) {
            $workspaceMatch.Groups['quoted'].Value
        } else {
            $workspaceMatch.Groups['bare'].Value
        }
        $portFile = Join-Path $workspace 'msmdsrv.port.txt'
        if (-not (Test-Path -LiteralPath $portFile -PathType Leaf)) {
            continue
        }
        $portText = [IO.File]::ReadAllText($portFile, [Text.Encoding]::Unicode)
        $portMatch = [regex]::Match($portText, '\d+')
        if (-not $portMatch.Success) {
            continue
        }
        $candidatePort = [int]$portMatch.Value
        if ($candidatePort -lt 1 -or $candidatePort -gt 65535) {
            continue
        }
        [void]$models.Add([pscustomobject]@{
            process = $candidate
            port = $candidatePort
        })
    }
    if ($models.Count -eq 0) {
        throw 'The exact Desktop process has no discoverable local semantic-model engine.'
    }
    if ($models.Count -ne 1) {
        throw "Expected one semantic-model engine below the exact Desktop process, found $($models.Count)."
    }
    $model = $models[0].process
    $port = [int]$models[0].port

    $stage = 'assembly-load'
    if ([string]::IsNullOrWhiteSpace([string]$desktop.ExecutablePath)) {
        throw 'Power BI Desktop executable path is unavailable from the process inventory.'
    }
    $desktopVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo([string]$desktop.ExecutablePath).FileVersion
    $adomdSource = Join-Path (Split-Path -Parent ([string]$desktop.ExecutablePath)) 'Microsoft.PowerBI.AdomdClient.dll'
    if (-not (Test-Path -LiteralPath $adomdSource -PathType Leaf)) {
        throw 'Power BI Desktop did not expose its bundled Microsoft.PowerBI.AdomdClient.dll.'
    }
    Copy-Item -LiteralPath $adomdSource -Destination $AdomdCopyPath -Force
    # LoadFrom avoids Add-Type's eager exported-type scan, which can fail on optional
    # Desktop-only types even when the ADOMD connection types are usable.
    [void][Reflection.Assembly]::LoadFrom($AdomdCopyPath)

    $stage = 'connection'
    $query = [IO.File]::ReadAllText($QueryPath, [Text.Encoding]::UTF8)
    $connection = [Microsoft.AnalysisServices.AdomdClient.AdomdConnection]::new(
        "Data Source=localhost:$port;Application Name=powerbi-cli"
    )
    $connection.Open()
    $command = $connection.CreateCommand()
    $command.CommandText = $query
    $command.CommandTimeout = [Math]::Max(1, [int][Math]::Ceiling($TimeoutMs / 1000.0))

    $stage = 'query-execution'
    $reader = $command.ExecuteReader()
    $columns = [System.Collections.Generic.List[object]]::new()
    for ($ordinal = 0; $ordinal -lt $reader.FieldCount; $ordinal++) {
        $fieldType = $reader.GetFieldType($ordinal)
        [void]$columns.Add([pscustomobject]@{
            ordinal = $ordinal
            name = $reader.GetName($ordinal)
            dataType = if ($null -eq $fieldType) { 'System.Object' } else { $fieldType.FullName }
        })
    }

    $script:truncatedCells = 0
    function Convert-ResultCell {
        param([object]$Value)
        if ($null -eq $Value -or $Value -is [DBNull]) {
            return $null
        }
        if ($Value -is [DateTime]) {
            return $Value.ToString('o')
        }
        if ($Value -is [DateTimeOffset]) {
            return $Value.ToString('o')
        }
        if ($Value -is [TimeSpan] -or $Value -is [Guid]) {
            return $Value.ToString()
        }
        if ($Value -is [byte[]]) {
            $Value = [Convert]::ToBase64String($Value)
        }
        if ($Value -is [string] -and $Value.Length -gt $MaxCellChars) {
            $script:truncatedCells++
            return $Value.Substring(0, $MaxCellChars)
        }
        return $Value
    }

    $rows = [System.Collections.Generic.List[object]]::new()
    while ($rows.Count -lt $MaxRows -and $reader.Read()) {
        $values = [object[]]::new($reader.FieldCount)
        for ($ordinal = 0; $ordinal -lt $reader.FieldCount; $ordinal++) {
            $values[$ordinal] = Convert-ResultCell -Value $reader.GetValue($ordinal)
        }
        [void]$rows.Add([pscustomobject]@{ values = $values })
    }
    $truncated = ($rows.Count -ge $MaxRows -and $reader.Read())
    $result = [pscustomobject]@{
        ok = $true
        stage = 'completed'
        desktopProcessId = [int]$desktop.ProcessId
        modelProcessId = [int]$model.ProcessId
        port = $port
        desktopVersion = $desktopVersion
        columns = [object[]]$columns.ToArray()
        rows = [object[]]$rows.ToArray()
        rowCount = $rows.Count
        columnCount = $columns.Count
        truncated = $truncated
        truncatedCells = $script:truncatedCells
    }
} catch {
    $result = [pscustomobject]@{
        ok = $false
        stage = $stage
        message = $_.Exception.Message
        errorType = $_.Exception.GetType().FullName
        desktopProcessId = if ($null -eq $desktop) { $null } else { [int]$desktop.ProcessId }
        modelProcessId = if ($null -eq $model) { $null } else { [int]$model.ProcessId }
        port = $port
        desktopVersion = $desktopVersion
    }
} finally {
    if ($null -ne $reader) {
        $reader.Dispose()
    }
    if ($null -ne $connection) {
        $connection.Dispose()
    }
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Compress -Depth 8))
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_evaluate_and_define_evaluate_queries() {
        assert!(validate_query_shape("EVALUATE ROW(\"Value\", 1)").is_ok());
        assert!(
            validate_query_shape(
                "-- comment\nDEFINE MEASURE 'Facts'[Scoped] = 1\nEVALUATE ROW(\"x\", [Scoped])"
            )
            .is_ok()
        );
    }

    #[test]
    fn refuses_non_query_and_fake_evaluate_tokens() {
        assert!(validate_query_shape("<Execute xmlns=\"urn:schemas\" />").is_err());
        assert!(validate_query_shape("DEFINE MEASURE 'x'[y] = 1").is_err());
        assert!(validate_query_shape("/* EVALUATE */ CREATE SOMETHING").is_err());
        assert!(validate_query_shape("\"EVALUATE\"").is_err());
    }

    #[test]
    fn parses_bounded_execute_contract() {
        let args = vec![
            "--project".to_string(),
            "sample.pbip".to_string(),
            "--query".to_string(),
            "EVALUATE ROW(\"x\", 1)".to_string(),
            "--allow-data-read".to_string(),
            "--max-rows".to_string(),
            "25".to_string(),
            "--max-cell-chars".to_string(),
            "100".to_string(),
            "--timeout-ms".to_string(),
            "5000".to_string(),
        ];
        let parsed = parse_args(&args).expect("parse execute options");
        assert_eq!(parsed.max_rows, 25);
        assert_eq!(parsed.max_cell_chars, 100);
        assert_eq!(parsed.timeout_ms, 5000);
        assert!(parsed.allow_data_read);
    }

    #[test]
    fn requires_exactly_one_query_source_and_valid_bounds() {
        assert!(parse_args(&["--project".into(), "x".into()]).is_err());
        assert!(
            parse_args(&[
                "--query".into(),
                "EVALUATE {1}".into(),
                "--query-file".into(),
                "q.dax".into()
            ])
            .is_err()
        );
        assert!(
            parse_args(&[
                "--query".into(),
                "EVALUATE {1}".into(),
                "--max-rows".into(),
                "0".into()
            ])
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn bridge_script_contains_exact_match_and_result_guards() {
        assert!(DAX_EXECUTE_SCRIPT.contains("exact document"));
        assert!(DAX_EXECUTE_SCRIPT.contains(".pbix"));
        assert!(DAX_EXECUTE_SCRIPT.contains("$rows.Count -lt $MaxRows"));
        assert!(DAX_EXECUTE_SCRIPT.contains("$Value.Length -gt $MaxCellChars"));
        assert!(DAX_EXECUTE_SCRIPT.contains("$command.CommandTimeout"));
        assert!(DAX_EXECUTE_SCRIPT.contains("Microsoft.PowerBI.AdomdClient.dll"));
    }
}
