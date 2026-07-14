use crate::project_io::copy_project_dir;
use crate::safety_scan::{contains_credential_like_text_str, contains_pii_suspect_text};
use crate::tmdl::load_table_documents;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::read::ZipArchive;
use zip::write::SimpleFileOptions;

const DEFAULT_MAX_ARCHIVE_ENTRIES: usize = 10_000;
const DEFAULT_MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_MAX_COMPRESSION_RATIO: u64 = 200;

#[derive(Debug)]
struct PackageOptions {
    package: Option<PathBuf>,
    project: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    out_file: Option<PathBuf>,
    source_root: Option<String>,
    include_unsafe: bool,
    include_unknown: bool,
    force: bool,
    dry_run: bool,
    max_entries: usize,
    max_entry_bytes: u64,
    max_total_bytes: u64,
    max_compression_ratio: u64,
}

impl Default for PackageOptions {
    fn default() -> Self {
        Self {
            package: None,
            project: None,
            out_dir: None,
            out_file: None,
            source_root: None,
            include_unsafe: false,
            include_unknown: false,
            force: false,
            dry_run: false,
            max_entries: DEFAULT_MAX_ARCHIVE_ENTRIES,
            max_entry_bytes: DEFAULT_MAX_ENTRY_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_compression_ratio: DEFAULT_MAX_COMPRESSION_RATIO,
        }
    }
}

#[derive(Debug, Clone)]
struct PackageEntry {
    index: usize,
    name: String,
    size: u64,
    compressed_size: u64,
    is_dir: bool,
    encrypted: bool,
    category: EntryCategory,
    safe_for_metadata_extract: bool,
}

#[derive(Debug)]
struct ExtractionResult {
    extracted: Vec<Value>,
    skipped: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EntryCategory {
    Pbip,
    Pbir,
    Tmdl,
    Theme,
    MetadataJson,
    UnsafeDataModel,
    UnsafeCache,
    UnsafeBinary,
    Unknown,
}

pub(crate) fn package_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "package requires a subcommand: inspect, extract, import, source-pack, or export-plan",
        )
        .with_hint("Use package commands for archive metadata doors; PBIX/PBIT opaque Desktop binary export is not guessed.")
        .with_suggested_command("powerbi-cli package inspect <file.pbix|file.pbit|file.zip> --json"));
    };

    match action.as_str() {
        "inspect" | "info" => inspect_package(rest),
        "extract" | "unpack" => extract_package(rest, false),
        "import" => extract_package(rest, true),
        "source-pack" | "source-package" | "source-zip" => source_pack(rest),
        "export-plan" | "pbit-plan" | "template-plan" => export_plan(rest),
        "export" | "compile" | "pack" => Err(CliError::unsupported_feature(
            "PBIX/PBIT binary export is not implemented because Microsoft documents Desktop export, not a public PBIP-to-PBIT writer format.",
        )
        .with_hint("Use `package source-pack` for deterministic source archives, `package export-plan` for the exact Desktop/manual export workflow, or keep PBIP as the source-control artifact.")
        .with_suggested_command("powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --json")),
        other => Err(CliError::invalid_args(format!(
            "unknown package command: {other}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for package` for supported package doors.")
        .with_suggested_command("powerbi-cli --json capabilities --for package")),
    }
}

fn inspect_package(args: &[String]) -> CliResult<Value> {
    let options = parse_package_args("package inspect", args)?;
    let package = required_package(options.package, "package inspect")?;
    let entries = package_entries(&package, None)?;
    let summary = package_summary(&package, &entries);
    let source_roots = package_source_roots(&entries);
    Ok(json!({
        "schema": "powerbi-cli.package.inspect.v1",
        "ok": true,
        "package": canonical_display(&package),
        "packageKind": package_kind(&package),
        "packageClass": package_class(&entries),
        "archive": summary,
        "sourceRoots": source_roots,
        "entries": entries.iter().map(entry_json).collect::<Vec<_>>(),
        "support": {
            "canExtractMetadata": entries.iter().any(|entry| entry.safe_for_metadata_extract),
            "canExtractSafeMetadata": entries.iter().any(|entry| entry.safe_for_metadata_extract),
            "canImportPbipSource": package_has_pbip_source(&entries),
            "canImportSourceProject": package_has_pbip_source(&entries),
            "canExportPbixOrPbit": false,
            "canWriteBinaryPackage": false,
            "noFakeFallbacks": true,
            "exportBoundary": "Use Power BI Desktop to export PBIX/PBIT. This CLI does not synthesize opaque Desktop package binaries."
        },
        "next": [
            format!("powerbi-cli package extract {} --out-dir <empty-dir> --json", command_arg(&package)),
            format!("powerbi-cli package import {} --out-dir <empty-dir> --json", command_arg(&package)),
            "powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --json".to_string(),
            "powerbi-cli package export-plan --project <project-dir-or.pbip> --kind pbit --json".to_string()
        ]
    }))
}

fn extract_package(args: &[String], require_importable: bool) -> CliResult<Value> {
    let options = parse_package_args(
        if require_importable {
            "package import"
        } else {
            "package extract"
        },
        args,
    )?;
    let package = required_package(options.package.clone(), "package extract")?;
    let out_dir = options.out_dir.as_ref().ok_or_else(|| {
        CliError::invalid_args("package extract/import requires --out-dir <empty-dir>")
            .with_hint("Choose an empty output directory; package extraction refuses to merge into existing files.")
            .with_suggested_command("powerbi-cli package extract <file.pbix|file.pbit|file.zip> --out-dir <empty-dir> --json")
    })?;
    reject_nonempty_out_dir(out_dir)?;

    let entries = package_entries(&package, Some(options.max_entries))?;
    if require_importable && !package_has_pbip_source(&entries) {
        return Err(CliError::unsupported_feature(
            "package import requires PBIP-style source entries inside the archive",
        )
        .with_hint("This archive can be inspected, but no .pbip plus Report/SemanticModel source tree was found to import.")
        .with_suggested_command(format!(
            "powerbi-cli package inspect {} --json",
            command_arg(&package)
        )));
    }
    let source_root =
        selected_source_root(&entries, options.source_root.as_deref(), require_importable)?;

    let out_dir_existed = out_dir.exists();
    fs::create_dir_all(out_dir)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", out_dir.display())))?;
    let extraction = extract_selected_entries(
        &package,
        out_dir,
        &entries,
        source_root.as_deref(),
        require_importable,
        &options,
    );
    let ExtractionResult { extracted, skipped } = match extraction {
        Ok(result) => result,
        Err(err) => {
            cleanup_partial_extraction(out_dir, out_dir_existed).map_err(|cleanup_err| {
                CliError::unexpected(format!(
                    "{}; additionally failed to clean partial extraction {}: {}",
                    err.message,
                    out_dir.display(),
                    cleanup_err.message
                ))
            })?;
            return Err(err);
        }
    };

    let validation = if require_importable {
        import_validation(out_dir)?
    } else {
        None
    };
    let validation_ok = validation
        .as_ref()
        .map(|report| report["ok"].as_bool().unwrap_or(false))
        .unwrap_or(true);
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let action = if require_importable {
        "import"
    } else {
        "extract"
    };
    Ok(json!({
        "schema": if require_importable { "powerbi-cli.package.import.v1" } else { "powerbi-cli.package.extract.v1" },
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": action,
        "package": canonical_display(&package),
        "packageKind": package_kind(&package),
        "packageClass": package_class(&entries),
        "sourceRoot": source_root,
        "outDir": canonical_display(out_dir),
        "policy": {
            "metadataOnlyByDefault": true,
            "includeUnsafe": options.include_unsafe,
            "includeUnknown": options.include_unknown,
            "noFakeFallbacks": true,
            "limits": {
                "maxEntries": options.max_entries,
                "maxEntryBytes": options.max_entry_bytes,
                "maxTotalBytes": options.max_total_bytes,
                "maxCompressionRatio": options.max_compression_ratio
            }
        },
        "counts": {
            "archiveEntries": entries.len(),
            "extracted": extracted.len(),
            "skipped": skipped.len()
        },
        "extracted": extracted,
        "skipped": skipped,
        "validation": validation,
        "next": [
            format!("powerbi-cli inspect --deep {} --json", command_arg(out_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(out_dir)),
            format!("powerbi-cli handoff check {} --json", command_arg(out_dir))
        ]
    }))
}

fn enforce_archive_entry_limit(entry_count: usize, max_entries: usize) -> CliResult<()> {
    if entry_count <= max_entries {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "archive contains {} entries, exceeding the extraction limit of {max_entries}",
        entry_count
    ))
    .with_hint("Inspect the archive before raising --max-entries; unusually large entry counts can indicate an archive bomb."))
}

fn extract_selected_entries(
    package: &Path,
    out_dir: &Path,
    entries: &[PackageEntry],
    source_root: Option<&str>,
    require_importable: bool,
    options: &PackageOptions,
) -> CliResult<ExtractionResult> {
    let mut archive = open_archive(package)?;
    let mut extracted = Vec::new();
    let mut skipped = Vec::new();
    let mut total_written = 0u64;
    for entry in entries {
        let inside_source_root = source_relative_entry(&entry.name, source_root).is_some();
        let should_extract = if require_importable {
            inside_source_root && entry_safe_for_source_import(entry, source_root)
        } else {
            entry.safe_for_metadata_extract
                || (options.include_unsafe && entry.category.is_unsafe())
                || (options.include_unknown && entry.category == EntryCategory::Unknown)
        };
        if !should_extract {
            skipped.push(entry_json(entry));
            continue;
        }
        if entry.encrypted {
            skipped.push(skipped_entry(entry, "encrypted-entry"));
            continue;
        }
        if entry.is_dir {
            continue;
        }
        let mut file = archive.by_index(entry.index).map_err(zip_error)?;
        let Some(enclosed) = file.enclosed_name() else {
            skipped.push(skipped_entry(entry, "unsafe-path"));
            continue;
        };
        let target_relative = if require_importable {
            source_relative_entry(&entry.name, source_root)
                .unwrap_or_else(|| enclosed.to_path_buf())
        } else {
            enclosed
        };
        let target = out_dir.join(target_relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                CliError::unexpected(format!("create {}: {err}", parent.display()))
            })?;
        }
        let mut output = File::create(&target)
            .map_err(|err| CliError::unexpected(format!("create {}: {err}", target.display())))?;
        let written =
            copy_zip_entry_with_limits(&mut file, &mut output, entry, &mut total_written, options)?;
        extracted.push(json!({
            "name": entry.name,
            "path": canonical_display(&target),
            "category": entry.category.as_str(),
            "size": written
        }));
    }
    Ok(ExtractionResult { extracted, skipped })
}

fn copy_zip_entry_with_limits(
    input: &mut impl Read,
    output: &mut impl Write,
    entry: &PackageEntry,
    total_written: &mut u64,
    options: &PackageOptions,
) -> CliResult<u64> {
    let mut buffer = [0u8; 64 * 1024];
    let mut entry_written = 0u64;
    loop {
        let count = input.read(&mut buffer).map_err(|err| {
            CliError::validation_failed(format!("read archive entry {}: {err}", entry.name))
        })?;
        if count == 0 {
            break;
        }
        entry_written = entry_written.checked_add(count as u64).ok_or_else(|| {
            CliError::validation_failed(format!(
                "archive entry size overflow while extracting {}",
                entry.name
            ))
        })?;
        let next_total = total_written.checked_add(count as u64).ok_or_else(|| {
            CliError::validation_failed("archive total size overflow during extraction")
        })?;
        if entry_written > options.max_entry_bytes {
            return Err(CliError::validation_failed(format!(
                "archive entry {} exceeds the per-entry extraction limit of {} bytes",
                entry.name, options.max_entry_bytes
            ))
            .with_hint("Inspect the entry before raising --max-entry-bytes."));
        }
        if next_total > options.max_total_bytes {
            return Err(CliError::validation_failed(format!(
                "archive extraction exceeds the total uncompressed limit of {} bytes while reading {}",
                options.max_total_bytes, entry.name
            ))
            .with_hint("Inspect the archive before raising --max-total-bytes."));
        }
        if compression_ratio_exceeded(
            entry_written,
            entry.compressed_size,
            options.max_compression_ratio,
        ) {
            return Err(CliError::validation_failed(format!(
                "archive entry {} exceeds the compression-ratio limit of {}:1",
                entry.name, options.max_compression_ratio
            ))
            .with_hint("A very high expansion ratio can indicate a ZIP bomb; inspect the archive before raising --max-compression-ratio."));
        }
        output.write_all(&buffer[..count]).map_err(|err| {
            CliError::unexpected(format!("write extracted entry {}: {err}", entry.name))
        })?;
        *total_written = next_total;
    }
    Ok(entry_written)
}

fn compression_ratio_exceeded(uncompressed: u64, compressed: u64, max_ratio: u64) -> bool {
    uncompressed > 0 && (compressed == 0 || uncompressed > compressed.saturating_mul(max_ratio))
}

fn cleanup_partial_extraction(out_dir: &Path, preserve_empty_directory: bool) -> CliResult<()> {
    if out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|err| {
            CliError::unexpected(format!(
                "remove partial extraction {}: {err}",
                out_dir.display()
            ))
        })?;
    }
    if preserve_empty_directory {
        fs::create_dir_all(out_dir).map_err(|err| {
            CliError::unexpected(format!(
                "restore empty extraction directory {}: {err}",
                out_dir.display()
            ))
        })?;
    }
    Ok(())
}

fn source_pack(args: &[String]) -> CliResult<Value> {
    let options = parse_source_pack_args(args)?;
    let project = options.project.ok_or_else(|| {
        CliError::invalid_args("package source-pack requires --project <project-dir-or.pbip>")
            .with_hint("Pass a PBIP source project. The command writes a deterministic source archive, not an opaque Desktop data package.")
            .with_suggested_command("powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --json")
    })?;
    let out_file = options.out_file.as_ref().ok_or_else(|| {
        CliError::invalid_args("package source-pack requires --out <archive.pbit|archive.pbix|archive.zip>")
            .with_hint("Choose the handoff archive path explicitly; existing files are refused unless --force is set.")
            .with_suggested_command("powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --json")
    })?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    if !validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "project is not valid for source packaging: {}",
            validation.errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli validate --strict {} --json",
            command_arg(&resolved.project_dir)
        )));
    }
    reject_source_pack_output(&resolved.project_dir, out_file, options.force)?;
    let files = source_project_files(&resolved.project_dir)?;
    let mut unapproved_files = Vec::new();
    for file in &files {
        let relative = project_relative_name(&resolved.project_dir, file)?;
        if source_pack_project_file_category(&resolved, &relative).is_none() {
            unapproved_files.push(relative);
        }
    }
    unapproved_files.sort();
    if !unapproved_files.is_empty() {
        return Err(CliError::validation_failed(format!(
            "project contains unapproved source-package files: {}",
            unapproved_files.join(", ")
        ))
        .with_hint("Source packages allow only PBIP/PBIR/TMDL project files and the documented generated sidecars; remove unknown files and every dot-directory before handoff.")
        .with_suggested_command(format!(
            "powerbi-cli handoff check {} --json",
            command_arg(&resolved.project_dir)
        )));
    }

    let mut archive_entries = files
        .iter()
        .map(|file| {
            Ok((
                project_relative_name(&resolved.project_dir, file)?,
                file.to_path_buf(),
            ))
        })
        .collect::<CliResult<Vec<_>>>()?;
    archive_entries.sort_by(|a, b| a.0.cmp(&b.0));
    scan_source_archive_content(&resolved, &archive_entries)?;

    if !options.dry_run {
        if let Some(parent) = out_file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|err| {
                CliError::unexpected(format!("create {}: {err}", parent.display()))
            })?;
        }
        write_source_archive(out_file, &archive_entries)?;
    }

    Ok(json!({
        "schema": "powerbi-cli.package.sourcePack.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "changed": !options.dry_run,
        "dryRun": options.dry_run,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "package": canonical_display(out_file),
        "packageKind": package_kind(out_file),
        "packageClass": "source-package",
        "canWriteBinaryPackage": false,
        "desktopBinaryCompatible": false,
        "noFakeFallbacks": true,
        "counts": {
            "entries": archive_entries.len(),
            "unapprovedRejected": unapproved_files.len(),
            "contentScanFailures": 0
        },
        "entries": archive_entries.iter().map(|(name, path)| json!({
            "name": name,
            "path": canonical_display(path),
            "category": source_pack_project_file_category(&resolved, name).map(EntryCategory::as_str).unwrap_or("unknown")
        })).collect::<Vec<_>>(),
        "validation": {
            "ok": true,
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli package inspect {} --json", command_arg(out_file)),
            format!("powerbi-cli package import {} --out-dir <empty-dir> --json", command_arg(out_file)),
            format!("powerbi-cli desktop open-check {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn export_plan(args: &[String]) -> CliResult<Value> {
    let options = parse_export_plan_args(args)?;
    let project = options.project.ok_or_else(|| {
        CliError::invalid_args("package export-plan requires --project <project-dir-or.pbip>")
            .with_hint("Pass the source PBIP project; the command returns an honest Desktop export plan.")
            .with_suggested_command("powerbi-cli package export-plan --project <project-dir-or.pbip> --kind pbit --json")
    })?;
    let kind = options
        .package
        .as_ref()
        .and_then(|path| path.to_str())
        .unwrap_or("pbit")
        .to_ascii_lowercase();
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let project_arg = command_arg(&resolved.project_dir);
    Ok(json!({
        "schema": "powerbi-cli.package.exportPlan.v1",
        "ok": validation.errors.is_empty(),
        "exitCode": if validation.errors.is_empty() { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "requestedKind": kind,
        "canWriteBinaryPackage": false,
        "noFakeFallbacks": true,
        "reason": "Power BI Desktop exposes PBIX/PBIT export. The public PBIP/PBIR/TMDL file docs do not define a complete PBIP-to-PBIT/PBIX writer contract for this CLI to synthesize.",
        "desktopWorkflow": [
            format!("Open {}", canonical_display(&resolved.pbip_path)),
            "In Power BI Desktop, use File > Export > Power BI template for .pbit or File > Save As for .pbix.",
            "Bring the exported file back only after running handoff checks and removing caches/credentials where required."
        ],
        "safeAlternatives": [
            "Keep the PBIP folder as the source-control and agent-authoring artifact.",
            "Use `package inspect` and `package import` for PBIX/PBIT archives that already contain PBIP/PBIR/TMDL source entries."
        ],
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli validate --strict {project_arg} --json"),
            format!("powerbi-cli handoff check {project_arg} --json"),
            format!("powerbi-cli desktop open-check {project_arg} --json")
        ]
    }))
}

fn package_entries(path: &Path, max_entries: Option<usize>) -> CliResult<Vec<PackageEntry>> {
    let mut archive = open_archive(path)?;
    if let Some(max_entries) = max_entries {
        enforce_archive_entry_limit(archive.len(), max_entries)?;
    }
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(zip_error)?;
        let name = file.name().replace('\\', "/");
        let category = classify_entry(&name);
        entries.push(PackageEntry {
            index,
            name,
            size: file.size(),
            compressed_size: file.compressed_size(),
            is_dir: file.is_dir(),
            encrypted: file.encrypted(),
            safe_for_metadata_extract: category.safe_for_metadata_extract(),
            category,
        });
    }
    Ok(entries)
}

fn open_archive(path: &Path) -> CliResult<ZipArchive<File>> {
    let file = File::open(path).map_err(|err| {
        CliError::file_not_found(format!("open package {}: {err}", path.display()))
    })?;
    ZipArchive::new(file).map_err(zip_error)
}

fn zip_error(err: zip::result::ZipError) -> CliError {
    CliError::validation_failed(format!("package is not a readable ZIP archive: {err}"))
        .with_hint("PBIX/PBIT inspection currently supports ZIP-readable packages with visible metadata entries.")
        .with_suggested_command("powerbi-cli package inspect <file.pbix|file.pbit|file.zip> --json")
}

fn classify_entry(name: &str) -> EntryCategory {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pbix") || lower.ends_with(".pbit") {
        return EntryCategory::UnsafeBinary;
    }
    if lower.contains("datamodel") {
        return EntryCategory::UnsafeDataModel;
    }
    if lower.contains(".pbi/cache")
        || lower.ends_with("cache.abf")
        || lower.ends_with("localsettings.json")
    {
        return EntryCategory::UnsafeCache;
    }
    if lower.ends_with(".pbip") {
        return EntryCategory::Pbip;
    }
    if lower.ends_with(".pbism") || lower.ends_with("/.platform") || lower.ends_with(".platform") {
        return EntryCategory::MetadataJson;
    }
    if lower.ends_with(".tmdl") || lower.contains(".semanticmodel/definition/") {
        return EntryCategory::Tmdl;
    }
    if lower.contains(".report/definition/")
        || lower.ends_with(".pbir")
        || lower.ends_with("visual.json")
        || lower.ends_with("pages.json")
        || lower.ends_with("bookmarks.json")
        || lower.ends_with(".bookmark.json")
    {
        return EntryCategory::Pbir;
    }
    if lower.contains(".report/staticresources/registeredresources/")
        || lower.ends_with(".json")
            && (lower.contains("theme") || lower.contains("registeredresources"))
    {
        return EntryCategory::Theme;
    }
    if lower.ends_with(".json") {
        return EntryCategory::MetadataJson;
    }
    EntryCategory::Unknown
}

impl EntryCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pbip => "pbip",
            Self::Pbir => "pbir",
            Self::Tmdl => "tmdl",
            Self::Theme => "theme",
            Self::MetadataJson => "metadata-json",
            Self::UnsafeDataModel => "unsafe-data-model",
            Self::UnsafeCache => "unsafe-cache",
            Self::UnsafeBinary => "unsafe-binary",
            Self::Unknown => "unknown",
        }
    }

    fn safe_for_metadata_extract(self) -> bool {
        matches!(
            self,
            Self::Pbip | Self::Pbir | Self::Tmdl | Self::Theme | Self::MetadataJson
        )
    }

    fn is_unsafe(self) -> bool {
        matches!(
            self,
            Self::UnsafeDataModel | Self::UnsafeCache | Self::UnsafeBinary
        )
    }
}

fn entry_json(entry: &PackageEntry) -> Value {
    json!({
        "index": entry.index,
        "name": entry.name,
        "category": entry.category.as_str(),
        "size": entry.size,
        "compressedSize": entry.compressed_size,
        "isDirectory": entry.is_dir,
        "encrypted": entry.encrypted,
        "safeForMetadataExtract": entry.safe_for_metadata_extract
    })
}

fn skipped_entry(entry: &PackageEntry, reason: &str) -> Value {
    let mut value = entry_json(entry);
    value["skipReason"] = Value::String(reason.to_string());
    value
}

fn package_summary(path: &Path, entries: &[PackageEntry]) -> Value {
    let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total_size = 0u64;
    let mut total_compressed = 0u64;
    for entry in entries {
        *by_category.entry(entry.category.as_str()).or_default() += 1;
        total_size = total_size.saturating_add(entry.size);
        total_compressed = total_compressed.saturating_add(entry.compressed_size);
    }
    json!({
        "kind": package_kind(path),
        "entries": entries.len(),
        "totalUncompressedBytes": total_size,
        "totalCompressedBytes": total_compressed,
        "byCategory": by_category,
        "unsafeEntries": entries.iter().filter(|entry| entry.category.is_unsafe()).count(),
        "metadataEntries": entries.iter().filter(|entry| entry.safe_for_metadata_extract).count(),
        "encryptedEntries": entries.iter().filter(|entry| entry.encrypted).count(),
        "hasPbip": entries.iter().any(|entry| entry.category == EntryCategory::Pbip),
        "hasPbir": entries.iter().any(|entry| entry.category == EntryCategory::Pbir),
        "hasTmdl": entries.iter().any(|entry| entry.category == EntryCategory::Tmdl),
        "hasUnsafeDataModel": entries.iter().any(|entry| entry.category == EntryCategory::UnsafeDataModel)
    })
}

fn package_kind(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("pbix") => "pbix",
        Some("pbit") => "pbit",
        Some("zip") => "zip",
        _ => "unknown",
    }
}

fn package_has_pbip_source(entries: &[PackageEntry]) -> bool {
    source_root_has_pbip_source(entries, None)
        || package_source_roots(entries)
            .iter()
            .any(|root| source_root_has_pbip_source(entries, Some(root)))
}

fn package_class(entries: &[PackageEntry]) -> &'static str {
    if package_has_pbip_source(entries) && !entries.iter().any(|entry| entry.category.is_unsafe()) {
        "source-package"
    } else if package_has_pbip_source(entries) {
        "source-bearing-desktop-package"
    } else if entries
        .iter()
        .any(|entry| entry.category == EntryCategory::UnsafeDataModel)
    {
        "opaque-desktop-package"
    } else {
        "metadata-archive"
    }
}

fn package_source_roots(entries: &[PackageEntry]) -> Vec<String> {
    let mut roots = BTreeSet::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.category == EntryCategory::Pbip)
    {
        let normalized = entry.name.replace('\\', "/");
        if let Some((root, _)) = normalized.rsplit_once('/')
            && !root.is_empty()
        {
            roots.insert(root.to_string());
        }
    }
    roots.into_iter().collect()
}

fn selected_source_root(
    entries: &[PackageEntry],
    requested: Option<&str>,
    require_importable: bool,
) -> CliResult<Option<String>> {
    if !require_importable {
        return Ok(None);
    }
    if let Some(requested) = requested {
        let normalized = normalize_source_root(requested)?;
        if source_root_has_pbip_source(entries, Some(&normalized)) {
            return Ok(Some(normalized));
        }
        return Err(CliError::invalid_args(format!(
            "requested source root was not found or is incomplete: {requested}"
        ))
        .with_hint("Use `package inspect` to list sourceRoots, then pass one with `--source-root`.")
        .with_suggested_command(
            "powerbi-cli package inspect <file.pbix|file.pbit|file.zip> --json",
        ));
    }
    let roots = package_source_roots(entries);
    if roots.len() > 1 {
        return Err(CliError::invalid_args(format!(
            "package contains multiple PBIP source roots: {}",
            roots.join(", ")
        ))
        .with_hint("Pass --source-root <root> so import strips the intended wrapper directory.")
        .with_suggested_command("powerbi-cli package import <file.pbix|file.pbit|file.zip> --source-root <root> --out-dir <empty-dir> --json"));
    }
    if let Some(root) = roots.first()
        && source_root_has_pbip_source(entries, Some(root))
    {
        return Ok(Some(root.clone()));
    }
    Ok(None)
}

fn normalize_source_root(value: &str) -> CliResult<String> {
    let normalized = value
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    if normalized.is_empty()
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(
            CliError::invalid_args(format!("invalid source root: {value}")).with_hint(
                "Source roots must be relative archive folders without . or .. components.",
            ),
        );
    }
    Ok(normalized)
}

fn source_root_has_pbip_source(entries: &[PackageEntry], root: Option<&str>) -> bool {
    let mut has_pbip = false;
    let mut has_report = false;
    let mut has_model = false;
    for entry in entries {
        let Some(relative) = source_relative_entry(&entry.name, root) else {
            continue;
        };
        let relative_name = relative.to_string_lossy().replace('\\', "/");
        let lower = relative_name.to_ascii_lowercase();
        if lower.ends_with(".pbip") && (root.is_some() || !lower.contains('/')) {
            has_pbip = true;
        }
        if lower.contains(".report/") {
            has_report = true;
        }
        if lower.contains(".semanticmodel/") {
            has_model = true;
        }
    }
    has_pbip && has_report && has_model
}

fn source_relative_entry(name: &str, source_root: Option<&str>) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|part| part == "." || part == ".." || part.is_empty())
    {
        return None;
    }
    let relative = if let Some(root) = source_root {
        let prefix = format!("{}/", root.trim_matches('/'));
        normalized.strip_prefix(&prefix)?.to_string()
    } else {
        normalized
    };
    if relative.is_empty() {
        return None;
    }
    Some(PathBuf::from(relative))
}

fn entry_safe_for_source_import(entry: &PackageEntry, source_root: Option<&str>) -> bool {
    source_relative_entry(&entry.name, source_root)
        .and_then(|relative| {
            source_pack_file_category(&relative.to_string_lossy().replace('\\', "/"))
        })
        .is_some()
}

fn source_project_files(project_dir: &Path) -> CliResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(project_dir).into_iter() {
        let entry =
            entry.map_err(|err| CliError::unexpected(format!("walk project files: {err}")))?;
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    Ok(files)
}

fn project_relative_name(project_dir: &Path, path: &Path) -> CliResult<String> {
    let relative = path.strip_prefix(project_dir).map_err(|err| {
        CliError::unexpected(format!(
            "make {} relative to {}: {err}",
            path.display(),
            project_dir.display()
        ))
    })?;
    let name = relative.to_string_lossy().replace('\\', "/");
    if name.is_empty()
        || name.starts_with('/')
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(CliError::validation_failed(format!(
            "unsafe project-relative path: {}",
            path.display()
        )));
    }
    Ok(name)
}

fn source_pack_file_category(relative_name: &str) -> Option<EntryCategory> {
    let normalized = relative_name.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    if normalized.eq_ignore_ascii_case(".powerbi-cli/source-templates.json") {
        return Some(EntryCategory::MetadataJson);
    }
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .take(parts.len().saturating_sub(1))
        .any(|part| part.starts_with('.'))
    {
        return None;
    }
    let lower = parts
        .iter()
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if parts.len() == 1 {
        return match lower[0].as_str() {
            name if name.ends_with(".pbip") => Some(EntryCategory::Pbip),
            ".gitignore" | "powerbi_handoff.md" => Some(EntryCategory::Unknown),
            "powerbi-cli.manifest.copy.json" => Some(EntryCategory::MetadataJson),
            _ => None,
        };
    }

    let root = &lower[0];
    if root.ends_with(".report") {
        if parts.len() == 2 && lower[1] == ".platform" {
            return Some(EntryCategory::MetadataJson);
        }
        if parts.len() == 2 && lower[1] == "definition.pbir" {
            return Some(EntryCategory::Pbir);
        }
        if report_definition_json_is_approved(&lower) {
            return Some(EntryCategory::Pbir);
        }
        if parts.len() >= 4
            && lower[1] == "staticresources"
            && matches!(lower[2].as_str(), "registeredresources" | "sharedresources")
            && lower.last().is_some_and(|name| name.ends_with(".json"))
        {
            return Some(EntryCategory::Theme);
        }
        return None;
    }
    if root.ends_with(".semanticmodel") {
        if parts.len() == 2 && lower[1] == ".platform" {
            return Some(EntryCategory::MetadataJson);
        }
        if parts.len() == 2 && lower[1] == "definition.pbism" {
            return Some(EntryCategory::MetadataJson);
        }
        if lower[1] == "definition"
            && parts.len() >= 3
            && lower.last().is_some_and(|name| name.ends_with(".tmdl"))
        {
            return Some(EntryCategory::Tmdl);
        }
    }
    None
}

fn source_pack_project_file_category(
    resolved: &crate::ResolvedProject,
    relative_name: &str,
) -> Option<EntryCategory> {
    let category = source_pack_file_category(relative_name)?;
    let normalized = relative_name.replace('\\', "/");
    if normalized.eq_ignore_ascii_case(".powerbi-cli/source-templates.json") {
        return Some(category);
    }
    let root = normalized.split('/').next()?;
    match category {
        EntryCategory::Pbip => resolved
            .pbip_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(root))
            .then_some(category),
        EntryCategory::Pbir | EntryCategory::Theme => resolved
            .report_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(root))
            .then_some(category),
        EntryCategory::Tmdl => resolved
            .semantic_model_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(root))
            .then_some(category),
        EntryCategory::MetadataJson if normalized.contains('/') => {
            let report_matches = resolved
                .report_dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(root));
            let model_matches = resolved
                .semantic_model_dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(root));
            (report_matches || model_matches).then_some(category)
        }
        EntryCategory::MetadataJson | EntryCategory::Unknown => Some(category),
        EntryCategory::UnsafeDataModel
        | EntryCategory::UnsafeCache
        | EntryCategory::UnsafeBinary => None,
    }
}

fn report_definition_json_is_approved(parts: &[String]) -> bool {
    match parts {
        [_, definition, file]
            if definition == "definition"
                && matches!(file.as_str(), "report.json" | "version.json") =>
        {
            true
        }
        [_, definition, pages, file]
            if definition == "definition" && pages == "pages" && file == "pages.json" =>
        {
            true
        }
        [_, definition, pages, _, file]
            if definition == "definition" && pages == "pages" && file == "page.json" =>
        {
            true
        }
        [_, definition, pages, _, visuals, _, file]
            if definition == "definition"
                && pages == "pages"
                && visuals == "visuals"
                && file == "visual.json" =>
        {
            true
        }
        [_, definition, bookmarks, file]
            if definition == "definition"
                && bookmarks == "bookmarks"
                && (file == "bookmarks.json" || file.ends_with(".bookmark.json")) =>
        {
            true
        }
        _ => false,
    }
}

fn scan_source_archive_content(
    resolved: &crate::ResolvedProject,
    entries: &[(String, PathBuf)],
) -> CliResult<()> {
    let mut credential_files = BTreeSet::new();
    let mut pii_review_files = BTreeSet::new();
    let mut non_dummy_partition_files = BTreeSet::new();
    for (name, path) in entries {
        let bytes = fs::read(path)
            .map_err(|err| CliError::unexpected(format!("scan {}: {err}", path.display())))?;
        let text = String::from_utf8_lossy(&bytes);
        if contains_credential_like_text_str(&text) {
            credential_files.insert(name.clone());
        }
        if contains_pii_suspect_text(&text) {
            pii_review_files.insert(name.clone());
        }
    }
    for doc in load_table_documents(resolved)? {
        let doc_relative = canonical_project_relative_name(&resolved.project_dir, &doc.path)?;
        for partition in doc.partitions {
            if partition
                .safety
                .findings
                .iter()
                .any(|finding| finding.code == "partition.pii_suspect_literal")
            {
                pii_review_files.insert(doc_relative.clone());
            }
            if partition
                .safety
                .findings
                .iter()
                .any(|finding| finding.code == "partition.credential_like_text")
            {
                credential_files.insert(doc_relative.clone());
            }
            if partition.source_kind != "dummyMTable" {
                non_dummy_partition_files.insert(doc_relative.clone());
            }
        }
    }
    if credential_files.is_empty()
        && pii_review_files.is_empty()
        && non_dummy_partition_files.is_empty()
    {
        return Ok(());
    }
    let mut reasons = Vec::new();
    if !credential_files.is_empty() {
        reasons.push(format!(
            "credential-like content in {}",
            credential_files.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !pii_review_files.is_empty() {
        reasons.push(format!(
            "PII-suspect row literals requiring review in {}",
            pii_review_files.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !non_dummy_partition_files.is_empty() {
        reasons.push(format!(
            "non-dummy or unverified partition source in {}",
            non_dummy_partition_files
                .into_iter()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Err(CliError::validation_failed(format!(
        "source package content scan failed: {}",
        reasons.join("; ")
    ))
    .with_hint("Remove credentials and real row data, replace external or unverified partitions with generated dummy tables, then rerun handoff check before creating the archive."))
}

fn canonical_project_relative_name(project_dir: &Path, path: &Path) -> CliResult<String> {
    let project_abs = fs::canonicalize(project_dir).map_err(|err| {
        CliError::unexpected(format!("resolve project {}: {err}", project_dir.display()))
    })?;
    let path_abs = fs::canonicalize(path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?;
    project_relative_name(&project_abs, &path_abs)
}

fn reject_source_pack_output(project_dir: &Path, out_file: &Path, force: bool) -> CliResult<()> {
    if out_file.exists() && !force {
        return Err(CliError::invalid_args(format!(
            "source package output already exists: {}",
            out_file.display()
        ))
        .with_hint("Pass --force after reviewing the existing archive, or choose a new --out path.")
        .with_suggested_command("powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --force --json"));
    }
    let project_abs = project_dir.canonicalize().map_err(|err| {
        CliError::unexpected(format!("canonicalize {}: {err}", project_dir.display()))
    })?;
    let out_abs = if out_file.is_absolute() {
        out_file.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| CliError::unexpected(format!("read current directory: {err}")))?
            .join(out_file)
    };
    let parent_abs = out_abs
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .unwrap_or_else(|| out_abs.parent().unwrap_or(Path::new(".")).to_path_buf());
    let comparable_out = out_abs
        .file_name()
        .map(|name| parent_abs.join(name))
        .unwrap_or(out_abs);
    if comparable_out.starts_with(&project_abs) {
        return Err(CliError::invalid_args(format!(
            "source package output must not be written inside the project: {}",
            comparable_out.display()
        ))
        .with_hint("Writing the archive into the project would package the package on later runs.")
        .with_suggested_command("powerbi-cli package source-pack --project <project-dir-or.pbip> --out <outside-project>/report-source.pbit --json"));
    }
    Ok(())
}

fn write_source_archive(path: &Path, entries: &[(String, PathBuf)]) -> CliResult<()> {
    let file = File::create(path)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", path.display())))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, source) in entries {
        zip.start_file(name, options).map_err(zip_error)?;
        let mut input = File::open(source)
            .map_err(|err| CliError::unexpected(format!("open {}: {err}", source.display())))?;
        io::copy(&mut input, &mut zip).map_err(|err| {
            CliError::unexpected(format!(
                "write {} from {} to {}: {err}",
                name,
                source.display(),
                path.display()
            ))
        })?;
    }
    zip.finish().map_err(zip_error)?;
    Ok(())
}

fn import_validation(out_dir: &Path) -> CliResult<Option<Value>> {
    match resolve_project(out_dir) {
        Ok(resolved) => {
            let report = validate_project(&resolved)?;
            Ok(Some(json!({
                "ok": report.errors.is_empty(),
                "projectDir": canonical_display(&resolved.project_dir),
                "pbip": canonical_display(&resolved.pbip_path),
                "warnings": report.warnings,
                "errors": report.errors,
                "counts": {
                    "tables": report.tables,
                    "relationships": report.relationships,
                    "measures": report.measures,
                    "pages": report.pages,
                    "visuals": report.visuals,
                    "boundVisuals": report.bound_visuals
                }
            })))
        }
        Err(err) => Ok(Some(json!({
            "ok": false,
            "error": {
                "code": err.code,
                "message": err.message,
                "hint": err.hint,
                "suggestedCommands": err.suggested_commands
            }
        }))),
    }
}

fn reject_nonempty_out_dir(out_dir: &Path) -> CliResult<()> {
    if out_dir.exists()
        && out_dir
            .read_dir()
            .map(|mut it| it.next().is_some())
            .unwrap_or(true)
    {
        return Err(CliError::invalid_args(format!(
            "output directory is not empty: {}",
            out_dir.display()
        ))
        .with_hint("Choose an empty extraction directory so package import is deterministic.")
        .with_suggested_command(
            "powerbi-cli package extract <file.pbix|file.pbit|file.zip> --out-dir <empty-dir> --json",
        ));
    }
    Ok(())
}

fn parse_package_args(command: &str, args: &[String]) -> CliResult<PackageOptions> {
    let mut options = PackageOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out-dir" | "--out" => {
                options.out_dir = Some(PathBuf::from(take_value(args, &mut i, "--out-dir")?));
            }
            "--include-unsafe" => {
                options.include_unsafe = true;
                i += 1;
            }
            "--include-unknown" => {
                options.include_unknown = true;
                i += 1;
            }
            "--source-root" => {
                options.source_root = Some(take_value(args, &mut i, "--source-root")?);
            }
            "--max-entries" => {
                require_extraction_limit_flag(command, "--max-entries")?;
                let value = take_value(args, &mut i, "--max-entries")?;
                options.max_entries = parse_positive_usize("--max-entries", &value)?;
            }
            "--max-entry-bytes" => {
                require_extraction_limit_flag(command, "--max-entry-bytes")?;
                let value = take_value(args, &mut i, "--max-entry-bytes")?;
                options.max_entry_bytes = parse_positive_u64("--max-entry-bytes", &value)?;
            }
            "--max-total-bytes" => {
                require_extraction_limit_flag(command, "--max-total-bytes")?;
                let value = take_value(args, &mut i, "--max-total-bytes")?;
                options.max_total_bytes = parse_positive_u64("--max-total-bytes", &value)?;
            }
            "--max-compression-ratio" => {
                require_extraction_limit_flag(command, "--max-compression-ratio")?;
                let value = take_value(args, &mut i, "--max-compression-ratio")?;
                options.max_compression_ratio =
                    parse_positive_u64("--max-compression-ratio", &value)?;
            }
            "--metadata-only" => {
                i += 1;
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!("unknown {command} flag: {other}"))
                    .with_hint("Run `powerbi-cli --json capabilities --for package` for supported package flags.")
                    .with_suggested_command("powerbi-cli --json capabilities --for package"));
            }
            other => {
                if options.package.is_some() {
                    return Err(CliError::invalid_args(format!(
                        "{command} accepts exactly one package path"
                    ))
                    .with_hint(format!(
                        "Run `powerbi-cli {command} <file.pbix|file.pbit|file.zip> --json`"
                    ))
                    .with_suggested_command(format!(
                        "powerbi-cli {command} <file.pbix|file.pbit|file.zip> --json"
                    )));
                }
                options.package = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(options)
}

fn require_extraction_limit_flag(command: &str, flag: &str) -> CliResult<()> {
    if command != "package inspect" {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} applies to package extract/import, not package inspect"
    ))
    .with_suggested_command(
        "powerbi-cli package extract <file.pbix|file.pbit|file.zip> --out-dir <empty-dir> --json",
    ))
}

fn parse_positive_usize(flag: &str, value: &str) -> CliResult<usize> {
    let parsed = value.parse::<usize>().map_err(|_| {
        CliError::invalid_args(format!("{flag} requires a positive integer: {value}"))
    })?;
    if parsed == 0 {
        return Err(CliError::invalid_args(format!(
            "{flag} requires a positive integer: {value}"
        )));
    }
    Ok(parsed)
}

fn parse_positive_u64(flag: &str, value: &str) -> CliResult<u64> {
    let parsed = value.parse::<u64>().map_err(|_| {
        CliError::invalid_args(format!("{flag} requires a positive integer: {value}"))
    })?;
    if parsed == 0 {
        return Err(CliError::invalid_args(format!(
            "{flag} requires a positive integer: {value}"
        )));
    }
    Ok(parsed)
}

fn parse_export_plan_args(args: &[String]) -> CliResult<PackageOptions> {
    let mut options = PackageOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--kind" | "--format" => {
                options.package = Some(PathBuf::from(take_value(args, &mut i, "--kind")?));
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown package export-plan flag: {other}"
                ))
                .with_hint("Run `powerbi-cli package export-plan --project <project-dir-or.pbip> --kind pbit --json`.")
                .with_suggested_command(
                    "powerbi-cli package export-plan --project <project-dir-or.pbip> --kind pbit --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_source_pack_args(args: &[String]) -> CliResult<PackageOptions> {
    let mut options = PackageOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--out" | "--out-file" | "-o" => {
                options.out_file = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            "--force" => {
                options.force = true;
                i += 1;
            }
            "--dry-run" => {
                options.dry_run = true;
                i += 1;
            }
            "--kind" | "--format" => {
                let kind = take_value(args, &mut i, "--kind")?;
                if !matches!(kind.as_str(), "pbit" | "pbix" | "zip") {
                    return Err(CliError::invalid_args(format!(
                        "package source-pack kind must be pbit, pbix, or zip: {kind}"
                    ))
                    .with_hint("The kind controls the archive file extension convention only; the output remains a source archive.")
                    .with_suggested_command("powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --json"));
                }
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown package source-pack flag: {other}"
                ))
                .with_hint("Run `powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --json`.")
                .with_suggested_command(
                    "powerbi-cli package source-pack --project <project-dir-or.pbip> --out report-source.pbit --json",
                ));
            }
        }
    }
    Ok(options)
}

fn required_package(package: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    package.ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires <file.pbix|file.pbit|file.zip>"))
            .with_hint("Pass the package file path explicitly.")
            .with_suggested_command(format!(
                "powerbi-cli {command} <file.pbix|file.pbit|file.zip> --json"
            ))
    })
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli --json capabilities --for package` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for package")
    })?;
    *index += 2;
    Ok(value.clone())
}

#[allow(dead_code)]
fn copy_project_for_future_package_export(source: &Path, out_dir: &Path) -> CliResult<()> {
    copy_project_dir(source, out_dir)
}
