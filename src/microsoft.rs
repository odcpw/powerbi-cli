use crate::child_process::{spawn_contained, terminate_after_exit, terminate_and_wait};
use crate::project_io::write_text_atomic;
use crate::{CliError, CliResult, EXIT_ORACLE_UNAVAILABLE, EXIT_SUCCESS, EXIT_VALIDATION_FAILED};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const INTEGRATION_LOCK_TEXT: &str = include_str!("../integrations/microsoft/integration-lock.json");
const PACKAGE_JSON_TEXT: &str = include_str!("../integrations/microsoft/package.json");
const PACKAGE_LOCK_TEXT: &str = include_str!("../integrations/microsoft/package-lock.json");
const ACTIVE_SCHEMA: &str = "powerbi-cli.microsoft-integrations-active.v1";
const STATUS_SCHEMA: &str = "powerbi-cli.integrations.status.v1";
const INSTALL_SCHEMA: &str = "powerbi-cli.integrations.install.v1";
const INSTALL_RECEIPT_NAME: &str = "powerbi-cli-install.json";
const ACTIVE_RECEIPT_NAME: &str = "active.json";
const CACHE_OVERRIDE: &str = "POWERBI_CLI_MICROSOFT_CACHE_DIR";
const CHILD_TIMEOUT: Duration = Duration::from_secs(30);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const OUTPUT_LIMIT: usize = 64 * 1024;
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);
static VERIFIED_TREES: OnceLock<Mutex<BTreeSet<(PathBuf, String)>>> = OnceLock::new();
#[cfg(test)]
static TREE_DIGEST_SCANS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MicrosoftComponent {
    ModelingMcp,
    ReportAuthoring,
    DesktopBridge,
}

impl MicrosoftComponent {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::ModelingMcp => "modeling-mcp",
            Self::ReportAuthoring => "report-authoring",
            Self::DesktopBridge => "desktop-bridge",
        }
    }

    fn parse(value: &str) -> CliResult<Self> {
        match value {
            "modeling-mcp" => Ok(Self::ModelingMcp),
            "report-authoring" => Ok(Self::ReportAuthoring),
            "desktop-bridge" => Ok(Self::DesktopBridge),
            _ => Err(CliError::invalid_args(format!(
                "unknown Microsoft integration component: {value}"
            ))
            .with_hint("Use modeling-mcp, report-authoring, or desktop-bridge.")
            .with_suggested_command("powerbi-cli integrations status --json")),
        }
    }

    fn all() -> [Self; 3] {
        [
            Self::ModelingMcp,
            Self::ReportAuthoring,
            Self::DesktopBridge,
        ]
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IntegrationLock {
    schema: String,
    lock_id: String,
    node_floor: u64,
    preview: bool,
    components: Vec<ComponentLock>,
    platform_artifacts: Vec<PlatformArtifactLock>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComponentLock {
    id: String,
    package: String,
    version: String,
    license: String,
    integrity: String,
    entrypoint: String,
    transport: String,
    protocol_version: Option<String>,
    server_name: Option<String>,
    server_version: Option<String>,
    tools_count: Option<usize>,
    tools_list_sha256: Option<String>,
    platforms: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformArtifactLock {
    platform: String,
    package: String,
    version: String,
    license: String,
    integrity: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActiveReceipt {
    schema: String,
    lock_id: String,
    lock_fingerprint: String,
    artifact_dir: String,
    tree_sha256: String,
    installed_at_unix_seconds: u64,
    node_version: String,
    components: BTreeMap<String, String>,
}

#[derive(Debug)]
struct LoadedLock {
    value: IntegrationLock,
    fingerprint: String,
    package_count: usize,
}

#[derive(Debug)]
pub(crate) struct InstalledMicrosoftTool {
    pub(crate) component: MicrosoftComponent,
    pub(crate) version: String,
    pub(crate) transport: String,
    pub(crate) entrypoint: PathBuf,
    pub(crate) artifact_dir: PathBuf,
    pub(crate) node: Option<PathBuf>,
    pub(crate) mcp_contract: Option<ModelingMcpContract>,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelingMcpContract {
    pub(crate) protocol_version: String,
    pub(crate) server_name: String,
    pub(crate) server_version: String,
    pub(crate) tools_count: usize,
    pub(crate) tools_list_sha256: String,
}

#[derive(Debug)]
pub(crate) struct BoundedChildOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout_bytes: Vec<u8>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) stdout_sha256: String,
    pub(crate) stderr_sha256: String,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
}

#[derive(Debug)]
struct CapturedStream {
    bytes: Vec<u8>,
    sha256: String,
    truncated: bool,
}

pub(crate) fn integrations_command(args: &[String]) -> CliResult<Value> {
    match args.split_first() {
        Some((action, rest)) if action == "status" => status_command(rest),
        Some((action, rest)) if action == "install" => install_command(rest),
        Some((action, _)) => Err(CliError::invalid_args(format!(
            "unknown integrations command: {action}"
        ))
        .with_hint("Run `powerbi-cli integrations status --json`.")
        .with_suggested_command("powerbi-cli integrations status --json")),
        None => Err(CliError::invalid_args(
            "integrations requires a subcommand: status or install",
        )
        .with_hint("Status is offline and read-only. Installation requires --allow-network.")
        .with_suggested_command("powerbi-cli integrations status --json")),
    }
}

pub(crate) fn shallow_summary_json() -> Value {
    match status(false, None) {
        Ok(status) => json!({
            "status": if status["ready"].as_bool().unwrap_or(false) { "pass" } else { "warn" },
            "ready": status["ready"],
            "platform": status["platform"],
            "node": status["node"],
            "components": status["components"],
            "lock": status["lock"],
            "cache": status["cache"],
            "childProcessesLaunched": 0,
            "next": ["powerbi-cli integrations status --json"]
        }),
        Err(error) => json!({
            "status": "warn",
            "ready": false,
            "code": error.code,
            "message": error.message,
            "childProcessesLaunched": 0,
            "next": ["powerbi-cli integrations status --json"]
        }),
    }
}

pub(crate) fn resolve_installed_component(
    component: MicrosoftComponent,
) -> CliResult<InstalledMicrosoftTool> {
    let lock = load_lock()?;
    let cache_root = cache_root()?;
    let receipt =
        read_active_receipt(&cache_root)?.ok_or_else(|| dependency_unavailable(component))?;
    verify_receipt_identity(&lock, &receipt)?;
    let artifact_dir = PathBuf::from(&receipt.artifact_dir);
    verify_tree_digest(&artifact_dir, &receipt.tree_sha256)?;
    let component_lock = component_lock(&lock.value, component)?;
    if !component_supported(component_lock, &host_platform()) {
        return Err(CliError::unsupported_feature(format!(
            "{} is unavailable on {}",
            component.id(),
            host_platform()
        ))
        .with_hint(
            "Run `powerbi-cli integrations status --deep --json` for the platform matrix.",
        ));
    }
    verify_component_files(&lock.value, component_lock, &artifact_dir, &host_platform())?;

    let entrypoint =
        resolved_entrypoint_path(&lock.value, component_lock, &artifact_dir, &host_platform())?;
    let node = (component != MicrosoftComponent::ModelingMcp)
        .then(|| find_executable("node"))
        .flatten();
    if component != MicrosoftComponent::ModelingMcp && node.is_none() {
        return Err(dependency_unavailable(component));
    }

    Ok(InstalledMicrosoftTool {
        component,
        version: component_lock.version.clone(),
        transport: component_lock.transport.clone(),
        entrypoint,
        artifact_dir,
        node,
        mcp_contract: modeling_mcp_contract(component_lock)?,
    })
}

fn modeling_mcp_contract(component: &ComponentLock) -> CliResult<Option<ModelingMcpContract>> {
    if component.id != MicrosoftComponent::ModelingMcp.id() {
        return Ok(None);
    }
    let missing =
        || integrity_error("modeling-mcp lock entry is missing its protocol/tool-surface identity");
    Ok(Some(ModelingMcpContract {
        protocol_version: component.protocol_version.clone().ok_or_else(missing)?,
        server_name: component.server_name.clone().ok_or_else(missing)?,
        server_version: component.server_version.clone().ok_or_else(missing)?,
        tools_count: component.tools_count.ok_or_else(missing)?,
        tools_list_sha256: component.tools_list_sha256.clone().ok_or_else(missing)?,
    }))
}

pub(crate) fn minimal_child_command(program: &Path, path_entries: &[PathBuf]) -> Command {
    let mut command = Command::new(program);
    configure_minimal_environment(&mut command, path_entries, None);
    command
}

pub(crate) fn validate_official_report(resolved: &crate::ResolvedProject) -> CliResult<Value> {
    ensure_single_report_artifact(resolved)?;
    let tool = resolve_installed_component(MicrosoftComponent::ReportAuthoring)?;
    let node = tool.node.as_ref().ok_or_else(|| {
        CliError::new(
            "dependency_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            "the exact report-authoring package requires Node 20+",
        )
    })?;
    let report_dir = fs::canonicalize(&resolved.report_dir).map_err(|error| {
        CliError::validation_failed(format!(
            "resolve official validator report directory {}: {error}",
            resolved.report_dir.display()
        ))
    })?;
    let mut command =
        minimal_child_command(node, &[parent_dir(node), parent_dir(&tool.entrypoint)]);
    command
        .arg(&tool.entrypoint)
        .arg("validate")
        .arg(&report_dir)
        .arg("--no-schema")
        .arg("--format")
        .arg("json");
    let output = run_bounded(command, official_report_timeout()).map_err(|error| {
        CliError::new(
            "backend_failed",
            crate::EXIT_ORACLE_FAILED,
            format!("official Microsoft report validation failed: {error}"),
        )
        .with_hint(
            "The exact child was terminated and reaped. Inspect integrations status --deep and the selected report artifact.",
        )
    })?;
    normalize_official_report_output(&tool, &report_dir, output)
}

fn ensure_single_report_artifact(resolved: &crate::ResolvedProject) -> CliResult<()> {
    let bytes = fs::read(&resolved.pbip_path).map_err(|error| {
        CliError::file_not_found(format!("read {}: {error}", resolved.pbip_path.display()))
    })?;
    if bytes.len() > 1024 * 1024 {
        return Err(CliError::validation_failed(
            "PBIP entry file exceeds the 1 MiB official-validation resolution limit",
        ));
    }
    let pbip: Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::validation_failed(format!(
            "parse JSON {}: {error}",
            resolved.pbip_path.display()
        ))
    })?;
    let report_count = pbip["artifacts"]
        .as_array()
        .map(|artifacts| {
            artifacts
                .iter()
                .filter(|artifact| {
                    artifact
                        .pointer("/report/path")
                        .and_then(Value::as_str)
                        .is_some()
                })
                .count()
        })
        .unwrap_or(0);
    if report_count != 1 {
        return Err(CliError::validation_failed(format!(
            "official report validation requires exactly one PBIP report artifact; found {report_count}"
        ))
        .with_hint("Pass a PBIP that identifies exactly one report."));
    }
    Ok(())
}

fn normalize_official_report_output(
    tool: &InstalledMicrosoftTool,
    report_dir: &Path,
    output: BoundedChildOutput,
) -> CliResult<Value> {
    if output.stdout_truncated {
        return Err(protocol_error(format!(
            "official report validator stdout exceeded {OUTPUT_LIMIT} bytes; stdoutSha256={}",
            output.stdout_sha256
        )));
    }
    let payload: Value = serde_json::from_slice(&output.stdout_bytes).map_err(|error| {
        protocol_error(format!(
            "parse official report validator JSON: {error}; stdoutSha256={}",
            output.stdout_sha256
        ))
    })?;
    if let Some(error) = payload.get("error") {
        let code = error["code"].as_str().unwrap_or("UNKNOWN_VENDOR_ERROR");
        let message = error["message"]
            .as_str()
            .map(bounded_redacted)
            .unwrap_or_else(|| "official validator returned an error envelope".to_string());
        return Err(CliError::new(
            "backend_failed",
            crate::EXIT_ORACLE_FAILED,
            format!("official report validator {code}: {message}"),
        )
        .with_hint("Inspect the exact report artifact and integration status."));
    }
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| protocol_error("official report validator response has no data object"))?;
    let result = data
        .get("result")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error("official report validator response has no data.result"))?;
    if !matches!(result, "succeeded" | "failed") {
        return Err(protocol_error(format!(
            "official report validator returned unknown result: {result}"
        )));
    }
    let error_count = required_vendor_count(data, "errorCount")?;
    let warning_count = required_vendor_count(data, "warningCount")?;
    let diagnostics = normalize_official_diagnostics(data.get("diagnostics"), report_dir)?;
    let observed_errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "error")
        .count() as u64;
    let observed_warnings = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "warning")
        .count() as u64;
    if error_count != observed_errors || warning_count != observed_warnings {
        return Err(protocol_error(format!(
            "official validator count mismatch: declared errors={error_count}, warnings={warning_count}; normalized errors={observed_errors}, warnings={observed_warnings}"
        )));
    }
    let ok = result == "succeeded" && error_count == 0;
    if ok != output.status.success() {
        return Err(protocol_error(format!(
            "official validator result/exit mismatch: result={result}, status={:?}",
            output.status.code()
        )));
    }
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "error")
        .cloned()
        .collect::<Vec<_>>();
    let warnings = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "warning")
        .cloned()
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.validation.microsoft-report.v1",
        "id": "microsoft-report",
        "ok": ok,
        "result": result,
        "official": true,
        "version": tool.version,
        "component": tool.component.id(),
        "transport": tool.transport,
        "schemaValidation": false,
        "schemaValidationReason": "--no-schema disables the validator's remote schema lookup; official consumed-surface rules still run",
        "readOnly": true,
        "counts": {
            "errors": error_count,
            "warnings": warning_count,
            "diagnosticCodes": diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic["code"].as_str())
                .collect::<BTreeSet<_>>()
                .len()
        },
        "errors": errors,
        "warnings": warnings,
        "diagnostics": diagnostics,
        "reportPath": crate::canonical_display(report_dir),
        "child": {
            "statusCode": output.status.code(),
            "stdoutSha256": output.stdout_sha256,
            "stderrSha256": output.stderr_sha256,
            "stdoutTruncated": output.stdout_truncated,
            "stderrTruncated": output.stderr_truncated,
            "stderr": output.stderr
        }
    }))
}

fn required_vendor_count(data: &Map<String, Value>, field: &str) -> CliResult<u64> {
    data.get(field).and_then(Value::as_u64).ok_or_else(|| {
        protocol_error(format!(
            "official report validator has no numeric data.{field}"
        ))
    })
}

fn normalize_official_diagnostics(
    diagnostics: Option<&Value>,
    report_dir: &Path,
) -> CliResult<Vec<Value>> {
    let Some(diagnostics) = diagnostics else {
        return Ok(Vec::new());
    };
    let groups = diagnostics
        .as_object()
        .ok_or_else(|| protocol_error("official report validator diagnostics must be an object"))?;
    let mut normalized = Vec::new();
    let report_display = report_dir.display().to_string();
    for (code, group) in groups {
        let severity = group["severity"]
            .as_str()
            .ok_or_else(|| protocol_error(format!("official diagnostic {code} has no severity")))?;
        if !matches!(severity, "error" | "warning") {
            return Err(protocol_error(format!(
                "official diagnostic {code} has unsupported severity {severity}"
            )));
        }
        let items = group["items"].as_array().ok_or_else(|| {
            protocol_error(format!("official diagnostic {code} has no items array"))
        })?;
        for item in items {
            let raw_message = item["message"].as_str().ok_or_else(|| {
                protocol_error(format!("official diagnostic {code} item has no message"))
            })?;
            let message = bounded_redacted(&raw_message.replace(&report_display, "<report>"));
            let file = item["file"]
                .as_str()
                .map(|file| normalize_official_file(file, report_dir));
            normalized.push(json!({
                "code": code,
                "severity": severity,
                "message": message,
                "file": file,
                "path": item["path"].as_str()
            }));
        }
    }
    Ok(normalized)
}

fn normalize_official_file(file: &str, report_dir: &Path) -> String {
    let file_normalized = normalized_vendor_path(file);
    let report_normalized = normalized_vendor_path(&report_dir.display().to_string());
    let comparable_file = if cfg!(windows) {
        file_normalized.to_ascii_lowercase()
    } else {
        file_normalized.clone()
    };
    let comparable_report = if cfg!(windows) {
        report_normalized.to_ascii_lowercase()
    } else {
        report_normalized.clone()
    };
    if comparable_file == comparable_report {
        return ".".to_string();
    }
    let prefix = format!("{}/", comparable_report.trim_end_matches('/'));
    if let Some(relative) = comparable_file.strip_prefix(&prefix) {
        let offset = file_normalized.len().saturating_sub(relative.len());
        return file_normalized[offset..].to_string();
    }
    bounded_redacted(file)
}

fn normalized_vendor_path(value: &str) -> String {
    let replaced = value.replace('\\', "/");
    replaced
        .strip_prefix("//?/")
        .unwrap_or(&replaced)
        .to_string()
}

fn protocol_error(message: impl Into<String>) -> CliError {
    CliError::new("protocol_failed", crate::EXIT_ORACLE_FAILED, message.into())
        .with_hint("The exact official validator response was rejected.")
}

fn official_report_timeout() -> Duration {
    if cfg!(debug_assertions)
        && let Ok(value) = env::var("POWERBI_CLI_TEST_REPORT_TIMEOUT_MS")
        && let Ok(milliseconds) = value.parse::<u64>()
        && (1..=30_000).contains(&milliseconds)
    {
        return Duration::from_millis(milliseconds);
    }
    CHILD_TIMEOUT
}

fn status_command(args: &[String]) -> CliResult<Value> {
    let mut deep = false;
    let mut component = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--deep" => {
                if deep {
                    return Err(CliError::invalid_args("--deep may be specified only once"));
                }
                deep = true;
                index += 1;
            }
            "--component" => {
                if component.is_some() {
                    return Err(CliError::invalid_args(
                        "--component may be specified only once",
                    ));
                }
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::invalid_args("--component requires a value"))?;
                component = Some(MicrosoftComponent::parse(value)?);
                index += 2;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown integrations status flag: {other}"
                ))
                .with_hint("Run `powerbi-cli integrations status --json`.")
                .with_suggested_command("powerbi-cli integrations status --json"));
            }
        }
    }
    status(deep, component)
}

fn status(deep: bool, selected: Option<MicrosoftComponent>) -> CliResult<Value> {
    let lock = load_lock()?;
    let platform = host_platform();
    let cache_root = cache_root()?;
    let receipt = read_active_receipt(&cache_root)?;
    let node_path = find_executable("node");
    let mut child_count = 0_u64;
    let mut node_version = receipt.as_ref().map(|receipt| receipt.node_version.clone());
    let mut node_meets_floor = node_path.is_some()
        && node_version
            .as_deref()
            .and_then(parse_node_major)
            .is_some_and(|major| major >= lock.value.node_floor);
    let mut node_diagnostic = Value::Null;

    if deep {
        if let Some(node) = &node_path {
            let mut command = minimal_child_command(node, &[parent_dir(node)]);
            command.arg("--version");
            child_count += 1;
            match run_bounded(command, CHILD_TIMEOUT) {
                Ok(output) if output.status.success() => {
                    let version = output.stdout.trim().to_string();
                    node_meets_floor = parse_node_major(&version)
                        .is_some_and(|major| major >= lock.value.node_floor);
                    node_version = Some(version);
                    node_diagnostic = child_provenance_json(&output);
                }
                Ok(output) => {
                    node_meets_floor = false;
                    node_diagnostic = child_provenance_json(&output);
                }
                Err(error) => {
                    node_meets_floor = false;
                    node_diagnostic = json!({"error": bounded_redacted(&error.to_string())});
                }
            }
        } else {
            node_meets_floor = false;
        }
    }

    let selected_components = selected.map_or_else(
        || MicrosoftComponent::all().to_vec(),
        |component| vec![component],
    );
    // A full deep status hashes the immutable graph once, then checks each package manifest.
    // A narrowed deep status uses the reusable resolver directly.
    let shared_deep_tree = if deep && selected.is_none() {
        receipt.as_ref().map_or(Value::Null, |receipt| {
            let artifact_dir = PathBuf::from(&receipt.artifact_dir);
            match verify_receipt_identity(&lock, receipt)
                .and_then(|_| verify_tree_digest(&artifact_dir, &receipt.tree_sha256))
            {
                Ok(()) => json!({"verified": true}),
                Err(error) => json!({
                    "verified": false,
                    "code": error.code,
                    "message": error.message
                }),
            }
        })
    } else {
        Value::Null
    };
    let mut component_results = Vec::new();
    let mut all_supported_ready = true;
    let status_context = ComponentStatusContext {
        lock: &lock,
        receipt: receipt.as_ref(),
        deep,
        node_ready: node_meets_floor,
        platform: &platform,
        single_selected: selected.is_some(),
        shared_deep_tree: &shared_deep_tree,
    };
    for component in selected_components {
        let component_lock = component_lock(&lock.value, component)?;
        let supported = component_supported(component_lock, &platform);
        let result = component_status(component, component_lock, supported, &status_context);
        child_count = child_count.saturating_add(
            result["deep"]["childProcessesLaunched"]
                .as_u64()
                .unwrap_or(0),
        );
        if component_blocks_readiness(
            selected.is_some(),
            supported,
            result["ready"].as_bool().unwrap_or(false),
        ) {
            all_supported_ready = false;
        }
        component_results.push(result);
    }

    Ok(json!({
        "schema": STATUS_SCHEMA,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "ready": all_supported_ready,
        "mode": if deep { "deep" } else { "shallow" },
        "selectedComponent": selected.map(MicrosoftComponent::id),
        "platform": {
            "host": platform,
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "wsl": is_wsl()
        },
        "lock": {
            "schema": lock.value.schema,
            "id": lock.value.lock_id,
            "fingerprint": lock.fingerprint,
            "packageCount": lock.package_count,
            "preview": lock.value.preview
        },
        "node": {
            "present": node_path.is_some(),
            "path": node_path.map(|path| path.to_string_lossy().into_owned()),
            "version": node_version,
            "versionSource": if deep { "executed" } else { "recorded-install" },
            "floorMajor": lock.value.node_floor,
            "meetsFloor": node_meets_floor,
            "deepDiagnostic": node_diagnostic
        },
        "cache": {
            "root": cache_root,
            "active": receipt.is_some(),
            "immutable": true
        },
        "components": component_results,
        "childProcessesLaunched": child_count,
        "next": if all_supported_ready {
            vec!["powerbi-cli integrations status --deep --json"]
        } else {
            vec!["powerbi-cli integrations install --allow-network --json"]
        }
    }))
}

fn component_blocks_readiness(selected: bool, supported: bool, ready: bool) -> bool {
    (supported || selected) && !ready
}

struct ComponentStatusContext<'a> {
    lock: &'a LoadedLock,
    receipt: Option<&'a ActiveReceipt>,
    deep: bool,
    node_ready: bool,
    platform: &'a str,
    single_selected: bool,
    shared_deep_tree: &'a Value,
}

fn component_status(
    component: MicrosoftComponent,
    component_lock: &ComponentLock,
    supported: bool,
    context: &ComponentStatusContext<'_>,
) -> Value {
    if !supported {
        return json!({
            "id": component.id(),
            "version": component_lock.version,
            "license": component_lock.license,
            "integrity": component_lock.integrity,
            "supported": false,
            "ready": false,
            "state": "unsupported-platform",
            "transport": component_lock.transport
        });
    }
    let Some(receipt) = context.receipt else {
        return json!({
            "id": component.id(),
            "version": component_lock.version,
            "license": component_lock.license,
            "integrity": component_lock.integrity,
            "supported": true,
            "ready": false,
            "state": "not-installed",
            "transport": component_lock.transport
        });
    };

    let artifact_dir = PathBuf::from(&receipt.artifact_dir);
    let receipt_matches = verify_receipt_identity(context.lock, receipt).is_ok()
        && receipt.components.get(component.id()) == Some(&component_lock.version);
    let entrypoint = resolved_entrypoint_path(
        &context.lock.value,
        component_lock,
        &artifact_dir,
        context.platform,
    )
    .ok();
    let shallow_files_present = entrypoint.as_ref().is_some_and(|path| path.is_file());
    let deep_result = if context.deep && receipt_matches && shallow_files_present {
        let resolved = if context.single_selected {
            resolve_installed_component(component).map(|tool| {
                json!({
                    "version": tool.version,
                    "transport": tool.transport,
                    "entrypoint": tool.entrypoint,
                    "artifactDir": tool.artifact_dir,
                    "node": tool.node,
                    "component": tool.component.id()
                })
            })
        } else if context.shared_deep_tree["verified"].as_bool() == Some(true) {
            verify_component_files(
                &context.lock.value,
                component_lock,
                &artifact_dir,
                context.platform,
            )
            .map(|_| {
                json!({
                    "version": component_lock.version,
                    "transport": component_lock.transport,
                    "entrypoint": entrypoint,
                    "artifactDir": artifact_dir,
                    "component": component.id()
                })
            })
        } else {
            Err(integrity_error(
                context.shared_deep_tree["message"]
                    .as_str()
                    .unwrap_or("immutable Microsoft cache tree verification failed"),
            ))
        };
        match resolved {
            Ok(tool) if component == MicrosoftComponent::ModelingMcp => {
                match resolve_installed_component(component)
                    .and_then(|installed| crate::mcp::deep_handshake(&installed))
                {
                    Ok(handshake) => json!({
                        "verified": true,
                        "method": "sha256-tree-package-manifests-and-mcp-handshake",
                        "tool": tool,
                        "handshake": handshake,
                        "childProcessesLaunched": 1
                    }),
                    Err(error) => json!({
                        "verified": false,
                        "method": "sha256-tree-package-manifests-and-mcp-handshake",
                        "code": error.code,
                        "message": error.message,
                        "childProcessesLaunched": 1
                    }),
                }
            }
            Ok(tool) => json!({
                "verified": true,
                "method": "sha256-tree-and-package-manifests",
                "tool": tool,
                "childProcessesLaunched": 0
            }),
            Err(error) => json!({
                "verified": false,
                "method": "sha256-tree-and-package-manifests",
                "code": error.code,
                "message": error.message
            }),
        }
    } else {
        Value::Null
    };
    let deep_verified = !context.deep || deep_result["verified"].as_bool().unwrap_or(false);
    let needs_node = component != MicrosoftComponent::ModelingMcp;
    let ready = receipt_matches
        && shallow_files_present
        && deep_verified
        && (!needs_node || context.node_ready);
    let state = if ready {
        if context.deep {
            "verified"
        } else {
            "installed"
        }
    } else if !receipt_matches {
        "lock-mismatch"
    } else if !shallow_files_present {
        "entrypoint-missing"
    } else if needs_node && !context.node_ready {
        "node-unavailable"
    } else {
        "integrity-failed"
    };
    json!({
        "id": component.id(),
        "version": component_lock.version,
        "license": component_lock.license,
        "integrity": component_lock.integrity,
        "supported": true,
        "ready": ready,
        "state": state,
        "transport": component_lock.transport,
        "cachePath": artifact_dir,
        "entrypoint": entrypoint,
        "deep": deep_result
    })
}

fn install_command(args: &[String]) -> CliResult<Value> {
    let mut allow_network = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--allow-network" => {
                if allow_network {
                    return Err(CliError::invalid_args(
                        "--allow-network may be specified only once",
                    ));
                }
                allow_network = true;
                index += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown integrations install flag: {other}"
                ))
                .with_hint("Installation is explicit: pass --allow-network.")
                .with_suggested_command(
                    "powerbi-cli integrations install --allow-network --json",
                ));
            }
        }
    }
    if !allow_network {
        return Err(CliError::invalid_args(
            "integrations install requires --allow-network",
        )
        .with_hint(
            "No registry, proxy, Node, or npm process was contacted. Review the exact pins, then opt in explicitly.",
        )
        .with_suggested_command(
            "powerbi-cli integrations install --allow-network --json",
        ));
    }
    install()
}

fn install() -> CliResult<Value> {
    let lock = load_lock()?;
    let platform = host_platform();
    let requested = MicrosoftComponent::all()
        .into_iter()
        .filter(|component| {
            component_lock(&lock.value, *component)
                .is_ok_and(|locked| component_supported(locked, &platform))
        })
        .collect::<Vec<_>>();
    for component in &requested {
        let component_lock = component_lock(&lock.value, *component)?;
        if !component_supported(component_lock, &platform) {
            return Err(CliError::unsupported_feature(format!(
                "{} cannot be installed on {}",
                component.id(),
                platform
            ))
            .with_hint("Run `powerbi-cli integrations status --json` for supported components."));
        }
    }

    let node = find_executable("node").ok_or_else(|| {
        CliError::new(
            "dependency_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            format!(
                "Node {}+ is required for Microsoft integrations",
                lock.value.node_floor
            ),
        )
        .with_hint("Install a supported Node release, then rerun the exact install command.")
    })?;
    let node_version = execute_node_version(&node, lock.value.node_floor)?;
    let npm = find_executable("npm").ok_or_else(|| {
        CliError::new(
            "dependency_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            "npm is required for the explicit Microsoft integration install",
        )
        .with_hint("Install npm alongside Node, then rerun the exact install command.")
    })?;
    let cache_root = cache_root()?;
    fs::create_dir_all(cache_root.join("artifacts")).map_err(|error| {
        CliError::unexpected(format!("create Microsoft integration cache: {error}"))
    })?;
    // A corrupt active pointer can be repaired by a successful install, but is never
    // replaced when staging fails.
    let previous = read_active_receipt(&cache_root).ok().flatten();
    let prior_version = previous.as_ref().map(|receipt| receipt.lock_id.clone());
    let artifact_dir = cache_root.join("artifacts").join(&lock.value.lock_id);
    let mut changes = vec![json!({
        "action": "activate-exact-microsoft-toolchain",
        "status": "planned",
        "path": artifact_dir,
        "lockId": lock.value.lock_id
    })];

    let activation_result = if artifact_dir.exists() {
        let receipt = read_install_receipt(&artifact_dir)?;
        verify_receipt_identity(&lock, &receipt)?;
        verify_tree_digest(&artifact_dir, &receipt.tree_sha256)?;
        for component in &requested {
            verify_component_files(
                &lock.value,
                component_lock(&lock.value, *component)?,
                &artifact_dir,
                &platform,
            )?;
        }
        activate(&cache_root, &receipt)?;
        "reused-verified-immutable-cache"
    } else {
        let stage = staging_path(&cache_root, &lock.value.lock_id);
        if stage.exists() {
            return Err(CliError::unexpected(format!(
                "refusing pre-existing integration staging directory: {}",
                stage.display()
            )));
        }
        fs::create_dir(&stage).map_err(|error| {
            CliError::unexpected(format!("create integration staging directory: {error}"))
        })?;
        let result = install_into_stage(
            &lock,
            &InstallStage {
                stage: &stage,
                artifact_dir: &artifact_dir,
                cache_root: &cache_root,
                node: &node,
                npm: &npm,
                node_version: &node_version,
                platform: &platform,
            },
        );
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = fs::remove_dir_all(&stage);
                return Err(error);
            }
        };
        fs::rename(&stage, &artifact_dir).map_err(|error| {
            let _ = fs::remove_dir_all(&stage);
            CliError::unexpected(format!(
                "atomically activate immutable integration artifact: {error}"
            ))
        })?;
        // The staged receipt already names the final immutable slot, so a crash after
        // rename never leaves a receipt that points back into the removed staging path.
        activate(&cache_root, &receipt)?;
        "installed-and-activated"
    };

    changes.push(json!({
        "action": "activate-exact-microsoft-toolchain",
        "status": "completed",
        "path": artifact_dir,
        "lockId": lock.value.lock_id
    }));
    let components = requested
        .iter()
        .map(|component| {
            let locked = component_lock(&lock.value, *component).expect("selected component lock");
            json!({
                "id": component.id(),
                "version": locked.version,
                "license": locked.license,
                "integrity": locked.integrity,
                "cachePath": artifact_dir,
                "activationResult": activation_result,
                "priorActiveVersion": prior_version
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": INSTALL_SCHEMA,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "readOnly": false,
        "mutates": true,
        "mutatesProject": false,
        "networkRequired": true,
        "networkAllowed": true,
        "lockId": lock.value.lock_id,
        "lockFingerprint": lock.fingerprint,
        "cachePath": artifact_dir,
        "activationResult": activation_result,
        "priorActiveVersion": prior_version,
        "components": components,
        "changes": changes,
        "next": ["powerbi-cli integrations status --deep --json"]
    }))
}

struct InstallStage<'a> {
    stage: &'a Path,
    artifact_dir: &'a Path,
    cache_root: &'a Path,
    node: &'a Path,
    npm: &'a Path,
    node_version: &'a str,
    platform: &'a str,
}

fn install_into_stage(lock: &LoadedLock, install: &InstallStage<'_>) -> CliResult<ActiveReceipt> {
    fs::write(install.stage.join("package.json"), PACKAGE_JSON_TEXT)
        .and_then(|_| fs::write(install.stage.join("package-lock.json"), PACKAGE_LOCK_TEXT))
        .and_then(|_| fs::write(install.stage.join(".npmrc-user-empty"), ""))
        .and_then(|_| fs::write(install.stage.join(".npmrc-global-empty"), ""))
        .map_err(|error| CliError::unexpected(format!("write exact npm stage inputs: {error}")))?;

    let mut command = Command::new(install.npm);
    command
        .current_dir(install.stage)
        .args([
            "ci",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--no-progress",
            "--omit=dev",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let path_entries = [parent_dir(install.node), parent_dir(install.npm)];
    configure_minimal_environment(
        &mut command,
        &path_entries,
        Some(InstallEnvironment {
            npm_cache: install.cache_root.join("npm-download-cache"),
            npm_user_config: install.stage.join(".npmrc-user-empty"),
            npm_global_config: install.stage.join(".npmrc-global-empty"),
        }),
    );
    let output = run_bounded(command, INSTALL_TIMEOUT).map_err(|error| {
        CliError::new(
            "backend_failed",
            crate::EXIT_ORACLE_FAILED,
            format!("exact npm install failed to execute: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(CliError::new(
            "backend_failed",
            crate::EXIT_ORACLE_FAILED,
            format!(
                "exact npm install failed; stderr={} stderrSha256={} truncated={}",
                bounded_redacted(&output.stderr),
                output.stderr_sha256,
                output.stderr_truncated
            ),
        )
        .with_hint(
            "The previous active cache was not changed. Resolve Node/npm/network access and retry.",
        ));
    }
    // Verify every top-level package even when --component narrowed the readiness response.
    for component in MicrosoftComponent::all() {
        let locked = component_lock(&lock.value, component)?;
        if component_supported(locked, install.platform)
            || component != MicrosoftComponent::ModelingMcp
        {
            verify_component_files(&lock.value, locked, install.stage, install.platform)?;
        }
    }
    let tree_sha256 = tree_sha256(install.stage)?;
    let receipt = new_install_receipt(
        lock,
        install.artifact_dir,
        tree_sha256,
        install.node_version,
    );
    write_receipt(&install.stage.join(INSTALL_RECEIPT_NAME), &receipt)?;
    Ok(receipt)
}

fn new_install_receipt(
    lock: &LoadedLock,
    artifact_dir: &Path,
    tree_sha256: String,
    node_version: &str,
) -> ActiveReceipt {
    ActiveReceipt {
        schema: ACTIVE_SCHEMA.to_string(),
        lock_id: lock.value.lock_id.clone(),
        lock_fingerprint: lock.fingerprint.clone(),
        artifact_dir: artifact_dir.to_string_lossy().into_owned(),
        tree_sha256,
        installed_at_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        node_version: node_version.to_string(),
        components: lock
            .value
            .components
            .iter()
            .map(|component| (component.id.clone(), component.version.clone()))
            .collect(),
    }
}

fn activate(cache_root: &Path, receipt: &ActiveReceipt) -> CliResult<()> {
    let text = serde_json::to_string_pretty(receipt)
        .map_err(|error| CliError::unexpected(format!("serialize activation receipt: {error}")))?;
    write_text_atomic(&cache_root.join(ACTIVE_RECEIPT_NAME), &format!("{text}\n"))
}

fn write_receipt(path: &Path, receipt: &ActiveReceipt) -> CliResult<()> {
    let text = serde_json::to_string_pretty(receipt)
        .map_err(|error| CliError::unexpected(format!("serialize install receipt: {error}")))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| CliError::unexpected(format!("write {}: {error}", path.display())))
}

fn load_lock() -> CliResult<LoadedLock> {
    let value: IntegrationLock = serde_json::from_str(INTEGRATION_LOCK_TEXT).map_err(|error| {
        integrity_error(format!(
            "parse committed Microsoft integration lock: {error}"
        ))
    })?;
    if value.schema != "powerbi-cli.microsoft-integrations-lock.v1"
        || value.lock_id.trim().is_empty()
        || value.node_floor < 20
    {
        return Err(integrity_error(
            "Microsoft integration lock schema, ID, or Node floor is invalid",
        ));
    }
    let package_lock: Value = serde_json::from_str(PACKAGE_LOCK_TEXT)
        .map_err(|error| integrity_error(format!("parse committed Microsoft npm lock: {error}")))?;
    if package_lock["lockfileVersion"].as_u64() != Some(3) {
        return Err(integrity_error(
            "Microsoft package-lock.json must use lockfileVersion 3",
        ));
    }
    let packages = package_lock["packages"]
        .as_object()
        .ok_or_else(|| integrity_error("Microsoft npm lock has no packages object"))?;
    let mut ids = BTreeSet::new();
    for component in &value.components {
        if !ids.insert(component.id.as_str()) {
            return Err(integrity_error(format!(
                "duplicate component ID in Microsoft integration lock: {}",
                component.id
            )));
        }
        verify_package_lock_entry(
            packages,
            &component.package,
            &component.version,
            &component.license,
            &component.integrity,
        )?;
        if component.entrypoint.contains("..") || Path::new(&component.entrypoint).is_absolute() {
            return Err(integrity_error(format!(
                "unsafe component entrypoint in integration lock: {}",
                component.entrypoint
            )));
        }
    }
    if ids != BTreeSet::from(["desktop-bridge", "modeling-mcp", "report-authoring"]) {
        return Err(integrity_error(
            "Microsoft integration lock must contain exactly the three canonical components",
        ));
    }
    for artifact in &value.platform_artifacts {
        verify_package_lock_entry(
            packages,
            &artifact.package,
            &artifact.version,
            &artifact.license,
            &artifact.integrity,
        )?;
    }
    for (path, package) in packages {
        if path.is_empty() {
            continue;
        }
        let version = package["version"].as_str().unwrap_or_default();
        let integrity = package["integrity"].as_str().unwrap_or_default();
        let resolved = package["resolved"].as_str().unwrap_or_default();
        if version.is_empty()
            || integrity.is_empty()
            || !integrity.starts_with("sha512-")
            || !resolved.starts_with("https://registry.npmjs.org/")
        {
            return Err(integrity_error(format!(
                "npm lock entry is incomplete or non-registry: {path}"
            )));
        }
    }
    let mut digest = Sha256::new();
    digest.update(INTEGRATION_LOCK_TEXT.as_bytes());
    digest.update([0]);
    digest.update(PACKAGE_LOCK_TEXT.as_bytes());
    Ok(LoadedLock {
        value,
        fingerprint: format!("sha256:{}", hex_digest(digest.finalize().as_slice())),
        package_count: packages.len().saturating_sub(1),
    })
}

fn verify_package_lock_entry(
    packages: &Map<String, Value>,
    package: &str,
    version: &str,
    license: &str,
    integrity: &str,
) -> CliResult<()> {
    let key = format!("node_modules/{package}");
    let entry = packages
        .get(&key)
        .ok_or_else(|| integrity_error(format!("npm lock is missing {package}")))?;
    if entry["version"] != version || entry["license"] != license || entry["integrity"] != integrity
    {
        return Err(integrity_error(format!(
            "npm lock does not match the exact version/license/integrity for {package}"
        )));
    }
    Ok(())
}

fn verify_receipt_identity(lock: &LoadedLock, receipt: &ActiveReceipt) -> CliResult<()> {
    if receipt.schema != ACTIVE_SCHEMA
        || receipt.lock_id != lock.value.lock_id
        || receipt.lock_fingerprint != lock.fingerprint
    {
        return Err(integrity_error(
            "active Microsoft integration receipt does not match the committed lock",
        ));
    }
    Ok(())
}

fn read_active_receipt(cache_root: &Path) -> CliResult<Option<ActiveReceipt>> {
    let path = cache_root.join(ACTIVE_RECEIPT_NAME);
    if !path.is_file() {
        return Ok(None);
    }
    let receipt = read_receipt(&path)?;
    let expected = cache_root.join("artifacts").join(&receipt.lock_id);
    if Path::new(&receipt.artifact_dir) != expected {
        return Err(integrity_error(
            "active Microsoft integration receipt points outside its immutable cache slot",
        ));
    }
    Ok(Some(receipt))
}

fn read_install_receipt(artifact_dir: &Path) -> CliResult<ActiveReceipt> {
    let receipt = read_receipt(&artifact_dir.join(INSTALL_RECEIPT_NAME))?;
    if Path::new(&receipt.artifact_dir) != artifact_dir {
        return Err(integrity_error(
            "Microsoft integration install receipt does not identify its immutable artifact directory",
        ));
    }
    Ok(receipt)
}

fn read_receipt(path: &Path) -> CliResult<ActiveReceipt> {
    let bytes = fs::read(path)
        .map_err(|error| integrity_error(format!("read Microsoft integration receipt: {error}")))?;
    if bytes.len() > OUTPUT_LIMIT {
        return Err(integrity_error(
            "Microsoft integration receipt exceeds the bounded size limit",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| integrity_error(format!("parse Microsoft integration receipt: {error}")))
}

fn verify_component_files(
    lock: &IntegrationLock,
    component: &ComponentLock,
    artifact_dir: &Path,
    platform: &str,
) -> CliResult<()> {
    verify_installed_package(artifact_dir, &component.package, &component.version)?;
    let entrypoint = package_dir(artifact_dir, &component.package).join(&component.entrypoint);
    if !entrypoint.is_file() {
        return Err(integrity_error(format!(
            "verified entrypoint is missing for {}: {}",
            component.id,
            entrypoint.display()
        )));
    }
    if component.id == "modeling-mcp" {
        let artifact = lock
            .platform_artifacts
            .iter()
            .find(|artifact| artifact.platform == platform)
            .ok_or_else(|| {
                CliError::unsupported_feature(format!(
                    "Modeling MCP has no pinned artifact for {platform}"
                ))
            })?;
        verify_installed_package(artifact_dir, &artifact.package, &artifact.version)?;
        let binary = platform_binary_path(artifact_dir, &artifact.package)?;
        if !binary.is_file() {
            return Err(integrity_error(format!(
                "pinned Modeling MCP platform binary is missing: {}",
                binary.display()
            )));
        }
    }
    Ok(())
}

fn verify_installed_package(artifact_dir: &Path, package: &str, version: &str) -> CliResult<()> {
    let manifest_path = package_dir(artifact_dir, package).join("package.json");
    let bytes = fs::read(&manifest_path)
        .map_err(|error| integrity_error(format!("read installed {package} manifest: {error}")))?;
    if bytes.len() > OUTPUT_LIMIT {
        return Err(integrity_error(format!(
            "installed {package} manifest exceeds the bounded size limit"
        )));
    }
    let manifest: Value = serde_json::from_slice(&bytes)
        .map_err(|error| integrity_error(format!("parse installed {package} manifest: {error}")))?;
    if manifest["name"] != package || manifest["version"] != version {
        return Err(integrity_error(format!(
            "installed package identity mismatch for {package}"
        )));
    }
    Ok(())
}

fn resolved_entrypoint_path(
    lock: &IntegrationLock,
    component: &ComponentLock,
    artifact_dir: &Path,
    platform: &str,
) -> CliResult<PathBuf> {
    if component.id == "modeling-mcp" {
        let artifact = lock
            .platform_artifacts
            .iter()
            .find(|artifact| artifact.platform == platform)
            .ok_or_else(|| {
                CliError::unsupported_feature(format!(
                    "Modeling MCP has no pinned artifact for {platform}"
                ))
            })?;
        platform_binary_path(artifact_dir, &artifact.package)
    } else {
        Ok(package_dir(artifact_dir, &component.package).join(&component.entrypoint))
    }
}

fn platform_binary_path(artifact_dir: &Path, package: &str) -> CliResult<PathBuf> {
    let package_root = package_dir(artifact_dir, package);
    let bytes = fs::read(package_root.join("package.json")).map_err(|error| {
        integrity_error(format!("read installed platform package manifest: {error}"))
    })?;
    let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| {
        integrity_error(format!(
            "parse installed platform package manifest: {error}"
        ))
    })?;
    let bin = manifest["bin"]
        .as_object()
        .and_then(|bins| bins.values().next())
        .and_then(Value::as_str)
        .ok_or_else(|| integrity_error("installed platform package has no binary entrypoint"))?;
    if bin.contains("..") || Path::new(bin).is_absolute() {
        return Err(integrity_error(
            "installed platform package has an unsafe binary entrypoint",
        ));
    }
    Ok(package_root.join(bin))
}

fn verify_tree_digest(artifact_dir: &Path, expected: &str) -> CliResult<()> {
    let canonical = fs::canonicalize(artifact_dir).map_err(|error| {
        integrity_error(format!(
            "canonicalize Microsoft integration artifact directory: {error}"
        ))
    })?;
    let key = (canonical, expected.to_string());
    let verified = VERIFIED_TREES.get_or_init(|| Mutex::new(BTreeSet::new()));
    if verified
        .lock()
        .map_err(|_| integrity_error("Microsoft integration verification cache lock poisoned"))?
        .contains(&key)
    {
        return Ok(());
    }
    let actual = tree_sha256(artifact_dir)?;
    if actual != expected {
        return Err(integrity_error(format!(
            "immutable Microsoft integration cache digest mismatch: expected {expected}, got {actual}"
        )));
    }
    verified
        .lock()
        .map_err(|_| integrity_error("Microsoft integration verification cache lock poisoned"))?
        .insert(key);
    Ok(())
}

fn tree_sha256(root: &Path) -> CliResult<String> {
    #[cfg(test)]
    TREE_DIGEST_SCANS.fetch_add(1, Ordering::Relaxed);
    if !root.is_dir() {
        return Err(integrity_error(format!(
            "Microsoft integration artifact directory is missing: {}",
            root.display()
        )));
    }
    let mut entries = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|entry| {
            entry.map_err(|error| {
                integrity_error(format!("walk Microsoft integration cache: {error}"))
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    entries.retain(|entry| {
        (entry.file_type().is_file() || entry.file_type().is_symlink())
            && entry.file_name() != OsStr::new(INSTALL_RECEIPT_NAME)
    });
    entries.sort_by_key(|entry| normalized_relative(root, entry.path()));
    let mut digest = Sha256::new();
    for entry in entries {
        let relative = normalized_relative(root, entry.path());
        digest.update(relative.as_bytes());
        digest.update([0]);
        if entry.file_type().is_symlink() {
            digest.update(b"symlink\0");
            let target = fs::read_link(entry.path()).map_err(|error| {
                integrity_error(format!("read cache symlink for digest: {error}"))
            })?;
            digest.update(target.as_os_str().to_string_lossy().as_bytes());
            digest.update([0]);
            continue;
        }
        digest.update(b"file\0");
        let mut file = fs::File::open(entry.path())
            .map_err(|error| integrity_error(format!("read cache file for digest: {error}")))?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| integrity_error(format!("hash cache file for digest: {error}")))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        digest.update([0]);
    }
    Ok(format!(
        "sha256:{}",
        hex_digest(digest.finalize().as_slice())
    ))
}

fn normalized_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn execute_node_version(node: &Path, floor: u64) -> CliResult<String> {
    let mut command = minimal_child_command(node, &[parent_dir(node)]);
    command.arg("--version");
    let output = run_bounded(command, CHILD_TIMEOUT).map_err(|error| {
        CliError::new(
            "dependency_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            format!("execute Node version check: {error}"),
        )
    })?;
    let version = output.stdout.trim().to_string();
    let major = parse_node_major(&version).ok_or_else(|| {
        CliError::new(
            "dependency_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            "Node returned an unrecognized version string",
        )
    })?;
    if !output.status.success() || major < floor {
        return Err(CliError::new(
            "dependency_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            format!("Node {floor}+ is required; resolved {version}"),
        ));
    }
    Ok(version)
}

fn parse_node_major(version: &str) -> Option<u64> {
    version
        .trim()
        .strip_prefix('v')
        .unwrap_or(version.trim())
        .split('.')
        .next()?
        .parse()
        .ok()
}

pub(crate) fn run_bounded(
    mut command: Command,
    timeout: Duration,
) -> io::Result<BoundedChildOutput> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn_contained(&mut command)?;
    let stdout = child
        .inner()
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child stdout was not piped"));
    let stdout = match stdout {
        Ok(stdout) => stdout,
        Err(error) => {
            let _ = terminate_and_wait(&mut child);
            return Err(error);
        }
    };
    let stderr = child
        .inner()
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("child stderr was not piped"));
    let stderr = match stderr {
        Ok(stderr) => stderr,
        Err(error) => {
            let _ = terminate_and_wait(&mut child);
            return Err(error);
        }
    };
    let stdout_thread = thread::spawn(move || capture_stream(stdout, OUTPUT_LIMIT));
    let stderr_thread = thread::spawn(move || capture_stream(stderr, OUTPUT_LIMIT));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break terminate_after_exit(&mut child, status)?,
            Ok(None) => {}
            Err(error) => {
                let _ = terminate_and_wait(&mut child);
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(error);
            }
        }
        if started.elapsed() >= timeout {
            let cleanup = terminate_and_wait(&mut child);
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            cleanup?;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("child exceeded {} ms and was reaped", timeout.as_millis()),
            ));
        }
        thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout_thread
        .join()
        .map_err(|_| io::Error::other("stdout capture thread panicked"))??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| io::Error::other("stderr capture thread panicked"))??;
    Ok(BoundedChildOutput {
        status,
        stdout_bytes: stdout.bytes.clone(),
        stdout: bounded_redacted(&String::from_utf8_lossy(&stdout.bytes)),
        stderr: bounded_redacted(&String::from_utf8_lossy(&stderr.bytes)),
        stdout_sha256: stdout.sha256,
        stderr_sha256: stderr.sha256,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
    })
}

fn capture_stream(mut reader: impl Read, limit: usize) -> io::Result<CapturedStream> {
    let mut retained = Vec::with_capacity(limit.min(8 * 1024));
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut total = 0_usize;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        total = total.saturating_add(count);
        if retained.len() < limit {
            let remaining = limit - retained.len();
            retained.extend_from_slice(&buffer[..count.min(remaining)]);
        }
    }
    Ok(CapturedStream {
        bytes: retained,
        sha256: format!("sha256:{}", hex_digest(digest.finalize().as_slice())),
        truncated: total > limit,
    })
}

pub(crate) fn bounded_redacted(text: &str) -> String {
    let home = env::var("USERPROFILE")
        .ok()
        .or_else(|| env::var("HOME").ok());
    let mut output = text
        .lines()
        .take(200)
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "authorization",
                "password",
                "passwd",
                "secret",
                "token",
                "connectionstring",
                "database_url",
                "npm_auth",
                "proxy-authorization",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
            {
                "[redacted]".to_string()
            } else if let Some(home) = &home {
                line.replace(home, "<home>")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if output.len() > OUTPUT_LIMIT {
        let mut boundary = OUTPUT_LIMIT;
        while !output.is_char_boundary(boundary) {
            boundary -= 1;
        }
        output.truncate(boundary);
    }
    output
}

fn child_provenance_json(output: &BoundedChildOutput) -> Value {
    json!({
        "statusCode": output.status.code(),
        "stdoutSha256": output.stdout_sha256,
        "stderrSha256": output.stderr_sha256,
        "stdoutTruncated": output.stdout_truncated,
        "stderrTruncated": output.stderr_truncated,
        "stderr": output.stderr
    })
}

#[derive(Debug)]
struct InstallEnvironment {
    npm_cache: PathBuf,
    npm_user_config: PathBuf,
    npm_global_config: PathBuf,
}

fn configure_minimal_environment(
    command: &mut Command,
    path_entries: &[PathBuf],
    install: Option<InstallEnvironment>,
) {
    command.env_clear();
    let mut unique = BTreeSet::<OsString>::new();
    for path in path_entries {
        unique.insert(path.as_os_str().to_os_string());
    }
    if let Ok(path) = env::join_paths(unique) {
        command.env("PATH", path);
    }
    for key in [
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("NO_UPDATE_NOTIFIER", "1")
        .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
        .env("NPM_CONFIG_AUDIT", "false")
        .env("NPM_CONFIG_FUND", "false")
        .env("NPM_CONFIG_PROGRESS", "false")
        .env("NPM_CONFIG_LOGLEVEL", "error")
        .env("NPM_CONFIG_REGISTRY", "https://registry.npmjs.org/");
    if let Some(install) = install {
        command
            .env("NPM_CONFIG_CACHE", install.npm_cache)
            .env("NPM_CONFIG_USERCONFIG", install.npm_user_config)
            .env("NPM_CONFIG_GLOBALCONFIG", install.npm_global_config);
    }
}

fn component_lock(
    lock: &IntegrationLock,
    component: MicrosoftComponent,
) -> CliResult<&ComponentLock> {
    lock.components
        .iter()
        .find(|locked| locked.id == component.id())
        .ok_or_else(|| integrity_error(format!("lock has no {} component", component.id())))
}

fn component_supported(component: &ComponentLock, platform: &str) -> bool {
    component.platforms.iter().any(|value| value == platform)
}

fn package_dir(artifact_dir: &Path, package: &str) -> PathBuf {
    package
        .split('/')
        .fold(artifact_dir.join("node_modules"), |path, part| {
            path.join(part)
        })
}

fn cache_root() -> CliResult<PathBuf> {
    if let Some(path) = env::var_os(CACHE_OVERRIDE) {
        let mut path = PathBuf::from(path);
        if path.as_os_str().is_empty() {
            return Err(CliError::invalid_args(format!(
                "{CACHE_OVERRIDE} must not be empty"
            )));
        }
        if !path.is_absolute() {
            path = env::current_dir()
                .map_err(|error| {
                    CliError::unexpected(format!("resolve current directory for cache: {error}"))
                })?
                .join(path);
        }
        return Ok(path);
    }
    if cfg!(windows) {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("powerbi-cli").join("microsoft"))
    } else if let Some(path) = env::var_os("XDG_CACHE_HOME") {
        Some(PathBuf::from(path).join("powerbi-cli").join("microsoft"))
    } else {
        env::var_os("HOME").map(|path| {
            PathBuf::from(path)
                .join(".cache")
                .join("powerbi-cli")
                .join("microsoft")
        })
    }
    .ok_or_else(|| {
        CliError::new(
            "dependency_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            "cannot resolve a user cache directory for Microsoft integrations",
        )
    })
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let candidates = executable_names(name);
    env::split_paths(&path)
        .flat_map(|directory| {
            candidates
                .iter()
                .map(move |candidate| directory.join(candidate))
        })
        .find(|candidate| candidate.is_file())
}

fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        if Path::new(name).extension().is_some() {
            vec![name.to_string()]
        } else {
            vec![
                format!("{name}.exe"),
                format!("{name}.cmd"),
                format!("{name}.bat"),
                name.to_string(),
            ]
        }
    } else {
        vec![name.to_string()]
    }
}

fn parent_dir(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn staging_path(cache_root: &Path, lock_id: &str) -> PathBuf {
    let counter = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
    cache_root.join(format!(
        ".staging-{lock_id}-{}-{counter}",
        std::process::id()
    ))
}

fn host_platform() -> String {
    format!("{}-{}", env::consts::OS, env::consts::ARCH)
}

fn is_wsl() -> bool {
    cfg!(target_os = "linux")
        && fs::read_to_string("/proc/sys/kernel/osrelease")
            .or_else(|_| fs::read_to_string("/proc/version"))
            .is_ok_and(|text| {
                let lower = text.to_ascii_lowercase();
                lower.contains("microsoft") || lower.contains("wsl")
            })
}

fn integrity_error(message: impl Into<String>) -> CliError {
    CliError::new(
        "integrity_failed",
        EXIT_VALIDATION_FAILED,
        message.into(),
    )
    .with_hint(
        "Do not use the cache. Restore the committed lock or rerun `integrations install --allow-network` into a clean immutable cache.",
    )
}

fn dependency_unavailable(component: MicrosoftComponent) -> CliError {
    CliError::new(
        "dependency_unavailable",
        EXIT_ORACLE_UNAVAILABLE,
        format!(
            "{} is not installed in the exact Microsoft integration cache",
            component.id()
        ),
    )
    .with_hint("Install the pinned graph explicitly; normal commands never download it.")
    .with_suggested_command("powerbi-cli integrations install --allow-network --json")
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysinfo::{Pid, ProcessesToUpdate, System};

    #[test]
    fn selected_unsupported_component_blocks_overall_readiness() {
        assert!(component_blocks_readiness(true, false, false));
        assert!(!component_blocks_readiness(false, false, false));
        assert!(component_blocks_readiness(false, true, false));
        assert!(!component_blocks_readiness(true, true, true));
    }

    #[test]
    fn successful_tree_verification_is_memoized_per_path_and_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("payload.txt"), "immutable").expect("payload");
        let expected = tree_sha256(temp.path()).expect("tree digest");
        TREE_DIGEST_SCANS.store(0, Ordering::Relaxed);

        verify_tree_digest(temp.path(), &expected).expect("first verification");
        verify_tree_digest(temp.path(), &expected).expect("cached verification");

        assert_eq!(TREE_DIGEST_SCANS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn staged_receipt_names_final_artifact_before_atomic_rename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stage = temp.path().join("stage");
        let artifact = temp.path().join("artifacts").join("exact-lock");
        fs::create_dir(&stage).expect("stage");
        fs::create_dir(artifact.parent().expect("artifact parent")).expect("artifacts");
        let lock = load_lock().expect("lock");
        let receipt = new_install_receipt(&lock, &artifact, "sha256:test".to_string(), "v22.14.0");
        write_receipt(&stage.join(INSTALL_RECEIPT_NAME), &receipt).expect("staged receipt");
        let staged = read_receipt(&stage.join(INSTALL_RECEIPT_NAME)).expect("read staged");
        assert_eq!(Path::new(&staged.artifact_dir), artifact);

        fs::rename(&stage, &artifact).expect("atomic rename");
        let installed = read_install_receipt(&artifact).expect("final receipt");
        assert_eq!(installed.artifact_dir, receipt.artifact_dir);
    }

    #[test]
    fn bounded_redaction_truncates_only_at_utf8_boundaries() {
        let input = format!("x{}", "é".repeat(OUTPUT_LIMIT));
        let output = bounded_redacted(&input);
        assert!(output.len() <= OUTPUT_LIMIT);
        assert!(output.starts_with('x'));
        assert!(output.chars().skip(1).all(|character| character == 'é'));
    }

    #[test]
    fn bounded_runner_reaps_descendants_that_inherit_its_pipes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let marker = temp.path().join("descendant.pid");
        let command = pipe_inheriting_descendant_command(&marker);
        let started = Instant::now();
        let output = run_bounded(command, Duration::from_secs(2)).expect("bounded child");
        assert!(output.status.success());
        assert!(started.elapsed() < Duration::from_secs(4));
        assert_process_from_marker_is_gone(&marker);
    }

    fn assert_process_from_marker_is_gone(path: &Path) {
        let started = Instant::now();
        let pid = loop {
            if let Ok(text) = fs::read_to_string(path) {
                break Pid::from_u32(text.trim().parse().expect("descendant pid"));
            }
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "descendant marker was not created"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let mut system = System::new();
        loop {
            system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
            if system.process(pid).is_none() {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "bounded runner left a descendant process running"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(windows)]
    fn pipe_inheriting_descendant_command(marker: &Path) -> Command {
        let powershell = PathBuf::from(env::var_os("SystemRoot").expect("SystemRoot"))
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let mut command = Command::new(&powershell);
        command.args(["-NoProfile", "-NonInteractive", "-Command"]);
        command.arg(format!(
            r#"
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = '{}'
$start.UseShellExecute = $false
$start.Arguments = '-NoProfile -NonInteractive -Command "Start-Sleep -Seconds 30"'
$child = [Diagnostics.Process]::Start($start)
[IO.File]::WriteAllText('{}', [string]$child.Id)
"#,
            powershell.display(),
            marker.display()
        ));
        command
    }

    #[cfg(unix)]
    fn pipe_inheriting_descendant_command(marker: &Path) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "sleep 30 & child=$!; printf '%s' \"$child\" > '{}'; exit 0",
            marker.display()
        ));
        command
    }
}
