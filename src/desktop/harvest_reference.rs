//! Archive a safe PBIR fragment from an already-saved Desktop project.
//!
//! The harvester is deliberately usable on Linux: it never launches Desktop
//! and therefore cannot claim a Desktop canvas proof.  It resolves a stable
//! page/visual/report handle, runs the selected file through the shared
//! harvested-fragment safety reader, and writes a provenance-bearing archive
//! outside the source project.

use crate::desktop_proof::{DESKTOP_PROOF_SCHEMA, ProofLevel};
use crate::input_safety::{
    INPUT_SAFETY_ERROR_CODE, InputKind, MAX_PROJECT_TEXT_BYTES, MAX_SNAPSHOT_BYTES,
    MAX_SNAPSHOT_FILES, read_bytes, read_harvested_fragment, validate_text,
};
use crate::pbir::{PageSelector, VisualSelector, find_page, find_visual, load_report_snapshot};
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const COMMAND: &str = "desktop harvest-reference";
const REFERENCE_SCHEMA: &str = "powerbi-cli.desktop-reference.v1";
const DEFAULT_LICENSE_NOTE: &str =
    "Source-project license and redistribution terms must be preserved with this reference.";

#[derive(Debug, Default)]
struct HarvestOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    out: Option<PathBuf>,
    desktop_version: Option<String>,
    license_note: Option<String>,
    dry_run: bool,
}

#[derive(Debug, Clone, Copy)]
enum FragmentKind {
    Visual,
    Page,
    Report,
}

impl FragmentKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Visual => "visual",
            Self::Page => "page",
            Self::Report => "report",
        }
    }

    const fn file_name(self) -> &'static str {
        match self {
            Self::Visual => "visual.json",
            Self::Page => "page.json",
            Self::Report => "report.json",
        }
    }
}

#[derive(Debug)]
struct FragmentSelection {
    kind: FragmentKind,
    handle: String,
    path: PathBuf,
}

/// Archive one Desktop-saved visual, page, or report fragment.
pub(crate) fn harvest_reference_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    let project = options.project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{COMMAND} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass an already-saved PBIP project directory or .pbip file.")
        .with_suggested_command(
            "powerbi-cli desktop harvest-reference --project <project-dir-or.pbip> --visual <handle> --out <reference.json> --json",
        )
    })?;
    let handle = options.handle.ok_or_else(|| {
        CliError::invalid_args(format!("{COMMAND} requires --visual <handle>"))
            .with_hint("Use `report pages list` or `report visuals list` for stable handles.")
            .with_suggested_command(
                "powerbi-cli desktop harvest-reference --project <project-dir-or.pbip> --visual visual:<page>:<visual> --out <reference.json> --json",
            )
    })?;
    let out = options.out.ok_or_else(|| {
        CliError::invalid_args(format!("{COMMAND} requires --out <reference.json>"))
            .with_hint("Choose a JSON path outside the selected PBIP project.")
            .with_suggested_command(
                "powerbi-cli desktop harvest-reference --project <project-dir-or.pbip> --visual <handle> --out docs/reference/desktop-authored-visuals/<name>.json --json",
            )
    })?;
    let license_note = options
        .license_note
        .unwrap_or_else(|| DEFAULT_LICENSE_NOTE.to_string());
    validate_text(&license_note, InputKind::SourceText)?;
    let desktop_version = options
        .desktop_version
        .unwrap_or_else(|| "unknown".to_string());
    if desktop_version.trim().is_empty() {
        return Err(CliError::invalid_args("--desktop-version must not be empty")
            .with_hint("Omit --desktop-version when the saved project has no version evidence; the archive records unknown.")
            .with_suggested_command(
                "powerbi-cli desktop harvest-reference --project <project-dir-or.pbip> --visual <handle> --out <reference.json> --json",
            ));
    }
    validate_text(&desktop_version, InputKind::SourceText)?;

    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let selection = select_fragment(&snapshot.pages, &resolved.report_dir, &handle)?;
    let source_path = canonical_source_path(&selection, &resolved.project_dir)?;

    // This is the shared safety boundary for Desktop-authored input.  In
    // particular, persisted slicer/filter values are refused rather than
    // silently removed.  The archive is never published if this call fails.
    let fragment = read_safe_fragment(&source_path)?;
    let source_bytes = read_bytes(&source_path, InputKind::HarvestedFragment)?;
    let fragment_sha256 = sha256_bytes(&source_bytes);
    let source_fingerprint = project_fingerprint(&resolved.project_dir)?;

    let destination = output_destination(&out, &resolved.project_dir, !options.dry_run)?;
    let name = destination
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::invalid_args("--out must have a Unicode file name"))?
        .to_string();
    let project_arg = command_arg(&resolved.project_dir);
    let out_arg = command_arg(&destination);
    let command_line = format!(
        "powerbi-cli {COMMAND} --project {project_arg} --visual {} --out {out_arg} --json",
        crate::cli_support::shell_arg(&handle)
    );
    let date = utc_date();
    let proof = proof_record(
        &selection,
        &name,
        &date,
        &desktop_version,
        &command_line,
        &source_path,
        &source_fingerprint,
    );
    let provenance = json!({
        "sourcePath": canonical_display(&source_path),
        "sourceProject": canonical_display(&resolved.project_dir),
        "desktopVersion": desktop_version,
        "date": date,
        "sourceFingerprint": source_fingerprint,
        "fragmentSha256": fragment_sha256,
        "licenseNote": license_note
    });
    let artifact = json!({
        "schema": REFERENCE_SCHEMA,
        "kind": selection.kind.as_str(),
        "handle": selection.handle,
        "name": name,
        "provenance": provenance,
        "proofLevel": ProofLevel::DesktopGoldenPending.as_str(),
        "proof": proof,
        "safety": {
            "persistedDataValues": "refused",
            "silentStripping": false,
            "desktopCompatibility": ProofLevel::DesktopGoldenPending.as_str()
        },
        "fragment": fragment
    });

    if !options.dry_run {
        write_json_atomic(&destination, &artifact)?;
    }

    let action = if options.dry_run { "preview" } else { "write" };
    let readback = match selection.kind {
        FragmentKind::Visual => format!(
            "powerbi-cli report visuals show --project {project_arg} --handle {} --json",
            crate::cli_support::shell_arg(&handle)
        ),
        FragmentKind::Page => format!(
            "powerbi-cli report pages show --project {project_arg} --handle {} --json",
            crate::cli_support::shell_arg(&handle)
        ),
        FragmentKind::Report => format!("powerbi-cli inspect --deep {project_arg} --json"),
    };
    let next = vec![
        readback,
        format!("powerbi-cli validate --strict {project_arg} --json"),
        format!(
            "powerbi-cli desktop harvest-reference --project {project_arg} --visual {} --out {out_arg} --json",
            crate::cli_support::shell_arg(&handle)
        ),
    ];
    Ok(json!({
        "schema": "powerbi-cli.desktop.harvestReference.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "action": action,
        "dryRun": options.dry_run,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "out": canonical_display(&destination),
        "source": {
            "kind": selection.kind.as_str(),
            "handle": selection.handle,
            "path": canonical_display(&source_path),
            "sha256": fragment_sha256,
            "sourceFingerprint": source_fingerprint
        },
        "destination": {
            "path": canonical_display(&destination),
            "name": name,
            "created": !options.dry_run
        },
        "provenance": provenance,
        "proofLevel": ProofLevel::DesktopGoldenPending.as_str(),
        "proof": proof,
        "safety": {
            "persistedDataValues": "refused",
            "silentStripping": false,
            "desktopCompatibility": ProofLevel::DesktopGoldenPending.as_str()
        },
        "changes": [{
            "path": canonical_display(&destination),
            "kind": "desktop-reference",
            "action": action,
            "before": if options.dry_run && destination.is_file() { Value::String("existing-file".to_string()) } else { Value::Null },
            "after": if options.dry_run { Value::String("provenance-stamped-reference".to_string()) } else { Value::String(canonical_display(&destination)) }
        }],
        "next": next,
        "notes": [
            "The source was read through input_safety::read_harvested_fragment; persisted selection/filter values are refused, never silently stripped.",
            "Already-saved projects on Linux do not carry Desktop execution evidence; this archive remains desktop-golden-pending."
        ],
        "warnings": [],
        "errors": []
    }))
}

fn parse_args(args: &[String]) -> CliResult<HarvestOptions> {
    let mut options = HarvestOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut index, "--project")?));
            }
            "--visual" | "--handle" => {
                options.handle = Some(take_value(args, &mut index, "--visual")?);
            }
            "--out" => {
                options.out = Some(PathBuf::from(take_value(args, &mut index, "--out")?));
            }
            "--desktop-version" => {
                options.desktop_version = Some(take_value(args, &mut index, "--desktop-version")?);
            }
            "--license-note" => {
                options.license_note = Some(take_value(args, &mut index, "--license-note")?);
            }
            "--dry-run" => {
                if options.dry_run {
                    return Err(CliError::invalid_args("choose --dry-run only once")
                        .with_suggested_command(
                            "powerbi-cli desktop harvest-reference --project <project-dir-or.pbip> --visual <handle> --out <reference.json> --dry-run --json",
                        ));
                }
                options.dry_run = true;
                index += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown {COMMAND} flag: {other}"
                ))
                .with_hint("Use --project, --visual, --out, and optional --desktop-version, --license-note, or --dry-run.")
                .with_suggested_command(
                    "powerbi-cli desktop harvest-reference --project <project-dir-or.pbip> --visual <handle> --out <reference.json> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli --json capabilities --for desktop` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for desktop")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn select_fragment(
    pages: &[crate::pbir::PageRecord],
    report_dir: &Path,
    handle: &str,
) -> CliResult<FragmentSelection> {
    // `inspect --deep` exposes the report root as `report:main`; retain the
    // short `report` spelling as a convenient alias for command examples.
    if matches!(handle, "report" | "report:main") {
        return Ok(FragmentSelection {
            kind: FragmentKind::Report,
            handle: "report:main".to_string(),
            path: report_dir.join("definition").join("report.json"),
        });
    }
    if handle.starts_with("visual:") {
        let visual = find_visual(
            pages,
            &VisualSelector {
                handle: Some(handle.to_string()),
                page: None,
                visual: None,
            },
            COMMAND,
        )?;
        return Ok(FragmentSelection {
            kind: FragmentKind::Visual,
            handle: visual.handle.clone(),
            path: visual.path.clone().ok_or_else(|| {
                CliError::validation_failed(format!(
                    "visual has no visual.json path in inspect output: {}",
                    visual.handle
                ))
            })?,
        });
    }
    if handle.starts_with("page:") {
        let page = find_page(
            pages,
            &PageSelector {
                handle: Some(handle.to_string()),
                name: None,
            },
            COMMAND,
        )?;
        return Ok(FragmentSelection {
            kind: FragmentKind::Page,
            handle: page.handle.clone(),
            path: page.path.clone().ok_or_else(|| {
                CliError::validation_failed(format!(
                    "page has no page.json path in inspect output: {}",
                    page.handle
                ))
            })?,
        });
    }
    Err(CliError::invalid_args(format!(
        "{COMMAND} requires a stable visual:<page>:<name>, page:<name>, or report:main handle; got {handle}"
    ))
    .with_hint("Use `report pages list` or `report visuals list` to obtain a stable handle.")
    .with_suggested_command(format!(
        "powerbi-cli {COMMAND} --project <project-dir-or.pbip> --visual visual:<page>:<visual> --out <reference.json> --json"
    )))
}

fn canonical_source_path(selection: &FragmentSelection, project_root: &Path) -> CliResult<PathBuf> {
    let expected = selection.kind.file_name();
    if selection.path.file_name().and_then(|value| value.to_str()) != Some(expected) {
        return Err(safety_refusal(format!(
            "selected {} fragment is not backed by {expected}: {}",
            selection.kind.as_str(),
            selection.path.display()
        )));
    }
    let source = fs::canonicalize(&selection.path).map_err(|error| {
        CliError::file_not_found(format!(
            "resolve harvested {} fragment {}: {error}",
            selection.kind.as_str(),
            selection.path.display()
        ))
    })?;
    if !source.starts_with(project_root) {
        return Err(safety_refusal(format!(
            "selected fragment escapes the source project: {}",
            source.display()
        )));
    }
    Ok(source)
}

fn read_safe_fragment(path: &Path) -> CliResult<Value> {
    let value = read_harvested_fragment(path)?;
    // The shared reader catches explicit cached/selected-value keys. Desktop
    // also serializes a selected slicer item as a filter comparison whose
    // literal lives below `filter/.../Value`; treat that shape as persisted
    // state here so the archive never carries a selection accidentally.
    if let Some(pointer) = persisted_selection_pointer(&value, "", false) {
        return Err(safety_refusal(format!(
            "persisted data values remain at {pointer}"
        )));
    }
    Ok(value)
}

fn persisted_selection_pointer(value: &Value, path: &str, in_filter: bool) -> Option<String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let pointer = format!("{path}/{}", escape_json_pointer(key));
                let lower = key.to_ascii_lowercase();
                let child_in_filter = in_filter
                    || matches!(
                        lower.as_str(),
                        "filter" | "filterconfig" | "where" | "condition" | "in"
                    );
                if child_in_filter && matches!(lower.as_str(), "value" | "values") {
                    return Some(pointer);
                }
                if let Some(found) = persisted_selection_pointer(child, &pointer, child_in_filter) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().enumerate().find_map(|(index, child)| {
            persisted_selection_pointer(child, &format!("{path}/{index}"), in_filter)
        }),
        _ => None,
    }
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn output_destination(
    requested: &Path,
    project_root: &Path,
    create_parent: bool,
) -> CliResult<PathBuf> {
    if requested.extension().and_then(|value| value.to_str()) != Some("json") {
        return Err(CliError::invalid_args(format!(
            "{COMMAND} --out must end in .json: {}",
            requested.display()
        ))
        .with_hint("Use docs/reference/desktop-authored-visuals/<name>.json or another JSON path outside the project."));
    }
    let parent = requested
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    reject_linked_ancestors(parent)?;
    if create_parent {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::unexpected(format!(
                "create harvest-reference output directory {}: {error}",
                parent.display()
            ))
        })?;
        reject_linked_ancestors(parent)?;
    }
    let parent = canonical_output_parent(parent)?;
    if parent.starts_with(project_root) {
        return Err(CliError::invalid_args(format!(
            "{COMMAND} output must be outside the source project: {}",
            requested.display()
        ))
        .with_hint(
            "Choose docs/reference/desktop-authored-visuals or another sibling/output directory.",
        ));
    }
    let file_name = requested
        .file_name()
        .ok_or_else(|| CliError::invalid_args(format!("{COMMAND} --out must name a JSON file")))?;
    let destination = parent.join(file_name);
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(safety_refusal(format!(
                "output target must be an ordinary file: {}",
                destination.display()
            )));
        }
    }
    Ok(destination)
}

fn canonical_output_parent(path: &Path) -> CliResult<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|error| {
            CliError::file_not_found(format!(
                "resolve harvest-reference output directory {}: {error}",
                path.display()
            ))
        });
    }
    let mut missing = Vec::new();
    let mut current = path;
    while !current.exists() {
        let name = current.file_name().ok_or_else(|| {
            CliError::file_not_found(format!(
                "resolve harvest-reference output directory {}",
                path.display()
            ))
        })?;
        missing.push(name.to_os_string());
        current = current.parent().ok_or_else(|| {
            CliError::file_not_found(format!(
                "resolve harvest-reference output directory {}",
                path.display()
            ))
        })?;
    }
    reject_linked_ancestors(current)?;
    let mut canonical = fs::canonicalize(current).map_err(|error| {
        CliError::file_not_found(format!(
            "resolve harvest-reference output directory {}: {error}",
            current.display()
        ))
    })?;
    for name in missing.iter().rev() {
        canonical.push(name);
    }
    Ok(canonical)
}

fn reject_linked_ancestors(path: &Path) -> CliResult<()> {
    let mut current = path;
    loop {
        match fs::symlink_metadata(current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(safety_refusal(format!(
                        "output directory contains a symbolic link: {}",
                        current.display()
                    )));
                }
                if !metadata.is_dir() {
                    return Err(CliError::invalid_args(format!(
                        "harvest-reference output parent is not a directory: {}",
                        current.display()
                    )));
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                current = current
                    .parent()
                    .ok_or_else(|| safety_refusal("output directory has no existing ancestor"))?;
            }
            Err(error) => {
                return Err(safety_refusal(format!(
                    "inspect output directory {}: {error}",
                    current.display()
                )));
            }
        }
    }
}

fn project_fingerprint(root: &Path) -> CliResult<String> {
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry =
            entry.map_err(|error| safety_refusal(format!("walk source project: {error}")))?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            safety_refusal(format!(
                "inspect source project entry {}: {error}",
                entry.path().display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(safety_refusal(format!(
                "source project contains a symbolic link: {}",
                entry.path().display()
            )));
        }
        if metadata.is_file() {
            if metadata.len() > MAX_PROJECT_TEXT_BYTES {
                return Err(safety_refusal(format!(
                    "source project file exceeds {} bytes: {}",
                    MAX_PROJECT_TEXT_BYTES,
                    entry.path().display()
                )));
            }
            total_bytes = total_bytes.saturating_add(metadata.len());
            if files.len() >= MAX_SNAPSHOT_FILES || total_bytes > MAX_SNAPSHOT_BYTES {
                return Err(safety_refusal(format!(
                    "source project exceeds {MAX_SNAPSHOT_FILES} files or {MAX_SNAPSHOT_BYTES} bytes"
                )));
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| safety_refusal("source project entry escaped the project root"))?
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            files.push((relative, entry.path().to_path_buf(), metadata.len()));
        } else if !metadata.is_dir() {
            return Err(safety_refusal(format!(
                "source project contains a non-file entry: {}",
                entry.path().display()
            )));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (relative, path, expected_len) in files {
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(b"file\0");
        let mut file = File::open(&path).map_err(|error| {
            safety_refusal(format!(
                "read source project file {}: {error}",
                path.display()
            ))
        })?;
        let mut bytes = Vec::with_capacity(expected_len as usize);
        file.read_to_end(&mut bytes).map_err(|error| {
            safety_refusal(format!(
                "hash source project file {}: {error}",
                path.display()
            ))
        })?;
        if bytes.len() as u64 != expected_len {
            return Err(safety_refusal(format!(
                "source project file changed while being fingerprinted: {}",
                path.display()
            )));
        }
        digest.update(&bytes);
        digest.update([0]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("sha256:{:x}", digest.finalize())
}

fn proof_record(
    selection: &FragmentSelection,
    name: &str,
    date: &str,
    desktop_version: &str,
    command_line: &str,
    source_path: &Path,
    source_fingerprint: &str,
) -> Value {
    json!({
        "schema": DESKTOP_PROOF_SCHEMA,
        "fixture": format!("desktop-authored-visuals/{name}"),
        "date": date,
        "desktopVersion": (desktop_version != "unknown").then_some(desktop_version),
        "commands": [command_line],
        "signals": {
            "featureIds": ["desktop.reference-harvest"],
            "schemaGolden": true,
            "desktopReferencePresent": true,
            "notes": [
                format!("Archived {} fragment for handle {}.", selection.kind.as_str(), selection.handle),
                "No Desktop execution evidence was supplied; compatibility remains desktop-golden-pending."
            ],
            "evidence": {
                "sourcePath": canonical_display(source_path),
                "sourceFingerprint": source_fingerprint
            }
        },
        "proofLevel": ProofLevel::DesktopGoldenPending.as_str()
    })
}

fn utc_date() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        / 86_400;
    // Howard Hinnant's civil_from_days conversion, with the Unix epoch offset.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_part = (5 * doy + 2) / 153;
    let day = doy - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn safety_refusal(detail: impl Into<String>) -> CliError {
    CliError::new(
        INPUT_SAFETY_ERROR_CODE,
        EXIT_VALIDATION_FAILED,
        format!("harvest-reference input refused: {}", detail.into()),
    )
    .with_hint("Inspect the bounded input-safety contract; persisted selection/filter values are refused and never silently stripped.")
    .with_suggested_command("powerbi-cli --json capabilities")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsoft_reference_fixtures_are_safe_except_persisted_slicer_state() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in ["pieChart", "donutChart", "matrix"] {
            let path = root
                .join("docs/reference/desktop-authored-visuals")
                .join(format!("{name}.visual.json"));
            let value = read_safe_fragment(&path).expect("reference fragment is safe");
            assert!(value["visual"]["visualType"].is_string());
        }
        let slicer = root.join("docs/reference/desktop-authored-visuals/slicer.visual.json");
        let error = read_safe_fragment(&slicer).expect_err("persisted slicer state");
        assert_eq!(error.code, INPUT_SAFETY_ERROR_CODE);
        assert!(error.message.contains("persisted data values"));
    }

    #[test]
    fn synthetic_fragment_is_read_without_stripping_and_tree_fingerprint_is_stable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fragment = temp.path().join("visual.json");
        fs::write(&fragment, r#"{"visual":{"visualType":"pieChart"}}"#).expect("fragment");
        let value = read_harvested_fragment(&fragment).expect("synthetic fragment");
        assert_eq!(value["visual"]["visualType"], "pieChart");
        let first = project_fingerprint(temp.path()).expect("first fingerprint");
        let second = project_fingerprint(temp.path()).expect("second fingerprint");
        assert_eq!(first, second);
        assert!(first.starts_with("sha256:"));
    }

    #[test]
    fn utc_date_is_iso_calendar_date() {
        let date = utc_date();
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }
}
