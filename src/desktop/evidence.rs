//! Foreground-safe Power BI Desktop screenshot evidence capture and publication.

#[cfg(any(windows, test))]
use crate::canonical_display;
#[cfg(windows)]
use crate::desktop_target::{DesktopTargetKind, ResolvedDesktopTarget};
#[cfg(windows)]
use crate::{CliError, CliResult};
#[cfg(any(windows, test))]
use serde::Deserialize;
#[cfg(any(windows, test))]
use serde_json::{Value, json};
#[cfg(any(windows, test))]
use std::fs;
#[cfg(any(windows, test))]
use std::io;
#[cfg(windows)]
use std::path::Component;
#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};
#[cfg(any(windows, test))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use super::launch::{Timed, run_powershell};

// Budget covers foreground activation plus a canvas settle delay before the capture itself.
#[cfg(windows)]
pub(super) const SCREENSHOT_CAPTURE_TIMEOUT_MS: u64 = 25_000;
#[cfg(windows)]
const SCREENSHOT_SETTLE_MS: u64 = 4_000;
#[cfg(any(windows, test))]
static SCREENSHOT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(any(windows, test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScreenshotDimensions {
    pub(super) width: u32,
    pub(super) height: u32,
}

#[cfg(any(windows, test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotCaptureResult {
    width: u32,
    height: u32,
    activation_succeeded: bool,
    foreground_verified: bool,
    foreground_process_id: Option<u32>,
    captured: bool,
}

#[cfg(windows)]
#[derive(Debug)]
pub(super) struct ScreenshotCapture {
    pub(super) dimensions: ScreenshotDimensions,
    pub(super) activation_succeeded: bool,
    pub(super) foreground_verified: bool,
    pub(super) foreground_process_id: Option<u32>,
    pub(super) replaced_existing: bool,
}

#[cfg(windows)]
#[derive(Debug)]
pub(super) enum ScreenshotCaptureOutcome {
    Captured(ScreenshotCapture),
    ForegroundUnverified {
        activation_succeeded: bool,
        foreground_process_id: Option<u32>,
    },
}

#[cfg(windows)]
pub(super) fn validate_screenshot_output(
    out: &Path,
    target: &ResolvedDesktopTarget,
) -> CliResult<PathBuf> {
    let out = canonicalize_with_missing_tail(&absolute_lexical_path(out)?)?;
    if target.kind == DesktopTargetKind::Pbip {
        let project_dir =
            canonicalize_with_missing_tail(&absolute_lexical_path(&target.project_dir)?)?;
        if path_is_within_directory(&out, &project_dir) {
            return Err(CliError::invalid_args(format!(
                "desktop screenshot --out must be outside the project directory: {}",
                project_dir.display()
            ))
            .with_hint(
                "Write Desktop evidence beside the project or under a separate proof/artifacts directory so the PBIP handoff stays clean.",
            )
            .with_suggested_command(
                "powerbi-cli desktop screenshot <project-dir-or.pbip-or.pbix> --out <outside-project/evidence.png> --json",
            ));
        }
    }
    if out
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("png"))
    {
        return Err(
            CliError::invalid_args("desktop screenshot --out must end in .png")
                .with_hint("Use a PNG evidence path separate from the selected document.")
                .with_suggested_command(
                    "powerbi-cli desktop screenshot <project-dir-or.pbip-or.pbix> --out <evidence.png> --json",
                ),
        );
    }
    Ok(out)
}

#[cfg(windows)]
fn canonicalize_with_missing_tail(path: &Path) -> CliResult<PathBuf> {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return Ok(normalize_lexically(path));
        };
        missing.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            return Ok(normalize_lexically(path));
        };
        existing = parent;
    }
    let mut resolved = fs::canonicalize(existing).map_err(|err| {
        CliError::unexpected(format!(
            "resolve output path ancestor {}: {err}",
            existing.display()
        ))
    })?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(normalize_lexically(&resolved))
}

#[cfg(windows)]
fn absolute_lexical_path(path: &Path) -> CliResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| CliError::unexpected(format!("resolve current directory: {err}")))?
            .join(path)
    };
    Ok(normalize_lexically(&absolute))
}

#[cfg(windows)]
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !normalized.has_root() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(any(windows, test))]
fn path_is_within_directory(path: &Path, directory: &Path) -> bool {
    if cfg!(windows) {
        let path = path
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        let directory = directory
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        path == directory || path.starts_with(&format!("{directory}/"))
    } else {
        path == directory || path.starts_with(directory)
    }
}

#[cfg(any(windows, test))]
pub(super) fn screenshot_observation_is_eligible(title_matched: Option<bool>) -> bool {
    title_matched == Some(true)
}

#[cfg(any(windows, test))]
pub(super) fn screenshot_changes(
    captured: bool,
    replaced_existing: bool,
    out: Option<&Path>,
    dimensions: Option<&ScreenshotDimensions>,
    foreground_verified: Option<bool>,
) -> Vec<Value> {
    if !captured {
        return Vec::new();
    }
    let out = out.expect("captured screenshot has a validated output path");
    vec![json!({
        "kind": "desktop.screenshot",
        "action": if replaced_existing { "replace" } else { "create" },
        "path": canonical_display(out),
        "before": if replaced_existing {
            json!({"exists": true, "format": "png"})
        } else {
            Value::Null
        },
        "after": {
            "exists": true,
            "format": "png",
            "width": dimensions.map(|value| value.width),
            "height": dimensions.map(|value| value.height),
            "foregroundVerified": foreground_verified
        }
    })]
}

#[cfg(windows)]
pub(super) fn capture_primary_display(
    out: &Path,
    foreground_pid: Option<u32>,
    allow_unverified_capture: bool,
) -> io::Result<Timed<ScreenshotCaptureOutcome>> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    if out.exists() && !out.is_file() {
        return Err(io::Error::other(format!(
            "screenshot output is not a file: {}",
            out.display()
        )));
    }
    let temp = unique_screenshot_sibling(out, "capture")?;
    let script = render_screenshot_script(
        &super::launch::desktop_argument_path(&temp),
        foreground_pid,
        SCREENSHOT_SETTLE_MS,
        allow_unverified_capture,
    );
    let capture = run_powershell(
        &script,
        Duration::from_millis(SCREENSHOT_CAPTURE_TIMEOUT_MS),
    )?;
    match capture {
        Timed::Completed(output) => {
            if let Err(err) = super::launch::ensure_powershell_success(
                &output,
                "primary-display screenshot capture",
            ) {
                remove_file_if_present(&temp);
                return Err(err);
            }
            let result: ScreenshotCaptureResult =
                match super::launch::parse_powershell_json(&output.stdout) {
                    Ok(result) => result,
                    Err(err) => {
                        remove_file_if_present(&temp);
                        return Err(err);
                    }
                };
            if !capture_is_authorized(&result, allow_unverified_capture) {
                remove_file_if_present(&temp);
                return Ok(Timed::Completed(
                    ScreenshotCaptureOutcome::ForegroundUnverified {
                        activation_succeeded: result.activation_succeeded,
                        foreground_process_id: result.foreground_process_id,
                    },
                ));
            }
            if !result.captured {
                remove_file_if_present(&temp);
                return Err(io::Error::other(
                    "screenshot script authorized capture but did not write a PNG",
                ));
            }
            let metadata = match fs::metadata(&temp) {
                Ok(metadata) => metadata,
                Err(err) => {
                    remove_file_if_present(&temp);
                    return Err(err);
                }
            };
            if metadata.len() == 0 {
                remove_file_if_present(&temp);
                return Err(io::Error::other(format!(
                    "screenshot capture wrote an empty temporary file: {}",
                    temp.display()
                )));
            }
            let replaced_existing = match publish_screenshot(&temp, out) {
                Ok(replaced_existing) => replaced_existing,
                Err(err) => {
                    remove_file_if_present(&temp);
                    return Err(err);
                }
            };
            Ok(Timed::Completed(ScreenshotCaptureOutcome::Captured(
                ScreenshotCapture {
                    dimensions: ScreenshotDimensions {
                        width: result.width,
                        height: result.height,
                    },
                    activation_succeeded: result.activation_succeeded,
                    foreground_verified: result.foreground_verified,
                    foreground_process_id: result.foreground_process_id,
                    replaced_existing,
                },
            )))
        }
        Timed::TimedOut => {
            remove_file_if_present(&temp);
            Ok(Timed::TimedOut)
        }
    }
}

#[cfg(any(windows, test))]
const SCREENSHOT_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class PowerBiCliForegroundWindow {
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
}
'@
$foregroundPid = __FOREGROUND_PID__
$allowUnverifiedCapture = __ALLOW_UNVERIFIED_CAPTURE__
$activationSucceeded = $false
$activationError = $null
try {
    if ($foregroundPid -gt 0) {
        $candidate = Get-Process -Id $foregroundPid -ErrorAction SilentlyContinue |
            Where-Object { $_.ProcessName -like 'PBIDesktop*' }
        if ($candidate) {
            $shell = New-Object -ComObject WScript.Shell
            $activationSucceeded = [bool]$shell.AppActivate([int]$candidate.Id)
        }
    }
} catch {
    $activationError = $_.Exception.Message
}
Start-Sleep -Milliseconds __SETTLE_MS__
$foregroundWindow = [PowerBiCliForegroundWindow]::GetForegroundWindow()
[uint32]$activeProcessId = 0
if ($foregroundWindow -ne [IntPtr]::Zero) {
    [void][PowerBiCliForegroundWindow]::GetWindowThreadProcessId($foregroundWindow, [ref]$activeProcessId)
}
$foregroundProcessId = if ($activeProcessId -gt 0) { [int]$activeProcessId } else { $null }
$foregroundVerified = ($foregroundPid -gt 0 -and $activeProcessId -eq $foregroundPid)
if (-not $foregroundVerified -and $foregroundPid -gt 0 -and $activeProcessId -gt 0) {
    try {
        $parents = @{}
        foreach ($process in @(Get-CimInstance Win32_Process -ErrorAction Stop)) {
            $parents[[int]$process.ProcessId] = [int]$process.ParentProcessId
        }
        $visited = [System.Collections.Generic.HashSet[int]]::new()
        $cursor = [int]$activeProcessId
        while ($cursor -gt 0 -and $visited.Add($cursor)) {
            if ($cursor -eq $foregroundPid) {
                $foregroundVerified = $true
                break
            }
            if (-not $parents.ContainsKey($cursor)) {
                break
            }
            $cursor = [int]$parents[$cursor]
        }
    } catch {}
}
$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$captured = $false
if ($foregroundVerified -or $allowUnverifiedCapture) {
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
        $bitmap.Save(__OUT_PATH__, [System.Drawing.Imaging.ImageFormat]::Png)
        $captured = $true
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}
$result = [pscustomobject]@{
    width = [int]$bounds.Width
    height = [int]$bounds.Height
    activationSucceeded = [bool]$activationSucceeded
    activationError = $activationError
    foregroundVerified = [bool]$foregroundVerified
    foregroundProcessId = $foregroundProcessId
    captured = [bool]$captured
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Compress))
"#;

#[cfg(any(windows, test))]
pub(super) fn render_screenshot_script(
    out_path: &str,
    foreground_pid: Option<u32>,
    settle_ms: u64,
    allow_unverified_capture: bool,
) -> String {
    SCREENSHOT_SCRIPT
        .replace(
            "__OUT_PATH__",
            &super::launch::powershell_single_quoted(out_path),
        )
        .replace(
            "__FOREGROUND_PID__",
            &foreground_pid.unwrap_or_default().to_string(),
        )
        .replace("__SETTLE_MS__", &settle_ms.to_string())
        .replace(
            "__ALLOW_UNVERIFIED_CAPTURE__",
            if allow_unverified_capture {
                "$true"
            } else {
                "$false"
            },
        )
}

#[cfg(any(windows, test))]
fn capture_is_authorized(result: &ScreenshotCaptureResult, allow_unverified_capture: bool) -> bool {
    result.foreground_verified || allow_unverified_capture
}

#[cfg(any(windows, test))]
fn unique_screenshot_sibling(path: &Path, role: &str) -> io::Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("screenshot.png");
    for _ in 0..1_024 {
        let sequence = SCREENSHOT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{file_name}.powerbi-cli-{}-{sequence}.{role}.tmp",
            std::process::id()
        ));
        if !candidate.try_exists()? {
            return Ok(candidate);
        }
    }
    Err(io::Error::other(format!(
        "could not allocate a unique temporary screenshot path beside {}",
        path.display()
    )))
}

#[cfg(any(windows, test))]
fn publish_screenshot(temp: &Path, out: &Path) -> io::Result<bool> {
    if !out.try_exists()? {
        fs::rename(temp, out)?;
        return Ok(false);
    }
    if !out.is_file() {
        return Err(io::Error::other(format!(
            "screenshot output is not a file: {}",
            out.display()
        )));
    }
    let backup = unique_screenshot_sibling(out, "previous")?;
    fs::rename(out, &backup)?;
    if let Err(publish_err) = fs::rename(temp, out) {
        let rollback = fs::rename(&backup, out);
        return Err(match rollback {
            Ok(()) => io::Error::other(format!(
                "publish captured screenshot while preserving previous evidence: {publish_err}"
            )),
            Err(rollback_err) => io::Error::other(format!(
                "publish captured screenshot failed ({publish_err}); restoring previous evidence also failed ({rollback_err}); previous evidence remains at {}",
                backup.display()
            )),
        });
    }
    if let Err(remove_err) = fs::remove_file(&backup) {
        let remove_new = fs::remove_file(out);
        let rollback = fs::rename(&backup, out);
        return Err(io::Error::other(format!(
            "remove previous screenshot backup after publish: {remove_err}; remove-new={remove_new:?}; restore-previous={rollback:?}"
        )));
    }
    Ok(true)
}

#[cfg(windows)]
fn remove_file_if_present(path: &Path) {
    if path.is_file() {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ScreenshotCaptureResult, ScreenshotDimensions, capture_is_authorized,
        path_is_within_directory, publish_screenshot, render_screenshot_script, screenshot_changes,
        screenshot_observation_is_eligible,
    };
    use crate::desktop::observe::render_window_query_script;
    use serde_json::json;
    use std::fs;
    use std::path::Path;

    #[test]
    fn foreground_verification_is_required_without_explicit_override() {
        let unverified = ScreenshotCaptureResult {
            width: 1920,
            height: 1080,
            activation_succeeded: false,
            foreground_verified: false,
            foreground_process_id: Some(777),
            captured: false,
        };
        assert_eq!((unverified.width, unverified.height), (1920, 1080));
        assert!(!unverified.activation_succeeded);
        assert_eq!(unverified.foreground_process_id, Some(777));
        assert!(!unverified.captured);
        assert!(!capture_is_authorized(&unverified, false));
        assert!(capture_is_authorized(&unverified, true));

        let verified = ScreenshotCaptureResult {
            foreground_verified: true,
            foreground_process_id: Some(42),
            ..unverified
        };
        assert!(capture_is_authorized(&verified, false));
    }

    #[test]
    fn screenshot_requires_an_exact_project_title_observation() {
        assert!(screenshot_observation_is_eligible(Some(true)));
        assert!(!screenshot_observation_is_eligible(Some(false)));
        assert!(!screenshot_observation_is_eligible(None));
    }

    #[test]
    fn screenshot_publish_replaces_only_after_capture_succeeds() {
        let temp = tempfile::tempdir().expect("tempdir");
        let out = temp.path().join("evidence.png");
        let capture = temp.path().join("capture.tmp");
        fs::write(&out, b"previous evidence").expect("old evidence");
        fs::write(&capture, b"new evidence").expect("captured evidence");

        assert!(publish_screenshot(&capture, &out).expect("replace evidence"));
        assert_eq!(fs::read(&out).expect("published evidence"), b"new evidence");
        assert!(!capture.exists());
    }

    #[test]
    fn screenshot_publish_failure_restores_previous_evidence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let out = temp.path().join("evidence.png");
        let missing_capture = temp.path().join("missing.tmp");
        fs::write(&out, b"previous evidence").expect("old evidence");

        let error = publish_screenshot(&missing_capture, &out)
            .expect_err("missing capture must fail publication");
        assert!(error.to_string().contains("preserving previous evidence"));
        assert_eq!(
            fs::read(&out).expect("restored evidence"),
            b"previous evidence"
        );
        assert_eq!(
            fs::read_dir(temp.path()).expect("temp directory").count(),
            1,
            "rollback must not leave a backup artifact"
        );
    }

    #[test]
    fn screenshot_changes_are_empty_on_failure_and_exact_on_success() {
        let temp = tempfile::tempdir().expect("tempdir");
        let out = temp.path().join("evidence.png");
        fs::write(&out, b"png").expect("evidence");
        let dimensions = ScreenshotDimensions {
            width: 1280,
            height: 720,
        };

        assert!(screenshot_changes(false, false, None, None, None).is_empty());
        assert_eq!(
            screenshot_changes(true, true, Some(&out), Some(&dimensions), Some(true),),
            vec![json!({
                "kind": "desktop.screenshot",
                "action": "replace",
                "path": super::canonical_display(&out),
                "before": {"exists": true, "format": "png"},
                "after": {
                    "exists": true,
                    "format": "png",
                    "width": 1280,
                    "height": 720,
                    "foregroundVerified": true
                }
            })]
        );
    }

    #[test]
    fn generated_window_and_capture_scripts_enforce_intended_desktop_pid() {
        let windows = render_window_query_script();
        assert!(windows.contains("$_.ProcessName -like 'PBIDesktop*'"));
        assert!(!windows.contains("$_.Id -eq"));

        let screenshot = render_screenshot_script(r"C:\proof\evidence.png", Some(91), 25, false);
        assert!(screenshot.contains("GetWindowThreadProcessId"));
        assert!(screenshot.contains("$foregroundVerified ="));
        assert!(screenshot.contains("Get-CimInstance Win32_Process"));
        assert!(screenshot.contains("$cursor -eq $foregroundPid"));
        assert!(screenshot.contains("if ($foregroundVerified -or $allowUnverifiedCapture)"));
        assert!(screenshot.contains("activationSucceeded = [bool]$activationSucceeded"));
        assert!(screenshot.contains("foregroundProcessId = $foregroundProcessId"));
        assert!(!screenshot.contains("$candidates +="));
    }

    #[test]
    fn screenshot_output_rejects_project_descendants_only() {
        let project = Path::new("workspace/project");
        assert!(path_is_within_directory(
            Path::new("workspace/project/evidence.png"),
            project
        ));
        assert!(path_is_within_directory(project, project));
        assert!(!path_is_within_directory(
            Path::new("workspace/evidence.png"),
            project
        ));
        assert!(!path_is_within_directory(
            Path::new("workspace/project-copy/evidence.png"),
            project
        ));
    }
}
