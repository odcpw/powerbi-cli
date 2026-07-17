use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::ambient_authority;
use cap_std::fs::{Dir as CapabilityDir, OpenOptions as CapabilityOpenOptions};
use file_id::{FileId, get_file_id};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::mcp::{
    StagedModelResult, StagedPartitionReplacement, StagedPartitionReplacementRequest,
    execute_staged_model_export_proof, execute_staged_partition_replacements,
    staged_partition_source_fingerprint,
};
use crate::microsoft::{MicrosoftComponent, resolve_installed_component};
use crate::project_io::write_json_new_atomic;
use crate::safety_scan::contains_credential_like_text_str;
use crate::tmdl::{
    MutationPlan, PartitionSelector, find_partition, load_table_documents_from_semantic_model,
    replace_partition_source_plan,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, canonical_display, resolve_project, validate_command,
};

const MAX_DEFINITION_FILES: usize = 10_000;
const MAX_DEFINITION_BYTES: u64 = 64 * 1024 * 1024;
const MAX_HASHED_TREE_FILES: usize = 20_000;
const MAX_HASHED_TREE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
const MAX_TEMPLATE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RESOURCE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SOURCE_TEXT_BYTES: u64 = 16 * 1024 * 1024;
const TMDL_SUBDIRECTORIES: [&str; 4] = ["cultures", "perspectives", "roles", "tables"];
const SOURCE_PROFILE_SCHEMA: &str = "powerbi-cli.source-profile.v1";
const WORKFLOW_PLAN_SCHEMA: &str = "powerbi-cli.workflow-plan.v1";
const WORKFLOW_RECEIPT_SCHEMA: &str = "powerbi-cli.workflow-receipt.v1";
const WORKFLOW_POLICY: &str = "powerbi-cli.workflow-policy.v1";
const WORKFLOW_RECEIPT_FILE: &str = "powerbi-cli-workflow-receipt.json";
const WORKFLOW_INCOMPLETE_FILE: &str = ".powerbi-cli-workflow-incomplete";
const WORKFLOW_EVIDENCE_DIR: &str = ".powerbi-cli-model-evidence";
const INTEGRATION_LOCK_BYTES: &[u8] =
    include_bytes!("../integrations/microsoft/integration-lock.json");

#[derive(Debug, Clone)]
pub(crate) struct PreparedStagedModel {
    pub(crate) source_root: PathBuf,
    pub(crate) semantic_model_root: PathBuf,
    pub(crate) definition_dir: PathBuf,
    pub(crate) export_root: PathBuf,
    quarantine_marker: PathBuf,
}

pub(crate) struct PreparedStagedModelReservation {
    paths: PreparedStagedModel,
    preparation: PreparationGuard,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceTreeSnapshot {
    root: PathBuf,
    before_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceTreeEvidence {
    pub(crate) before_sha256: String,
    pub(crate) after_sha256: String,
    pub(crate) byte_identical: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportShapeProof {
    pub(crate) export_root: PathBuf,
    pub(crate) definition_sha256: String,
    pub(crate) file_count: usize,
    pub(crate) total_bytes: u64,
}

struct PreparationGuard {
    quarantine_marker: PathBuf,
    export_root: PathBuf,
    cleanup_tombstone: PathBuf,
    marker_created: bool,
    export_identity: Option<FileId>,
    definition_identity: Option<FileId>,
}

struct OwnedWorkflowOutput {
    root: PathBuf,
    capability: CapabilityDir,
    identity: FileId,
}

impl OwnedWorkflowOutput {
    fn create(path: &Path) -> CliResult<Self> {
        require_absent(path, "workflow output")?;
        let name = path
            .file_name()
            .ok_or_else(|| CliError::validation_failed("workflow output needs a directory name"))?;
        let parent = path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let expected = canonical_plain_directory(parent, "workflow output parent")?.join(name);
        fs::create_dir(path).map_err(|error| {
            CliError::unexpected(format!(
                "create workflow output {}: {error}",
                path.display()
            ))
        })?;
        let root = canonical_plain_directory(path, "workflow output")?;
        if root != expected {
            return Err(CliError::validation_failed(
                "created workflow output changed canonical identity",
            ));
        }
        let identity = get_file_id(&root).map_err(|error| {
            CliError::unexpected(format!(
                "open stable workflow output identity {}: {error}",
                root.display()
            ))
        })?;
        let capability =
            CapabilityDir::open_ambient_dir(&root, ambient_authority()).map_err(|error| {
                CliError::unexpected(format!(
                    "open workflow output directory capability {}: {error}",
                    root.display()
                ))
            })?;
        Ok(Self {
            root,
            capability,
            identity,
        })
    }

    fn verify_root(&self) -> CliResult<()> {
        let current = canonical_plain_directory(&self.root, "workflow output")?;
        let current_identity = get_file_id(&current).map_err(|error| {
            CliError::unexpected(format!(
                "open current workflow output identity {}: {error}",
                current.display()
            ))
        })?;
        if current != self.root || current_identity != self.identity {
            return Err(CliError::validation_failed(
                "workflow output filesystem identity changed during the run",
            ));
        }
        Ok(())
    }

    fn prepare_new_relative(&self, relative: &Path, label: &str) -> CliResult<PathBuf> {
        let relative_text = unicode_path(relative, label)?;
        let relative = validate_relative_path(&relative_text, label)?;
        relative.file_name().ok_or_else(|| {
            CliError::validation_failed(format!("{label} needs a final filename"))
        })?;
        let mut parent = self.capability.try_clone().map_err(|error| {
            CliError::unexpected(format!(
                "clone workflow output directory capability: {error}"
            ))
        })?;
        if let Some(relative_parent) = relative.parent() {
            for component in relative_parent.components() {
                let std::path::Component::Normal(name) = component else {
                    return Err(CliError::validation_failed(format!(
                        "{label} escaped the workflow output"
                    )));
                };
                match parent.symlink_metadata(name) {
                    Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {}
                    Ok(_) => {
                        return Err(CliError::validation_failed(format!(
                            "{label} parent is not an ordinary directory: {}",
                            self.root.join(relative_parent).display()
                        )));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        parent.create_dir(name).map_err(|error| {
                            CliError::unexpected(format!(
                                "create workflow-owned directory component {}: {error}",
                                name.to_string_lossy()
                            ))
                        })?;
                    }
                    Err(error) => {
                        return Err(CliError::unexpected(format!(
                            "inspect workflow-owned directory component {}: {error}",
                            name.to_string_lossy()
                        )));
                    }
                }
                parent = parent.open_dir_nofollow(name).map_err(|error| {
                    CliError::validation_failed(format!(
                        "open ordinary workflow-owned directory component {}: {error}",
                        name.to_string_lossy()
                    ))
                })?;
            }
        }
        let file_name = relative.file_name().expect("validated final filename");
        match parent.symlink_metadata(file_name) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(relative),
            Ok(_) => Err(CliError::invalid_args(format!(
                "{label} already exists and will not be replaced: {}",
                self.root.join(&relative).display()
            ))),
            Err(error) => Err(CliError::unexpected(format!(
                "inspect {label} {}: {error}",
                self.root.join(&relative).display()
            ))),
        }
    }

    fn open_parent_nofollow(&self, relative: &Path, label: &str) -> CliResult<CapabilityDir> {
        let mut parent = self.capability.try_clone().map_err(|error| {
            CliError::unexpected(format!(
                "clone workflow output directory capability: {error}"
            ))
        })?;
        if let Some(relative_parent) = relative.parent() {
            for component in relative_parent.components() {
                let std::path::Component::Normal(name) = component else {
                    return Err(CliError::validation_failed(format!(
                        "{label} escaped the workflow output"
                    )));
                };
                parent = parent.open_dir_nofollow(name).map_err(|error| {
                    CliError::validation_failed(format!(
                        "open ordinary workflow-owned directory component {}: {error}",
                        name.to_string_lossy()
                    ))
                })?;
            }
        }
        Ok(parent)
    }

    fn create_new_file_after(
        &self,
        relative: &Path,
        label: &str,
        before_capability_open: impl FnOnce(),
    ) -> CliResult<cap_std::fs::File> {
        let relative = self.prepare_new_relative(relative, label)?;
        before_capability_open();
        let parent = self.open_parent_nofollow(&relative, label)?;
        let file_name = relative.file_name().expect("validated final filename");
        let mut options = CapabilityOpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        parent.open_with(file_name, &options).map_err(|error| {
            CliError::unexpected(format!(
                "create {label} through the output directory capability {}: {error}",
                self.root.join(relative).display()
            ))
        })
    }

    fn verify_file(&self, relative: &Path, label: &str, max_bytes: u64) -> CliResult<FileClaim> {
        let relative_text = unicode_path(relative, label)?;
        let relative = validate_relative_path(&relative_text, label)?;
        let parent = self.open_parent_nofollow(&relative, label)?;
        let file_name = relative.file_name().ok_or_else(|| {
            CliError::validation_failed(format!("{label} needs a final filename"))
        })?;
        let path_metadata = parent.symlink_metadata(file_name).map_err(|error| {
            CliError::unexpected(format!(
                "inspect {label} {}: {error}",
                self.root.join(&relative).display()
            ))
        })?;
        if !path_metadata.is_file() || path_metadata.is_symlink() {
            return Err(CliError::validation_failed(format!(
                "{label} is not an ordinary file: {}",
                self.root.join(&relative).display()
            )));
        }
        let mut options = CapabilityOpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let mut file = parent.open_with(file_name, &options).map_err(|error| {
            CliError::unexpected(format!(
                "open {label} through the output directory capability {}: {error}",
                self.root.join(&relative).display()
            ))
        })?;
        let metadata = file.metadata().map_err(|error| {
            CliError::unexpected(format!(
                "inspect opened {label} {}: {error}",
                self.root.join(&relative).display()
            ))
        })?;
        if !metadata.is_file() || metadata.len() > max_bytes {
            return Err(CliError::validation_failed(format!(
                "{label} is not a bounded ordinary file: {}",
                self.root.join(&relative).display()
            )));
        }
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|error| {
                CliError::unexpected(format!(
                    "read {label} {}: {error}",
                    self.root.join(&relative).display()
                ))
            })?;
            if read == 0 {
                break;
            }
            total = total.checked_add(read as u64).ok_or_else(|| {
                CliError::validation_failed(format!("{label} byte count overflow"))
            })?;
            if total > max_bytes || total > metadata.len() {
                return Err(CliError::validation_failed(format!(
                    "{label} changed or exceeded its byte limit while reading"
                )));
            }
            hasher.update(&buffer[..read]);
        }
        if total != metadata.len() {
            return Err(CliError::validation_failed(format!(
                "{label} changed length while reading"
            )));
        }
        Ok(FileClaim {
            path: unicode_path(&self.root.join(&relative), label)?,
            sha256: format!("sha256:{:x}", hasher.finalize()),
            bytes: total,
        })
    }

    fn write_new_file(&self, relative: &Path, bytes: &[u8], label: &str) -> CliResult<FileId> {
        self.write_new_file_after(relative, bytes, label, || {})
    }

    fn write_new_file_after(
        &self,
        relative: &Path,
        bytes: &[u8],
        label: &str,
        before_capability_open: impl FnOnce(),
    ) -> CliResult<FileId> {
        let mut file = self.create_new_file_after(relative, label, before_capability_open)?;
        let identity = capability_file_id(&file, label)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| {
                CliError::unexpected(format!(
                    "write {label} {}: {error}",
                    self.root.join(relative).display()
                ))
            })?;
        drop(file);
        let claim = self.verify_file(relative, label, bytes.len() as u64)?;
        if claim.bytes != bytes.len() as u64 || claim.sha256 != sha256_bytes(bytes) {
            return Err(CliError::validation_failed(format!(
                "{label} failed exact capability-relative readback: {}",
                self.root.join(relative).display()
            )));
        }
        Ok(identity)
    }

    fn remove_owned_file(
        &self,
        relative: &Path,
        expected_identity: &FileId,
        label: &str,
    ) -> CliResult<()> {
        let relative_text = unicode_path(relative, label)?;
        let relative = validate_relative_path(&relative_text, label)?;
        let parent = self.open_parent_nofollow(&relative, label)?;
        let file_name = relative.file_name().ok_or_else(|| {
            CliError::validation_failed(format!("{label} needs a final filename"))
        })?;
        let path = self.root.join(&relative);
        let metadata = parent.symlink_metadata(file_name).map_err(|error| {
            CliError::unexpected(format!("inspect {label} {}: {error}", path.display()))
        })?;
        let mut options = CapabilityOpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = parent.open_with(file_name, &options).map_err(|error| {
            CliError::unexpected(format!("open {label} identity {}: {error}", path.display()))
        })?;
        let identity = capability_file_id(&file, label)?;
        if !metadata.is_file() || metadata.is_symlink() || &identity != expected_identity {
            return Err(CliError::validation_failed(format!(
                "{label} filesystem identity changed during the run"
            )));
        }
        parent.remove_file(file_name).map_err(|error| {
            CliError::unexpected(format!("remove {label} {}: {error}", path.display()))
        })
    }

    fn cleanup_if_empty(self) -> Option<String> {
        if self.verify_root().is_err()
            || !directory_has_entries(&self.root).is_ok_and(|has_entries| !has_entries)
        {
            return None;
        }
        let root = self.root.clone();
        drop(self);
        fs::remove_dir(&root).err().map(|error| error.to_string())
    }
}

fn capability_file_id(file: &cap_std::fs::File, label: &str) -> CliResult<FileId> {
    let metadata = file.metadata().map_err(|error| {
        CliError::unexpected(format!("read stable {label} filesystem identity: {error}"))
    })?;
    Ok(FileId::new_inode(metadata.dev(), metadata.ino()))
}

impl PreparationGuard {
    fn new(quarantine_marker: PathBuf, export_root: PathBuf, cleanup_tombstone: PathBuf) -> Self {
        Self {
            quarantine_marker,
            export_root,
            cleanup_tombstone,
            marker_created: false,
            export_identity: None,
            definition_identity: None,
        }
    }

    fn disarm(&mut self) {
        self.marker_created = false;
        self.export_identity = None;
        self.definition_identity = None;
    }

    fn cleanup_owned_empty_export(&mut self) {
        let (Some(export_identity), Some(definition_identity)) =
            (&self.export_identity, &self.definition_identity)
        else {
            return;
        };
        if fs::rename(&self.export_root, &self.cleanup_tombstone).is_err() {
            return;
        }
        let moved_identity = get_file_id(&self.cleanup_tombstone);
        if moved_identity.as_ref().ok() != Some(export_identity) {
            if !self.export_root.exists() {
                let _ = fs::rename(&self.cleanup_tombstone, &self.export_root);
            }
            return;
        }
        let moved_definition = self.cleanup_tombstone.join("definition");
        if get_file_id(&moved_definition).as_ref().ok() == Some(definition_identity) {
            let _ = fs::remove_dir(&moved_definition);
        }
        if get_file_id(&self.cleanup_tombstone).as_ref().ok() == Some(export_identity) {
            let _ = fs::remove_dir(&self.cleanup_tombstone);
        }
    }
}

impl Drop for PreparationGuard {
    fn drop(&mut self) {
        self.cleanup_owned_empty_export();
        if self.marker_created {
            let _ = fs::remove_file(&self.quarantine_marker);
        }
    }
}

impl PreparedStagedModel {
    pub(crate) fn prepare(
        source_root: &Path,
        semantic_model_root: &Path,
        workflow_root: &Path,
        export_root: &Path,
    ) -> Result<PreparedStagedModelReservation, String> {
        let source_root = canonical_directory(source_root, "source project")?;
        let semantic_model_root =
            canonical_directory(semantic_model_root, "staged semantic model")?;
        let definition_dir = canonical_directory(
            &semantic_model_root.join("definition"),
            "staged semantic-model definition",
        )?;
        validate_tmdl_definition(&definition_dir)?;
        if paths_overlap(&source_root, &semantic_model_root) {
            return Err(
                "the staged semantic model must not overlap the source project".to_string(),
            );
        }

        let workflow_root = canonical_directory(workflow_root, "workflow root")?;
        let export_parent = export_root.parent().ok_or_else(|| {
            format!(
                "fresh MCP export path has no parent: {}",
                export_root.display()
            )
        })?;
        let export_parent = fs::canonicalize(export_parent).map_err(|error| {
            format!(
                "resolve fresh MCP export parent {}: {error}",
                export_parent.display()
            )
        })?;
        let export_name = export_root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "MCP export directory name is not Unicode".to_string())?;
        let export_candidate = workflow_root.join(export_name);
        if export_parent != workflow_root {
            return Err(format!(
                "MCP export must be one direct workflow-owned child of {}",
                workflow_root.display()
            ));
        }
        for protected in [&source_root, &semantic_model_root, &definition_dir] {
            if paths_overlap(&export_candidate, protected) {
                return Err(format!(
                    "MCP export path overlaps protected model content: {}",
                    protected.display()
                ));
            }
        }
        let quarantine_marker =
            workflow_root.join(format!(".{export_name}.powerbi-cli-quarantine"));
        let cleanup_tombstone = workflow_root.join(format!(".{export_name}.powerbi-cli-cleanup"));
        match fs::symlink_metadata(&cleanup_tombstone) {
            Ok(_) => {
                return Err(format!(
                    "private MCP export cleanup path is occupied: {}",
                    cleanup_tombstone.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "inspect private MCP export cleanup path {}: {error}",
                    cleanup_tombstone.display()
                ));
            }
        }
        let mut preparation = PreparationGuard::new(
            quarantine_marker.clone(),
            export_candidate.clone(),
            cleanup_tombstone,
        );
        let mut marker = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&quarantine_marker)
            .map_err(|error| format!("arm MCP export quarantine: {error}"))?;
        preparation.marker_created = true;
        marker
            .write_all(b"armed\n")
            .and_then(|()| marker.sync_all())
            .map_err(|error| format!("sync MCP export quarantine: {error}"))?;
        match fs::symlink_metadata(&export_candidate) {
            Ok(_) => {
                return Err(format!(
                    "MCP export path must not exist before this invocation: {}",
                    export_candidate.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&export_candidate).map_err(|error| {
                    format!(
                        "atomically create workflow-owned MCP export directory {}: {error}",
                        export_candidate.display()
                    )
                })?
            }
            Err(error) => {
                return Err(format!(
                    "inspect MCP export path {}: {error}",
                    export_candidate.display()
                ));
            }
        }
        preparation.export_identity = Some(get_file_id(&export_candidate).map_err(|error| {
            format!(
                "open stable identity for MCP export directory {}: {error}",
                export_candidate.display()
            )
        })?);
        fs::create_dir(export_candidate.join("definition")).map_err(|error| {
            format!(
                "atomically create ordinary MCP TMDL target {}/definition: {error}",
                export_candidate.display()
            )
        })?;
        preparation.definition_identity = Some(
            get_file_id(export_candidate.join("definition")).map_err(|error| {
                format!("open stable identity for MCP export definition: {error}")
            })?,
        );
        let export_root = canonical_directory(&export_candidate, "MCP export")?;
        if export_root.parent() != Some(workflow_root.as_path()) {
            return Err("canonical MCP export escaped the workflow root".to_string());
        }
        Ok(PreparedStagedModelReservation {
            paths: Self {
                source_root,
                semantic_model_root,
                definition_dir,
                export_root,
                quarantine_marker,
            },
            preparation,
        })
    }

    pub(crate) fn validate_export(&self) -> Result<ExportShapeProof, String> {
        let current_export = canonical_directory(&self.export_root, "MCP export")?;
        if current_export != self.export_root {
            return Err("MCP export identity changed after preparation".to_string());
        }
        let mut entries = fs::read_dir(&current_export)
            .map_err(|error| format!("read MCP export {}: {error}", current_export.display()))?
            .map(|entry| {
                entry.map_err(|error| {
                    format!(
                        "read MCP export entry {}: {error}",
                        current_export.display()
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        if entries.len() != 1 || entries[0].file_name() != "definition" {
            return Err(
                "MCP export must contain exactly one definition/ directory; root TMDL and unexpected files are forbidden"
                    .to_string(),
            );
        }
        let metadata = entries[0]
            .metadata()
            .map_err(|error| format!("inspect exported definition: {error}"))?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err("exported definition must be an ordinary directory".to_string());
        }
        let definition = canonical_directory(&entries[0].path(), "exported definition")?;
        if definition.parent() != Some(current_export.as_path()) {
            return Err("exported definition escaped the fresh export root".to_string());
        }
        let summary = validate_tmdl_definition(&definition)?;
        Ok(ExportShapeProof {
            export_root: current_export,
            definition_sha256: summary.sha256,
            file_count: summary.file_count,
            total_bytes: summary.total_bytes,
        })
    }

    pub(crate) fn ensure_export_empty(&self) -> Result<(), String> {
        let current_export = canonical_directory(&self.export_root, "MCP export")?;
        if current_export != self.export_root {
            return Err("MCP export identity changed after preparation".to_string());
        }
        let definition =
            canonical_directory(&current_export.join("definition"), "fresh MCP TMDL target")?;
        if definition.parent() != Some(current_export.as_path())
            || directory_has_entries(&definition)?
            || fs::read_dir(&current_export)
                .map_err(|error| format!("read MCP export root: {error}"))?
                .count()
                != 1
        {
            return Err(format!(
                "MCP export target is no longer the one fresh empty definition directory: {}",
                current_export.display()
            ));
        }
        Ok(())
    }

    pub(crate) fn mark_export_failure_only(&self) -> Result<(), String> {
        let current_export = canonical_directory(&self.export_root, "MCP export")?;
        if current_export != self.export_root {
            return Err("MCP export identity changed after preparation".to_string());
        }
        let marker = current_export.join(".powerbi-cli-failure-only");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .map_err(|error| format!("create failure-only export marker: {error}"))?;
        file.write_all(b"This vendor export is evidence from a failed isolated workflow and must not be installed.\n")
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("write failure-only export marker: {error}"))
    }

    pub(crate) fn disarm_export_quarantine(&self) -> Result<(), String> {
        fs::remove_file(&self.quarantine_marker)
            .map_err(|error| format!("disarm MCP export quarantine: {error}"))
    }
}

impl PreparedStagedModelReservation {
    pub(crate) fn paths(&self) -> &PreparedStagedModel {
        &self.paths
    }

    pub(crate) fn commit(mut self) -> PreparedStagedModel {
        self.preparation.disarm();
        self.paths
    }
}

impl SourceTreeSnapshot {
    pub(crate) fn capture(root: &Path) -> Result<Self, String> {
        let root = canonical_directory(root, "source project")?;
        let before_sha256 = hash_tree(&root)?.sha256;
        Ok(Self {
            root,
            before_sha256,
        })
    }

    pub(crate) fn verify(&self) -> Result<SourceTreeEvidence, String> {
        let current = canonical_directory(&self.root, "source project")?;
        if current != self.root {
            return Err("source project canonical identity changed during workflow".to_string());
        }
        let after_sha256 = hash_tree(&self.root)?.sha256;
        Ok(SourceTreeEvidence {
            byte_identical: self.before_sha256 == after_sha256,
            before_sha256: self.before_sha256.clone(),
            after_sha256,
        })
    }

    pub(crate) fn expected_after_sha256(
        &self,
        replacements: &[(PathBuf, String)],
    ) -> Result<String, String> {
        let mut overrides = BTreeMap::new();
        for (path, text) in replacements {
            let canonical = fs::canonicalize(path).map_err(|error| {
                format!("resolve expected tree file {}: {error}", path.display())
            })?;
            let relative = canonical.strip_prefix(&self.root).map_err(|_| {
                format!(
                    "expected tree replacement escaped snapshot root: {}",
                    path.display()
                )
            })?;
            if relative.as_os_str().is_empty()
                || overrides
                    .insert(relative.to_path_buf(), text.as_bytes().to_vec())
                    .is_some()
            {
                return Err(format!(
                    "expected tree replacement is empty or duplicated: {}",
                    path.display()
                ));
            }
        }
        hash_tree_with_overrides(&self.root, &overrides).map(|summary| summary.sha256)
    }
}

#[derive(Debug)]
struct TreeSummary {
    sha256: String,
    file_count: usize,
    total_bytes: u64,
}

fn validate_tmdl_definition(definition: &Path) -> Result<TreeSummary, String> {
    let definition = canonical_directory(definition, "TMDL definition")?;
    let mut database = false;
    let mut model = false;
    let mut table_files = 0_usize;
    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;

    for entry in WalkDir::new(&definition).follow_links(false) {
        let entry = entry
            .map_err(|error| format!("walk TMDL definition {}: {error}", definition.display()))?;
        if entry.path() == definition {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&definition)
            .map_err(|error| format!("inspect exported TMDL relative path: {error}"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("inspect TMDL path {}: {error}", entry.path().display()))?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(format!(
                "TMDL definition contains a symlink, junction, or reparse point: {}",
                entry.path().display()
            ));
        }
        let components = relative.components().count();
        if metadata.is_dir() {
            if components != 1
                || !relative
                    .to_str()
                    .is_some_and(|value| TMDL_SUBDIRECTORIES.contains(&value))
            {
                return Err(format!(
                    "TMDL definition contains an unexpected directory: {}",
                    relative.display()
                ));
            }
            continue;
        }
        if !metadata.is_file()
            || !(components == 1 || components == 2)
            || entry.path().extension().and_then(|value| value.to_str()) != Some("tmdl")
        {
            return Err(format!(
                "TMDL definition contains an unexpected file: {}",
                relative.display()
            ));
        }
        if components == 2 {
            let parent = relative
                .parent()
                .and_then(Path::to_str)
                .ok_or_else(|| format!("TMDL path is not Unicode: {}", relative.display()))?;
            if !TMDL_SUBDIRECTORIES.contains(&parent) {
                return Err(format!(
                    "TMDL file is outside an expected one-level collection: {}",
                    relative.display()
                ));
            }
            if parent == "tables" {
                table_files = table_files.saturating_add(1);
            }
        }
        database |= relative == Path::new("database.tmdl");
        model |= relative == Path::new("model.tmdl");
        file_count = file_count.saturating_add(1);
        total_bytes = total_bytes.saturating_add(metadata.len());
        if file_count > MAX_DEFINITION_FILES || total_bytes > MAX_DEFINITION_BYTES {
            return Err("TMDL definition exceeds the bounded file or byte cap".to_string());
        }
        let text = read_bounded(entry.path(), MAX_DEFINITION_BYTES, "TMDL definition file")
            .map_err(|error| error.message)?;
        let text = std::str::from_utf8(&text).map_err(|_| {
            format!(
                "TMDL definition file must be UTF-8: {}",
                entry.path().display()
            )
        })?;
        if contains_credential_like_text_str(text) {
            return Err(format!(
                "TMDL definition contains credential-like text: {}",
                entry.path().display()
            ));
        }
        let canonical = fs::canonicalize(entry.path())
            .map_err(|error| format!("resolve TMDL file {}: {error}", entry.path().display()))?;
        if !canonical.starts_with(&definition) {
            return Err(format!(
                "TMDL file escaped the definition root: {}",
                entry.path().display()
            ));
        }
    }
    if !database || !model || table_files == 0 {
        return Err(
            "TMDL definition requires database.tmdl, model.tmdl, and at least one tables/*.tmdl"
                .to_string(),
        );
    }
    let hash = hash_tree(&definition)?;
    Ok(TreeSummary {
        sha256: hash.sha256,
        file_count,
        total_bytes,
    })
}

fn hash_tree(root: &Path) -> Result<TreeSummary, String> {
    hash_tree_with_overrides(root, &BTreeMap::new())
}

fn hash_tree_with_overrides(
    root: &Path,
    overrides: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<TreeSummary, String> {
    hash_tree_inner(root, overrides, &[])
}

fn hash_tree_with_exclusions(root: &Path, exclusions: &[&Path]) -> Result<TreeSummary, String> {
    hash_tree_inner(root, &BTreeMap::new(), exclusions)
}

fn hash_tree_inner(
    root: &Path,
    overrides: &BTreeMap<PathBuf, Vec<u8>>,
    exclusions: &[&Path],
) -> Result<TreeSummary, String> {
    hash_tree_inner_bounded(
        root,
        overrides,
        exclusions,
        MAX_HASHED_TREE_FILES,
        MAX_HASHED_TREE_BYTES,
    )
}

fn hash_tree_inner_bounded(
    root: &Path,
    overrides: &BTreeMap<PathBuf, Vec<u8>>,
    exclusions: &[&Path],
    max_files: usize,
    max_bytes: u64,
) -> Result<TreeSummary, String> {
    hash_tree_inner_bounded_with_opener(root, overrides, exclusions, max_files, max_bytes, |path| {
        File::open(path).map_err(|error| format!("open {}: {error}", path.display()))
    })
}

fn hash_tree_inner_bounded_with_opener(
    root: &Path,
    overrides: &BTreeMap<PathBuf, Vec<u8>>,
    exclusions: &[&Path],
    max_files: usize,
    max_bytes: u64,
    mut open_file: impl FnMut(&Path) -> Result<File, String>,
) -> Result<TreeSummary, String> {
    let root = canonical_directory(root, "hashed tree")?;
    let max_entries = max_files.saturating_mul(4).saturating_add(1_024);
    let mut paths = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = entry.map_err(|error| format!("walk {}: {error}", root.display()))?;
        if paths.len() >= max_entries {
            return Err("tree exceeds the bounded filesystem-entry cap".to_string());
        }
        paths.push(entry);
    }
    paths.sort_by(|left, right| left.path().cmp(right.path()));
    let mut hasher = Sha256::new();
    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    for entry in paths {
        if entry.path() == root {
            continue;
        }
        let relative_path = entry
            .path()
            .strip_prefix(&root)
            .map_err(|error| format!("hash relative path: {error}"))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("inspect {}: {error}", entry.path().display()))?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(format!(
                "tree contains a symlink, junction, or reparse point: {}",
                entry.path().display()
            ));
        }
        if exclusions.contains(&relative_path) {
            continue;
        }
        let relative = relative_path
            .to_str()
            .ok_or_else(|| format!("path is not Unicode: {}", entry.path().display()))?
            .replace('\\', "/");
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        if metadata.is_dir() {
            hasher.update(b"dir");
        } else if metadata.is_file() {
            hasher.update(b"file");
            let relative_path = entry
                .path()
                .strip_prefix(&root)
                .map_err(|error| format!("hash override path: {error}"))?;
            let file_bytes = overrides
                .get(relative_path)
                .map_or(metadata.len(), |bytes| bytes.len() as u64);
            if file_count >= max_files
                || total_bytes
                    .checked_add(file_bytes)
                    .is_none_or(|next| next > max_bytes)
            {
                return Err("tree exceeds the bounded file or byte cap".to_string());
            }
            if let Some(bytes) = overrides.get(relative_path) {
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(bytes);
            } else {
                hasher.update(metadata.len().to_le_bytes());
                let mut file = open_file(entry.path())?;
                let mut bytes_read = 0_u64;
                loop {
                    let read = file
                        .read(&mut buffer)
                        .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
                    if read == 0 {
                        break;
                    }
                    bytes_read = bytes_read.saturating_add(read as u64);
                    if bytes_read > metadata.len()
                        || total_bytes
                            .checked_add(bytes_read)
                            .is_none_or(|next| next > max_bytes)
                    {
                        return Err(
                            "tree file grew beyond its bounded metadata while hashing".to_string()
                        );
                    }
                    hasher.update(&buffer[..read]);
                }
                if bytes_read != metadata.len() {
                    return Err("tree file changed length while hashing".to_string());
                }
            }
            file_count += 1;
            total_bytes += file_bytes;
        } else {
            return Err(format!(
                "tree contains an unsupported filesystem object: {}",
                entry.path().display()
            ));
        }
    }
    for relative in overrides.keys() {
        if !root.join(relative).is_file() {
            return Err(format!(
                "expected tree replacement is not an existing ordinary file: {}",
                relative.display()
            ));
        }
    }
    Ok(TreeSummary {
        sha256: format!("sha256:{}", hex_digest(&hasher.finalize())),
        file_count,
        total_bytes,
    })
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err(format!(
            "{label} must be an ordinary directory: {}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|error| format!("resolve {label} {}: {error}", path.display()))
}

fn directory_has_entries(path: &Path) -> Result<bool, String> {
    fs::read_dir(path)
        .map_err(|error| format!("read directory {}: {error}", path.display()))
        .map(|mut entries| entries.next().is_some())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceProfile {
    schema: String,
    profile_id: String,
    resources: BTreeMap<String, ResourceSpec>,
    replacements: Vec<ReplacementSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceSpec {
    #[serde(default)]
    path: Option<String>,
    expected_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplacementSpec {
    operation: String,
    table: String,
    partition: String,
    expected_before_sha256: String,
    template: String,
    expected_connector: String,
    resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileClaim {
    path: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedSource {
    project_root: String,
    pbip_relative: String,
    closure_sha256: String,
    files: Vec<FileClaim>,
}

#[derive(Debug, Clone, Copy)]
enum SelectedArtifactKind {
    Report,
    SemanticModel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedResource {
    source: FileClaim,
    output_relative: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlannedReplacement {
    table: String,
    partition: String,
    expected_before_sha256: String,
    template: String,
    expected_connector: String,
    resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowPlan {
    schema: String,
    plan_fingerprint: String,
    policy: String,
    profile_id: String,
    profile: FileClaim,
    source: PlannedSource,
    templates: BTreeMap<String, FileClaim>,
    resources: BTreeMap<String, PlannedResource>,
    replacements: Vec<PlannedReplacement>,
    integration_lock_sha256: String,
    output_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ValidationClaim {
    native_version: String,
    native_errors: u64,
    native_warnings: u64,
    official_errors: u64,
    official_warnings: u64,
    official_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowReceipt {
    schema: String,
    receipt_checksum: String,
    plan_fingerprint: String,
    output_tree_sha256: String,
    source_closure_sha256: String,
    model: ModelReceipt,
    validation: ValidationClaim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelReceipt {
    component: String,
    package_version: String,
    server_version: String,
    local_process: bool,
    transport: String,
    children_reaped: bool,
    pumps_joined: bool,
    forced_cleanup: bool,
    source_before_sha256: String,
    source_after_sha256: String,
    stage_before_sha256: String,
    stage_after_sha256: String,
    expected_stage_sha256: String,
    evidence: EvidenceClaim,
    replacements: Vec<ReplacementReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceClaim {
    path: String,
    definition_sha256: String,
    file_count: usize,
    total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplacementReceipt {
    table: String,
    partition: String,
    before_sha256: String,
    requested_sha256: String,
    readback_sha256: String,
    materialized_sha256: String,
}

struct ExpectedStage {
    before_sha256: String,
    after_sha256: String,
    modified_source_files: BTreeSet<String>,
    requested_sha256: BTreeMap<(String, String), String>,
    requested_semantic_sha256: BTreeMap<(String, String), String>,
}

pub(crate) fn workflow_command(args: &[String]) -> CliResult<Value> {
    match args.split_first() {
        Some((command, rest)) if command == "plan" => workflow_plan(rest),
        Some((command, rest)) if command == "run" => workflow_run(rest),
        Some((command, rest)) if command == "verify" => workflow_verify(rest),
        Some((command, _)) => Err(CliError::invalid_args(format!(
            "unknown workflow command: {command}"
        ))
        .with_hint("Use workflow plan, workflow run, or workflow verify.")),
        None => Err(CliError::invalid_args(
            "workflow requires one subcommand: plan, run, or verify",
        )),
    }
}

fn workflow_plan(args: &[String]) -> CliResult<Value> {
    let options = parse_plan_args(args)?;
    let plan_path = resolve_new_file_candidate(&options.plan_path, "workflow plan")?;
    let output_dir = resolve_new_directory_candidate(&options.output_dir)?;
    validate_credential_free_path(&plan_path, "workflow plan")?;
    validate_credential_free_path(&output_dir, "workflow output")?;
    let output_dir_text = unicode_path(&output_dir, "workflow output")?;
    let resolved = resolve_project(&options.project)?;
    let project_root = canonical_plain_directory(&resolved.project_dir, "project root")?;
    validate_credential_free_path(&project_root, "project root")?;
    if plan_path.starts_with(&project_root) {
        return Err(CliError::invalid_args(
            "workflow plan file must be outside the entire source project root",
        ));
    }
    if paths_overlap(&project_root, &output_dir) {
        return Err(CliError::invalid_args(
            "workflow output must not overlap the source project",
        ));
    }
    let selected_pbip = fs::canonicalize(&resolved.pbip_path)
        .map_err(|error| CliError::unexpected(format!("resolve selected PBIP: {error}")))?;
    let selected_report = fs::canonicalize(&resolved.report_dir)
        .map_err(|error| CliError::unexpected(format!("resolve selected Report: {error}")))?;
    let selected_model = fs::canonicalize(&resolved.semantic_model_dir).map_err(|error| {
        CliError::unexpected(format!("resolve selected SemanticModel: {error}"))
    })?;
    if plan_path == selected_pbip
        || plan_path.starts_with(&selected_report)
        || plan_path.starts_with(&selected_model)
    {
        return Err(CliError::invalid_args(
            "workflow plan file must be outside the selected PBIP artifact closure",
        ));
    }
    let profile_path = canonical_plain_file(&options.profile, "source profile", MAX_PROFILE_BYTES)?;
    validate_credential_free_path(&profile_path, "source profile")?;
    let profile_bytes = read_bounded(&profile_path, MAX_PROFILE_BYTES, "source profile")?;
    let profile_text = std::str::from_utf8(&profile_bytes)
        .map_err(|_| CliError::validation_failed("source profile must be UTF-8 JSON"))?;
    if contains_credential_like_text_str(profile_text) {
        return Err(CliError::validation_failed(
            "source profile contains credential-like content",
        ));
    }
    let profile: SourceProfile = serde_json::from_slice(&profile_bytes).map_err(|error| {
        CliError::validation_failed(format!(
            "parse source profile {}: {error}",
            profile_path.display()
        ))
    })?;
    validate_profile_shape(&profile)?;
    let profile_dir = profile_path.parent().expect("canonical file has parent");
    let resources = resolve_profile_resources(&profile, profile_dir, &options.resources)?;
    let templates = resolve_profile_templates(&profile, profile_dir)?;
    let source = source_manifest(&resolved, &project_root)?;

    for replacement in &profile.replacements {
        let actual = staged_partition_source_fingerprint(
            &resolved.semantic_model_dir,
            &replacement.table,
            &replacement.partition,
        )
        .map_err(|failure| CliError::validation_failed(failure.message().to_string()))?;
        if actual != replacement.expected_before_sha256 {
            return Err(CliError::validation_failed(format!(
                "partition source drift for {}.{}: expected {}, found {}",
                replacement.table,
                replacement.partition,
                replacement.expected_before_sha256,
                actual
            )));
        }
        let template = templates
            .get(&replacement.template)
            .ok_or_else(|| CliError::validation_failed("resolved template is missing"))?;
        let text = read_utf8_claim(template, MAX_TEMPLATE_BYTES, "M template")?;
        validate_template(&text, replacement)?;
    }

    let mut plan = WorkflowPlan {
        schema: WORKFLOW_PLAN_SCHEMA.to_string(),
        plan_fingerprint: String::new(),
        policy: WORKFLOW_POLICY.to_string(),
        profile_id: profile.profile_id.clone(),
        profile: claim_for_file(&profile_path, MAX_PROFILE_BYTES)?,
        source,
        templates,
        resources,
        replacements: profile
            .replacements
            .iter()
            .map(|item| PlannedReplacement {
                table: item.table.clone(),
                partition: item.partition.clone(),
                expected_before_sha256: item.expected_before_sha256.clone(),
                template: item.template.clone(),
                expected_connector: item.expected_connector.clone(),
                resources: item.resources.clone(),
            })
            .collect(),
        integration_lock_sha256: sha256_bytes(INTEGRATION_LOCK_BYTES),
        output_dir: output_dir_text,
    };
    plan.plan_fingerprint = plan_fingerprint(&plan)?;
    write_json_new_atomic(
        &plan_path,
        &serde_json::to_value(&plan).map_err(json_serialize_error)?,
    )?;
    Ok(json!({
        "schema": WORKFLOW_PLAN_SCHEMA,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "profileId": plan.profile_id,
        "plan": canonical_display(&plan_path),
        "planFingerprint": plan.plan_fingerprint,
        "selectedFiles": plan.source.files.len(),
        "resources": plan.resources.len(),
        "replacements": plan.replacements.len(),
        "outputDir": plan.output_dir,
        "next": [format!("powerbi-cli workflow run --plan {} --confirm {} --json", plan_path.display(), plan.plan_fingerprint)]
    }))
}

fn workflow_run(args: &[String]) -> CliResult<Value> {
    let (plan_path, confirmation) = parse_run_args(args)?;
    let plan = load_plan(&plan_path)?;
    if confirmation != plan.plan_fingerprint {
        return Err(CliError::invalid_args(
            "workflow run confirmation does not exactly match the plan fingerprint",
        ));
    }
    verify_plan_inputs(&plan)?;
    let model_tool = resolve_installed_component(MicrosoftComponent::ModelingMcp)?;
    // Resolve both exact sidecars before creating any workflow-owned output.
    let _report_tool = resolve_installed_component(MicrosoftComponent::ReportAuthoring)?;
    let output_dir = PathBuf::from(&plan.output_dir);
    let source_root = PathBuf::from(&plan.source.project_root);
    let output = OwnedWorkflowOutput::create(&output_dir)?;
    let incomplete_identity = match output.write_new_file(
        Path::new(WORKFLOW_INCOMPLETE_FILE),
        b"workflow incomplete; do not publish\n",
        "workflow incomplete marker",
    ) {
        Ok(identity) => identity,
        Err(error) => {
            let cleanup = output
                .cleanup_if_empty()
                .map(|cleanup| format!("; empty output cleanup also failed: {cleanup}"))
                .unwrap_or_default();
            return Err(CliError::unexpected(format!(
                "mark incomplete workflow output: {}{cleanup}",
                error.message
            )));
        }
    };

    copy_claimed_files(&source_root, &output, &plan.source.files)?;
    copy_resources(&plan, &output)?;
    output.verify_root()?;
    let staged_pbip = output_dir.join(&plan.source.pbip_relative);
    let staged = resolve_project(&staged_pbip)?;
    let replacements = materialize_replacements(&plan, &output_dir)?;
    let request = StagedPartitionReplacementRequest {
        source_root: source_semantic_root(&plan)?,
        staged_semantic_model_root: staged.semantic_model_dir.clone(),
        workflow_root: output_dir.clone(),
        fresh_export_root: output_dir.join(WORKFLOW_EVIDENCE_DIR),
        replacements,
    };
    let success = match execute_staged_partition_replacements(&model_tool, &request, true) {
        StagedModelResult::Succeeded(success) => success,
        StagedModelResult::Failed(failure) => {
            return Err(CliError::new(
                "backend_failed",
                crate::EXIT_ORACLE_FAILED,
                format!(
                    "staged model workflow failed during {}: {}",
                    failure.phase,
                    failure.error.message()
                ),
            ));
        }
    };
    output.verify_root()?;
    verify_plan_inputs(&plan)?;
    let output_before_validation = hash_workflow_output(&output_dir)?.sha256;
    let validation = validate_command(&[
        "--strict".to_string(),
        "--backend".to_string(),
        "all".to_string(),
        staged_pbip.to_string_lossy().into_owned(),
    ])?;
    if validation["ok"] != Value::Bool(true) {
        return Err(CliError::validation_failed(
            "workflow output failed required native and official validation",
        ));
    }
    let output_after_validation = hash_workflow_output(&output_dir)?.sha256;
    if output_before_validation != output_after_validation {
        return Err(CliError::validation_failed(
            "a validation backend changed the workflow output",
        ));
    }
    let validation_claim = validation_claim(&validation)?;
    let mcp_contract = model_tool.mcp_contract.as_ref().ok_or_else(|| {
        CliError::unexpected("installed modeling MCP has no exact contract identity")
    })?;
    let mut receipt = WorkflowReceipt {
        schema: WORKFLOW_RECEIPT_SCHEMA.to_string(),
        receipt_checksum: String::new(),
        plan_fingerprint: plan.plan_fingerprint.clone(),
        output_tree_sha256: output_after_validation,
        source_closure_sha256: plan.source.closure_sha256.clone(),
        model: ModelReceipt {
            component: model_tool.component.id().to_string(),
            package_version: model_tool.version.clone(),
            server_version: mcp_contract.server_version.clone(),
            local_process: true,
            transport: model_tool.transport.clone(),
            children_reaped: success.cleanup.children_reaped,
            pumps_joined: success.cleanup.pumps_joined,
            forced_cleanup: success.cleanup.forced,
            source_before_sha256: success.source.before_sha256,
            source_after_sha256: success.source.after_sha256,
            stage_before_sha256: success.stage_definition.before_sha256,
            stage_after_sha256: success.stage_definition.after_sha256,
            expected_stage_sha256: success.expected_stage_sha256,
            evidence: EvidenceClaim {
                path: WORKFLOW_EVIDENCE_DIR.to_string(),
                definition_sha256: success.export.definition_sha256,
                file_count: success.export.file_count,
                total_bytes: success.export.total_bytes,
            },
            replacements: success
                .replacements
                .into_iter()
                .map(|item| ReplacementReceipt {
                    table: item.table,
                    partition: item.partition,
                    before_sha256: item.before_sha256,
                    requested_sha256: item.requested_sha256,
                    readback_sha256: item.readback_sha256,
                    materialized_sha256: item.materialized_sha256,
                })
                .collect(),
        },
        validation: validation_claim,
    };
    validate_receipt_claims(&plan, &receipt, &output_dir)?;
    receipt.receipt_checksum = receipt_checksum(&receipt)?;
    let receipt_bytes = serde_json::to_vec_pretty(&receipt).map_err(json_serialize_error)?;
    output.write_new_file(
        Path::new(WORKFLOW_RECEIPT_FILE),
        &receipt_bytes,
        "workflow receipt",
    )?;
    output.verify_file(
        Path::new(WORKFLOW_RECEIPT_FILE),
        "workflow receipt",
        MAX_PROFILE_BYTES,
    )?;
    output.verify_root()?;
    output.remove_owned_file(
        Path::new(WORKFLOW_INCOMPLETE_FILE),
        &incomplete_identity,
        "workflow incomplete marker",
    )?;
    output.verify_root()?;
    Ok(json!({
        "schema": WORKFLOW_RECEIPT_SCHEMA,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "planFingerprint": receipt.plan_fingerprint,
        "receiptChecksum": receipt.receipt_checksum,
        "outputDir": canonical_display(&output_dir),
        "receipt": canonical_display(&output_dir.join(WORKFLOW_RECEIPT_FILE)),
        "validation": receipt.validation,
        "childrenReaped": receipt.model.children_reaped,
        "pumpsJoined": receipt.model.pumps_joined,
        "next": [format!("powerbi-cli workflow verify --plan {} --json", plan_path.display())]
    }))
}

fn workflow_verify(args: &[String]) -> CliResult<Value> {
    let plan_path = parse_verify_args(args)?;
    let plan = load_plan(&plan_path)?;
    verify_plan_inputs(&plan)?;
    let output_dir = canonical_plain_directory(Path::new(&plan.output_dir), "workflow output")?;
    let incomplete = output_dir.join(WORKFLOW_INCOMPLETE_FILE);
    match fs::symlink_metadata(&incomplete) {
        Ok(metadata) if metadata_is_link_or_reparse(&metadata) => {
            return Err(CliError::validation_failed(
                "workflow incomplete marker is a link or reparse point",
            ));
        }
        Ok(_) => {
            return Err(CliError::validation_failed(
                "workflow output is marked incomplete",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::unexpected(format!(
                "inspect workflow incomplete marker: {error}"
            )));
        }
    }
    let receipt_path = output_dir.join(WORKFLOW_RECEIPT_FILE);
    let receipt: WorkflowReceipt =
        read_json_bounded(&receipt_path, MAX_PROFILE_BYTES, "workflow receipt")?;
    if receipt.schema != WORKFLOW_RECEIPT_SCHEMA
        || receipt.receipt_checksum != receipt_checksum(&receipt)?
        || receipt.plan_fingerprint != plan.plan_fingerprint
        || receipt.source_closure_sha256 != plan.source.closure_sha256
    {
        return Err(CliError::validation_failed(
            "workflow receipt identity or checksum does not match the plan",
        ));
    }
    validate_receipt_claims(&plan, &receipt, &output_dir)?;
    let output_hash = hash_workflow_output(&output_dir)?.sha256;
    if receipt.output_tree_sha256 != output_hash {
        return Err(CliError::validation_failed(
            "workflow output hash does not match the receipt claim",
        ));
    }
    let staged_pbip = output_dir.join(&plan.source.pbip_relative);
    let validation = validate_command(&[
        "--strict".to_string(),
        "--backend".to_string(),
        "all".to_string(),
        staged_pbip.to_string_lossy().into_owned(),
    ])?;
    let validation_now = validation_claim(&validation)?;
    if validation_now != receipt.validation {
        return Err(CliError::validation_failed(
            "workflow validation evidence drifted from the receipt claim",
        ));
    }
    let output_after_validation = hash_workflow_output(&output_dir)?.sha256;
    if output_after_validation != output_hash {
        return Err(CliError::validation_failed(
            "a validation backend changed the workflow output during verification",
        ));
    }
    Ok(json!({
        "schema": "powerbi-cli.workflow-verify.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "planFingerprint": plan.plan_fingerprint,
        "receiptChecksum": receipt.receipt_checksum,
        "outputTreeSha256": output_hash,
        "validation": validation_now,
        "sourceInputsUnchanged": true,
        "receiptClaimsValid": true,
        "evidenceClaimsValid": true
    }))
}

#[derive(Debug)]
struct PlanOptions {
    project: PathBuf,
    profile: PathBuf,
    plan_path: PathBuf,
    output_dir: PathBuf,
    resources: BTreeMap<String, PathBuf>,
}

fn parse_plan_args(args: &[String]) -> CliResult<PlanOptions> {
    let mut project = None;
    let mut profile = None;
    let mut plan_path = None;
    let mut output_dir = None;
    let mut resources = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--project" => set_once(&mut project, PathBuf::from(value), flag)?,
            "--profile" => set_once(&mut profile, PathBuf::from(value), flag)?,
            "--out" => set_once(&mut plan_path, PathBuf::from(value), flag)?,
            "--out-dir" => set_once(&mut output_dir, PathBuf::from(value), flag)?,
            "--resource" => {
                let (name, path) = value
                    .split_once('=')
                    .ok_or_else(|| CliError::invalid_args("--resource must use name=path"))?;
                validate_name(name, "resource")?;
                if resources
                    .insert(name.to_string(), PathBuf::from(path))
                    .is_some()
                {
                    return Err(CliError::invalid_args(format!(
                        "duplicate --resource override: {name}"
                    )));
                }
            }
            _ => {
                return Err(CliError::invalid_args(format!(
                    "unknown workflow plan flag: {flag}"
                )));
            }
        }
        index += 2;
    }
    Ok(PlanOptions {
        project: project
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --project"))?,
        profile: profile
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --profile"))?,
        plan_path: plan_path
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --out"))?,
        output_dir: output_dir
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --out-dir"))?,
        resources,
    })
}

fn parse_run_args(args: &[String]) -> CliResult<(PathBuf, String)> {
    let mut plan = None;
    let mut confirm = None;
    parse_pairs(args, |flag, value| match flag {
        "--plan" => set_once(&mut plan, PathBuf::from(value), flag),
        "--confirm" => set_once(&mut confirm, value.to_string(), flag),
        _ => Err(CliError::invalid_args(format!(
            "unknown workflow run flag: {flag}"
        ))),
    })?;
    Ok((
        plan.ok_or_else(|| CliError::invalid_args("workflow run requires --plan"))?,
        confirm.ok_or_else(|| CliError::invalid_args("workflow run requires --confirm"))?,
    ))
}

fn parse_verify_args(args: &[String]) -> CliResult<PathBuf> {
    let mut plan = None;
    parse_pairs(args, |flag, value| match flag {
        "--plan" => set_once(&mut plan, PathBuf::from(value), flag),
        _ => Err(CliError::invalid_args(format!(
            "unknown workflow verify flag: {flag}"
        ))),
    })?;
    plan.ok_or_else(|| CliError::invalid_args("workflow verify requires --plan"))
}

fn parse_pairs(
    args: &[String],
    mut visit: impl FnMut(&str, &str) -> CliResult<()>,
) -> CliResult<()> {
    if !args.len().is_multiple_of(2) {
        return Err(CliError::invalid_args("workflow flag requires a value"));
    }
    for pair in args.chunks_exact(2) {
        visit(&pair[0], &pair[1])?;
    }
    Ok(())
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> CliResult<()> {
    if slot.replace(value).is_some() {
        Err(CliError::invalid_args(format!(
            "{flag} may be specified only once"
        )))
    } else {
        Ok(())
    }
}

fn validate_profile_shape(profile: &SourceProfile) -> CliResult<()> {
    if profile.schema != SOURCE_PROFILE_SCHEMA {
        return Err(CliError::validation_failed(format!(
            "unsupported source profile schema: {}",
            profile.schema
        )));
    }
    validate_name(&profile.profile_id, "profile ID")?;
    if profile.resources.len() > 32 {
        return Err(CliError::validation_failed(
            "source profile supports at most 32 resources",
        ));
    }
    if profile.replacements.is_empty() || profile.replacements.len() > 100 {
        return Err(CliError::validation_failed(
            "source profile requires between 1 and 100 partition replacements",
        ));
    }
    let mut handles = std::collections::BTreeSet::new();
    let mut referenced_resources = std::collections::BTreeSet::new();
    for (name, resource) in &profile.resources {
        validate_name(name, "resource")?;
        if !is_sha256(&resource.expected_sha256) {
            return Err(CliError::validation_failed(format!(
                "resource {name} requires an exact lowercase expectedSha256"
            )));
        }
    }
    for replacement in &profile.replacements {
        if replacement.operation != "partition.replaceSource" {
            return Err(CliError::validation_failed(
                "the only supported source-profile operation is partition.replaceSource",
            ));
        }
        validate_identifier(&replacement.table, "table")?;
        validate_identifier(&replacement.partition, "partition")?;
        validate_relative_path(&replacement.template, "M template")?;
        validate_connector(&replacement.expected_connector)?;
        if !is_sha256(&replacement.expected_before_sha256) {
            return Err(CliError::validation_failed(format!(
                "invalid expectedBeforeSha256 for {}.{}",
                replacement.table, replacement.partition
            )));
        }
        if !handles.insert(format!("{}\0{}", replacement.table, replacement.partition)) {
            return Err(CliError::validation_failed(
                "duplicate table/partition replacement in source profile",
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for name in &replacement.resources {
            if !profile.resources.contains_key(name) || !names.insert(name) {
                return Err(CliError::validation_failed(format!(
                    "replacement has an unknown or duplicate resource: {name}"
                )));
            }
            referenced_resources.insert(name.as_str());
        }
    }
    if referenced_resources.len() != profile.resources.len() {
        return Err(CliError::validation_failed(
            "every registered source-profile resource must be used by a replacement",
        ));
    }
    Ok(())
}

fn validate_name(value: &str, label: &str) -> CliResult<()> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CliError::validation_failed(format!(
            "{label} must use 1-80 ASCII letters, digits, '-' or '_'"
        )));
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> CliResult<()> {
    if value.trim() != value
        || value.is_empty()
        || value.len() > 256
        || value.contains(['\r', '\n', '\0'])
    {
        return Err(CliError::validation_failed(format!(
            "invalid {label} identifier"
        )));
    }
    Ok(())
}

fn validate_connector(value: &str) -> CliResult<()> {
    const CONNECTORS: &[&str] = &["Excel.Workbook", "PostgreSQL.Database"];
    if !CONNECTORS.contains(&value) {
        return Err(CliError::validation_failed(
            "expectedConnector must name one supported closed connector function",
        ));
    }
    Ok(())
}

fn validate_relative_path(value: &str, label: &str) -> CliResult<PathBuf> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(CliError::validation_failed(format!(
            "{label} must be a profile-relative forward-slash path without '..'"
        )));
    }
    Ok(path.to_path_buf())
}

fn resolve_profile_resources(
    profile: &SourceProfile,
    profile_dir: &Path,
    overrides: &BTreeMap<String, PathBuf>,
) -> CliResult<BTreeMap<String, PlannedResource>> {
    for name in overrides.keys() {
        if !profile.resources.contains_key(name) {
            return Err(CliError::invalid_args(format!(
                "--resource override is not registered by the profile: {name}"
            )));
        }
    }
    let mut result = BTreeMap::new();
    for (name, spec) in &profile.resources {
        let selected = if let Some(path) = overrides.get(name) {
            path.clone()
        } else {
            let relative = spec.path.as_deref().ok_or_else(|| {
                CliError::invalid_args(format!("resource {name} requires --resource {name}=<path>"))
            })?;
            profile_dir.join(validate_relative_path(relative, "resource")?)
        };
        let source = canonical_plain_file(&selected, "resource", MAX_RESOURCE_BYTES)?;
        validate_credential_free_path(&source, "resource")?;
        if !overrides.contains_key(name) && !source.starts_with(profile_dir) {
            return Err(CliError::validation_failed(format!(
                "profile-relative resource escaped the profile directory: {name}"
            )));
        }
        let claim = claim_for_file(&source, MAX_RESOURCE_BYTES)?;
        if claim.sha256 != spec.expected_sha256 {
            return Err(CliError::validation_failed(format!(
                "resource {name} does not match its profile expectedSha256"
            )));
        }
        let file_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| CliError::validation_failed("resource filename must be Unicode"))?;
        result.insert(
            name.clone(),
            PlannedResource {
                source: claim,
                output_relative: format!("resources/{name}/{file_name}"),
            },
        );
    }
    Ok(result)
}

fn resolve_profile_templates(
    profile: &SourceProfile,
    profile_dir: &Path,
) -> CliResult<BTreeMap<String, FileClaim>> {
    let mut result = BTreeMap::new();
    for replacement in &profile.replacements {
        if result.contains_key(&replacement.template) {
            continue;
        }
        let relative = validate_relative_path(&replacement.template, "M template")?;
        let path = canonical_plain_file(
            &profile_dir.join(relative),
            "M template",
            MAX_TEMPLATE_BYTES,
        )?;
        validate_credential_free_path(&path, "M template")?;
        if !path.starts_with(profile_dir) {
            return Err(CliError::validation_failed(
                "profile-relative M template escaped the profile directory",
            ));
        }
        let claim = claim_for_file(&path, MAX_TEMPLATE_BYTES)?;
        let text = read_utf8_claim(&claim, MAX_TEMPLATE_BYTES, "M template")?;
        if contains_credential_like_text_str(&text) {
            return Err(CliError::validation_failed(format!(
                "M template contains credential-like content: {}",
                replacement.template
            )));
        }
        result.insert(replacement.template.clone(), claim);
    }
    Ok(result)
}

fn validate_template(text: &str, replacement: &ReplacementSpec) -> CliResult<()> {
    if text.trim().is_empty() {
        return Err(CliError::validation_failed(format!(
            "M template for {}.{} is empty",
            replacement.table, replacement.partition
        )));
    }
    let tokens = m_tokens(text)?;
    validate_expected_connector_call(&tokens, &replacement.expected_connector)?;
    let placeholders = template_placeholders(text, &tokens)?;
    let expected = replacement
        .resources
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if placeholders != expected {
        return Err(CliError::validation_failed(format!(
            "M template resource placeholders do not exactly match the declared resources for {}.{}",
            replacement.table, replacement.partition
        )));
    }
    match replacement.expected_connector.as_str() {
        "Excel.Workbook" if replacement.resources.len() != 1 => {
            return Err(CliError::validation_failed(
                "Excel.Workbook source templates require exactly one declared file resource",
            ));
        }
        "PostgreSQL.Database" if !replacement.resources.is_empty() => {
            return Err(CliError::validation_failed(
                "PostgreSQL.Database source templates do not accept file resources",
            ));
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MToken {
    Ident(String),
    String(String),
    LParen,
    RParen,
    Comma,
    Equals,
    Other(char),
}

fn m_tokens(text: &str) -> CliResult<Vec<MToken>> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0_usize;
    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        if current.is_whitespace() {
            index += 1;
        } else if current == '/' && next == Some('/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
        } else if current == '/' && next == Some('*') {
            index += 2;
            let mut depth = 1_usize;
            while index < chars.len() && depth != 0 {
                let pair = chars.get(index + 1).copied();
                if chars[index] == '/' && pair == Some('*') {
                    depth = depth.saturating_add(1);
                    index += 2;
                } else if chars[index] == '*' && pair == Some('/') {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            if depth != 0 {
                return Err(CliError::validation_failed(
                    "M template contains an unterminated block comment",
                ));
            }
        } else if current == '"' {
            index += 1;
            let mut value = String::new();
            loop {
                let Some(ch) = chars.get(index).copied() else {
                    return Err(CliError::validation_failed(
                        "M template contains an unterminated string",
                    ));
                };
                if ch == '"' && chars.get(index + 1) == Some(&'"') {
                    value.push('"');
                    index += 2;
                } else if ch == '"' {
                    index += 1;
                    break;
                } else {
                    value.push(ch);
                    index += 1;
                }
            }
            tokens.push(MToken::String(value));
        } else if current.is_ascii_alphabetic() || current == '_' {
            let start = index;
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '_' | '.'))
            {
                index += 1;
            }
            tokens.push(MToken::Ident(chars[start..index].iter().collect()));
        } else {
            tokens.push(match current {
                '(' => MToken::LParen,
                ')' => MToken::RParen,
                ',' => MToken::Comma,
                '=' => MToken::Equals,
                _ => MToken::Other(current),
            });
            index += 1;
        }
    }
    Ok(tokens)
}

fn validate_expected_connector_call(tokens: &[MToken], expected: &str) -> CliResult<()> {
    const SAFE_NAMESPACES: &[&str] = &[
        "Binary",
        "Combiner",
        "Currency",
        "Date",
        "DateTime",
        "DateTimeZone",
        "Decimal",
        "Duration",
        "Int16",
        "Int32",
        "Int64",
        "List",
        "Logical",
        "Number",
        "Percentage",
        "Record",
        "Replacer",
        "Splitter",
        "Table",
        "Text",
        "Time",
        "Type",
        "Uri",
    ];
    if tokens
        .iter()
        .any(|token| matches!(token, MToken::Other('#')))
    {
        return Err(CliError::validation_failed(
            "M template hash intrinsics and #shared indirection are outside the closed source grammar",
        ));
    }
    if tokens.windows(2).any(|pair| {
        matches!(
            pair,
            [MToken::RParen | MToken::String(_), MToken::LParen]
                | [MToken::Other(']' | '}' | '?'), MToken::LParen]
        ) || matches!(pair, [MToken::Other(value), MToken::LParen] if value.is_ascii_digit())
    }) {
        return Err(CliError::validation_failed(
            "M template computed or dynamically selected function invocation is outside the closed source grammar",
        ));
    }
    let calls = tokens
        .windows(2)
        .filter_map(|pair| match pair {
            [MToken::Ident(name), MToken::LParen] => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for identifier in tokens.iter().filter_map(|token| match token {
        MToken::Ident(identifier) if identifier.contains('.') => Some(identifier.as_str()),
        _ => None,
    }) {
        let allowed_source =
            identifier == expected || expected == "Excel.Workbook" && identifier == "File.Contents";
        let safe_transform = identifier
            .split_once('.')
            .is_some_and(|(namespace, _)| SAFE_NAMESPACES.contains(&namespace))
            || identifier == "Value.NativeQuery";
        if !allowed_source && !safe_transform {
            return Err(CliError::validation_failed(format!(
                "M template references unknown, dynamic, or unexpected function {identifier}"
            )));
        }
    }
    for call in &calls {
        let allowed_source =
            *call == expected || expected == "Excel.Workbook" && *call == "File.Contents";
        let safe_transform = call
            .split_once('.')
            .is_some_and(|(namespace, _)| SAFE_NAMESPACES.contains(&namespace))
            || *call == "Value.NativeQuery";
        if !allowed_source && !safe_transform {
            return Err(CliError::validation_failed(format!(
                "M template invokes unknown, dynamic, or unexpected function {call}"
            )));
        }
    }
    if calls.iter().filter(|call| **call == expected).count() != 1 {
        return Err(CliError::validation_failed(format!(
            "M template must execute exactly one root {expected} connector call"
        )));
    }
    let root = tokens.windows(4).position(|items| {
        matches!(items, [MToken::Ident(binding), MToken::Equals, MToken::Ident(connector), MToken::LParen]
            if binding == "Source" && connector == expected)
    });
    let Some(root) = root else {
        return Err(CliError::validation_failed(format!(
            "M template root flow must bind Source directly to {expected}(...)"
        )));
    };
    match expected {
        "Excel.Workbook" => {
            let nested = tokens.get(root + 4..root + 8);
            if !matches!(
                nested,
                Some([
                    MToken::Ident(reader),
                    MToken::LParen,
                    MToken::String(path),
                    MToken::RParen
                ]) if reader == "File.Contents" && resource_placeholder_name(path).is_some()
            ) || calls
                .iter()
                .filter(|call| **call == "File.Contents")
                .count()
                != 1
            {
                return Err(CliError::validation_failed(
                    "Excel.Workbook must receive one File.Contents(\"{{powerbi-cli.resourcePath:name}}\") as its first argument",
                ));
            }
        }
        "PostgreSQL.Database" => {
            if !matches!(
                tokens.get(root + 4..root + 7),
                Some([MToken::String(_), MToken::Comma, MToken::String(_)])
            ) {
                return Err(CliError::validation_failed(
                    "PostgreSQL.Database root flow requires literal server and database names",
                ));
            }
        }
        _ => unreachable!("connector allowlist validated before template parsing"),
    }
    for value in tokens.iter().filter_map(|token| match token {
        MToken::String(value) => Some(value),
        _ => None,
    }) {
        let windows_drive = value.as_bytes().get(1) == Some(&b':')
            && value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic);
        if value.contains("://")
            || value.starts_with(['/', '\\'])
            || value.contains(['/', '\\'])
            || windows_drive
        {
            return Err(CliError::validation_failed(
                "M template contains a hard-coded file or URI path; use a declared resource placeholder",
            ));
        }
    }
    Ok(())
}

fn resource_placeholder_name(value: &str) -> Option<&str> {
    value
        .strip_prefix("{{powerbi-cli.resourcePath:")
        .and_then(|value| value.strip_suffix("}}"))
        .filter(|name| validate_name(name, "resource placeholder").is_ok())
}

fn template_placeholders(
    text: &str,
    tokens: &[MToken],
) -> CliResult<std::collections::BTreeSet<String>> {
    const PREFIX: &str = "{{powerbi-cli.resourcePath:";
    let mut names = std::collections::BTreeSet::new();
    let raw_count = text.matches("{{powerbi-cli.").count();
    let mut token_count = 0_usize;
    for value in tokens.iter().filter_map(|token| match token {
        MToken::String(value) if value.contains("{{powerbi-cli.") => Some(value),
        _ => None,
    }) {
        token_count = token_count.saturating_add(1);
        let Some(name) = value
            .strip_prefix(PREFIX)
            .and_then(|value| value.strip_suffix("}}"))
        else {
            return Err(CliError::validation_failed(
                "resource placeholders must be the complete contents of an M string literal",
            ));
        };
        validate_name(name, "resource placeholder")?;
        names.insert(name.to_string());
    }
    if raw_count != token_count {
        return Err(CliError::validation_failed(
            "resource placeholders are allowed only inside actual M string literals",
        ));
    }
    Ok(names)
}

fn source_manifest(resolved: &crate::ResolvedProject, root: &Path) -> CliResult<PlannedSource> {
    let root = canonical_plain_directory(root, "project root")?;
    let pbip = canonical_plain_file(&resolved.pbip_path, "PBIP", MAX_PROFILE_BYTES)?;
    let report = canonical_plain_directory(&resolved.report_dir, "report artifact")?;
    let model = canonical_plain_directory(&resolved.semantic_model_dir, "semantic model artifact")?;
    for selected in [&pbip, &report, &model] {
        if !selected.starts_with(&root) {
            return Err(CliError::validation_failed(
                "selected PBIP artifact closure escaped its project root",
            ));
        }
    }
    let mut selected = BTreeMap::<String, PathBuf>::new();
    validate_selected_text_file(&pbip, "PBIP")?;
    add_selected_file(&mut selected, &root, &pbip)?;
    add_selected_tree(&mut selected, &root, &report, SelectedArtifactKind::Report)?;
    add_selected_tree(
        &mut selected,
        &root,
        &model,
        SelectedArtifactKind::SemanticModel,
    )?;
    let mut files = Vec::with_capacity(selected.len());
    let mut aggregate = Sha256::new();
    let mut total = 0_u64;
    for (relative, path) in selected {
        let claim = claim_for_file(&path, MAX_RESOURCE_BYTES)?;
        total = total.saturating_add(claim.bytes);
        if files.len() >= MAX_HASHED_TREE_FILES || total > MAX_HASHED_TREE_BYTES {
            return Err(CliError::validation_failed(
                "selected PBIP artifact closure exceeds the file or byte cap",
            ));
        }
        aggregate.update((relative.len() as u64).to_le_bytes());
        aggregate.update(relative.as_bytes());
        aggregate.update(claim.sha256.as_bytes());
        files.push(FileClaim {
            path: relative,
            ..claim
        });
    }
    let pbip_relative = normalized_relative(&root, &pbip)?;
    Ok(PlannedSource {
        project_root: unicode_path(&root, "project root")?,
        pbip_relative,
        closure_sha256: format!("sha256:{}", hex_digest(&aggregate.finalize())),
        files,
    })
}

fn add_selected_tree(
    selected: &mut BTreeMap<String, PathBuf>,
    root: &Path,
    tree: &Path,
    kind: SelectedArtifactKind,
) -> CliResult<()> {
    let mut entries_seen = 0_usize;
    for entry in WalkDir::new(tree).follow_links(false) {
        entries_seen = entries_seen.saturating_add(1);
        if entries_seen
            > MAX_HASHED_TREE_FILES
                .saturating_mul(4)
                .saturating_add(1_024)
        {
            return Err(CliError::validation_failed(
                "selected artifact closure exceeds the filesystem-entry cap",
            ));
        }
        let entry = entry.map_err(|error| {
            CliError::unexpected(format!(
                "walk selected artifact {}: {error}",
                tree.display()
            ))
        })?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            CliError::unexpected(format!(
                "inspect selected artifact {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(CliError::validation_failed(format!(
                "selected artifact closure contains a link or reparse point: {}",
                entry.path().display()
            )));
        }
        let artifact_relative = entry.path().strip_prefix(tree).map_err(|_| {
            CliError::validation_failed("selected artifact path escaped its artifact root")
        })?;
        if artifact_relative.components().any(|component| {
            component.as_os_str().to_str().is_some_and(|part| {
                part.eq_ignore_ascii_case(".git") || part.eq_ignore_ascii_case(".pbi")
            })
        }) {
            return Err(CliError::validation_failed(format!(
                "selected artifact contains a forbidden private/cache directory: {}",
                artifact_relative.display()
            )));
        }
        if metadata.is_file() {
            if !selected_artifact_file_allowed(kind, artifact_relative) {
                return Err(CliError::validation_failed(format!(
                    "selected artifact contains a file outside the narrow PBIR/TMDL closure: {}",
                    artifact_relative.display()
                )));
            }
            if selected_artifact_file_is_text(kind, artifact_relative) {
                validate_selected_text_file(entry.path(), "selected artifact source")?;
            }
            add_selected_file(selected, root, entry.path())?;
            if selected.len() > MAX_HASHED_TREE_FILES {
                return Err(CliError::validation_failed(
                    "selected PBIP artifact closure exceeds the file cap",
                ));
            }
        } else if !metadata.is_dir() {
            return Err(CliError::validation_failed(format!(
                "selected artifact closure contains an unsupported filesystem object: {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

fn selected_artifact_file_allowed(kind: SelectedArtifactKind, relative: &Path) -> bool {
    let parts = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    let file_name = parts.last().copied().unwrap_or_default();
    let lower_name = file_name.to_ascii_lowercase();
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if lower_name == "localsettings.json"
        || matches!(
            extension.as_str(),
            "abf" | "pbix" | "pbit" | "csv" | "xlsx" | "xls" | "parquet" | "db" | "sqlite" | "zip"
        )
    {
        return false;
    }
    match kind {
        SelectedArtifactKind::Report => {
            relative == Path::new("definition.pbir")
                || relative == Path::new(".platform")
                || report_definition_json_allowed(&parts)
                || (parts.first() == Some(&"StaticResources")
                    && matches!(
                        parts.get(1).copied(),
                        Some("RegisteredResources" | "SharedResources")
                    )
                    && parts.len() <= 8
                    && matches!(
                        extension.as_str(),
                        "json" | "png" | "jpg" | "jpeg" | "gif" | "svg" | "woff" | "woff2" | "ttf"
                    ))
        }
        SelectedArtifactKind::SemanticModel => {
            relative == Path::new("definition.pbism")
                || relative == Path::new("diagramLayout.json")
                || relative == Path::new(".platform")
                || (parts.first() == Some(&"definition")
                    && extension == "tmdl"
                    && (parts.len() == 2
                        || (parts.len() == 3 && TMDL_SUBDIRECTORIES.contains(&parts[1]))))
        }
    }
}

fn report_definition_json_allowed(parts: &[&str]) -> bool {
    matches!(
        parts,
        [
            "definition",
            "version.json" | "report.json" | "mobileState.json"
        ] | ["definition", "pages", "pages.json"]
            | ["definition", "bookmarks", "bookmarks.json"]
            | ["definition", "pages", _, "page.json"]
            | ["definition", "bookmarks", _, "bookmark.json"]
            | ["definition", "pages", _, "visuals", _, "visual.json"]
    )
}

fn selected_artifact_file_is_text(kind: SelectedArtifactKind, relative: &Path) -> bool {
    let extension = relative
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "json" | "pbir" | "pbism" | "tmdl" | "svg"
    ) || relative == Path::new(".platform")
        || matches!(kind, SelectedArtifactKind::Report) && relative == Path::new("definition.pbir")
}

fn validate_selected_text_file(path: &Path, label: &str) -> CliResult<()> {
    let bytes = read_bounded(path, MAX_SOURCE_TEXT_BYTES, label)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        CliError::validation_failed(format!("{label} must be UTF-8: {}", path.display()))
    })?;
    if contains_credential_like_text_str(text) {
        return Err(CliError::validation_failed(format!(
            "{label} contains credential-like content: {}",
            path.display()
        )));
    }
    Ok(())
}

fn add_selected_file(
    selected: &mut BTreeMap<String, PathBuf>,
    root: &Path,
    path: &Path,
) -> CliResult<()> {
    let relative = normalized_relative(root, path)?;
    selected.insert(relative, path.to_path_buf());
    Ok(())
}

fn normalized_relative(root: &Path, path: &Path) -> CliResult<String> {
    path.strip_prefix(root)
        .map_err(|_| CliError::validation_failed("selected path escaped project root"))?
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::validation_failed("selected path is empty or not Unicode"))
}

fn load_plan(path: &Path) -> CliResult<WorkflowPlan> {
    let canonical_plan = canonical_plain_file(path, "workflow plan", MAX_PROFILE_BYTES)?;
    validate_credential_free_path(&canonical_plan, "workflow plan")?;
    let plan: WorkflowPlan = read_json_bounded(path, MAX_PROFILE_BYTES, "workflow plan")?;
    if plan.schema != WORKFLOW_PLAN_SCHEMA
        || plan.policy != WORKFLOW_POLICY
        || plan.plan_fingerprint != plan_fingerprint(&plan)?
    {
        return Err(CliError::validation_failed(
            "workflow plan schema, policy, or fingerprint is invalid",
        ));
    }
    if plan.integration_lock_sha256 != sha256_bytes(INTEGRATION_LOCK_BYTES) {
        return Err(CliError::validation_failed(
            "workflow plan was created for a different exact Microsoft integration lock",
        ));
    }
    validate_name(&plan.profile_id, "profile ID")?;
    validate_relative_path(&plan.source.pbip_relative, "planned PBIP")?;
    let source_root = canonical_plain_directory(
        Path::new(&plan.source.project_root),
        "planned source project root",
    )?;
    validate_credential_free_path(&source_root, "planned source project root")?;
    if canonical_plan.starts_with(&source_root) {
        return Err(CliError::validation_failed(
            "workflow plan file is inside the source project root",
        ));
    }
    validate_planned_output_location(&source_root, &plan.output_dir)?;
    Ok(plan)
}

fn validate_planned_output_location(source_root: &Path, output: &str) -> CliResult<PathBuf> {
    let path = Path::new(output);
    if !path.is_absolute() {
        return Err(CliError::validation_failed(
            "planned workflow output must be an absolute canonical path",
        ));
    }
    let resolved = match fs::symlink_metadata(path) {
        Ok(_) => canonical_plain_directory(path, "planned workflow output")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            resolve_new_directory_candidate(path)?
        }
        Err(error) => {
            return Err(CliError::unexpected(format!(
                "inspect planned workflow output {}: {error}",
                path.display()
            )));
        }
    };
    if paths_overlap(source_root, &resolved) {
        return Err(CliError::validation_failed(
            "planned workflow output overlaps the source project root",
        ));
    }
    if Path::new(output) != resolved {
        return Err(CliError::validation_failed(
            "planned workflow output path is not its exact canonical identity",
        ));
    }
    validate_credential_free_path(&resolved, "planned workflow output")?;
    Ok(resolved)
}

fn verify_plan_inputs(plan: &WorkflowPlan) -> CliResult<()> {
    validate_credential_free_path(Path::new(&plan.profile.path), "source profile")?;
    verify_file_claim(&plan.profile, MAX_PROFILE_BYTES, "source profile")?;
    let profile: SourceProfile = read_json_bounded(
        Path::new(&plan.profile.path),
        MAX_PROFILE_BYTES,
        "source profile",
    )?;
    validate_profile_shape(&profile)?;
    if contains_credential_like_text_str(&String::from_utf8_lossy(&read_bounded(
        Path::new(&plan.profile.path),
        MAX_PROFILE_BYTES,
        "source profile",
    )?)) {
        return Err(CliError::validation_failed(
            "source profile content drifted or is no longer safe",
        ));
    }
    validate_profile_derived_plan(plan, &profile)?;
    for claim in plan.templates.values() {
        verify_file_claim(claim, MAX_TEMPLATE_BYTES, "M template")?;
        let text = read_utf8_claim(claim, MAX_TEMPLATE_BYTES, "M template")?;
        if contains_credential_like_text_str(&text) {
            return Err(CliError::validation_failed(
                "M template contains credential-like content",
            ));
        }
    }
    for resource in plan.resources.values() {
        verify_file_claim(&resource.source, MAX_RESOURCE_BYTES, "resource")?;
        validate_relative_path(&resource.output_relative, "resource output")?;
    }
    let root = canonical_plain_directory(Path::new(&plan.source.project_root), "project root")?;
    let pbip = root.join(validate_relative_path(
        &plan.source.pbip_relative,
        "planned PBIP",
    )?);
    let resolved = resolve_project(&pbip)?;
    let current = source_manifest(&resolved, &root)?;
    if current.closure_sha256 != plan.source.closure_sha256
        || current.files != plan.source.files
        || current.pbip_relative != plan.source.pbip_relative
    {
        return Err(CliError::validation_failed(
            "selected PBIP artifact closure drifted after workflow planning",
        ));
    }
    for replacement in &plan.replacements {
        let actual = staged_partition_source_fingerprint(
            &resolved.semantic_model_dir,
            &replacement.table,
            &replacement.partition,
        )
        .map_err(|failure| CliError::validation_failed(failure.message().to_string()))?;
        if actual != replacement.expected_before_sha256 {
            return Err(CliError::validation_failed(format!(
                "partition source drift for {}.{}",
                replacement.table, replacement.partition
            )));
        }
        let claim = plan.templates.get(&replacement.template).ok_or_else(|| {
            CliError::validation_failed("workflow plan references an unknown template")
        })?;
        let text = read_utf8_claim(claim, MAX_TEMPLATE_BYTES, "M template")?;
        validate_planned_template(&text, replacement)?;
    }
    Ok(())
}

fn validate_profile_derived_plan(plan: &WorkflowPlan, profile: &SourceProfile) -> CliResult<()> {
    let profile_path = Path::new(&plan.profile.path);
    let profile_dir = profile_path.parent().ok_or_else(|| {
        CliError::validation_failed("canonical source profile has no parent directory")
    })?;
    if plan.resources.keys().ne(profile.resources.keys()) {
        return Err(CliError::validation_failed(
            "workflow plan resource slots do not exactly match the source profile",
        ));
    }
    let mut overrides = BTreeMap::new();
    for (name, spec) in &profile.resources {
        if spec.path.is_none() {
            let planned = plan.resources.get(name).ok_or_else(|| {
                CliError::validation_failed("workflow plan is missing a profile resource slot")
            })?;
            overrides.insert(name.clone(), PathBuf::from(&planned.source.path));
        }
    }
    let expected_resources = resolve_profile_resources(profile, profile_dir, &overrides)?;
    let expected_templates = resolve_profile_templates(profile, profile_dir)?;
    let expected_replacements = profile
        .replacements
        .iter()
        .map(|item| PlannedReplacement {
            table: item.table.clone(),
            partition: item.partition.clone(),
            expected_before_sha256: item.expected_before_sha256.clone(),
            template: item.template.clone(),
            expected_connector: item.expected_connector.clone(),
            resources: item.resources.clone(),
        })
        .collect::<Vec<_>>();
    if plan.profile_id != profile.profile_id
        || plan.resources != expected_resources
        || plan.templates != expected_templates
        || plan.replacements != expected_replacements
    {
        return Err(CliError::validation_failed(
            "workflow plan semantics do not exactly reconstruct from the current source profile",
        ));
    }
    Ok(())
}

fn validate_planned_template(text: &str, replacement: &PlannedReplacement) -> CliResult<()> {
    validate_template(
        text,
        &ReplacementSpec {
            operation: "partition.replaceSource".to_string(),
            table: replacement.table.clone(),
            partition: replacement.partition.clone(),
            expected_before_sha256: replacement.expected_before_sha256.clone(),
            template: replacement.template.clone(),
            expected_connector: replacement.expected_connector.clone(),
            resources: replacement.resources.clone(),
        },
    )
}

fn source_semantic_root(plan: &WorkflowPlan) -> CliResult<PathBuf> {
    let root = PathBuf::from(&plan.source.project_root);
    let pbip = root.join(&plan.source.pbip_relative);
    resolve_project(&pbip).map(|resolved| resolved.semantic_model_dir)
}

fn copy_claimed_files(
    source: &Path,
    target: &OwnedWorkflowOutput,
    claims: &[FileClaim],
) -> CliResult<()> {
    for claim in claims {
        let relative = validate_relative_path(&claim.path, "selected closure file")?;
        let input = source.join(&relative);
        verify_file_claim(
            &FileClaim {
                path: unicode_path(&input, "selected closure file")?,
                sha256: claim.sha256.clone(),
                bytes: claim.bytes,
            },
            MAX_RESOURCE_BYTES,
            "selected closure file",
        )?;
        copy_new_output_file(&input, target, &relative, claim)?;
    }
    Ok(())
}

fn copy_resources(plan: &WorkflowPlan, output: &OwnedWorkflowOutput) -> CliResult<()> {
    for resource in plan.resources.values() {
        verify_file_claim(&resource.source, MAX_RESOURCE_BYTES, "resource")?;
        let relative = validate_relative_path(&resource.output_relative, "resource output")?;
        copy_new_output_file(
            Path::new(&resource.source.path),
            output,
            &relative,
            &resource.source,
        )?;
    }
    Ok(())
}

fn copy_new_output_file(
    source: &Path,
    output: &OwnedWorkflowOutput,
    relative: &Path,
    expected: &FileClaim,
) -> CliResult<()> {
    let mut input = File::open(source).map_err(|error| {
        CliError::unexpected(format!("open copied source {}: {error}", source.display()))
    })?;
    let mut target = output.create_new_file_after(relative, "workflow-owned copied file", || {})?;
    std::io::copy(&mut input, &mut target)
        .and_then(|_| target.sync_all())
        .map_err(|error| {
            CliError::unexpected(format!(
                "copy {} through the output directory capability to {}: {error}",
                source.display(),
                output.root.join(relative).display()
            ))
        })?;
    drop(target);
    let actual = output.verify_file(relative, "workflow-owned copied file", MAX_RESOURCE_BYTES)?;
    if actual.sha256 != expected.sha256 || actual.bytes != expected.bytes {
        return Err(CliError::validation_failed(format!(
            "copied file failed contained readback: {}",
            output.root.join(relative).display()
        )));
    }
    Ok(())
}

fn materialize_replacements(
    plan: &WorkflowPlan,
    output: &Path,
) -> CliResult<Vec<StagedPartitionReplacement>> {
    let mut replacements = Vec::with_capacity(plan.replacements.len());
    for replacement in &plan.replacements {
        let claim = plan.templates.get(&replacement.template).ok_or_else(|| {
            CliError::validation_failed("workflow plan references an unknown template")
        })?;
        let mut expression = read_utf8_claim(claim, MAX_TEMPLATE_BYTES, "M template")?;
        validate_planned_template(&expression, replacement)?;
        for name in &replacement.resources {
            let resource = plan.resources.get(name).ok_or_else(|| {
                CliError::validation_failed(format!(
                    "workflow plan references unknown resource {name}"
                ))
            })?;
            let path = canonical_plain_file(
                &output.join(&resource.output_relative),
                "staged resource",
                MAX_RESOURCE_BYTES,
            )?;
            let escaped = m_file_path_content(&path, "staged resource")?;
            expression = expression.replace(
                &format!("{{{{powerbi-cli.resourcePath:{name}}}}}"),
                &escaped,
            );
        }
        if expression.contains("{{powerbi-cli.") || contains_credential_like_text_str(&expression) {
            return Err(CliError::validation_failed(format!(
                "complete transformed M expression failed closed checks for {}.{}",
                replacement.table, replacement.partition
            )));
        }
        replacements.push(StagedPartitionReplacement {
            table: replacement.table.clone(),
            partition: replacement.partition.clone(),
            expected_before_sha256: replacement.expected_before_sha256.clone(),
            complete_m_expression: expression,
        });
    }
    Ok(replacements)
}

fn expected_stage(plan: &WorkflowPlan, output: &Path) -> CliResult<ExpectedStage> {
    let source_root = canonical_plain_directory(
        Path::new(&plan.source.project_root),
        "planned source project root",
    )?;
    let semantic_root = canonical_plain_directory(&source_semantic_root(plan)?, "source model")?;
    let definition =
        canonical_plain_directory(&semantic_root.join("definition"), "source model definition")?;
    let before = validate_tmdl_definition(&definition).map_err(CliError::validation_failed)?;
    let snapshot = SourceTreeSnapshot::capture(&definition).map_err(CliError::validation_failed)?;
    let replacements = materialize_replacements(plan, output)?;
    let docs = load_table_documents_from_semantic_model(&semantic_root)?;
    let mut native_plans = BTreeMap::<PathBuf, MutationPlan>::new();
    let mut requested_sha256 = BTreeMap::new();
    let mut requested_semantic_sha256 = BTreeMap::new();
    for replacement in replacements {
        let selector = PartitionSelector {
            table: Some(replacement.table.clone()),
            name: Some(replacement.partition.clone()),
            ..PartitionSelector::default()
        };
        let native =
            replace_partition_source_plan(&docs, &selector, &replacement.complete_m_expression)?;
        let path = fs::canonicalize(&native.path).map_err(|error| {
            CliError::unexpected(format!(
                "resolve expected staged write {}: {error}",
                native.path.display()
            ))
        })?;
        if !path.starts_with(&definition) {
            return Err(CliError::validation_failed(
                "expected staged partition write escaped the source definition",
            ));
        }
        if let Some(composed) = native_plans.get_mut(&path) {
            let before_block = native.before_block.as_deref().ok_or_else(|| {
                CliError::validation_failed("expected partition plan has no before source block")
            })?;
            let after_block = native.after_block.as_deref().ok_or_else(|| {
                CliError::validation_failed("expected partition plan has no after source block")
            })?;
            let matches = composed
                .new_text
                .match_indices(before_block)
                .map(|(start, _)| start)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(CliError::validation_failed(
                    "same-file expected partition replacements are not uniquely composable",
                ));
            }
            composed
                .new_text
                .replace_range(matches[0]..matches[0] + before_block.len(), after_block);
        } else {
            native_plans.insert(path, native);
        }
        let key = (replacement.table, replacement.partition);
        requested_semantic_sha256.insert(
            key.clone(),
            m_semantic_sha256(&replacement.complete_m_expression)?,
        );
        requested_sha256.insert(
            key,
            source_expression_sha256(&replacement.complete_m_expression),
        );
    }
    let overrides = native_plans
        .values()
        .map(|plan| (plan.path.clone(), plan.new_text.clone()))
        .collect::<Vec<_>>();
    let after_sha256 = snapshot
        .expected_after_sha256(&overrides)
        .map_err(CliError::validation_failed)?;
    let modified_source_files = native_plans
        .keys()
        .map(|path| normalized_relative(&source_root, path))
        .collect::<CliResult<BTreeSet<_>>>()?;
    Ok(ExpectedStage {
        before_sha256: before.sha256,
        after_sha256,
        modified_source_files,
        requested_sha256,
        requested_semantic_sha256,
    })
}

fn source_expression_sha256(value: &str) -> String {
    let normalized = value
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    sha256_bytes(normalized.trim_matches('\n').as_bytes())
}

fn m_semantic_sha256(value: &str) -> CliResult<String> {
    let tokens = m_tokens(value)?;
    let mut bytes = Vec::new();
    for token in tokens {
        match token {
            MToken::Ident(value) => {
                bytes.push(b'i');
                bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
                bytes.extend_from_slice(value.as_bytes());
            }
            MToken::String(value) => {
                bytes.push(b's');
                bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
                bytes.extend_from_slice(value.as_bytes());
            }
            MToken::LParen => bytes.push(b'('),
            MToken::RParen => bytes.push(b')'),
            MToken::Comma => bytes.push(b','),
            MToken::Equals => bytes.push(b'='),
            MToken::Other(value) => {
                bytes.push(b'o');
                bytes.extend_from_slice(&(value as u32).to_le_bytes());
            }
        }
    }
    Ok(sha256_bytes(&bytes))
}

fn partition_source_semantic_sha256(
    semantic_model_root: &Path,
    table: &str,
    partition: &str,
) -> CliResult<String> {
    let docs = load_table_documents_from_semantic_model(semantic_model_root)?;
    let record = find_partition(
        &docs,
        &PartitionSelector {
            table: Some(table.to_string()),
            name: Some(partition.to_string()),
            ..PartitionSelector::default()
        },
    )?;
    let source = record.source.as_deref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "partition has no complete M source: {table}.{partition}"
        ))
    })?;
    m_semantic_sha256(source)
}

fn validation_claim(validation: &Value) -> CliResult<ValidationClaim> {
    let official = validation
        .pointer("/validators/microsoftReport")
        .ok_or_else(|| {
            CliError::validation_failed("validation result lacks official backend evidence")
        })?;
    let native_errors = validation["errors"]
        .as_array()
        .map_or(0, |items| items.len()) as u64;
    let native_warnings = validation["warnings"]
        .as_array()
        .map_or(0, |items| items.len()) as u64;
    let official_errors = official
        .pointer("/counts/errors")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            CliError::validation_failed("official validation result lacks error count")
        })?;
    let official_warnings = official
        .pointer("/counts/warnings")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            CliError::validation_failed("official validation result lacks warning count")
        })?;
    let official_version = official["version"].as_str().ok_or_else(|| {
        CliError::validation_failed("official validation result lacks exact version")
    })?;
    Ok(ValidationClaim {
        native_version: env!("CARGO_PKG_VERSION").to_string(),
        native_errors,
        native_warnings,
        official_errors,
        official_warnings,
        official_version: official_version.to_string(),
    })
}

fn validate_receipt_claims(
    plan: &WorkflowPlan,
    receipt: &WorkflowReceipt,
    output: &Path,
) -> CliResult<()> {
    validate_receipt_semantics(plan, receipt)?;
    let expected = expected_stage(plan, output)?;
    if receipt.model.stage_before_sha256 != expected.before_sha256
        || receipt.model.stage_after_sha256 != expected.after_sha256
        || receipt.model.expected_stage_sha256 != expected.after_sha256
    {
        return Err(CliError::validation_failed(
            "workflow stage hashes do not reconstruct from the profile, source, and resources",
        ));
    }
    let model_tool = resolve_installed_component(MicrosoftComponent::ModelingMcp)?;
    let report_tool = resolve_installed_component(MicrosoftComponent::ReportAuthoring)?;
    let model_contract = model_tool.mcp_contract.as_ref().ok_or_else(|| {
        CliError::validation_failed("installed modeling MCP lacks exact contract identity")
    })?;
    if receipt.model.package_version != model_tool.version
        || receipt.model.server_version != model_contract.server_version
        || receipt.model.transport != model_tool.transport
        || receipt.validation.official_version != report_tool.version
    {
        return Err(CliError::validation_failed(
            "workflow receipt backend versions do not match the exact installed sidecars",
        ));
    }
    let staged = resolve_project(&output.join(&plan.source.pbip_relative))?;
    let staged_definition = validate_tmdl_definition(&staged.semantic_model_dir.join("definition"))
        .map_err(CliError::validation_failed)?;
    if staged_definition.sha256 != expected.after_sha256 {
        return Err(CliError::validation_failed(
            "actual staged definition tree does not match the reconstructed expected stage",
        ));
    }
    verify_staged_copies(plan, output, &expected.modified_source_files)?;
    let evidence = validate_evidence_claim(output, &receipt.model.evidence)?;
    if evidence.definition_sha256 != receipt.model.evidence.definition_sha256
        || evidence.file_count != receipt.model.evidence.file_count
        || evidence.total_bytes != receipt.model.evidence.total_bytes
    {
        return Err(CliError::validation_failed(
            "workflow model evidence does not match the receipt claim",
        ));
    }
    let proof_scratch = tempfile::Builder::new()
        .prefix("powerbi-cli-model-proof-")
        .tempdir()
        .map_err(|error| {
            CliError::unexpected(format!(
                "create private canonical model-proof directory: {error}"
            ))
        })?;
    let canonical_export = execute_staged_model_export_proof(
        &model_tool,
        &source_semantic_root(plan)?,
        &staged.semantic_model_dir,
        proof_scratch.path(),
    )
    .map_err(|error| {
        CliError::validation_failed(format!(
            "derive canonical staged-model export proof: {}",
            error.message()
        ))
    })?;
    validate_canonical_export_binding(&evidence, &canonical_export)?;
    let evidence_after_proof = validate_evidence_claim(output, &receipt.model.evidence)?;
    validate_canonical_export_binding(&evidence_after_proof, &canonical_export)?;
    for replacement in &receipt.model.replacements {
        let key = (replacement.table.clone(), replacement.partition.clone());
        if expected.requested_sha256.get(&key) != Some(&replacement.requested_sha256) {
            return Err(CliError::validation_failed(format!(
                "partition request hash does not reconstruct from the current profile: {}.{}",
                replacement.table, replacement.partition
            )));
        }
        let current = staged_partition_source_fingerprint(
            &staged.semantic_model_dir,
            &replacement.table,
            &replacement.partition,
        )
        .map_err(|failure| CliError::validation_failed(failure.message().to_string()))?;
        if current != replacement.materialized_sha256 {
            return Err(CliError::validation_failed(format!(
                "materialized partition evidence does not match output readback: {}.{}",
                replacement.table, replacement.partition
            )));
        }
        let exported = partition_source_semantic_sha256(
            &evidence.export_root,
            &replacement.table,
            &replacement.partition,
        )?;
        if expected.requested_semantic_sha256.get(&key) != Some(&exported) {
            return Err(CliError::validation_failed(format!(
                "exported model evidence is not semantically the partition readback: {}.{}",
                replacement.table, replacement.partition
            )));
        }
    }
    Ok(())
}

fn validate_canonical_export_binding(
    evidence: &ExportShapeProof,
    canonical_export: &ExportShapeProof,
) -> CliResult<()> {
    if canonical_export.definition_sha256 != evidence.definition_sha256
        || canonical_export.file_count != evidence.file_count
        || canonical_export.total_bytes != evidence.total_bytes
    {
        return Err(CliError::validation_failed(
            "workflow model evidence is not the exact canonical export of the staged model",
        ));
    }
    Ok(())
}

fn verify_staged_copies(
    plan: &WorkflowPlan,
    output: &Path,
    modified_source_files: &BTreeSet<String>,
) -> CliResult<()> {
    let source_paths = plan
        .source
        .files
        .iter()
        .map(|claim| claim.path.as_str())
        .collect::<BTreeSet<_>>();
    let resource_paths = plan
        .resources
        .values()
        .map(|resource| resource.output_relative.as_str())
        .collect::<BTreeSet<_>>();
    for claim in &plan.source.files {
        if modified_source_files.contains(&claim.path) {
            continue;
        }
        let actual = claim_for_file(
            &output.join(validate_relative_path(&claim.path, "staged closure file")?),
            MAX_RESOURCE_BYTES,
        )?;
        if actual.sha256 != claim.sha256 || actual.bytes != claim.bytes {
            return Err(CliError::validation_failed(format!(
                "staged closure file differs from its planned source: {}",
                claim.path
            )));
        }
    }
    for resource in plan.resources.values() {
        let actual = claim_for_file(
            &output.join(validate_relative_path(
                &resource.output_relative,
                "staged resource",
            )?),
            MAX_RESOURCE_BYTES,
        )?;
        if actual.sha256 != resource.source.sha256 || actual.bytes != resource.source.bytes {
            return Err(CliError::validation_failed(
                "staged resource differs from its profile-bound source",
            ));
        }
    }
    for entry in WalkDir::new(output).follow_links(false) {
        let entry = entry.map_err(|error| {
            CliError::unexpected(format!("walk staged workflow output: {error}"))
        })?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            CliError::unexpected(format!(
                "inspect staged workflow output {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata_is_link_or_reparse(&metadata) {
            return Err(CliError::validation_failed(format!(
                "workflow output contains a link or reparse point: {}",
                entry.path().display()
            )));
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(CliError::validation_failed(format!(
                "workflow output contains an unsupported filesystem object: {}",
                entry.path().display()
            )));
        }
        let relative = normalized_relative(output, entry.path())?;
        let allowed = source_paths.contains(relative.as_str())
            || resource_paths.contains(relative.as_str())
            || relative == WORKFLOW_RECEIPT_FILE
            || relative == WORKFLOW_INCOMPLETE_FILE
            || relative.starts_with(&format!("{WORKFLOW_EVIDENCE_DIR}/"));
        if !allowed {
            return Err(CliError::validation_failed(format!(
                "workflow output contains a file outside its planned closure: {relative}"
            )));
        }
    }
    Ok(())
}

fn validate_receipt_semantics(plan: &WorkflowPlan, receipt: &WorkflowReceipt) -> CliResult<()> {
    if receipt.model.component != "modeling-mcp"
        || !receipt.model.local_process
        || receipt.model.transport != "stdio"
        || !receipt.model.children_reaped
        || !receipt.model.pumps_joined
        || receipt.model.source_before_sha256 != receipt.model.source_after_sha256
        || receipt.model.stage_after_sha256 != receipt.model.expected_stage_sha256
        || receipt.model.evidence.path != WORKFLOW_EVIDENCE_DIR
        || receipt.validation.native_version != env!("CARGO_PKG_VERSION")
        || receipt.validation.native_errors != 0
        || receipt.validation.official_errors != 0
        || receipt.model.replacements.len() != plan.replacements.len()
    {
        return Err(CliError::validation_failed(
            "workflow receipt semantic invariants are not satisfied",
        ));
    }
    for hash in [
        &receipt.output_tree_sha256,
        &receipt.source_closure_sha256,
        &receipt.model.source_before_sha256,
        &receipt.model.source_after_sha256,
        &receipt.model.stage_before_sha256,
        &receipt.model.stage_after_sha256,
        &receipt.model.expected_stage_sha256,
        &receipt.model.evidence.definition_sha256,
    ] {
        if !is_sha256(hash) {
            return Err(CliError::validation_failed(
                "workflow receipt contains an invalid evidence hash",
            ));
        }
    }
    for (planned, observed) in plan.replacements.iter().zip(&receipt.model.replacements) {
        if observed.table != planned.table
            || observed.partition != planned.partition
            || observed.before_sha256 != planned.expected_before_sha256
            || observed.requested_sha256 != observed.readback_sha256
            || observed.requested_sha256 != observed.materialized_sha256
            || !is_sha256(&observed.requested_sha256)
        {
            return Err(CliError::validation_failed(format!(
                "workflow receipt partition evidence is inconsistent: {}.{}",
                planned.table, planned.partition
            )));
        }
    }
    Ok(())
}

fn validate_evidence_claim(output: &Path, claim: &EvidenceClaim) -> CliResult<ExportShapeProof> {
    let relative = validate_relative_path(&claim.path, "evidence path")?;
    if relative.components().count() != 1 {
        return Err(CliError::validation_failed(
            "model evidence must be one direct output child",
        ));
    }
    let export_root = canonical_plain_directory(&output.join(relative), "model evidence")?;
    let definition =
        canonical_plain_directory(&export_root.join("definition"), "model evidence definition")?;
    let mut root_entries = fs::read_dir(&export_root)
        .map_err(|error| CliError::unexpected(format!("read model evidence: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| CliError::unexpected(format!("read model evidence entry: {error}")))?;
    root_entries.sort_by_key(|entry| entry.file_name());
    if root_entries.len() != 1 || root_entries[0].file_name() != "definition" {
        return Err(CliError::validation_failed(
            "model evidence root must contain exactly definition/",
        ));
    }
    let summary = validate_tmdl_definition(&definition).map_err(CliError::validation_failed)?;
    Ok(ExportShapeProof {
        export_root,
        definition_sha256: summary.sha256,
        file_count: summary.file_count,
        total_bytes: summary.total_bytes,
    })
}

fn plan_fingerprint(plan: &WorkflowPlan) -> CliResult<String> {
    let mut payload = plan.clone();
    payload.plan_fingerprint.clear();
    serde_json::to_vec(&payload)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(json_serialize_error)
}

fn receipt_checksum(receipt: &WorkflowReceipt) -> CliResult<String> {
    let mut payload = receipt.clone();
    payload.receipt_checksum.clear();
    serde_json::to_vec(&payload)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(json_serialize_error)
}

fn json_serialize_error(error: serde_json::Error) -> CliError {
    CliError::unexpected(format!("serialize workflow JSON: {error}"))
}

fn read_json_bounded<T: serde::de::DeserializeOwned>(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> CliResult<T> {
    let bytes = read_bounded(path, max_bytes, label)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::validation_failed(format!("parse {label} {}: {error}", path.display()))
    })
}

fn read_bounded(path: &Path, max_bytes: u64, label: &str) -> CliResult<Vec<u8>> {
    let path = canonical_plain_file(path, label, max_bytes)?;
    let mut file = File::open(&path).map_err(|error| {
        CliError::file_not_found(format!("open {label} {}: {error}", path.display()))
    })?;
    let expected_len = file
        .metadata()
        .map_err(|error| {
            CliError::unexpected(format!("inspect {label} {}: {error}", path.display()))
        })?
        .len();
    let mut bytes = Vec::with_capacity(expected_len.min(max_bytes) as usize);
    std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::file_not_found(format!("read {label} {}: {error}", path.display()))
        })?;
    if bytes.len() as u64 > max_bytes || bytes.len() as u64 != expected_len {
        return Err(CliError::validation_failed(format!(
            "{label} changed length or exceeded {max_bytes} bytes while being read: {}",
            path.display()
        )));
    }
    Ok(bytes)
}

fn read_utf8_claim(claim: &FileClaim, max_bytes: u64, label: &str) -> CliResult<String> {
    verify_file_claim(claim, max_bytes, label)?;
    let bytes = read_bounded(Path::new(&claim.path), max_bytes, label)?;
    String::from_utf8(bytes)
        .map_err(|_| CliError::validation_failed(format!("{label} must be UTF-8")))
}

fn claim_for_file(path: &Path, max_bytes: u64) -> CliResult<FileClaim> {
    let path = canonical_plain_file(path, "input file", max_bytes)?;
    let metadata = fs::metadata(&path)
        .map_err(|error| CliError::unexpected(format!("inspect {}: {error}", path.display())))?;
    Ok(FileClaim {
        path: unicode_path(&path, "input file")?,
        sha256: sha256_file_bounded(&path, max_bytes, metadata.len())?,
        bytes: metadata.len(),
    })
}

fn unicode_path(path: &Path, label: &str) -> CliResult<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        CliError::validation_failed(format!(
            "{label} path must be Unicode; lossy paths are never persisted or sent across a mutation boundary"
        ))
    })
}

fn validate_credential_free_path(path: &Path, label: &str) -> CliResult<()> {
    let value = unicode_path(path, label)?;
    if contains_credential_like_text_str(&value) {
        return Err(CliError::validation_failed(format!(
            "{label} path contains credential-like content"
        )));
    }
    Ok(())
}

fn m_string_content(value: &str) -> CliResult<String> {
    if value.chars().any(char::is_control) {
        return Err(CliError::validation_failed(
            "staged resource path contains a control character that cannot cross the M boundary",
        ));
    }
    Ok(value.replace('#', "#(0023)").replace('"', "\"\""))
}

fn m_file_path_content(path: &Path, label: &str) -> CliResult<String> {
    let canonical = unicode_path(path, label)?;
    let power_query_path = if let Some(stripped) = canonical.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{stripped}")
    } else if let Some(stripped) = canonical.strip_prefix(r"\\?\") {
        stripped.to_owned()
    } else {
        canonical
    };
    m_string_content(&power_query_path)
}

fn verify_file_claim(claim: &FileClaim, max_bytes: u64, label: &str) -> CliResult<()> {
    let actual = claim_for_file(Path::new(&claim.path), max_bytes)?;
    if actual.path != claim.path || actual.bytes != claim.bytes || actual.sha256 != claim.sha256 {
        return Err(CliError::validation_failed(format!(
            "{label} drifted after workflow planning: {}",
            claim.path
        )));
    }
    Ok(())
}

#[cfg(test)]
fn sha256_file(path: &Path) -> CliResult<String> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::file_not_found(format!("inspect {}: {error}", path.display()))
    })?;
    sha256_file_bounded(path, MAX_RESOURCE_BYTES, metadata.len())
}

fn sha256_file_bounded(path: &Path, max_bytes: u64, expected_len: u64) -> CliResult<String> {
    let mut file = File::open(path)
        .map_err(|error| CliError::file_not_found(format!("open {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| CliError::unexpected(format!("read {}: {error}", path.display())))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes || total > expected_len {
            return Err(CliError::validation_failed(format!(
                "file grew beyond its bounded metadata while hashing: {}",
                path.display()
            )));
        }
        hasher.update(&buffer[..read]);
    }
    if total != expected_len {
        return Err(CliError::validation_failed(format!(
            "file changed length while hashing: {}",
            path.display()
        )));
    }
    Ok(format!("sha256:{}", hex_digest(&hasher.finalize())))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex_digest(&hasher.finalize()))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn canonical_plain_file(path: &Path, label: &str, max_bytes: u64) -> CliResult<PathBuf> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::file_not_found(format!("inspect {label} {}: {error}", path.display()))
    })?;
    if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) || metadata.len() > max_bytes {
        return Err(CliError::validation_failed(format!(
            "{label} must be an ordinary file no larger than {max_bytes} bytes: {}",
            path.display()
        )));
    }
    fs::canonicalize(path).map_err(|error| {
        CliError::unexpected(format!("resolve {label} {}: {error}", path.display()))
    })
}

fn canonical_plain_directory(path: &Path, label: &str) -> CliResult<PathBuf> {
    canonical_directory(path, label).map_err(CliError::validation_failed)
}

fn resolve_new_directory_candidate(path: &Path) -> CliResult<PathBuf> {
    require_absent(path, "workflow output")?;
    let name = path
        .file_name()
        .ok_or_else(|| CliError::invalid_args("workflow output needs a directory name"))?;
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = canonical_plain_directory(parent, "workflow output parent")?;
    Ok(parent.join(name))
}

fn resolve_new_file_candidate(path: &Path, label: &str) -> CliResult<PathBuf> {
    require_absent(path, label)?;
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = canonical_plain_directory(parent, &format!("{label} parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| CliError::invalid_args(format!("{label} needs a filename")))?;
    Ok(parent.join(name))
}

fn require_absent(path: &Path, label: &str) -> CliResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(CliError::invalid_args(format!(
            "{label} already exists and will not be replaced: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::unexpected(format!(
            "inspect {label} {}: {error}",
            path.display()
        ))),
    }
}

fn hash_workflow_output(output: &Path) -> CliResult<TreeSummary> {
    hash_tree_with_exclusions(
        output,
        &[
            Path::new(WORKFLOW_RECEIPT_FILE),
            Path::new(WORKFLOW_INCOMPLETE_FILE),
        ],
    )
    .map_err(CliError::validation_failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_profile_needs_no_resource_and_registered_resources_must_be_used() {
        let mut profile = SourceProfile {
            schema: SOURCE_PROFILE_SCHEMA.into(),
            profile_id: "postgres-work".into(),
            resources: BTreeMap::new(),
            replacements: vec![ReplacementSpec {
                operation: "partition.replaceSource".into(),
                table: "FactSales".into(),
                partition: "FactSales".into(),
                expected_before_sha256:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
                template: "templates/FactSales.m".into(),
                expected_connector: "PostgreSQL.Database".into(),
                resources: Vec::new(),
            }],
        };
        assert!(validate_profile_shape(&profile).is_ok());
        profile
            .resources
            .insert(
                "unused".into(),
                ResourceSpec {
                    path: None,
                    expected_sha256:
                        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                            .into(),
                },
            );
        assert!(validate_profile_shape(&profile).is_err());
    }

    #[test]
    fn staged_resource_path_is_encoded_as_m_string_content() {
        assert_eq!(
            m_string_content("C:\\data\\book#(cr)\"copy.xlsx").expect("M content"),
            "C:\\data\\book#(0023)(cr)\"\"copy.xlsx"
        );
        assert!(m_string_content("bad\npath").is_err());
    }

    #[test]
    fn staged_resource_path_uses_power_query_compatible_windows_spelling() {
        assert_eq!(
            m_file_path_content(Path::new(r"\\?\C:\data\book.xlsx"), "resource")
                .expect("drive path"),
            r"C:\data\book.xlsx"
        );
        assert_eq!(
            m_file_path_content(Path::new(r"\\?\UNC\server\share\book.xlsx"), "resource")
                .expect("UNC path"),
            r"\\server\share\book.xlsx"
        );
        assert_eq!(
            m_file_path_content(Path::new(r"C:\data\book.xlsx"), "resource")
                .expect("ordinary path"),
            r"C:\data\book.xlsx"
        );
    }

    #[test]
    fn export_guard_accepts_only_fresh_definition_shape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        let export = workflow.join("mcp-export");
        let prepared = PreparedStagedModel::prepare(&source, &stage, &workflow, &export)
            .expect("prepared paths")
            .commit();
        copy_definition(&stage.join("definition"), &export.join("definition"));
        let proof = prepared.validate_export().expect("valid export");
        assert_eq!(proof.file_count, 3);

        fs::remove_dir_all(export.join("definition")).expect("remove definition");
        fs::write(export.join("database.tmdl"), "database Unsafe").expect("root-level tmdl");
        assert!(prepared.validate_export().is_err());
    }

    #[test]
    fn protected_or_existing_export_targets_are_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        fs::create_dir(workflow.join("occupied")).expect("occupied");
        fs::write(workflow.join("occupied").join("keep.txt"), "keep").expect("occupied file");
        assert!(
            PreparedStagedModel::prepare(&source, &stage, &workflow, &workflow.join("occupied"))
                .is_err()
        );
        assert!(
            PreparedStagedModel::prepare(&source, &stage, &workflow, &stage.join("definition"))
                .is_err()
        );
    }

    #[test]
    fn failed_export_reservation_cleans_only_owned_state_and_retry_succeeds() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        let export = workflow.join("mcp-export");
        fs::create_dir(&export).expect("preexisting empty export");
        assert!(PreparedStagedModel::prepare(&source, &stage, &workflow, &export).is_err());
        assert!(export.is_dir(), "preexisting caller directory was removed");
        assert!(!workflow.join(".mcp-export.powerbi-cli-quarantine").exists());
        fs::remove_dir(&export).expect("remove caller directory");
        let prepared = PreparedStagedModel::prepare(&source, &stage, &workflow, &export)
            .expect("retry after failed reservation")
            .commit();
        assert!(prepared.export_root.join("definition").is_dir());
    }

    #[test]
    fn export_guard_never_deletes_a_replacement_at_the_reserved_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let stage = temp.path().join("stage.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_fixture(&source);
        copy_fixture(&stage);
        fs::create_dir(&workflow).expect("workflow");
        let export = workflow.join("mcp-export");
        let reservation = PreparedStagedModel::prepare(&source, &stage, &workflow, &export)
            .expect("prepared paths");
        let moved_owned = workflow.join("owned-moved-away");
        fs::rename(&export, &moved_owned).expect("move the originally owned directory");
        fs::create_dir(&export).expect("replacement export directory");
        fs::write(export.join("keep.txt"), "foreign replacement").expect("replacement content");

        drop(reservation);

        assert_eq!(
            fs::read_to_string(export.join("keep.txt")).expect("replacement survives"),
            "foreign replacement"
        );
        assert!(moved_owned.join("definition").is_dir());
        assert!(!workflow.join(".mcp-export.powerbi-cli-cleanup").exists());
        assert!(!workflow.join(".mcp-export.powerbi-cli-quarantine").exists());
    }

    #[test]
    fn source_snapshot_proves_byte_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        copy_fixture(&source);
        let snapshot = SourceTreeSnapshot::capture(&source).expect("snapshot");
        let unchanged = snapshot.verify().expect("verify unchanged");
        assert!(unchanged.byte_identical);
        fs::write(source.join("definition.pbism"), "changed").expect("change source");
        let changed = snapshot.verify().expect("verify changed");
        assert!(!changed.byte_identical);
    }

    #[test]
    fn source_profile_plan_is_deterministic_and_selects_only_the_pbip_closure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        fs::create_dir(fixture.project.join("Sibling.Report")).expect("sibling");
        fs::write(
            fixture
                .project
                .join("Sibling.Report")
                .join("do-not-copy.json"),
            "{}",
        )
        .expect("sibling file");
        fs::create_dir(fixture.project.join(".git")).expect("git dir");
        fs::write(fixture.project.join(".git").join("config"), "private").expect("git config");
        fs::create_dir(fixture.project.join("data")).expect("data dir");
        fs::write(
            fixture.project.join("data").join("unregistered.xlsx"),
            "private",
        )
        .expect("unregistered data");
        let source_before = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project root"),
        )
        .expect("before manifest");

        let first = temp.path().join("first.plan.json");
        let second = temp.path().join("second.plan.json");
        let output = temp.path().join("output");
        let a = plan_fixture(&fixture, &first, &output).expect("first plan");
        let b = plan_fixture(&fixture, &second, &output).expect("second plan");
        assert_eq!(a["planFingerprint"], b["planFingerprint"]);
        assert_eq!(
            fs::read(&first).expect("first"),
            fs::read(&second).expect("second")
        );
        let plan = load_plan(&first).expect("load plan");
        assert!(plan.source.files.iter().all(|file| {
            !file.path.starts_with("Sibling.Report/")
                && !file.path.starts_with(".git/")
                && !file.path.starts_with("data/")
        }));
        let source_after = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project root"),
        )
        .expect("after manifest");
        assert_eq!(source_before.closure_sha256, source_after.closure_sha256);
    }

    #[test]
    fn plan_rejects_drift_overwrite_path_escape_and_credentials() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        assert!(plan_fixture(&fixture, &plan_path, &output).is_err());
        fs::write(&fixture.template, "let Password = \"secret\" in Password")
            .expect("credential template");
        let plan = load_plan(&plan_path).expect("load fingerprinted plan");
        assert!(
            verify_plan_inputs(&plan).is_err(),
            "template drift must fail"
        );

        let unsafe_profile = temp.path().join("unsafe-profile.json");
        let mut value: Value =
            serde_json::from_slice(&fs::read(&fixture.profile).expect("profile"))
                .expect("profile JSON");
        value["resources"]["workbook"]["path"] = Value::String("../outside.xlsx".into());
        fs::write(
            &unsafe_profile,
            serde_json::to_vec_pretty(&value).expect("unsafe JSON"),
        )
        .expect("unsafe profile");
        let args = plan_args(
            &fixture.pbip,
            &unsafe_profile,
            &temp.path().join("unsafe.plan.json"),
            &temp.path().join("unsafe-output"),
        );
        assert!(
            workflow_plan(&args).is_err(),
            "profile path escape must fail"
        );

        fs::create_dir(&output).expect("occupied output");
        let another = temp.path().join("another.plan.json");
        assert!(plan_fixture(&fixture, &another, &output).is_err());
    }

    #[test]
    fn credential_like_override_path_is_rejected_before_plan_or_output_persistence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let override_path = temp.path().join("password=secret.xlsx");
        fs::write(&override_path, "neutral bytes").expect("override resource");
        let plan_path = temp.path().join("credential-path.plan.json");
        let output = temp.path().join("credential-path-output");
        let mut args = plan_args(&fixture.pbip, &fixture.profile, &plan_path, &output);
        args.extend([
            "--resource".into(),
            format!("workbook={}", override_path.display()),
        ]);

        assert!(workflow_plan(&args).is_err());
        assert!(!plan_path.exists(), "credential-like path reached the plan");
        assert!(!output.exists(), "credential-like path reached the output");
    }

    #[test]
    fn plan_rejects_unsafe_files_inside_selected_artifacts() {
        for relative in [
            Path::new("Synthetic.Report/.pbi/cache.abf"),
            Path::new("Synthetic.Report/localSettings.json"),
            Path::new("Synthetic.Report/definition/pages/private/data.csv"),
            Path::new("Synthetic.Report/definition/pages/private/data.json"),
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let fixture = workflow_fixture(temp.path());
            let unsafe_path = fixture.project.join(relative);
            fs::create_dir_all(unsafe_path.parent().expect("unsafe parent")).expect("unsafe dirs");
            fs::write(&unsafe_path, "private").expect("unsafe file");
            assert!(
                plan_fixture(
                    &fixture,
                    &temp.path().join("unsafe.plan.json"),
                    &temp.path().join("unsafe-output")
                )
                .is_err(),
                "unsafe selected artifact file was accepted: {}",
                relative.display()
            );
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let table = fixture
            .project
            .join("Synthetic.SemanticModel/definition/tables/Synthetic.tmdl");
        let mut text = fs::read_to_string(&table).expect("table");
        text.push_str("\n\tannotation password = \"secret\"\n");
        fs::write(&table, text).expect("credential-bearing TMDL");
        assert!(
            plan_fixture(
                &fixture,
                &temp.path().join("credential.plan.json"),
                &temp.path().join("credential-output")
            )
            .is_err()
        );

        for (name, bytes) in [
            (
                "credential.svg",
                b"<svg><!-- password=secret --></svg>".as_slice(),
            ),
            ("invalid.svg", &[0xff, 0xfe][..]),
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let fixture = workflow_fixture(temp.path());
            let svg = fixture
                .project
                .join("Synthetic.Report/StaticResources/RegisteredResources")
                .join(name);
            fs::create_dir_all(svg.parent().expect("SVG parent")).expect("SVG directory");
            fs::write(&svg, bytes).expect("unsafe SVG");
            assert!(
                plan_fixture(
                    &fixture,
                    &temp.path().join("unsafe-svg.plan.json"),
                    &temp.path().join("unsafe-svg-output"),
                )
                .is_err(),
                "unsafe SVG text was accepted: {name}"
            );
        }
    }

    #[test]
    fn plan_and_recomputed_fingerprint_cannot_write_inside_source_project() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let inside_plan = fixture.project.join("workflow.plan.json");
        assert!(plan_fixture(&fixture, &inside_plan, &temp.path().join("outside-output")).is_err());
        assert!(!inside_plan.exists());
        assert!(
            plan_fixture(
                &fixture,
                &temp.path().join("inside-output.plan.json"),
                &fixture.project.join("generated-output")
            )
            .is_err()
        );
        assert!(!fixture.project.join("generated-output").exists());

        let plan_path = temp.path().join("normal.plan.json");
        plan_fixture(&fixture, &plan_path, &temp.path().join("normal-output"))
            .expect("normal plan");
        let mut recomputed: WorkflowPlan =
            read_json_bounded(&plan_path, MAX_PROFILE_BYTES, "workflow plan").expect("plan JSON");
        recomputed.output_dir = canonical_display(&fixture.project.join("recomputed-output"));
        recomputed.plan_fingerprint =
            plan_fingerprint(&recomputed).expect("recomputed fingerprint");
        fs::write(
            &plan_path,
            serde_json::to_vec_pretty(&recomputed).expect("recomputed JSON"),
        )
        .expect("recomputed plan");
        assert!(load_plan(&plan_path).is_err());
        assert!(
            workflow_run(&[
                "--plan".into(),
                plan_path.to_string_lossy().into_owned(),
                "--confirm".into(),
                recomputed.plan_fingerprint,
            ])
            .is_err()
        );
        assert!(!fixture.project.join("recomputed-output").exists());
    }

    #[test]
    fn resealed_plan_cannot_widen_profile_derived_semantics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        plan_fixture(&fixture, &plan_path, &temp.path().join("output")).expect("plan");
        let original = load_plan(&plan_path).expect("load plan");

        let mut cases = Vec::new();
        let mut connector = original.clone();
        connector.replacements[0].expected_connector = "PostgreSQL.Database".into();
        connector.replacements[0].resources.clear();
        cases.push(connector);

        let mut resource = original.clone();
        resource
            .resources
            .get_mut("workbook")
            .expect("resource")
            .output_relative = "resources/workbook/renamed.xlsx".into();
        cases.push(resource);

        let mut template = original.clone();
        template.replacements[0].template = "templates/Other.m".into();
        cases.push(template);

        for mut resealed in cases {
            resealed.plan_fingerprint = plan_fingerprint(&resealed).expect("reseal");
            assert!(
                validate_profile_derived_plan(
                    &resealed,
                    &read_json_bounded(
                        Path::new(&resealed.profile.path),
                        MAX_PROFILE_BYTES,
                        "profile",
                    )
                    .expect("profile"),
                )
                .is_err(),
                "recomputed self-hash widened profile-derived semantics"
            );
        }
    }

    #[test]
    fn connector_identity_ignores_comments_and_strings_and_rejects_other_connectors() {
        assert!(
            validate_expected_connector_call(
                &m_tokens("let Note = \"Excel.Workbook(\", Source = Web.Contents(\"https://invalid\") in Source").expect("tokens"),
                "Excel.Workbook"
            )
            .is_err()
        );
        assert!(
            validate_expected_connector_call(
                &m_tokens(
                    "let /* Excel.Workbook( */ Source = Web.Contents(\"https://invalid\") in Source"
                )
                .expect("tokens"),
                "Excel.Workbook"
            )
            .is_err()
        );
        assert!(
            validate_expected_connector_call(
                &m_tokens("let Good = Excel.Workbook(File.Contents(\"book.xlsx\"), null, true), Bad = Web.Contents(\"https://invalid\") in Good").expect("tokens"),
                "Excel.Workbook"
            )
            .is_err()
        );
        assert!(
            validate_expected_connector_call(
                &m_tokens("let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true) in Source").expect("tokens"),
                "Excel.Workbook"
            )
            .is_ok()
        );

        let replacement = ReplacementSpec {
            operation: "partition.replaceSource".into(),
            table: "Fact".into(),
            partition: "Fact".into(),
            expected_before_sha256:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            template: "templates/Fact.m".into(),
            expected_connector: "Excel.Workbook".into(),
            resources: vec!["workbook".into()],
        };
        let good = "let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), Typed = Table.TransformColumnTypes(Source, {}) in Typed";
        assert!(validate_template(good, &replacement).is_ok());
        assert_eq!(
            m_semantic_sha256(good).expect("semantic M"),
            m_semantic_sha256("let\n Source=Excel.Workbook( File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"),null,true),/* vendor formatting */Typed=Table.TransformColumnTypes(Source,{})\nin Typed").expect("reformatted semantic M")
        );
        for unsafe_m in [
            "let Source = Excel.Workbook(File.Contents(\"C:\\\\private\\\\book.xlsx\"), null, true) in Source",
            "let Source = Excel.Workbook(File.Contents(\"https://invalid/book.xlsx\"), null, true) in Source",
            "let Connector = Excel.Workbook, Source = Connector(\"{{powerbi-cli.resourcePath:workbook}}\") in Source",
            "let Root = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), Source = Root in Source",
            "let Source = Excel.Workbook(File.Contents(\"book-{{powerbi-cli.resourcePath:workbook}}\"), null, true) in Source",
            "let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), Leak = Mystery.Cloud(\"x\") in Source",
            "let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), F = Web.Contents, Leak = Value.Invoke(F, {\"x\"}) in Source",
        ] {
            assert!(
                validate_template(unsafe_m, &replacement).is_err(),
                "unsafe M template was accepted: {unsafe_m}"
            );
        }

        let postgres = ReplacementSpec {
            expected_connector: "PostgreSQL.Database".into(),
            resources: Vec::new(),
            ..replacement.clone()
        };
        assert!(
            validate_template(
                "let Source = PostgreSQL.Database(\"db.internal:5432\", \"analytics\"), Rows = Table.SelectRows(Source, each true) in Rows",
                &postgres,
            )
            .is_ok()
        );
        assert!(
            validate_template(
                "let Source = PostgreSQL.Database(\"db.internal:5432\", \"analytics\"), Extra = ([Run = PostgreSQL.Database][Run])(\"other.internal:5432\", \"other\") in Source",
                &postgres,
            )
            .is_err(),
            "computed connector invocation bypassed the closed M grammar"
        );
        let postgres_with_file = ReplacementSpec {
            resources: vec!["workbook".into()],
            ..postgres
        };
        assert!(
            validate_template(
                "let Source = PostgreSQL.Database(\"db.internal:5432\", \"analytics\") in Source",
                &postgres_with_file,
            )
            .is_err()
        );
    }

    #[test]
    fn complete_transformed_m_is_materialized_without_template_payload_in_plan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        let plan = load_plan(&plan_path).expect("load plan");
        let serialized = fs::read_to_string(&plan_path).expect("plan text");
        assert!(!serialized.contains("Table.TransformColumnTypes"));
        let owned_output = OwnedWorkflowOutput::create(&output).expect("output");
        copy_resources(&plan, &owned_output).expect("copy resources");
        let replacements = materialize_replacements(&plan, &output).expect("materialize M");
        let expression = &replacements[0].complete_m_expression;
        assert!(expression.contains("Excel.Workbook"));
        assert!(expression.contains("Navigation"));
        assert!(expression.contains("Table.TransformColumnTypes"));
        assert!(!expression.contains("{{powerbi-cli."));
        assert!(expression.contains("resources"));
        #[cfg(windows)]
        {
            assert!(expression.contains("File.Contents(\""));
            assert!(!expression.contains(r"\\?\"));
        }
        assert!(
            template_placeholders(
                "let Source = File.Contents({{powerbi-cli.resourcePath:workbook}}) in Source",
                &m_tokens(
                    "let Source = File.Contents({{powerbi-cli.resourcePath:workbook}}) in Source"
                )
                .expect("tokens")
            )
            .is_err()
        );
    }

    #[test]
    fn recomputed_receipt_checksum_cannot_bypass_semantics_and_copy_failure_preserves_source() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        let plan = load_plan(&plan_path).expect("load plan");
        let source_before = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project"),
        )
        .expect("before");
        let owned_output = OwnedWorkflowOutput::create(&output).expect("output");
        owned_output
            .write_new_file(Path::new("occupied"), b"keep", "occupied test file")
            .expect("occupied");
        let claim = claim_for_file(&fixture.resource, MAX_RESOURCE_BYTES).expect("claim");
        assert!(
            copy_new_output_file(
                &fixture.resource,
                &owned_output,
                Path::new("occupied"),
                &claim
            )
            .is_err()
        );
        let source_after = source_manifest(
            &resolve_project(&fixture.pbip).expect("resolved"),
            &fs::canonicalize(&fixture.project).expect("project"),
        )
        .expect("after");
        assert_eq!(source_before.closure_sha256, source_after.closure_sha256);

        let mut tampered = WorkflowReceipt {
            schema: WORKFLOW_RECEIPT_SCHEMA.into(),
            receipt_checksum: String::new(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
            output_tree_sha256:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
            source_closure_sha256: plan.source.closure_sha256.clone(),
            model: valid_model_receipt(&plan),
            validation: ValidationClaim {
                native_version: env!("CARGO_PKG_VERSION").into(),
                native_errors: 0,
                native_warnings: 0,
                official_errors: 0,
                official_warnings: 0,
                official_version: "0.1.4".into(),
            },
        };
        assert!(validate_receipt_semantics(&plan, &tampered).is_ok());
        tampered.model.children_reaped = false;
        tampered.receipt_checksum = receipt_checksum(&tampered).expect("recomputed checksum");
        assert!(validate_receipt_semantics(&plan, &tampered).is_err());
        fs::write(
            output.join(WORKFLOW_RECEIPT_FILE),
            serde_json::to_vec_pretty(&tampered).expect("receipt JSON"),
        )
        .expect("receipt");
        assert!(
            workflow_verify(&["--plan".into(), plan_path.to_string_lossy().into_owned()]).is_err()
        );
    }

    #[test]
    fn workflow_output_identity_swap_keeps_copy_bound_to_opened_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.bin");
        fs::write(&source, "planned bytes").expect("source");
        let claim = claim_for_file(&source, MAX_RESOURCE_BYTES).expect("source claim");
        let output_path = temp.path().join("output");
        let output = OwnedWorkflowOutput::create(&output_path).expect("owned output");
        let displaced = temp.path().join("displaced-output");
        if let Err(error) = fs::rename(&output_path, &displaced) {
            #[cfg(windows)]
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(32)
            {
                assert!(output_path.is_dir(), "opened root remained at its path");
                return;
            }
            panic!("displace owned output: {error}");
        }
        fs::create_dir(&output_path).expect("replacement output");

        copy_new_output_file(&source, &output, Path::new("copied/source.bin"), &claim)
            .expect("copy remains bound to opened output root");
        assert!(!output_path.join("copied/source.bin").exists());
        assert_eq!(
            fs::read(displaced.join("copied/source.bin")).expect("capability destination"),
            b"planned bytes"
        );
        assert!(
            output.verify_root().is_err(),
            "publication identity changed"
        );
    }

    #[test]
    fn workflow_output_capability_cannot_be_redirected_by_root_alias_swap() {
        use std::cell::Cell;

        let temp = tempfile::tempdir().expect("tempdir");
        let output_path = temp.path().join("output");
        let displaced = temp.path().join("opened-output");
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).expect("outside");
        let output = OwnedWorkflowOutput::create(&output_path).expect("owned output");
        let renamed = Cell::new(false);
        let aliased = Cell::new(false);

        let mut file = output
            .create_new_file_after(Path::new("proof.bin"), "capability race proof", || {
                match fs::rename(&output_path, &displaced) {
                    Ok(()) => renamed.set(true),
                    Err(error) => {
                        #[cfg(windows)]
                        if error.kind() == std::io::ErrorKind::PermissionDenied
                            || error.raw_os_error() == Some(32)
                        {
                            return;
                        }
                        panic!("rename opened output root at capability boundary: {error}");
                    }
                }

                #[cfg(unix)]
                let alias_result = std::os::unix::fs::symlink(&outside, &output_path);
                #[cfg(windows)]
                let alias_result = std::os::windows::fs::symlink_dir(&outside, &output_path);
                match alias_result {
                    Ok(()) => aliased.set(true),
                    Err(error) => {
                        #[cfg(windows)]
                        if error.kind() == std::io::ErrorKind::PermissionDenied
                            || error.raw_os_error() == Some(1314)
                        {
                            return;
                        }
                        panic!("install outside directory alias: {error}");
                    }
                }
            })
            .expect("capability-relative create");
        file.write_all(b"capability bytes")
            .expect("capability write");
        file.sync_all().expect("capability sync");
        drop(file);

        #[cfg(unix)]
        {
            assert!(
                renamed.get(),
                "opened root was renamed at the write boundary"
            );
            assert!(aliased.get(), "outside symlink replaced the ambient path");
        }
        let landed = if renamed.get() {
            displaced.join("proof.bin")
        } else {
            output_path.join("proof.bin")
        };
        assert_eq!(
            fs::read(&landed).expect("capability-owned file"),
            b"capability bytes"
        );
        assert!(
            !outside.join("proof.bin").exists(),
            "outside alias received a workflow write"
        );
        if aliased.get() {
            assert!(
                !output_path.join("proof.bin").exists(),
                "ambient replacement path received a workflow write"
            );
        }
        if renamed.get() {
            assert!(
                output.verify_root().is_err(),
                "publication identity changed"
            );
        }
    }

    #[test]
    fn reconstructed_stage_and_copy_evidence_reject_resealed_artifact_swaps() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = workflow_fixture(temp.path());
        let plan_path = temp.path().join("workflow.plan.json");
        let output = temp.path().join("output");
        plan_fixture(&fixture, &plan_path, &output).expect("plan");
        let plan = load_plan(&plan_path).expect("load plan");
        let owned_output = OwnedWorkflowOutput::create(&output).expect("output");
        copy_claimed_files(
            Path::new(&plan.source.project_root),
            &owned_output,
            &plan.source.files,
        )
        .expect("copy closure");
        copy_resources(&plan, &owned_output).expect("copy resources");
        let staged = resolve_project(&output.join(&plan.source.pbip_relative)).expect("stage");
        let materialized = materialize_replacements(&plan, &output).expect("materialized");
        let docs = load_table_documents_from_semantic_model(&staged.semantic_model_dir)
            .expect("stage docs");
        let replacement = &materialized[0];
        let mutation = replace_partition_source_plan(
            &docs,
            &PartitionSelector {
                table: Some(replacement.table.clone()),
                name: Some(replacement.partition.clone()),
                ..PartitionSelector::default()
            },
            &replacement.complete_m_expression,
        )
        .expect("stage mutation");
        fs::write(&mutation.path, &mutation.new_text).expect("materialize stage");

        let expected = expected_stage(&plan, &output).expect("expected stage");
        let actual = validate_tmdl_definition(&staged.semantic_model_dir.join("definition"))
            .expect("actual stage");
        assert_eq!(actual.sha256, expected.after_sha256);
        verify_staged_copies(&plan, &output, &expected.modified_source_files)
            .expect("exact staged copies");

        let report_file = output.join("Synthetic.Report/definition/report.json");
        let report_before = fs::read(&report_file).expect("report before");
        fs::write(&report_file, "{\"swapped\":true}").expect("swap report");
        assert!(verify_staged_copies(&plan, &output, &expected.modified_source_files).is_err());
        fs::write(&report_file, report_before).expect("restore report");

        let model_file = staged.semantic_model_dir.join("definition/model.tmdl");
        let model_before = fs::read(&model_file).expect("model before");
        fs::write(&model_file, "model Swapped\n").expect("swap unrelated TMDL");
        assert_ne!(
            validate_tmdl_definition(&staged.semantic_model_dir.join("definition"))
                .expect("tampered definition")
                .sha256,
            expected.after_sha256
        );
        fs::write(&model_file, model_before).expect("restore model");

        let evidence_root = output.join(WORKFLOW_EVIDENCE_DIR);
        copy_definition(
            &staged.semantic_model_dir.join("definition"),
            &evidence_root.join("definition"),
        );
        let requested = expected
            .requested_sha256
            .get(&(replacement.table.clone(), replacement.partition.clone()))
            .expect("request hash");
        assert_eq!(
            &staged_partition_source_fingerprint(
                &evidence_root,
                &replacement.table,
                &replacement.partition,
            )
            .expect("evidence fingerprint"),
            requested
        );
        let canonical_evidence = validate_evidence_claim(
            &output,
            &EvidenceClaim {
                path: WORKFLOW_EVIDENCE_DIR.into(),
                definition_sha256: String::new(),
                file_count: 0,
                total_bytes: 0,
            },
        )
        .expect("canonical evidence");
        let injected = evidence_root.join("definition/tables/Injected.tmdl");
        fs::write(
            &injected,
            "table Injected\n\n\tpartition Injected = m\n\t\tmode: import\n\t\tsource =\n\t\t\tlet Source = 1 in Source\n",
        )
        .expect("inject unrelated evidence table");
        let tampered_evidence = validate_evidence_claim(
            &output,
            &EvidenceClaim {
                path: WORKFLOW_EVIDENCE_DIR.into(),
                definition_sha256: String::new(),
                file_count: 0,
                total_bytes: 0,
            },
        )
        .expect("shape-valid injected evidence");
        let mut resealed = WorkflowReceipt {
            schema: WORKFLOW_RECEIPT_SCHEMA.into(),
            receipt_checksum: String::new(),
            plan_fingerprint: plan.plan_fingerprint.clone(),
            output_tree_sha256: hash_workflow_output(&output)
                .expect("tampered output hash")
                .sha256,
            source_closure_sha256: plan.source.closure_sha256.clone(),
            model: valid_model_receipt(&plan),
            validation: ValidationClaim {
                native_version: env!("CARGO_PKG_VERSION").into(),
                native_errors: 0,
                native_warnings: 0,
                official_errors: 0,
                official_warnings: 0,
                official_version: "0.1.4".into(),
            },
        };
        resealed.model.evidence = EvidenceClaim {
            path: WORKFLOW_EVIDENCE_DIR.into(),
            definition_sha256: tampered_evidence.definition_sha256.clone(),
            file_count: tampered_evidence.file_count,
            total_bytes: tampered_evidence.total_bytes,
        };
        resealed.receipt_checksum = receipt_checksum(&resealed).expect("resealed receipt");
        assert_eq!(
            resealed.receipt_checksum,
            receipt_checksum(&resealed).unwrap()
        );
        assert!(
            validate_canonical_export_binding(&tampered_evidence, &canonical_evidence).is_err(),
            "recomputed evidence and receipt claims bypassed canonical stage binding"
        );
        fs::remove_file(&injected).expect("remove injected table");

        let evidence_model = evidence_root.join("definition/model.tmdl");
        let evidence_model_before = fs::read(&evidence_model).expect("evidence model before");
        fs::OpenOptions::new()
            .append(true)
            .open(&evidence_model)
            .and_then(|mut file| file.write_all(b"\n// password=secret\n"))
            .expect("credential comment");
        assert!(
            validate_evidence_claim(
                &output,
                &EvidenceClaim {
                    path: WORKFLOW_EVIDENCE_DIR.into(),
                    definition_sha256: String::new(),
                    file_count: 0,
                    total_bytes: 0,
                },
            )
            .is_err(),
            "credential-bearing evidence comment was accepted"
        );
        fs::write(&evidence_model, evidence_model_before).expect("restore evidence model");

        let evidence_docs =
            load_table_documents_from_semantic_model(&evidence_root).expect("evidence docs");
        let swapped = replace_partition_source_plan(
            &evidence_docs,
            &PartitionSelector {
                table: Some(replacement.table.clone()),
                name: Some(replacement.partition.clone()),
                ..PartitionSelector::default()
            },
            "let Source = 1 in Source",
        )
        .expect("swap evidence");
        fs::write(swapped.path, swapped.new_text).expect("write swapped evidence");
        assert_ne!(
            &staged_partition_source_fingerprint(
                &evidence_root,
                &replacement.table,
                &replacement.partition,
            )
            .expect("swapped evidence fingerprint"),
            requested
        );
    }

    #[test]
    fn tree_hash_is_bounded_and_rejects_links() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("one"), "1").expect("one");
        fs::write(temp.path().join("two"), "22").expect("two");
        assert!(hash_tree_inner_bounded(temp.path(), &BTreeMap::new(), &[], 1, 100).is_err());
        assert!(hash_tree_inner_bounded(temp.path(), &BTreeMap::new(), &[], 10, 1).is_err());
        assert!(hash_tree_inner_bounded(temp.path(), &BTreeMap::new(), &[], 10, 100).is_ok());

        let oversized = tempfile::tempdir().expect("oversized tempdir");
        File::create(oversized.path().join("large"))
            .and_then(|file| file.set_len(1024 * 1024))
            .expect("sparse oversized file");
        let mut opens = 0_usize;
        let result = hash_tree_inner_bounded_with_opener(
            oversized.path(),
            &BTreeMap::new(),
            &[],
            10,
            1,
            |path| {
                opens += 1;
                File::open(path).map_err(|error| error.to_string())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            opens, 0,
            "oversized file was opened before byte-cap rejection"
        );

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(temp.path().join("one"), temp.path().join("link"))
                .expect("symlink");
            assert!(hash_tree(temp.path()).is_err());
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(temp.path().join("one"), temp.path().join("link"))
                .is_ok()
            {
                assert!(hash_tree(temp.path()).is_err());
            }
        }

        let dangling_root = tempfile::tempdir().expect("dangling tempdir");
        let marker = dangling_root.path().join(WORKFLOW_INCOMPLETE_FILE);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dangling_root.path().join("missing"), &marker)
                .expect("dangling marker");
            assert!(
                hash_tree_with_exclusions(
                    dangling_root.path(),
                    &[Path::new(WORKFLOW_INCOMPLETE_FILE)]
                )
                .is_err(),
                "excluded dangling marker bypassed link inspection"
            );
        }
        #[cfg(windows)]
        {
            if std::os::windows::fs::symlink_file(dangling_root.path().join("missing"), &marker)
                .is_ok()
            {
                assert!(
                    hash_tree_with_exclusions(
                        dangling_root.path(),
                        &[Path::new(WORKFLOW_INCOMPLETE_FILE)]
                    )
                    .is_err(),
                    "excluded dangling marker bypassed reparse inspection"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires exact installed Microsoft modeling MCP and report validator sidecars"]
    fn workflow_plan_run_verify_with_exact_installed_sidecars() {
        let temp = tempfile::tempdir().expect("tempdir");
        let schema_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/sales.schema.json");
        let schema: Value =
            serde_json::from_slice(&fs::read(&schema_path).expect("schema")).expect("schema JSON");
        let project = temp.path().join("exact-source");
        crate::scaffold_schema_value(schema, &schema_path, &project, false)
            .expect("scaffold exact source fixture");
        let resolved = resolve_project(&project).expect("resolve scaffold");
        let expected =
            staged_partition_source_fingerprint(&resolved.semantic_model_dir, "DimDate", "DimDate")
                .expect("source fingerprint");
        let profile_dir = temp.path().join("exact-profile");
        fs::create_dir_all(profile_dir.join("templates")).expect("templates");
        fs::create_dir_all(profile_dir.join("data")).expect("data");
        let resource = profile_dir.join("data/synthetic.xlsx");
        fs::write(&resource, "neutral bytes").expect("resource");
        let resource_sha256 = sha256_file(&resource).expect("resource hash");
        fs::write(
            profile_dir.join("templates/DimDate.m"),
            "let\n    Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true),\n    Navigation = Source{[Item=\"DimDate\",Kind=\"Table\"]}[Data],\n    Typed = Table.TransformColumnTypes(Navigation, {{\"DateKey\", Int64.Type}})\nin\n    Typed\n",
        )
        .expect("template");
        let profile = profile_dir.join("source-profile.json");
        fs::write(
            &profile,
            serde_json::to_vec_pretty(&json!({
                "schema": SOURCE_PROFILE_SCHEMA,
                "profileId": "exact-neutral",
                "resources": {"workbook": {
                    "path": "data/synthetic.xlsx",
                    "expectedSha256": resource_sha256
                }},
                "replacements": [{
                    "operation": "partition.replaceSource",
                    "table": "DimDate",
                    "partition": "DimDate",
                    "expectedBeforeSha256": expected,
                    "template": "templates/DimDate.m",
                    "expectedConnector": "Excel.Workbook",
                    "resources": ["workbook"]
                }]
            }))
            .expect("profile JSON"),
        )
        .expect("profile");
        let plan_path = temp.path().join("exact.plan.json");
        let output = temp.path().join("exact-output");
        let planned = workflow_plan(&plan_args(&project, &profile, &plan_path, &output))
            .expect("exact workflow plan");
        workflow_run(&[
            "--plan".into(),
            plan_path.to_string_lossy().into_owned(),
            "--confirm".into(),
            planned["planFingerprint"]
                .as_str()
                .expect("fingerprint")
                .into(),
        ])
        .expect("exact workflow run");
        let receipt_text =
            fs::read_to_string(output.join(WORKFLOW_RECEIPT_FILE)).expect("exact workflow receipt");
        assert!(!receipt_text.contains("Excel.Workbook"));
        assert!(!receipt_text.contains("Table.TransformColumnTypes"));
        assert!(!receipt_text.contains("synthetic.xlsx"));
        let receipt: WorkflowReceipt =
            serde_json::from_str(&receipt_text).expect("exact receipt JSON");
        assert!(receipt.model.children_reaped && receipt.model.pumps_joined);
        assert_eq!(
            receipt.model.source_before_sha256,
            receipt.model.source_after_sha256
        );
        assert_eq!(
            receipt.model.stage_after_sha256,
            receipt.model.expected_stage_sha256
        );
        let output_before_verify = hash_tree(&output).expect("output before verify").sha256;
        workflow_verify(&["--plan".into(), plan_path.to_string_lossy().into_owned()])
            .expect("exact workflow verify");
        let output_after_verify = hash_tree(&output).expect("output after verify").sha256;
        assert_eq!(
            output_before_verify, output_after_verify,
            "verify mutated output"
        );
    }

    struct WorkflowFixture {
        project: PathBuf,
        pbip: PathBuf,
        profile: PathBuf,
        template: PathBuf,
        resource: PathBuf,
    }

    fn workflow_fixture(root: &Path) -> WorkflowFixture {
        let project = root.join("Project");
        let report = project.join("Synthetic.Report");
        let model = project.join("Synthetic.SemanticModel");
        fs::create_dir_all(&report).expect("report");
        let fixture_model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/conformance/microsoft/modeling-mcp/Synthetic.SemanticModel");
        copy_tree_test(&fixture_model, &model);
        let pbip = project.join("Synthetic.pbip");
        fs::write(
            &pbip,
            r#"{"version":"1.0","artifacts":[{"report":{"path":"Synthetic.Report"}}]}"#,
        )
        .expect("pbip");
        fs::write(
            report.join("definition.pbir"),
            r#"{"version":"4.0","datasetReference":{"byPath":{"path":"../Synthetic.SemanticModel"}}}"#,
        )
        .expect("pbir");
        fs::create_dir_all(report.join("definition")).expect("report definition");
        fs::write(report.join("definition/report.json"), "{}\n").expect("report file");
        let profile_dir = root.join("profile");
        fs::create_dir_all(profile_dir.join("templates")).expect("templates");
        fs::create_dir_all(profile_dir.join("data")).expect("data");
        let template = profile_dir.join("templates/Synthetic.m");
        fs::write(
            &template,
            "let\n    Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true),\n    Navigation = Source{[Item=\"Sheet1\",Kind=\"Sheet\"]}[Data],\n    Typed = Table.TransformColumnTypes(Navigation, {{\"Value\", Int64.Type}})\nin\n    Typed\n",
        )
        .expect("template");
        let resource = profile_dir.join("data/synthetic.xlsx");
        fs::write(&resource, "neutral synthetic workbook bytes").expect("resource");
        let resource_sha256 = sha256_file(&resource).expect("resource hash");
        let expected = staged_partition_source_fingerprint(&model, "Synthetic", "Synthetic")
            .expect("source fingerprint");
        let profile = profile_dir.join("source-profile.json");
        let value = json!({
            "schema": SOURCE_PROFILE_SCHEMA,
            "profileId": "neutral-synthetic",
            "resources": {"workbook": {
                "path": "data/synthetic.xlsx",
                "expectedSha256": resource_sha256
            }},
            "replacements": [{
                "operation": "partition.replaceSource",
                "table": "Synthetic",
                "partition": "Synthetic",
                "expectedBeforeSha256": expected,
                "template": "templates/Synthetic.m",
                "expectedConnector": "Excel.Workbook",
                "resources": ["workbook"]
            }]
        });
        fs::write(
            &profile,
            serde_json::to_vec_pretty(&value).expect("profile JSON"),
        )
        .expect("profile");
        WorkflowFixture {
            project,
            pbip,
            profile,
            template,
            resource,
        }
    }

    fn plan_args(project: &Path, profile: &Path, plan: &Path, output: &Path) -> Vec<String> {
        vec![
            "--project".into(),
            project.to_string_lossy().into_owned(),
            "--profile".into(),
            profile.to_string_lossy().into_owned(),
            "--out".into(),
            plan.to_string_lossy().into_owned(),
            "--out-dir".into(),
            output.to_string_lossy().into_owned(),
        ]
    }

    fn plan_fixture(fixture: &WorkflowFixture, plan: &Path, output: &Path) -> CliResult<Value> {
        workflow_plan(&plan_args(&fixture.pbip, &fixture.profile, plan, output))
    }

    fn copy_tree_test(source: &Path, target: &Path) {
        for entry in WalkDir::new(source) {
            let entry = entry.expect("fixture entry");
            let relative = entry.path().strip_prefix(source).expect("relative");
            let output = target.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir_all(output).expect("fixture directory");
            } else {
                fs::copy(entry.path(), output).expect("fixture file");
            }
        }
    }

    fn valid_model_receipt(plan: &WorkflowPlan) -> ModelReceipt {
        let hash = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        ModelReceipt {
            component: "modeling-mcp".into(),
            package_version: "0.5.0-beta.11".into(),
            server_version: "0.5.0.0".into(),
            local_process: true,
            transport: "stdio".into(),
            children_reaped: true,
            pumps_joined: true,
            forced_cleanup: false,
            source_before_sha256: hash.into(),
            source_after_sha256: hash.into(),
            stage_before_sha256: hash.into(),
            stage_after_sha256: hash.into(),
            expected_stage_sha256: hash.into(),
            evidence: EvidenceClaim {
                path: WORKFLOW_EVIDENCE_DIR.into(),
                definition_sha256: hash.into(),
                file_count: 0,
                total_bytes: 0,
            },
            replacements: plan
                .replacements
                .iter()
                .map(|replacement| ReplacementReceipt {
                    table: replacement.table.clone(),
                    partition: replacement.partition.clone(),
                    before_sha256: replacement.expected_before_sha256.clone(),
                    requested_sha256: hash.into(),
                    readback_sha256: hash.into(),
                    materialized_sha256: hash.into(),
                })
                .collect(),
        }
    }

    fn copy_fixture(target: &Path) {
        fs::create_dir_all(target.join("definition").join("tables")).expect("fixture dirs");
        fs::write(target.join("definition.pbism"), "{\"version\":\"4.0\"}").expect("pbism");
        fs::write(
            target.join("definition").join("database.tmdl"),
            "database Synthetic\n\tcompatibilityLevel: 1600\n",
        )
        .expect("database");
        fs::write(
            target.join("definition").join("model.tmdl"),
            "model Model\n\tculture: en-US\n",
        )
        .expect("model");
        fs::write(
            target
                .join("definition")
                .join("tables")
                .join("Synthetic.tmdl"),
            "table Synthetic\n",
        )
        .expect("table");
    }

    fn copy_definition(source: &Path, target: &Path) {
        fs::create_dir_all(target.join("tables")).expect("tables");
        for name in ["database.tmdl", "model.tmdl"] {
            fs::copy(source.join(name), target.join(name)).expect("copy root TMDL");
        }
        fs::copy(
            source.join("tables").join("Synthetic.tmdl"),
            target.join("tables").join("Synthetic.tmdl"),
        )
        .expect("copy table");
    }
}
