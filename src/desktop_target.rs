use crate::{CliError, CliResult, ResolvedProject, canonical_display, resolve_project};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use zip::read::ZipArchive;

const MAX_PBIX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DesktopTargetKind {
    Pbip,
    Pbix,
}

impl DesktopTargetKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pbip => "pbip",
            Self::Pbix => "pbix",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PbixArchiveInfo {
    pub(crate) entries: usize,
    pub(crate) has_data_model: bool,
    pub(crate) has_report_definition: bool,
    pub(crate) has_legacy_report_layout: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedDesktopTarget {
    pub(crate) input_path: PathBuf,
    pub(crate) artifact_path: PathBuf,
    pub(crate) project_dir: PathBuf,
    pub(crate) name: String,
    pub(crate) kind: DesktopTargetKind,
    pub(crate) project: Option<ResolvedProject>,
    pub(crate) pbix: Option<PbixArchiveInfo>,
}

impl ResolvedDesktopTarget {
    pub(crate) fn project(&self) -> Option<&ResolvedProject> {
        self.project.as_ref()
    }

    pub(crate) fn require_live_model(&self) -> CliResult<()> {
        if self.kind == DesktopTargetKind::Pbix
            && !self
                .pbix
                .as_ref()
                .is_some_and(|archive| archive.has_data_model)
        {
            return Err(CliError::validation_failed(format!(
                "PBIX has no embedded DataModel for live semantic-model access: {}",
                self.artifact_path.display()
            ))
            .with_hint(
                "Open a PBIX with a local semantic model, or use a PBIP SemanticModel definition for offline authoring.",
            ));
        }
        Ok(())
    }

    pub(crate) fn artifact_json(&self) -> Value {
        let pbix = self.pbix.as_ref();
        json!({
            "kind": self.kind.as_str(),
            "input": self.input_path.display().to_string(),
            "name": self.name,
            "path": canonical_display(&self.artifact_path),
            "projectDir": canonical_display(&self.project_dir),
            "pbip": self.project.as_ref().map(|project| canonical_display(&project.pbip_path)),
            "pbix": (self.kind == DesktopTargetKind::Pbix).then(|| canonical_display(&self.artifact_path)),
            "archive": pbix.map(|archive| json!({
                "entries": archive.entries,
                "hasDataModel": archive.has_data_model,
                "hasReportDefinition": archive.has_report_definition,
                "hasLegacyReportLayout": archive.has_legacy_report_layout
            }))
        })
    }
}

pub(crate) fn resolve_desktop_target(path: &Path) -> CliResult<ResolvedDesktopTarget> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pbix"))
    {
        return resolve_pbix_target(path);
    }

    let project = resolve_project(path).map_err(|error| {
        if path.extension().is_some() {
            error.with_hint("Pass a PBIP project directory, .pbip file, or .pbix Desktop file.")
        } else {
            error
        }
    })?;
    let artifact_path = fs::canonicalize(&project.pbip_path).map_err(|error| {
        CliError::unexpected(format!(
            "resolve PBIP Desktop target {}: {error}",
            project.pbip_path.display()
        ))
    })?;
    let project_dir = fs::canonicalize(&project.project_dir).map_err(|error| {
        CliError::unexpected(format!(
            "resolve PBIP project directory {}: {error}",
            project.project_dir.display()
        ))
    })?;
    let name = artifact_name(&artifact_path)?;
    Ok(ResolvedDesktopTarget {
        input_path: path.to_path_buf(),
        artifact_path,
        project_dir,
        name,
        kind: DesktopTargetKind::Pbip,
        project: Some(project),
        pbix: None,
    })
}

fn resolve_pbix_target(path: &Path) -> CliResult<ResolvedDesktopTarget> {
    let metadata = fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CliError::file_not_found(format!("PBIX file not found: {}", path.display()))
        } else {
            CliError::unexpected(format!("inspect PBIX file {}: {error}", path.display()))
        }
    })?;
    if !metadata.is_file() {
        return Err(CliError::invalid_args(format!(
            "PBIX target is not a regular file: {}",
            path.display()
        )));
    }
    let artifact_path = fs::canonicalize(path).map_err(|error| {
        CliError::unexpected(format!("resolve PBIX target {}: {error}", path.display()))
    })?;
    let archive = inspect_pbix_archive(&artifact_path)?;
    if !archive.has_report_definition && !archive.has_legacy_report_layout {
        return Err(CliError::validation_failed(format!(
            "file has no recognizable Power BI report payload: {}",
            artifact_path.display()
        ))
        .with_hint("Run `powerbi-cli package inspect <file.pbix> --json` for archive details."));
    }
    let project_dir = artifact_path
        .parent()
        .ok_or_else(|| CliError::unexpected("resolved PBIX path has no parent directory"))?
        .to_path_buf();
    let name = artifact_name(&artifact_path)?;
    Ok(ResolvedDesktopTarget {
        input_path: path.to_path_buf(),
        artifact_path,
        project_dir,
        name,
        kind: DesktopTargetKind::Pbix,
        project: None,
        pbix: Some(archive),
    })
}

fn inspect_pbix_archive(path: &Path) -> CliResult<PbixArchiveInfo> {
    let file = File::open(path)
        .map_err(|error| CliError::unexpected(format!("open PBIX {}: {error}", path.display())))?;
    let mut archive = ZipArchive::new(file).map_err(|error| {
        CliError::validation_failed(format!(
            "PBIX is not a readable Power BI package archive: {}: {error}",
            path.display()
        ))
    })?;
    if archive.len() > MAX_PBIX_ENTRIES {
        return Err(CliError::validation_failed(format!(
            "PBIX archive contains {} entries; limit is {MAX_PBIX_ENTRIES}",
            archive.len()
        )));
    }

    let mut has_data_model = false;
    let mut has_report_definition = false;
    let mut has_legacy_report_layout = false;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| {
            CliError::validation_failed(format!(
                "read PBIX archive entry {index} from {}: {error}",
                path.display()
            ))
        })?;
        let name = entry.name().replace('\\', "/").to_ascii_lowercase();
        has_data_model |= name == "datamodel";
        has_report_definition |= name.starts_with("report/definition/");
        has_legacy_report_layout |= name == "report/layout";
    }
    Ok(PbixArchiveInfo {
        entries: archive.len(),
        has_data_model,
        has_report_definition,
        has_legacy_report_layout,
    })
}

fn artifact_name(path: &Path) -> CliResult<String> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            CliError::invalid_args(format!(
                "Desktop target has no valid Unicode file stem: {}",
                path.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn write_pbix(path: &Path, entries: &[&str]) {
        let file = File::create(path).expect("create PBIX fixture");
        let mut archive = zip::ZipWriter::new(file);
        for name in entries {
            archive
                .start_file(*name, SimpleFileOptions::default())
                .expect("start fixture entry");
            archive.write_all(b"fixture").expect("write fixture entry");
        }
        archive.finish().expect("finish PBIX fixture");
    }

    #[test]
    fn resolves_pbix_as_a_desktop_artifact_without_a_fake_project() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pbix = temp.path().join("Example.pbix");
        write_pbix(&pbix, &["Report/definition/report.json", "DataModel"]);

        let target = resolve_desktop_target(&pbix).expect("resolve PBIX");
        assert_eq!(target.kind, DesktopTargetKind::Pbix);
        assert_eq!(target.name, "Example");
        assert!(target.project().is_none());
        assert!(target.pbix.as_ref().is_some_and(|info| info.has_data_model));
        target.require_live_model().expect("embedded model");
    }

    #[test]
    fn rejects_non_power_bi_zip_with_pbix_extension() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pbix = temp.path().join("NotPowerBi.pbix");
        write_pbix(&pbix, &["notes.txt"]);

        let error = resolve_desktop_target(&pbix).expect_err("missing report must fail");
        assert_eq!(error.code, "validation_failed");
    }

    #[test]
    fn thin_pbix_can_open_but_cannot_run_local_model_queries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pbix = temp.path().join("Thin.pbix");
        write_pbix(&pbix, &["Report/Layout"]);

        let target = resolve_desktop_target(&pbix).expect("resolve thin PBIX");
        assert!(target.require_live_model().is_err());
    }
}
