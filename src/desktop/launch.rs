//! Desktop command dispatch, preflight validation, and bounded launch orchestration.

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
use crate::feature_catalog::unsupported_feature_error;
#[cfg(windows)]
use crate::lint::lint_project;
use crate::{CliError, CliResult, canonical_display};
#[cfg(windows)]
use crate::{
    EXIT_ORACLE_FAILED, EXIT_ORACLE_UNAVAILABLE, EXIT_PROOF_INCOMPLETE, EXIT_SUCCESS,
    EXIT_VALIDATION_FAILED, ValidationReport, command_arg, validate_project,
};
use serde_json::Value;
#[cfg(windows)]
use serde_json::json;
#[cfg(windows)]
use std::io;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::{Command, Stdio};
#[cfg(any(windows, test))]
use std::time::Duration;
#[cfg(windows)]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
#[cfg(windows)]
const COMMAND_POLL_INTERVAL_MS: u64 = 25;
#[cfg(windows)]
const DESKTOP_COMMAND_PROOF_LEVEL: &str = "unit-smoke";

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
    enable_oracle: bool,
    preflight: PreflightMode,
    preflight_explicit: bool,
}

impl Default for DesktopOptions {
    fn default() -> Self {
        Self {
            project: None,
            desktop_path: None,
            out: None,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            allow_unverified_capture: false,
            enable_oracle: false,
            preflight: PreflightMode::Strict,
            preflight_explicit: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreflightMode {
    Strict,
    Normal,
    Skip,
}

impl PreflightMode {
    fn parse(value: &str) -> CliResult<Self> {
        match value {
            "strict" => Ok(Self::Strict),
            "normal" => Ok(Self::Normal),
            "skip" => Ok(Self::Skip),
            _ => Err(CliError::invalid_args(
                "--preflight must be one of: strict, normal, skip",
            )
            .with_hint("Use strict by default, normal to omit lint, or skip for an explicit no-preflight launch.")
            .with_suggested_command(
                "powerbi-cli desktop open <project-dir-or.pbip-or.pbix> --preflight normal --json",
            )),
        }
    }

    #[cfg(windows)]
    fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Normal => "normal",
            Self::Skip => "skip",
        }
    }
}

#[cfg(windows)]
use super::cleanup::{
    ProcessIdentity, cleanup_after_launch, cleanup_unresolved_after_launch, read_process_identity,
};

#[cfg(windows)]
#[derive(Debug, Clone)]
struct DesktopLaunchPlan {
    method: &'static str,
    detection_path_used_for_launch: bool,
    requested_desktop_path: Option<String>,
    file_association_reason: Option<&'static str>,
}

#[cfg(windows)]
use super::observe::{
    WINDOW_POLL_INTERVAL_MS, WindowObservation, managed_session_process_id, observe_window,
    snapshot_desktop_process_ids, unproven_signals,
};

#[cfg(windows)]
#[derive(Debug)]
pub(super) struct Watchdog {
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

    pub(super) fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub(super) fn elapsed_ms(&self) -> u64 {
        duration_ms(self.elapsed())
    }

    pub(super) fn remaining(&self) -> Duration {
        remaining_budget(self.budget, self.elapsed())
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub(crate) enum Timed<T> {
    Completed(T),
    TimedOut,
}

#[cfg(windows)]
use super::evidence::{
    SCREENSHOT_CAPTURE_TIMEOUT_MS, ScreenshotCaptureOutcome, ScreenshotDimensions,
    capture_primary_display, validate_screenshot_output,
};
#[cfg(windows)]
use super::evidence::{screenshot_changes, screenshot_observation_is_eligible};

pub(crate) fn desktop_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args(
                "desktop requires a subcommand: open, close, open-check, screenshot, refresh-check, canvas-check, or bridge",
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
        "refresh-check" | "refreshCheck" => {
            Err(unsupported_feature_error("desktop.refresh-check"))
        }
        "canvas-check" | "canvasCheck" => {
            Err(unsupported_feature_error("desktop.canvas-check"))
        }
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

    let preflight_applicable = target.project().is_some();
    let validation_performed = preflight_applicable && options.preflight != PreflightMode::Skip;
    let validation = match (target.project(), validation_performed) {
        (Some(project), true) => validate_project(project)?,
        _ => ValidationReport::default(),
    };
    let validation_ok = validation.errors.is_empty();
    let strict_preflight_enabled =
        target.kind == DesktopTargetKind::Pbip && options.preflight == PreflightMode::Strict;
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
    let strict_preflight_ok = !strict_preflight_enabled || (validation_ok && lint_error_count == 0);
    let preflight_ok = match options.preflight {
        PreflightMode::Strict => validation_ok && lint_error_count == 0,
        PreflightMode::Normal => validation_ok,
        PreflightMode::Skip => true,
    };
    let preflight = json!({
        "mode": options.preflight.as_str(),
        "defaulted": !options.preflight_explicit,
        "applicable": preflight_applicable,
        "performed": validation_performed,
        "validationPerformed": validation_performed,
        "lintPerformed": lint.is_some(),
        "skipped": preflight_applicable && options.preflight == PreflightMode::Skip,
        "ok": preflight_ok,
        "message": match (preflight_applicable, options.preflight) {
            (false, _) => "PBIX target resolution performs its bounded package checks separately; PBIP validation/lint preflight is not applicable.",
            (true, PreflightMode::Strict) => "Strict PBIP validation and lint preflight performed.",
            (true, PreflightMode::Normal) => "Normal PBIP validation performed without lint.",
            (true, PreflightMode::Skip) => "PBIP validation and lint preflight skipped by explicit request.",
        }
    });
    let mut detection = detect_power_bi_desktop(options.desktop_path.as_deref());
    let oracle_enabled = oracle_enabled(options.enable_oracle);
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
            "Desktop oracle launch is disabled; set POWERBI_DESKTOP_ORACLE=1 or pass --enable-oracle to opt in."
                .to_string();
        diagnostics.push(json!({
            "code": "oracle_disabled",
            "severity": "error",
            "message": format!(
                "Set POWERBI_DESKTOP_ORACLE=1 or pass --enable-oracle on a Windows machine with Power BI Desktop installed to run {}.",
                operation.command_path()
            ),
            "hint": "Set POWERBI_DESKTOP_ORACLE=1 or pass --enable-oracle"
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
        "preflight": preflight,
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
            "performed": validation_performed,
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
            "--enable-oracle" | "--enableOracle" => {
                options.enable_oracle = true;
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
            "--preflight" if operation == DesktopOperation::Open => {
                if options.preflight_explicit {
                    return Err(CliError::invalid_args(
                        "desktop open accepts --preflight only once",
                    )
                    .with_suggested_command(operation.suggested_command()));
                }
                let value = take_value(args, &mut i, "--preflight")?;
                options.preflight = PreflightMode::parse(&value)?;
                options.preflight_explicit = true;
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
pub(super) fn render_launch_script(pbip_path: &str) -> String {
    LAUNCH_SCRIPT.replace("__PBIP_PATH__", &powershell_single_quoted(pbip_path))
}

#[cfg(windows)]
pub(super) fn run_powershell(
    script: &str,
    timeout: Duration,
) -> io::Result<Timed<std::process::Output>> {
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
pub(super) fn ensure_powershell_success(
    output: &std::process::Output,
    action: &str,
) -> io::Result<()> {
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
pub(super) fn parse_powershell_json<T>(bytes: &[u8]) -> io::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim().trim_start_matches('\u{feff}');
    serde_json::from_str(text)
        .map_err(|err| io::Error::other(format!("parse PowerShell JSON output: {err}: {text}")))
}

#[cfg(any(windows, test))]
pub(super) fn powershell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(any(windows, test))]
pub(super) fn desktop_argument_path(path: &Path) -> String {
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
pub(super) fn render_version_script(desktop_path: &str) -> String {
    VERSION_SCRIPT.replace("__DESKTOP_PATH__", &powershell_single_quoted(desktop_path))
}

#[cfg(windows)]
fn oracle_enabled(flag: bool) -> bool {
    flag || env_oracle_enabled()
}

#[cfg(windows)]
fn env_oracle_enabled() -> bool {
    std::env::var("POWERBI_DESKTOP_ORACLE")
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
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
        DesktopOperation, PreflightMode, desktop_argument_path, detect_power_bi_desktop,
        ensure_desktop_platform, parse_desktop_args, remaining_budget,
    };
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

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

        let enabled = parse_desktop_args(
            DesktopOperation::Open,
            &["report.pbip".to_string(), "--enable-oracle".to_string()],
        )
        .expect("open options");
        assert!(enabled.enable_oracle);

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
    fn desktop_open_preflight_defaults_to_strict_and_accepts_closed_modes() {
        let default = parse_desktop_args(DesktopOperation::Open, &["report.pbip".to_string()])
            .expect("default open options");
        assert_eq!(default.preflight, PreflightMode::Strict);
        assert!(!default.preflight_explicit);

        for (value, expected) in [
            ("strict", PreflightMode::Strict),
            ("normal", PreflightMode::Normal),
            ("skip", PreflightMode::Skip),
        ] {
            let options = parse_desktop_args(
                DesktopOperation::Open,
                &[
                    "report.pbip".to_string(),
                    "--preflight".to_string(),
                    value.to_string(),
                ],
            )
            .expect("preflight mode");
            assert_eq!(options.preflight, expected);
            assert!(options.preflight_explicit);
        }

        let invalid = parse_desktop_args(
            DesktopOperation::Open,
            &[
                "report.pbip".to_string(),
                "--preflight".to_string(),
                "fast".to_string(),
            ],
        )
        .expect_err("invalid preflight mode");
        assert_eq!(invalid.code, "invalid_args");

        let other_operation = parse_desktop_args(
            DesktopOperation::OpenCheck,
            &[
                "report.pbip".to_string(),
                "--preflight".to_string(),
                "skip".to_string(),
            ],
        )
        .expect_err("preflight flag is desktop-open only");
        assert!(
            other_operation
                .message
                .contains("unknown desktop open-check flag")
        );
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
}
