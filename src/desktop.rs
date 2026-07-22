use crate::bridge::desktop_bridge_command;
#[cfg(windows)]
use crate::contract::CONTRACT_VERSION;
use crate::desktop_session::close_desktop_session_command;
#[cfg(windows)]
use crate::desktop_session::{
    DesktopSessionDraft, DesktopSessionLock, ManagedDesktopSession, close_desktop_session,
    open_desktop_session,
};
#[cfg(windows)]
use crate::desktop_target::{DesktopTargetKind, ResolvedDesktopTarget, resolve_desktop_target};
#[cfg(windows)]
use crate::lint::lint_project;
use crate::{CliError, CliResult, canonical_display};
#[cfg(windows)]
use crate::{
    EXIT_ORACLE_FAILED, EXIT_ORACLE_UNAVAILABLE, EXIT_PROOF_INCOMPLETE, EXIT_SUCCESS,
    EXIT_VALIDATION_FAILED, ValidationReport, command_arg, validate_project,
};
#[cfg(any(windows, test))]
use serde::Deserialize;
use serde_json::Value;
#[cfg(any(windows, test))]
use serde_json::json;
#[cfg(windows)]
use std::collections::BTreeSet;
#[cfg(any(windows, test))]
use std::fs;
#[cfg(any(windows, test))]
use std::io;
#[cfg(windows)]
use std::path::Component;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::{Command, Stdio};
#[cfg(any(windows, test))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(windows, test))]
use std::time::Duration;
#[cfg(windows)]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
#[cfg(windows)]
const WINDOW_POLL_INTERVAL_MS: u64 = 250;
#[cfg(windows)]
const COMMAND_POLL_INTERVAL_MS: u64 = 25;
#[cfg(windows)]
pub(crate) const CLEANUP_TIMEOUT_MS: u64 = 15_000;
// Budget covers foreground activation plus a canvas settle delay before the capture itself.
#[cfg(windows)]
const SCREENSHOT_CAPTURE_TIMEOUT_MS: u64 = 25_000;
#[cfg(windows)]
const SCREENSHOT_SETTLE_MS: u64 = 4_000;
#[cfg(windows)]
const DESKTOP_COMMAND_PROOF_LEVEL: &str = "unit-smoke";
#[cfg(any(windows, test))]
static SCREENSHOT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub(crate) struct PowerBiDesktopDetection {
    pub(crate) found: bool,
    pub(crate) path: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) checked: Vec<String>,
    pub(crate) source: String,
    #[cfg(windows)]
    path_buf: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopOperation {
    Open,
    OpenCheck,
    Screenshot,
}

impl DesktopOperation {
    fn command_path(self) -> &'static str {
        match self {
            Self::Open => "desktop open",
            Self::OpenCheck => "desktop open-check",
            Self::Screenshot => "desktop screenshot",
        }
    }

    #[cfg(windows)]
    fn output_schema(self) -> &'static str {
        match self {
            Self::Open => "powerbi-cli.desktop.open.v1",
            Self::OpenCheck => "powerbi-cli.desktop.openCheck.v1",
            Self::Screenshot => "powerbi-cli.desktop.screenshot.v1",
        }
    }

    fn suggested_command(self) -> &'static str {
        match self {
            Self::Open => {
                "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> --timeout-ms 120000 --json"
            }
            Self::OpenCheck => {
                "powerbi-cli desktop open-check <project-dir-or.pbip-or.pbix> --timeout-ms 120000 --json"
            }
            Self::Screenshot => {
                "powerbi-cli desktop screenshot <project-dir-or.pbip-or.pbix> --out <evidence.png> --timeout-ms 120000 --json"
            }
        }
    }
}

#[derive(Debug)]
struct DesktopOptions {
    project: Option<PathBuf>,
    desktop_path: Option<PathBuf>,
    out: Option<PathBuf>,
    timeout_ms: u64,
    allow_unverified_capture: bool,
}

impl Default for DesktopOptions {
    fn default() -> Self {
        Self {
            project: None,
            desktop_path: None,
            out: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            allow_unverified_capture: false,
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessIdentity {
    pub(crate) process_id: u32,
    pub(crate) creation_time_utc: String,
    #[cfg(windows)]
    pub(crate) executable_path: Option<String>,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct DesktopLaunchPlan {
    method: &'static str,
    detection_path_used_for_launch: bool,
    requested_desktop_path: Option<String>,
    file_association_reason: Option<&'static str>,
}

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessWindow {
    id: u32,
    process_name: String,
    #[serde(default)]
    main_window_title: String,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct WindowObservation {
    attempted: bool,
    window_observed: Option<bool>,
    title_matched: Option<bool>,
    observed_window_title: Option<String>,
    observed_process_id: Option<u32>,
    observed_process_name: Option<String>,
    observed_at_ms: Option<u64>,
    launch_elapsed_ms: Option<u64>,
    elapsed_ms: u64,
    timed_out: bool,
    completed_reason: &'static str,
    polls: u64,
    candidate_process_ids: Vec<u32>,
    exact_title_candidate_count: usize,
    selection_reason: Option<&'static str>,
}

#[cfg(windows)]
impl WindowObservation {
    fn not_attempted() -> Self {
        Self {
            attempted: false,
            window_observed: None,
            title_matched: None,
            observed_window_title: None,
            observed_process_id: None,
            observed_process_name: None,
            observed_at_ms: None,
            launch_elapsed_ms: None,
            elapsed_ms: 0,
            timed_out: false,
            completed_reason: "not-attempted",
            polls: 0,
            candidate_process_ids: Vec::new(),
            exact_title_candidate_count: 0,
            selection_reason: None,
        }
    }

    fn timed_out(watchdog: &Watchdog, launch_elapsed_ms: u64) -> Self {
        Self {
            attempted: true,
            window_observed: Some(false),
            title_matched: None,
            observed_window_title: None,
            observed_process_id: None,
            observed_process_name: None,
            observed_at_ms: None,
            launch_elapsed_ms: Some(launch_elapsed_ms),
            elapsed_ms: watchdog.elapsed_ms(),
            timed_out: true,
            completed_reason: "timeout",
            polls: 0,
            candidate_process_ids: Vec::new(),
            exact_title_candidate_count: 0,
            selection_reason: None,
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct Watchdog {
    started: Instant,
    budget: Duration,
}

#[cfg(windows)]
impl Watchdog {
    fn new(timeout_ms: u64) -> Self {
        Self {
            started: Instant::now(),
            budget: Duration::from_millis(timeout_ms),
        }
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn elapsed_ms(&self) -> u64 {
        duration_ms(self.elapsed())
    }

    fn remaining(&self) -> Duration {
        remaining_budget(self.budget, self.elapsed())
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub(crate) enum Timed<T> {
    Completed(T),
    TimedOut,
}

#[cfg(any(windows, test))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotDimensions {
    width: u32,
    height: u32,
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
struct ScreenshotCapture {
    dimensions: ScreenshotDimensions,
    activation_succeeded: bool,
    foreground_verified: bool,
    foreground_process_id: Option<u32>,
    replaced_existing: bool,
}

#[cfg(windows)]
#[derive(Debug)]
enum ScreenshotCaptureOutcome {
    Captured(ScreenshotCapture),
    ForegroundUnverified {
        activation_succeeded: bool,
        foreground_process_id: Option<u32>,
    },
}

pub(crate) fn desktop_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args(
                "desktop requires a subcommand: open, close, open-check, screenshot, or bridge",
            )
                .with_hint(
                    "Run powerbi-cli --json capabilities --for desktop for supported Desktop oracle commands.",
                )
                .with_suggested_command(
                    "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> --json",
                )
                .with_suggested_command("powerbi-cli desktop close --json")
                .with_suggested_command(
                    "powerbi-cli desktop open-check <project-dir-or.pbip-or.pbix> --json",
                )
                .with_suggested_command(
                    "powerbi-cli desktop screenshot <project-dir-or.pbip-or.pbix> --out <evidence.png> --json",
                ),
        );
    };

    match action.as_str() {
        "open" => run_desktop(DesktopOperation::Open, rest),
        "close" => close_desktop_session_command(rest),
        "open-check" | "openCheck" => run_desktop(DesktopOperation::OpenCheck, rest),
        "screenshot" => run_desktop(DesktopOperation::Screenshot, rest),
        "bridge" => desktop_bridge_command(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown desktop command: {action}"
        ))
        .with_hint("Run powerbi-cli --json capabilities --for desktop for supported Desktop oracle commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for desktop")),
    }
}

pub(crate) fn detect_power_bi_desktop(override_path: Option<&Path>) -> PowerBiDesktopDetection {
    let mut candidates = Vec::new();
    let mut source = "not-found".to_string();
    if let Some(path) = override_path {
        candidates.push(path.to_path_buf());
    } else {
        candidates.extend(power_bi_desktop_candidates());
    }
    let found = candidates.iter().find(|path| path.exists()).cloned();
    if found.is_some() {
        source = if override_path.is_some() {
            "override".to_string()
        } else {
            "known-path".to_string()
        };
    }
    PowerBiDesktopDetection {
        found: found.is_some(),
        path: found.as_ref().map(|path| canonical_display(path)),
        // Version probing is deliberately opt-in and bounded inside `run_desktop`.
        // Detection is also used by `doctor`, which must remain a side-effect-free
        // filesystem check.
        version: None,
        checked: candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        source,
        #[cfg(windows)]
        path_buf: found,
    }
}

#[cfg(windows)]
fn run_desktop(operation: DesktopOperation, args: &[String]) -> CliResult<Value> {
    let options = parse_desktop_args(operation, args)?;
    ensure_desktop_platform(std::env::consts::OS)?;
    let document = options.project.as_ref().ok_or_else(|| {
        CliError::invalid_args(format!(
            "{} requires <project-dir-or.pbip-or.pbix>",
            operation.command_path()
        ))
        .with_hint("Pass a PBIP project directory, .pbip file, or .pbix Desktop file.")
        .with_suggested_command(operation.suggested_command())
    })?;
    let target = resolve_desktop_target(document)?;
    let screenshot_out = match operation {
        DesktopOperation::Open | DesktopOperation::OpenCheck => None,
        DesktopOperation::Screenshot => {
            let out = options.out.as_ref().ok_or_else(|| {
                CliError::invalid_args("desktop screenshot requires --out <file.png>")
                    .with_hint("Choose a PNG evidence path separate from the selected document.")
                    .with_suggested_command(operation.suggested_command())
            })?;
            Some(validate_screenshot_output(out, &target)?)
        }
    };

    let validation = match target.project() {
        Some(project) => validate_project(project)?,
        None => ValidationReport::default(),
    };
    let validation_ok = validation.errors.is_empty();
    let strict_preflight_enabled = target.kind == DesktopTargetKind::Pbip;
    let lint = if validation_ok && strict_preflight_enabled {
        Some(lint_project(
            target.project().expect("PBIP target has a project"),
            &validation,
        )?)
    } else {
        None
    };
    let lint_error_count = lint
        .as_ref()
        .and_then(|value| value["counts"]["errors"].as_u64())
        .unwrap_or_default();
    let strict_preflight_ok = validation_ok && lint_error_count == 0;
    let mut detection = detect_power_bi_desktop(options.desktop_path.as_deref());
    let oracle_enabled = oracle_enabled();
    let launch_plan = desktop_launch_plan(&options, &detection);
    let project_name = target.name.clone();

    let mut diagnostics = Vec::new();
    let mut launched = false;
    let mut launch_attempted = false;
    let mut desktop_process_id = None;
    let mut baseline_process_ids = Vec::new();
    let mut observation = WindowObservation::not_attempted();
    let mut screenshot_captured = false;
    let mut screenshot_dimensions: Option<ScreenshotDimensions> = None;
    let mut screenshot_activation_succeeded: Option<bool> = None;
    let mut screenshot_foreground_verified: Option<bool> = None;
    let mut screenshot_foreground_process_id: Option<u32> = None;
    let mut screenshot_replaced_existing = false;
    let mut screenshot_error: Option<String> = None;
    let mut launch_timestamp_unix_ms: Option<u64> = None;
    let mut exit_code = EXIT_SUCCESS;
    let mut observed_stage = "not-attempted";
    let mut proof_status = "not-attempted".to_string();
    let mut proof_message = "Desktop oracle observation was not attempted.".to_string();
    let mut prior_session_cleanup = Value::Null;
    let mut managed_session_lock: Option<DesktopSessionLock> = None;
    let mut managed_session: Option<ManagedDesktopSession> = None;
    let mut session_persisted = false;
    let mut association_identity: Option<ProcessIdentity> = None;
    let mut observed_identity: Option<ProcessIdentity> = None;

    if !validation_ok {
        exit_code = EXIT_VALIDATION_FAILED;
        proof_status = "validation-failed".to_string();
        proof_message = "Local PBIP validation failed before Desktop launch.".to_string();
        diagnostics.push(json!({
            "code": "project_validation_failed",
            "severity": "error",
            "message": "Local validation failed before Desktop oracle launch."
        }));
    } else if !strict_preflight_ok {
        exit_code = EXIT_VALIDATION_FAILED;
        proof_status = "strict-validation-failed".to_string();
        proof_message =
            "Strict validation/lint preflight failed before Desktop launch.".to_string();
        diagnostics.push(json!({
            "code": "strict_preflight_failed",
            "severity": "error",
            "message": "Strict validation/lint preflight failed before Desktop oracle launch.",
            "findings": lint_error_findings(lint.as_ref())
        }));
    } else if !oracle_enabled {
        exit_code = EXIT_ORACLE_UNAVAILABLE;
        proof_status = "oracle-disabled".to_string();
        proof_message =
            "Desktop oracle launch is disabled; set POWERBI_DESKTOP_ORACLE=1 to opt in."
                .to_string();
        diagnostics.push(json!({
            "code": "oracle_disabled",
            "severity": "error",
            "message": format!(
                "Set POWERBI_DESKTOP_ORACLE=1 on a Windows machine with Power BI Desktop installed to run {}.",
                operation.command_path()
            )
        }));
    } else if !detection.found {
        exit_code = EXIT_ORACLE_UNAVAILABLE;
        proof_status = "desktop-not-found".to_string();
        proof_message = "Power BI Desktop was not found.".to_string();
        diagnostics.push(json!({
            "code": "desktop_not_found",
            "severity": "error",
            "message": "Power BI Desktop was not found. Install Desktop or pass --desktop-path <PBIDesktop.exe>."
        }));
    } else if let Some(desktop_path) = detection.path_buf.clone() {
        if operation == DesktopOperation::Open {
            let lock = DesktopSessionLock::acquire()?;
            prior_session_cleanup = close_desktop_session(&lock)?;
            if prior_session_cleanup["ok"].as_bool() != Some(true) {
                return Err(CliError::unexpected(
                    "could not close the previous CLI-owned Power BI Desktop session",
                )
                .with_hint(
                    "Inspect the desktop close response, close only the recorded owned session, and retry.",
                )
                .with_suggested_command("powerbi-cli desktop close --json"));
            }
            managed_session_lock = Some(lock);
        }
        let watchdog = Watchdog::new(options.timeout_ms);
        let version_probe_completed = match desktop_file_version(
            Some(&desktop_path),
            watchdog.remaining(),
        ) {
            Ok(Timed::Completed(version)) => {
                detection.version = version;
                true
            }
            Ok(Timed::TimedOut) => {
                exit_code = EXIT_ORACLE_FAILED;
                proof_status = "observation-setup-timeout".to_string();
                proof_message = "The launch/observation watchdog expired while probing the Power BI Desktop version before launch.".to_string();
                diagnostics.push(json!({
                    "code": "oracle_failed",
                    "severity": "error",
                    "message": "Timed out while probing the Power BI Desktop version inside the setup budget."
                }));
                false
            }
            Err(err) => {
                diagnostics.push(json!({
                    "code": "desktop_version_unavailable",
                    "severity": "warning",
                    "message": format!("Could not read the Power BI Desktop version inside the setup budget: {err}")
                }));
                true
            }
        };
        if version_probe_completed {
            match snapshot_desktop_process_ids(watchdog.remaining()) {
                Ok(Timed::Completed(process_ids)) => {
                    baseline_process_ids = process_ids;
                    launch_attempted = true;
                    launch_timestamp_unix_ms = Some(unix_time_ms().map_err(|err| {
                        CliError::unexpected(format!(
                            "record Desktop launch timestamp for ownership-safe cleanup: {err}"
                        ))
                    })?);
                    match launch_desktop(
                        &desktop_path,
                        &target.artifact_path,
                        &launch_plan,
                        watchdog.remaining(),
                    ) {
                        Ok(Timed::Completed(launched_pid)) => {
                            desktop_process_id = Some(launched_pid);
                            launched = true;
                            observed_stage = "desktop-launch";
                            match read_process_identity(launched_pid) {
                                Ok(identity) => association_identity = identity,
                                Err(error) => diagnostics.push(json!({
                                    "code": "desktop_association_identity_failed",
                                    "severity": "error",
                                    "message": error.message
                                })),
                            }
                            let launch_elapsed_ms = watchdog.elapsed_ms();
                            match observe_window(
                                launched_pid,
                                &baseline_process_ids,
                                &project_name,
                                &watchdog,
                                launch_elapsed_ms,
                            ) {
                                Ok(observed) => {
                                    observation = observed;
                                    if let Some(process_id) = observation.observed_process_id {
                                        if process_id == launched_pid {
                                            observed_identity = association_identity.clone();
                                        } else {
                                            match read_process_identity(process_id) {
                                                Ok(identity) => observed_identity = identity,
                                                Err(error) => diagnostics.push(json!({
                                                    "code": "desktop_observed_identity_failed",
                                                    "severity": "error",
                                                    "message": error.message
                                                })),
                                            }
                                        }
                                    }
                                    if observation.title_matched == Some(true) {
                                        observed_stage = "desktop-window";
                                        proof_status = "window-observed".to_string();
                                        proof_message = "Power BI Desktop exposed a non-empty main window title whose normalized project stem exactly matched the PBIP project name. Canvas render and refresh remain unproven.".to_string();
                                    } else if observation.exact_title_candidate_count > 1 {
                                        proof_status = "window-title-ambiguous".to_string();
                                        proof_message = "Power BI Desktop exposed several windows with the same report title, but none could be tied safely to the new launch. The oracle refused to guess which report instance was intended.".to_string();
                                        diagnostics.push(json!({
                                            "code": "desktop_title_ambiguous",
                                            "severity": "warning",
                                            "message": "Several Power BI Desktop windows matched the project title; close duplicate report instances or leave the newly launched instance open and retry.",
                                            "matchingWindowCount": observation.exact_title_candidate_count,
                                            "candidateProcessIds": observation.candidate_process_ids
                                        }));
                                    } else if observation.window_observed == Some(true) {
                                        proof_status = "window-title-timeout".to_string();
                                        proof_message = "Power BI Desktop exposed a titled window, but its normalized project stem did not exactly match the PBIP project name before the watchdog expired. Process launch remains observed; canvas render and refresh remain unproven.".to_string();
                                        diagnostics.push(json!({
                                        "code": "desktop_title_not_matched",
                                        "severity": "warning",
                                        "message": "A Desktop window title was observed, but its normalized project stem did not exactly match the PBIP project name within the launch/observation budget.",
                                        "observedWindowTitle": observation.observed_window_title
                                    }));
                                    } else {
                                        proof_status = "window-observation-timeout".to_string();
                                        proof_message = "Power BI Desktop launch succeeded, but no relevant non-empty main window title appeared before the watchdog expired. Process launch remains observed; this timeout is not an oracle failure.".to_string();
                                        diagnostics.push(json!({
                                        "code": "desktop_observation_timeout",
                                        "severity": "warning",
                                        "message": "Desktop launch succeeded, but window observation exhausted the timeout budget."
                                    }));
                                    }

                                    if operation == DesktopOperation::Screenshot {
                                        if screenshot_observation_is_eligible(
                                            observation.title_matched,
                                        ) {
                                            let out = screenshot_out
                                                .as_ref()
                                                .expect("screenshot output was validated");
                                            match capture_primary_display(
                                            out,
                                            observation.observed_process_id,
                                            options.allow_unverified_capture,
                                        ) {
                                            Ok(Timed::Completed(
                                                ScreenshotCaptureOutcome::Captured(capture),
                                            )) => {
                                                screenshot_captured = true;
                                                screenshot_dimensions = Some(capture.dimensions);
                                                screenshot_activation_succeeded =
                                                    Some(capture.activation_succeeded);
                                                screenshot_foreground_verified =
                                                    Some(capture.foreground_verified);
                                                screenshot_foreground_process_id =
                                                    capture.foreground_process_id;
                                                screenshot_replaced_existing =
                                                    capture.replaced_existing;
                                                proof_status = if capture.foreground_verified {
                                                    "screenshot-captured".to_string()
                                                } else {
                                                    "screenshot-captured-unverified-foreground"
                                                        .to_string()
                                                };
                                                proof_message = if capture.foreground_verified {
                                                    "Captured the primary display only after verifying that the foreground window belonged to the exactly matched Power BI Desktop process. The PNG is evidence for manual/agent review, not automated compatibility proof.".to_string()
                                                } else {
                                                    diagnostics.push(json!({
                                                        "code": "unverified_capture_allowed",
                                                        "severity": "warning",
                                                        "message": "The primary display was captured without verified foreground ownership because --allow-unverified-capture was explicitly passed; the PNG may contain unrelated sensitive screen content."
                                                    }));
                                                    "Captured the primary display without verified foreground ownership under the explicit --allow-unverified-capture override. Treat the PNG as sensitive and untrusted evidence.".to_string()
                                                };
                                            }
                                            Ok(Timed::Completed(
                                                ScreenshotCaptureOutcome::ForegroundUnverified {
                                                    activation_succeeded,
                                                    foreground_process_id,
                                                },
                                            )) => {
                                                exit_code = EXIT_ORACLE_FAILED;
                                                proof_status =
                                                    "screenshot-foreground-unverified".to_string();
                                                proof_message = "Desktop window observation succeeded, but screenshot capture was refused because the intended Power BI Desktop process did not own the foreground window.".to_string();
                                                screenshot_activation_succeeded =
                                                    Some(activation_succeeded);
                                                screenshot_foreground_verified = Some(false);
                                                screenshot_foreground_process_id =
                                                    foreground_process_id;
                                                screenshot_error = Some(
                                                    "Foreground verification failed; no PNG was published. Pass --allow-unverified-capture only if the risk of capturing unrelated sensitive screen content is explicitly accepted."
                                                        .to_string(),
                                                );
                                                diagnostics.push(json!({
                                                    "code": "oracle_failed",
                                                    "severity": "error",
                                                    "message": "Screenshot capture was refused because foreground ownership did not match the intended Power BI Desktop process; no PNG was published."
                                                }));
                                            }
                                            Ok(Timed::TimedOut) => {
                                                exit_code = EXIT_ORACLE_FAILED;
                                                proof_status =
                                                    "screenshot-capture-timeout".to_string();
                                                proof_message = "Desktop window observation succeeded, but primary-display screenshot capture timed out.".to_string();
                                                screenshot_error = Some(format!(
                                                    "Primary-display capture exceeded its {SCREENSHOT_CAPTURE_TIMEOUT_MS} ms safety timeout."
                                                ));
                                                diagnostics.push(json!({
                                                    "code": "oracle_failed",
                                                    "severity": "error",
                                                    "message": "Primary-display screenshot capture timed out."
                                                }));
                                            }
                                            Err(err) => {
                                                exit_code = EXIT_ORACLE_FAILED;
                                                proof_status =
                                                    "screenshot-capture-failed".to_string();
                                                proof_message = "Desktop window observation succeeded, but primary-display screenshot capture failed.".to_string();
                                                screenshot_error = Some(err.to_string());
                                                diagnostics.push(json!({
                                                    "code": "oracle_failed",
                                                    "severity": "error",
                                                    "message": format!("Primary-display screenshot capture failed: {err}")
                                                }));
                                            }
                                        }
                                        } else {
                                            exit_code = EXIT_PROOF_INCOMPLETE;
                                            if observation.exact_title_candidate_count > 1 {
                                                proof_status =
                                                    "screenshot-not-captured-title-ambiguous"
                                                        .to_string();
                                                proof_message = "Desktop launch succeeded, but several pre-existing Power BI Desktop windows shared the project title and none could be tied safely to the launch, so no screenshot was captured.".to_string();
                                                diagnostics.push(json!({
                                                    "code": "proof_incomplete",
                                                    "severity": "warning",
                                                    "message": "No screenshot was captured because the exact report title matched several ambiguous pre-existing Desktop windows."
                                                }));
                                            } else if observation.window_observed == Some(true) {
                                                proof_status =
                                                    "screenshot-not-captured-title-mismatch"
                                                        .to_string();
                                                proof_message = "Desktop launch succeeded, but no Power BI Desktop window title exactly matched the project identity, so no screenshot was captured. This is incomplete evidence, not an oracle failure.".to_string();
                                                diagnostics.push(json!({
                                                    "code": "proof_incomplete",
                                                    "severity": "warning",
                                                    "message": "No screenshot was captured because the observed Desktop window title did not exactly match the project identity."
                                                }));
                                            } else {
                                                proof_status =
                                                    "screenshot-not-captured-timeout".to_string();
                                                proof_message = "Desktop launch succeeded, but no titled Desktop window appeared within the launch/observation budget, so no screenshot was captured. This is incomplete evidence, not an oracle failure.".to_string();
                                                diagnostics.push(json!({
                                                    "code": "proof_incomplete",
                                                    "severity": "warning",
                                                    "message": "No screenshot was captured because Desktop window observation timed out after launch."
                                                }));
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    exit_code = EXIT_ORACLE_FAILED;
                                    observation =
                                        WindowObservation::timed_out(&watchdog, launch_elapsed_ms);
                                    observation.window_observed = None;
                                    observation.title_matched = None;
                                    observation.timed_out = false;
                                    observation.completed_reason = "observer-error";
                                    proof_status = "window-observation-failed".to_string();
                                    proof_message =
                                    "Power BI Desktop launched, but the window observer failed."
                                        .to_string();
                                    diagnostics.push(json!({
                                    "code": "oracle_failed",
                                    "severity": "error",
                                    "message": format!("Power BI Desktop window observation failed: {err}")
                                }));
                                }
                            }
                        }
                        Ok(Timed::TimedOut) => {
                            exit_code = EXIT_ORACLE_FAILED;
                            proof_status = "launch-timeout".to_string();
                            proof_message = "The Desktop launch command exceeded the launch/observation watchdog before a process id was confirmed.".to_string();
                            diagnostics.push(json!({
                            "code": "oracle_failed",
                            "severity": "error",
                            "message": "Power BI Desktop launch timed out before process start could be confirmed."
                        }));
                        }
                        Err(err) => {
                            exit_code = EXIT_ORACLE_FAILED;
                            proof_status = "launch-failed".to_string();
                            proof_message = "Power BI Desktop launch failed.".to_string();
                            diagnostics.push(json!({
                                "code": "oracle_failed",
                                "severity": "error",
                                "message": format!("Power BI Desktop launch failed: {err}")
                            }));
                        }
                    }
                }
                Ok(Timed::TimedOut) => {
                    exit_code = EXIT_ORACLE_FAILED;
                    proof_status = "observation-setup-timeout".to_string();
                    proof_message = "The launch/observation watchdog expired while recording the pre-launch Desktop process baseline.".to_string();
                    diagnostics.push(json!({
                    "code": "oracle_failed",
                    "severity": "error",
                    "message": "Timed out while recording the Desktop process baseline before launch."
                }));
                }
                Err(err) => {
                    exit_code = EXIT_ORACLE_FAILED;
                    proof_status = "observation-setup-failed".to_string();
                    proof_message =
                        "Could not record the Desktop process baseline before launch.".to_string();
                    diagnostics.push(json!({
                    "code": "oracle_failed",
                    "severity": "error",
                    "message": format!("Could not record the Desktop process baseline before launch: {err}")
                }));
                }
            }
        }
    } else {
        exit_code = EXIT_ORACLE_FAILED;
        proof_status = "oracle-failed".to_string();
        proof_message = "Desktop detection was inconsistent.".to_string();
        diagnostics.push(json!({
            "code": "oracle_failed",
            "severity": "error",
            "message": "Desktop detection reported available but no executable path was resolved."
        }));
    }

    if operation == DesktopOperation::Open && exit_code == EXIT_SUCCESS {
        if let Some(observed_process_id) = managed_session_process_id(
            observation.title_matched,
            observation.observed_process_id,
            &baseline_process_ids,
        ) {
            if let Some(identity) = observed_identity.clone() {
                let draft = DesktopSessionDraft {
                    document_kind: target.kind.as_str().to_string(),
                    document_name: project_name.clone(),
                    document_path: canonical_display(&target.artifact_path),
                    desktop_path: canonical_display(&desktop_path_from_detection(&detection)?),
                    association_process_id: desktop_process_id
                        .expect("successful Desktop launch has an association PID"),
                    observed_identity: identity,
                    baseline_process_ids: baseline_process_ids.clone(),
                    launch_timestamp_unix_ms: launch_timestamp_unix_ms
                        .expect("successful Desktop launch has a timestamp"),
                    opened_at_unix_ms: unix_time_ms().map_err(|error| {
                        CliError::unexpected(format!("record Desktop session time: {error}"))
                    })?,
                };
                match open_desktop_session(
                    managed_session_lock
                        .as_ref()
                        .expect("managed Desktop open holds its lifecycle lock"),
                    draft,
                ) {
                    Ok(session) => {
                        managed_session = Some(session);
                        session_persisted = true;
                        proof_status = "managed-session-open".to_string();
                        proof_message = "Power BI Desktop opened as the single CLI-owned interactive session. Run powerbi-cli desktop close --json when inspection is complete.".to_string();
                    }
                    Err(error) => {
                        exit_code = EXIT_ORACLE_FAILED;
                        proof_status = "session-identity-failed".to_string();
                        proof_message =
                            "Desktop opened, but its exact process identity could not be recorded."
                                .to_string();
                        diagnostics.push(json!({
                            "code": "desktop_session_identity_failed",
                            "severity": "error",
                            "message": error.message
                        }));
                    }
                }
            } else {
                exit_code = EXIT_ORACLE_FAILED;
                proof_status = "session-identity-missing".to_string();
                proof_message = "Desktop opened, but its exact process identity disappeared before ownership could be recorded.".to_string();
                diagnostics.push(json!({
                    "code": "desktop_session_identity_missing",
                    "severity": "error",
                    "message": format!("The exactly observed Desktop PID {observed_process_id} was no longer running when ownership was recorded.")
                }));
            }
        } else {
            exit_code = EXIT_PROOF_INCOMPLETE;
            proof_status = "managed-session-not-owned".to_string();
            proof_message = "Desktop launched, but the exact project window was not a new post-baseline process; the launch will be cleaned up.".to_string();
        }
    }

    let cleanup = cleanup_after_launch(
        launch_attempted,
        association_identity.as_ref(),
        observed_identity.as_ref(),
        &baseline_process_ids,
        operation != DesktopOperation::Open || !session_persisted,
        launch_timestamp_unix_ms,
    );
    if cleanup_unresolved_after_launch(launch_attempted, &cleanup) {
        exit_code = EXIT_ORACLE_FAILED;
        proof_status = "cleanup-failed".to_string();
        proof_message =
            "Desktop proof signals were recorded, but spawned-process cleanup failed.".to_string();
        diagnostics.push(json!({
            "code": "desktop_cleanup_failed",
            "severity": "error",
            "message": "Power BI Desktop launch was attempted but spawned-process cleanup failed.",
            "cleanup": cleanup
        }));
    }

    let proof_passed = match operation {
        DesktopOperation::Open => session_persisted && exit_code == EXIT_SUCCESS,
        DesktopOperation::OpenCheck => launched && exit_code == EXIT_SUCCESS,
        DesktopOperation::Screenshot => {
            screenshot_captured
                && screenshot_foreground_verified == Some(true)
                && exit_code == EXIT_SUCCESS
        }
    };
    let process_id = desktop_process_id.map(Value::from).unwrap_or(Value::Null);
    let window_observed = observation
        .window_observed
        .map(Value::Bool)
        .unwrap_or(Value::Null);
    let title_matched = observation
        .title_matched
        .map(Value::Bool)
        .unwrap_or(Value::Null);
    let unproven_signals = unproven_signals(&observation);
    let screenshot_path = screenshot_out
        .as_ref()
        .map(|path| {
            if screenshot_captured {
                canonical_display(path)
            } else {
                path.display().to_string()
            }
        })
        .map(Value::String)
        .unwrap_or(Value::Null);
    let screenshot_width = screenshot_dimensions
        .as_ref()
        .map(|value| Value::from(value.width))
        .unwrap_or(Value::Null);
    let screenshot_height = screenshot_dimensions
        .as_ref()
        .map(|value| Value::from(value.height))
        .unwrap_or(Value::Null);
    let changes = screenshot_changes(
        screenshot_captured,
        screenshot_replaced_existing,
        screenshot_out.as_deref(),
        screenshot_dimensions.as_ref(),
        screenshot_foreground_verified,
    );

    let mut response = json!({
        "schema": operation.output_schema(),
        "tool": "powerbi-cli",
        "contractVersion": CONTRACT_VERSION,
        "ok": exit_code == EXIT_SUCCESS,
        "exitCode": exit_code,
        "changes": changes,
        "document": target.artifact_json(),
        "oracle": {
            "kind": "powerBiDesktop",
            "available": detection.found && cfg!(windows) && oracle_enabled,
            "platform": std::env::consts::OS,
            "desktopPath": detection.path,
            "desktopVersion": detection.version,
            "detection": {
                "checked": detection.checked,
                "source": detection.source,
                "oracleEnabled": oracle_enabled,
                "requestedDesktopPath": launch_plan.requested_desktop_path
            }
        },
        "validation": {
            "ok": validation_ok,
            "warnings": validation.warnings,
            "errors": validation.errors,
            "counts": {
                "jsonFilesChecked": validation.json_files_checked,
                "tables": validation.tables,
                "relationships": validation.relationships,
                "measures": validation.measures,
                "pages": validation.pages,
                "visuals": validation.visuals,
                "boundVisuals": validation.bound_visuals
            },
            "strict": {
                "enabled": strict_preflight_enabled,
                "ok": strict_preflight_ok,
                "lint": lint
            }
        },
        "proof": {
            "level": DESKTOP_COMMAND_PROOF_LEVEL,
            "observedStage": observed_stage,
            "status": proof_status,
            "passed": proof_passed,
            "claimedCompatibility": false,
            "requiresManualReview": true,
            "requiredCompatibilityLevel": "desktop-canvas-refresh",
            "timeoutMs": options.timeout_ms,
            "timeoutScope": "total budget for the bounded Desktop version probe, process baseline, Desktop launch, and window/title observation; cleanup and screenshot encoding use separate bounded safety timeouts",
            "signals": {
                "processStarted": launched,
                "processId": process_id,
                "desktopVersion": detection.version,
                "launchMethod": launch_plan.method,
                "detectionPathUsedForLaunch": launch_plan.detection_path_used_for_launch,
                "fileAssociationReason": launch_plan.file_association_reason,
                "launchTimestampUnixMs": launch_timestamp_unix_ms,
                "cleanup": cleanup,
                "windowObserved": window_observed,
                "titleMatched": title_matched,
                "observedWindowTitle": observation.observed_window_title,
                "observedProcessId": observation.observed_process_id,
                "observedProcessName": observation.observed_process_name,
                "windowSelectionReason": observation.selection_reason,
                "observation": {
                    "attempted": observation.attempted,
                    "watchdogScope": "desktop-launch-and-window-observation",
                    "budgetMs": options.timeout_ms,
                    "launchElapsedMs": observation.launch_elapsed_ms,
                    "elapsedMs": observation.elapsed_ms,
                    "observedAtMs": observation.observed_at_ms,
                    "pollIntervalMs": WINDOW_POLL_INTERVAL_MS,
                    "polls": observation.polls,
                    "timedOut": observation.timed_out,
                    "completedReason": observation.completed_reason,
                    "baselineProcessIds": baseline_process_ids,
                    "candidateProcessIds": observation.candidate_process_ids,
                    "exactTitleCandidateCount": observation.exact_title_candidate_count
                },
                "screenshotCaptured": if operation == DesktopOperation::Screenshot {
                    Value::Bool(screenshot_captured)
                } else {
                    Value::Null
                },
                "screenshotPath": screenshot_path,
                "screenshotActivationSucceeded": if operation == DesktopOperation::Screenshot {
                    screenshot_activation_succeeded.map(Value::Bool).unwrap_or(Value::Null)
                } else {
                    Value::Null
                },
                "screenshotForegroundVerified": if operation == DesktopOperation::Screenshot {
                    screenshot_foreground_verified.map(Value::Bool).unwrap_or(Value::Null)
                } else {
                    Value::Null
                },
                "screenshotForegroundProcessId": if operation == DesktopOperation::Screenshot {
                    screenshot_foreground_process_id.map(Value::from).unwrap_or(Value::Null)
                } else {
                    Value::Null
                },
                "issuesDialogObserved": Value::Null,
                "canvasRendered": Value::Null,
                "blankCanvasRejected": Value::Null,
                "refreshCompleted": Value::Null
            },
            "unprovenSignals": unproven_signals,
            "compatibility": {
                "claimed": false,
                "currentLevel": DESKTOP_COMMAND_PROOF_LEVEL,
                "observedStage": observed_stage,
                "requiredLevel": "desktop-canvas-refresh",
                "reason": "Desktop launch and exact-title observations are reported as observedStage, not as non-canonical proof levels. Neither observation nor a primary-display screenshot proves that the report canvas rendered, dummy partitions refreshed, or issue banners/dialogs are absent."
            },
            "manualReview": {
                "required": true,
                "checklist": [
                    "Confirm the expected report page tabs are visible.",
                    "Confirm visuals render with dummy rows and are not blank.",
                    "Confirm no issue banners or relationship/data errors remain.",
                    "Refresh in Desktop and re-run fixture normalize/verify after saving a Desktop-authored fixture."
                ]
            },
            "message": proof_message
        },
        "diagnostics": diagnostics,
        "next": desktop_next_commands(operation, &target),
        "plannedNext": [
            "desktop refresh-check",
            "desktop save-check"
        ]
    });

    if operation == DesktopOperation::Open {
        response
            .as_object_mut()
            .expect("desktop response is an object")
            .insert(
                "session".to_string(),
                json!({
                    "state": if session_persisted {
                        "open"
                    } else if cleanup["closed"].as_bool() == Some(true) {
                        "closed"
                    } else {
                        "unknown"
                    },
                    "owned": session_persisted,
                    "document": managed_session.as_ref().map(|_| canonical_display(&target.artifact_path)),
                    "desktopProcessId": managed_session.as_ref().map(|session| session.identity.process_id),
                    "desktopProcessCreationTimeUtc": managed_session.as_ref().map(|session| session.identity.creation_time_utc.as_str()),
                    "desktopExecutablePath": managed_session.as_ref().and_then(|session| session.identity.executable_path.as_deref()),
                    "receiptPath": managed_session.as_ref().map(|session| canonical_display(&session.receipt_path)),
                    "cleanupCommand": "powerbi-cli desktop close --json",
                    "priorSessionCleanup": prior_session_cleanup
                }),
            );
    }

    if operation == DesktopOperation::Screenshot {
        response
            .as_object_mut()
            .expect("desktop response is an object")
            .insert(
                "screenshot".to_string(),
                json!({
                    "path": screenshot_path,
                    "captured": screenshot_captured,
                    "format": "png",
                    "display": "primary",
                    "width": screenshot_width,
                    "height": screenshot_height,
                    "captureTimeoutMs": SCREENSHOT_CAPTURE_TIMEOUT_MS,
                    "activationSucceeded": screenshot_activation_succeeded,
                    "foregroundVerified": screenshot_foreground_verified,
                    "foregroundProcessId": screenshot_foreground_process_id,
                    "allowUnverifiedCapture": options.allow_unverified_capture,
                    "error": screenshot_error,
                    "purpose": "Evidence capture for manual/agent review.",
                    "automatedCompatibilityProof": false,
                    "limitations": [
                        "The PNG captures the primary display, not a parsed Power BI canvas.",
                        "The CLI does not inspect pixels, visuals, issue banners, dialogs, or refresh state.",
                        "A human or screen-capable agent must review the evidence."
                    ]
                }),
            );
    }

    Ok(response)
}

#[cfg(any(windows, test))]
fn cleanup_unresolved_after_launch(launch_attempted: bool, cleanup: &Value) -> bool {
    launch_attempted
        && cleanup["requested"].as_bool() == Some(true)
        && cleanup["closed"].as_bool() != Some(true)
}

#[cfg(not(windows))]
fn run_desktop(operation: DesktopOperation, args: &[String]) -> CliResult<Value> {
    let _options = parse_desktop_args(operation, args)?;
    ensure_desktop_platform(std::env::consts::OS)?;
    Err(CliError::unexpected(
        "Desktop oracle platform dispatch failed",
    ))
}

#[cfg(windows)]
fn desktop_path_from_detection(detection: &PowerBiDesktopDetection) -> CliResult<PathBuf> {
    detection.path_buf.clone().ok_or_else(|| {
        CliError::unexpected("Desktop detection lost its executable path before session receipt")
    })
}

#[cfg(windows)]
fn desktop_next_commands(
    operation: DesktopOperation,
    target: &ResolvedDesktopTarget,
) -> Vec<String> {
    if operation == DesktopOperation::Open {
        return vec!["powerbi-cli desktop close --json".to_string()];
    }
    if target.kind == DesktopTargetKind::Pbix {
        return vec![
            format!(
                "powerbi-cli package inspect {} --json",
                command_arg(&target.artifact_path)
            ),
            format!(
                "powerbi-cli model dax execute --project {} --query \"EVALUATE ROW('Value', 1)\" --allow-data-read --json",
                command_arg(&target.artifact_path)
            ),
            "powerbi-cli --json capabilities --for desktop".to_string(),
        ];
    }
    vec![
        format!(
            "powerbi-cli validate --strict {} --json",
            command_arg(&target.project_dir)
        ),
        format!(
            "powerbi-cli fixture normalize {} --json",
            command_arg(&target.project_dir)
        ),
        "powerbi-cli --json capabilities --for desktop".to_string(),
    ]
}

fn parse_desktop_args(operation: DesktopOperation, args: &[String]) -> CliResult<DesktopOptions> {
    let mut options = DesktopOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                set_project(
                    operation,
                    &mut options.project,
                    PathBuf::from(take_value(args, &mut i, "--project")?),
                )?;
            }
            "--desktop-path" | "--desktop" => {
                options.desktop_path =
                    Some(PathBuf::from(take_value(args, &mut i, "--desktop-path")?));
            }
            "--out" if operation == DesktopOperation::Screenshot => {
                if options.out.is_some() {
                    return Err(CliError::invalid_args(
                        "desktop screenshot accepts exactly one --out path",
                    )
                    .with_hint("Pass one PNG evidence path separate from the selected document.")
                    .with_suggested_command(operation.suggested_command()));
                }
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            "--leave-open" | "--leaveOpen" => {
                return Err(CliError::invalid_args(
                    "--leave-open has no bounded ownership lifetime",
                )
                .with_hint(
                    "Use desktop open for an interactive CLI-owned session, then desktop close when inspection is complete.",
                )
                .with_suggested_command(
                    "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> --json",
                )
                .with_suggested_command("powerbi-cli desktop close --json"));
            }
            "--allow-unverified-capture" if operation == DesktopOperation::Screenshot => {
                options.allow_unverified_capture = true;
                i += 1;
            }
            "--timeout-ms" | "--timeoutMs" => {
                let value = take_value(args, &mut i, "--timeout-ms")?;
                options.timeout_ms = value.parse::<u64>().map_err(|_| {
                    CliError::invalid_args("--timeout-ms must be a positive integer")
                        .with_hint("Use milliseconds, for example --timeout-ms 120000.")
                        .with_suggested_command(operation.suggested_command())
                })?;
                if options.timeout_ms == 0 {
                    return Err(
                        CliError::invalid_args("--timeout-ms must be greater than zero")
                            .with_hint("Use a positive millisecond budget.")
                            .with_suggested_command(operation.suggested_command()),
                    );
                }
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown {} flag: {other}",
                    operation.command_path()
                ))
                .with_hint(format!(
                    "Run powerbi-cli --json capabilities --for \"{}\" for exact flags.",
                    operation.command_path()
                ))
                .with_suggested_command(format!(
                    "powerbi-cli --json capabilities --for \"{}\"",
                    operation.command_path()
                )));
            }
            positional => {
                set_project(operation, &mut options.project, PathBuf::from(positional))?;
                i += 1;
            }
        }
    }
    Ok(options)
}

pub(crate) fn ensure_desktop_platform(platform: &str) -> CliResult<()> {
    if platform == "windows" {
        return Ok(());
    }
    Err(CliError::unsupported_feature(format!(
        "desktop oracle commands are unsupported on {platform}; Power BI Desktop automation requires Windows"
    ))
    .with_hint(
        "Use native PBIP/PBIX inspection on this platform, then move Desktop-only work to an explicitly opted-in Windows machine.",
    )
    .with_suggested_command(
        "powerbi-cli package inspect <file.pbix> --json",
    ))
}

fn set_project(
    operation: DesktopOperation,
    current: &mut Option<PathBuf>,
    next: PathBuf,
) -> CliResult<()> {
    if current.is_some() {
        return Err(CliError::invalid_args(format!(
            "{} accepts exactly one project path",
            operation.command_path()
        ))
        .with_hint("Use either a positional project path or --project, not both.")
        .with_suggested_command(operation.suggested_command()));
    }
    *current = Some(next);
    Ok(())
}

#[cfg(windows)]
fn validate_screenshot_output(out: &Path, target: &ResolvedDesktopTarget) -> CliResult<PathBuf> {
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

#[cfg(windows)]
fn unproven_signals(observation: &WindowObservation) -> Vec<&'static str> {
    let mut signals = Vec::new();
    if observation.window_observed.is_none() {
        signals.push("windowObserved");
    }
    if observation.title_matched.is_none() {
        signals.push("titleMatched");
    }
    signals.extend([
        "issuesDialogObserved",
        "canvasRendered",
        "blankCanvasRejected",
        "refreshCompleted",
    ]);
    signals
}

#[cfg(any(windows, test))]
fn screenshot_observation_is_eligible(title_matched: Option<bool>) -> bool {
    title_matched == Some(true)
}

#[cfg(windows)]
fn observe_window(
    launched_pid: u32,
    baseline_process_ids: &[u32],
    project_name: &str,
    watchdog: &Watchdog,
    launch_elapsed_ms: u64,
) -> io::Result<WindowObservation> {
    let mut observation = WindowObservation::timed_out(watchdog, launch_elapsed_ms);
    observation.timed_out = false;
    observation.completed_reason = "polling";
    let baseline = baseline_process_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut candidate_ids = BTreeSet::new();

    loop {
        let remaining = watchdog.remaining();
        if remaining.is_zero() {
            observation.timed_out = true;
            observation.completed_reason = "timeout";
            break;
        }
        observation.polls += 1;
        let processes = match query_desktop_windows(remaining)? {
            Timed::Completed(processes) => processes,
            Timed::TimedOut => {
                observation.timed_out = true;
                observation.completed_reason = "timeout";
                break;
            }
        };
        let mut titled_candidates = processes
            .into_iter()
            .filter(|process| is_power_bi_desktop_process(&process.process_name))
            .filter(|process| {
                process.id == launched_pid
                    || !baseline.contains(&process.id)
                    || title_matches_project(&process.main_window_title, project_name)
            })
            .filter(|process| !process.main_window_title.trim().is_empty())
            .collect::<Vec<_>>();
        titled_candidates.sort_by_key(|process| process.id);
        for process in &titled_candidates {
            candidate_ids.insert(process.id);
        }
        let selection = select_window_candidate(
            &titled_candidates,
            launched_pid,
            baseline_process_ids,
            project_name,
        );
        observation.exact_title_candidate_count = observation
            .exact_title_candidate_count
            .max(selection.exact_title_candidate_count);
        if let Some(process) = selection.process {
            let matched = title_matches_project(&process.main_window_title, project_name);
            observation.window_observed = Some(true);
            observation.title_matched = Some(matched);
            observation.observed_window_title = Some(process.main_window_title);
            observation.observed_process_id = Some(process.id);
            observation.observed_process_name = Some(process.process_name);
            observation.selection_reason = selection.reason;
            observation
                .observed_at_ms
                .get_or_insert_with(|| watchdog.elapsed_ms());
            if matched {
                observation.completed_reason = "title-matched";
                break;
            }
        }
        let sleep_for = watchdog
            .remaining()
            .min(Duration::from_millis(WINDOW_POLL_INTERVAL_MS));
        if sleep_for.is_zero() {
            observation.timed_out = true;
            observation.completed_reason = "timeout";
            break;
        }
        std::thread::sleep(sleep_for);
    }

    observation.elapsed_ms = watchdog.elapsed_ms();
    observation.candidate_process_ids = candidate_ids.into_iter().collect();
    Ok(observation)
}

#[cfg(any(windows, test))]
struct WindowCandidateSelection {
    process: Option<ProcessWindow>,
    exact_title_candidate_count: usize,
    reason: Option<&'static str>,
}

#[cfg(any(windows, test))]
fn select_window_candidate(
    processes: &[ProcessWindow],
    launched_pid: u32,
    baseline_process_ids: &[u32],
    project_name: &str,
) -> WindowCandidateSelection {
    let exact = processes
        .iter()
        .filter(|process| title_matches_project(&process.main_window_title, project_name))
        .collect::<Vec<_>>();
    let exact_title_candidate_count = exact.len();

    let selected = exact
        .iter()
        .find(|process| process.id == launched_pid)
        .map(|process| ((*process).clone(), "association-launch-pid"))
        .or_else(|| {
            exact
                .iter()
                .find(|process| !baseline_process_ids.contains(&process.id))
                .map(|process| ((*process).clone(), "new-desktop-process"))
        })
        .or_else(|| (exact.len() == 1).then(|| (exact[0].clone(), "unique-title-fallback")))
        .or_else(|| {
            processes
                .iter()
                .find(|process| process.id == launched_pid)
                .map(|process| (process.clone(), "association-launch-diagnostic"))
        })
        .or_else(|| {
            processes
                .iter()
                .find(|process| !baseline_process_ids.contains(&process.id))
                .map(|process| (process.clone(), "new-process-diagnostic"))
        });

    WindowCandidateSelection {
        process: selected.as_ref().map(|(process, _)| process.clone()),
        exact_title_candidate_count,
        reason: selected.map(|(_, reason)| reason),
    }
}

#[cfg(any(windows, test))]
fn managed_session_process_id(
    title_matched: Option<bool>,
    observed_process_id: Option<u32>,
    baseline_process_ids: &[u32],
) -> Option<u32> {
    if title_matched != Some(true) {
        return None;
    }
    observed_process_id.filter(|process_id| !baseline_process_ids.contains(process_id))
}

#[cfg(any(windows, test))]
fn title_matches_project(title: &str, project_name: &str) -> bool {
    let title = normalize_window_title(title);
    let project_name = normalize_window_title(project_name);
    if project_name.is_empty() {
        return false;
    }
    if title == project_name {
        return true;
    }
    [" - ", " – ", " — "].iter().any(|separator| {
        title
            .rsplit_once(separator)
            .is_some_and(|(stem, suffix)| stem == project_name && suffix == "power bi desktop")
    })
}

#[cfg(any(windows, test))]
fn normalize_window_title(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(any(windows, test))]
fn is_power_bi_desktop_process(process_name: &str) -> bool {
    process_name
        .trim()
        .to_ascii_lowercase()
        .starts_with("pbidesktop")
}

#[cfg(windows)]
fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(windows)]
fn unix_time_ms() -> io::Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| io::Error::other(format!("system clock is before Unix epoch: {err}")))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| io::Error::other("current Unix timestamp does not fit in u64 milliseconds"))
}

#[cfg(any(windows, test))]
fn remaining_budget(budget: Duration, elapsed: Duration) -> Duration {
    budget.saturating_sub(elapsed)
}

#[cfg(windows)]
fn snapshot_desktop_process_ids(timeout: Duration) -> io::Result<Timed<Vec<u32>>> {
    let script = render_process_snapshot_script();
    match run_powershell(&script, timeout)? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "Desktop process snapshot")?;
            let processes: Vec<ProcessWindow> = parse_powershell_json(&output.stdout)?;
            Ok(Timed::Completed(
                processes.into_iter().map(|process| process.id).collect(),
            ))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const PROCESS_SNAPSHOT_SCRIPT: &str = r#"
$items = @(
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessName -like 'PBIDesktop*' -or $_.ProcessName -eq 'msmdsrv' } |
        ForEach-Object {
            [pscustomobject]@{
                id = [int]$_.Id
                processName = [string]$_.ProcessName
                mainWindowTitle = [string]$_.MainWindowTitle
            }
        }
)
[Console]::Out.Write((ConvertTo-Json -InputObject $items -Compress))
"#;

#[cfg(any(windows, test))]
fn render_process_snapshot_script() -> String {
    PROCESS_SNAPSHOT_SCRIPT.to_string()
}

#[cfg(windows)]
fn query_desktop_windows(timeout: Duration) -> io::Result<Timed<Vec<ProcessWindow>>> {
    let script = render_window_query_script();
    match run_powershell(&script, timeout)? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "Desktop window query")?;
            Ok(Timed::Completed(parse_powershell_json(&output.stdout)?))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const WINDOW_QUERY_SCRIPT: &str = r#"
$items = @(
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessName -like 'PBIDesktop*' } |
        ForEach-Object {
            [pscustomobject]@{
                id = [int]$_.Id
                processName = [string]$_.ProcessName
                mainWindowTitle = [string]$_.MainWindowTitle
            }
        }
)
[Console]::Out.Write((ConvertTo-Json -InputObject $items -Compress))
"#;

#[cfg(any(windows, test))]
fn render_window_query_script() -> String {
    WINDOW_QUERY_SCRIPT.to_string()
}

#[cfg(windows)]
fn launch_desktop(
    _desktop_path: &Path,
    pbip_path: &Path,
    _launch_plan: &DesktopLaunchPlan,
    timeout: Duration,
) -> io::Result<Timed<u32>> {
    let pbip_arg = desktop_argument_path(pbip_path);
    let script = render_launch_script(&pbip_arg);
    match run_powershell(&script, timeout)? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "PowerShell Start-Process")?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let process_id = stdout.trim().parse::<u32>().map_err(|err| {
                io::Error::other(format!(
                    "PowerShell Start-Process returned invalid process id {stdout:?}: {err}"
                ))
            })?;
            Ok(Timed::Completed(process_id))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const LAUNCH_SCRIPT: &str = r#"
$p = Start-Process -FilePath __PBIP_PATH__ -PassThru
[Console]::Out.Write($p.Id)
"#;

#[cfg(any(windows, test))]
fn render_launch_script(pbip_path: &str) -> String {
    LAUNCH_SCRIPT.replace("__PBIP_PATH__", &powershell_single_quoted(pbip_path))
}

#[cfg(any(windows, test))]
fn screenshot_changes(
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
fn capture_primary_display(
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
        &desktop_argument_path(&temp),
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
            if let Err(err) =
                ensure_powershell_success(&output, "primary-display screenshot capture")
            {
                remove_file_if_present(&temp);
                return Err(err);
            }
            let result: ScreenshotCaptureResult = match parse_powershell_json(&output.stdout) {
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
fn render_screenshot_script(
    out_path: &str,
    foreground_pid: Option<u32>,
    settle_ms: u64,
    allow_unverified_capture: bool,
) -> String {
    SCREENSHOT_SCRIPT
        .replace("__OUT_PATH__", &powershell_single_quoted(out_path))
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

#[cfg(windows)]
fn cleanup_after_launch(
    launch_attempted: bool,
    association_identity: Option<&ProcessIdentity>,
    observed_identity: Option<&ProcessIdentity>,
    baseline_process_ids: &[u32],
    close_after: bool,
    launch_timestamp_unix_ms: Option<u64>,
) -> Value {
    let association_process_id = association_identity.map(|identity| identity.process_id);
    let observed_process_id = observed_identity.map(|identity| identity.process_id);
    if !launch_attempted || !close_after {
        return json!({
            "requested": close_after,
            "attempted": false,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "closed": Value::Null,
            "skipped": [],
            "refusedReason": Value::Null,
            "errors": []
        });
    }
    if association_identity.is_none() && observed_identity.is_none() {
        return cleanup_refused(
            baseline_process_ids,
            launch_timestamp_unix_ms,
            "cleanup refused because no exact launch process identity was confirmed",
        );
    }
    let Some(launch_timestamp_unix_ms) = launch_timestamp_unix_ms else {
        return cleanup_refused(
            baseline_process_ids,
            None,
            "cleanup refused because the launch timestamp was unavailable",
        );
    };
    match cleanup_spawned_processes(
        association_identity,
        observed_identity,
        baseline_process_ids,
        launch_timestamp_unix_ms,
    ) {
        Ok(Timed::Completed(cleanup)) => json!({
            "requested": true,
            "attempted": true,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": cleanup["targeted"],
            "targetedProcessIds": cleanup["targetedProcessIds"],
            "remainingProcessIds": cleanup["remainingProcessIds"],
            "closed": cleanup["closed"],
            "skipped": cleanup["skipped"],
            "refusedReason": Value::Null,
            "errors": cleanup["errors"]
        }),
        Ok(Timed::TimedOut) => json!({
            "requested": true,
            "attempted": true,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "closed": false,
            "skipped": [],
            "refusedReason": Value::Null,
            "errors": [format!("spawned-process cleanup exceeded {CLEANUP_TIMEOUT_MS} ms")]
        }),
        Err(err) => json!({
            "requested": true,
            "attempted": true,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "closed": false,
            "skipped": [],
            "refusedReason": Value::Null,
            "errors": [err.to_string()]
        }),
    }
}

#[cfg(windows)]
fn cleanup_refused(
    baseline_process_ids: &[u32],
    launch_timestamp_unix_ms: Option<u64>,
    reason: &str,
) -> Value {
    json!({
        "requested": true,
        "attempted": false,
        "associationProcessId": Value::Null,
        "baselineProcessIds": baseline_process_ids,
        "launchTimestampUnixMs": launch_timestamp_unix_ms,
        "targeted": [],
        "targetedProcessIds": [],
        "remainingProcessIds": [],
        "closed": Value::Null,
        "skipped": [],
        "refusedReason": reason,
        "errors": []
    })
}

#[cfg(windows)]
pub(crate) fn cleanup_spawned_processes(
    association_identity: Option<&ProcessIdentity>,
    observed_identity: Option<&ProcessIdentity>,
    baseline_process_ids: &[u32],
    launch_timestamp_unix_ms: u64,
) -> io::Result<Timed<Value>> {
    let script = render_cleanup_script(
        association_identity,
        observed_identity,
        baseline_process_ids,
        launch_timestamp_unix_ms,
    );
    match run_powershell(&script, Duration::from_millis(CLEANUP_TIMEOUT_MS))? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "spawned Desktop process cleanup")?;
            Ok(Timed::Completed(parse_powershell_json(&output.stdout)?))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const CLEANUP_SCRIPT: &str = r#"
$baseline = @(__BASELINE_IDS__)
$associationPid = __ASSOCIATION_PID__
$associationCreationTimeUtc = __ASSOCIATION_CREATION_TIME_UTC__
$observedPid = __OBSERVED_PID__
$observedCreationTimeUtc = __OBSERVED_CREATION_TIME_UTC__
$launchTimeUtc = [DateTimeOffset]::FromUnixTimeMilliseconds(__LAUNCH_TIME_UNIX_MS__).UtcDateTime
$targetReasons = @{}
$targetCreationUtc = @{}
$lineageRoots = [System.Collections.Generic.HashSet[int]]::new()
$skipped = [System.Collections.Generic.List[object]]::new()
$errors = [System.Collections.Generic.List[string]]::new()
try {
    $rows = @(Get-CimInstance Win32_Process -ErrorAction Stop)
} catch {
    $rows = @()
    [void]$errors.Add("process inventory failed: $($_.Exception.Message)")
}
$rowsById = @{}
foreach ($row in $rows) {
    $rowsById[[int]$row.ProcessId] = $row
}

function Add-OwnedTarget {
    param(
        [int]$ProcessId,
        [string]$Reason,
        [string]$ExpectedCreationTimeUtc = '',
        [bool]$RequireDesktop = $false
    )
    if ($ProcessId -le 0 -or $targetReasons.ContainsKey($ProcessId)) {
        return $false
    }
    if ($baseline -contains $ProcessId) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'baseline-pid' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: process existed in the pre-launch baseline")
        return $false
    }
    if (-not $rowsById.ContainsKey($ProcessId)) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-unavailable' })
        [void]$errors.Add("PID ${ProcessId} ownership unresolved: CIM process row unavailable")
        return $false
    }
    $row = $rowsById[$ProcessId]
    if ($RequireDesktop -and [string]$row.Name -notlike 'PBIDesktop*') {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'owned-root-is-not-desktop' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: recorded root is not a PBIDesktop process")
        return $false
    }
    if ($null -eq $row.CreationDate) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-unavailable' })
        [void]$errors.Add("PID ${ProcessId} ownership unresolved: CreationDate unavailable")
        return $false
    }
    try {
        $createdAtUtc = ([DateTime]$row.CreationDate).ToUniversalTime()
    } catch {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-invalid' })
        [void]$errors.Add("PID ${ProcessId} ownership unresolved: invalid CreationDate")
        return $false
    }
    if ($createdAtUtc -le $launchTimeUtc) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'created-before-or-at-launch' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: CreationDate predates or equals launch")
        return $false
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedCreationTimeUtc) -and $createdAtUtc -ne ([DateTime]::Parse($ExpectedCreationTimeUtc)).ToUniversalTime()) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-no-longer-matches-recorded-identity' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: CreationDate no longer matches the recorded launch identity")
        return $false
    }
    $targetReasons[$ProcessId] = $Reason
    $targetCreationUtc[$ProcessId] = $createdAtUtc
    [void]$lineageRoots.Add($ProcessId)
    return $true
}

if ($associationPid -gt 0) {
    if ([string]::IsNullOrWhiteSpace($associationCreationTimeUtc)) {
        [void]$errors.Add("PID ${associationPid} cleanup refused: recorded association creation time is unavailable")
    } elseif (Get-Process -Id $associationPid -ErrorAction SilentlyContinue) {
        [void](Add-OwnedTarget -ProcessId $associationPid -Reason 'association-launch-pid' -ExpectedCreationTimeUtc $associationCreationTimeUtc -RequireDesktop $true)
    } else {
        [void]$skipped.Add([pscustomobject]@{ pid = $associationPid; reason = 'association-pid-already-exited' })
    }
}
if ($observedPid -gt 0 -and $observedPid -ne $associationPid) {
    if ([string]::IsNullOrWhiteSpace($observedCreationTimeUtc)) {
        [void]$errors.Add("PID ${observedPid} cleanup refused: recorded observed creation time is unavailable")
    } elseif (Get-Process -Id $observedPid -ErrorAction SilentlyContinue) {
        [void](Add-OwnedTarget -ProcessId $observedPid -Reason 'exact-observed-pid' -ExpectedCreationTimeUtc $observedCreationTimeUtc -RequireDesktop $true)
    } else {
        [void]$skipped.Add([pscustomobject]@{ pid = $observedPid; reason = 'observed-pid-already-exited' })
    }
}
$changed = $true
while ($changed) {
    $changed = $false
    foreach ($row in $rows) {
        $parentId = [int]$row.ParentProcessId
        $childId = [int]$row.ProcessId
        if ($lineageRoots.Contains($parentId) -and -not $targetReasons.ContainsKey($childId)) {
            if (Add-OwnedTarget -ProcessId $childId -Reason "descendant-of-$parentId") {
                $changed = $true
            }
        }
    }
}
$orderedTargets = @($targetReasons.Keys | ForEach-Object { [int]$_ } | Sort-Object -Descending)
$targeted = @(
    $orderedTargets | ForEach-Object {
        $ownedCreationTime = [DateTime]($targetCreationUtc[[int]$_])
        [pscustomobject]@{
            pid = [int]$_
            reason = [string]$targetReasons[[int]$_]
            creationTimeUtc = $ownedCreationTime.ToString('o')
        }
    }
)
foreach ($targetId in $orderedTargets) {
    if ($baseline -contains [int]$targetId) {
        [void]$errors.Add("PID ${targetId} kill refused: baseline PID")
        continue
    }
    $currentRows = @(Get-CimInstance Win32_Process -Filter "ProcessId = $targetId" -ErrorAction SilentlyContinue)
    if ($currentRows.Count -eq 0) {
        continue
    }
    $currentRow = $currentRows[0]
    if ($null -eq $currentRow.CreationDate) {
        [void]$errors.Add("PID ${targetId} kill refused: current CreationDate unavailable")
        continue
    }
    $currentCreatedAtUtc = ([DateTime]$currentRow.CreationDate).ToUniversalTime()
    $ownedCreatedAtUtc = [DateTime]($targetCreationUtc[[int]$targetId])
    if (
        $currentCreatedAtUtc -le $launchTimeUtc -or
        $currentCreatedAtUtc -ne $ownedCreatedAtUtc
    ) {
        [void]$errors.Add("PID ${targetId} kill refused: creation time no longer matches owned process")
        continue
    }
    try {
        Stop-Process -Id $targetId -Force -ErrorAction Stop
    } catch {
        if (Get-Process -Id $targetId -ErrorAction SilentlyContinue) {
            [void]$errors.Add("PID ${targetId} stop failed: $($_.Exception.Message)")
        }
    }
}
Start-Sleep -Milliseconds 200
$remaining = @(
    $orderedTargets |
        Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue } |
        ForEach-Object { [int]$_ }
)
$result = [pscustomobject]@{
    targeted = @($targeted)
    targetedProcessIds = @($orderedTargets)
    remainingProcessIds = @($remaining)
    closed = ($remaining.Count -eq 0 -and $errors.Count -eq 0)
    skipped = @($skipped)
    errors = @($errors)
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Compress -Depth 6))
"#;

#[cfg(any(windows, test))]
fn render_cleanup_script(
    association_identity: Option<&ProcessIdentity>,
    observed_identity: Option<&ProcessIdentity>,
    baseline_process_ids: &[u32],
    launch_timestamp_unix_ms: u64,
) -> String {
    let baseline = baseline_process_ids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    CLEANUP_SCRIPT
        .replace("__BASELINE_IDS__", &baseline)
        .replace(
            "__ASSOCIATION_PID__",
            &association_identity
                .map(|identity| identity.process_id)
                .unwrap_or_default()
                .to_string(),
        )
        .replace(
            "__ASSOCIATION_CREATION_TIME_UTC__",
            &powershell_single_quoted(
                association_identity
                    .map(|identity| identity.creation_time_utc.as_str())
                    .unwrap_or_default(),
            ),
        )
        .replace(
            "__OBSERVED_PID__",
            &observed_identity
                .map(|identity| identity.process_id)
                .unwrap_or_default()
                .to_string(),
        )
        .replace(
            "__OBSERVED_CREATION_TIME_UTC__",
            &powershell_single_quoted(
                observed_identity
                    .map(|identity| identity.creation_time_utc.as_str())
                    .unwrap_or_default(),
            ),
        )
        .replace(
            "__LAUNCH_TIME_UNIX_MS__",
            &launch_timestamp_unix_ms.to_string(),
        )
}

#[cfg(windows)]
pub(crate) fn read_process_identity(process_id: u32) -> CliResult<Option<ProcessIdentity>> {
    let script = PROCESS_IDENTITY_SCRIPT.replace("__PROCESS_ID__", &process_id.to_string());
    match run_powershell(&script, Duration::from_millis(5_000)).map_err(|error| {
        CliError::unexpected(format!(
            "inspect Power BI Desktop process {process_id}: {error}"
        ))
    })? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "Power BI Desktop process identity")
                .map_err(|error| CliError::unexpected(error.to_string()))?;
            parse_powershell_json(&output.stdout).map_err(|error| {
                CliError::unexpected(format!(
                    "parse Power BI Desktop process {process_id} identity: {error}"
                ))
            })
        }
        Timed::TimedOut => Err(CliError::unexpected(format!(
            "Power BI Desktop process {process_id} identity check exceeded 5000 ms"
        ))),
    }
}

#[cfg(any(windows, test))]
const PROCESS_IDENTITY_SCRIPT: &str = r#"
$row = @(Get-CimInstance Win32_Process -Filter "ProcessId = __PROCESS_ID__" -ErrorAction Stop)
if ($row.Count -eq 0) {
    [Console]::Out.Write('null')
    return
}
$process = $row[0]
$result = [pscustomobject]@{
    processId = [int]$process.ProcessId
    creationTimeUtc = ([DateTime]$process.CreationDate).ToUniversalTime().ToString('o')
    executablePath = if ([string]::IsNullOrWhiteSpace([string]$process.ExecutablePath)) { $null } else { [string]$process.ExecutablePath }
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Compress))
"#;

#[cfg(test)]
fn render_process_identity_script(process_id: u32) -> String {
    PROCESS_IDENTITY_SCRIPT.replace("__PROCESS_ID__", &process_id.to_string())
}

#[cfg(windows)]
fn run_powershell(script: &str, timeout: Duration) -> io::Result<Timed<std::process::Output>> {
    if timeout.is_zero() {
        return Ok(Timed::TimedOut);
    }
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); {script}"
    );
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
        ])
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    run_command_with_timeout(command, timeout)
}

#[cfg(windows)]
pub(crate) fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> io::Result<Timed<std::process::Output>> {
    let started = Instant::now();
    let mut child = command.spawn()?;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map(Timed::Completed);
        }
        let remaining = remaining_budget(timeout, started.elapsed());
        if remaining.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Timed::TimedOut);
        }
        std::thread::sleep(remaining.min(Duration::from_millis(COMMAND_POLL_INTERVAL_MS)));
    }
}

#[cfg(windows)]
fn ensure_powershell_success(output: &std::process::Output, action: &str) -> io::Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{action} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[cfg(windows)]
fn parse_powershell_json<T>(bytes: &[u8]) -> io::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim().trim_start_matches('\u{feff}');
    serde_json::from_str(text)
        .map_err(|err| io::Error::other(format!("parse PowerShell JSON output: {err}: {text}")))
}

#[cfg(any(windows, test))]
fn powershell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(any(windows, test))]
fn desktop_argument_path(path: &Path) -> String {
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
fn desktop_launch_plan(
    options: &DesktopOptions,
    detection: &PowerBiDesktopDetection,
) -> DesktopLaunchPlan {
    let requested_desktop_path = options
        .desktop_path
        .as_ref()
        .map(|path| canonical_display(path));
    DesktopLaunchPlan {
        method: desktop_launch_method(),
        detection_path_used_for_launch: cfg!(not(windows)) && detection.path_buf.is_some(),
        requested_desktop_path,
        file_association_reason: if cfg!(windows) {
            Some(
                "Power BI Desktop Store installs reject direct PBIP executable arguments; Windows Desktop proof launches the .pbip through the registered file association after executable detection. The returned association PID may be a short-lived proxy, so observation requires an exact project title on PBIDesktop and cleanup combines parent lineage with baseline, executable-path, and post-launch creation-time guards.",
            )
        } else {
            None
        },
    }
}

#[cfg(windows)]
fn desktop_launch_method() -> &'static str {
    if cfg!(windows) {
        "windows-file-association"
    } else {
        "direct-executable"
    }
}

fn power_bi_desktop_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    append_store_install_candidates(&mut candidates);
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let windows_apps = PathBuf::from(local_app_data).join("Microsoft\\WindowsApps");
        candidates.push(
            windows_apps.join("Microsoft.MicrosoftPowerBIDesktop_8wekyb3d8bbwe\\PBIDesktop.exe"),
        );
        candidates.push(
            windows_apps
                .join("Microsoft.MicrosoftPowerBIDesktop_8wekyb3d8bbwe\\PBIDesktopStore.exe"),
        );
        candidates.push(windows_apps.join("PBIDesktopStore.exe"));
    }
    candidates.push(PathBuf::from(
        "C:\\Program Files\\Microsoft Power BI Desktop\\bin\\PBIDesktop.exe",
    ));
    candidates.push(PathBuf::from(
        "C:\\Program Files (x86)\\Microsoft Power BI Desktop\\bin\\PBIDesktop.exe",
    ));
    candidates
}

fn append_store_install_candidates(candidates: &mut Vec<PathBuf>) {
    let Ok(program_files) = std::env::var("ProgramFiles") else {
        return;
    };
    let windows_apps = PathBuf::from(program_files).join("WindowsApps");
    let Ok(entries) = std::fs::read_dir(windows_apps) else {
        return;
    };
    let mut package_dirs = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("Microsoft.MicrosoftPowerBIDesktop_"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    package_dirs.sort();
    package_dirs.reverse();
    for package_dir in package_dirs {
        candidates.push(package_dir.join("bin\\PBIDesktop.exe"));
    }
}

#[cfg(windows)]
fn lint_error_findings(lint: Option<&Value>) -> Vec<Value> {
    lint.and_then(|value| value["findings"].as_array())
        .map(|findings| {
            findings
                .iter()
                .filter(|finding| finding["severity"] == "error")
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(windows)]
fn desktop_file_version(
    path: Option<&Path>,
    timeout: Duration,
) -> io::Result<Timed<Option<String>>> {
    let Some(path) = path else {
        return Ok(Timed::Completed(None));
    };
    let script = render_version_script(&desktop_argument_path(path));
    match run_powershell(&script, timeout)? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "Power BI Desktop version probe")?;
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(Timed::Completed((!version.is_empty()).then_some(version)))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const VERSION_SCRIPT: &str = "(Get-Item -LiteralPath __DESKTOP_PATH__).VersionInfo.ProductVersion";

#[cfg(any(windows, test))]
fn render_version_script(desktop_path: &str) -> String {
    VERSION_SCRIPT.replace("__DESKTOP_PATH__", &powershell_single_quoted(desktop_path))
}

#[cfg(windows)]
fn oracle_enabled() -> bool {
    std::env::var("POWERBI_DESKTOP_ORACLE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run powerbi-cli --json capabilities --for desktop for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for desktop")
    })?;
    *index += 2;
    Ok(value.clone())
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopOperation, ProcessIdentity, ProcessWindow, ScreenshotCaptureResult,
        ScreenshotDimensions, capture_is_authorized, cleanup_unresolved_after_launch,
        desktop_argument_path, detect_power_bi_desktop, ensure_desktop_platform,
        is_power_bi_desktop_process, managed_session_process_id, parse_desktop_args,
        path_is_within_directory, powershell_single_quoted, publish_screenshot, remaining_budget,
        render_cleanup_script, render_launch_script, render_process_identity_script,
        render_process_snapshot_script, render_screenshot_script, render_version_script,
        render_window_query_script, screenshot_changes, screenshot_observation_is_eligible,
        select_window_candidate, title_matches_project,
    };
    use serde_json::json;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    fn process_identity(process_id: u32, creation_time_utc: &str) -> ProcessIdentity {
        ProcessIdentity {
            process_id,
            creation_time_utc: creation_time_utc.to_string(),
            #[cfg(windows)]
            executable_path: Some(r"C:\Program Files\Power BI\PBIDesktop.exe".to_string()),
        }
    }

    #[test]
    fn desktop_argument_path_strips_verbatim_drive_prefix() {
        let path = Path::new(r"\\?\C:\Reports\RegionalSales.pbip");
        assert_eq!(
            desktop_argument_path(path),
            r"C:\Reports\RegionalSales.pbip"
        );
    }

    #[test]
    fn desktop_argument_path_strips_verbatim_unc_prefix() {
        let path = Path::new(r"\\?\UNC\server\share\RegionalSales.pbip");
        assert_eq!(
            desktop_argument_path(path),
            r"\\server\share\RegionalSales.pbip"
        );
    }

    #[test]
    fn desktop_argument_path_leaves_normal_paths_alone() {
        let path = Path::new(r"C:\Reports\RegionalSales.pbip");
        assert_eq!(
            desktop_argument_path(path),
            r"C:\Reports\RegionalSales.pbip"
        );
    }

    #[test]
    fn title_matching_uses_exact_normalized_project_stem() {
        // Committed Desktop proof artifacts record the plain project stem.
        assert!(title_matches_project(
            "WorkshopOperations",
            "workshopoperations"
        ));
        assert!(title_matches_project(
            "WorkshopOperations - Power BI Desktop",
            "workshopoperations"
        ));
        assert!(title_matches_project(
            "  WorkshopOperations   –   Power BI Desktop  ",
            "WorkshopOperations"
        ));
        assert!(title_matches_project(
            "WorkshopOperations — Power BI Desktop",
            "WorkshopOperations"
        ));
        assert!(!title_matches_project(
            "OtherReport - Power BI Desktop",
            "WorkshopOperations"
        ));
        assert!(!title_matches_project(
            "AnnualSales - Power BI Desktop",
            "Sales"
        ));
        assert!(!title_matches_project(
            "Sales Dashboard - Power BI Desktop",
            "Sales"
        ));
        assert!(!title_matches_project(
            "Sales - Power BI Desktop Preview",
            "Sales"
        ));
        assert!(!title_matches_project("Power BI Desktop", ""));
    }

    #[test]
    fn every_window_candidate_must_be_a_desktop_process() {
        assert!(is_power_bi_desktop_process("PBIDesktop"));
        assert!(is_power_bi_desktop_process("PBIDesktopStore"));
        assert!(!is_power_bi_desktop_process("explorer"));
        assert!(!is_power_bi_desktop_process("msmdsrv"));
    }

    #[test]
    fn duplicate_titles_prefer_the_new_process_instead_of_the_oldest_pid() {
        let processes = vec![
            ProcessWindow {
                id: 17264,
                process_name: "PBIDesktop".to_string(),
                main_window_title: "SafetyDashboard".to_string(),
            },
            ProcessWindow {
                id: 37004,
                process_name: "PBIDesktop".to_string(),
                main_window_title: "SafetyDashboard - Power BI Desktop".to_string(),
            },
        ];
        let selected = select_window_candidate(&processes, 999, &[17264], "SafetyDashboard");

        let process = selected.process.expect("new process");
        assert_eq!(process.id, 37004);
        assert_eq!(process.process_name, "PBIDesktop");
        assert_eq!(selected.reason, Some("new-desktop-process"));
        assert_eq!(selected.exact_title_candidate_count, 2);
    }

    #[test]
    fn duplicate_baseline_titles_are_ambiguous_instead_of_guessed() {
        let processes = vec![
            ProcessWindow {
                id: 100,
                process_name: "PBIDesktop".to_string(),
                main_window_title: "SameReport".to_string(),
            },
            ProcessWindow {
                id: 200,
                process_name: "PBIDesktopStore".to_string(),
                main_window_title: "SameReport".to_string(),
            },
        ];
        let selected = select_window_candidate(&processes, 999, &[100, 200], "SameReport");

        assert!(selected.process.is_none());
        assert_eq!(selected.reason, None);
        assert_eq!(selected.exact_title_candidate_count, 2);
    }

    #[test]
    fn managed_session_never_owns_a_unique_baseline_window() {
        assert_eq!(
            managed_session_process_id(Some(true), Some(41), &[41]),
            None
        );
        assert_eq!(
            managed_session_process_id(Some(true), Some(42), &[41]),
            Some(42)
        );
        assert_eq!(managed_session_process_id(Some(false), Some(42), &[]), None);
    }

    #[test]
    fn launched_one_shot_requires_verified_cleanup() {
        assert!(cleanup_unresolved_after_launch(
            true,
            &json!({"requested": true, "attempted": false, "closed": null})
        ));
        assert!(cleanup_unresolved_after_launch(
            true,
            &json!({"requested": true, "attempted": true, "closed": false})
        ));
        assert!(!cleanup_unresolved_after_launch(
            true,
            &json!({"requested": true, "attempted": true, "closed": true})
        ));
        assert!(!cleanup_unresolved_after_launch(
            true,
            &json!({"requested": false, "attempted": false, "closed": null})
        ));
        assert!(!cleanup_unresolved_after_launch(
            false,
            &json!({"requested": true, "attempted": false, "closed": null})
        ));
    }

    #[test]
    fn unsupported_platform_is_rejected_before_oracle_evaluation() {
        assert!(ensure_desktop_platform("windows").is_ok());
        let error = ensure_desktop_platform("linux").expect_err("Linux is unsupported");
        assert_eq!(error.code, "unsupported_feature");
        assert_eq!(error.exit_code, 2);
        assert_eq!(
            error.message,
            "desktop oracle commands are unsupported on linux; Power BI Desktop automation requires Windows"
        );
    }

    #[test]
    fn desktop_detection_defers_version_probe() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable = temp.path().join("PBIDesktop.exe");
        fs::write(&executable, b"not executed").expect("fake executable");

        let detection = detect_power_bi_desktop(Some(&executable));
        assert!(detection.found);
        assert_eq!(detection.source, "override");
        assert_eq!(detection.version, None);
    }

    #[test]
    fn allow_unverified_capture_is_screenshot_only() {
        let options = parse_desktop_args(
            DesktopOperation::Screenshot,
            &[
                "report.pbip".to_string(),
                "--out".to_string(),
                "proof.png".to_string(),
                "--allow-unverified-capture".to_string(),
            ],
        )
        .expect("screenshot options");
        assert!(options.allow_unverified_capture);

        let error = parse_desktop_args(
            DesktopOperation::OpenCheck,
            &[
                "report.pbip".to_string(),
                "--allow-unverified-capture".to_string(),
            ],
        )
        .expect_err("open-check must reject capture override");
        assert_eq!(error.code, "invalid_args");
        assert!(error.message.contains("unknown desktop open-check flag"));
    }

    #[test]
    fn leave_open_is_rejected_in_favor_of_managed_sessions() {
        for operation in [
            DesktopOperation::Open,
            DesktopOperation::OpenCheck,
            DesktopOperation::Screenshot,
        ] {
            let error = parse_desktop_args(
                operation,
                &["report.pbip".to_string(), "--leave-open".to_string()],
            )
            .expect_err("unbounded Desktop ownership must be rejected");
            assert_eq!(error.code, "invalid_args");
            assert_eq!(
                error.message,
                "--leave-open has no bounded ownership lifetime"
            );
            assert_eq!(
                error.suggested_commands,
                [
                    "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> --json",
                    "powerbi-cli desktop close --json"
                ]
            );
        }
    }

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
    fn generated_powershell_scripts_are_fully_substituted_and_safely_quoted() {
        let adversarial = r"C:\Power BI\März $facts`tick O'Brien\Sales.pbip";
        let creation_time = "2026-07-22T10:15:31.1234567Z";
        let quoted_path = powershell_single_quoted(adversarial);
        let quoted_creation_time = powershell_single_quoted(creation_time);
        assert_eq!(
            quoted_path,
            r"'C:\Power BI\März $facts`tick O''Brien\Sales.pbip'"
        );

        let snapshot = render_process_snapshot_script();
        let windows = render_window_query_script();
        let launch = render_launch_script(adversarial);
        let screenshot = render_screenshot_script(adversarial, Some(4242), 4000, false);
        let association = process_identity(4242, creation_time);
        let observed = process_identity(5252, creation_time);
        let cleanup = render_cleanup_script(
            Some(&association),
            Some(&observed),
            &[7, 11, 4242],
            1_725_000_000_123,
        );
        let identity = render_process_identity_script(5252);
        let version = render_version_script(adversarial);

        for (name, script) in [
            ("snapshot", snapshot.as_str()),
            ("window query", windows.as_str()),
            ("launch", launch.as_str()),
            ("screenshot", screenshot.as_str()),
            ("cleanup", cleanup.as_str()),
            ("identity", identity.as_str()),
            ("version", version.as_str()),
        ] {
            assert!(
                !script.contains("__"),
                "{name} left a placeholder: {script}"
            );
            assert!(
                !has_powershell_variable_colon_trap(script),
                "{name} contains a $identifier: parsing trap: {script}"
            );
        }

        for script in [&launch, &screenshot, &version] {
            assert!(script.contains(&quoted_path));
        }
        assert!(cleanup.contains(&quoted_creation_time));
        assert!(cleanup.contains("$observedPid = 5252"));
        assert!(cleanup.contains("$baseline = @(7,11,4242)"));
        assert!(identity.contains("ProcessId = 5252"));
        assert!(screenshot.contains("$allowUnverifiedCapture = $false"));
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
    fn generated_cleanup_script_guards_every_kill_with_owned_creation_and_baseline_checks() {
        let association = process_identity(501, "2026-07-22T10:15:31.1234567Z");
        let observed = process_identity(777, "2026-07-22T10:15:32.1234567Z");
        let script = render_cleanup_script(
            Some(&association),
            Some(&observed),
            &[100, 501],
            1_725_000_000_123,
        );
        assert_eq!(script.matches("Stop-Process").count(), 1);
        let baseline_guard = script
            .find("if ($baseline -contains [int]$targetId)")
            .expect("per-kill baseline guard");
        let creation_guard = script
            .find("$currentCreatedAtUtc -le $launchTimeUtc")
            .expect("per-kill creation-time guard");
        let stop = script.find("Stop-Process").expect("bounded kill");
        assert!(baseline_guard < stop);
        assert!(creation_guard < stop);
        assert!(script.contains("if ($baseline -contains $ProcessId)"));
        assert!(script.contains("if ($createdAtUtc -le $launchTimeUtc)"));
        assert!(script.contains("'association-launch-pid'"));
        assert!(script.contains("'exact-observed-pid'"));
        assert!(script.contains("'creation-time-no-longer-matches-recorded-identity'"));
        assert!(script.contains("$associationCreationTimeUtc"));
        assert!(script.contains("$observedCreationTimeUtc"));
        assert!(script.contains("-RequireDesktop $true"));
        assert!(script.contains("[string]$row.Name -notlike 'PBIDesktop*'"));
        assert!(!script.contains("'exact-project-title-match'"));
        assert!(!script.contains("'executable-path-and-created-after-launch'"));
        assert!(script.contains("descendant-of-$parentId"));
        assert!(script.contains("targeted = @($targeted)"));
        assert!(!script.contains("$targetIds.Add"));
    }

    fn has_powershell_variable_colon_trap(script: &str) -> bool {
        let bytes = script.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'$' {
                index += 1;
                continue;
            }
            let mut end = index + 1;
            if end >= bytes.len() || !(bytes[end].is_ascii_alphabetic() || bytes[end] == b'_') {
                index += 1;
                continue;
            }
            end += 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b':' {
                return true;
            }
            index = end;
        }
        false
    }

    #[test]
    fn timeout_budget_saturates_at_zero() {
        let budget = Duration::from_millis(1_000);
        assert_eq!(
            remaining_budget(budget, Duration::from_millis(250)),
            Duration::from_millis(750)
        );
        assert_eq!(
            remaining_budget(budget, Duration::from_millis(1_000)),
            Duration::ZERO
        );
        assert_eq!(
            remaining_budget(budget, Duration::from_millis(1_500)),
            Duration::ZERO
        );
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
