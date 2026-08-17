//! Bounded MCP I/O pumps, child-process monitoring, and cleanup handling.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct StreamCapture {
    pub(super) tail: String,
    pub(super) sha256: String,
    pub(super) total_bytes: u64,
    pub(super) truncated: bool,
}

#[derive(Debug, Clone)]
pub(super) struct MonitorReport {
    pub(super) status: Option<ExitStatus>,
    pub(super) forced: bool,
    pub(super) tree_termination_attempted: bool,
    pub(super) root_reaped: bool,
    pub(super) captured_descendants: usize,
    pub(super) descendants_gone: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct McpCleanupReport {
    pub(crate) children_reaped: bool,
    pub(crate) forced: bool,
    pub(crate) stderr_sha256: String,
    pub(crate) stderr_truncated: bool,
    pub(super) stderr: StreamCapture,
    pub(super) monitor: MonitorReport,
    pub(super) pumps_joined: bool,
    pub(super) join_failure: Option<PumpJoinFailure>,
}

#[derive(Debug)]
pub(super) enum WriterCommand {
    Frame(Vec<u8>),
    Close,
}

#[derive(Debug)]
pub(super) enum ReaderEvent {
    Frame(Vec<u8>),
    Failure(String),
    Eof,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum MonitorCommand {
    Graceful,
    Force,
}

#[derive(Debug, Clone)]
pub(super) struct PumpJoinFailure {
    pub(super) kind: McpFailureKind,
    pub(super) message: String,
}

impl PumpJoinFailure {
    pub(super) fn backend(message: impl Into<String>) -> Self {
        Self {
            kind: McpFailureKind::Backend,
            message: message.into(),
        }
    }

    pub(super) fn panicked(label: &str) -> Self {
        Self {
            kind: McpFailureKind::Panicked,
            message: format!("MCP {label} pump join failed: worker thread panicked"),
        }
    }

    pub(super) fn into_failure(self) -> McpFailure {
        match self.kind {
            McpFailureKind::Protocol => McpFailure::protocol(self.message),
            McpFailureKind::Backend => McpFailure::backend(self.message),
            McpFailureKind::Cancelled => McpFailure::cancelled(self.message),
            McpFailureKind::Panicked => McpFailure::panicked(self.message),
        }
    }
}

pub(super) fn writer_pump(
    mut stdin: ChildStdin,
    receiver: Receiver<WriterCommand>,
) -> Result<(), String> {
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

pub(super) fn reader_pump(
    mut stdout: ChildStdout,
    sender: SyncSender<ReaderEvent>,
    frame_limit: usize,
    total_limit: usize,
) -> Result<(), String> {
    read_frames(&mut stdout, &sender, frame_limit, total_limit).map(|_| ())
}

pub(super) fn read_frames(
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

pub(super) fn stderr_pump(mut stderr: ChildStderr, limit: usize) -> Result<StreamCapture, String> {
    capture_stderr(&mut stderr, limit)
}

pub(super) fn capture_stderr(reader: &mut dyn Read, limit: usize) -> Result<StreamCapture, String> {
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

pub(super) struct ChildGuard {
    child: Option<GroupChild>,
    armed: bool,
}

impl ChildGuard {
    pub(super) fn new(child: GroupChild) -> Self {
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

pub(super) fn monitor_pump(
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

pub(super) struct TreeTermination {
    status: ExitStatus,
}

pub(super) fn terminate_child_tree(child: &mut GroupChild) -> io::Result<TreeTermination> {
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

pub(super) fn join_pump<T>(
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

pub(super) fn join_monitor(
    handle: &mut Option<JoinHandle<MonitorReport>>,
) -> Result<MonitorReport, PumpJoinFailure> {
    let Some(handle) = handle.take() else {
        return Err(PumpJoinFailure::backend("MCP monitor handle is missing"));
    };
    handle
        .join()
        .map_err(|_| PumpJoinFailure::panicked("monitor"))
}

pub(super) fn join_stderr(
    handle: &mut Option<JoinHandle<Result<StreamCapture, String>>>,
) -> Result<StreamCapture, PumpJoinFailure> {
    join_pump(handle, "stderr")
}
