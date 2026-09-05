//! Preflight the complete plan output set before publishing any candidate.
use crate::{CliError, CliResult};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) fn path(out: &Path, index: usize) -> PathBuf {
    let mut name = out.as_os_str().to_os_string();
    name.push(format!(".variant-{index}.json"));
    PathBuf::from(name)
}

pub(crate) fn write(files: &[(PathBuf, Value)], force: bool) -> CliResult<()> {
    for (path, _) in files {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if !force || !meta.is_file() || meta.file_type().is_symlink() => {
                return Err(CliError::invalid_args(format!("plan output already exists or is not a regular file: {}", path.display()))
                    .with_pointer("/out")
                    .with_hint("Choose unused output paths, or use --force to replace regular files.")
                    .with_suggested_command("powerbi-cli report plan --schema <schema.json> --objective <objective> --variants 1 --out <new-dashboard.json> --json"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CliError::unexpected(error.to_string())),
        }
    }
    let mut pending = Vec::new();
    if !force {
        let mut created = Vec::new();
        for (path, value) in files {
            if let Err(error) = crate::project_io::write_json_new_atomic(path, value) {
                for path in created {
                    std::fs::remove_file(path).map_err(|error| {
                        CliError::unexpected(format!("output rollback failed: {error}"))
                    })?;
                }
                return Err(error);
            }
            created.push(path);
        }
        return Ok(());
    }
    for (path, value) in files {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| CliError::unexpected(error.to_string()))?;
        }
        let text = serde_json::to_string_pretty(value).expect("JSON value");
        match crate::project_io::begin_text_atomic(path, &text) {
            Ok(write) => pending.push(write),
            Err(error) => {
                for write in pending.into_iter().rev() {
                    write.rollback()?;
                }
                return Err(error);
            }
        }
    }
    for write in &mut pending {
        write.commit_batch()?;
    }
    Ok(())
}
