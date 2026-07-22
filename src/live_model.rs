use crate::EXIT_ORACLE_FAILED;
use crate::desktop_target::ResolvedDesktopTarget;
use crate::{CliError, CliResult, EXIT_ORACLE_UNAVAILABLE};
#[cfg(windows)]
use serde::Deserialize;
#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[cfg(windows)]
use crate::desktop::{Timed, run_command_with_timeout};
#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveModelEndpoint {
    pub(crate) desktop_process_id: u32,
    pub(crate) model_process_id: u32,
    pub(crate) port: u16,
    pub(crate) desktop_version: String,
    #[cfg(windows)]
    pub(crate) desktop_executable: PathBuf,
    #[cfg(windows)]
    pub(crate) desktop_creation_ticks: i64,
    #[cfg(windows)]
    pub(crate) model_creation_ticks: i64,
    #[cfg(windows)]
    pub(crate) model_workspace: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OperationDeadline {
    started: Instant,
    budget: Duration,
}

impl OperationDeadline {
    pub(crate) fn new(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
        }
    }

    pub(crate) fn remaining(&self, stage: &str) -> CliResult<Duration> {
        let remaining = self.budget.saturating_sub(self.started.elapsed());
        if remaining.is_zero() {
            return Err(live_model_error(
                "desktop_operation_timeout",
                format!("Desktop live-model deadline was exhausted before {stage}"),
            ));
        }
        Ok(remaining)
    }
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryResult {
    ok: bool,
    stage: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    desktop_process_id: Option<u32>,
    #[serde(default)]
    model_process_id: Option<u32>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    desktop_version: Option<String>,
    #[serde(default)]
    desktop_executable: Option<PathBuf>,
    #[serde(default)]
    desktop_creation_ticks: Option<i64>,
    #[serde(default)]
    model_creation_ticks: Option<i64>,
    #[serde(default)]
    model_workspace: Option<PathBuf>,
}

pub(crate) fn desktop_oracle_enabled() -> bool {
    std::env::var("POWERBI_DESKTOP_ORACLE")
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

pub(crate) fn resolve_live_model_endpoint(
    target: &ResolvedDesktopTarget,
    timeout: Duration,
) -> CliResult<LiveModelEndpoint> {
    if !cfg!(windows) {
        return Err(CliError::new(
            "unsupported_feature",
            EXIT_ORACLE_UNAVAILABLE,
            format!(
                "Power BI Desktop live-model access is Windows-only; current platform is {}",
                std::env::consts::OS
            ),
        ));
    }
    if !desktop_oracle_enabled() {
        return Err(CliError::new(
            "oracle_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            "set POWERBI_DESKTOP_ORACLE=1 to opt in to the local Desktop model bridge",
        ));
    }
    target.require_live_model()?;
    if timeout.is_zero() {
        return Err(CliError::invalid_args(
            "live-model discovery timeout must be greater than zero",
        ));
    }

    #[cfg(windows)]
    {
        return resolve_windows(target, timeout);
    }

    #[allow(unreachable_code)]
    Err(CliError::unexpected(
        "Desktop live-model platform dispatch failed",
    ))
}

pub(crate) fn revalidate_live_model_endpoint(
    target: &ResolvedDesktopTarget,
    expected: &LiveModelEndpoint,
    timeout: Duration,
) -> CliResult<()> {
    let current = resolve_live_model_endpoint(target, timeout)?;
    if &current != expected {
        return Err(live_model_error(
            "desktop_model_identity_changed",
            "the exact Desktop/model PID, creation time, workspace, executable, version, or port changed during the operation",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn resolve_windows(
    target: &ResolvedDesktopTarget,
    timeout: Duration,
) -> CliResult<LiveModelEndpoint> {
    let runtime = tempfile::Builder::new()
        .prefix("powerbi-cli-live-model-")
        .tempdir()
        .map_err(|error| {
            CliError::unexpected(format!(
                "create temporary Desktop discovery directory: {error}"
            ))
        })?;
    let script_path = runtime.path().join("discover-live-model.ps1");
    fs::write(&script_path, LIVE_MODEL_DISCOVERY_SCRIPT).map_err(|error| {
        CliError::unexpected(format!("write temporary Desktop discovery script: {error}"))
    })?;

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
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match run_command_with_timeout(command, timeout) {
        Ok(Timed::Completed(output)) if output.status.success() => output,
        Ok(Timed::Completed(output)) => {
            return Err(live_model_error(
                "desktop-discovery-process",
                format!(
                    "Desktop discovery process failed: {}",
                    bounded_message(&String::from_utf8_lossy(&output.stderr))
                ),
            ));
        }
        Ok(Timed::TimedOut) => {
            return Err(live_model_error(
                "desktop-discovery-timeout",
                format!(
                    "Desktop live-model discovery exceeded {} ms",
                    timeout.as_millis()
                ),
            ));
        }
        Err(error) => {
            return Err(live_model_error(
                "desktop-discovery-process",
                format!("Desktop discovery process could not run: {error}"),
            ));
        }
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let result: DiscoveryResult = serde_json::from_str(text.trim().trim_start_matches('\u{feff}'))
        .map_err(|error| {
            live_model_error(
                "desktop-discovery-protocol",
                format!("Desktop discovery returned invalid JSON: {error}"),
            )
        })?;
    if !result.ok {
        let detail = result
            .message
            .unwrap_or_else(|| "Desktop live-model discovery failed without a message".to_string());
        return Err(live_model_error(
            "desktop_model_unavailable",
            format!(
                "{} at stage {}{}",
                detail,
                result.stage,
                result
                    .error_type
                    .map(|value| format!(" ({value})"))
                    .unwrap_or_default()
            ),
        ));
    }

    let endpoint = LiveModelEndpoint {
        desktop_process_id: required(result.desktop_process_id, "desktopProcessId")?,
        model_process_id: required(result.model_process_id, "modelProcessId")?,
        port: required(result.port, "port")?,
        desktop_version: required(result.desktop_version, "desktopVersion")?,
        desktop_executable: required(result.desktop_executable, "desktopExecutable")?,
        desktop_creation_ticks: required(result.desktop_creation_ticks, "desktopCreationTicks")?,
        model_creation_ticks: required(result.model_creation_ticks, "modelCreationTicks")?,
        model_workspace: required(result.model_workspace, "modelWorkspace")?,
    };
    let executable = fs::canonicalize(&endpoint.desktop_executable).map_err(|error| {
        live_model_error(
            "desktop-discovery-protocol",
            format!(
                "resolve discovered Desktop executable {}: {error}",
                endpoint.desktop_executable.display()
            ),
        )
    })?;
    let file_name = executable
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !file_name.to_ascii_lowercase().starts_with("pbidesktop")
        || !file_name.to_ascii_lowercase().ends_with(".exe")
    {
        return Err(live_model_error(
            "desktop-discovery-protocol",
            "discovered executable is not a Power BI Desktop executable",
        ));
    }
    let workspace = fs::canonicalize(&endpoint.model_workspace).map_err(|error| {
        live_model_error(
            "desktop-discovery-protocol",
            format!(
                "resolve discovered model workspace {}: {error}",
                endpoint.model_workspace.display()
            ),
        )
    })?;
    if endpoint.desktop_creation_ticks <= 0 || endpoint.model_creation_ticks <= 0 {
        return Err(live_model_error(
            "desktop-discovery-protocol",
            "discovered process creation identity is invalid",
        ));
    }
    Ok(LiveModelEndpoint {
        desktop_executable: executable,
        model_workspace: workspace,
        ..endpoint
    })
}

#[cfg(windows)]
fn required<T>(value: Option<T>, field: &str) -> CliResult<T> {
    value.ok_or_else(|| {
        live_model_error(
            "desktop-discovery-protocol",
            format!("successful Desktop discovery omitted {field}"),
        )
    })
}

fn live_model_error(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::new(code, EXIT_ORACLE_FAILED, message)
        .with_hint(
            "Keep exactly one Power BI Desktop process open for the canonical PBIP/PBIX path and retry.",
        )
        .with_suggested_command("powerbi-cli desktop close --json")
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
pub(crate) fn windows_argument_path(path: &Path) -> String {
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
const LIVE_MODEL_DISCOVERY_SCRIPT: &str = r#"
param([Parameter(Mandatory = $true)][string]$DocumentPath)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$stage = 'desktop-discovery'
$desktop = $null
$model = $null
$port = $null
$desktopVersion = $null
$workspace = $null
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
        foreach ($token in $tokens) {
            if (-not $token.EndsWith($documentExtension, [StringComparison]::OrdinalIgnoreCase)) {
                continue
            }
            try {
                $candidateDocument = [IO.Path]::GetFullPath($token)
                if ([string]::Equals($candidateDocument, $documentFull, [StringComparison]::OrdinalIgnoreCase)) {
                    [void]$desktopMatches.Add($candidate)
                    break
                }
            } catch {}
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
            if ($descendantIds.Contains([int]$candidate.ParentProcessId) -and -not $descendantIds.Contains([int]$candidate.ProcessId)) {
                [void]$descendantIds.Add([int]$candidate.ProcessId)
                $changed = $true
            }
        }
    }
    $models = [System.Collections.Generic.List[object]]::new()
    foreach ($candidate in @($processes | Where-Object { $_.Name -eq 'msmdsrv.exe' })) {
        if (-not $descendantIds.Contains([int]$candidate.ProcessId)) { continue }
        $workspaceMatch = [regex]::Match(
            [string]$candidate.CommandLine,
            '(?:^|\s)-s\s+(?:"(?<quoted>[^\"]+)"|(?<bare>\S+))',
            [Text.RegularExpressions.RegexOptions]::IgnoreCase
        )
        if (-not $workspaceMatch.Success) { continue }
        $workspace = if ($workspaceMatch.Groups['quoted'].Success) {
            $workspaceMatch.Groups['quoted'].Value
        } else {
            $workspaceMatch.Groups['bare'].Value
        }
        $portFile = Join-Path $workspace 'msmdsrv.port.txt'
        if (-not (Test-Path -LiteralPath $portFile -PathType Leaf)) { continue }
        $portText = [IO.File]::ReadAllText($portFile, [Text.Encoding]::Unicode)
        $portMatch = [regex]::Match($portText, '\d+')
        if (-not $portMatch.Success) { continue }
        $candidatePort = [int]$portMatch.Value
        if ($candidatePort -lt 1 -or $candidatePort -gt 65535) { continue }
        [void]$models.Add([pscustomobject]@{
            process = $candidate
            port = $candidatePort
            workspace = $workspace
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
    $workspace = [IO.Path]::GetFullPath([string]$models[0].workspace)
    if ([string]::IsNullOrWhiteSpace([string]$desktop.ExecutablePath)) {
        throw 'Power BI Desktop executable path is unavailable from the process inventory.'
    }
    $desktopVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo([string]$desktop.ExecutablePath).FileVersion
    $result = [pscustomobject]@{
        ok = $true
        stage = 'completed'
        desktopProcessId = [int]$desktop.ProcessId
        modelProcessId = [int]$model.ProcessId
        port = $port
        desktopVersion = $desktopVersion
        desktopExecutable = [string]$desktop.ExecutablePath
        desktopCreationTicks = [int64]$desktop.CreationDate.ToUniversalTime().Ticks
        modelCreationTicks = [int64]$model.CreationDate.ToUniversalTime().Ticks
        modelWorkspace = $workspace
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
        desktopCreationTicks = $null
        modelCreationTicks = $null
        modelWorkspace = $workspace
    }
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Compress -Depth 4))
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn oracle_opt_in_is_narrow() {
        // The environment is process-global, so only assert the parser through
        // a local equivalent instead of mutating it in parallel tests.
        let accepted = |value: &str| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES");
        assert!(accepted("1"));
        assert!(accepted("true"));
        assert!(!accepted("on"));
        assert!(!accepted("2"));
    }

    #[test]
    fn operation_deadline_refuses_an_exhausted_budget() {
        let deadline = super::OperationDeadline::new(std::time::Duration::ZERO);
        let error = deadline.remaining("test phase").expect_err("zero budget");
        assert_eq!(error.code, "desktop_operation_timeout");
        assert!(error.message.contains("test phase"));
    }

    #[cfg(windows)]
    #[test]
    fn discovery_script_requires_exact_document_and_descendant_engine() {
        use super::LIVE_MODEL_DISCOVERY_SCRIPT;

        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains(".pbip"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains(".pbix"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("GetFullPath($token)"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("OrdinalIgnoreCase"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("$desktopMatches.Count -ne 1"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("$descendantIds.Contains"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("$models.Count -ne 1"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("msmdsrv.port.txt"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("desktopCreationTicks"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("modelCreationTicks"));
        assert!(LIVE_MODEL_DISCOVERY_SCRIPT.contains("modelWorkspace"));
    }
}
