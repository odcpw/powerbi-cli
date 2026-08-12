use crate::{CliError, CliResult, read_dir_entry, read_json_value};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProject {
    pub(crate) project_dir: PathBuf,
    pub(crate) pbip_path: PathBuf,
    pub(crate) report_dir: PathBuf,
    pub(crate) semantic_model_dir: PathBuf,
}

pub(crate) fn resolve_project(path: &Path) -> CliResult<ResolvedProject> {
    if path.extension().and_then(|value| value.to_str()) == Some("pbip") {
        // `Path::parent()` is an empty path for a root-level relative filename
        // such as `Sales.pbip`. Treat that spelling as the current directory so
        // every command accepts the same convenient project reference.
        let project_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        return resolve_project_from_pbip(&project_dir, path);
    }

    if !path.exists() {
        return Err(CliError::file_not_found(format!(
            "project path does not exist: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(CliError::invalid_args(format!(
            "project path must be a directory or .pbip file: {}",
            path.display()
        )));
    }
    let pbips = fs::read_dir(path)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?
        .map(|entry| read_dir_entry(path, entry, "resolve project directory"))
        .collect::<CliResult<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|entry| entry.extension().and_then(|value| value.to_str()) == Some("pbip"))
        .collect::<Vec<_>>();
    match pbips.as_slice() {
        [pbip] => resolve_project_from_pbip(path, pbip),
        [] => Err(CliError::file_not_found(format!(
            "no .pbip file found in {}",
            path.display()
        ))),
        _ => Err(CliError::invalid_args(format!(
            "multiple .pbip files found in {}; pass the intended .pbip path",
            path.display()
        ))),
    }
}

fn resolve_project_from_pbip(project_dir: &Path, pbip_path: &Path) -> CliResult<ResolvedProject> {
    if !pbip_path.exists() {
        return Err(CliError::file_not_found(format!(
            "pbip file does not exist: {}",
            pbip_path.display()
        )));
    }
    let pbip = read_json_value(pbip_path)?;
    let report_rel = pbip["artifacts"]
        .as_array()
        .and_then(|artifacts| artifacts.first())
        .and_then(|artifact| artifact["report"]["path"].as_str())
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} does not contain artifacts[0].report.path",
                pbip_path.display()
            ))
        })?;
    let report_dir =
        resolve_project_reference(project_dir, project_dir, report_rel, "PBIP report artifact")?;
    let pbir = read_json_value(&report_dir.join("definition.pbir"))?;
    let semantic_rel = pbir["datasetReference"]["byPath"]["path"]
        .as_str()
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} does not contain datasetReference.byPath.path",
                report_dir.join("definition.pbir").display()
            ))
        })?;
    let semantic_model_dir = resolve_project_reference(
        project_dir,
        &report_dir,
        semantic_rel,
        "PBIR semantic-model artifact",
    )?;
    Ok(ResolvedProject {
        project_dir: project_dir.to_path_buf(),
        pbip_path: pbip_path.to_path_buf(),
        report_dir,
        semantic_model_dir,
    })
}

fn clean_relative_path(value: &str) -> CliResult<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::validation_failed(
            "project reference path must not be empty",
        ));
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') || trimmed.contains(':') {
        return Err(CliError::validation_failed(format!(
            "project reference must be relative, got {value}"
        )));
    }
    let mut result = PathBuf::new();
    for part in trimmed.split('/') {
        match part {
            "" | "." => {}
            component => result.push(component),
        }
    }
    Ok(result)
}

fn resolve_project_reference(
    project_root: &Path,
    base: &Path,
    value: &str,
    label: &str,
) -> CliResult<PathBuf> {
    let relative = clean_relative_path(value)?;
    let canonical_root = fs::canonicalize(project_root).map_err(|err| {
        CliError::file_not_found(format!(
            "resolve project root {}: {err}",
            project_root.display()
        ))
    })?;
    let canonical_base = fs::canonicalize(base).map_err(|err| {
        CliError::file_not_found(format!("resolve {label} base {}: {err}", base.display()))
    })?;
    if !canonical_base.starts_with(&canonical_root) {
        return Err(project_reference_escape(label, value));
    }

    let mut target = canonical_base;
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(component) => target.push(component),
            Component::ParentDir => {
                if !target.pop() || !target.starts_with(&canonical_root) {
                    return Err(project_reference_escape(label, value));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(project_reference_escape(label, value));
            }
        }
        if !target.starts_with(&canonical_root) {
            return Err(project_reference_escape(label, value));
        }
    }

    // Existing links must not redirect the selected artifact closure outside
    // the PBIP project. For a missing final artifact, check its nearest
    // existing ancestor so required-file validation can still report the
    // missing in-project path as a normal structured failure.
    let mut existing_ancestor = target.as_path();
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor
            .parent()
            .ok_or_else(|| project_reference_escape(label, value))?;
    }
    let canonical_ancestor = fs::canonicalize(existing_ancestor).map_err(|err| {
        CliError::file_not_found(format!(
            "resolve {label} ancestor {}: {err}",
            existing_ancestor.display()
        ))
    })?;
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(project_reference_escape(label, value));
    }

    if target.exists() {
        let canonical_target = fs::canonicalize(&target).map_err(|err| {
            CliError::file_not_found(format!("resolve {label} {}: {err}", target.display()))
        })?;
        if !canonical_target.starts_with(&canonical_root) {
            return Err(project_reference_escape(label, value));
        }
        return Ok(canonical_target);
    }
    Ok(target)
}

fn project_reference_escape(label: &str, value: &str) -> CliError {
    CliError::validation_failed(format!(
        "{label} reference escapes the selected PBIP project: {value}"
    ))
    .with_hint(
        "Keep report and semantic-model artifacts inside the selected PBIP project directory.",
    )
}
