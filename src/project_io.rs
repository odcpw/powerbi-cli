use crate::{CliError, CliResult, walkdir_entry};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use walkdir::WalkDir;

static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn copy_project_dir(source: &Path, out_dir: &Path) -> CliResult<()> {
    if out_dir.exists() && directory_has_entries(out_dir)? {
        return Err(CliError::invalid_args(format!(
            "output directory is not empty: {}",
            out_dir.display()
        ))
        .with_hint("Choose an empty --out-dir, or use --in-place after reviewing --dry-run.")
        .with_suggested_command(
            "powerbi-cli <mutation-command> --project <project-dir-or.pbip> --dry-run --json",
        ));
    }
    reject_nested_output(source, out_dir)?;

    fs::create_dir_all(out_dir)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", out_dir.display())))?;
    for entry in WalkDir::new(source)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
    {
        let entry = walkdir_entry(source, entry, "walk project copy source")?;
        let from = entry.path();
        let relative = from
            .strip_prefix(source)
            .map_err(|err| CliError::unexpected(format!("copy project path: {err}")))?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        let to = out_dir.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&to)
                .map_err(|err| CliError::unexpected(format!("create {}: {err}", to.display())))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    CliError::unexpected(format!("create {}: {err}", parent.display()))
                })?;
            }
            fs::copy(from, &to).map_err(|err| {
                CliError::unexpected(format!(
                    "copy {} to {}: {err}",
                    from.display(),
                    to.display()
                ))
            })?;
        }
    }
    Ok(())
}

pub(crate) fn write_text_atomic(path: &Path, text: &str) -> CliResult<()> {
    begin_text_atomic(path, text)?.commit()
}

pub(crate) fn write_text_atomic_validated<T>(
    path: &Path,
    text: &str,
    validate: impl FnOnce() -> CliResult<T>,
    is_valid: impl FnOnce(&T) -> bool,
) -> CliResult<(T, bool)> {
    let pending = begin_text_atomic(path, text)?;
    match validate() {
        Ok(validation) => {
            if is_valid(&validation) {
                pending.commit()?;
                Ok((validation, true))
            } else {
                pending.rollback()?;
                Ok((validation, false))
            }
        }
        Err(validation_error) => match pending.rollback() {
            Ok(()) => Err(validation_error),
            Err(rollback_error) => Err(CliError::unexpected(format!(
                "{}; rollback after validation error also failed: {}",
                validation_error.message, rollback_error.message
            ))),
        },
    }
}

struct PendingTextWrite {
    path: PathBuf,
    backup: Option<PathBuf>,
}

impl PendingTextWrite {
    fn commit(self) -> CliResult<()> {
        if let Some(backup) = self.backup {
            fs::remove_file(&backup).map_err(|err| {
                CliError::unexpected(format!(
                    "atomic replace succeeded for {}, but backup cleanup failed; backup retained at {}: {err}",
                    self.path.display(),
                    backup.display()
                ))
            })?;
        }
        Ok(())
    }

    fn rollback(self) -> CliResult<()> {
        let Some(backup) = self.backup else {
            return fs::remove_file(&self.path).map_err(|err| {
                CliError::unexpected(format!(
                    "remove newly written file {} during rollback: {err}",
                    self.path.display()
                ))
            });
        };

        let parent = self.path.parent().ok_or_else(|| {
            CliError::unexpected(format!("path has no parent: {}", self.path.display()))
        })?;
        let file_name = atomic_file_name(&self.path);
        let displaced = unique_sibling_path(parent, &file_name, "rollback");
        fs::rename(&self.path, &displaced).map_err(|err| {
            CliError::unexpected(format!(
                "prepare rollback of {}; original backup retained at {}: {err}",
                self.path.display(),
                backup.display()
            ))
        })?;

        if let Err(restore_error) = fs::rename(&backup, &self.path) {
            let reinstall_error = fs::rename(&displaced, &self.path).err();
            let reinstall_detail = reinstall_error.map_or_else(
                || "mutated file was reinstalled".to_string(),
                |err| {
                    format!(
                        "mutated file could not be reinstalled and remains at {}: {err}",
                        displaced.display()
                    )
                },
            );
            return Err(CliError::unexpected(format!(
                "restore original {} from backup failed; original backup retained at {}: {restore_error}; {reinstall_detail}",
                self.path.display(),
                backup.display()
            )));
        }

        fs::remove_file(&displaced).map_err(|err| {
            CliError::unexpected(format!(
                "original {} was restored, but rolled-back content remains at {}: {err}",
                self.path.display(),
                displaced.display()
            ))
        })
    }
}

fn begin_text_atomic(path: &Path, text: &str) -> CliResult<PendingTextWrite> {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::unexpected(format!("path has no parent: {}", path.display())))?;
    let file_name = atomic_file_name(path);
    let tmp = unique_sibling_path(parent, &file_name, "tmp");
    let backup = unique_sibling_path(parent, &file_name, "bak");

    write_synced_temp(&tmp, text)?;

    let backup = if path.exists() {
        let metadata = fs::symlink_metadata(path).map_err(|err| {
            CliError::unexpected(format!(
                "inspect atomic replace target {}: {err}",
                path.display()
            ))
        })?;
        if !metadata.file_type().is_file() {
            let cleanup_detail = cleanup_temp_after_error(&tmp);
            return Err(CliError::unexpected(format!(
                "atomic replace target is not a regular file: {}{cleanup_detail}",
                path.display(),
            )));
        }
        if let Err(err) = fs::rename(path, &backup) {
            let cleanup_detail = cleanup_temp_after_error(&tmp);
            return Err(CliError::unexpected(format!(
                "move original {} to backup {}: {err}{cleanup_detail}",
                path.display(),
                backup.display()
            )));
        }
        Some(backup)
    } else {
        None
    };

    install_atomic_temp(path, &tmp, backup.as_deref())?;
    Ok(PendingTextWrite {
        path: path.to_path_buf(),
        backup,
    })
}

fn install_atomic_temp(path: &Path, tmp: &Path, backup: Option<&Path>) -> CliResult<()> {
    if let Err(replace_error) = fs::rename(tmp, path) {
        if let Some(backup) = backup {
            if let Err(restore_error) = fs::rename(backup, path) {
                return Err(CliError::unexpected(format!(
                    "replace {} with {} failed: {replace_error}; restoring the original also failed: {restore_error}; original backup retained at {}",
                    path.display(),
                    tmp.display(),
                    backup.display()
                )));
            }
            let cleanup_detail = cleanup_temp_after_error(tmp);
            return Err(CliError::unexpected(format!(
                "replace {} with {} failed and the original was restored: {replace_error}{cleanup_detail}",
                path.display(),
                tmp.display()
            )));
        }
        let cleanup_detail = cleanup_temp_after_error(tmp);
        return Err(CliError::unexpected(format!(
            "install new file {} from {}: {replace_error}{cleanup_detail}",
            path.display(),
            tmp.display()
        )));
    }
    Ok(())
}

fn write_synced_temp(path: &Path, text: &str) -> CliResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", path.display())))?;
    if let Err(err) = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let cleanup_detail = cleanup_temp_after_error(path);
        return Err(CliError::unexpected(format!(
            "write and sync {}: {err}{cleanup_detail}",
            path.display()
        )));
    }
    Ok(())
}

fn cleanup_temp_after_error(path: &Path) -> String {
    match fs::remove_file(path) {
        Ok(()) => String::new(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => format!(
            "; temporary file could not be removed and remains at {}: {err}",
            path.display()
        ),
    }
}

fn atomic_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("powerbi-cli")
        .to_string()
}

fn unique_sibling_path(parent: &Path, file_name: &str, extension: &str) -> PathBuf {
    let counter = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{file_name}.{}.{}.{}",
        std::process::id(),
        counter,
        extension
    ))
}

pub(crate) fn write_json_atomic(path: &Path, value: &Value) -> CliResult<()> {
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    write_text_atomic(path, &text)
}

/// Atomically publishes a new JSON file without ever replacing an existing
/// target.
///
/// The temporary file is created beside the target, written, and synced before
/// `hard_link` performs the publication. Creating a hard link is an atomic
/// create-only operation: it fails if another process wins the target-path
/// race, while keeping the synced bytes on the same filesystem. The
/// invocation-owned temporary name is removed after either outcome.
pub(crate) fn write_json_new_atomic(path: &Path, value: &Value) -> CliResult<()> {
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    let file_name = atomic_file_name(path);
    let tmp = unique_sibling_path(parent, &file_name, "new");
    write_synced_temp(&tmp, &text)?;
    publish_new_atomic_temp(path, &tmp)
}

fn publish_new_atomic_temp(path: &Path, tmp: &Path) -> CliResult<()> {
    match fs::hard_link(tmp, path) {
        Ok(()) => fs::remove_file(tmp).map_err(|err| {
            CliError::unexpected(format!(
                "published new file {}, but could not remove invocation-owned temporary file {}: {err}",
                path.display(),
                tmp.display()
            ))
        }),
        Err(publish_error) => {
            let target_exists = publish_error.kind() == std::io::ErrorKind::AlreadyExists
                || fs::symlink_metadata(path).is_ok();
            let cleanup_detail = cleanup_temp_after_error(tmp);
            if target_exists {
                Err(CliError::invalid_args(format!(
                    "output file already exists and was not replaced: {}{cleanup_detail}",
                    path.display()
                )))
            } else {
                Err(CliError::unexpected(format!(
                    "atomically publish new file {} from {}: {publish_error}{cleanup_detail}",
                    path.display(),
                    tmp.display()
                )))
            }
        }
    }
}

pub(crate) fn write_json_pretty(path: &Path, value: &Value) -> CliResult<()> {
    if path.exists() {
        return write_json_atomic(path, value);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    fs::write(path, text)
        .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))
}

fn directory_has_entries(path: &Path) -> CliResult<bool> {
    let mut entries = fs::read_dir(path)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?;
    Ok(entries.next().is_some())
}

fn reject_nested_output(source: &Path, out_dir: &Path) -> CliResult<()> {
    let source_abs = fs::canonicalize(source)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", source.display())))?;
    match fs::symlink_metadata(out_dir) {
        Ok(metadata) => {
            if output_metadata_is_link_or_reparse(&metadata) {
                return Err(CliError::invalid_args(format!(
                    "--out-dir must not be a symlink, junction, or reparse point: {}",
                    out_dir.display()
                ))
                .with_hint("Choose a real sibling or separate directory so the project-copy destination cannot alias the source."));
            }
            let out_abs = fs::canonicalize(out_dir).map_err(|err| {
                CliError::unexpected(format!("resolve {}: {err}", out_dir.display()))
            })?;
            return reject_resolved_nested_output(&source_abs, &out_abs, out_dir);
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(CliError::unexpected(format!(
                "inspect output target {}: {err}",
                out_dir.display()
            )));
        }
    }
    let parent = out_dir.parent().unwrap_or_else(|| Path::new("."));
    let parent_abs = if parent.exists() {
        fs::canonicalize(parent)
            .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", parent.display())))?
    } else {
        fs::canonicalize(parent.parent().unwrap_or_else(|| Path::new("."))).map_err(|err| {
            CliError::unexpected(format!("resolve parent for {}: {err}", out_dir.display()))
        })?
    };
    let out_abs = parent_abs.join(out_dir.file_name().unwrap_or(out_dir.as_os_str()));
    reject_resolved_nested_output(&source_abs, &out_abs, out_dir)
}

fn reject_resolved_nested_output(
    source_abs: &Path,
    out_abs: &Path,
    display_path: &Path,
) -> CliResult<()> {
    if out_abs.starts_with(source_abs) {
        return Err(CliError::invalid_args(format!(
            "--out-dir must not be inside the source project: {}",
            display_path.display()
        ))
        .with_hint("Choose a sibling or separate output directory so project copy cannot recurse.")
        .with_suggested_command(
            "powerbi-cli <mutation-command> --project <project-dir-or.pbip> --dry-run --json",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_replace_moves_original_to_backup_and_commits_new_text() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("model.tmdl");
        fs::write(&path, "original").expect("write original");

        write_text_atomic(&path, "replacement").expect("atomic replace");

        assert_eq!(
            fs::read_to_string(&path).expect("read replacement"),
            "replacement"
        );
        let entries = fs::read_dir(temp.path())
            .expect("read tempdir")
            .collect::<Result<Vec<_>, _>>()
            .expect("read entries");
        assert_eq!(
            entries.len(),
            1,
            "successful replace must remove its backup"
        );
    }

    #[test]
    fn new_atomic_json_creates_parent_and_never_clobbers_existing_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("nested").join("plan.json");
        let first = serde_json::json!({"winner": "first"});
        let second = serde_json::json!({"winner": "second"});

        write_json_new_atomic(&path, &first).expect("publish first JSON");
        let original = fs::read(&path).expect("read first JSON bytes");
        let error = write_json_new_atomic(&path, &second)
            .expect_err("an existing target must never be replaced");

        assert_eq!(error.code, "invalid_args");
        assert_eq!(
            fs::read(&path).expect("read winning JSON bytes"),
            original,
            "the losing publication must not alter the existing target"
        );
        let entries = fs::read_dir(path.parent().expect("target parent"))
            .expect("read target parent")
            .collect::<Result<Vec<_>, _>>()
            .expect("read entries");
        assert_eq!(entries.len(), 1, "no invocation-owned temp may remain");
    }

    #[test]
    fn new_atomic_publication_loses_target_race_without_altering_winner() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("receipt.json");
        let tmp = temp.path().join(".receipt.json.racing.new");
        write_synced_temp(&tmp, "candidate").expect("write synced candidate");

        // Simulate another process publishing after our temporary file is
        // synced but immediately before our atomic create-only publication.
        fs::write(&path, "race winner").expect("publish race winner");
        let error = publish_new_atomic_temp(&path, &tmp)
            .expect_err("the racing publication must win without replacement");

        assert_eq!(error.code, "invalid_args");
        assert_eq!(
            fs::read_to_string(&path).expect("read race winner"),
            "race winner"
        );
        assert!(
            !tmp.exists(),
            "the losing invocation must remove its owned temporary file"
        );
    }

    #[test]
    fn project_copy_skips_git_repository_internals() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let output = temp.path().join("output");
        fs::create_dir_all(source.join(".git").join("objects")).expect("git objects");
        fs::write(source.join("Report.pbip"), "{}\n").expect("project file");
        fs::write(source.join(".git").join("HEAD"), "ref: refs/heads/main\n").expect("git head");
        fs::write(
            source.join(".git").join("objects").join("object"),
            "repository data",
        )
        .expect("git object");

        copy_project_dir(&source, &output).expect("copy project");

        assert!(output.join("Report.pbip").is_file());
        assert!(
            !output.join(".git").exists(),
            "project copies must never embed repository internals"
        );
    }

    #[test]
    fn failed_install_restores_original_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("model.tmdl");
        let backup = temp.path().join("model.backup");
        let missing_tmp = temp.path().join("missing.tmp");
        fs::write(&backup, "original").expect("write backup");

        let error = install_atomic_temp(&path, &missing_tmp, Some(&backup))
            .expect_err("missing temporary file must fail");

        assert!(error.message.contains("the original was restored"));
        assert_eq!(
            fs::read_to_string(&path).expect("read restored file"),
            "original"
        );
        assert!(
            !backup.exists(),
            "restored backup must move back to the target"
        );
    }

    #[test]
    fn failed_restore_keeps_backup_and_reports_its_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("model.tmdl");
        let backup = temp.path().join("model.backup");
        let tmp = temp.path().join("replacement.tmp");
        fs::create_dir(&path).expect("occupy target with directory");
        fs::write(&backup, "original").expect("write backup");
        fs::write(&tmp, "replacement").expect("write temporary replacement");

        let error = install_atomic_temp(&path, &tmp, Some(&backup))
            .expect_err("occupied target must prevent install and restore");

        assert!(
            backup.is_file(),
            "failed restore must retain the original backup"
        );
        assert!(
            tmp.is_file(),
            "failed install must retain the replacement for diagnosis"
        );
        assert!(
            error.message.contains(&backup.display().to_string()),
            "error must identify retained backup: {}",
            error.message
        );
    }

    #[test]
    fn validation_failure_rolls_back_and_reports_unmodified() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("model.tmdl");
        fs::write(&path, "valid original").expect("write original");

        let (validation, modified) = write_text_atomic_validated(
            &path,
            "invalid replacement",
            || Ok::<_, CliError>("relationship references a missing column"),
            |_| false,
        )
        .expect("validation rollback");

        assert_eq!(validation, "relationship references a missing column");
        assert!(!modified);
        assert_eq!(
            fs::read_to_string(&path).expect("read restored file"),
            "valid original"
        );
    }

    #[test]
    fn successful_validation_commits_and_reports_modified() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("model.tmdl");
        fs::write(&path, "original").expect("write original");

        let (_, modified) = write_text_atomic_validated(
            &path,
            "valid replacement",
            || Ok::<_, CliError>(()),
            |_| true,
        )
        .expect("validated replace");

        assert!(modified);
        assert_eq!(
            fs::read_to_string(&path).expect("read committed file"),
            "valid replacement"
        );
    }
}

fn output_metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod nested_output_tests {
    use super::*;

    #[test]
    fn existing_output_is_compared_by_its_canonical_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let nested = source.join("empty-output");
        fs::create_dir_all(&nested).expect("nested output");

        let err = reject_nested_output(&source, &nested).expect_err("nested output rejected");
        assert_eq!(err.code, "invalid_args");
        assert!(err.message.contains("must not be inside"));
    }

    #[test]
    fn resolved_alias_inside_source_is_rejected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let nested_target = source.join("nested-target");
        fs::create_dir_all(&nested_target).expect("nested target");
        let source_abs = fs::canonicalize(&source).expect("canonical source");
        let target_abs = fs::canonicalize(&nested_target).expect("canonical target");
        let outside_alias = temp.path().join("outside-junction");

        let err = reject_resolved_nested_output(&source_abs, &target_abs, &outside_alias)
            .expect_err("resolved alias into source rejected");
        assert_eq!(err.code, "invalid_args");
        assert!(err.message.contains("outside-junction"));
    }

    #[test]
    fn resolved_sibling_output_is_allowed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let sibling = temp.path().join("sibling-output");
        fs::create_dir_all(&source).expect("source");
        fs::create_dir_all(&sibling).expect("sibling");
        let source_abs = fs::canonicalize(&source).expect("canonical source");
        let sibling_abs = fs::canonicalize(&sibling).expect("canonical sibling");

        reject_resolved_nested_output(&source_abs, &sibling_abs, &sibling)
            .expect("sibling output allowed");
    }

    #[cfg(windows)]
    #[test]
    fn directory_symlink_output_is_rejected_when_creation_is_available() {
        use std::os::windows::fs::symlink_dir;

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let target = source.join("empty-target");
        let output = temp.path().join("output-link");
        fs::create_dir_all(&target).expect("target");
        if let Err(err) = symlink_dir(&target, &output) {
            if err.kind() == std::io::ErrorKind::PermissionDenied
                || err.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("create directory symlink: {err}");
        }

        let err = reject_nested_output(&source, &output).expect_err("symlink output rejected");
        assert_eq!(err.code, "invalid_args");
        assert!(err.message.contains("symlink, junction, or reparse point"));
    }
}
