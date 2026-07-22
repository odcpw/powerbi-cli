#[cfg(windows)]
use crate::contract::CONTRACT_VERSION;
#[cfg(not(windows))]
use crate::desktop::ensure_desktop_platform;
#[cfg(windows)]
use crate::desktop::{
    CLEANUP_TIMEOUT_MS, ProcessIdentity, Timed, cleanup_spawned_processes, ensure_desktop_platform,
    read_process_identity,
};
#[cfg(windows)]
use crate::project_io::write_json_atomic;
use crate::{CliError, CliResult};
#[cfg(windows)]
use crate::{EXIT_ORACLE_FAILED, EXIT_SUCCESS, canonical_display};
#[cfg(windows)]
use serde::Deserialize;
use serde_json::Value;
#[cfg(windows)]
use serde_json::json;
#[cfg(windows)]
use std::fs;
#[cfg(windows)]
use std::fs::OpenOptions;
#[cfg(windows)]
use std::io::{self, Write};
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
pub(crate) struct DesktopSessionLock {
    path: PathBuf,
    nonce: String,
}

#[cfg(windows)]
impl DesktopSessionLock {
    pub(crate) fn acquire() -> CliResult<Self> {
        let receipt_path = desktop_session_path()?;
        let root = receipt_path.parent().ok_or_else(|| {
            CliError::unexpected("Desktop session receipt has no parent directory")
        })?;
        fs::create_dir_all(root).map_err(|error| {
            CliError::unexpected(format!(
                "create Desktop session state directory {}: {error}",
                root.display()
            ))
        })?;
        let path = root.join("desktop-session.lock");
        let nonce = format!(
            "{}:{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                CliError::validation_failed(format!(
                    "another managed Desktop session operation is active: {error}"
                ))
                .with_hint(format!(
                    "Run Desktop lifecycle commands serially. If a prior CLI process crashed, remove only the stale lock at {}.",
                    path.display()
                ))
                .with_suggested_command("powerbi-cli desktop close --json")
            })?;
        file.write_all(nonce.as_bytes()).map_err(|error| {
            let _ = fs::remove_file(&path);
            CliError::unexpected(format!("write Desktop session operation lock: {error}"))
        })?;
        Ok(Self { path, nonce })
    }
}

#[cfg(windows)]
impl Drop for DesktopSessionLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).is_ok_and(|value| value == self.nonce) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct DesktopSessionDraft {
    pub(crate) document_kind: String,
    pub(crate) document_name: String,
    pub(crate) document_path: String,
    pub(crate) desktop_path: String,
    pub(crate) association_process_id: u32,
    pub(crate) observed_identity: ProcessIdentity,
    pub(crate) baseline_process_ids: Vec<u32>,
    pub(crate) launch_timestamp_unix_ms: u64,
    pub(crate) opened_at_unix_ms: u64,
}

#[cfg(windows)]
#[derive(Debug)]
pub(crate) struct ManagedDesktopSession {
    pub(crate) receipt_path: PathBuf,
    pub(crate) identity: ProcessIdentity,
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopSessionReceipt {
    schema: String,
    document_kind: String,
    document_name: String,
    document_path: String,
    desktop_path: String,
    association_process_id: u32,
    observed_process_id: u32,
    observed_process_creation_time_utc: String,
    observed_executable_path: Option<String>,
    baseline_process_ids: Vec<u32>,
    launch_timestamp_unix_ms: u64,
    opened_at_unix_ms: u64,
}

#[cfg(windows)]
pub(crate) fn open_desktop_session(
    _lock: &DesktopSessionLock,
    draft: DesktopSessionDraft,
) -> CliResult<ManagedDesktopSession> {
    let identity = draft.observed_identity;
    let receipt_path = desktop_session_path()?;
    let parent = receipt_path
        .parent()
        .ok_or_else(|| CliError::unexpected("Desktop session receipt has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| {
        CliError::unexpected(format!(
            "create Desktop session state directory {}: {error}",
            parent.display()
        ))
    })?;
    write_json_atomic(
        &receipt_path,
        &json!({
            "schema": "powerbi-cli.desktop.session.v2",
            "documentKind": draft.document_kind,
            "documentName": draft.document_name,
            "documentPath": draft.document_path,
            "desktopPath": draft.desktop_path,
            "associationProcessId": draft.association_process_id,
            "observedProcessId": identity.process_id,
            "observedProcessCreationTimeUtc": identity.creation_time_utc,
            "observedExecutablePath": identity.executable_path,
            "baselineProcessIds": draft.baseline_process_ids,
            "launchTimestampUnixMs": draft.launch_timestamp_unix_ms,
            "openedAtUnixMs": draft.opened_at_unix_ms
        }),
    )?;
    Ok(ManagedDesktopSession {
        receipt_path,
        identity,
    })
}

#[cfg(windows)]
pub(crate) fn close_desktop_session_command(args: &[String]) -> CliResult<Value> {
    reject_close_arguments(args)?;
    ensure_desktop_platform(std::env::consts::OS)?;
    let lock = DesktopSessionLock::acquire()?;
    close_desktop_session(&lock)
}

#[cfg(not(windows))]
pub(crate) fn close_desktop_session_command(args: &[String]) -> CliResult<Value> {
    reject_close_arguments(args)?;
    ensure_desktop_platform(std::env::consts::OS)?;
    Err(CliError::unexpected(
        "Desktop session platform dispatch failed",
    ))
}

fn reject_close_arguments(args: &[String]) -> CliResult<()> {
    if let Some(argument) = args.first() {
        return Err(CliError::invalid_args(format!(
            "desktop close accepts no arguments: {argument}"
        ))
        .with_hint("The CLI closes only its single recorded Desktop session.")
        .with_suggested_command("powerbi-cli desktop close --json"));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn close_desktop_session(_lock: &DesktopSessionLock) -> CliResult<Value> {
    let receipt_path = desktop_session_path()?;
    if !receipt_path.exists() {
        return Ok(no_session_response(&receipt_path));
    }
    let receipt = read_receipt(&receipt_path)?;
    let current_identity = read_process_identity(receipt.observed_process_id)?;
    let identity_matches = current_identity.as_ref().is_some_and(|identity| {
        identity.creation_time_utc == receipt.observed_process_creation_time_utc
    });
    if !identity_matches {
        remove_session_receipt(&receipt_path)?;
        return Ok(already_closed_response(&receipt_path, &receipt));
    }
    let current_identity = current_identity.expect("matching Desktop identity exists");

    let mut cleanup = match cleanup_spawned_processes(
        Some(&current_identity),
        Some(&current_identity),
        &receipt.baseline_process_ids,
        receipt.launch_timestamp_unix_ms,
    ) {
        Ok(Timed::Completed(cleanup)) => cleanup,
        Ok(Timed::TimedOut) => cleanup_failure(
            receipt.observed_process_id,
            format!("Desktop session cleanup exceeded {CLEANUP_TIMEOUT_MS} ms"),
        ),
        Err(error) => cleanup_failure(receipt.observed_process_id, error.to_string()),
    };
    if let Some(object) = cleanup.as_object_mut() {
        object.insert("attempted".to_string(), Value::Bool(true));
        object.insert("identityMatched".to_string(), Value::Bool(true));
    }
    let closed = cleanup["closed"].as_bool() == Some(true);
    if closed {
        remove_session_receipt(&receipt_path)?;
    }
    Ok(json!({
        "schema": "powerbi-cli.desktop.close.v1",
        "tool": "powerbi-cli",
        "contractVersion": CONTRACT_VERSION,
        "ok": closed,
        "exitCode": if closed { EXIT_SUCCESS } else { EXIT_ORACLE_FAILED },
        "session": {
            "state": if closed { "closed" } else { "open" },
            "alreadyClosed": false,
            "document": receipt.document_path,
            "documentKind": receipt.document_kind,
            "documentName": receipt.document_name,
            "desktopPath": receipt.desktop_path,
            "associationProcessId": receipt.association_process_id,
            "desktopProcessId": receipt.observed_process_id,
            "desktopProcessCreationTimeUtc": receipt.observed_process_creation_time_utc,
            "desktopExecutablePath": receipt.observed_executable_path,
            "openedAtUnixMs": receipt.opened_at_unix_ms,
            "receiptPath": canonical_display(&receipt_path),
            "receiptRemoved": closed
        },
        "cleanup": cleanup,
        "next": if closed {
            Vec::<String>::new()
        } else {
            vec!["powerbi-cli desktop close --json".to_string()]
        }
    }))
}

#[cfg(windows)]
fn desktop_session_path() -> CliResult<PathBuf> {
    if let Some(root) = std::env::var_os("POWERBI_CLI_STATE_DIR") {
        return Ok(PathBuf::from(root).join("desktop-session.json"));
    }
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        CliError::unexpected("LOCALAPPDATA is unavailable for the Desktop session receipt")
            .with_hint("Set POWERBI_CLI_STATE_DIR to a private writable directory and retry.")
    })?;
    Ok(PathBuf::from(local_app_data)
        .join("powerbi-cli")
        .join("desktop-session.json"))
}

#[cfg(windows)]
fn read_receipt(path: &Path) -> CliResult<DesktopSessionReceipt> {
    let text = fs::read_to_string(path).map_err(|error| {
        CliError::unexpected(format!(
            "read Desktop session receipt {}: {error}",
            path.display()
        ))
    })?;
    let receipt: DesktopSessionReceipt = serde_json::from_str(&text).map_err(|error| {
        CliError::unexpected(format!(
            "parse Desktop session receipt {}: {error}",
            path.display()
        ))
        .with_hint("Do not kill processes by title. Inspect or remove only this invalid receipt.")
    })?;
    if receipt.schema != "powerbi-cli.desktop.session.v2" {
        return Err(CliError::unexpected(format!(
            "unsupported Desktop session receipt schema: {}",
            receipt.schema
        ))
        .with_hint("Do not kill processes by title. Inspect or remove only this stale receipt."));
    }
    Ok(receipt)
}

#[cfg(windows)]
fn remove_session_receipt(path: &Path) -> CliResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::unexpected(format!(
            "remove Desktop session receipt {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(windows)]
fn no_session_response(receipt_path: &Path) -> Value {
    json!({
        "schema": "powerbi-cli.desktop.close.v1",
        "tool": "powerbi-cli",
        "contractVersion": CONTRACT_VERSION,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "session": {
            "state": "none",
            "alreadyClosed": true,
            "receiptPath": canonical_display(receipt_path)
        },
        "cleanup": {
            "attempted": false,
            "identityMatched": Value::Null,
            "closed": true,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "errors": []
        },
        "next": []
    })
}

#[cfg(windows)]
fn already_closed_response(path: &Path, receipt: &DesktopSessionReceipt) -> Value {
    json!({
        "schema": "powerbi-cli.desktop.close.v1",
        "tool": "powerbi-cli",
        "contractVersion": CONTRACT_VERSION,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "session": {
            "state": "none",
            "alreadyClosed": true,
            "document": receipt.document_path,
            "documentKind": receipt.document_kind,
            "desktopProcessId": receipt.observed_process_id,
            "openedAtUnixMs": receipt.opened_at_unix_ms,
            "receiptPath": canonical_display(path),
            "receiptRemoved": true
        },
        "cleanup": {
            "attempted": false,
            "closed": true,
            "identityMatched": false,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "errors": []
        },
        "next": []
    })
}

#[cfg(windows)]
fn cleanup_failure(process_id: u32, message: String) -> Value {
    json!({
        "targeted": [],
        "targetedProcessIds": [],
        "remainingProcessIds": [process_id],
        "attempted": true,
        "identityMatched": true,
        "closed": false,
        "skipped": [],
        "errors": [message]
    })
}
