//! Shared workflow integrity types, staged-model guards, hashing, bounded I/O, and path helpers.

use super::*;

pub(super) const MAX_DEFINITION_FILES: usize = 10_000;
pub(super) const MAX_DEFINITION_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const MAX_HASHED_TREE_FILES: usize = 20_000;
pub(super) const MAX_HASHED_TREE_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_TEMPLATE_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_RESOURCE_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const TMDL_SUBDIRECTORIES: [&str; 4] = ["cultures", "perspectives", "roles", "tables"];
pub(super) const SOURCE_PROFILE_SCHEMA: &str = "powerbi-cli.source-profile.v1";
pub(super) const WORKFLOW_PLAN_SCHEMA: &str = "powerbi-cli.workflow-plan.v1";
pub(super) const WORKFLOW_RECEIPT_SCHEMA: &str = "powerbi-cli.workflow-receipt.v1";
pub(super) const WORKFLOW_POLICY: &str = "powerbi-cli.workflow-policy.v1";
pub(super) const WORKFLOW_RECEIPT_FILE: &str = "powerbi-cli-workflow-receipt.json";
pub(super) const WORKFLOW_INCOMPLETE_FILE: &str = ".powerbi-cli-workflow-incomplete";
pub(super) const WORKFLOW_EVIDENCE_DIR: &str = ".powerbi-cli-model-evidence";
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

pub(super) struct PreparationGuard {
    quarantine_marker: PathBuf,
    export_root: PathBuf,
    cleanup_tombstone: PathBuf,
    marker_created: bool,
    export_identity: Option<FileId>,
    definition_identity: Option<FileId>,
}

pub(super) struct OwnedWorkflowOutput {
    root: PathBuf,
    capability: CapabilityDir,
    identity: FileId,
}

impl OwnedWorkflowOutput {
    pub(super) fn create(path: &Path) -> CliResult<Self> {
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

    pub(super) fn verify_root(&self) -> CliResult<()> {
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

    pub(super) fn ensure_relative_directory(&self, relative: &Path, label: &str) -> CliResult<()> {
        if relative.as_os_str().is_empty() {
            return Ok(());
        }
        let relative_text = unicode_path(relative, label)?;
        let relative = validate_relative_path(&relative_text, label)?;
        let mut directory = self.capability.try_clone().map_err(|error| {
            CliError::unexpected(format!(
                "clone workflow output directory capability: {error}"
            ))
        })?;
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(CliError::validation_failed(format!(
                    "{label} escaped the workflow output"
                )));
            };
            match directory.symlink_metadata(name) {
                Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {}
                Ok(_) => {
                    return Err(CliError::validation_failed(format!(
                        "{label} is not an ordinary directory: {}",
                        self.root.join(&relative).display()
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    directory.create_dir(name).map_err(|error| {
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
            directory = directory.open_dir_nofollow(name).map_err(|error| {
                CliError::validation_failed(format!(
                    "open ordinary workflow-owned directory component {}: {error}",
                    name.to_string_lossy()
                ))
            })?;
        }
        Ok(())
    }

    pub(super) fn prepare_new_relative(&self, relative: &Path, label: &str) -> CliResult<PathBuf> {
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

    pub(super) fn open_parent_nofollow(
        &self,
        relative: &Path,
        label: &str,
    ) -> CliResult<CapabilityDir> {
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

    pub(super) fn create_new_file_after(
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

    pub(super) fn verify_file(
        &self,
        relative: &Path,
        label: &str,
        max_bytes: u64,
    ) -> CliResult<FileClaim> {
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

    pub(super) fn write_new_file(
        &self,
        relative: &Path,
        bytes: &[u8],
        label: &str,
    ) -> CliResult<FileId> {
        self.write_new_file_after(relative, bytes, label, || {})
    }

    pub(super) fn write_new_file_after(
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

    pub(super) fn remove_owned_file(
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

    pub(super) fn cleanup_if_empty(self) -> Option<String> {
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

pub(super) fn capability_file_id(file: &cap_std::fs::File, label: &str) -> CliResult<FileId> {
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
pub(crate) struct TreeSummary {
    pub(crate) sha256: String,
    pub(crate) file_count: usize,
    pub(crate) total_bytes: u64,
}

pub(crate) fn validate_tmdl_definition(definition: &Path) -> Result<TreeSummary, String> {
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

pub(super) fn hash_tree(root: &Path) -> Result<TreeSummary, String> {
    hash_tree_with_overrides(root, &BTreeMap::new())
}

pub(super) fn hash_tree_with_overrides(
    root: &Path,
    overrides: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<TreeSummary, String> {
    hash_tree_inner(root, overrides, &[])
}

pub(super) fn hash_tree_with_exclusions(
    root: &Path,
    exclusions: &[&Path],
) -> Result<TreeSummary, String> {
    hash_tree_inner(root, &BTreeMap::new(), exclusions)
}

pub(super) fn hash_tree_inner(
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

pub(super) fn hash_tree_inner_bounded(
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

pub(super) fn hash_tree_inner_bounded_with_opener(
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

pub(super) fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
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

pub(super) fn directory_has_entries(path: &Path) -> Result<bool, String> {
    fs::read_dir(path)
        .map_err(|error| format!("read directory {}: {error}", path.display()))
        .map(|mut entries| entries.next().is_some())
}

pub(super) fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

#[cfg(windows)]
pub(super) fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(super) fn metadata_is_link_or_reparse(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub(super) fn hex_digest(bytes: &[u8]) -> String {
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
pub(super) struct SourceProfile {
    pub(super) schema: String,
    pub(super) profile_id: String,
    pub(super) resources: BTreeMap<String, ResourceSpec>,
    pub(super) replacements: Vec<ReplacementSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ResourceSpec {
    #[serde(default)]
    pub(super) path: Option<String>,
    pub(super) expected_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReplacementSpec {
    pub(super) operation: String,
    pub(super) table: String,
    pub(super) partition: String,
    pub(super) expected_before_sha256: String,
    pub(super) template: String,
    pub(super) expected_connector: String,
    pub(super) resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FileClaim {
    pub(super) path: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlannedSource {
    pub(super) project_root: String,
    pub(super) pbip_relative: String,
    pub(super) closure_sha256: String,
    pub(super) files: Vec<FileClaim>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum SelectedArtifactKind {
    Report,
    SemanticModel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlannedResource {
    pub(super) source: FileClaim,
    pub(super) output_relative: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlannedReplacement {
    pub(super) table: String,
    pub(super) partition: String,
    pub(super) expected_before_sha256: String,
    pub(super) template: String,
    pub(super) expected_connector: String,
    pub(super) resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WorkflowPlan {
    pub(super) schema: String,
    pub(super) plan_fingerprint: String,
    pub(super) policy: String,
    pub(super) profile_id: String,
    pub(super) profile: FileClaim,
    pub(super) source: PlannedSource,
    pub(super) templates: BTreeMap<String, FileClaim>,
    pub(super) resources: BTreeMap<String, PlannedResource>,
    pub(super) replacements: Vec<PlannedReplacement>,
    pub(super) integration_lock_sha256: String,
    pub(super) output_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ValidationClaim {
    pub(super) native_version: String,
    pub(super) native_errors: u64,
    pub(super) native_warnings: u64,
    pub(super) official_errors: u64,
    pub(super) official_warnings: u64,
    pub(super) official_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WorkflowReceipt {
    pub(super) schema: String,
    pub(super) receipt_checksum: String,
    pub(super) plan_fingerprint: String,
    pub(super) output_tree_sha256: String,
    pub(super) source_closure_sha256: String,
    pub(super) model: ModelReceipt,
    pub(super) validation: ValidationClaim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ModelReceipt {
    pub(super) component: String,
    pub(super) package_version: String,
    pub(super) server_version: String,
    pub(super) local_process: bool,
    pub(super) transport: String,
    pub(super) children_reaped: bool,
    pub(super) pumps_joined: bool,
    pub(super) forced_cleanup: bool,
    pub(super) source_before_sha256: String,
    pub(super) source_after_sha256: String,
    pub(super) stage_before_sha256: String,
    pub(super) stage_after_sha256: String,
    pub(super) expected_stage_sha256: String,
    pub(super) evidence: EvidenceClaim,
    pub(super) replacements: Vec<ReplacementReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EvidenceClaim {
    pub(super) path: String,
    pub(super) definition_sha256: String,
    pub(super) file_count: usize,
    pub(super) total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReplacementReceipt {
    pub(super) table: String,
    pub(super) partition: String,
    pub(super) before_sha256: String,
    pub(super) requested_sha256: String,
    pub(super) readback_sha256: String,
    pub(super) materialized_sha256: String,
}

pub(super) struct ExpectedStage {
    pub(super) before_sha256: String,
    pub(super) after_sha256: String,
    pub(super) modified_source_files: BTreeSet<String>,
    pub(super) requested_sha256: BTreeMap<(String, String), String>,
    pub(super) requested_semantic_sha256: BTreeMap<(String, String), String>,
}

pub(super) fn parse_pairs(
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

pub(super) fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> CliResult<()> {
    if slot.replace(value).is_some() {
        Err(CliError::invalid_args(format!(
            "{flag} may be specified only once"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn validate_profile_shape(profile: &SourceProfile) -> CliResult<()> {
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

pub(super) fn validate_name(value: &str, label: &str) -> CliResult<()> {
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

pub(super) fn validate_identifier(value: &str, label: &str) -> CliResult<()> {
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

pub(super) fn validate_connector(value: &str) -> CliResult<()> {
    const CONNECTORS: &[&str] = &["Excel.Workbook", "PostgreSQL.Database"];
    if !CONNECTORS.contains(&value) {
        return Err(CliError::validation_failed(
            "expectedConnector must name one supported closed connector function",
        ));
    }
    Ok(())
}

pub(super) fn validate_relative_path(value: &str, label: &str) -> CliResult<PathBuf> {
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

pub(super) fn resolve_profile_resources(
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

pub(super) fn resolve_profile_templates(
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

pub(super) fn validate_template(text: &str, replacement: &ReplacementSpec) -> CliResult<()> {
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
pub(super) enum MToken {
    Ident(String),
    String(String),
    LParen,
    RParen,
    Comma,
    Equals,
    Other(char),
}

pub(super) fn m_tokens(text: &str) -> CliResult<Vec<MToken>> {
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

pub(super) fn validate_expected_connector_call(tokens: &[MToken], expected: &str) -> CliResult<()> {
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

pub(super) fn resource_placeholder_name(value: &str) -> Option<&str> {
    value
        .strip_prefix("{{powerbi-cli.resourcePath:")
        .and_then(|value| value.strip_suffix("}}"))
        .filter(|name| validate_name(name, "resource placeholder").is_ok())
}

pub(super) fn template_placeholders(
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

pub(super) fn source_manifest(
    resolved: &crate::ResolvedProject,
    root: &Path,
) -> CliResult<PlannedSource> {
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

pub(super) fn add_selected_tree(
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

pub(super) fn selected_artifact_file_allowed(kind: SelectedArtifactKind, relative: &Path) -> bool {
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

pub(super) fn report_definition_json_allowed(parts: &[&str]) -> bool {
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

pub(super) fn selected_artifact_file_is_text(kind: SelectedArtifactKind, relative: &Path) -> bool {
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

pub(super) fn validate_selected_text_file(path: &Path, label: &str) -> CliResult<()> {
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

pub(super) fn add_selected_file(
    selected: &mut BTreeMap<String, PathBuf>,
    root: &Path,
    path: &Path,
) -> CliResult<()> {
    let relative = normalized_relative(root, path)?;
    selected.insert(relative, path.to_path_buf());
    Ok(())
}

pub(super) fn normalized_relative(root: &Path, path: &Path) -> CliResult<String> {
    path.strip_prefix(root)
        .map_err(|_| CliError::validation_failed("selected path escaped project root"))?
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::validation_failed("selected path is empty or not Unicode"))
}

pub(super) fn load_plan(path: &Path) -> CliResult<WorkflowPlan> {
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

pub(super) fn validate_planned_output_location(
    source_root: &Path,
    output: &str,
) -> CliResult<PathBuf> {
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

pub(super) fn verify_plan_inputs(plan: &WorkflowPlan) -> CliResult<()> {
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

pub(super) fn validate_profile_derived_plan(
    plan: &WorkflowPlan,
    profile: &SourceProfile,
) -> CliResult<()> {
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

pub(super) fn validate_planned_template(
    text: &str,
    replacement: &PlannedReplacement,
) -> CliResult<()> {
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

pub(super) fn source_semantic_root(plan: &WorkflowPlan) -> CliResult<PathBuf> {
    let root = PathBuf::from(&plan.source.project_root);
    let pbip = root.join(&plan.source.pbip_relative);
    resolve_project(&pbip).map(|resolved| resolved.semantic_model_dir)
}

pub(super) fn copy_new_output_file(
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

pub(super) fn materialize_replacements(
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

pub(super) fn expected_stage(plan: &WorkflowPlan, output: &Path) -> CliResult<ExpectedStage> {
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

pub(super) fn source_expression_sha256(value: &str) -> String {
    let normalized = value
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    sha256_bytes(normalized.trim_matches('\n').as_bytes())
}

pub(super) fn m_semantic_sha256(value: &str) -> CliResult<String> {
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

pub(super) fn partition_source_semantic_sha256(
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

pub(super) fn validation_claim(validation: &Value) -> CliResult<ValidationClaim> {
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

pub(super) fn validate_receipt_claims(
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

pub(super) fn validate_canonical_export_binding(
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

pub(super) fn verify_staged_copies(
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

pub(super) fn validate_receipt_semantics(
    plan: &WorkflowPlan,
    receipt: &WorkflowReceipt,
) -> CliResult<()> {
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

pub(super) fn validate_evidence_claim(
    output: &Path,
    claim: &EvidenceClaim,
) -> CliResult<ExportShapeProof> {
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

pub(super) fn plan_fingerprint(plan: &WorkflowPlan) -> CliResult<String> {
    let mut payload = plan.clone();
    payload.plan_fingerprint.clear();
    serde_json::to_vec(&payload)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(json_serialize_error)
}

pub(super) fn receipt_checksum(receipt: &WorkflowReceipt) -> CliResult<String> {
    let mut payload = receipt.clone();
    payload.receipt_checksum.clear();
    serde_json::to_vec(&payload)
        .map(|bytes| sha256_bytes(&bytes))
        .map_err(json_serialize_error)
}

pub(super) fn json_serialize_error(error: serde_json::Error) -> CliError {
    CliError::unexpected(format!("serialize workflow JSON: {error}"))
}

pub(super) fn read_json_bounded<T: serde::de::DeserializeOwned>(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> CliResult<T> {
    let bytes = read_bounded(path, max_bytes, label)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::validation_failed(format!("parse {label} {}: {error}", path.display()))
    })
}

pub(super) fn read_bounded(path: &Path, max_bytes: u64, label: &str) -> CliResult<Vec<u8>> {
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

pub(super) fn read_utf8_claim(claim: &FileClaim, max_bytes: u64, label: &str) -> CliResult<String> {
    verify_file_claim(claim, max_bytes, label)?;
    let bytes = read_bounded(Path::new(&claim.path), max_bytes, label)?;
    String::from_utf8(bytes)
        .map_err(|_| CliError::validation_failed(format!("{label} must be UTF-8")))
}

pub(super) fn claim_for_file(path: &Path, max_bytes: u64) -> CliResult<FileClaim> {
    let path = canonical_plain_file(path, "input file", max_bytes)?;
    let metadata = fs::metadata(&path)
        .map_err(|error| CliError::unexpected(format!("inspect {}: {error}", path.display())))?;
    Ok(FileClaim {
        path: unicode_path(&path, "input file")?,
        sha256: sha256_file_bounded(&path, max_bytes, metadata.len())?,
        bytes: metadata.len(),
    })
}

pub(super) fn unicode_path(path: &Path, label: &str) -> CliResult<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        CliError::validation_failed(format!(
            "{label} path must be Unicode; lossy paths are never persisted or sent across a mutation boundary"
        ))
    })
}

pub(super) fn validate_credential_free_path(path: &Path, label: &str) -> CliResult<()> {
    let value = unicode_path(path, label)?;
    if contains_credential_like_text_str(&value) {
        return Err(CliError::validation_failed(format!(
            "{label} path contains credential-like content"
        )));
    }
    Ok(())
}

pub(super) fn m_string_content(value: &str) -> CliResult<String> {
    if value.chars().any(char::is_control) {
        return Err(CliError::validation_failed(
            "staged resource path contains a control character that cannot cross the M boundary",
        ));
    }
    Ok(value.replace('#', "#(0023)").replace('"', "\"\""))
}

pub(super) fn m_file_path_content(path: &Path, label: &str) -> CliResult<String> {
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

pub(super) fn verify_file_claim(claim: &FileClaim, max_bytes: u64, label: &str) -> CliResult<()> {
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
pub(super) fn sha256_file(path: &Path) -> CliResult<String> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::file_not_found(format!("inspect {}: {error}", path.display()))
    })?;
    sha256_file_bounded(path, MAX_RESOURCE_BYTES, metadata.len())
}

pub(super) fn sha256_file_bounded(
    path: &Path,
    max_bytes: u64,
    expected_len: u64,
) -> CliResult<String> {
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

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex_digest(&hasher.finalize()))
}

pub(super) fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub(super) fn canonical_plain_file(path: &Path, label: &str, max_bytes: u64) -> CliResult<PathBuf> {
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

pub(super) fn canonical_plain_directory(path: &Path, label: &str) -> CliResult<PathBuf> {
    canonical_directory(path, label).map_err(CliError::validation_failed)
}

pub(super) fn resolve_new_directory_candidate(path: &Path) -> CliResult<PathBuf> {
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

pub(super) fn resolve_new_file_candidate(path: &Path, label: &str) -> CliResult<PathBuf> {
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

pub(super) fn require_absent(path: &Path, label: &str) -> CliResult<()> {
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

pub(super) fn hash_workflow_output(output: &Path) -> CliResult<TreeSummary> {
    hash_tree_with_exclusions(
        output,
        &[
            Path::new(WORKFLOW_RECEIPT_FILE),
            Path::new(WORKFLOW_INCOMPLETE_FILE),
        ],
    )
    .map_err(CliError::validation_failed)
}
