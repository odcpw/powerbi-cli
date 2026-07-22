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

#[derive(Debug, Clone)]
struct StreamCapture {
    tail: String,
    sha256: String,
    total_bytes: u64,
    truncated: bool,
}

#[derive(Debug, Clone)]
struct MonitorReport {
    status: Option<ExitStatus>,
    forced: bool,
    tree_termination_attempted: bool,
    root_reaped: bool,
    captured_descendants: usize,
    descendants_gone: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct McpCleanupReport {
    pub(crate) children_reaped: bool,
    pub(crate) forced: bool,
    pub(crate) stderr_sha256: String,
    pub(crate) stderr_truncated: bool,
    stderr: StreamCapture,
    monitor: MonitorReport,
    pumps_joined: bool,
    join_failure: Option<PumpJoinFailure>,
}

#[derive(Debug)]
enum WriterCommand {
    Frame(Vec<u8>),
    Close,
}

#[derive(Debug)]
enum ReaderEvent {
    Frame(Vec<u8>),
    Failure(String),
    Eof,
}

#[derive(Debug, Clone, Copy)]
enum MonitorCommand {
    Graceful,
    Force,
}

#[derive(Debug, Clone)]
struct PumpJoinFailure {
    kind: McpFailureKind,
    message: String,
}

impl PumpJoinFailure {
    fn backend(message: impl Into<String>) -> Self {
        Self {
            kind: McpFailureKind::Backend,
            message: message.into(),
        }
    }

    fn panicked(label: &str) -> Self {
        Self {
            kind: McpFailureKind::Panicked,
            message: format!("MCP {label} pump join failed: worker thread panicked"),
        }
    }

    fn into_failure(self) -> McpFailure {
        match self.kind {
            McpFailureKind::Protocol => McpFailure::protocol(self.message),
            McpFailureKind::Backend => McpFailure::backend(self.message),
            McpFailureKind::Cancelled => McpFailure::cancelled(self.message),
            McpFailureKind::Panicked => McpFailure::panicked(self.message),
        }
    }
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
pub(crate) struct StagedPartitionReplacementRequest {
    pub(crate) source_root: PathBuf,
    pub(crate) staged_semantic_model_root: PathBuf,
    pub(crate) workflow_root: PathBuf,
    pub(crate) fresh_export_root: PathBuf,
    pub(crate) replacements: Vec<StagedPartitionReplacement>,
}

#[derive(Debug, Clone)]
pub(crate) struct StagedPartitionReplacement {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) expected_before_sha256: String,
    pub(crate) complete_m_expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartitionReplacementEvidence {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) before_sha256: String,
    pub(crate) requested_sha256: String,
    pub(crate) readback_sha256: String,
    pub(crate) materialized_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelCleanupEvidence {
    pub(crate) children_reaped: bool,
    pub(crate) pumps_joined: bool,
    pub(crate) forced: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StagedModelSuccess {
    pub(crate) replacements: Vec<PartitionReplacementEvidence>,
    pub(crate) export: ExportShapeProof,
    pub(crate) source: SourceTreeEvidence,
    pub(crate) stage_definition: SourceTreeEvidence,
    pub(crate) expected_stage_sha256: String,
    pub(crate) cleanup: ModelCleanupEvidence,
}

#[derive(Debug, Clone)]
pub(crate) struct StagedModelFailure {
    pub(crate) phase: &'static str,
    pub(crate) error: McpFailure,
}

#[derive(Debug, Clone)]
pub(crate) enum StagedModelResult {
    Succeeded(StagedModelSuccess),
    Failed(StagedModelFailure),
}

pub(crate) fn staged_partition_source_fingerprint(
    semantic_model_root: &Path,
    table: &str,
    partition: &str,
) -> Result<String, McpFailure> {
    let docs = load_table_documents_from_semantic_model(semantic_model_root)
        .map_err(|error| McpFailure::protocol(error.message))?;
    let selector = PartitionSelector {
        table: Some(checked_identifier(table, "table")?),
        name: Some(checked_identifier(partition, "partition")?),
        ..PartitionSelector::default()
    };
    let record =
        find_partition(&docs, &selector).map_err(|error| McpFailure::protocol(error.message))?;
    let source = record.source.as_deref().ok_or_else(|| {
        McpFailure::protocol(format!(
            "partition has no complete M source: {}.{}",
            table, partition
        ))
    })?;
    Ok(source_expression_sha256(source))
}

pub(crate) fn execute_staged_partition_replacements(
    tool: &InstalledMicrosoftTool,
    request: &StagedPartitionReplacementRequest,
    allow_model_write: bool,
) -> StagedModelResult {
    let prepared = match PreparedModelRun::new(request, allow_model_write) {
        Ok(prepared) => prepared,
        Err(failure) => return StagedModelResult::Failed(failure),
    };
    let mut session = match McpSession::open_exact(
        tool,
        McpSessionMode::ConfirmedWrite,
        McpSessionConfig::default(),
    ) {
        Ok(session) => session,
        Err(error) => {
            return finish_model_run(
                &prepared,
                CoreModelOutcome::Failed(CoreModelFailure::new("handshake", error)),
                None,
            );
        }
    };
    if let Err(error) = session.handshake() {
        let cleanup = session.shutdown(false);
        return finish_model_run(
            &prepared,
            CoreModelOutcome::Failed(CoreModelFailure::new("handshake", error)),
            Some(cleanup_evidence(&cleanup)),
        );
    }
    let core = run_prepared_model(&mut session, &prepared);
    let cancelled = core.failure_kind() == Some(McpFailureKind::Cancelled);
    let cleanup = session.shutdown(!cancelled && !session.poisoned);
    let cleanup = cleanup_evidence(&cleanup);
    let core = if cleanup.children_reaped && cleanup.pumps_joined {
        materialize_verified(&prepared, core)
    } else {
        core
    };
    finish_model_run(&prepared, core, Some(cleanup))
}

pub(crate) fn execute_staged_model_export_proof(
    tool: &InstalledMicrosoftTool,
    source_root: &Path,
    staged_semantic_model_root: &Path,
    scratch_root: &Path,
) -> Result<ExportShapeProof, McpFailure> {
    let reservation = PreparedStagedModel::prepare(
        source_root,
        staged_semantic_model_root,
        scratch_root,
        &scratch_root.join("canonical-export"),
    )
    .map_err(McpFailure::protocol)?;
    let prepared = reservation.commit();
    let mut session =
        McpSession::open_exact(tool, McpSessionMode::ReadOnly, McpSessionConfig::default())?;
    let proof = session.handshake().and_then(|_| {
        let connection = connect_exact(&mut session, &prepared.definition_dir)?;
        prepared
            .ensure_export_empty()
            .map_err(McpFailure::protocol)?;
        call_tool_payload(
            &mut session,
            &McpOperation::ExportTmdlFolder {
                connection_name: connection.name,
                folder_path: prepared.export_root.join("definition"),
            },
            "ExportToTmdlFolder",
        )?;
        prepared.validate_export().map_err(McpFailure::protocol)
    });
    let cleanup = session.shutdown(proof.is_ok() && !session.poisoned);
    if !cleanup.children_reaped || !cleanup.pumps_joined {
        let _ = prepared.mark_export_failure_only();
        return Err(McpFailure::backend(
            "canonical staged-model export proof cleanup was incomplete",
        )
        .with_cleanup(&cleanup));
    }
    match proof {
        Ok(proof) => {
            prepared
                .disarm_export_quarantine()
                .map_err(McpFailure::backend)?;
            Ok(proof)
        }
        Err(error) => {
            let _ = prepared.mark_export_failure_only();
            Err(error.with_cleanup(&cleanup))
        }
    }
}

trait ModelMcpClient {
    fn call_model(&mut self, operation: &McpOperation) -> Result<Value, McpFailure>;
}

impl ModelMcpClient for McpSession {
    fn call_model(&mut self, operation: &McpOperation) -> Result<Value, McpFailure> {
        self.call(operation)
    }
}

struct PreparedModelRun {
    paths: PreparedStagedModel,
    source_snapshot: SourceTreeSnapshot,
    stage_snapshot: SourceTreeSnapshot,
    expected_stage_sha256: String,
    replacements: Vec<PreparedReplacement>,
    native_plans: Vec<MutationPlan>,
}

struct PreparedReplacement {
    table: String,
    partition: String,
    before_sha256: String,
    requested_sha256: String,
    expression: String,
}

impl PreparedModelRun {
    #[allow(clippy::result_large_err)]
    fn new(
        request: &StagedPartitionReplacementRequest,
        allow_model_write: bool,
    ) -> Result<Self, StagedModelFailure> {
        if !allow_model_write {
            return Err(StagedModelFailure::unprepared(
                "consent",
                McpFailure::protocol(
                    "model writes require explicit --allow-model-write-equivalent consent",
                ),
            ));
        }
        if request.replacements.is_empty() || request.replacements.len() > 100 {
            return Err(StagedModelFailure::unprepared(
                "prepare",
                McpFailure::protocol(
                    "staged model writes require between 1 and 100 typed partition replacements",
                ),
            ));
        }
        if request.replacements.iter().any(|replacement| {
            contains_credential_like_text_str(&replacement.complete_m_expression)
        }) {
            return Err(StagedModelFailure::unprepared(
                "credential-scan",
                McpFailure::protocol(
                    "complete M expressions with credential-like text are forbidden at the staged MCP boundary",
                ),
            ));
        }
        let reservation = PreparedStagedModel::prepare(
            &request.source_root,
            &request.staged_semantic_model_root,
            &request.workflow_root,
            &request.fresh_export_root,
        )
        .map_err(|message| {
            StagedModelFailure::unprepared("paths", McpFailure::protocol(message))
        })?;
        let paths = reservation.paths();
        let source_snapshot =
            SourceTreeSnapshot::capture(&paths.source_root).map_err(|message| {
                StagedModelFailure::unprepared("source-proof", McpFailure::backend(message))
            })?;
        let stage_snapshot =
            SourceTreeSnapshot::capture(&paths.definition_dir).map_err(|message| {
                StagedModelFailure::unprepared("stage-proof", McpFailure::backend(message))
            })?;
        let docs = load_table_documents_from_semantic_model(&paths.semantic_model_root).map_err(
            |error| StagedModelFailure::unprepared("prepare", McpFailure::protocol(error.message)),
        )?;
        let mut handles = BTreeSet::new();
        let mut native_plans = BTreeMap::<PathBuf, MutationPlan>::new();
        let mut replacements = Vec::with_capacity(request.replacements.len());
        for replacement in &request.replacements {
            let table = checked_identifier(&replacement.table, "table")
                .map_err(|error| StagedModelFailure::unprepared("prepare", error))?;
            let partition = checked_identifier(&replacement.partition, "partition")
                .map_err(|error| StagedModelFailure::unprepared("prepare", error))?;
            let handle = format!("{table}\u{0}{partition}");
            if !handles.insert(handle) {
                return Err(StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol("duplicate typed partition replacement"),
                ));
            }
            let selector = PartitionSelector {
                table: Some(table.clone()),
                name: Some(partition.clone()),
                ..PartitionSelector::default()
            };
            let record = find_partition(&docs, &selector).map_err(|error| {
                StagedModelFailure::unprepared("prepare", McpFailure::protocol(error.message))
            })?;
            let before = record.source.as_deref().ok_or_else(|| {
                StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol(format!(
                        "partition has no complete M source: {table}.{partition}"
                    )),
                )
            })?;
            let before_sha256 = source_expression_sha256(before);
            if replacement.expected_before_sha256 != before_sha256 {
                return Err(StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol(format!(
                        "partition before fingerprint drift for {table}.{partition}"
                    )),
                ));
            }
            let expression = normalized_source_expression(&replacement.complete_m_expression)
                .ok_or_else(|| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::protocol("complete M expression must not be empty"),
                    )
                })?;
            let requested_sha256 = source_expression_sha256(&expression);
            let native_plan = replace_partition_source_plan(&docs, &selector, &expression)
                .map_err(|error| {
                    StagedModelFailure::unprepared("prepare", McpFailure::protocol(error.message))
                })?;
            let canonical_plan_path =
                std::fs::canonicalize(&native_plan.path).map_err(|error| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::backend(format!(
                            "resolve native partition write {}: {error}",
                            native_plan.path.display()
                        )),
                    )
                })?;
            if !canonical_plan_path.starts_with(&paths.definition_dir) {
                return Err(StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol("native partition write escaped the staged definition"),
                ));
            }
            if let Some(composed) = native_plans.get_mut(&canonical_plan_path) {
                let before = native_plan.before_block.as_deref().ok_or_else(|| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::backend("native partition plan has no before block"),
                    )
                })?;
                let after = native_plan.after_block.as_deref().ok_or_else(|| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::backend("native partition plan has no after block"),
                    )
                })?;
                let mut matches = composed.new_text.match_indices(before);
                let Some((start, _)) = matches.next() else {
                    return Err(StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::protocol(
                            "same-file partition replacements could not be composed exactly",
                        ),
                    ));
                };
                if matches.next().is_some() {
                    return Err(StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::protocol(
                            "same-file partition replacement is ambiguous in the original TMDL",
                        ),
                    ));
                }
                composed
                    .new_text
                    .replace_range(start..start + before.len(), after);
            } else {
                native_plans.insert(canonical_plan_path, native_plan);
            }
            replacements.push(PreparedReplacement {
                table,
                partition,
                before_sha256,
                requested_sha256,
                expression,
            });
        }
        let native_plans = native_plans.into_values().collect::<Vec<_>>();
        let expected_replacements = native_plans
            .iter()
            .map(|plan| (plan.path.clone(), plan.new_text.clone()))
            .collect::<Vec<_>>();
        let expected_stage_sha256 = stage_snapshot
            .expected_after_sha256(&expected_replacements)
            .map_err(|message| {
                StagedModelFailure::unprepared("stage-proof", McpFailure::backend(message))
            })?;
        Ok(Self {
            paths: reservation.commit(),
            source_snapshot,
            stage_snapshot,
            expected_stage_sha256,
            replacements,
            native_plans,
        })
    }
}

enum CoreModelOutcome {
    Verified(CoreModelSuccess),
    Materialized(CoreModelSuccess),
    Failed(CoreModelFailure),
}

impl CoreModelOutcome {
    fn failure_kind(&self) -> Option<McpFailureKind> {
        match self {
            Self::Verified(_) | Self::Materialized(_) => None,
            Self::Failed(failure) => Some(failure.error.kind()),
        }
    }
}

struct CoreModelSuccess {
    replacements: Vec<PartitionReplacementEvidence>,
    export: ExportShapeProof,
}

struct CoreModelFailure {
    phase: &'static str,
    error: McpFailure,
}

impl CoreModelFailure {
    fn new(phase: &'static str, error: McpFailure) -> Self {
        Self { phase, error }
    }
}

impl StagedModelFailure {
    fn unprepared(phase: &'static str, error: McpFailure) -> Self {
        Self { phase, error }
    }
}

fn run_prepared_model<C: ModelMcpClient>(
    client: &mut C,
    prepared: &PreparedModelRun,
) -> CoreModelOutcome {
    let connection = match connect_exact(client, &prepared.paths.definition_dir) {
        Ok(connection) => connection,
        Err(error) => {
            return CoreModelOutcome::Failed(CoreModelFailure::new("connection", error));
        }
    };
    let connection_name = connection.name.clone();
    for replacement in &prepared.replacements {
        if let Err(error) = call_tool_payload(
            client,
            &McpOperation::ReplacePartitionSource {
                connection_name: connection_name.clone(),
                table_name: replacement.table.clone(),
                partition_name: replacement.partition.clone(),
                expression: replacement.expression.clone(),
            },
            "Update",
        ) {
            return CoreModelOutcome::Failed(offline_failure(
                "write",
                sanitize_write_failure(error),
            ));
        }
    }
    let mut evidence = Vec::with_capacity(prepared.replacements.len());
    for replacement in &prepared.replacements {
        let readback = match call_tool_payload(
            client,
            &McpOperation::GetPartition {
                connection_name: connection_name.clone(),
                table_name: replacement.table.clone(),
                partition_name: replacement.partition.clone(),
            },
            "GET",
        )
        .and_then(|payload| {
            exact_partition_readback(&payload, &replacement.table, &replacement.partition)
        }) {
            Ok(readback) => readback,
            Err(error) => {
                return CoreModelOutcome::Failed(offline_failure("readback", error));
            }
        };
        let readback = match normalized_source_expression(&readback) {
            Some(readback) => readback,
            None => {
                return CoreModelOutcome::Failed(offline_failure(
                    "readback",
                    McpFailure::protocol("partition readback returned an empty expression"),
                ));
            }
        };
        let readback_sha256 = source_expression_sha256(&readback);
        if readback != replacement.expression || readback_sha256 != replacement.requested_sha256 {
            return CoreModelOutcome::Failed(offline_failure(
                "readback",
                McpFailure::protocol(format!(
                    "partition readback mismatch for {}.{}",
                    replacement.table, replacement.partition
                )),
            ));
        }
        evidence.push(PartitionReplacementEvidence {
            table: replacement.table.clone(),
            partition: replacement.partition.clone(),
            before_sha256: replacement.before_sha256.clone(),
            requested_sha256: replacement.requested_sha256.clone(),
            readback_sha256,
            materialized_sha256: String::new(),
        });
    }
    if let Err(message) = prepared.paths.ensure_export_empty() {
        return CoreModelOutcome::Failed(offline_failure(
            "export-guard",
            McpFailure::protocol(message),
        ));
    }
    if let Err(error) = call_tool_payload(
        client,
        &McpOperation::ExportTmdlFolder {
            connection_name,
            folder_path: prepared.paths.export_root.join("definition"),
        },
        "ExportToTmdlFolder",
    ) {
        return CoreModelOutcome::Failed(offline_failure("export", error));
    }
    let export = match prepared.paths.validate_export() {
        Ok(export) => export,
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "export-proof",
                McpFailure::protocol(message),
            ));
        }
    };
    match prepared.stage_snapshot.verify() {
        Ok(stage) if stage.byte_identical => {}
        Ok(_) => {
            return CoreModelOutcome::Failed(offline_failure(
                "stage-proof",
                McpFailure::protocol(
                    "staged definition changed before native readback materialization",
                ),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "stage-proof",
                McpFailure::backend(message),
            ));
        }
    }
    CoreModelOutcome::Verified(CoreModelSuccess {
        replacements: evidence,
        export,
    })
}

fn materialize_verified(prepared: &PreparedModelRun, core: CoreModelOutcome) -> CoreModelOutcome {
    let CoreModelOutcome::Verified(mut success) = core else {
        return core;
    };
    match prepared.source_snapshot.verify() {
        Ok(source) if source.byte_identical => {}
        Ok(_) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-source-proof",
                McpFailure::protocol(
                    "source project changed before native readback materialization",
                ),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-source-proof",
                McpFailure::backend(message),
            ));
        }
    }
    match prepared.stage_snapshot.verify() {
        Ok(stage) if stage.byte_identical => {}
        Ok(_) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-stage-proof",
                McpFailure::protocol(
                    "staged definition changed before isolated MCP cleanup completed",
                ),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-stage-proof",
                McpFailure::backend(message),
            ));
        }
    }
    for plan in &prepared.native_plans {
        if let Err(error) = write_text_atomic(&plan.path, &plan.new_text) {
            return CoreModelOutcome::Failed(offline_failure(
                "materialize",
                McpFailure::backend(error.message),
            ));
        }
    }
    for (replacement, replacement_evidence) in prepared
        .replacements
        .iter()
        .zip(success.replacements.iter_mut())
    {
        match staged_partition_source_fingerprint(
            &prepared.paths.semantic_model_root,
            &replacement.table,
            &replacement.partition,
        ) {
            Ok(materialized) if materialized == replacement.requested_sha256 => {
                replacement_evidence.materialized_sha256 = materialized;
            }
            Ok(_) => {
                return CoreModelOutcome::Failed(offline_failure(
                    "materialize",
                    McpFailure::protocol(format!(
                        "native materialization readback mismatch for {}.{}",
                        replacement.table, replacement.partition
                    )),
                ));
            }
            Err(error) => {
                return CoreModelOutcome::Failed(offline_failure("materialize", error));
            }
        }
    }
    match prepared.stage_snapshot.verify() {
        Ok(stage) if stage.after_sha256 == prepared.expected_stage_sha256 => {}
        Ok(stage) => {
            return CoreModelOutcome::Failed(offline_failure(
                "materialize-proof",
                McpFailure::protocol(format!(
                    "staged definition differs from the exact expected-after tree: expected {}, got {}",
                    prepared.expected_stage_sha256, stage.after_sha256
                )),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "materialize-proof",
                McpFailure::backend(message),
            ));
        }
    }
    CoreModelOutcome::Materialized(success)
}

fn offline_failure(phase: &'static str, error: McpFailure) -> CoreModelFailure {
    CoreModelFailure { phase, error }
}

fn sanitize_write_failure(error: McpFailure) -> McpFailure {
    McpFailure::new(
        error.kind(),
        format!(
            "Modeling MCP partition Update failed; vendorDetailSha256={}",
            sha256_bytes(error.message().as_bytes())
        ),
    )
}

fn finish_model_run(
    prepared: &PreparedModelRun,
    core: CoreModelOutcome,
    cleanup: Option<ModelCleanupEvidence>,
) -> StagedModelResult {
    let source = prepared.source_snapshot.verify();
    let stage = prepared.stage_snapshot.verify();
    let source_evidence = source.as_ref().ok().cloned();
    let stage_evidence = stage.as_ref().ok().cloned();
    if let Err(message) = source {
        let _ = prepared.paths.mark_export_failure_only();
        return StagedModelResult::Failed(StagedModelFailure {
            phase: "source-proof",
            error: McpFailure::backend(message),
        });
    }
    if source_evidence
        .as_ref()
        .is_some_and(|evidence| !evidence.byte_identical)
    {
        let _ = prepared.paths.mark_export_failure_only();
        return StagedModelResult::Failed(StagedModelFailure {
            phase: "source-proof",
            error: McpFailure::protocol("source project changed during staged model workflow"),
        });
    }
    if cleanup
        .as_ref()
        .is_some_and(|cleanup| !cleanup.children_reaped || !cleanup.pumps_joined)
    {
        let _ = prepared.paths.mark_export_failure_only();
        return StagedModelResult::Failed(StagedModelFailure {
            phase: "cleanup",
            error: McpFailure::backend("Modeling MCP cleanup was incomplete"),
        });
    }
    match core {
        CoreModelOutcome::Materialized(success) => {
            let Some(cleanup) = cleanup else {
                let _ = prepared.paths.mark_export_failure_only();
                return StagedModelResult::Failed(StagedModelFailure {
                    phase: "cleanup",
                    error: McpFailure::backend(
                        "successful isolated MCP workflow requires cleanup evidence",
                    ),
                });
            };
            if let Err(message) = prepared.paths.disarm_export_quarantine() {
                let _ = prepared.paths.mark_export_failure_only();
                return StagedModelResult::Failed(StagedModelFailure {
                    phase: "export-quarantine",
                    error: McpFailure::backend(message),
                });
            }
            StagedModelResult::Succeeded(StagedModelSuccess {
                replacements: success.replacements,
                export: success.export,
                source: source_evidence.expect("source proof captured for prepared run"),
                stage_definition: stage_evidence.expect("stage proof captured for prepared run"),
                expected_stage_sha256: prepared.expected_stage_sha256.clone(),
                cleanup,
            })
        }
        CoreModelOutcome::Verified(_success) => {
            let _ = prepared.paths.mark_export_failure_only();
            StagedModelResult::Failed(StagedModelFailure {
                phase: "cleanup",
                error: McpFailure::backend(
                    "isolated MCP result was not materialized after complete cleanup",
                ),
            })
        }
        CoreModelOutcome::Failed(failure) => {
            let _ = prepared.paths.mark_export_failure_only();
            StagedModelResult::Failed(StagedModelFailure {
                phase: failure.phase,
                error: failure.error,
            })
        }
    }
}

fn cleanup_evidence(cleanup: &McpCleanupReport) -> ModelCleanupEvidence {
    ModelCleanupEvidence {
        children_reaped: cleanup.children_reaped,
        pumps_joined: cleanup.pumps_joined,
        forced: cleanup.forced,
    }
}

struct ExactConnection {
    name: String,
}

fn connect_exact<C: ModelMcpClient>(
    client: &mut C,
    definition_dir: &Path,
) -> Result<ExactConnection, McpFailure> {
    let connected = call_tool_payload(
        client,
        &McpOperation::ConnectFolder {
            folder_path: definition_dir.to_path_buf(),
        },
        "ConnectFolder",
    )?;
    let data = payload_data_object(&connected)?;
    let connection_name = required_object_string(data, "connectionName", "connection data")?;
    let connected_path = required_object_string(data, "folderPath", "connection data")?;
    require_exact_canonical_path(&connected_path, definition_dir, "connected folder")?;
    let listed = call_tool_payload(client, &McpOperation::ListConnections, "ListConnections")?;
    let entries = listed
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("ListConnections has no data array"))?;
    let mut exact = 0_usize;
    let mut exact_name = None;
    for entry in entries {
        let Some(object) = entry.as_object() else {
            return Err(McpFailure::protocol(
                "ListConnections contains a non-object entry",
            ));
        };
        let listed_name =
            required_object_string(object, "connectionName", "connection list entry")?;
        let source_path = required_object_string(object, "sourcePath", "connection list entry")?;
        if require_exact_canonical_path(&source_path, definition_dir, "listed folder").is_ok() {
            exact = exact.saturating_add(1);
            exact_name = Some(listed_name);
        }
    }
    if exact != 1 || exact_name.as_deref() != Some(connection_name.as_str()) {
        return Err(McpFailure::protocol(format!(
            "expected exactly one globally unique staged folder path with the ConnectFolder identity, found exact={exact}"
        )));
    }
    Ok(ExactConnection {
        name: connection_name,
    })
}

fn require_exact_canonical_path(
    reported: &str,
    expected: &Path,
    label: &str,
) -> Result<(), McpFailure> {
    let reported = std::fs::canonicalize(reported)
        .map_err(|error| McpFailure::protocol(format!("resolve {label} {reported}: {error}")))?;
    if reported != expected {
        return Err(McpFailure::protocol(format!(
            "{label} does not match the exact staged definition"
        )));
    }
    Ok(())
}

fn call_tool_payload<C: ModelMcpClient>(
    client: &mut C,
    operation: &McpOperation,
    expected_operation: &str,
) -> Result<Value, McpFailure> {
    let result = client.call_model(operation)?;
    parse_tool_payload(&result, expected_operation)
}

fn parse_tool_payload(result: &Value, expected_operation: &str) -> Result<Value, McpFailure> {
    let result = exact_object(result, &["_meta", "content", "isError"], "MCP tool result")?;
    validate_tool_result_metadata(result, expected_operation)?;
    match result.get("isError").and_then(Value::as_bool) {
        Some(false) => {}
        Some(true) => {
            return Err(McpFailure::backend(format!(
                "MCP tool {expected_operation} returned an error result"
            )));
        }
        None => {
            return Err(McpFailure::protocol(
                "MCP tool result isError must be exactly one boolean",
            ));
        }
    }
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("MCP tool result has no content array"))?;
    if content.len() != 1 {
        return Err(McpFailure::protocol(
            "MCP tool result must contain exactly one text payload",
        ));
    }
    let content = content[0]
        .as_object()
        .ok_or_else(|| McpFailure::protocol("MCP tool content is not one object"))?;
    exact_keys(content, &["text", "type"], "MCP tool text content")?;
    if content.get("type").and_then(Value::as_str) != Some("text") {
        return Err(McpFailure::protocol(
            "MCP tool content is not the expected text payload",
        ));
    }
    let text = content
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| McpFailure::protocol("MCP tool content has no text"))?;
    if text.len() > DEFAULT_TOTAL_RESPONSE_LIMIT {
        return Err(McpFailure::protocol(
            "MCP tool text payload exceeds the bounded response cap",
        ));
    }
    let payload: Value = serde_json::from_str(text)
        .map_err(|error| McpFailure::protocol(format!("parse MCP tool payload: {error}")))?;
    if expected_operation == "Update" {
        let object = payload.as_object().ok_or_else(|| {
            McpFailure::protocol("partition Update payload is not the exact empty object")
        })?;
        if !object.is_empty() {
            return Err(McpFailure::protocol(
                "partition Update payload is not the exact empty object",
            ));
        }
        return Ok(payload);
    }
    let operation = payload
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| McpFailure::protocol("MCP tool payload has no operation"))?;
    if operation != expected_operation {
        return Err(McpFailure::protocol(format!(
            "MCP tool payload operation drift: expected {expected_operation}, got {operation}"
        )));
    }
    Ok(payload)
}

fn validate_tool_result_metadata(
    result: &Map<String, Value>,
    expected_operation: &str,
) -> Result<(), McpFailure> {
    let metadata = exact_object(
        result
            .get("_meta")
            .ok_or_else(|| McpFailure::protocol("MCP tool result has no _meta object"))?,
        &["annotations"],
        "MCP tool result _meta",
    )?;
    let annotations = exact_object(
        metadata
            .get("annotations")
            .ok_or_else(|| McpFailure::protocol("MCP tool result has no annotations object"))?,
        &["readOnlyHint", "title"],
        "MCP tool result annotations",
    )?;
    let (expected_title, expected_read_only) = match expected_operation {
        "ConnectFolder" => ("connection_operations.connectfolder", true),
        "ListConnections" => ("connection_operations.listconnections", true),
        "GET" => ("partition_operations.get", true),
        "Update" => ("partition_operations.update", false),
        "ExportToTmdlFolder" => ("database_operations.exporttotmdlfolder", true),
        _ => {
            return Err(McpFailure::protocol(format!(
                "MCP tool metadata has no closed policy for {expected_operation}"
            )));
        }
    };
    if annotations.get("title").and_then(Value::as_str) != Some(expected_title)
        || annotations.get("readOnlyHint").and_then(Value::as_bool) != Some(expected_read_only)
    {
        return Err(McpFailure::protocol(format!(
            "MCP tool result annotations drifted from the exact {expected_operation} contract"
        )));
    }
    Ok(())
}

fn payload_data_object(payload: &Value) -> Result<&Map<String, Value>, McpFailure> {
    payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| McpFailure::protocol("MCP tool payload has no data object"))
}

fn exact_partition_readback(
    payload: &Value,
    table: &str,
    partition: &str,
) -> Result<String, McpFailure> {
    let payload = exact_object(
        payload,
        &["message", "operation", "results", "summary", "warnings"],
        "partition Get payload",
    )?;
    if payload.get("operation").and_then(Value::as_str) != Some("GET") {
        return Err(McpFailure::protocol(
            "partition Get payload operation must be exactly GET",
        ));
    }
    required_object_string(payload, "message", "partition Get payload")?;
    let summary = exact_object(
        payload
            .get("summary")
            .ok_or_else(|| McpFailure::protocol("partition Get payload has no summary"))?,
        &[
            "executionTime",
            "failureCount",
            "successCount",
            "totalItems",
        ],
        "partition Get summary",
    )?;
    let execution_time = required_object_string(summary, "executionTime", "partition Get summary")?;
    if execution_time.is_empty()
        || summary.get("failureCount").and_then(Value::as_u64) != Some(0)
        || summary.get("successCount").and_then(Value::as_u64) != Some(1)
        || summary.get("totalItems").and_then(Value::as_u64) != Some(1)
    {
        return Err(McpFailure::protocol(
            "partition Get summary does not prove exactly one successful readback",
        ));
    }
    let warnings = payload
        .get("warnings")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("partition Get warnings must be one array"))?;
    if !warnings.is_empty() {
        return Err(McpFailure::protocol(
            "partition Get returned warnings and cannot be trusted as exact readback",
        ));
    }
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("partition Get has no results array"))?;
    let [result] = results.as_slice() else {
        return Err(McpFailure::protocol(format!(
            "partition Get returned {} results for {table}.{partition}; exactly one is required",
            results.len()
        )));
    };
    let result = exact_object(
        result,
        &["data", "index", "itemIdentifier", "message", "warnings"],
        "partition Get result",
    )?;
    if result.get("index").and_then(Value::as_u64) != Some(0)
        || required_object_string(result, "itemIdentifier", "partition Get result")?.is_empty()
        || required_object_string(result, "message", "partition Get result")?.is_empty()
        || !result
            .get("warnings")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    {
        return Err(McpFailure::protocol(
            "partition Get result metadata does not prove one warning-free item",
        ));
    }
    let data = result
        .get("data")
        .ok_or_else(|| McpFailure::protocol("partition Get result has no data"))?;
    let data = exact_object(
        data,
        &[
            "annotations",
            "attributes",
            "dataView",
            "description",
            "errorMessage",
            "expression",
            "extendedProperties",
            "mode",
            "modifiedTime",
            "name",
            "sourceType",
            "state",
            "tableName",
        ],
        "partition Get data",
    )?;
    for field in ["annotations", "extendedProperties"] {
        if data.get(field).and_then(Value::as_array).is_none() {
            return Err(McpFailure::protocol(format!(
                "partition Get data field {field} must be one array"
            )));
        }
    }
    for field in [
        "attributes",
        "dataView",
        "description",
        "errorMessage",
        "mode",
        "modifiedTime",
        "state",
    ] {
        required_object_string(data, field, "partition Get data")?;
    }
    if data.get("errorMessage").and_then(Value::as_str) != Some("")
        || data.get("mode").and_then(Value::as_str) == Some("")
        || data.get("state").and_then(Value::as_str) == Some("")
    {
        return Err(McpFailure::protocol(
            "partition Get data is not a successful materialized partition",
        ));
    }
    if data.get("tableName").and_then(Value::as_str) != Some(table)
        || data.get("name").and_then(Value::as_str) != Some(partition)
    {
        return Err(McpFailure::protocol(format!(
            "partition Get did not return the exact requested identity {table}.{partition}"
        )));
    }
    if data.get("sourceType").and_then(Value::as_str) != Some("M") {
        return Err(McpFailure::protocol(
            "partition Get readback is not an M source",
        ));
    }
    required_object_string(data, "expression", "partition Get data")
}

fn normalized_source_expression(value: &str) -> Option<String> {
    let value = value
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let value = value.trim_matches('\n');
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn source_expression_sha256(value: &str) -> String {
    normalized_source_expression(value)
        .map_or_else(|| sha256_bytes(b""), |value| sha256_bytes(value.as_bytes()))
}

#[derive(Debug, Clone)]
pub(crate) enum McpOperation {
    ListConnections,
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
            Self::ListConnections | Self::ConnectFolder { .. } => "connection_operations",
            Self::GetPartition { .. } | Self::ReplacePartitionSource { .. } => {
                "partition_operations"
            }
            Self::ExportTmdlFolder { .. } => "database_operations",
        }
    }

    fn arguments(&self) -> Result<Value, McpFailure> {
        match self {
            Self::ListConnections => Ok(json!({"request": {"operation": "ListConnections"}})),
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

fn writer_pump(mut stdin: ChildStdin, receiver: Receiver<WriterCommand>) -> Result<(), String> {
    while let Ok(command) = receiver.recv() {
        match command {
            WriterCommand::Frame(frame) => {
                stdin
                    .write_all(&frame)
                    .and_then(|_| stdin.flush())
                    .map_err(|error| format!("write MCP stdin: {error}"))?;
            }
            WriterCommand::Close => break,
        }
    }
    drop(stdin);
    Ok(())
}

fn reader_pump(
    mut stdout: ChildStdout,
    sender: SyncSender<ReaderEvent>,
    frame_limit: usize,
    total_limit: usize,
) -> Result<(), String> {
    read_frames(&mut stdout, &sender, frame_limit, total_limit).map(|_| ())
}

fn read_frames(
    reader: &mut dyn Read,
    sender: &SyncSender<ReaderEvent>,
    frame_limit: usize,
    total_limit: usize,
) -> Result<usize, String> {
    let mut buffer = [0_u8; 8192];
    let mut frame = Vec::with_capacity(frame_limit.min(8192));
    let mut total = 0_usize;
    let mut frames = 0_usize;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("read MCP stdout: {error}"))?;
        if count == 0 {
            if !frame.is_empty() {
                deliver_reader_event(
                    sender,
                    ReaderEvent::Failure("MCP stdout ended with a partial frame".to_string()),
                )?;
            } else {
                deliver_reader_event(sender, ReaderEvent::Eof)?;
            }
            return Ok(frames);
        }
        total = total.saturating_add(count);
        if total > total_limit {
            let message = format!("MCP stdout exceeds the {total_limit}-byte session cap");
            let _ = sender.try_send(ReaderEvent::Failure(message.clone()));
            return Err(message);
        }
        for byte in &buffer[..count] {
            if *byte == b'\n' {
                if !frame.is_empty() {
                    let completed = std::mem::take(&mut frame);
                    deliver_reader_event(sender, ReaderEvent::Frame(completed))?;
                    frames = frames.saturating_add(1);
                }
            } else if frame.len() == frame_limit {
                let message = format!("MCP frame exceeds the {frame_limit}-byte cap");
                let _ = sender.try_send(ReaderEvent::Failure(message.clone()));
                return Err(message);
            } else {
                frame.push(*byte);
            }
        }
    }
}

fn deliver_reader_event(
    sender: &SyncSender<ReaderEvent>,
    event: ReaderEvent,
) -> Result<(), String> {
    sender.try_send(event).map_err(|error| match error {
        TrySendError::Full(_) => {
            "bounded MCP response queue saturated; reader stopped fail-closed".to_string()
        }
        TrySendError::Disconnected(_) => "MCP response receiver was dropped".to_string(),
    })
}

fn stderr_pump(mut stderr: ChildStderr, limit: usize) -> Result<StreamCapture, String> {
    capture_stderr(&mut stderr, limit)
}

fn capture_stderr(reader: &mut dyn Read, limit: usize) -> Result<StreamCapture, String> {
    let mut tail = VecDeque::with_capacity(limit);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8192];
    let mut total = 0_u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("read MCP stderr: {error}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        total = total.saturating_add(count as u64);
        for byte in &buffer[..count] {
            if tail.len() == limit {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    let bytes = tail.into_iter().collect::<Vec<_>>();
    Ok(StreamCapture {
        tail: redact_vendor_text(&String::from_utf8_lossy(&bytes)),
        sha256: format!("sha256:{}", hex_digest(digest.finalize().as_slice())),
        total_bytes: total,
        truncated: total > limit as u64,
    })
}

struct ChildGuard {
    child: Option<GroupChild>,
    armed: bool,
}

impl ChildGuard {
    fn new(child: GroupChild) -> Self {
        Self {
            child: Some(child),
            armed: true,
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.armed
            && let Some(child) = self.child.as_mut()
        {
            let _ = terminate_child_tree(child);
        }
    }
}

fn monitor_pump(
    mut owned_child: ChildGuard,
    receiver: Receiver<MonitorCommand>,
    session_timeout: Duration,
    cleanup_timeout: Duration,
) -> MonitorReport {
    let Some(child) = owned_child.child.as_mut() else {
        return MonitorReport {
            status: None,
            forced: true,
            tree_termination_attempted: false,
            root_reaped: false,
            captured_descendants: 0,
            descendants_gone: false,
        };
    };
    let root = Pid::from_u32(child.id());
    let mut system = System::new();
    let mut descendants = BTreeSet::new();
    let started = Instant::now();
    let mut shutdown_started = None;
    let mut force = false;
    loop {
        capture_descendant_identities(&mut system, root, &mut descendants);
        match child.try_wait() {
            Ok(Some(status)) => {
                let termination = terminate_exited_child_group(child, status).ok();
                let root_reaped = termination.is_some();
                let descendants_gone = termination.is_some()
                    && terminate_captured_descendants(&descendants, cleanup_timeout);
                owned_child.armed = !root_reaped;
                return MonitorReport {
                    status: termination.map(|termination| termination.status),
                    forced: force,
                    tree_termination_attempted: true,
                    root_reaped,
                    captured_descendants: descendants.len(),
                    descendants_gone,
                };
            }
            Ok(None) => {}
            Err(_) => {
                return MonitorReport {
                    status: None,
                    forced: force,
                    tree_termination_attempted: false,
                    root_reaped: false,
                    captured_descendants: descendants.len(),
                    descendants_gone: false,
                };
            }
        }
        match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(MonitorCommand::Graceful) => {
                shutdown_started.get_or_insert_with(Instant::now);
            }
            Ok(MonitorCommand::Force) | Err(RecvTimeoutError::Disconnected) => {
                force = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if started.elapsed() >= session_timeout {
            force = true;
        }
        if shutdown_started.is_some_and(|value| value.elapsed() >= cleanup_timeout) {
            force = true;
        }
        if force {
            capture_descendant_identities(&mut system, root, &mut descendants);
            let termination = terminate_child_tree(child).ok();
            let root_reaped = termination.is_some();
            let descendants_gone = termination.is_some()
                && terminate_captured_descendants(&descendants, cleanup_timeout);
            owned_child.armed = !root_reaped;
            return MonitorReport {
                status: termination.as_ref().map(|termination| termination.status),
                forced: true,
                tree_termination_attempted: true,
                root_reaped,
                captured_descendants: descendants.len(),
                descendants_gone,
            };
        }
    }
}

struct TreeTermination {
    status: ExitStatus,
}

fn terminate_child_tree(child: &mut GroupChild) -> io::Result<TreeTermination> {
    let status = terminate_and_wait(child)?;
    Ok(TreeTermination { status })
}

fn terminate_exited_child_group(
    child: &mut GroupChild,
    status: ExitStatus,
) -> io::Result<TreeTermination> {
    let status = terminate_after_exit(child, status)?;
    Ok(TreeTermination { status })
}

fn capture_descendant_identities(
    system: &mut System,
    root: Pid,
    output: &mut BTreeSet<(u32, u64)>,
) {
    refresh_process_tree(system);
    let mut pids = Vec::new();
    collect_descendants(system, root, &mut pids);
    for pid in pids {
        if let Some(process) = system.process(pid) {
            output.insert((pid.as_u32(), process.start_time()));
        }
    }
}

fn signal_captured_descendants(system: &System, descendants: &BTreeSet<(u32, u64)>) {
    for (pid, started) in descendants.iter().rev() {
        if let Some(process) = system.process(Pid::from_u32(*pid))
            && process.start_time() == *started
        {
            let _ = process.kill_with(Signal::Kill);
        }
    }
}

fn terminate_captured_descendants(
    descendants: &BTreeSet<(u32, u64)>,
    cleanup_timeout: Duration,
) -> bool {
    if descendants.is_empty() {
        return true;
    }
    let started = Instant::now();
    let mut system = System::new();
    loop {
        refresh_process_tree(&mut system);
        signal_captured_descendants(&system, descendants);
        let alive = descendants.iter().any(|(pid, process_started)| {
            system
                .process(Pid::from_u32(*pid))
                .is_some_and(|process| process.start_time() == *process_started)
        });
        if !alive {
            return true;
        }
        if started.elapsed() >= cleanup_timeout {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn refresh_process_tree(system: &mut System) {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
}

fn collect_descendants(system: &System, parent: Pid, output: &mut Vec<Pid>) {
    let children = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| (process.parent() == Some(parent)).then_some(*pid))
        .collect::<Vec<_>>();
    for child in children {
        output.push(child);
        collect_descendants(system, child, output);
    }
}

fn join_pump<T>(
    handle: &mut Option<JoinHandle<Result<T, String>>>,
    label: &str,
) -> Result<T, PumpJoinFailure> {
    let Some(handle) = handle.take() else {
        return Err(PumpJoinFailure::backend(format!(
            "MCP {label} pump handle is missing"
        )));
    };
    match handle.join() {
        Ok(result) => result.map_err(PumpJoinFailure::backend),
        Err(_) => Err(PumpJoinFailure::panicked(label)),
    }
}

fn join_monitor(
    handle: &mut Option<JoinHandle<MonitorReport>>,
) -> Result<MonitorReport, PumpJoinFailure> {
    let Some(handle) = handle.take() else {
        return Err(PumpJoinFailure::backend("MCP monitor handle is missing"));
    };
    handle
        .join()
        .map_err(|_| PumpJoinFailure::panicked("monitor"))
}

fn join_stderr(
    handle: &mut Option<JoinHandle<Result<StreamCapture, String>>>,
) -> Result<StreamCapture, PumpJoinFailure> {
    join_pump(handle, "stderr")
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

    #[derive(Clone, Copy)]
    enum FakeExportShape {
        Valid,
        RootTmdl,
    }

    struct FakeModelClient {
        definition_dir: PathBuf,
        calls: Vec<&'static str>,
        expressions: std::collections::BTreeMap<(String, String), String>,
        fail_at: Option<&'static str>,
        fail_kind: McpFailureKind,
        failure_message: Option<String>,
        readback_mismatch: bool,
        duplicate_connection: bool,
        duplicate_path_different_name: bool,
        export_shape: FakeExportShape,
    }

    impl FakeModelClient {
        fn new(definition_dir: PathBuf) -> Self {
            Self {
                definition_dir,
                calls: Vec::new(),
                expressions: std::collections::BTreeMap::new(),
                fail_at: None,
                fail_kind: McpFailureKind::Backend,
                failure_message: None,
                readback_mismatch: false,
                duplicate_connection: false,
                duplicate_path_different_name: false,
                export_shape: FakeExportShape::Valid,
            }
        }

        fn enter(&mut self, label: &'static str) -> Result<(), McpFailure> {
            self.calls.push(label);
            if self.fail_at == Some(label) {
                return Err(McpFailure::new(
                    self.fail_kind,
                    self.failure_message
                        .clone()
                        .unwrap_or_else(|| format!("injected {label} failure")),
                ));
            }
            Ok(())
        }
    }

    impl ModelMcpClient for FakeModelClient {
        fn call_model(&mut self, operation: &McpOperation) -> Result<Value, McpFailure> {
            match operation {
                McpOperation::ConnectFolder { folder_path } => {
                    self.enter("connect")?;
                    assert_eq!(folder_path, &self.definition_dir);
                    Ok(fake_tool_result(json!({
                        "operation": "ConnectFolder",
                        "data": {
                            "connectionName": "fixture-connection",
                            "folderPath": self.definition_dir
                        }
                    })))
                }
                McpOperation::ListConnections => {
                    self.enter("list")?;
                    let mut connections = vec![json!({
                        "connectionName": "fixture-connection",
                        "sourcePath": self.definition_dir
                    })];
                    if self.duplicate_connection {
                        connections.push(json!({
                            "connectionName": "fixture-connection",
                            "sourcePath": self.definition_dir
                        }));
                    }
                    if self.duplicate_path_different_name {
                        connections.push(json!({
                            "connectionName": "different-connection",
                            "sourcePath": self.definition_dir
                        }));
                    }
                    Ok(fake_tool_result(json!({
                        "operation": "ListConnections",
                        "data": connections
                    })))
                }
                McpOperation::ReplacePartitionSource {
                    table_name,
                    partition_name,
                    expression,
                    ..
                } => {
                    self.enter("update")?;
                    self.expressions.insert(
                        (table_name.clone(), partition_name.clone()),
                        expression.clone(),
                    );
                    Ok(fake_tool_result(json!({})))
                }
                McpOperation::GetPartition {
                    table_name,
                    partition_name,
                    ..
                } => {
                    self.enter("get")?;
                    let expression = if self.readback_mismatch {
                        "let\n\tSource = #table({}, {})\nin\n\tSource".to_string()
                    } else {
                        self.expressions
                            .get(&(table_name.clone(), partition_name.clone()))
                            .expect("updated fake expression")
                            .clone()
                    };
                    Ok(fake_tool_result(json!({
                        "operation": "GET",
                        "message": "Retrieved one partition",
                        "results": [{
                            "index": 0,
                            "itemIdentifier": format!("{table_name}.{partition_name}"),
                            "message": "Retrieved partition",
                            "warnings": [],
                            "data": {
                                "annotations": [],
                                "attributes": "",
                                "dataView": "",
                                "description": "",
                                "errorMessage": "",
                                "name": partition_name,
                                "tableName": table_name,
                                "sourceType": "M",
                                "expression": expression,
                                "extendedProperties": [],
                                "mode": "import",
                                "modifiedTime": "2026-07-17T00:00:00Z",
                                "state": "Ready"
                            }
                        }],
                        "summary": {
                            "executionTime": "00:00:00.001",
                            "failureCount": 0,
                            "successCount": 1,
                            "totalItems": 1
                        },
                        "warnings": []
                    })))
                }
                McpOperation::ExportTmdlFolder { folder_path, .. } => {
                    self.enter("export")?;
                    match self.export_shape {
                        FakeExportShape::Valid => write_fake_tmdl_folder(folder_path),
                        FakeExportShape::RootTmdl => {
                            std::fs::write(folder_path.join("database.tmdl"), "database Unsafe")
                                .expect("root TMDL");
                        }
                    }
                    Ok(fake_tool_result(json!({
                        "operation": "ExportToTmdlFolder",
                        "data": {}
                    })))
                }
            }
        }
    }

    fn fake_tool_result(payload: Value) -> Value {
        let operation = payload
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or("Update");
        let (title, read_only) = match operation {
            "ConnectFolder" => ("connection_operations.connectfolder", true),
            "ListConnections" => ("connection_operations.listconnections", true),
            "GET" | "Get" => ("partition_operations.get", true),
            "Update" => ("partition_operations.update", false),
            "ExportToTmdlFolder" => ("database_operations.exporttotmdlfolder", true),
            unexpected => panic!("fake tool payload has unsupported operation {unexpected}"),
        };
        json!({
            "_meta": {
                "annotations": {
                    "readOnlyHint": read_only,
                    "title": title
                }
            },
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&payload).expect("serialize fake payload")
            }],
            "isError": false
        })
    }

    #[test]
    fn update_response_requires_the_exact_pinned_empty_object() {
        assert_eq!(
            parse_tool_payload(&fake_tool_result(json!({})), "Update")
                .expect("exact beta.11 Update response"),
            json!({})
        );
        for drifted in [
            json!([]),
            json!({"operation": "Update"}),
            json!({"data": {}}),
        ] {
            assert!(parse_tool_payload(&fake_tool_result(drifted), "Update").is_err());
        }
        let mut extra_result_key = fake_tool_result(json!({}));
        extra_result_key["structuredContent"] = json!({});
        assert!(parse_tool_payload(&extra_result_key, "Update").is_err());
        let mut extra_content_key = fake_tool_result(json!({}));
        extra_content_key["content"][0]["annotations"] = json!({});
        assert!(parse_tool_payload(&extra_content_key, "Update").is_err());
        let mut missing_error_flag = fake_tool_result(json!({}));
        missing_error_flag
            .as_object_mut()
            .expect("tool result")
            .remove("isError");
        assert!(parse_tool_payload(&missing_error_flag, "Update").is_err());
        let mut missing_metadata = fake_tool_result(json!({}));
        missing_metadata
            .as_object_mut()
            .expect("tool result")
            .remove("_meta");
        assert!(parse_tool_payload(&missing_metadata, "Update").is_err());
        let mut extra_annotation = fake_tool_result(json!({}));
        extra_annotation["_meta"]["annotations"]["destructiveHint"] = json!(false);
        assert!(parse_tool_payload(&extra_annotation, "Update").is_err());
        let mut wrong_annotation = fake_tool_result(json!({}));
        wrong_annotation["_meta"]["annotations"]["readOnlyHint"] = json!(true);
        assert!(parse_tool_payload(&wrong_annotation, "Update").is_err());
    }

    #[test]
    fn partition_readback_requires_the_exact_closed_shape() {
        let valid = json!({
            "operation": "GET",
            "message": "Retrieved one partition",
            "results": [{
                "index": 0,
                "itemIdentifier": "Fact.Fact",
                "message": "Retrieved partition",
                "warnings": [],
                "data": {
                    "annotations": [],
                    "attributes": "",
                    "dataView": "",
                    "description": "",
                    "errorMessage": "",
                    "name": "Fact",
                    "tableName": "Fact",
                    "sourceType": "M",
                    "expression": "let Source = 1 in Source",
                    "extendedProperties": [],
                    "mode": "import",
                    "modifiedTime": "2026-07-17T00:00:00Z",
                    "state": "Ready"
                }
            }],
            "summary": {
                "executionTime": "00:00:00.001",
                "failureCount": 0,
                "successCount": 1,
                "totalItems": 1
            },
            "warnings": []
        });
        assert_eq!(
            exact_partition_readback(&valid, "Fact", "Fact").expect("closed Get response"),
            "let Source = 1 in Source"
        );

        let mut extra_result = valid.clone();
        extra_result["results"]
            .as_array_mut()
            .expect("results")
            .push(valid["results"][0].clone());
        let mut extra_payload_field = valid.clone();
        extra_payload_field["unexpected"] = json!(true);
        let mut extra_result_field = valid.clone();
        extra_result_field["results"][0]["unexpected"] = json!(true);
        let mut extra_data_field = valid.clone();
        extra_data_field["results"][0]["data"]["unexpected"] = json!(true);
        let mut extra_summary_field = valid.clone();
        extra_summary_field["summary"]["unexpected"] = json!(true);
        let mut warning = valid.clone();
        warning["warnings"] = json!(["drift"]);
        let mut wrong_count = valid.clone();
        wrong_count["summary"]["totalItems"] = json!(2);
        let mut wrong_operation_casing = valid.clone();
        wrong_operation_casing["operation"] = json!("Get");
        for drifted in [
            extra_result,
            extra_payload_field,
            extra_result_field,
            extra_data_field,
            extra_summary_field,
            warning,
            wrong_count,
            wrong_operation_casing,
        ] {
            assert!(exact_partition_readback(&drifted, "Fact", "Fact").is_err());
        }
    }

    fn write_fake_tmdl_folder(definition: &Path) {
        assert!(definition.is_dir(), "ordinary export definition target");
        std::fs::create_dir(definition.join("tables")).expect("export tables");
        std::fs::write(
            definition.join("database.tmdl"),
            "database Synthetic\n\tcompatibilityLevel: 1600\n",
        )
        .expect("export database");
        std::fs::write(
            definition.join("model.tmdl"),
            "model Model\n\tculture: en-US\n",
        )
        .expect("export model");
        std::fs::write(
            definition.join("tables").join("Synthetic.tmdl"),
            "table Synthetic\n",
        )
        .expect("export table");
    }

    fn copy_model_fixture(target: &Path) {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("conformance")
            .join("microsoft")
            .join("modeling-mcp")
            .join("Synthetic.SemanticModel");
        std::fs::create_dir_all(target.join("definition").join("tables"))
            .expect("model fixture directories");
        for relative in [
            Path::new("definition.pbism"),
            Path::new("definition").join("database.tmdl").as_path(),
            Path::new("definition").join("model.tmdl").as_path(),
            Path::new("definition")
                .join("tables")
                .join("Synthetic.tmdl")
                .as_path(),
        ] {
            let target_file = target.join(relative);
            if let Some(parent) = target_file.parent() {
                std::fs::create_dir_all(parent).expect("fixture parent");
            }
            std::fs::copy(fixture.join(relative), target_file).expect("copy model fixture");
        }
    }

    fn staged_request(temp: &tempfile::TempDir) -> StagedPartitionReplacementRequest {
        let source = temp.path().join("source");
        let stage = temp.path().join("stage").join("Synthetic.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_model_fixture(&source);
        copy_model_fixture(&stage);
        std::fs::create_dir(&workflow).expect("workflow directory");
        let expected = staged_partition_source_fingerprint(&stage, "Synthetic", "Synthetic")
            .expect("before fingerprint");
        StagedPartitionReplacementRequest {
            source_root: source,
            staged_semantic_model_root: stage,
            fresh_export_root: workflow.join("mcp-export"),
            workflow_root: workflow,
            replacements: vec![StagedPartitionReplacement {
                table: "Synthetic".to_string(),
                partition: "Synthetic".to_string(),
                expected_before_sha256: expected,
                complete_m_expression:
                    "let\n\tSource = #table(type table [Value = Int64.Type], {{2}})\nin\n\tSource"
                        .to_string(),
            }],
        }
    }

    fn run_fake(
        request: &StagedPartitionReplacementRequest,
        fake: &mut FakeModelClient,
    ) -> StagedModelResult {
        let prepared = PreparedModelRun::new(request, true).expect("prepared fake model run");
        let core = run_prepared_model(fake, &prepared);
        let cleanup = ModelCleanupEvidence {
            children_reaped: true,
            pumps_joined: true,
            forced: false,
        };
        let core = materialize_verified(&prepared, core);
        finish_model_run(&prepared, core, Some(cleanup))
    }

    #[test]
    fn staged_model_requires_consent_before_path_or_process_mutation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        assert!(!request.fresh_export_root.exists());
        let failure = match PreparedModelRun::new(&request, false) {
            Ok(_) => panic!("consent was not required"),
            Err(failure) => failure,
        };
        assert_eq!(failure.phase, "consent");
        assert!(!request.fresh_export_root.exists());
    }

    #[test]
    fn staged_model_rejects_credential_like_m_before_export_preparation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        request.replacements[0].complete_m_expression =
            "let apiKey = \"unique-secret-value\" in apiKey".to_string();
        let failure = match PreparedModelRun::new(&request, true) {
            Ok(_) => panic!("credential-like M expression was accepted"),
            Err(failure) => failure,
        };
        assert_eq!(failure.phase, "credential-scan");
        assert!(!request.fresh_export_root.exists());
    }

    #[test]
    fn late_preparation_failure_releases_export_reservation_for_retry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        let expected = request.replacements[0].expected_before_sha256.clone();
        request.replacements[0].expected_before_sha256 = sha256_bytes(b"drifted source");
        let failure = match PreparedModelRun::new(&request, true) {
            Ok(_) => panic!("drifted before fingerprint was accepted"),
            Err(failure) => failure,
        };
        assert_eq!(failure.phase, "prepare");
        assert!(!request.fresh_export_root.exists());
        assert!(
            !request
                .workflow_root
                .join(".mcp-export.powerbi-cli-quarantine")
                .exists()
        );

        request.replacements[0].expected_before_sha256 = expected;
        let prepared =
            PreparedModelRun::new(&request, true).expect("retry after late preparation failure");
        assert!(prepared.paths.export_root.join("definition").is_dir());
    }

    #[test]
    fn staged_model_write_error_never_echoes_complete_m() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let sentinel = "UNIQUE_COMPLETE_M_SENTINEL_SHOULD_NOT_ESCAPE";
        let mut fake = FakeModelClient::new(definition);
        fake.fail_at = Some("update");
        fake.failure_message = Some(format!("vendor echoed M: {sentinel}"));
        let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
            panic!("write error unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "write");
        assert!(!failure.error.message().contains(sentinel));
        assert!(failure.error.message().contains("vendorDetailSha256="));
    }

    #[test]
    fn staged_model_success_has_exact_order_and_only_native_source_materialization() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let result = run_fake(&request, &mut fake);
        assert_eq!(fake.calls, ["connect", "list", "update", "get", "export"]);
        let StagedModelResult::Succeeded(success) = result else {
            panic!("fake staged workflow did not succeed")
        };
        assert!(success.source.byte_identical);
        assert!(!success.stage_definition.byte_identical);
        assert_eq!(
            success.stage_definition.after_sha256,
            success.expected_stage_sha256
        );
        let [replacement] = success.replacements.as_slice() else {
            panic!("expected one replacement")
        };
        assert_eq!(replacement.requested_sha256, replacement.readback_sha256);
        assert_eq!(replacement.readback_sha256, replacement.materialized_sha256);
        let table = std::fs::read_to_string(
            request
                .staged_semantic_model_root
                .join("definition")
                .join("tables")
                .join("Synthetic.tmdl"),
        )
        .expect("materialized table");
        assert!(table.contains("{{2}}"));
        assert!(table.ends_with("\t\tannotation PBI_NavigationStepName = Navigation\n"));
    }

    #[test]
    fn staged_model_failures_quarantine_export_and_leave_source_and_stage_identical() {
        let cases = [
            (
                Some("update"),
                McpFailureKind::Backend,
                false,
                FakeExportShape::Valid,
                "write",
            ),
            (
                None,
                McpFailureKind::Backend,
                true,
                FakeExportShape::Valid,
                "readback",
            ),
            (
                Some("get"),
                McpFailureKind::Cancelled,
                false,
                FakeExportShape::Valid,
                "readback",
            ),
            (
                Some("export"),
                McpFailureKind::Backend,
                false,
                FakeExportShape::Valid,
                "export",
            ),
            (
                None,
                McpFailureKind::Backend,
                false,
                FakeExportShape::RootTmdl,
                "export-proof",
            ),
        ];
        for (fail_at, fail_kind, mismatch, export_shape, expected_phase) in cases {
            let temp = tempfile::tempdir().expect("tempdir");
            let request = staged_request(&temp);
            let definition =
                std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                    .expect("canonical definition");
            let source_snapshot =
                SourceTreeSnapshot::capture(&request.source_root).expect("source snapshot");
            let stage_snapshot = SourceTreeSnapshot::capture(&definition).expect("stage snapshot");
            let mut fake = FakeModelClient::new(definition);
            fake.fail_at = fail_at;
            fake.fail_kind = fail_kind;
            fake.readback_mismatch = mismatch;
            fake.export_shape = export_shape;
            let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
                panic!("failure case {expected_phase} unexpectedly succeeded")
            };
            assert_eq!(failure.phase, expected_phase);
            assert!(
                source_snapshot
                    .verify()
                    .expect("source proof")
                    .byte_identical
            );
            assert!(stage_snapshot.verify().expect("stage proof").byte_identical);
            assert!(
                request
                    .fresh_export_root
                    .join(".powerbi-cli-failure-only")
                    .is_file()
            );
        }
    }

    #[test]
    fn staged_model_exact_tree_proof_rejects_unrelated_concurrent_change() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition.clone());
        let prepared = PreparedModelRun::new(&request, true).expect("prepared model run");
        let core = run_prepared_model(&mut fake, &prepared);
        std::fs::write(
            definition.join("model.tmdl"),
            "model Model\n\tculture: de-CH\n",
        )
        .expect("concurrent stage mutation");
        let core = materialize_verified(&prepared, core);
        let StagedModelResult::Failed(failure) = finish_model_run(&prepared, core, None) else {
            panic!("unrelated concurrent stage mutation escaped the exact tree proof")
        };
        assert_eq!(failure.phase, "post-cleanup-stage-proof");
        assert!(
            prepared
                .source_snapshot
                .verify()
                .expect("source proof")
                .byte_identical
        );
        assert!(
            !prepared
                .stage_snapshot
                .verify()
                .expect("stage proof")
                .byte_identical
        );
    }

    #[test]
    fn staged_model_rechecks_source_before_the_first_native_write() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let prepared = PreparedModelRun::new(&request, true).expect("prepared model run");
        let core = run_prepared_model(&mut fake, &prepared);
        std::fs::write(
            request.source_root.join("definition").join("model.tmdl"),
            "model Model\n\tculture: de-CH\n",
        )
        .expect("concurrent source mutation");
        let CoreModelOutcome::Failed(failure) = materialize_verified(&prepared, core) else {
            panic!("concurrent source mutation escaped the pre-write proof")
        };
        assert_eq!(failure.phase, "post-cleanup-source-proof");
        assert!(
            prepared
                .stage_snapshot
                .verify()
                .expect("stage proof")
                .byte_identical,
            "source drift must be rejected before the first staged write"
        );
    }

    #[test]
    fn staged_model_exact_tree_proof_supports_two_distinct_tmdl_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        let tables = request
            .staged_semantic_model_root
            .join("definition")
            .join("tables");
        let synthetic =
            std::fs::read_to_string(tables.join("Synthetic.tmdl")).expect("synthetic table");
        std::fs::write(
            tables.join("Other.tmdl"),
            synthetic.replace("Synthetic", "Other"),
        )
        .expect("second table");
        let expected = staged_partition_source_fingerprint(
            &request.staged_semantic_model_root,
            "Other",
            "Other",
        )
        .expect("second before fingerprint");
        request.replacements.push(StagedPartitionReplacement {
            table: "Other".to_string(),
            partition: "Other".to_string(),
            expected_before_sha256: expected,
            complete_m_expression:
                "let\n\tSource = #table(type table [Value = Int64.Type], {{3}})\nin\n\tSource"
                    .to_string(),
        });
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let StagedModelResult::Succeeded(success) = run_fake(&request, &mut fake) else {
            panic!("two-file staged workflow did not succeed")
        };
        assert_eq!(success.replacements.len(), 2);
        assert_eq!(
            success.stage_definition.after_sha256,
            success.expected_stage_sha256
        );
        assert!(
            std::fs::read_to_string(tables.join("Synthetic.tmdl"))
                .expect("synthetic after")
                .contains("{{2}}")
        );
        assert!(
            std::fs::read_to_string(tables.join("Other.tmdl"))
                .expect("other after")
                .contains("{{3}}")
        );
    }

    #[test]
    fn staged_model_composes_two_replacements_in_one_tmdl_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        let table_path = request
            .staged_semantic_model_root
            .join("definition")
            .join("tables")
            .join("Synthetic.tmdl");
        let mut table = std::fs::read_to_string(&table_path).expect("synthetic table");
        table.push_str(
            "\n\tpartition Other = m\n\t\tmode: import\n\t\tsource =\n\t\t\tlet\n\t\t\t\tSource = #table(type table [Value = Int64.Type], {{10}})\n\t\t\tin\n\t\t\t\tSource\n",
        );
        std::fs::write(&table_path, table).expect("second same-file partition");
        let expected = staged_partition_source_fingerprint(
            &request.staged_semantic_model_root,
            "Synthetic",
            "Other",
        )
        .expect("second before fingerprint");
        request.replacements.push(StagedPartitionReplacement {
            table: "Synthetic".to_string(),
            partition: "Other".to_string(),
            expected_before_sha256: expected,
            complete_m_expression:
                "let\n\tSource = #table(type table [Value = Int64.Type], {{3}})\nin\n\tSource"
                    .to_string(),
        });
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let StagedModelResult::Succeeded(success) = run_fake(&request, &mut fake) else {
            panic!("same-file staged workflow did not succeed")
        };
        assert_eq!(success.replacements.len(), 2);
        let materialized = std::fs::read_to_string(table_path).expect("materialized table");
        assert!(materialized.contains("{{2}}"));
        assert!(materialized.contains("{{3}}"));
        assert!(!materialized.contains("{{10}}"));
    }

    #[test]
    fn staged_model_rechecks_fresh_export_and_exact_connection_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let prepared = PreparedModelRun::new(&request, true).expect("prepared model run");
        std::fs::write(request.fresh_export_root.join("intruder.txt"), "occupied")
            .expect("occupy export after preparation");
        let mut fake = FakeModelClient::new(prepared.paths.definition_dir.clone());
        let core = run_prepared_model(&mut fake, &prepared);
        let StagedModelResult::Failed(failure) = finish_model_run(&prepared, core, None) else {
            panic!("occupied export unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "export-guard");
        assert!(!fake.calls.contains(&"export"));

        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        fake.duplicate_connection = true;
        let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
            panic!("duplicate connection unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "connection");

        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        fake.duplicate_path_different_name = true;
        let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
            panic!("duplicate canonical path with another name unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "connection");
    }

    #[test]
    #[ignore = "requires the exact installed Microsoft Modeling MCP package"]
    fn exact_real_disposable_offline_workflow() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let tool = crate::microsoft::resolve_installed_component(MicrosoftComponent::ModelingMcp)
            .expect("installed Modeling MCP");
        let success = match execute_staged_partition_replacements(&tool, &request, true) {
            StagedModelResult::Succeeded(success) => success,
            StagedModelResult::Failed(failure) => panic!(
                "exact offline workflow failed at {}: {}",
                failure.phase,
                failure.error.message()
            ),
        };
        assert!(success.cleanup.children_reaped);
        assert!(success.cleanup.pumps_joined);
        assert!(success.source.byte_identical);
        assert_eq!(
            success.stage_definition.after_sha256,
            success.expected_stage_sha256
        );
        assert_eq!(
            success.export.export_root,
            std::fs::canonicalize(&request.fresh_export_root).expect("canonical export")
        );
        assert!(success.export.file_count >= 3);
        assert!(
            request
                .fresh_export_root
                .join("definition")
                .join("database.tmdl")
                .is_file()
        );
        assert!(
            !request
                .fresh_export_root
                .join(".powerbi-cli-failure-only")
                .exists()
        );
    }

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
