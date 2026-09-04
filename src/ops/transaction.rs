use super::{Op, ValidatedPlan};
use crate::project_io::{
    PendingTextWrite, begin_text_atomic, copy_project_dir, write_json_new_atomic,
};
use crate::{
    CliError, CliResult, EXIT_VALIDATION_FAILED, ResolvedProject, ValidationReport,
    canonical_display, command_arg, input_safety, resolve_project, validate_project, walkdir_entry,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;
use walkdir::WalkDir;

/// The result returned by a kernel after one operation has been applied to the
/// working copy. Kernels can add their normal mutation `changes` and readback
/// commands without knowing whether the caller will eventually commit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpOutcome {
    pub(crate) changed: bool,
    pub(crate) changes: Vec<Value>,
    pub(crate) readback: Vec<String>,
    pub(crate) warnings: Vec<Value>,
    pub(crate) created_handles: Vec<String>,
}

impl OpOutcome {
    pub(crate) fn unchanged() -> Self {
        Self::default()
    }

    pub(crate) fn changed() -> Self {
        Self {
            changed: true,
            ..Self::default()
        }
    }
}

/// A filesystem-level change observed between two operation stages. `before`
/// and `after` are SHA-256 digests (rather than file contents) so a journal can
/// safely accompany an agent response without accidentally carrying data rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Change {
    pub(crate) path: PathBuf,
    pub(crate) before: Option<String>,
    pub(crate) after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransactionReceipt {
    pub(crate) outcomes: Vec<OpOutcome>,
    pub(crate) changes: Vec<Change>,
}

#[derive(Debug)]
pub(crate) struct PlanFailure {
    pub(crate) failed_index: usize,
    pub(crate) error: Box<CliError>,
    pub(crate) succeeded: Vec<usize>,
}

impl PlanFailure {
    fn new(failed_index: usize, error: CliError, succeeded: &[usize]) -> Self {
        Self {
            failed_index,
            error: Box::new(error),
            succeeded: succeeded.to_vec(),
        }
    }
}

/// A kernel applies one typed operation against the transaction's staged
/// working copy. Implementations can call [`Transaction::working_project`] and
/// then reuse the existing path-based mutation modules unchanged.
pub(crate) trait OpKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome>;
}

/// Closures are useful for compiler wiring and tests while concrete kernels
/// can implement [`OpKernel`] directly in later beads.
impl<F> OpKernel for F
where
    F: FnMut(&Op, &ResolvedProject) -> CliResult<OpOutcome>,
{
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let project = transaction.working_project()?;
        self(operation, &project)
    }
}

/// A deterministic timestamp source for snapshot naming and manifests.
pub(crate) trait SnapshotClock {
    fn now_utc(&self) -> String;
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemSnapshotClock;

impl SnapshotClock for SystemSnapshotClock {
    fn now_utc(&self) -> String {
        unix_timestamp()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FixedSnapshotClock {
    pub(crate) timestamp: String,
}

impl FixedSnapshotClock {
    pub(crate) fn new(timestamp: impl Into<String>) -> Self {
        Self {
            timestamp: timestamp.into(),
        }
    }
}

impl SnapshotClock for FixedSnapshotClock {
    fn now_utc(&self) -> String {
        self.timestamp.clone()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SnapshotOptions {
    pub(crate) snapshot_dir: Option<PathBuf>,
    pub(crate) created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitReceipt {
    pub(crate) mode: &'static str,
    pub(crate) project_dir: PathBuf,
    pub(crate) snapshot_dir: Option<PathBuf>,
}

/// A working-copy transaction. No mutation reaches `source` until one of the
/// commit methods succeeds; dropping this value discards the temporary tree.
#[derive(Debug)]
pub(crate) struct Transaction {
    work: TempDir,
    pub(crate) source: ResolvedProject,
    pub(crate) journal: Vec<Change>,
    aborted: bool,
}

impl Transaction {
    pub(crate) fn begin(source: ResolvedProject) -> CliResult<Self> {
        let work = tempfile::tempdir().map_err(|error| {
            CliError::unexpected(format!("create operation working copy: {error}"))
        })?;
        // `copy_project_dir` intentionally skips repository internals; scan
        // the project tree first so a symlink cannot silently disappear from
        // the transaction's working copy.
        collect_files(&source.project_dir)?;
        copy_project_dir(&source.project_dir, work.path())?;
        Ok(Self {
            work,
            source,
            journal: Vec::new(),
            aborted: false,
        })
    }

    pub(crate) fn work_dir(&self) -> &Path {
        self.work.path()
    }

    pub(crate) fn working_project(&self) -> CliResult<ResolvedProject> {
        resolve_project(self.work.path())
    }

    pub(crate) fn validate_working_copy(&self) -> CliResult<ValidationReport> {
        let project = self.working_project()?;
        validate_project(&project)
    }

    pub(crate) fn apply_all<K: OpKernel>(
        &mut self,
        plan: &ValidatedPlan,
        kernel: &mut K,
    ) -> Result<TransactionReceipt, PlanFailure> {
        if self.aborted {
            return Err(PlanFailure::new(
                0,
                CliError::invalid_args("operation transaction has already been aborted"),
                &[],
            ));
        }
        let mut outcomes = Vec::with_capacity(plan.ops.len());
        let mut succeeded = Vec::with_capacity(plan.ops.len());

        for validated in &plan.ops {
            let before = match collect_files(self.work.path()) {
                Ok(files) => files,
                Err(error) => {
                    self.aborted = true;
                    return Err(PlanFailure::new(validated.index, error, &succeeded));
                }
            };
            let mut outcome = match kernel.apply(&validated.operation, self) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.aborted = true;
                    return Err(PlanFailure::new(validated.index, error, &succeeded));
                }
            };
            let after = match collect_files(self.work.path()) {
                Ok(files) => files,
                Err(error) => {
                    self.aborted = true;
                    return Err(PlanFailure::new(validated.index, error, &succeeded));
                }
            };
            self.journal.extend(tree_changes(&before, &after));
            if let Some(handle) = validated.operation.declared_handle() {
                if outcome.created_handles.is_empty() {
                    outcome.created_handles.push(handle.to_string());
                } else if !outcome.created_handles.iter().any(|item| item == handle) {
                    self.aborted = true;
                    return Err(PlanFailure::new(
                        validated.index,
                        CliError::new(
                            "ops.handle_mismatch",
                            EXIT_VALIDATION_FAILED,
                            format!(
                                "operation `{}` declared handle `{handle}` but kernel returned [{}]",
                                validated.operation.tag(),
                                outcome.created_handles.join(", ")
                            ),
                        )
                        .with_pointer(format!("/ops/{}/handle", validated.index)),
                        &succeeded,
                    ));
                }
            }
            outcomes.push(outcome);
            succeeded.push(validated.index);
        }

        let validation = match self.validate_working_copy() {
            Ok(validation) => validation,
            Err(error) => {
                self.aborted = true;
                return Err(PlanFailure::new(plan.ops.len(), error, &succeeded));
            }
        };
        if !validation.errors.is_empty() {
            self.aborted = true;
            return Err(PlanFailure::new(
                plan.ops.len(),
                CliError::validation_failed(format!(
                    "staged operation plan failed native validation: {}",
                    validation.errors.join("; ")
                )),
                &succeeded,
            ));
        }

        Ok(TransactionReceipt {
            outcomes,
            changes: self.journal.clone(),
        })
    }

    /// Commit the validated working copy to a new output directory. A
    /// same-filesystem directory rename is attempted first; copying is the
    /// explicit fallback for a cross-device temporary directory.
    pub(crate) fn commit_out_dir(self, out_dir: &Path, force: bool) -> CliResult<CommitReceipt> {
        if self.aborted {
            return Err(CliError::invalid_args(
                "operation transaction was aborted and cannot be committed",
            ));
        }
        validate_output_target(&self.source.project_dir, out_dir)?;
        prepare_output_directory(out_dir, force)?;
        if let Some(parent) = out_dir.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
                CliError::unexpected(format!(
                    "create output parent {}: {error}",
                    parent.display()
                ))
            })?;
        }

        match fs::rename(self.work.path(), out_dir) {
            Ok(()) => {
                std::mem::forget(self.work);
                Ok(CommitReceipt {
                    mode: "out-dir",
                    project_dir: out_dir.to_path_buf(),
                    snapshot_dir: None,
                })
            }
            Err(rename_error) => {
                if let Err(copy_error) = copy_project_dir(self.work.path(), out_dir) {
                    let cleanup = cleanup_detail(out_dir);
                    return Err(CliError::unexpected(format!(
                        "rename working copy to {} failed: {rename_error}; copying it also failed: {}{cleanup}",
                        out_dir.display(),
                        copy_error.message
                    )));
                }
                Ok(CommitReceipt {
                    mode: "out-dir",
                    project_dir: out_dir.to_path_buf(),
                    snapshot_dir: None,
                })
            }
        }
    }

    pub(crate) fn commit_in_place(self, options: SnapshotOptions) -> CliResult<CommitReceipt> {
        let clock = options
            .created_at
            .as_deref()
            .map(|timestamp| FixedSnapshotClock::new(timestamp.to_string()));
        match clock {
            Some(clock) => self.commit_in_place_with_clock(options.snapshot_dir, &clock),
            None => self.commit_in_place_with_clock(options.snapshot_dir, &SystemSnapshotClock),
        }
    }

    pub(crate) fn commit_in_place_with_clock(
        mut self,
        requested_snapshot_dir: Option<PathBuf>,
        clock: &dyn SnapshotClock,
    ) -> CliResult<CommitReceipt> {
        if self.aborted {
            return Err(CliError::invalid_args(
                "operation transaction was aborted and cannot be committed",
            ));
        }
        let source_root = self.source.project_dir.clone();
        let source_files = collect_files(&source_root)?;
        let work_files = collect_files(self.work.path())?;
        let changes = tree_changes(&source_files, &work_files);
        if changes.is_empty() {
            return Ok(CommitReceipt {
                mode: "in-place",
                project_dir: source_root.clone(),
                snapshot_dir: None,
            });
        }

        let created_at = clock.now_utc();
        let snapshot_dir = snapshot_path(&source_root, requested_snapshot_dir, &created_at)?;
        write_snapshot(
            &source_root,
            &snapshot_dir,
            &created_at,
            &source_files,
            &work_files,
            &changes,
        )?;

        if try_directory_swap(&mut self, &source_root, &snapshot_dir)? {
            return Ok(CommitReceipt {
                mode: "in-place",
                project_dir: source_root.clone(),
                snapshot_dir: Some(snapshot_dir),
            });
        }

        apply_per_file_changes(&source_root, &source_files, &work_files, &changes)?;
        Ok(CommitReceipt {
            mode: "in-place",
            project_dir: source_root.clone(),
            snapshot_dir: Some(snapshot_dir),
        })
    }
}

/// Restore the source bytes captured by an in-place transaction snapshot.
///
/// The manifest is treated as an optimistic-concurrency record: every file
/// must still match either its captured after-state or its already-restored
/// before-state before a replacement is published. Restoration uses the same
/// temporary-tree and directory-swap machinery as a normal transaction, so a
/// malformed or stale snapshot never leaves a partially restored project.
pub(crate) fn restore_snapshot(source_root: &Path, snapshot_dir: &Path) -> CliResult<()> {
    let source_root = canonical_restore_source(source_root)?;
    let snapshot_root = canonical_restore_snapshot(snapshot_dir, &source_root)?;
    let manifest_path = snapshot_root.join("manifest.v1.json");
    let manifest = read_snapshot_manifest(&manifest_path)?;
    if manifest["project"].as_str() != Some(canonical_display(&source_root).as_str()) {
        return Err(CliError::validation_failed(
            "snapshot manifest belongs to a different project",
        ));
    }
    let entries = manifest["files"]
        .as_array()
        .ok_or_else(|| CliError::validation_failed("snapshot manifest files must be an array"))?;
    let source_files = collect_files(&source_root)?;
    let work = tempfile::tempdir().map_err(|error| {
        CliError::unexpected(format!("create snapshot restore working copy: {error}"))
    })?;
    copy_project_dir(&source_root, work.path())?;
    let mut restored_paths = BTreeSet::new();

    for (index, entry) in entries.iter().enumerate() {
        let object = entry.as_object().ok_or_else(|| {
            CliError::validation_failed(format!(
                "snapshot manifest files[{index}] must be an object"
            ))
        })?;
        let relative = object.get("path").and_then(Value::as_str).ok_or_else(|| {
            CliError::validation_failed(format!(
                "snapshot manifest files[{index}].path must be a string"
            ))
        })?;
        let relative = restore_relative_path(relative, index)?;
        if !restored_paths.insert(relative.to_path_buf()) {
            return Err(CliError::validation_failed(format!(
                "snapshot manifest contains duplicate path at files[{index}]: {}",
                relative.display()
            )));
        }

        let expected_before = match object.get("beforeSha256") {
            Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or_else(|| {
                        CliError::validation_failed(format!(
                            "snapshot manifest files[{index}].beforeSha256 must be a string or null"
                        ))
                    })?
                    .to_string(),
            ),
            None => {
                return Err(CliError::validation_failed(format!(
                    "snapshot manifest files[{index}] requires beforeSha256"
                )));
            }
        };
        let expected_after = match object.get("afterSha256") {
            Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_str()
                    .ok_or_else(|| {
                        CliError::validation_failed(format!(
                            "snapshot manifest files[{index}].afterSha256 must be a string or null"
                        ))
                    })?
                    .to_string(),
            ),
            None => {
                return Err(CliError::validation_failed(format!(
                    "snapshot manifest files[{index}] requires afterSha256"
                )));
            }
        };
        let actual = source_files.get(relative).map(|bytes| digest(bytes));
        // Permit an idempotent restore when the source is already at the
        // captured before-state, but refuse an unrelated concurrent edit.
        if actual != expected_after && actual != expected_before {
            return Err(CliError::validation_failed(format!(
                "snapshot state hash mismatch for {}",
                relative.display()
            )));
        }

        let target = work.path().join(relative);
        let existed = object
            .get("existed")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                CliError::validation_failed(format!(
                    "snapshot manifest files[{index}].existed must be a boolean"
                ))
            })?;
        if existed {
            let snapshot_file = snapshot_root.join(relative);
            let bytes = read_snapshot_file(&snapshot_file, relative)?;
            let snapshot_digest = digest(&bytes);
            if expected_before.as_deref() != Some(snapshot_digest.as_str()) {
                return Err(CliError::validation_failed(format!(
                    "snapshot bytes do not match beforeSha256 for {}",
                    relative.display()
                )));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    CliError::unexpected(format!(
                        "create snapshot restore parent {}: {error}",
                        parent.display()
                    ))
                })?;
            }
            fs::write(&target, bytes).map_err(|error| {
                CliError::unexpected(format!(
                    "write snapshot restore file {}: {error}",
                    target.display()
                ))
            })?;
        } else {
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(CliError::validation_failed(format!(
                        "snapshot restore target is a symlink: {}",
                        target.display()
                    )));
                }
                Ok(metadata) if metadata.is_dir() => {
                    return Err(CliError::validation_failed(format!(
                        "snapshot restore target is a directory: {}",
                        target.display()
                    )));
                }
                Ok(_) => fs::remove_file(&target).map_err(|error| {
                    CliError::unexpected(format!(
                        "remove snapshot restore file {}: {error}",
                        target.display()
                    ))
                })?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(CliError::unexpected(format!(
                        "inspect snapshot restore target {}: {error}",
                        target.display()
                    )));
                }
            }
        }
    }

    let work_files = collect_files(work.path())?;
    let changes = tree_changes(&source_files, &work_files);
    if changes
        .iter()
        .any(|change| !restored_paths.contains(&change.path))
    {
        return Err(CliError::validation_failed(
            "snapshot manifest did not account for every restored change",
        ));
    }
    let restored = resolve_project(work.path())?;
    let validation = validate_project(&restored)?;
    if !validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "snapshot restore failed native validation: {}",
            validation.errors.join("; ")
        )));
    }

    let source = resolve_project(&source_root)?;
    let mut transaction = Transaction {
        work,
        source,
        journal: changes,
        aborted: false,
    };
    if try_directory_swap(&mut transaction, &source_root, &snapshot_root)? {
        return Ok(());
    }
    apply_per_file_changes(
        &source_root,
        &source_files,
        &work_files,
        &transaction.journal,
    )
}

fn canonical_restore_source(source_root: &Path) -> CliResult<PathBuf> {
    let metadata = fs::symlink_metadata(source_root).map_err(|error| {
        CliError::file_not_found(format!(
            "inspect snapshot restore source {}: {error}",
            source_root.display()
        ))
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CliError::validation_failed(
            "snapshot restore source must be an ordinary directory",
        ));
    }
    fs::canonicalize(source_root).map_err(|error| {
        CliError::file_not_found(format!(
            "resolve snapshot restore source {}: {error}",
            source_root.display()
        ))
    })
}

fn canonical_restore_snapshot(snapshot_dir: &Path, source_root: &Path) -> CliResult<PathBuf> {
    let metadata = fs::symlink_metadata(snapshot_dir).map_err(|error| {
        CliError::file_not_found(format!(
            "inspect snapshot restore directory {}: {error}",
            snapshot_dir.display()
        ))
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CliError::validation_failed(
            "snapshot restore directory must be an ordinary directory",
        ));
    }
    let snapshot_root = fs::canonicalize(snapshot_dir).map_err(|error| {
        CliError::file_not_found(format!(
            "resolve snapshot restore directory {}: {error}",
            snapshot_dir.display()
        ))
    })?;
    if snapshot_root.starts_with(source_root) {
        return Err(CliError::validation_failed(
            "snapshot restore directory must be outside the project",
        ));
    }
    // Reject links anywhere in the untrusted snapshot tree before following
    // a manifest path into it.
    collect_files(&snapshot_root)?;
    Ok(snapshot_root)
}

fn read_snapshot_manifest(path: &Path) -> CliResult<Value> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::file_not_found(format!(
            "inspect snapshot manifest {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CliError::validation_failed(
            "snapshot manifest must be an ordinary file",
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        CliError::unexpected(format!(
            "read snapshot manifest {}: {error}",
            path.display()
        ))
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::validation_failed(format!("snapshot manifest is not valid JSON: {error}"))
    })?;
    if value["schema"] != "powerbi-cli.snapshot.manifest.v1" || value["manifest"] != "v1" {
        return Err(CliError::validation_failed(
            "snapshot manifest schema must be powerbi-cli.snapshot.manifest.v1",
        ));
    }
    Ok(value)
}

fn restore_relative_path(value: &str, index: usize) -> CliResult<&Path> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CliError::validation_failed(format!(
            "snapshot manifest files[{index}].path must contain only relative normal components: {value}"
        )));
    }
    Ok(path)
}

fn read_snapshot_file(path: &Path, relative: &Path) -> CliResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::validation_failed(format!(
            "snapshot file for {} is unavailable: {error}",
            relative.display()
        ))
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CliError::validation_failed(format!(
            "snapshot file for {} must be an ordinary file",
            relative.display()
        )));
    }
    fs::read(path).map_err(|error| {
        CliError::unexpected(format!("read snapshot file {}: {error}", path.display()))
    })
}

fn try_directory_swap(
    transaction: &mut Transaction,
    source_root: &Path,
    snapshot_dir: &Path,
) -> CliResult<bool> {
    // `copy_project_dir` intentionally omits repository internals. Keep an
    // in-place directory swap from deleting a source worktree's `.git`; the
    // per-file path below preserves untouched directories and files.
    match fs::symlink_metadata(source_root.join(".git")) {
        Ok(_) => return Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::unexpected(format!(
                "inspect source repository metadata: {error}"
            )));
        }
    }
    let parent = source_root.parent().ok_or_else(|| {
        CliError::unexpected(format!(
            "project path has no parent: {}",
            source_root.display()
        ))
    })?;
    let displaced = unique_path(
        parent,
        &format!(
            ".{}-powerbi-cli-old-{}",
            source_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("project"),
            snapshot_dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("snapshot")
        ),
    );
    // Allocate a replacement TempDir before moving the working directory. If
    // allocation fails, the source remains untouched and the transaction can
    // safely fall back to per-file replacement.
    let replacement_work = tempfile::tempdir()
        .map_err(|error| CliError::unexpected(format!("retain operation working copy: {error}")))?;
    if fs::rename(source_root, &displaced).is_err() {
        return Ok(false);
    }
    match fs::rename(transaction.work.path(), source_root) {
        Ok(()) => {
            let old_work = std::mem::replace(&mut transaction.work, replacement_work);
            let _ = remove_exact_directory(&displaced);
            std::mem::forget(old_work);
            Ok(true)
        }
        Err(rename_error) => {
            if let Err(restore_error) = fs::rename(&displaced, source_root) {
                return Err(CliError::unexpected(format!(
                    "replace project directory failed: {rename_error}; restoring original also failed: {restore_error}; snapshot retained at {}",
                    snapshot_dir.display()
                )));
            }
            Ok(false)
        }
    }
}

fn apply_per_file_changes(
    source_root: &Path,
    source_files: &BTreeMap<PathBuf, Vec<u8>>,
    work_files: &BTreeMap<PathBuf, Vec<u8>>,
    changes: &[Change],
) -> CliResult<()> {
    let mut pending_writes = Vec::<PendingTextWrite>::new();
    let mut pending_deletes = Vec::<PendingDeletion>::new();
    let mut created_dirs = Vec::<PathBuf>::new();

    let result = (|| {
        for change in changes {
            let source_path = source_root.join(&change.path);
            let source_bytes = source_files.get(&change.path);
            let work_bytes = work_files.get(&change.path);
            match (source_bytes, work_bytes) {
                (Some(_), Some(bytes)) => {
                    ensure_parent_directories(
                        source_path.parent().unwrap_or(source_root),
                        &mut created_dirs,
                    )?;
                    let text = String::from_utf8(bytes.clone()).map_err(|_| {
                        CliError::unsupported_feature(format!(
                            "in-place operation cannot atomically replace non-UTF-8 file: {}",
                            change.path.display()
                        ))
                    })?;
                    pending_writes.push(begin_text_atomic(&source_path, &text)?);
                }
                (None, Some(bytes)) => {
                    ensure_parent_directories(
                        source_path.parent().unwrap_or(source_root),
                        &mut created_dirs,
                    )?;
                    let text = String::from_utf8(bytes.clone()).map_err(|_| {
                        CliError::unsupported_feature(format!(
                            "in-place operation cannot atomically create non-UTF-8 file: {}",
                            change.path.display()
                        ))
                    })?;
                    pending_writes.push(begin_text_atomic(&source_path, &text)?);
                }
                (Some(_), None) => {
                    let backup = unique_path(
                        source_path.parent().unwrap_or(source_root),
                        &format!(
                            ".{}-powerbi-cli-delete",
                            source_path
                                .file_name()
                                .and_then(|value| value.to_str())
                                .unwrap_or("file")
                        ),
                    );
                    fs::rename(&source_path, &backup).map_err(|error| {
                        CliError::unexpected(format!(
                            "stage deletion of {} for rollback: {error}",
                            source_path.display()
                        ))
                    })?;
                    pending_deletes.push(PendingDeletion {
                        path: source_path,
                        backup,
                        committed: false,
                    });
                }
                (None, None) => {}
            }
        }
        Ok::<(), CliError>(())
    })();

    if let Err(error) = result {
        let rollback_error = rollback_pending(pending_writes, pending_deletes, created_dirs);
        return Err(combine_rollback_errors(
            error,
            rollback_error,
            None,
            "during staging".to_string(),
        ));
    }

    for index in 0..pending_writes.len() {
        if let Err(error) = pending_writes[index].commit_batch() {
            let rollback_error = rollback_pending(
                std::mem::take(&mut pending_writes),
                pending_deletes,
                created_dirs,
            );
            let restore_error = restore_original_files(source_root, source_files, changes);
            return Err(combine_rollback_errors(
                error,
                rollback_error,
                restore_error,
                format!("after committing in-place replacement {index}"),
            ));
        }
    }
    for index in 0..pending_deletes.len() {
        if let Err(error) = fs::remove_file(&pending_deletes[index].backup) {
            let failed_path = pending_deletes[index].path.display().to_string();
            let backup_path = pending_deletes[index].backup.display().to_string();
            let rollback_error = rollback_pending(
                pending_writes,
                std::mem::take(&mut pending_deletes),
                created_dirs,
            );
            let restore_error = restore_original_files(source_root, source_files, changes);
            return Err(combine_rollback_errors(
                CliError::unexpected(format!(
                    "in-place replacement succeeded for {}, but deletion backup cleanup failed at {}: {error}",
                    failed_path, backup_path
                )),
                rollback_error,
                restore_error,
                format!("after committing in-place deletion {index}"),
            ));
        }
        pending_deletes[index].committed = true;
    }
    Ok(())
}

#[derive(Debug)]
struct PendingDeletion {
    path: PathBuf,
    backup: PathBuf,
    committed: bool,
}

fn rollback_pending(
    pending_writes: Vec<PendingTextWrite>,
    pending_deletes: Vec<PendingDeletion>,
    created_dirs: Vec<PathBuf>,
) -> Option<CliError> {
    let mut failures = Vec::new();
    for pending in pending_writes.into_iter().rev() {
        if let Err(error) = pending.rollback() {
            failures.push(error.message);
        }
    }
    for pending in pending_deletes.into_iter().rev() {
        if pending.committed {
            continue;
        }
        if let Err(error) = fs::rename(&pending.backup, &pending.path) {
            failures.push(format!(
                "restore deleted {} from {}: {error}",
                pending.path.display(),
                pending.backup.display()
            ));
        }
    }
    for directory in created_dirs.into_iter().rev() {
        if let Err(error) = fs::remove_dir(&directory)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            failures.push(format!(
                "remove created directory {}: {error}",
                directory.display()
            ));
        }
    }
    (!failures.is_empty()).then(|| CliError::unexpected(failures.join("; ")))
}

fn restore_original_files(
    source_root: &Path,
    source_files: &BTreeMap<PathBuf, Vec<u8>>,
    changes: &[Change],
) -> Option<CliError> {
    let mut failures = Vec::new();
    for change in changes {
        let path = source_root.join(&change.path);
        match source_files.get(&change.path) {
            Some(bytes) => {
                if let Err(error) = fs::write(&path, bytes) {
                    failures.push(format!("restore original {}: {error}", path.display()));
                }
            }
            None => match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("remove new {}: {error}", path.display())),
            },
        }
    }
    (!failures.is_empty()).then(|| CliError::unexpected(failures.join("; ")))
}

fn combine_rollback_errors(
    error: CliError,
    rollback_error: Option<CliError>,
    restore_error: Option<CliError>,
    context: String,
) -> CliError {
    let mut details = Vec::new();
    if let Some(rollback) = rollback_error {
        details.push(format!("rollback also failed: {}", rollback.message));
    }
    if let Some(restore) = restore_error {
        details.push(format!("source restore also failed: {}", restore.message));
    }
    if details.is_empty() {
        error
    } else {
        CliError::unexpected(format!(
            "{} {context}: {}",
            error.message,
            details.join("; ")
        ))
    }
}

fn ensure_parent_directories(path: &Path, created: &mut Vec<PathBuf>) -> CliResult<()> {
    let mut missing = Vec::new();
    let mut current = path;
    while !current.exists() {
        missing.push(current.to_path_buf());
        current = current.parent().ok_or_else(|| {
            CliError::unexpected(format!("path has no existing parent: {}", path.display()))
        })?;
    }
    for directory in missing.into_iter().rev() {
        fs::create_dir(&directory).map_err(|error| {
            CliError::unexpected(format!(
                "create in-place parent {}: {error}",
                directory.display()
            ))
        })?;
        created.push(directory);
    }
    Ok(())
}

fn collect_files(root: &Path) -> CliResult<BTreeMap<PathBuf, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
    {
        let entry = walkdir_entry(root, entry, "walk operation project")?;
        if entry.file_type().is_symlink() {
            return Err(CliError::validation_failed(format!(
                "operation project contains unsupported symlink: {}",
                entry.path().display()
            )));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| CliError::unexpected(format!("operation project path: {error}")))?
            .to_path_buf();
        let bytes = fs::read(entry.path()).map_err(|error| {
            CliError::unexpected(format!(
                "read operation project file {}: {error}",
                entry.path().display()
            ))
        })?;
        files.insert(relative, bytes);
    }
    Ok(files)
}

fn tree_changes(
    before: &BTreeMap<PathBuf, Vec<u8>>,
    after: &BTreeMap<PathBuf, Vec<u8>>,
) -> Vec<Change> {
    let paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    paths
        .into_iter()
        .filter_map(|path| {
            let old = before.get(&path);
            let new = after.get(&path);
            (old != new).then(|| Change {
                path,
                before: old.map(|bytes| digest(bytes)),
                after: new.map(|bytes| digest(bytes)),
            })
        })
        .collect()
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn validate_output_target(source: &Path, output: &Path) -> CliResult<()> {
    if output == source {
        return Err(CliError::invalid_args(
            "operation out-dir must differ from the source project",
        ));
    }
    if output.exists()
        && fs::symlink_metadata(output)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
    {
        return Err(CliError::invalid_args(format!(
            "operation out-dir must not be a symlink: {}",
            output.display()
        )));
    }
    let source = fs::canonicalize(source)
        .map_err(|error| CliError::unexpected(format!("resolve source project: {error}")))?;
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        CliError::unexpected(format!(
            "create output parent {}: {error}",
            parent.display()
        ))
    })?;
    let output_absolute = if output.exists() {
        fs::canonicalize(output)
            .map_err(|error| CliError::unexpected(format!("resolve output directory: {error}")))?
    } else {
        fs::canonicalize(parent)
            .map_err(|error| CliError::unexpected(format!("resolve output parent: {error}")))?
            .join(output.file_name().unwrap_or(output.as_os_str()))
    };
    if output_absolute.starts_with(&source) {
        return Err(CliError::invalid_args(format!(
            "operation out-dir must not be inside the source project: {}",
            output.display()
        )));
    }
    Ok(())
}

fn prepare_output_directory(output: &Path, force: bool) -> CliResult<()> {
    let Ok(metadata) = fs::symlink_metadata(output) else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() {
        return Err(CliError::invalid_args(format!(
            "operation out-dir must not be a symlink: {}",
            output.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(CliError::invalid_args(format!(
            "operation out-dir is not a directory: {}",
            output.display()
        )));
    }
    let nonempty = fs::read_dir(output)
        .map_err(|error| CliError::unexpected(format!("read output directory: {error}")))?
        .next()
        .is_some();
    if nonempty && !force {
        return Err(CliError::invalid_args(format!(
            "operation out-dir is not empty: {}",
            output.display()
        )));
    }
    remove_exact_directory(output).map_err(|error| {
        CliError::unexpected(format!(
            "prepare operation out-dir {}: {error}",
            output.display()
        ))
    })
}

fn snapshot_path(
    source_root: &Path,
    requested: Option<PathBuf>,
    created_at: &str,
) -> CliResult<PathBuf> {
    let candidate = requested.unwrap_or_else(|| {
        let name = source_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("project");
        source_root
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{name}-snapshot-{}", path_timestamp(created_at)))
    });
    // Keep the snapshot boundary in the shared input-safety contract. It
    // canonicalizes the sibling/outside destination, rejects links/reparse
    // points and occupied paths, and probes the parent before any bytes are
    // written.
    input_safety::snapshot_destination(source_root, Some(&candidate))
}

fn write_snapshot(
    source_root: &Path,
    snapshot_dir: &Path,
    created_at: &str,
    source_files: &BTreeMap<PathBuf, Vec<u8>>,
    work_files: &BTreeMap<PathBuf, Vec<u8>>,
    changes: &[Change],
) -> CliResult<()> {
    let parent = snapshot_dir.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        CliError::unexpected(format!(
            "create snapshot parent {}: {error}",
            parent.display()
        ))
    })?;
    fs::create_dir(snapshot_dir).map_err(|error| {
        CliError::unexpected(format!(
            "create snapshot {}: {error}",
            snapshot_dir.display()
        ))
    })?;
    let write_result = (|| {
        let mut files = Vec::new();
        for change in changes {
            let Some(bytes) = source_files.get(&change.path) else {
                files.push(json!({
                    "path": change.path,
                    "existed": false,
                    "beforeSha256": Value::Null,
                    "afterSha256": change.after
                }));
                continue;
            };
            let target = snapshot_dir.join(&change.path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    CliError::unexpected(format!(
                        "create snapshot file parent {}: {error}",
                        parent.display()
                    ))
                })?;
            }
            fs::write(&target, bytes).map_err(|error| {
                CliError::unexpected(format!("write snapshot file {}: {error}", target.display()))
            })?;
            files.push(json!({
                "path": change.path,
                "existed": true,
                "beforeSha256": change.before,
                "afterSha256": work_files.get(&change.path).map(|value| digest(value))
            }));
        }
        let manifest = json!({
            "schema": "powerbi-cli.snapshot.manifest.v1",
            "manifest": "v1",
            "project": canonical_display(source_root),
            "createdAt": created_at,
            "files": files,
            "restoreCommand": format!(
                "powerbi-cli ops apply --restore {} --in-place --json",
                command_arg(snapshot_dir)
            )
        });
        write_json_new_atomic(&snapshot_dir.join("manifest.v1.json"), &manifest)
    })();
    if let Err(error) = write_result {
        let cleanup = cleanup_detail(snapshot_dir);
        return Err(CliError::unexpected(format!(
            "snapshot creation failed: {}{cleanup}",
            error.message
        )));
    }
    Ok(())
}

fn path_timestamp(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn unix_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn unique_path(parent: &Path, stem: &str) -> PathBuf {
    let mut candidate = parent.join(stem);
    let mut ordinal = 0usize;
    while candidate.exists() {
        ordinal += 1;
        candidate = parent.join(format!("{stem}-{ordinal}"));
    }
    candidate
}

fn remove_exact_directory(path: &Path) -> Result<(), std::io::Error> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path)
}

fn cleanup_detail(path: &Path) -> String {
    match remove_exact_directory(path) {
        Ok(()) => String::new(),
        Err(error) => format!("; cleanup of {} failed: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AddMeasure, AddVisual, Op, OpPlan, ProjectIndex};
    use super::*;

    fn minimal_project(root: &Path) -> ResolvedProject {
        let report_dir = root.join("Report.Report");
        let semantic_dir = root.join("Model.SemanticModel");
        fs::create_dir_all(report_dir.join("definition").join("pages").join("pages"))
            .expect("pages");
        fs::create_dir_all(semantic_dir.join("definition").join("tables")).expect("tables");
        fs::write(
            root.join("Report.pbip"),
            r#"{"version":"1.0","artifacts":[{"report":{"path":"Report.Report"}}]}"#,
        )
        .expect("pbip");
        fs::write(
            report_dir.join("definition.pbir"),
            r#"{"datasetReference":{"byPath":{"path":"../Model.SemanticModel"}}}"#,
        )
        .expect("pbir");
        fs::write(semantic_dir.join("definition.pbism"), "{}\n").expect("pbism");
        fs::write(report_dir.join("definition").join("version.json"), "{}\n").expect("version");
        fs::write(report_dir.join("definition").join("report.json"), "{}\n").expect("report");
        fs::write(
            report_dir
                .join("definition")
                .join("pages")
                .join("pages.json"),
            r#"{"pageOrder":[],"activePageName":""}"#,
        )
        .expect("pages json");
        fs::write(
            semantic_dir.join("definition").join("database.tmdl"),
            "database\n",
        )
        .expect("database");
        fs::write(
            semantic_dir.join("definition").join("model.tmdl"),
            "model\n",
        )
        .expect("model");
        fs::write(
            semantic_dir.join("definition").join("relationships.tmdl"),
            "\n",
        )
        .expect("relationships");
        ResolvedProject {
            project_dir: root.to_path_buf(),
            pbip_path: root.join("Report.pbip"),
            report_dir,
            semantic_model_dir: semantic_dir,
        }
    }

    fn measure(handle: &str) -> Op {
        Op::AddMeasure(AddMeasure {
            handle: handle.into(),
            table: "Sales".into(),
            name: "Revenue".into(),
            expression: "SUM(Sales[Revenue])".into(),
            format_string: None,
            format_string_definition: None,
            description: None,
            display_folder: None,
        })
    }

    fn visual(handle: &str) -> Op {
        Op::AddVisual(AddVisual {
            handle: handle.into(),
            page: "page:Overview".into(),
            visual_type: "card".into(),
            name: None,
            title: None,
            mode: None,
            single_select: None,
            position: None,
            bindings: Vec::new(),
        })
    }

    #[test]
    fn transaction_failure_at_index_two_never_publishes_output_or_changes_source() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        fs::create_dir_all(&source_root).expect("source");
        let source = minimal_project(&source_root);
        let before = collect_files(&source_root).expect("source files");
        let out = temp.path().join("out");
        let plan = OpPlan::new(vec![
            measure("measure:Sales:Revenue0"),
            measure("measure:Sales:Revenue1"),
            measure("measure:Sales:Revenue2"),
            measure("measure:Sales:Revenue3"),
        ]);
        let validated = plan.validate(&ProjectIndex::empty()).expect("valid plan");
        let mut transaction = Transaction::begin(source).expect("transaction");
        let mut kernel = |operation: &Op, project: &ResolvedProject| -> CliResult<OpOutcome> {
            let marker = project
                .project_dir
                .join(format!("marker-{}.txt", operation.tag()));
            fs::write(marker, operation.idempotent_key())
                .map_err(|error| CliError::unexpected(format!("marker: {error}")))?;
            if operation
                .declared_handle()
                .is_some_and(|handle| handle.ends_with("Revenue2"))
            {
                return Err(CliError::unsupported_feature("test kernel refusal"));
            }
            Ok(OpOutcome::changed())
        };
        let failure = transaction
            .apply_all(&validated, &mut kernel)
            .expect_err("index two must fail");
        assert_eq!(failure.failed_index, 2);
        assert_eq!(failure.succeeded, vec![0, 1]);
        let commit_error = transaction
            .commit_out_dir(&out, false)
            .expect_err("an aborted transaction cannot publish output");
        assert_eq!(commit_error.code, "invalid_args");
        assert!(!out.exists(), "failed transaction must not create out-dir");
        assert_eq!(collect_files(&source_root).expect("source after"), before);
        let snapshots = fs::read_dir(temp.path())
            .expect("temporary entries")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("source-snapshot-")
            })
            .count();
        assert_eq!(snapshots, 0, "failed plan must not create a snapshot");
    }

    #[test]
    fn successful_out_dir_commit_renames_the_working_copy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        fs::create_dir_all(&source_root).expect("source");
        let source = minimal_project(&source_root);
        let out = temp.path().join("out");
        let transaction = Transaction::begin(source).expect("transaction");
        let receipt = transaction
            .commit_out_dir(&out, false)
            .expect("out-dir commit");
        assert_eq!(receipt.mode, "out-dir");
        assert!(out.join("Report.pbip").is_file());
        assert!(source_root.join("Report.pbip").is_file());
    }

    #[test]
    fn in_place_commit_writes_fixed_clock_snapshot_before_replacing_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        fs::create_dir_all(&source_root).expect("source");
        let source = minimal_project(&source_root);
        fs::create_dir_all(source_root.join(".git")).expect("git directory");
        fs::write(
            source_root.join(".git").join("HEAD"),
            "ref: refs/heads/main\n",
        )
        .expect("git marker");
        let transaction = Transaction::begin(source).expect("transaction");
        fs::write(transaction.work_dir().join("marker.txt"), "changed\n").expect("marker");
        let clock = FixedSnapshotClock::new("2026-09-04T12:00:00Z");
        let receipt = transaction
            .commit_in_place_with_clock(None, &clock)
            .expect("in-place commit");
        let snapshot = receipt.snapshot_dir.expect("snapshot");
        assert_eq!(
            snapshot,
            temp.path().join("source-snapshot-2026-09-04T12-00-00Z")
        );
        assert!(snapshot.join("manifest.v1.json").is_file());
        assert_eq!(
            fs::read_to_string(source_root.join("marker.txt")).expect("marker"),
            "changed\n"
        );
        assert_eq!(
            fs::read_to_string(source_root.join(".git").join("HEAD")).expect("git marker"),
            "ref: refs/heads/main\n"
        );
        let manifest: Value =
            serde_json::from_slice(&fs::read(snapshot.join("manifest.v1.json")).expect("manifest"))
                .expect("manifest json");
        assert_eq!(manifest["createdAt"], "2026-09-04T12:00:00Z");
        assert!(
            manifest["restoreCommand"]
                .as_str()
                .is_some_and(|command| command.contains("ops apply --restore"))
        );
    }

    #[test]
    fn failed_plan_does_not_create_snapshot_or_modify_in_place_source() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        fs::create_dir_all(&source_root).expect("source");
        let source = minimal_project(&source_root);
        let before = collect_files(&source_root).expect("before");
        let plan = OpPlan::new(vec![
            measure("measure:Sales:Revenue0"),
            measure("measure:Sales:Revenue1"),
            measure("measure:Sales:Revenue2"),
            measure("measure:Sales:Revenue3"),
        ]);
        let validated = plan.validate(&ProjectIndex::empty()).expect("plan");
        let mut transaction = Transaction::begin(source).expect("transaction");
        let mut kernel = |_operation: &Op, _project: &ResolvedProject| {
            Err::<OpOutcome, _>(CliError::unsupported_feature("refused"))
        };
        let failure = transaction
            .apply_all(&validated, &mut kernel)
            .expect_err("failure");
        assert_eq!(failure.failed_index, 0);
        drop(transaction);
        assert_eq!(collect_files(&source_root).expect("after"), before);
        assert_eq!(fs::read_dir(temp.path()).expect("temp entries").count(), 1);
    }

    #[test]
    fn snapshot_restore_reinstates_the_captured_before_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_root = temp.path().join("source");
        let schema_path = Path::new("examples/sales.schema.json");
        let schema: Value =
            serde_json::from_str(include_str!("../../examples/sales.schema.json")).expect("schema");
        crate::scaffold_schema_value(schema, schema_path, &source_root, false)
            .expect("scaffold source");
        let source = resolve_project(&source_root).expect("resolve source");
        let before = collect_files(&source_root).expect("before");
        let transaction = Transaction::begin(source).expect("transaction");
        let changed_file = transaction
            .work_dir()
            .join("SalesOperations.Report")
            .join("definition")
            .join("report.json");
        fs::write(&changed_file, b"{\"changed\":true}\n").expect("change report");
        let receipt = transaction
            .commit_in_place_with_clock(None, &FixedSnapshotClock::new("2026-09-04T12:00:00Z"))
            .expect("commit");
        let snapshot = receipt.snapshot_dir.expect("snapshot");
        restore_snapshot(&source_root, &snapshot).expect("restore snapshot");
        assert_eq!(collect_files(&source_root).expect("after"), before);
    }
}
