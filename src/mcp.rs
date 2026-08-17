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
mod client;
mod staged;

pub(crate) use cleanup::McpCleanupReport;
use cleanup::{
    ChildGuard, MonitorCommand, MonitorReport, PumpJoinFailure, ReaderEvent, StreamCapture,
    WriterCommand, join_monitor, join_pump, join_stderr, monitor_pump, reader_pump, stderr_pump,
    terminate_child_tree, writer_pump,
};
#[cfg(test)]
use cleanup::{capture_stderr, read_frames};
pub(crate) use client::*;
use client::{
    DEFAULT_CLEANUP_TIMEOUT, DEFAULT_TOTAL_RESPONSE_LIMIT, checked_identifier, exact_keys,
    exact_object, hex_digest, redact_vendor_text, required_object_string, sha256_bytes,
};
pub(crate) use staged::*;
