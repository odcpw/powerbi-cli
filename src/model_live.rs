use crate::cli_support::take_value;
use crate::desktop_target::resolve_desktop_target;
use crate::live_model::{
    OperationDeadline, desktop_oracle_enabled, resolve_live_model_endpoint,
    revalidate_live_model_endpoint,
};
use crate::mcp::execute_live_tmdl_export;
use crate::microsoft::{MicrosoftComponent, resolve_installed_component};
use crate::workflow::validate_tmdl_definition;
use crate::{
    CliError, CliResult, EXIT_ORACLE_UNAVAILABLE, EXIT_SUCCESS, canonical_display,
    validate_desktop_runtime_project,
};
use file_id::{FileId, get_file_id};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
static EXPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct ExportOptions {
    document: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    allow_model_read: bool,
    timeout_ms: u64,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            document: None,
            out_dir: None,
            allow_model_read: false,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

pub(crate) fn live_model_command(args: &[String]) -> CliResult<Value> {
    match args.split_first() {
        Some((action, rest)) if action == "export-tmdl" => export_tmdl(rest),
        Some((action, _)) => Err(CliError::invalid_args(format!(
            "unknown model live command: {action}"
        ))
        .with_hint("Run the focused capabilities contract before using live Desktop access.")
        .with_suggested_command(
            "powerbi-cli --json capabilities --for \"model live export-tmdl\"",
        )),
        None => Err(CliError::invalid_args(
            "model live requires a subcommand: export-tmdl",
        )
        .with_hint(
            "The command exports only the semantic model from an exact already-open PBIP/PBIX document.",
        )
        .with_suggested_command(
            "powerbi-cli --json capabilities --for \"model live export-tmdl\"",
        )),
    }
}

fn export_tmdl(args: &[String]) -> CliResult<Value> {
    let options = parse_export_args(args)?;
    if !options.allow_model_read {
        return Err(CliError::invalid_args(
            "model live export-tmdl requires --allow-model-read",
        )
        .with_hint(
            "Exported TMDL contains semantic metadata, DAX, Power Query source expressions, and possibly small static table values. Review the destination and opt in explicitly.",
        )
        .with_suggested_command(
            "powerbi-cli model live export-tmdl --document <file.pbix-or.pbip> --out-dir <fresh-dir> --allow-model-read --json",
        ));
    }
    let document = options.document.as_ref().ok_or_else(|| {
        CliError::invalid_args("model live export-tmdl requires --document <pbix-or-pbip>")
    })?;
    let out_dir = options.out_dir.as_ref().ok_or_else(|| {
        CliError::invalid_args("model live export-tmdl requires --out-dir <fresh-dir>")
    })?;
    let target = resolve_desktop_target(document)?;
    target.require_live_model()?;

    let validation = match target.project() {
        Some(project) => {
            let report = validate_desktop_runtime_project(project)?;
            if !report.errors.is_empty() {
                return Err(CliError::validation_failed(
                    "PBIP runtime validation failed before live TMDL export",
                )
                .with_hint(report.errors.join("; ")));
            }
            json!({
                "kind": "pbip-runtime",
                "ok": true,
                "warnings": report.warnings
            })
        }
        None => json!({
            "kind": "pbix-archive",
            "ok": true,
            "archive": target.pbix.as_ref().map(|info| json!({
                "entries": info.entries,
                "hasDataModel": info.has_data_model,
                "hasReportDefinition": info.has_report_definition,
                "hasLegacyReportLayout": info.has_legacy_report_layout
            }))
        }),
    };

    if !cfg!(windows) {
        return Err(CliError::new(
            "unsupported_feature",
            EXIT_ORACLE_UNAVAILABLE,
            format!(
                "model live export-tmdl is Windows-only; current platform is {}",
                std::env::consts::OS
            ),
        ));
    }
    if !desktop_oracle_enabled() {
        return Err(CliError::new(
            "oracle_unavailable",
            EXIT_ORACLE_UNAVAILABLE,
            "set POWERBI_DESKTOP_ORACLE=1 to opt in to exact local Desktop model access",
        ));
    }

    let deadline = OperationDeadline::new(Duration::from_millis(options.timeout_ms));
    let mut export = LiveExportReservation::prepare(out_dir)?;
    let endpoint = resolve_live_model_endpoint(
        &target,
        deadline.remaining("initial Desktop endpoint discovery")?,
    )?;
    let tool = resolve_installed_component(MicrosoftComponent::ModelingMcp)?;
    let mcp_budget = deadline.remaining("Modeling MCP export")?;
    let mcp = execute_live_tmdl_export(
        &tool,
        endpoint.port,
        &export.definition_dir,
        mcp_budget,
        |mcp_remaining| {
            revalidate_live_model_endpoint(
                &target,
                &endpoint,
                deadline
                    .remaining("Desktop endpoint revalidation")?
                    .min(mcp_remaining),
            )
        },
    )?;
    deadline.remaining("TMDL output validation")?;
    let summary = validate_tmdl_definition(&export.definition_dir).map_err(|message| {
        CliError::validation_failed(format!(
            "Microsoft MCP produced an unsafe or invalid TMDL export: {message}"
        ))
    })?;
    deadline.remaining("TMDL publication")?;
    let published = export.publish()?;

    Ok(json!({
        "schema": "powerbi-cli.model.live.export-tmdl.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "document": target.artifact_json(),
        "output": {
            "kind": "tmdl-definition-export",
            "root": canonical_display(&published),
            "definition": canonical_display(&published.join("definition")),
            "fileCount": summary.file_count,
            "totalBytes": summary.total_bytes,
            "sha256": summary.sha256
        },
        "engine": {
            "kind": "power-bi-desktop-local-analysis-services",
            "desktopProcessId": endpoint.desktop_process_id,
            "modelProcessId": endpoint.model_process_id,
            "desktopVersion": endpoint.desktop_version,
            "portReturned": false
        },
        "integration": {
            "component": "modeling-mcp",
            "version": tool.version,
            "server": {
                "name": mcp.handshake.server_name,
                "version": mcp.handshake.server_version,
                "protocolVersion": mcp.handshake.protocol_version,
                "toolsCount": mcp.handshake.tools_count,
                "toolsListSha256": mcp.handshake.tools_list_sha256
            },
            "connectionIdentityReturned": false,
            "notificationsSeen": mcp.notifications_seen,
            "cleanup": {
                "childrenReaped": mcp.cleanup_children_reaped,
                "pumpsJoined": mcp.cleanup_pumps_joined,
                "forced": mcp.cleanup_forced,
                "stderrSha256": mcp.stderr_sha256
            }
        },
        "safety": {
            "allowModelRead": options.allow_model_read,
            "readOnlyMcpSession": true,
            "exactOpenDocumentMatchRequired": true,
            "endpointRevalidatedBeforeAndAfterMcp": true,
            "mcpEndpointReadbackAvailable": false,
            "autoLaunch": false,
            "modelWrites": false,
            "freshQuarantineThenPublish": true,
            "credentialLikeTextRejected": true,
            "reportPagesExported": false,
            "fullPbixToPbipConversion": false
        },
        "limits": {
            "timeoutMs": options.timeout_ms,
            "timeoutScope": "end-to-end-discovery-mcp-validation-publication-with-reserved-cleanup-budget"
        },
        "validation": validation,
        "instructions": [
            "Review the exported definition before wrapping it in a PBIP semantic-model artifact.",
            "This command exports semantic-model TMDL only; it does not extract or reconstruct report pages."
        ],
        "next": [
            "powerbi-cli desktop close --json"
        ]
    }))
}

fn parse_export_args(args: &[String]) -> CliResult<ExportOptions> {
    let mut options = ExportOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--document" => {
                if options.document.is_some() {
                    return Err(duplicate_flag("--document"));
                }
                options.document = Some(PathBuf::from(take_value(args, &mut i, "--document")?));
            }
            "--out-dir" => {
                if options.out_dir.is_some() {
                    return Err(duplicate_flag("--out-dir"));
                }
                options.out_dir = Some(PathBuf::from(take_value(args, &mut i, "--out-dir")?));
            }
            "--allow-model-read" => {
                options.allow_model_read = true;
                i += 1;
            }
            "--timeout-ms" => {
                let value = take_value(args, &mut i, "--timeout-ms")?;
                options.timeout_ms = value.parse::<u64>().map_err(|_| {
                    CliError::invalid_args("--timeout-ms requires an integer from 1000 to 300000")
                })?;
                if !(1_000..=MAX_TIMEOUT_MS).contains(&options.timeout_ms) {
                    return Err(CliError::invalid_args(
                        "--timeout-ms must be from 1000 to 300000",
                    ));
                }
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model live export-tmdl flag: {other}"
                ))
                .with_hint("Use explicit --document, --out-dir, and --allow-model-read flags.")
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"model live export-tmdl\"",
                ));
            }
        }
    }
    Ok(options)
}

fn duplicate_flag(flag: &str) -> CliError {
    CliError::invalid_args(format!("{flag} may be provided only once"))
}

struct LiveExportReservation {
    parent: PathBuf,
    requested: PathBuf,
    quarantine: PathBuf,
    definition_dir: PathBuf,
    cleanup_tombstone: PathBuf,
    quarantine_identity: FileId,
    definition_identity: FileId,
    published: bool,
}

impl LiveExportReservation {
    fn prepare(requested: &Path) -> CliResult<Self> {
        match fs::symlink_metadata(requested) {
            Ok(_) => {
                return Err(CliError::invalid_args(format!(
                    "live TMDL output must not exist before the command: {}",
                    requested.display()
                ))
                .with_hint(
                    "Choose one fresh output directory. The command never merges with or replaces model files.",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(CliError::unexpected(format!(
                    "inspect live TMDL output {}: {error}",
                    requested.display()
                )));
            }
        }
        let parent = requested.parent().unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent).map_err(|error| {
            CliError::invalid_args(format!(
                "live TMDL output parent must already exist: {}: {error}",
                parent.display()
            ))
            .with_hint(
                "Create only the ordinary parent directory, then retry with a fresh child path.",
            )
        })?;
        reject_link_or_reparse(&parent, "live TMDL output parent")?;
        let name = requested
            .file_name()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| CliError::invalid_args("live TMDL output has no directory name"))?;
        let requested = parent.join(name);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let (quarantine, cleanup_tombstone) = (0..64)
            .find_map(|_| {
                let sequence = EXPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let stem = format!(
                    ".powerbi-cli-live-export-{}-{now}-{sequence}",
                    std::process::id()
                );
                let candidate = parent.join(&stem);
                let cleanup = parent.join(format!("{stem}-cleanup"));
                if fs::symlink_metadata(&cleanup).is_ok() {
                    return None;
                }
                match fs::create_dir(&candidate) {
                    Ok(()) => Some(Ok((candidate, cleanup))),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .ok_or_else(|| CliError::unexpected("could not reserve a live TMDL quarantine"))?
            .map_err(|error| {
                CliError::unexpected(format!("create live TMDL quarantine: {error}"))
            })?;
        let definition_dir = quarantine.join("definition");
        if let Err(error) = fs::create_dir(&definition_dir) {
            let _ = fs::remove_dir(&quarantine);
            return Err(CliError::unexpected(format!(
                "create live TMDL definition quarantine: {error}"
            )));
        }
        let quarantine_identity = get_file_id(&quarantine).map_err(|error| {
            CliError::unexpected(format!(
                "open stable live TMDL quarantine identity: {error}"
            ))
        })?;
        let definition_identity = get_file_id(&definition_dir).map_err(|error| {
            CliError::unexpected(format!(
                "open stable live TMDL definition identity: {error}"
            ))
        })?;
        Ok(Self {
            parent,
            requested,
            quarantine,
            definition_dir,
            cleanup_tombstone,
            quarantine_identity,
            definition_identity,
            published: false,
        })
    }

    fn publish(&mut self) -> CliResult<PathBuf> {
        self.verify_owned_export(&self.quarantine)?;
        require_absent(&self.requested, "live TMDL output")?;
        fs::rename(&self.quarantine, &self.requested).map_err(|error| {
            CliError::unexpected(format!(
                "atomically publish live TMDL export {}: {error}",
                self.requested.display()
            ))
        })?;
        if let Err(error) = self.verify_owned_export(&self.requested) {
            if !self.quarantine.exists() {
                let _ = fs::rename(&self.requested, &self.quarantine);
            }
            return Err(error);
        }
        self.published = true;
        let published = fs::canonicalize(&self.requested).map_err(|error| {
            CliError::unexpected(format!(
                "resolve published live TMDL export {}: {error}",
                self.requested.display()
            ))
        })?;
        if published != self.requested {
            return Err(CliError::validation_failed(
                "published live TMDL export escaped its exact output parent",
            ));
        }
        Ok(published)
    }

    fn verify_owned_export(&self, root: &Path) -> CliResult<()> {
        if root.parent() != Some(self.parent.as_path()) {
            return Err(CliError::validation_failed(
                "live TMDL quarantine escaped its exact output parent",
            ));
        }
        reject_link_or_reparse(root, "live TMDL quarantine")?;
        if get_file_id(root).as_ref().ok() != Some(&self.quarantine_identity) {
            return Err(CliError::validation_failed(
                "live TMDL quarantine filesystem identity changed during the run",
            ));
        }
        let definition = root.join("definition");
        reject_link_or_reparse(&definition, "live TMDL definition quarantine")?;
        if get_file_id(&definition).as_ref().ok() != Some(&self.definition_identity) {
            return Err(CliError::validation_failed(
                "live TMDL definition filesystem identity changed during the run",
            ));
        }
        let mut entries = fs::read_dir(root)
            .map_err(|error| {
                CliError::unexpected(format!(
                    "read live TMDL quarantine {}: {error}",
                    root.display()
                ))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                CliError::unexpected(format!("read live TMDL quarantine entry: {error}"))
            })?;
        if entries.len() != 1
            || entries
                .pop()
                .is_none_or(|entry| entry.file_name() != "definition")
        {
            return Err(CliError::validation_failed(
                "live TMDL quarantine must contain only the definition directory",
            ));
        }
        Ok(())
    }

    fn cleanup(&self) {
        if self.verify_owned_export(&self.quarantine).is_err()
            || fs::symlink_metadata(&self.cleanup_tombstone).is_ok()
        {
            return;
        }
        if fs::rename(&self.quarantine, &self.cleanup_tombstone).is_err() {
            return;
        }
        if self.verify_owned_export(&self.cleanup_tombstone).is_err() {
            if !self.quarantine.exists() {
                let _ = fs::rename(&self.cleanup_tombstone, &self.quarantine);
            }
            return;
        }
        let _ = fs::remove_dir_all(&self.cleanup_tombstone);
    }
}

impl Drop for LiveExportReservation {
    fn drop(&mut self) {
        if !self.published {
            self.cleanup();
        }
    }
}

fn reject_link_or_reparse(path: &Path, label: &str) -> CliResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::unexpected(format!("inspect {label} {}: {error}", path.display()))
    })?;
    let linked = metadata.file_type().is_symlink() || {
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
    };
    if linked || !metadata.is_dir() {
        return Err(CliError::invalid_args(format!(
            "{label} must be an ordinary directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn require_absent(path: &Path, label: &str) -> CliResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(CliError::validation_failed(format!(
            "{label} appeared during the run: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::unexpected(format!(
            "inspect {label} {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_requires_explicit_model_read_consent() {
        let error = export_tmdl(&[]).expect_err("consent is required first");
        assert_eq!(error.code, "invalid_args");
        assert!(error.message.contains("--allow-model-read"));
    }

    #[test]
    fn reservation_publishes_only_to_a_fresh_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("model-export");
        let mut reservation = LiveExportReservation::prepare(&output).expect("reserve");
        fs::write(
            reservation.definition_dir.join("database.tmdl"),
            "database X",
        )
        .expect("fixture");
        let published = reservation.publish().expect("publish");
        assert_eq!(
            published,
            fs::canonicalize(&output).expect("canonical output")
        );
        assert!(output.join("definition/database.tmdl").is_file());
        assert!(LiveExportReservation::prepare(&output).is_err());
    }

    #[test]
    fn failed_reservation_is_cleaned() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("model-export");
        let quarantine = {
            let reservation = LiveExportReservation::prepare(&output).expect("reserve");
            reservation.quarantine.clone()
        };
        assert!(!quarantine.exists());
        assert!(!output.exists());
    }

    #[test]
    fn failed_reservation_never_deletes_a_replacement_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("model-export");
        let reservation = LiveExportReservation::prepare(&output).expect("reserve");
        let quarantine = reservation.quarantine.clone();
        let moved_owned = temp.path().join("owned-moved-away");
        fs::rename(&quarantine, &moved_owned).expect("move owned quarantine");
        fs::create_dir(&quarantine).expect("replacement quarantine");
        fs::write(quarantine.join("keep.txt"), "foreign replacement").expect("foreign file");

        drop(reservation);

        assert_eq!(
            fs::read_to_string(quarantine.join("keep.txt")).expect("replacement survives"),
            "foreign replacement"
        );
        assert!(moved_owned.join("definition").is_dir());
    }

    #[test]
    fn publication_rejects_a_replacement_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("model-export");
        let mut reservation = LiveExportReservation::prepare(&output).expect("reserve");
        let quarantine = reservation.quarantine.clone();
        let moved_owned = temp.path().join("owned-moved-away");
        fs::rename(&quarantine, &moved_owned).expect("move owned quarantine");
        fs::create_dir(&quarantine).expect("replacement quarantine");
        fs::create_dir(quarantine.join("definition")).expect("replacement definition");

        let error = reservation.publish().expect_err("replacement must fail");

        assert_eq!(error.code, "validation_failed");
        assert!(!output.exists());
        assert!(quarantine.join("definition").is_dir());
        assert!(moved_owned.join("definition").is_dir());
    }
}
