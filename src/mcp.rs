use crate::child_process::{spawn_contained, terminate_after_exit, terminate_and_wait};
use crate::microsoft::{
    InstalledMicrosoftTool, MicrosoftComponent, ModelingMcpContract, minimal_child_command,
};
use crate::project_io::write_text_atomic;
use crate::safety_scan::contains_credential_like_text_str;
use crate::tmdl::{
    MutationPlan, PartitionSelector, find_partition, load_table_documents_from_semantic_model,
    replace_partition_source_plan,
};
use crate::workflow::{
    ExportShapeProof, PreparedStagedModel, SourceTreeEvidence, SourceTreeSnapshot,
};
use crate::{CliError, CliResult, EXIT_ORACLE_FAILED};
use command_group::GroupChild;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, Signal, System};

mod cleanup;
mod staged;

pub(crate) use cleanup::McpCleanupReport;
use cleanup::{
    ChildGuard, MonitorCommand, MonitorReport, PumpJoinFailure, ReaderEvent, StreamCapture,
    WriterCommand, join_monitor, join_pump, join_stderr, monitor_pump, reader_pump, stderr_pump,
    terminate_child_tree, writer_pump,
};
#[cfg(test)]
use cleanup::{capture_stderr, read_frames};
pub(crate) use staged::*;

const DEFAULT_FRAME_LIMIT: usize = 512 * 1024;
const DEFAULT_TOTAL_RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
const DEFAULT_STDERR_LIMIT: usize = 32 * 1024;
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const DEFAULT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const CHANNEL_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpSessionMode {
    ReadOnly,
    ConfirmedWrite,
}

impl McpSessionMode {
    fn is_read_only(self) -> bool {
        self == Self::ReadOnly
    }
}

#[derive(Debug, Clone)]
pub(crate) struct McpSessionConfig {
    pub(crate) frame_limit: usize,
    pub(crate) total_response_limit: usize,
    pub(crate) stderr_limit: usize,
    pub(crate) call_timeout: Duration,
    pub(crate) session_timeout: Duration,
    pub(crate) cleanup_timeout: Duration,
}

impl Default for McpSessionConfig {
    fn default() -> Self {
        Self {
            frame_limit: DEFAULT_FRAME_LIMIT,
            total_response_limit: DEFAULT_TOTAL_RESPONSE_LIMIT,
            stderr_limit: DEFAULT_STDERR_LIMIT,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            session_timeout: DEFAULT_SESSION_TIMEOUT,
            cleanup_timeout: DEFAULT_CLEANUP_TIMEOUT,
        }
    }
}

impl McpSessionConfig {
    fn validate(&self) -> Result<(), McpFailure> {
        if self.frame_limit == 0
            || self.total_response_limit < self.frame_limit
            || self.stderr_limit == 0
            || self.call_timeout.is_zero()
            || self.session_timeout.is_zero()
            || self.cleanup_timeout.is_zero()
        {
            return Err(McpFailure::protocol(
                "invalid bounded MCP session configuration",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpFailureKind {
    Protocol,
    Backend,
    Cancelled,
    Panicked,
}

#[derive(Debug, Clone)]
pub(crate) struct McpFailure {
    kind: McpFailureKind,
    message: String,
    stderr_tail: Option<String>,
    stderr_sha256: Option<String>,
    children_reaped: Option<bool>,
}

impl McpFailure {
    fn protocol(message: impl Into<String>) -> Self {
        Self::new(McpFailureKind::Protocol, message)
    }

    fn backend(message: impl Into<String>) -> Self {
        Self::new(McpFailureKind::Backend, message)
    }

    fn cancelled(message: impl Into<String>) -> Self {
        Self::new(McpFailureKind::Cancelled, message)
    }

    fn panicked(message: impl Into<String>) -> Self {
        Self::new(McpFailureKind::Panicked, message)
    }

    fn new(kind: McpFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            stderr_tail: None,
            stderr_sha256: None,
            children_reaped: None,
        }
    }

    pub(crate) fn kind(&self) -> McpFailureKind {
        self.kind
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    fn with_cleanup(mut self, cleanup: &McpCleanupReport) -> Self {
        self.stderr_tail = Some(cleanup.stderr.tail.clone());
        self.stderr_sha256 = Some(cleanup.stderr.sha256.clone());
        self.children_reaped = Some(cleanup.children_reaped);
        self
    }

    fn into_cli_error(self) -> CliError {
        let code = if self.kind == McpFailureKind::Protocol {
            "protocol_failed"
        } else {
            "backend_failed"
        };
        let mut detail = self.message;
        if let Some(hash) = self.stderr_sha256 {
            detail.push_str(&format!("; vendorStderrSha256={hash}"));
        }
        if let Some(reaped) = self.children_reaped {
            detail.push_str(&format!("; childrenReaped={reaped}"));
        }
        if let Some(tail) = self.stderr_tail.filter(|value| !value.is_empty()) {
            detail.push_str(&format!("; vendorStderr={tail}"));
        }
        CliError::new(code, EXIT_ORACLE_FAILED, detail)
            .with_hint(
                "Run `powerbi-cli integrations status --deep --component modeling-mcp --json` and reinstall the exact integration if the pinned protocol surface drifted.",
            )
            .with_suggested_command(
                "powerbi-cli integrations status --deep --component modeling-mcp --json",
            )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct McpHandshake {
    pub(crate) protocol_version: String,
    pub(crate) server_name: String,
    pub(crate) server_version: String,
    pub(crate) tools_count: usize,
    pub(crate) tools_list_sha256: String,
    pub(crate) notifications_seen: usize,
}

pub(crate) struct McpSession {
    mode: McpSessionMode,
    expected: ModelingMcpContract,
    config: McpSessionConfig,
    started: Instant,
    writer_tx: SyncSender<WriterCommand>,
    reader_rx: Receiver<ReaderEvent>,
    monitor_tx: SyncSender<MonitorCommand>,
    writer: Option<JoinHandle<Result<(), String>>>,
    reader: Option<JoinHandle<Result<(), String>>>,
    stderr: Option<JoinHandle<Result<StreamCapture, String>>>,
    monitor: Option<JoinHandle<MonitorReport>>,
    next_id: u64,
    pending_id: Option<u64>,
    notifications_seen: usize,
    initialized: bool,
    poisoned: bool,
    cleanup: Option<McpCleanupReport>,
}

impl McpSession {
    pub(crate) fn open_exact(
        tool: &InstalledMicrosoftTool,
        mode: McpSessionMode,
        config: McpSessionConfig,
    ) -> Result<Self, McpFailure> {
        if tool.component != MicrosoftComponent::ModelingMcp {
            return Err(McpFailure::protocol(
                "the MCP session requires the exact modeling-mcp component",
            ));
        }
        if tool.transport != "stdio" {
            return Err(McpFailure::protocol(format!(
                "unsupported Modeling MCP transport: {}",
                tool.transport
            )));
        }
        let expected = tool.mcp_contract.clone().ok_or_else(|| {
            McpFailure::protocol("the installed Modeling MCP has no pinned handshake contract")
        })?;
        let path_entry = tool
            .entrypoint
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut command = minimal_child_command(&tool.entrypoint, &[path_entry]);
        command.arg("--start").arg("--compatibility=powerbi");
        match mode {
            McpSessionMode::ReadOnly => {
                command.arg("--read-only");
            }
            McpSessionMode::ConfirmedWrite => {
                // The high-level staged API checks explicit model-write consent before this
                // process exists. A second server elicitation would be redundant and is not
                // accepted by the closed JSON-RPC policy.
                command.arg("--read-write");
            }
        }
        Self::open_command(command, expected, mode, config)
    }

    fn open_command(
        mut command: Command,
        expected: ModelingMcpContract,
        mode: McpSessionMode,
        config: McpSessionConfig,
    ) -> Result<Self, McpFailure> {
        config.validate()?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = spawn_contained(&mut command)
            .map_err(|error| McpFailure::backend(format!("start Modeling MCP: {error}")))?;
        let stdin = child.inner().stdin.take().ok_or_else(|| {
            let _ = terminate_child_tree(&mut child);
            McpFailure::backend("Modeling MCP stdin was not piped")
        })?;
        let stdout = child.inner().stdout.take().ok_or_else(|| {
            let _ = terminate_child_tree(&mut child);
            McpFailure::backend("Modeling MCP stdout was not piped")
        })?;
        let stderr = child.inner().stderr.take().ok_or_else(|| {
            let _ = terminate_child_tree(&mut child);
            McpFailure::backend("Modeling MCP stderr was not piped")
        })?;

        let (writer_tx, writer_rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (reader_tx, reader_rx) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (monitor_tx, monitor_rx) = mpsc::sync_channel(2);
        let frame_limit = config.frame_limit;
        let total_limit = config.total_response_limit;
        let stderr_limit = config.stderr_limit;
        let cleanup_timeout = config.cleanup_timeout;
        let session_timeout = config.session_timeout;
        let lifecycle_timeout = session_timeout
            .checked_add(config.cleanup_timeout)
            .unwrap_or(session_timeout);

        let writer = thread::Builder::new()
            .name("mcp-writer".to_string())
            .spawn(move || writer_pump(stdin, writer_rx))
            .map_err(|error| {
                let _ = terminate_child_tree(&mut child);
                McpFailure::backend(format!("start MCP writer pump: {error}"))
            })?;
        let reader = match thread::Builder::new()
            .name("mcp-reader".to_string())
            .spawn(move || reader_pump(stdout, reader_tx, frame_limit, total_limit))
        {
            Ok(reader) => reader,
            Err(error) => {
                let _ = terminate_child_tree(&mut child);
                let _ = writer_tx.try_send(WriterCommand::Close);
                let _ = writer.join();
                return Err(McpFailure::backend(format!(
                    "start MCP reader pump: {error}"
                )));
            }
        };
        let stderr = match thread::Builder::new()
            .name("mcp-stderr".to_string())
            .spawn(move || stderr_pump(stderr, stderr_limit))
        {
            Ok(stderr) => stderr,
            Err(error) => {
                let _ = terminate_child_tree(&mut child);
                let _ = writer_tx.try_send(WriterCommand::Close);
                let _ = writer.join();
                let _ = reader.join();
                return Err(McpFailure::backend(format!(
                    "start MCP stderr pump: {error}"
                )));
            }
        };
        let owned_child = ChildGuard::new(child);
        let monitor = match thread::Builder::new()
            .name("mcp-monitor".to_string())
            .spawn(move || {
                monitor_pump(owned_child, monitor_rx, lifecycle_timeout, cleanup_timeout)
            }) {
            Ok(monitor) => monitor,
            Err(error) => {
                let _ = writer_tx.try_send(WriterCommand::Close);
                let _ = writer.join();
                let _ = reader.join();
                let _ = stderr.join();
                return Err(McpFailure::backend(format!(
                    "start MCP process monitor: {error}"
                )));
            }
        };
        Ok(Self {
            mode,
            expected,
            config,
            started: Instant::now(),
            writer_tx,
            reader_rx,
            monitor_tx,
            writer: Some(writer),
            reader: Some(reader),
            stderr: Some(stderr),
            monitor: Some(monitor),
            next_id: 1,
            pending_id: None,
            notifications_seen: 0,
            initialized: false,
            poisoned: false,
            cleanup: None,
        })
    }

    pub(crate) fn handshake(&mut self) -> Result<McpHandshake, McpFailure> {
        if self.initialized {
            return Err(McpFailure::protocol(
                "the Modeling MCP session is already initialized",
            ));
        }
        let initialize = self.request(
            "initialize",
            json!({
                "protocolVersion": self.expected.protocol_version,
                "capabilities": {},
                "clientInfo": {
                    "name": "powerbi-cli",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )?;
        let protocol_version = required_string(&initialize, "protocolVersion")?;
        let server = initialize
            .get("serverInfo")
            .and_then(Value::as_object)
            .ok_or_else(|| McpFailure::protocol("initialize result has no serverInfo object"))?;
        let server_name = required_object_string(server, "name", "serverInfo")?;
        let server_version = required_object_string(server, "version", "serverInfo")?;
        if protocol_version != self.expected.protocol_version
            || server_name != self.expected.server_name
            || server_version != self.expected.server_version
        {
            return Err(McpFailure::protocol(format!(
                "Modeling MCP identity drift: expected protocol {}/{}/{}, got {}/{}/{}",
                self.expected.protocol_version,
                self.expected.server_name,
                self.expected.server_version,
                protocol_version,
                server_name,
                server_version
            )));
        }
        self.notify("notifications/initialized", json!({}))?;
        let tools_result = self.request("tools/list", json!({}))?;
        let (tools_count, tools_list_sha256) = normalized_tools_identity(&tools_result)?;
        if tools_count != self.expected.tools_count
            || tools_list_sha256 != self.expected.tools_list_sha256
        {
            return Err(McpFailure::protocol(format!(
                "Modeling MCP tool surface drift: expected {} tools/{}, got {tools_count}/{tools_list_sha256}",
                self.expected.tools_count, self.expected.tools_list_sha256
            )));
        }
        self.initialized = true;
        Ok(McpHandshake {
            protocol_version,
            server_name,
            server_version,
            tools_count,
            tools_list_sha256,
            notifications_seen: self.notifications_seen,
        })
    }

    pub(crate) fn call(&mut self, operation: &McpOperation) -> Result<Value, McpFailure> {
        if !self.initialized {
            return Err(McpFailure::protocol(
                "Modeling MCP tools cannot be called before a verified handshake",
            ));
        }
        let tool_name = operation.tool_name();
        let arguments = operation.arguments()?;
        ClosedToolPolicy::authorize(tool_name, &arguments, self.mode)?;
        let result = self.request(
            "tools/call",
            json!({"name": tool_name, "arguments": arguments}),
        )?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(McpFailure::backend(format!(
                "Modeling MCP tool {tool_name} returned an error result"
            )));
        }
        Ok(result)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, McpFailure> {
        if self.poisoned {
            return Err(McpFailure::cancelled(
                "the Modeling MCP session was cancelled and cannot accept more calls",
            ));
        }
        if self.pending_id.is_some() {
            return Err(McpFailure::protocol(
                "MCP request serialization invariant violated: another request is pending",
            ));
        }
        let call_timeout = self.remaining_call_timeout()?;
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| McpFailure::protocol("MCP request identifier space was exhausted"))?;
        self.pending_id = Some(id);
        let send_result = self.send_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }));
        if let Err(error) = send_result {
            self.pending_id = None;
            return Err(error);
        }
        let deadline = Instant::now() + call_timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.cancel_request(id, "powerbi-cli MCP call deadline exceeded");
                self.pending_id = None;
                self.poisoned = true;
                return Err(McpFailure::cancelled(format!(
                    "Modeling MCP call {id} exceeded {} ms",
                    self.config.call_timeout.as_millis()
                )));
            }
            match self.reader_rx.recv_timeout(deadline - now) {
                Ok(ReaderEvent::Frame(frame)) => {
                    let message: Value = match serde_json::from_slice(&frame) {
                        Ok(message) => message,
                        Err(error) => {
                            self.pending_id = None;
                            self.poisoned = true;
                            return Err(McpFailure::protocol(format!(
                                "malformed MCP JSON frame: {error}"
                            )));
                        }
                    };
                    let incoming = match self.classify_message(id, &message) {
                        Ok(incoming) => incoming,
                        Err(error) => {
                            self.pending_id = None;
                            return Err(error);
                        }
                    };
                    match incoming {
                        Incoming::Response(result) => {
                            self.pending_id = None;
                            return Ok(result);
                        }
                        Incoming::Notification => {
                            self.notifications_seen = self.notifications_seen.saturating_add(1);
                        }
                    }
                }
                Ok(ReaderEvent::Failure(message)) => {
                    self.pending_id = None;
                    self.poisoned = true;
                    return Err(McpFailure::protocol(message));
                }
                Ok(ReaderEvent::Eof) => {
                    self.pending_id = None;
                    self.poisoned = true;
                    return Err(McpFailure::backend(
                        "Modeling MCP closed stdout while a request was pending",
                    ));
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    self.pending_id = None;
                    self.poisoned = true;
                    return Err(McpFailure::backend(
                        "Modeling MCP reader pump stopped while a request was pending",
                    ));
                }
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), McpFailure> {
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    fn send_json(&self, message: &Value) -> Result<(), McpFailure> {
        let mut frame = serde_json::to_vec(message)
            .map_err(|error| McpFailure::protocol(format!("serialize MCP request: {error}")))?;
        if frame.len() > self.config.frame_limit {
            return Err(McpFailure::protocol(format!(
                "outbound MCP frame exceeds {} bytes",
                self.config.frame_limit
            )));
        }
        frame.push(b'\n');
        self.writer_tx
            .send(WriterCommand::Frame(frame))
            .map_err(|_| McpFailure::backend("Modeling MCP writer pump is unavailable"))
    }

    fn remaining_call_timeout(&self) -> Result<Duration, McpFailure> {
        let elapsed = self.started.elapsed();
        if elapsed >= self.config.session_timeout {
            return Err(McpFailure::cancelled(
                "Modeling MCP session budget was exhausted",
            ));
        }
        Ok(self
            .config
            .call_timeout
            .min(self.config.session_timeout - elapsed))
    }

    fn cancel_request(&self, id: u64, reason: &str) {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "notifications/cancelled",
            "params": {"requestId": id, "reason": reason}
        });
        if let Ok(mut frame) = serde_json::to_vec(&message)
            && frame.len() <= self.config.frame_limit
        {
            frame.push(b'\n');
            let _ = self.writer_tx.try_send(WriterCommand::Frame(frame));
        }
    }

    fn classify_message(
        &mut self,
        expected_id: u64,
        value: &Value,
    ) -> Result<Incoming, McpFailure> {
        match classify_incoming(expected_id, self.pending_id, value) {
            Ok(incoming) => Ok(incoming),
            Err(error) => {
                if error.kind() == McpFailureKind::Protocol {
                    self.poisoned = true;
                }
                Err(error)
            }
        }
    }

    pub(crate) fn shutdown(&mut self, graceful: bool) -> McpCleanupReport {
        if let Some(existing) = &self.cleanup {
            return existing.clone();
        }
        let _ = self.writer_tx.try_send(WriterCommand::Close);
        let force = !graceful || self.poisoned;
        let _ = self.monitor_tx.try_send(if force {
            MonitorCommand::Force
        } else {
            MonitorCommand::Graceful
        });

        let mut writer = self.writer.take();
        let mut monitor = self.monitor.take();
        let mut reader = self.reader.take();
        let mut stderr = self.stderr.take();
        let writer_result = join_pump(&mut writer, "writer");
        let monitor_result = join_monitor(&mut monitor);
        let reader_result = join_pump(&mut reader, "reader");
        let stderr_result = join_stderr(&mut stderr);
        let pumps_joined = writer_result.is_ok() && reader_result.is_ok() && stderr_result.is_ok();
        let join_failure = writer_result
            .as_ref()
            .err()
            .or_else(|| monitor_result.as_ref().err())
            .or_else(|| reader_result.as_ref().err())
            .or_else(|| stderr_result.as_ref().err())
            .cloned();
        let monitor = monitor_result.unwrap_or(MonitorReport {
            status: None,
            forced: true,
            tree_termination_attempted: true,
            root_reaped: false,
            captured_descendants: 0,
            descendants_gone: false,
        });
        let stderr = stderr_result.unwrap_or_else(|failure| StreamCapture {
            tail: redact_vendor_text(&failure.message),
            sha256: sha256_bytes(failure.message.as_bytes()),
            total_bytes: failure.message.len() as u64,
            truncated: false,
        });
        let report = McpCleanupReport {
            children_reaped: monitor.root_reaped && monitor.descendants_gone,
            forced: monitor.forced,
            stderr_sha256: stderr.sha256.clone(),
            stderr_truncated: stderr.truncated,
            stderr,
            monitor,
            pumps_joined,
            join_failure,
        };
        self.cleanup = Some(report.clone());
        report
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        if self.cleanup.is_none() {
            let _ = self.shutdown(false);
        }
    }
}

#[derive(Debug)]
enum Incoming {
    Response(Value),
    Notification,
}

fn classify_incoming(
    expected_id: u64,
    pending_id: Option<u64>,
    value: &Value,
) -> Result<Incoming, McpFailure> {
    let object = value
        .as_object()
        .ok_or_else(|| McpFailure::protocol("MCP frame must be one JSON object"))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpFailure::protocol(
            "MCP message has an unsupported jsonrpc version",
        ));
    }
    if let Some(method) = object.get("method").and_then(Value::as_str) {
        if object.contains_key("id") {
            return Err(McpFailure::protocol(format!(
                "unsupported MCP server request/elicitation: {method}"
            )));
        }
        if !matches!(
            method,
            "notifications/message"
                | "notifications/progress"
                | "notifications/tools/list_changed"
                | "notifications/prompts/list_changed"
        ) {
            return Err(McpFailure::protocol(format!(
                "unsupported MCP notification: {method}"
            )));
        }
        return Ok(Incoming::Notification);
    }
    let id = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| McpFailure::protocol("MCP response has no numeric id"))?;
    if id != expected_id || pending_id != Some(id) {
        return Err(McpFailure::protocol(format!(
            "unexpected MCP response id {id}; expected {expected_id}"
        )));
    }
    match (object.get("result"), object.get("error")) {
        (Some(result), None) => Ok(Incoming::Response(result.clone())),
        (None, Some(error)) => Err(McpFailure::backend(format!(
            "Modeling MCP request {id} failed: {}",
            bounded_error_summary(error)
        ))),
        _ => Err(McpFailure::protocol(
            "MCP response must contain exactly one of result or error",
        )),
    }
}

pub(crate) fn deep_handshake(tool: &InstalledMicrosoftTool) -> CliResult<Value> {
    let mut session =
        McpSession::open_exact(tool, McpSessionMode::ReadOnly, McpSessionConfig::default())
            .map_err(McpFailure::into_cli_error)?;
    let handshake = session.handshake();
    let cleanup = session.shutdown(handshake.is_ok());
    match handshake {
        Ok(handshake) if cleanup.children_reaped && cleanup.pumps_joined => Ok(json!({
            "verified": true,
            "method": "mcp-initialize-and-tools-list",
            "protocolVersion": handshake.protocol_version,
            "server": {
                "name": handshake.server_name,
                "version": handshake.server_version
            },
            "tools": {
                "count": handshake.tools_count,
                "normalizedSha256": handshake.tools_list_sha256
            },
            "transport": "stdio",
            "readOnly": true,
            "notificationsSeen": handshake.notifications_seen,
            "childrenReaped": cleanup.children_reaped,
            "pumpsJoined": cleanup.pumps_joined,
            "forcedCleanup": cleanup.forced,
            "stderrSha256": cleanup.stderr_sha256,
            "stderrTruncated": cleanup.stderr_truncated,
            "stderrBytes": cleanup.stderr.total_bytes,
            "processStatus": cleanup.monitor.status.as_ref().and_then(ExitStatus::code),
            "processTreeTerminationAttempted": cleanup.monitor.tree_termination_attempted,
            "rootReaped": cleanup.monitor.root_reaped,
            "capturedDescendants": cleanup.monitor.captured_descendants,
            "capturedDescendantsGone": cleanup.monitor.descendants_gone
        })),
        Ok(_) => {
            let failure = cleanup.join_failure.clone().map_or_else(
                || {
                    McpFailure::backend(
                        "Modeling MCP handshake succeeded but child cleanup was incomplete",
                    )
                },
                PumpJoinFailure::into_failure,
            );
            Err(failure.with_cleanup(&cleanup).into_cli_error())
        }
        Err(error) => Err(error.with_cleanup(&cleanup).into_cli_error()),
    }
}

#[derive(Debug, Clone)]
pub(crate) enum McpOperation {
    ListConnections,
    ConnectLocalEndpoint {
        port: u16,
    },
    ConnectFolder {
        folder_path: PathBuf,
    },
    GetPartition {
        connection_name: String,
        table_name: String,
        partition_name: String,
    },
    ReplacePartitionSource {
        connection_name: String,
        table_name: String,
        partition_name: String,
        expression: String,
    },
    ExportTmdlFolder {
        connection_name: String,
        folder_path: PathBuf,
    },
}

impl McpOperation {
    fn tool_name(&self) -> &'static str {
        match self {
            Self::ListConnections
            | Self::ConnectLocalEndpoint { .. }
            | Self::ConnectFolder { .. } => "connection_operations",
            Self::GetPartition { .. } | Self::ReplacePartitionSource { .. } => {
                "partition_operations"
            }
            Self::ExportTmdlFolder { .. } => "database_operations",
        }
    }

    fn arguments(&self) -> Result<Value, McpFailure> {
        match self {
            Self::ListConnections => Ok(json!({"request": {"operation": "ListConnections"}})),
            Self::ConnectLocalEndpoint { port } => Ok(json!({
                "request": {
                    "operation": "Connect",
                    "dataSource": format!("localhost:{port}")
                }
            })),
            Self::ConnectFolder { folder_path } => Ok(json!({
                "request": {
                    "operation": "ConnectFolder",
                    "folderPath": checked_path(folder_path, "folderPath")?
                }
            })),
            Self::GetPartition {
                connection_name,
                table_name,
                partition_name,
            } => Ok(json!({
                "request": {
                    "operation": "Get",
                    "connectionName": checked_identifier(connection_name, "connectionName")?,
                    "references": [{
                        "tableName": checked_identifier(table_name, "tableName")?,
                        "name": checked_identifier(partition_name, "partitionName")?
                    }]
                }
            })),
            Self::ReplacePartitionSource {
                connection_name,
                table_name,
                partition_name,
                expression,
            } => {
                if expression.trim().is_empty() || expression.len() > DEFAULT_FRAME_LIMIT {
                    return Err(McpFailure::protocol(
                        "partition source expression is empty or exceeds the MCP payload cap",
                    ));
                }
                let arguments = json!({
                    "request": {
                        "operation": "Update",
                        "connectionName": checked_identifier(connection_name, "connectionName")?,
                        "definitions": [{
                            "tableName": checked_identifier(table_name, "tableName")?,
                            "name": checked_identifier(partition_name, "partitionName")?,
                            "sourceType": "M",
                            "expression": expression
                        }],
                        "options": {
                            "continueOnError": false,
                            "useTransaction": false
                        }
                    }
                });
                let largest_envelope = json!({
                    "jsonrpc": "2.0",
                    "id": u64::MAX,
                    "method": "tools/call",
                    "params": {
                        "name": self.tool_name(),
                        "arguments": &arguments
                    }
                });
                let encoded_len = serde_json::to_vec(&largest_envelope)
                    .map_err(|error| {
                        McpFailure::protocol(format!("serialize bounded partition update: {error}"))
                    })?
                    .len();
                if encoded_len > DEFAULT_FRAME_LIMIT {
                    return Err(McpFailure::protocol(
                        "partition source expression exceeds the MCP frame budget after encoding",
                    ));
                }
                Ok(arguments)
            }
            Self::ExportTmdlFolder {
                connection_name,
                folder_path,
            } => Ok(json!({
                "request": {
                    "operation": "ExportToTmdlFolder",
                    "connectionName": checked_identifier(connection_name, "connectionName")?,
                    "tmdlFolderPath": checked_path(folder_path, "tmdlFolderPath")?
                }
            })),
        }
    }
}

struct ClosedToolPolicy;

impl ClosedToolPolicy {
    fn authorize(tool: &str, arguments: &Value, mode: McpSessionMode) -> Result<(), McpFailure> {
        let top = exact_object(arguments, &["request"], "tool arguments")?;
        let request = top
            .get("request")
            .and_then(Value::as_object)
            .ok_or_else(|| McpFailure::protocol("MCP tool request must be one object"))?;
        let operation = request
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| McpFailure::protocol("MCP tool request has no operation"))?;
        let (allowed, write) = match (tool, operation) {
            ("connection_operations", "ListConnections") => (&["operation"][..], false),
            ("connection_operations", "Connect") => (&["dataSource", "operation"][..], false),
            ("connection_operations", "ConnectFolder") => (&["folderPath", "operation"][..], false),
            ("partition_operations", "Get") => {
                (&["connectionName", "operation", "references"][..], false)
            }
            ("partition_operations", "Update") => (
                &["connectionName", "definitions", "operation", "options"][..],
                true,
            ),
            ("database_operations", "ExportToTmdlFolder") => (
                &["connectionName", "operation", "tmdlFolderPath"][..],
                false,
            ),
            _ => {
                return Err(McpFailure::protocol(format!(
                    "MCP tool/operation is outside the closed policy: {tool}/{operation}"
                )));
            }
        };
        exact_keys(request, allowed, "tool request")?;
        validate_nested_policy(tool, operation, request)?;
        if write && mode.is_read_only() {
            return Err(McpFailure::protocol(format!(
                "write operation {tool}/{operation} is forbidden in a read-only MCP session"
            )));
        }
        Ok(())
    }
}

fn validate_nested_policy(
    tool: &str,
    operation: &str,
    request: &Map<String, Value>,
) -> Result<(), McpFailure> {
    match (tool, operation) {
        ("connection_operations", "Connect") => {
            let data_source = request
                .get("dataSource")
                .and_then(Value::as_str)
                .ok_or_else(|| McpFailure::protocol("Connect dataSource must be a string"))?;
            let port = data_source
                .strip_prefix("localhost:")
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    McpFailure::protocol(
                        "Connect is restricted to one validated localhost:<u16> endpoint",
                    )
                })?;
            if data_source != format!("localhost:{port}") {
                return Err(McpFailure::protocol(
                    "Connect endpoint must use canonical localhost:<u16> form",
                ));
            }
        }
        ("partition_operations", "Get") => {
            validate_single_item_array(request, "references", &["name", "tableName"])?;
        }
        ("partition_operations", "Update") => {
            let item = validate_single_item_array(
                request,
                "definitions",
                &["expression", "name", "sourceType", "tableName"],
            )?;
            if item.get("sourceType").and_then(Value::as_str) != Some("M") {
                return Err(McpFailure::protocol(
                    "partition Update accepts only a complete M source expression",
                ));
            }
            let options = request
                .get("options")
                .and_then(Value::as_object)
                .ok_or_else(|| McpFailure::protocol("partition Update requires closed options"))?;
            exact_keys(
                options,
                &["continueOnError", "useTransaction"],
                "partition Update options",
            )?;
            if options.get("continueOnError").and_then(Value::as_bool) != Some(false)
                || options.get("useTransaction").and_then(Value::as_bool) != Some(false)
            {
                return Err(McpFailure::protocol(
                    "offline partition Update requires continueOnError=false and useTransaction=false",
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_single_item_array<'a>(
    request: &'a Map<String, Value>,
    field: &str,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, McpFailure> {
    let items = request
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol(format!("{field} must be one array")))?;
    if items.len() != 1 {
        return Err(McpFailure::protocol(format!(
            "{field} must contain exactly one typed operation"
        )));
    }
    let item = items[0]
        .as_object()
        .ok_or_else(|| McpFailure::protocol(format!("{field}[0] must be one object")))?;
    exact_keys(item, keys, field)?;
    Ok(item)
}

fn exact_object<'a>(
    value: &'a Value,
    keys: &[&str],
    label: &str,
) -> Result<&'a Map<String, Value>, McpFailure> {
    let object = value
        .as_object()
        .ok_or_else(|| McpFailure::protocol(format!("{label} must be one object")))?;
    exact_keys(object, keys, label)?;
    Ok(object)
}

fn exact_keys(object: &Map<String, Value>, keys: &[&str], label: &str) -> Result<(), McpFailure> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = keys.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(McpFailure::protocol(format!(
            "{label} fields are outside the closed policy: expected [{}], got [{}]",
            expected.iter().copied().collect::<Vec<_>>().join(", "),
            actual.iter().copied().collect::<Vec<_>>().join(", ")
        )));
    }
    Ok(())
}

fn normalized_tools_identity(result: &Value) -> Result<(usize, String), McpFailure> {
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("tools/list result has no tools array"))?;
    let mut names = BTreeSet::new();
    let mut normalized = Vec::with_capacity(tools.len());
    for tool in tools {
        let object = tool
            .as_object()
            .ok_or_else(|| McpFailure::protocol("tools/list contains a non-object tool"))?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| McpFailure::protocol("tools/list contains a tool with no name"))?;
        if !names.insert(name.to_string()) {
            return Err(McpFailure::protocol(format!(
                "tools/list contains duplicate tool name {name}"
            )));
        }
        if !object.get("inputSchema").is_some_and(Value::is_object) {
            return Err(McpFailure::protocol(format!(
                "tools/list tool {name} has no object inputSchema"
            )));
        }
        normalized.push((name.to_string(), normalize_json(tool)));
    }
    normalized.sort_by(|left, right| left.0.cmp(&right.0));
    let normalized = Value::Array(normalized.into_iter().map(|(_, value)| value).collect());
    let bytes = serde_json::to_vec(&normalized)
        .map_err(|error| McpFailure::protocol(format!("normalize tools/list: {error}")))?;
    Ok((tools.len(), sha256_bytes(&bytes)))
}

fn normalize_json(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut normalized = Map::new();
            for key in keys {
                normalized.insert(key.clone(), normalize_json(&object[key]));
            }
            Value::Object(normalized)
        }
        Value::Array(values) => Value::Array(values.iter().map(normalize_json).collect()),
        other => other.clone(),
    }
}

fn required_string(value: &Value, key: &str) -> Result<String, McpFailure> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| McpFailure::protocol(format!("MCP result has no string {key}")))
}

fn required_object_string(
    object: &Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<String, McpFailure> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| McpFailure::protocol(format!("{label} has no string {key}")))
}

fn checked_identifier(value: &str, label: &str) -> Result<String, McpFailure> {
    let value = value.trim();
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(McpFailure::protocol(format!(
            "{label} is empty or outside the closed identifier policy"
        )));
    }
    Ok(value.to_string())
}

fn checked_path(path: &Path, label: &str) -> Result<String, McpFailure> {
    if !path.is_absolute() {
        return Err(McpFailure::protocol(format!(
            "{label} must be an absolute workflow-owned path"
        )));
    }
    let value = path.to_str().ok_or_else(|| {
        McpFailure::protocol(format!(
            "{label} must be valid Unicode within the closed path policy"
        ))
    })?;
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(McpFailure::protocol(format!(
            "{label} is outside the closed path policy"
        )));
    }
    Ok(value.to_string())
}

fn bounded_error_summary(error: &Value) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown MCP error");
    let summary = code.map_or_else(
        || message.to_string(),
        |code| format!("code {code}: {message}"),
    );
    redact_vendor_text(&summary)
}

fn redact_vendor_text(text: &str) -> String {
    let home = env::var("USERPROFILE")
        .ok()
        .or_else(|| env::var("HOME").ok());
    let mut output = text
        .lines()
        .rev()
        .take(200)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
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
    if output.len() > DEFAULT_STDERR_LIMIT {
        let mut start = output.len() - DEFAULT_STDERR_LIMIT;
        while !output.is_char_boundary(start) {
            start += 1;
        }
        output = output[start..].to_string();
    }
    output
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("sha256:{}", hex_digest(digest.finalize().as_slice()))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn tools_identity_is_order_independent_but_schema_sensitive() {
        let a = json!({"tools": [
            {"name": "z", "description": "last", "inputSchema": {"type": "object"}},
            {"inputSchema": {"properties": {}, "type": "object"}, "name": "a"}
        ]});
        let b = json!({"tools": [
            {"name": "a", "inputSchema": {"type": "object", "properties": {}}},
            {"inputSchema": {"type": "object"}, "description": "last", "name": "z"}
        ]});
        assert_eq!(
            normalized_tools_identity(&a).expect("identity"),
            normalized_tools_identity(&b).expect("identity")
        );
        let changed = json!({"tools": [
            {"name": "a", "inputSchema": {"type": "object"}},
            {"name": "z", "description": "changed", "inputSchema": {"type": "object"}}
        ]});
        assert_ne!(
            normalized_tools_identity(&a).expect("identity"),
            normalized_tools_identity(&changed).expect("identity")
        );
    }

    #[test]
    fn closed_policy_rejects_unknown_nested_and_read_only_writes() {
        let connect = McpOperation::ConnectLocalEndpoint { port: 65_348 };
        let connect_arguments = connect.arguments().expect("typed local endpoint");
        ClosedToolPolicy::authorize(
            connect.tool_name(),
            &connect_arguments,
            McpSessionMode::ReadOnly,
        )
        .expect("canonical localhost endpoint");
        assert!(
            ClosedToolPolicy::authorize(
                "connection_operations",
                &json!({"request": {
                    "operation": "Connect",
                    "dataSource": "127.0.0.1:65348"
                }}),
                McpSessionMode::ReadOnly
            )
            .is_err()
        );
        assert!(
            ClosedToolPolicy::authorize(
                "connection_operations",
                &json!({"request": {
                    "operation": "Connect",
                    "dataSource": "localhost:65348",
                    "initialCatalog": "unvalidated"
                }}),
                McpSessionMode::ReadOnly
            )
            .is_err()
        );
        assert!(
            ClosedToolPolicy::authorize(
                "arbitrary_tool",
                &json!({"request": {"operation": "Anything"}}),
                McpSessionMode::ReadOnly
            )
            .is_err()
        );
        assert!(
            ClosedToolPolicy::authorize(
                "connection_operations",
                &json!({"request": {
                    "operation": "ListConnections",
                    "nested": {"operation": "Delete"}
                }}),
                McpSessionMode::ReadOnly
            )
            .is_err()
        );
        let update = McpOperation::ReplacePartitionSource {
            connection_name: "folder".to_string(),
            table_name: "Fact".to_string(),
            partition_name: "Fact".to_string(),
            expression: "let Source = #table({}, {}) in Source".to_string(),
        };
        let arguments = update.arguments().expect("typed arguments");
        assert!(
            ClosedToolPolicy::authorize(update.tool_name(), &arguments, McpSessionMode::ReadOnly)
                .is_err()
        );
        ClosedToolPolicy::authorize(
            update.tool_name(),
            &arguments,
            McpSessionMode::ConfirmedWrite,
        )
        .expect("confirmed write");
    }

    #[test]
    fn partition_update_cap_accounts_for_the_complete_json_rpc_frame() {
        let operation = |expression: String| McpOperation::ReplacePartitionSource {
            connection_name: "folder".to_string(),
            table_name: "Fact".to_string(),
            partition_name: "Fact".to_string(),
            expression,
        };
        let mut accepted = 1_usize;
        let mut rejected = DEFAULT_FRAME_LIMIT + 1;
        while accepted + 1 < rejected {
            let candidate = accepted + (rejected - accepted) / 2;
            if operation("x".repeat(candidate)).arguments().is_ok() {
                accepted = candidate;
            } else {
                rejected = candidate;
            }
        }
        let accepted_arguments = operation("x".repeat(accepted))
            .arguments()
            .expect("largest bounded expression");
        let accepted_frame = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": u64::MAX,
            "method": "tools/call",
            "params": {
                "name": "partition_operations",
                "arguments": accepted_arguments
            }
        }))
        .expect("accepted frame");
        assert!(accepted_frame.len() <= DEFAULT_FRAME_LIMIT);
        assert!(operation("x".repeat(rejected)).arguments().is_err());
        let rejected_arguments = json!({
            "request": {
                "operation": "Update",
                "connectionName": "folder",
                "definitions": [{
                    "tableName": "Fact",
                    "name": "Fact",
                    "sourceType": "M",
                    "expression": "x".repeat(rejected)
                }],
                "options": {"continueOnError": false, "useTransaction": false}
            }
        });
        let rejected_frame = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": u64::MAX,
            "method": "tools/call",
            "params": {
                "name": "partition_operations",
                "arguments": rejected_arguments
            }
        }))
        .expect("rejected frame");
        assert!(rejected_frame.len() > DEFAULT_FRAME_LIMIT);
    }

    #[test]
    fn checked_path_rejects_non_unicode_paths() {
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from("/tmp").join(std::ffi::OsString::from_vec(vec![0xff]))
        };
        #[cfg(windows)]
        let path = {
            use std::os::windows::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_wide(&[
                b'C' as u16,
                b':' as u16,
                b'\\' as u16,
                0xd800,
            ]))
        };
        let error = checked_path(&path, "folderPath").expect_err("non-Unicode path");
        assert!(error.message().contains("valid Unicode"));
    }

    #[test]
    fn protocol_rejects_wrong_ids_and_server_elicitation_fail_closed() {
        let wrong_id = classify_incoming(
            7,
            Some(7),
            &json!({"jsonrpc": "2.0", "id": 8, "result": {}}),
        )
        .expect_err("a response for another request must be rejected");
        assert_eq!(wrong_id.kind(), McpFailureKind::Protocol);
        assert!(wrong_id.message.contains("unexpected MCP response id"));

        let elicitation = classify_incoming(
            7,
            Some(7),
            &json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "elicitation/create",
                "params": {}
            }),
        )
        .expect_err("server elicitation is outside the closed policy");
        assert_eq!(elicitation.kind(), McpFailureKind::Protocol);
        assert!(elicitation.message.contains("server request/elicitation"));

        let batch = classify_incoming(7, Some(7), &json!([]))
            .expect_err("JSON-RPC batches are not accepted");
        assert_eq!(batch.kind(), McpFailureKind::Protocol);
    }

    #[test]
    fn fake_server_identity_and_tool_surface_drift_fail_closed() {
        let tools = json!({"tools": []});
        let (_, tools_hash) = normalized_tools_identity(&tools).expect("tools identity");
        let mut identity_drift = McpSession::open_command(
            good_fake_server_command(),
            ModelingMcpContract {
                protocol_version: "wrong-protocol".to_string(),
                server_name: "fake-powerbi-mcp".to_string(),
                server_version: "1.2.3".to_string(),
                tools_count: 0,
                tools_list_sha256: tools_hash.clone(),
            },
            McpSessionMode::ReadOnly,
            McpSessionConfig::default(),
        )
        .expect("open identity-drift fake");
        let error = identity_drift
            .handshake()
            .expect_err("identity drift must fail");
        assert_eq!(error.kind(), McpFailureKind::Protocol);
        assert!(error.message.contains("identity drift"));
        assert!(identity_drift.shutdown(false).children_reaped);

        let mut tools_drift = McpSession::open_command(
            good_fake_server_command(),
            ModelingMcpContract {
                protocol_version: "test-v1".to_string(),
                server_name: "fake-powerbi-mcp".to_string(),
                server_version: "1.2.3".to_string(),
                tools_count: 0,
                tools_list_sha256: sha256_bytes(b"different tools surface"),
            },
            McpSessionMode::ReadOnly,
            McpSessionConfig::default(),
        )
        .expect("open tool-drift fake");
        let error = tools_drift
            .handshake()
            .expect_err("tool-surface drift must fail");
        assert_eq!(error.kind(), McpFailureKind::Protocol);
        assert!(error.message.contains("tool surface drift"));
        assert!(tools_drift.shutdown(false).children_reaped);
    }

    #[test]
    fn fragmented_and_interleaved_frames_are_reassembled_with_hard_caps() {
        let (sender, receiver) = mpsc::sync_channel(8);
        let bytes = b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
        let mut reader = OneByteReader::new(bytes);
        let frames = read_frames(&mut reader, &sender, 128, 512).expect("frames");
        assert_eq!(frames, 2);
        assert!(matches!(receiver.recv(), Ok(ReaderEvent::Frame(_))));
        assert!(matches!(receiver.recv(), Ok(ReaderEvent::Frame(_))));
        assert!(matches!(receiver.recv(), Ok(ReaderEvent::Eof)));

        let (sender, receiver) = mpsc::sync_channel(2);
        let mut oversized = Cursor::new(vec![b'x'; 33]);
        assert!(read_frames(&mut oversized, &sender, 32, 128).is_err());
        assert!(matches!(receiver.recv(), Ok(ReaderEvent::Failure(_))));
    }

    #[test]
    fn saturated_reader_queue_stops_without_blocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        sender
            .try_send(ReaderEvent::Frame(b"occupied".to_vec()))
            .expect("fill bounded queue");
        let (finished_tx, finished_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut reader = Cursor::new(b"{\"jsonrpc\":\"2.0\"}\n".to_vec());
            let result = read_frames(&mut reader, &sender, 128, 512);
            let _ = finished_tx.try_send(result);
        });
        let error = finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("saturated reader must terminate")
            .expect_err("saturated reader must fail closed");
        assert!(error.contains("queue saturated"));
        worker.join().expect("reader worker");
    }

    #[test]
    fn stderr_flood_is_hashed_bounded_and_redacted() {
        let mut bytes = vec![b'x'; 128 * 1024];
        bytes.extend_from_slice(b"\npassword=super-secret\n");
        let expected_hash = sha256_bytes(&bytes);
        let mut reader = Cursor::new(bytes);
        let captured = capture_stderr(&mut reader, 1024).expect("capture");
        assert_eq!(captured.sha256, expected_hash);
        assert!(captured.truncated);
        assert!(captured.tail.contains("[redacted]"));
        assert!(!captured.tail.contains("super-secret"));
        assert!(captured.tail.len() <= 1024);
    }

    #[test]
    fn vendor_stderr_tail_is_utf8_boundary_safe() {
        let input = "é".repeat(DEFAULT_STDERR_LIMIT);
        let output = redact_vendor_text(&input);
        assert!(output.len() <= DEFAULT_STDERR_LIMIT);
        assert!(output.is_char_boundary(0));
        assert!(output.chars().all(|character| character == 'é'));
    }

    #[test]
    fn fake_server_handshake_handles_fragmentation_notifications_and_stderr_flood() {
        let tools = json!({"tools": []});
        let (_, tools_list_sha256) = normalized_tools_identity(&tools).expect("tools identity");
        let expected = ModelingMcpContract {
            protocol_version: "test-v1".to_string(),
            server_name: "fake-powerbi-mcp".to_string(),
            server_version: "1.2.3".to_string(),
            tools_count: 0,
            tools_list_sha256,
        };
        let mut session = McpSession::open_command(
            good_fake_server_command(),
            expected,
            McpSessionMode::ReadOnly,
            McpSessionConfig {
                call_timeout: Duration::from_secs(3),
                session_timeout: Duration::from_secs(10),
                cleanup_timeout: Duration::from_secs(2),
                ..McpSessionConfig::default()
            },
        )
        .expect("open fake server");
        let handshake = session.handshake().expect("fake handshake");
        assert_eq!(handshake.notifications_seen, 1);
        let cleanup = session.shutdown(true);
        assert!(cleanup.children_reaped);
        assert!(cleanup.stderr_truncated);
        assert!(cleanup.stderr.tail.contains("[redacted]"));
        assert!(!cleanup.stderr.tail.contains("super-secret"));
    }

    #[test]
    fn fake_server_timeout_cancels_and_reaps_without_deadlock() {
        let expected = ModelingMcpContract {
            protocol_version: "test-v1".to_string(),
            server_name: "fake-powerbi-mcp".to_string(),
            server_version: "1.2.3".to_string(),
            tools_count: 0,
            tools_list_sha256: sha256_bytes(b"[]"),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let descendant_pid = temp.path().join("descendant.pid");
        let started = Instant::now();
        let mut session = McpSession::open_command(
            hanging_fake_server_command(&descendant_pid),
            expected,
            McpSessionMode::ReadOnly,
            McpSessionConfig {
                call_timeout: Duration::from_secs(1),
                session_timeout: Duration::from_secs(2),
                cleanup_timeout: Duration::from_millis(500),
                ..McpSessionConfig::default()
            },
        )
        .expect("open hanging fake");
        wait_for_file(&descendant_pid);
        let error = session.handshake().expect_err("deadline must cancel");
        assert_eq!(error.kind, McpFailureKind::Cancelled);
        let cleanup = session.shutdown(false);
        assert!(cleanup.children_reaped);
        assert!(cleanup.forced);
        assert!(cleanup.monitor.tree_termination_attempted);
        assert!(cleanup.monitor.root_reaped);
        assert!(cleanup.monitor.descendants_gone);
        assert!(started.elapsed() < Duration::from_secs(5));
        let pid_text = std::fs::read_to_string(&descendant_pid).expect("descendant pid marker");
        let pid = Pid::from_u32(pid_text.trim().parse().expect("descendant pid"));
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        assert!(
            system.process(pid).is_none(),
            "timeout cleanup left a descendant process running"
        );
    }

    #[test]
    fn graceful_root_exit_also_reaps_captured_descendants() {
        let expected = ModelingMcpContract {
            protocol_version: "test-v1".to_string(),
            server_name: "fake-powerbi-mcp".to_string(),
            server_version: "1.2.3".to_string(),
            tools_count: 0,
            tools_list_sha256: sha256_bytes(b"[]"),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let descendant_pid = temp.path().join("descendant.pid");
        let mut session = McpSession::open_command(
            graceful_descendant_fake_server_command(&descendant_pid),
            expected,
            McpSessionMode::ReadOnly,
            McpSessionConfig {
                call_timeout: Duration::from_secs(3),
                session_timeout: Duration::from_secs(10),
                cleanup_timeout: Duration::from_secs(2),
                ..McpSessionConfig::default()
            },
        )
        .expect("open graceful descendant fake");
        session.handshake().expect("handshake");
        wait_for_file(&descendant_pid);
        let descendant = process_identity_from_marker(&descendant_pid);
        let cleanup = session.shutdown(true);
        assert!(cleanup.children_reaped);
        assert!(!cleanup.forced);
        assert!(cleanup.monitor.root_reaped);
        assert!(cleanup.monitor.descendants_gone);
        assert_process_identities_are_gone(&[descendant]);
    }

    #[test]
    fn child_guard_drop_terminates_the_owned_process_tree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let descendant_pid = temp.path().join("descendant.pid");
        let mut command = hanging_fake_server_command(&descendant_pid);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = spawn_contained(&mut command).expect("spawn owned child tree");
        child
            .inner()
            .stdin
            .take()
            .expect("child stdin")
            .write_all(b"start\n")
            .expect("start child tree");
        wait_for_file(&descendant_pid);
        let descendant = process_identity_from_marker(&descendant_pid);
        drop(ChildGuard::new(child));
        assert_process_identities_are_gone(&[descendant]);
    }

    #[test]
    fn spawn_time_container_reaps_descendants_created_during_shutdown() {
        let temp = tempfile::tempdir().expect("tempdir");
        let descendant_pids = temp.path().join("descendants.pid");
        let mut command = racing_descendant_command(&descendant_pids);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = spawn_contained(&mut command).expect("spawn contained descendant race");
        wait_for_pid_lines(&descendant_pids, 3);
        let descendants = process_identities_from_markers(&descendant_pids);
        assert!(descendants.len() >= 3);
        drop(ChildGuard::new(child));
        assert_process_identities_are_gone(&descendants);
    }

    fn wait_for_file(path: &Path) {
        let started = Instant::now();
        while !path.is_file() && started.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(path.is_file(), "process marker was not created");
    }

    fn wait_for_pid_lines(path: &Path, expected: usize) {
        let started = Instant::now();
        loop {
            let count = std::fs::read_to_string(path)
                .map(|contents| contents.lines().count())
                .unwrap_or(0);
            if count >= expected {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "racing descendant process markers were not created"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn process_identity_from_marker(path: &Path) -> (u32, u64) {
        let pid_text = std::fs::read_to_string(path).expect("descendant pid marker");
        let pid = Pid::from_u32(pid_text.trim().parse().expect("descendant pid"));
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        let process = system
            .process(pid)
            .expect("descendant process must exist before cleanup");
        (pid.as_u32(), process.start_time())
    }

    fn process_identities_from_markers(path: &Path) -> Vec<(u32, u64)> {
        let pid_text = std::fs::read_to_string(path).expect("descendant pid list");
        let pids = pid_text
            .lines()
            .map(|value| Pid::from_u32(value.parse().expect("descendant pid")))
            .collect::<Vec<_>>();
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
        pids.into_iter()
            .filter_map(|pid| {
                system
                    .process(pid)
                    .map(|process| (pid.as_u32(), process.start_time()))
            })
            .collect()
    }

    fn assert_process_identities_are_gone(identities: &[(u32, u64)]) {
        let started = Instant::now();
        let pids = identities
            .iter()
            .map(|(pid, _)| Pid::from_u32(*pid))
            .collect::<Vec<_>>();
        let mut system = System::new();
        loop {
            system.refresh_processes(ProcessesToUpdate::Some(&pids), true);
            let alive = identities.iter().any(|(pid, process_started)| {
                system
                    .process(Pid::from_u32(*pid))
                    .is_some_and(|process| process.start_time() == *process_started)
            });
            if !alive {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "cleanup left a captured descendant process identity running"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(windows)]
    fn good_fake_server_command() -> Command {
        let mut command = powershell_command();
        command.arg(
            r#"
$null = [Console]::In.ReadLine()
[Console]::Error.Write(('x' * 70000))
[Console]::Error.WriteLine("`ntoken=super-secret")
[Console]::Out.Write('{"jsonrpc":"2.0","method":"notifications/message","params":{}}' + "`n")
[Console]::Out.Flush()
[Console]::Out.Write('{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"test-v1",')
[Console]::Out.Flush()
Start-Sleep -Milliseconds 20
[Console]::Out.Write('"serverInfo":{"name":"fake-powerbi-mcp","version":"1.2.3"}}}' + "`n")
[Console]::Out.Flush()
$null = [Console]::In.ReadLine()
$null = [Console]::In.ReadLine()
[Console]::Out.WriteLine('{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}')
[Console]::Out.Flush()
"#,
        );
        command
    }

    #[cfg(windows)]
    fn hanging_fake_server_command(descendant_pid: &Path) -> Command {
        let mut command = powershell_command();
        command.arg(format!(
r#"
$child = Start-Process "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30' -WindowStyle Hidden -PassThru
[IO.File]::WriteAllText('{}', [string]$child.Id)
$null = [Console]::In.ReadLine()
[Console]::Error.WriteLine('token=super-secret')
Start-Sleep -Seconds 30
"#,
            descendant_pid.display()
        ));
        command
    }

    #[cfg(windows)]
    fn graceful_descendant_fake_server_command(descendant_pid: &Path) -> Command {
        let mut command = powershell_command();
        command.arg(format!(
            r#"
$null = [Console]::In.ReadLine()
[Console]::Out.WriteLine('{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"test-v1","serverInfo":{{"name":"fake-powerbi-mcp","version":"1.2.3"}}}}}}')
[Console]::Out.Flush()
$null = [Console]::In.ReadLine()
$null = [Console]::In.ReadLine()
[Console]::Out.WriteLine('{{"jsonrpc":"2.0","id":2,"result":{{"tools":[]}}}}')
[Console]::Out.Flush()
$child = Start-Process "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30' -WindowStyle Hidden -PassThru
[IO.File]::WriteAllText('{}', [string]$child.Id)
Start-Sleep -Milliseconds 250
"#,
            descendant_pid.display()
        ));
        command
    }

    #[cfg(windows)]
    fn racing_descendant_command(descendant_pids: &Path) -> Command {
        let mut command = powershell_command();
        command.arg(format!(
            r#"
while ($true) {{
    $child = Start-Process "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList '-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 30' -WindowStyle Hidden -PassThru
    [IO.File]::AppendAllText('{}', ([string]$child.Id + [Environment]::NewLine))
}}
"#,
            descendant_pids.display()
        ));
        command
    }

    #[cfg(windows)]
    fn powershell_command() -> Command {
        let system_root = env::var_os("SystemRoot").expect("SystemRoot");
        let executable = PathBuf::from(system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let mut command = Command::new(executable);
        command.args(["-NoProfile", "-NonInteractive", "-Command"]);
        command
    }

    #[cfg(unix)]
    fn good_fake_server_command() -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(
            r#"
IFS= read -r init
head -c 70000 /dev/zero | tr '\0' x >&2
printf '\ntoken=super-secret\n' >&2
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/message","params":{}}'
printf '%s' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"test-v1",'
sleep 0.02
printf '%s\n' '"serverInfo":{"name":"fake-powerbi-mcp","version":"1.2.3"}}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}'
"#,
        );
        command
    }

    #[cfg(unix)]
    fn hanging_fake_server_command(descendant_pid: &Path) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "sleep 30 & echo $! > '{}'; IFS= read -r init; printf 'token=super-secret\\n' >&2; sleep 30",
            descendant_pid.display()
        ));
        command
    }

    #[cfg(unix)]
    fn graceful_descendant_fake_server_command(descendant_pid: &Path) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "IFS= read -r init; printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"protocolVersion\":\"test-v1\",\"serverInfo\":{{\"name\":\"fake-powerbi-mcp\",\"version\":\"1.2.3\"}}}}}}'; IFS= read -r initialized; IFS= read -r list; printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"tools\":[]}}}}'; sleep 30 & echo $! > '{}'; sleep 0.25",
            descendant_pid.display()
        ));
        command
    }

    #[cfg(unix)]
    fn racing_descendant_command(descendant_pids: &Path) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "while :; do sleep 30 & echo $! >> '{}'; sleep 0.01; done",
            descendant_pids.display()
        ));
        command
    }

    struct OneByteReader<'a> {
        bytes: &'a [u8],
        offset: usize,
    }

    impl<'a> OneByteReader<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self { bytes, offset: 0 }
        }
    }

    impl Read for OneByteReader<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.offset == self.bytes.len() {
                return Ok(0);
            }
            output[0] = self.bytes[self.offset];
            self.offset += 1;
            Ok(1)
        }
    }
}
