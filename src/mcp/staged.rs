//! Staged-model replacement preparation, execution, and exact proof machinery.

use super::*;

#[derive(Debug, Clone)]
pub(crate) struct StagedPartitionReplacementRequest {
    pub(crate) source_root: PathBuf,
    pub(crate) staged_semantic_model_root: PathBuf,
    pub(crate) workflow_root: PathBuf,
    pub(crate) fresh_export_root: PathBuf,
    pub(crate) replacements: Vec<StagedPartitionReplacement>,
}

#[derive(Debug, Clone)]
pub(crate) struct StagedPartitionReplacement {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) expected_before_sha256: String,
    pub(crate) complete_m_expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartitionReplacementEvidence {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) before_sha256: String,
    pub(crate) requested_sha256: String,
    pub(crate) readback_sha256: String,
    pub(crate) materialized_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelCleanupEvidence {
    pub(crate) children_reaped: bool,
    pub(crate) pumps_joined: bool,
    pub(crate) forced: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StagedModelSuccess {
    pub(crate) replacements: Vec<PartitionReplacementEvidence>,
    pub(crate) export: ExportShapeProof,
    pub(crate) source: SourceTreeEvidence,
    pub(crate) stage_definition: SourceTreeEvidence,
    pub(crate) expected_stage_sha256: String,
    pub(crate) cleanup: ModelCleanupEvidence,
}

#[derive(Debug, Clone)]
pub(crate) struct StagedModelFailure {
    pub(crate) phase: &'static str,
    pub(crate) error: McpFailure,
}

#[derive(Debug, Clone)]
pub(crate) enum StagedModelResult {
    Succeeded(StagedModelSuccess),
    Failed(StagedModelFailure),
}

pub(crate) fn staged_partition_source_fingerprint(
    semantic_model_root: &Path,
    table: &str,
    partition: &str,
) -> Result<String, McpFailure> {
    let docs = load_table_documents_from_semantic_model(semantic_model_root)
        .map_err(|error| McpFailure::protocol(error.message))?;
    let selector = PartitionSelector {
        table: Some(checked_identifier(table, "table")?),
        name: Some(checked_identifier(partition, "partition")?),
        ..PartitionSelector::default()
    };
    let record =
        find_partition(&docs, &selector).map_err(|error| McpFailure::protocol(error.message))?;
    let source = record.source.as_deref().ok_or_else(|| {
        McpFailure::protocol(format!(
            "partition has no complete M source: {}.{}",
            table, partition
        ))
    })?;
    Ok(source_expression_sha256(source))
}

pub(crate) fn execute_staged_partition_replacements(
    tool: &InstalledMicrosoftTool,
    request: &StagedPartitionReplacementRequest,
    allow_model_write: bool,
) -> StagedModelResult {
    let prepared = match PreparedModelRun::new(request, allow_model_write) {
        Ok(prepared) => prepared,
        Err(failure) => return StagedModelResult::Failed(failure),
    };
    let mut session = match McpSession::open_exact(
        tool,
        McpSessionMode::ConfirmedWrite,
        McpSessionConfig::default(),
    ) {
        Ok(session) => session,
        Err(error) => {
            return finish_model_run(
                &prepared,
                CoreModelOutcome::Failed(CoreModelFailure::new("handshake", error)),
                None,
            );
        }
    };
    if let Err(error) = session.handshake() {
        let cleanup = session.shutdown(false);
        return finish_model_run(
            &prepared,
            CoreModelOutcome::Failed(CoreModelFailure::new("handshake", error)),
            Some(cleanup_evidence(&cleanup)),
        );
    }
    let core = run_prepared_model(&mut session, &prepared);
    let cancelled = core.failure_kind() == Some(McpFailureKind::Cancelled);
    let cleanup = session.shutdown(!cancelled && !session.poisoned);
    let cleanup = cleanup_evidence(&cleanup);
    let core = if cleanup.children_reaped && cleanup.pumps_joined {
        materialize_verified(&prepared, core)
    } else {
        core
    };
    finish_model_run(&prepared, core, Some(cleanup))
}

pub(crate) fn execute_staged_model_export_proof(
    tool: &InstalledMicrosoftTool,
    source_root: &Path,
    staged_semantic_model_root: &Path,
    scratch_root: &Path,
) -> Result<ExportShapeProof, McpFailure> {
    let reservation = PreparedStagedModel::prepare(
        source_root,
        staged_semantic_model_root,
        scratch_root,
        &scratch_root.join("canonical-export"),
    )
    .map_err(McpFailure::protocol)?;
    let prepared = reservation.commit();
    let mut session =
        McpSession::open_exact(tool, McpSessionMode::ReadOnly, McpSessionConfig::default())?;
    let proof = session.handshake().and_then(|_| {
        let connection = connect_exact(&mut session, &prepared.definition_dir)?;
        prepared
            .ensure_export_empty()
            .map_err(McpFailure::protocol)?;
        call_tool_payload(
            &mut session,
            &McpOperation::ExportTmdlFolder {
                connection_name: connection.name,
                folder_path: prepared.export_root.join("definition"),
            },
            "ExportToTmdlFolder",
        )?;
        prepared.validate_export().map_err(McpFailure::protocol)
    });
    let cleanup = session.shutdown(proof.is_ok() && !session.poisoned);
    if !cleanup.children_reaped || !cleanup.pumps_joined {
        let _ = prepared.mark_export_failure_only();
        return Err(McpFailure::backend(
            "canonical staged-model export proof cleanup was incomplete",
        )
        .with_cleanup(&cleanup));
    }
    match proof {
        Ok(proof) => {
            prepared
                .disarm_export_quarantine()
                .map_err(McpFailure::backend)?;
            Ok(proof)
        }
        Err(error) => {
            let _ = prepared.mark_export_failure_only();
            Err(error.with_cleanup(&cleanup))
        }
    }
}

#[derive(Debug)]
pub(crate) struct LiveTmdlExportMcpProof {
    pub(crate) handshake: McpHandshake,
    pub(crate) notifications_seen: usize,
    pub(crate) cleanup_children_reaped: bool,
    pub(crate) cleanup_pumps_joined: bool,
    pub(crate) cleanup_forced: bool,
    pub(crate) stderr_sha256: String,
}

pub(crate) fn execute_live_tmdl_export<F>(
    tool: &InstalledMicrosoftTool,
    port: u16,
    definition_dir: &Path,
    timeout: Duration,
    mut verify_endpoint: F,
) -> CliResult<LiveTmdlExportMcpProof>
where
    F: FnMut(Duration) -> CliResult<()>,
{
    let cleanup_timeout = DEFAULT_CLEANUP_TIMEOUT
        .min(timeout / 4)
        .max(Duration::from_millis(100));
    let session_timeout = timeout.checked_sub(cleanup_timeout).ok_or_else(|| {
        CliError::new(
            "desktop_operation_timeout",
            EXIT_ORACLE_FAILED,
            "live TMDL export deadline left no MCP session budget",
        )
    })?;
    let config = McpSessionConfig {
        call_timeout: session_timeout,
        session_timeout,
        cleanup_timeout,
        ..McpSessionConfig::default()
    };
    let mut session = McpSession::open_exact(tool, McpSessionMode::ReadOnly, config)
        .map_err(McpFailure::into_cli_error)?;
    let operation = (|| {
        let handshake = session.handshake()?;
        let verification_budget = session.remaining_call_timeout()?;
        verify_endpoint(verification_budget).map_err(|error| {
            McpFailure::backend(format!(
                "live Desktop endpoint revalidation failed before MCP connect: {} ({})",
                error.message, error.code
            ))
        })?;
        let connection = connect_local_exact(&mut session, port)?;
        call_tool_payload(
            &mut session,
            &McpOperation::ExportTmdlFolder {
                connection_name: connection.name.clone(),
                folder_path: definition_dir.to_path_buf(),
            },
            "ExportToTmdlFolder",
        )?;
        let verification_budget = session.remaining_call_timeout()?;
        verify_endpoint(verification_budget).map_err(|error| {
            McpFailure::backend(format!(
                "live Desktop endpoint revalidation failed after MCP export: {} ({})",
                error.message, error.code
            ))
        })?;
        Ok::<_, McpFailure>((handshake, connection))
    })();
    let cleanup = session.shutdown(operation.is_ok() && !session.poisoned);
    if !cleanup.children_reaped || !cleanup.pumps_joined {
        return Err(
            McpFailure::backend("live TMDL export MCP cleanup was incomplete")
                .with_cleanup(&cleanup)
                .into_cli_error(),
        );
    }
    let (handshake, _connection) =
        operation.map_err(|error| error.with_cleanup(&cleanup).into_cli_error())?;
    Ok(LiveTmdlExportMcpProof {
        notifications_seen: session.notifications_seen,
        handshake,
        cleanup_children_reaped: cleanup.children_reaped,
        cleanup_pumps_joined: cleanup.pumps_joined,
        cleanup_forced: cleanup.forced,
        stderr_sha256: cleanup.stderr_sha256,
    })
}

trait ModelMcpClient {
    fn call_model(&mut self, operation: &McpOperation) -> Result<Value, McpFailure>;
}

impl ModelMcpClient for McpSession {
    fn call_model(&mut self, operation: &McpOperation) -> Result<Value, McpFailure> {
        self.call(operation)
    }
}

struct PreparedModelRun {
    paths: PreparedStagedModel,
    source_snapshot: SourceTreeSnapshot,
    stage_snapshot: SourceTreeSnapshot,
    expected_stage_sha256: String,
    replacements: Vec<PreparedReplacement>,
    native_plans: Vec<MutationPlan>,
}

struct PreparedReplacement {
    table: String,
    partition: String,
    before_sha256: String,
    requested_sha256: String,
    expression: String,
}

impl PreparedModelRun {
    #[allow(clippy::result_large_err)]
    fn new(
        request: &StagedPartitionReplacementRequest,
        allow_model_write: bool,
    ) -> Result<Self, StagedModelFailure> {
        if !allow_model_write {
            return Err(StagedModelFailure::unprepared(
                "consent",
                McpFailure::protocol(
                    "model writes require explicit --allow-model-write-equivalent consent",
                ),
            ));
        }
        if request.replacements.is_empty() || request.replacements.len() > 100 {
            return Err(StagedModelFailure::unprepared(
                "prepare",
                McpFailure::protocol(
                    "staged model writes require between 1 and 100 typed partition replacements",
                ),
            ));
        }
        if request.replacements.iter().any(|replacement| {
            contains_credential_like_text_str(&replacement.complete_m_expression)
        }) {
            return Err(StagedModelFailure::unprepared(
                "credential-scan",
                McpFailure::protocol(
                    "complete M expressions with credential-like text are forbidden at the staged MCP boundary",
                ),
            ));
        }
        let reservation = PreparedStagedModel::prepare(
            &request.source_root,
            &request.staged_semantic_model_root,
            &request.workflow_root,
            &request.fresh_export_root,
        )
        .map_err(|message| {
            StagedModelFailure::unprepared("paths", McpFailure::protocol(message))
        })?;
        let paths = reservation.paths();
        let source_snapshot =
            SourceTreeSnapshot::capture(&paths.source_root).map_err(|message| {
                StagedModelFailure::unprepared("source-proof", McpFailure::backend(message))
            })?;
        let stage_snapshot =
            SourceTreeSnapshot::capture(&paths.definition_dir).map_err(|message| {
                StagedModelFailure::unprepared("stage-proof", McpFailure::backend(message))
            })?;
        let docs = load_table_documents_from_semantic_model(&paths.semantic_model_root).map_err(
            |error| StagedModelFailure::unprepared("prepare", McpFailure::protocol(error.message)),
        )?;
        let mut handles = BTreeSet::new();
        let mut native_plans = BTreeMap::<PathBuf, MutationPlan>::new();
        let mut replacements = Vec::with_capacity(request.replacements.len());
        for replacement in &request.replacements {
            let table = checked_identifier(&replacement.table, "table")
                .map_err(|error| StagedModelFailure::unprepared("prepare", error))?;
            let partition = checked_identifier(&replacement.partition, "partition")
                .map_err(|error| StagedModelFailure::unprepared("prepare", error))?;
            let handle = format!("{table}\u{0}{partition}");
            if !handles.insert(handle) {
                return Err(StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol("duplicate typed partition replacement"),
                ));
            }
            let selector = PartitionSelector {
                table: Some(table.clone()),
                name: Some(partition.clone()),
                ..PartitionSelector::default()
            };
            let record = find_partition(&docs, &selector).map_err(|error| {
                StagedModelFailure::unprepared("prepare", McpFailure::protocol(error.message))
            })?;
            let before = record.source.as_deref().ok_or_else(|| {
                StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol(format!(
                        "partition has no complete M source: {table}.{partition}"
                    )),
                )
            })?;
            let before_sha256 = source_expression_sha256(before);
            if replacement.expected_before_sha256 != before_sha256 {
                return Err(StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol(format!(
                        "partition before fingerprint drift for {table}.{partition}"
                    )),
                ));
            }
            let expression = normalized_source_expression(&replacement.complete_m_expression)
                .ok_or_else(|| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::protocol("complete M expression must not be empty"),
                    )
                })?;
            let requested_sha256 = source_expression_sha256(&expression);
            let native_plan = replace_partition_source_plan(&docs, &selector, &expression)
                .map_err(|error| {
                    StagedModelFailure::unprepared("prepare", McpFailure::protocol(error.message))
                })?;
            let canonical_plan_path =
                std::fs::canonicalize(&native_plan.path).map_err(|error| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::backend(format!(
                            "resolve native partition write {}: {error}",
                            native_plan.path.display()
                        )),
                    )
                })?;
            if !canonical_plan_path.starts_with(&paths.definition_dir) {
                return Err(StagedModelFailure::unprepared(
                    "prepare",
                    McpFailure::protocol("native partition write escaped the staged definition"),
                ));
            }
            if let Some(composed) = native_plans.get_mut(&canonical_plan_path) {
                let before = native_plan.before_block.as_deref().ok_or_else(|| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::backend("native partition plan has no before block"),
                    )
                })?;
                let after = native_plan.after_block.as_deref().ok_or_else(|| {
                    StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::backend("native partition plan has no after block"),
                    )
                })?;
                let mut matches = composed.new_text.match_indices(before);
                let Some((start, _)) = matches.next() else {
                    return Err(StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::protocol(
                            "same-file partition replacements could not be composed exactly",
                        ),
                    ));
                };
                if matches.next().is_some() {
                    return Err(StagedModelFailure::unprepared(
                        "prepare",
                        McpFailure::protocol(
                            "same-file partition replacement is ambiguous in the original TMDL",
                        ),
                    ));
                }
                composed
                    .new_text
                    .replace_range(start..start + before.len(), after);
            } else {
                native_plans.insert(canonical_plan_path, native_plan);
            }
            replacements.push(PreparedReplacement {
                table,
                partition,
                before_sha256,
                requested_sha256,
                expression,
            });
        }
        let native_plans = native_plans.into_values().collect::<Vec<_>>();
        let expected_replacements = native_plans
            .iter()
            .map(|plan| (plan.path.clone(), plan.new_text.clone()))
            .collect::<Vec<_>>();
        let expected_stage_sha256 = stage_snapshot
            .expected_after_sha256(&expected_replacements)
            .map_err(|message| {
                StagedModelFailure::unprepared("stage-proof", McpFailure::backend(message))
            })?;
        Ok(Self {
            paths: reservation.commit(),
            source_snapshot,
            stage_snapshot,
            expected_stage_sha256,
            replacements,
            native_plans,
        })
    }
}

enum CoreModelOutcome {
    Verified(CoreModelSuccess),
    Materialized(CoreModelSuccess),
    Failed(CoreModelFailure),
}

impl CoreModelOutcome {
    fn failure_kind(&self) -> Option<McpFailureKind> {
        match self {
            Self::Verified(_) | Self::Materialized(_) => None,
            Self::Failed(failure) => Some(failure.error.kind()),
        }
    }
}

struct CoreModelSuccess {
    replacements: Vec<PartitionReplacementEvidence>,
    export: ExportShapeProof,
}

struct CoreModelFailure {
    phase: &'static str,
    error: McpFailure,
}

impl CoreModelFailure {
    fn new(phase: &'static str, error: McpFailure) -> Self {
        Self { phase, error }
    }
}

impl StagedModelFailure {
    fn unprepared(phase: &'static str, error: McpFailure) -> Self {
        Self { phase, error }
    }
}

fn run_prepared_model<C: ModelMcpClient>(
    client: &mut C,
    prepared: &PreparedModelRun,
) -> CoreModelOutcome {
    let connection = match connect_exact(client, &prepared.paths.definition_dir) {
        Ok(connection) => connection,
        Err(error) => {
            return CoreModelOutcome::Failed(CoreModelFailure::new("connection", error));
        }
    };
    let connection_name = connection.name.clone();
    for replacement in &prepared.replacements {
        if let Err(error) = call_tool_payload(
            client,
            &McpOperation::ReplacePartitionSource {
                connection_name: connection_name.clone(),
                table_name: replacement.table.clone(),
                partition_name: replacement.partition.clone(),
                expression: replacement.expression.clone(),
            },
            "Update",
        ) {
            return CoreModelOutcome::Failed(offline_failure(
                "write",
                sanitize_write_failure(error),
            ));
        }
    }
    let mut evidence = Vec::with_capacity(prepared.replacements.len());
    for replacement in &prepared.replacements {
        let readback = match call_tool_payload(
            client,
            &McpOperation::GetPartition {
                connection_name: connection_name.clone(),
                table_name: replacement.table.clone(),
                partition_name: replacement.partition.clone(),
            },
            "GET",
        )
        .and_then(|payload| {
            exact_partition_readback(&payload, &replacement.table, &replacement.partition)
        }) {
            Ok(readback) => readback,
            Err(error) => {
                return CoreModelOutcome::Failed(offline_failure("readback", error));
            }
        };
        let readback = match normalized_source_expression(&readback) {
            Some(readback) => readback,
            None => {
                return CoreModelOutcome::Failed(offline_failure(
                    "readback",
                    McpFailure::protocol("partition readback returned an empty expression"),
                ));
            }
        };
        let readback_sha256 = source_expression_sha256(&readback);
        if readback != replacement.expression || readback_sha256 != replacement.requested_sha256 {
            return CoreModelOutcome::Failed(offline_failure(
                "readback",
                McpFailure::protocol(format!(
                    "partition readback mismatch for {}.{}",
                    replacement.table, replacement.partition
                )),
            ));
        }
        evidence.push(PartitionReplacementEvidence {
            table: replacement.table.clone(),
            partition: replacement.partition.clone(),
            before_sha256: replacement.before_sha256.clone(),
            requested_sha256: replacement.requested_sha256.clone(),
            readback_sha256,
            materialized_sha256: String::new(),
        });
    }
    if let Err(message) = prepared.paths.ensure_export_empty() {
        return CoreModelOutcome::Failed(offline_failure(
            "export-guard",
            McpFailure::protocol(message),
        ));
    }
    if let Err(error) = call_tool_payload(
        client,
        &McpOperation::ExportTmdlFolder {
            connection_name,
            folder_path: prepared.paths.export_root.join("definition"),
        },
        "ExportToTmdlFolder",
    ) {
        return CoreModelOutcome::Failed(offline_failure("export", error));
    }
    let export = match prepared.paths.validate_export() {
        Ok(export) => export,
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "export-proof",
                McpFailure::protocol(message),
            ));
        }
    };
    match prepared.stage_snapshot.verify() {
        Ok(stage) if stage.byte_identical => {}
        Ok(_) => {
            return CoreModelOutcome::Failed(offline_failure(
                "stage-proof",
                McpFailure::protocol(
                    "staged definition changed before native readback materialization",
                ),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "stage-proof",
                McpFailure::backend(message),
            ));
        }
    }
    CoreModelOutcome::Verified(CoreModelSuccess {
        replacements: evidence,
        export,
    })
}

fn materialize_verified(prepared: &PreparedModelRun, core: CoreModelOutcome) -> CoreModelOutcome {
    let CoreModelOutcome::Verified(mut success) = core else {
        return core;
    };
    match prepared.source_snapshot.verify() {
        Ok(source) if source.byte_identical => {}
        Ok(_) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-source-proof",
                McpFailure::protocol(
                    "source project changed before native readback materialization",
                ),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-source-proof",
                McpFailure::backend(message),
            ));
        }
    }
    match prepared.stage_snapshot.verify() {
        Ok(stage) if stage.byte_identical => {}
        Ok(_) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-stage-proof",
                McpFailure::protocol(
                    "staged definition changed before isolated MCP cleanup completed",
                ),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "post-cleanup-stage-proof",
                McpFailure::backend(message),
            ));
        }
    }
    for plan in &prepared.native_plans {
        if let Err(error) = write_text_atomic(&plan.path, &plan.new_text) {
            return CoreModelOutcome::Failed(offline_failure(
                "materialize",
                McpFailure::backend(error.message),
            ));
        }
    }
    for (replacement, replacement_evidence) in prepared
        .replacements
        .iter()
        .zip(success.replacements.iter_mut())
    {
        match staged_partition_source_fingerprint(
            &prepared.paths.semantic_model_root,
            &replacement.table,
            &replacement.partition,
        ) {
            Ok(materialized) if materialized == replacement.requested_sha256 => {
                replacement_evidence.materialized_sha256 = materialized;
            }
            Ok(_) => {
                return CoreModelOutcome::Failed(offline_failure(
                    "materialize",
                    McpFailure::protocol(format!(
                        "native materialization readback mismatch for {}.{}",
                        replacement.table, replacement.partition
                    )),
                ));
            }
            Err(error) => {
                return CoreModelOutcome::Failed(offline_failure("materialize", error));
            }
        }
    }
    match prepared.stage_snapshot.verify() {
        Ok(stage) if stage.after_sha256 == prepared.expected_stage_sha256 => {}
        Ok(stage) => {
            return CoreModelOutcome::Failed(offline_failure(
                "materialize-proof",
                McpFailure::protocol(format!(
                    "staged definition differs from the exact expected-after tree: expected {}, got {}",
                    prepared.expected_stage_sha256, stage.after_sha256
                )),
            ));
        }
        Err(message) => {
            return CoreModelOutcome::Failed(offline_failure(
                "materialize-proof",
                McpFailure::backend(message),
            ));
        }
    }
    CoreModelOutcome::Materialized(success)
}

fn offline_failure(phase: &'static str, error: McpFailure) -> CoreModelFailure {
    CoreModelFailure { phase, error }
}

fn sanitize_write_failure(error: McpFailure) -> McpFailure {
    McpFailure::new(
        error.kind(),
        format!(
            "Modeling MCP partition Update failed; vendorDetailSha256={}",
            sha256_bytes(error.message().as_bytes())
        ),
    )
}

fn finish_model_run(
    prepared: &PreparedModelRun,
    core: CoreModelOutcome,
    cleanup: Option<ModelCleanupEvidence>,
) -> StagedModelResult {
    let source = prepared.source_snapshot.verify();
    let stage = prepared.stage_snapshot.verify();
    let source_evidence = source.as_ref().ok().cloned();
    let stage_evidence = stage.as_ref().ok().cloned();
    if let Err(message) = source {
        let _ = prepared.paths.mark_export_failure_only();
        return StagedModelResult::Failed(StagedModelFailure {
            phase: "source-proof",
            error: McpFailure::backend(message),
        });
    }
    if source_evidence
        .as_ref()
        .is_some_and(|evidence| !evidence.byte_identical)
    {
        let _ = prepared.paths.mark_export_failure_only();
        return StagedModelResult::Failed(StagedModelFailure {
            phase: "source-proof",
            error: McpFailure::protocol("source project changed during staged model workflow"),
        });
    }
    if cleanup
        .as_ref()
        .is_some_and(|cleanup| !cleanup.children_reaped || !cleanup.pumps_joined)
    {
        let _ = prepared.paths.mark_export_failure_only();
        return StagedModelResult::Failed(StagedModelFailure {
            phase: "cleanup",
            error: McpFailure::backend("Modeling MCP cleanup was incomplete"),
        });
    }
    match core {
        CoreModelOutcome::Materialized(success) => {
            let Some(cleanup) = cleanup else {
                let _ = prepared.paths.mark_export_failure_only();
                return StagedModelResult::Failed(StagedModelFailure {
                    phase: "cleanup",
                    error: McpFailure::backend(
                        "successful isolated MCP workflow requires cleanup evidence",
                    ),
                });
            };
            if let Err(message) = prepared.paths.disarm_export_quarantine() {
                let _ = prepared.paths.mark_export_failure_only();
                return StagedModelResult::Failed(StagedModelFailure {
                    phase: "export-quarantine",
                    error: McpFailure::backend(message),
                });
            }
            StagedModelResult::Succeeded(StagedModelSuccess {
                replacements: success.replacements,
                export: success.export,
                source: source_evidence.expect("source proof captured for prepared run"),
                stage_definition: stage_evidence.expect("stage proof captured for prepared run"),
                expected_stage_sha256: prepared.expected_stage_sha256.clone(),
                cleanup,
            })
        }
        CoreModelOutcome::Verified(_success) => {
            let _ = prepared.paths.mark_export_failure_only();
            StagedModelResult::Failed(StagedModelFailure {
                phase: "cleanup",
                error: McpFailure::backend(
                    "isolated MCP result was not materialized after complete cleanup",
                ),
            })
        }
        CoreModelOutcome::Failed(failure) => {
            let _ = prepared.paths.mark_export_failure_only();
            StagedModelResult::Failed(StagedModelFailure {
                phase: failure.phase,
                error: failure.error,
            })
        }
    }
}

fn cleanup_evidence(cleanup: &McpCleanupReport) -> ModelCleanupEvidence {
    ModelCleanupEvidence {
        children_reaped: cleanup.children_reaped,
        pumps_joined: cleanup.pumps_joined,
        forced: cleanup.forced,
    }
}

struct ExactConnection {
    name: String,
}

fn connect_local_exact<C: ModelMcpClient>(
    client: &mut C,
    port: u16,
) -> Result<ExactConnection, McpFailure> {
    let connected = call_tool_payload(
        client,
        &McpOperation::ConnectLocalEndpoint { port },
        "Connect",
    )?;
    let connection_name = connected
        .get("data")
        .and_then(|data| {
            data.as_str().or_else(|| {
                data.as_object()
                    .and_then(|object| object.get("connectionName"))
                    .and_then(Value::as_str)
            })
        })
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            McpFailure::protocol(format!(
                "Connect payload omitted a data connection identity; fields=[{}], dataType={}",
                connected
                    .as_object()
                    .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_else(|| "non-object".to_string()),
                match connected.get("data") {
                    Some(Value::Null) => "null",
                    Some(Value::Bool(_)) => "bool",
                    Some(Value::Number(_)) => "number",
                    Some(Value::String(_)) => "string",
                    Some(Value::Array(_)) => "array",
                    Some(Value::Object(_)) => "object",
                    None => "missing",
                }
            ))
        })?;
    let listed = call_tool_payload(client, &McpOperation::ListConnections, "ListConnections")?;
    let entries = listed
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("ListConnections has no data array"))?;
    let exact = entries
        .iter()
        .filter_map(Value::as_object)
        .filter(|entry| {
            entry.get("connectionName").and_then(Value::as_str) == Some(connection_name.as_str())
        })
        .count();
    if exact != 1 {
        return Err(McpFailure::protocol(format!(
            "expected exactly one listed connection for the opaque name returned by Connect, found {exact}"
        )));
    }
    Ok(ExactConnection {
        name: connection_name,
    })
}

fn connect_exact<C: ModelMcpClient>(
    client: &mut C,
    definition_dir: &Path,
) -> Result<ExactConnection, McpFailure> {
    let connected = call_tool_payload(
        client,
        &McpOperation::ConnectFolder {
            folder_path: definition_dir.to_path_buf(),
        },
        "ConnectFolder",
    )?;
    let data = payload_data_object(&connected)?;
    let connection_name = required_object_string(data, "connectionName", "connection data")?;
    let connected_path = required_object_string(data, "folderPath", "connection data")?;
    require_exact_canonical_path(&connected_path, definition_dir, "connected folder")?;
    let listed = call_tool_payload(client, &McpOperation::ListConnections, "ListConnections")?;
    let entries = listed
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("ListConnections has no data array"))?;
    let mut exact = 0_usize;
    let mut exact_name = None;
    for entry in entries {
        let Some(object) = entry.as_object() else {
            return Err(McpFailure::protocol(
                "ListConnections contains a non-object entry",
            ));
        };
        let listed_name =
            required_object_string(object, "connectionName", "connection list entry")?;
        let source_path = required_object_string(object, "sourcePath", "connection list entry")?;
        if require_exact_canonical_path(&source_path, definition_dir, "listed folder").is_ok() {
            exact = exact.saturating_add(1);
            exact_name = Some(listed_name);
        }
    }
    if exact != 1 || exact_name.as_deref() != Some(connection_name.as_str()) {
        return Err(McpFailure::protocol(format!(
            "expected exactly one globally unique staged folder path with the ConnectFolder identity, found exact={exact}"
        )));
    }
    Ok(ExactConnection {
        name: connection_name,
    })
}

fn require_exact_canonical_path(
    reported: &str,
    expected: &Path,
    label: &str,
) -> Result<(), McpFailure> {
    let reported = std::fs::canonicalize(reported)
        .map_err(|error| McpFailure::protocol(format!("resolve {label} {reported}: {error}")))?;
    if reported != expected {
        return Err(McpFailure::protocol(format!(
            "{label} does not match the exact staged definition"
        )));
    }
    Ok(())
}

fn call_tool_payload<C: ModelMcpClient>(
    client: &mut C,
    operation: &McpOperation,
    expected_operation: &str,
) -> Result<Value, McpFailure> {
    let result = client.call_model(operation)?;
    parse_tool_payload(&result, expected_operation)
}

fn parse_tool_payload(result: &Value, expected_operation: &str) -> Result<Value, McpFailure> {
    let result = exact_object(result, &["_meta", "content", "isError"], "MCP tool result")?;
    validate_tool_result_metadata(result, expected_operation)?;
    match result.get("isError").and_then(Value::as_bool) {
        Some(false) => {}
        Some(true) => {
            return Err(McpFailure::backend(format!(
                "MCP tool {expected_operation} returned an error result"
            )));
        }
        None => {
            return Err(McpFailure::protocol(
                "MCP tool result isError must be exactly one boolean",
            ));
        }
    }
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("MCP tool result has no content array"))?;
    if content.len() != 1 {
        return Err(McpFailure::protocol(
            "MCP tool result must contain exactly one text payload",
        ));
    }
    let content = content[0]
        .as_object()
        .ok_or_else(|| McpFailure::protocol("MCP tool content is not one object"))?;
    exact_keys(content, &["text", "type"], "MCP tool text content")?;
    if content.get("type").and_then(Value::as_str) != Some("text") {
        return Err(McpFailure::protocol(
            "MCP tool content is not the expected text payload",
        ));
    }
    let text = content
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| McpFailure::protocol("MCP tool content has no text"))?;
    if text.len() > DEFAULT_TOTAL_RESPONSE_LIMIT {
        return Err(McpFailure::protocol(
            "MCP tool text payload exceeds the bounded response cap",
        ));
    }
    let payload: Value = serde_json::from_str(text)
        .map_err(|error| McpFailure::protocol(format!("parse MCP tool payload: {error}")))?;
    if expected_operation == "Update" {
        let object = payload.as_object().ok_or_else(|| {
            McpFailure::protocol("partition Update payload is not the exact empty object")
        })?;
        if !object.is_empty() {
            return Err(McpFailure::protocol(
                "partition Update payload is not the exact empty object",
            ));
        }
        return Ok(payload);
    }
    let operation = payload
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| McpFailure::protocol("MCP tool payload has no operation"))?;
    if operation != expected_operation {
        return Err(McpFailure::protocol(format!(
            "MCP tool payload operation drift: expected {expected_operation}, got {operation}"
        )));
    }
    Ok(payload)
}

fn validate_tool_result_metadata(
    result: &Map<String, Value>,
    expected_operation: &str,
) -> Result<(), McpFailure> {
    let metadata = exact_object(
        result
            .get("_meta")
            .ok_or_else(|| McpFailure::protocol("MCP tool result has no _meta object"))?,
        &["annotations"],
        "MCP tool result _meta",
    )?;
    let annotations = exact_object(
        metadata
            .get("annotations")
            .ok_or_else(|| McpFailure::protocol("MCP tool result has no annotations object"))?,
        &["readOnlyHint", "title"],
        "MCP tool result annotations",
    )?;
    let (expected_title, expected_read_only) = match expected_operation {
        "Connect" => ("connection_operations.connect", true),
        "ConnectFolder" => ("connection_operations.connectfolder", true),
        "ListConnections" => ("connection_operations.listconnections", true),
        "GET" => ("partition_operations.get", true),
        "Update" => ("partition_operations.update", false),
        "ExportToTmdlFolder" => ("database_operations.exporttotmdlfolder", true),
        _ => {
            return Err(McpFailure::protocol(format!(
                "MCP tool metadata has no closed policy for {expected_operation}"
            )));
        }
    };
    if annotations.get("title").and_then(Value::as_str) != Some(expected_title)
        || annotations.get("readOnlyHint").and_then(Value::as_bool) != Some(expected_read_only)
    {
        return Err(McpFailure::protocol(format!(
            "MCP tool result annotations drifted from the exact {expected_operation} contract"
        )));
    }
    Ok(())
}

fn payload_data_object(payload: &Value) -> Result<&Map<String, Value>, McpFailure> {
    payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| McpFailure::protocol("MCP tool payload has no data object"))
}

fn exact_partition_readback(
    payload: &Value,
    table: &str,
    partition: &str,
) -> Result<String, McpFailure> {
    let payload = exact_object(
        payload,
        &["message", "operation", "results", "summary", "warnings"],
        "partition Get payload",
    )?;
    if payload.get("operation").and_then(Value::as_str) != Some("GET") {
        return Err(McpFailure::protocol(
            "partition Get payload operation must be exactly GET",
        ));
    }
    required_object_string(payload, "message", "partition Get payload")?;
    let summary = exact_object(
        payload
            .get("summary")
            .ok_or_else(|| McpFailure::protocol("partition Get payload has no summary"))?,
        &[
            "executionTime",
            "failureCount",
            "successCount",
            "totalItems",
        ],
        "partition Get summary",
    )?;
    let execution_time = required_object_string(summary, "executionTime", "partition Get summary")?;
    if execution_time.is_empty()
        || summary.get("failureCount").and_then(Value::as_u64) != Some(0)
        || summary.get("successCount").and_then(Value::as_u64) != Some(1)
        || summary.get("totalItems").and_then(Value::as_u64) != Some(1)
    {
        return Err(McpFailure::protocol(
            "partition Get summary does not prove exactly one successful readback",
        ));
    }
    let warnings = payload
        .get("warnings")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("partition Get warnings must be one array"))?;
    if !warnings.is_empty() {
        return Err(McpFailure::protocol(
            "partition Get returned warnings and cannot be trusted as exact readback",
        ));
    }
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| McpFailure::protocol("partition Get has no results array"))?;
    let [result] = results.as_slice() else {
        return Err(McpFailure::protocol(format!(
            "partition Get returned {} results for {table}.{partition}; exactly one is required",
            results.len()
        )));
    };
    let result = exact_object(
        result,
        &["data", "index", "itemIdentifier", "message", "warnings"],
        "partition Get result",
    )?;
    if result.get("index").and_then(Value::as_u64) != Some(0)
        || required_object_string(result, "itemIdentifier", "partition Get result")?.is_empty()
        || required_object_string(result, "message", "partition Get result")?.is_empty()
        || !result
            .get("warnings")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    {
        return Err(McpFailure::protocol(
            "partition Get result metadata does not prove one warning-free item",
        ));
    }
    let data = result
        .get("data")
        .ok_or_else(|| McpFailure::protocol("partition Get result has no data"))?;
    let data = exact_object(
        data,
        &[
            "annotations",
            "attributes",
            "dataView",
            "description",
            "errorMessage",
            "expression",
            "extendedProperties",
            "mode",
            "modifiedTime",
            "name",
            "sourceType",
            "state",
            "tableName",
        ],
        "partition Get data",
    )?;
    for field in ["annotations", "extendedProperties"] {
        if data.get(field).and_then(Value::as_array).is_none() {
            return Err(McpFailure::protocol(format!(
                "partition Get data field {field} must be one array"
            )));
        }
    }
    for field in [
        "attributes",
        "dataView",
        "description",
        "errorMessage",
        "mode",
        "modifiedTime",
        "state",
    ] {
        required_object_string(data, field, "partition Get data")?;
    }
    if data.get("errorMessage").and_then(Value::as_str) != Some("")
        || data.get("mode").and_then(Value::as_str) == Some("")
        || data.get("state").and_then(Value::as_str) == Some("")
    {
        return Err(McpFailure::protocol(
            "partition Get data is not a successful materialized partition",
        ));
    }
    if data.get("tableName").and_then(Value::as_str) != Some(table)
        || data.get("name").and_then(Value::as_str) != Some(partition)
    {
        return Err(McpFailure::protocol(format!(
            "partition Get did not return the exact requested identity {table}.{partition}"
        )));
    }
    if data.get("sourceType").and_then(Value::as_str) != Some("M") {
        return Err(McpFailure::protocol(
            "partition Get readback is not an M source",
        ));
    }
    required_object_string(data, "expression", "partition Get data")
}

fn normalized_source_expression(value: &str) -> Option<String> {
    let value = value
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let value = value.trim_matches('\n');
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn source_expression_sha256(value: &str) -> String {
    normalized_source_expression(value)
        .map_or_else(|| sha256_bytes(b""), |value| sha256_bytes(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    enum FakeExportShape {
        Valid,
        RootTmdl,
    }

    struct FakeModelClient {
        definition_dir: PathBuf,
        calls: Vec<&'static str>,
        expressions: std::collections::BTreeMap<(String, String), String>,
        fail_at: Option<&'static str>,
        fail_kind: McpFailureKind,
        failure_message: Option<String>,
        readback_mismatch: bool,
        duplicate_connection: bool,
        duplicate_path_different_name: bool,
        export_shape: FakeExportShape,
    }

    impl FakeModelClient {
        fn new(definition_dir: PathBuf) -> Self {
            Self {
                definition_dir,
                calls: Vec::new(),
                expressions: std::collections::BTreeMap::new(),
                fail_at: None,
                fail_kind: McpFailureKind::Backend,
                failure_message: None,
                readback_mismatch: false,
                duplicate_connection: false,
                duplicate_path_different_name: false,
                export_shape: FakeExportShape::Valid,
            }
        }

        fn enter(&mut self, label: &'static str) -> Result<(), McpFailure> {
            self.calls.push(label);
            if self.fail_at == Some(label) {
                return Err(McpFailure::new(
                    self.fail_kind,
                    self.failure_message
                        .clone()
                        .unwrap_or_else(|| format!("injected {label} failure")),
                ));
            }
            Ok(())
        }
    }

    impl ModelMcpClient for FakeModelClient {
        fn call_model(&mut self, operation: &McpOperation) -> Result<Value, McpFailure> {
            match operation {
                McpOperation::ConnectLocalEndpoint { port } => {
                    self.enter("connect-local")?;
                    assert_ne!(*port, 0);
                    Ok(fake_tool_result(json!({
                        "operation": "Connect",
                        "data": "fixture-connection"
                    })))
                }
                McpOperation::ConnectFolder { folder_path } => {
                    self.enter("connect")?;
                    assert_eq!(folder_path, &self.definition_dir);
                    Ok(fake_tool_result(json!({
                        "operation": "ConnectFolder",
                        "data": {
                            "connectionName": "fixture-connection",
                            "folderPath": self.definition_dir
                        }
                    })))
                }
                McpOperation::ListConnections => {
                    self.enter("list")?;
                    let mut connections = vec![json!({
                        "connectionName": "fixture-connection",
                        "sourcePath": self.definition_dir
                    })];
                    if self.duplicate_connection {
                        connections.push(json!({
                            "connectionName": "fixture-connection",
                            "sourcePath": self.definition_dir
                        }));
                    }
                    if self.duplicate_path_different_name {
                        connections.push(json!({
                            "connectionName": "different-connection",
                            "sourcePath": self.definition_dir
                        }));
                    }
                    Ok(fake_tool_result(json!({
                        "operation": "ListConnections",
                        "data": connections
                    })))
                }
                McpOperation::ReplacePartitionSource {
                    table_name,
                    partition_name,
                    expression,
                    ..
                } => {
                    self.enter("update")?;
                    self.expressions.insert(
                        (table_name.clone(), partition_name.clone()),
                        expression.clone(),
                    );
                    Ok(fake_tool_result(json!({})))
                }
                McpOperation::GetPartition {
                    table_name,
                    partition_name,
                    ..
                } => {
                    self.enter("get")?;
                    let expression = if self.readback_mismatch {
                        "let\n\tSource = #table({}, {})\nin\n\tSource".to_string()
                    } else {
                        self.expressions
                            .get(&(table_name.clone(), partition_name.clone()))
                            .expect("updated fake expression")
                            .clone()
                    };
                    Ok(fake_tool_result(json!({
                        "operation": "GET",
                        "message": "Retrieved one partition",
                        "results": [{
                            "index": 0,
                            "itemIdentifier": format!("{table_name}.{partition_name}"),
                            "message": "Retrieved partition",
                            "warnings": [],
                            "data": {
                                "annotations": [],
                                "attributes": "",
                                "dataView": "",
                                "description": "",
                                "errorMessage": "",
                                "name": partition_name,
                                "tableName": table_name,
                                "sourceType": "M",
                                "expression": expression,
                                "extendedProperties": [],
                                "mode": "import",
                                "modifiedTime": "2026-07-17T00:00:00Z",
                                "state": "Ready"
                            }
                        }],
                        "summary": {
                            "executionTime": "00:00:00.001",
                            "failureCount": 0,
                            "successCount": 1,
                            "totalItems": 1
                        },
                        "warnings": []
                    })))
                }
                McpOperation::ExportTmdlFolder { folder_path, .. } => {
                    self.enter("export")?;
                    match self.export_shape {
                        FakeExportShape::Valid => write_fake_tmdl_folder(folder_path),
                        FakeExportShape::RootTmdl => {
                            std::fs::write(folder_path.join("database.tmdl"), "database Unsafe")
                                .expect("root TMDL");
                        }
                    }
                    Ok(fake_tool_result(json!({
                        "operation": "ExportToTmdlFolder",
                        "data": {}
                    })))
                }
            }
        }
    }

    fn fake_tool_result(payload: Value) -> Value {
        let operation = payload
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or("Update");
        let (title, read_only) = match operation {
            "Connect" => ("connection_operations.connect", true),
            "ConnectFolder" => ("connection_operations.connectfolder", true),
            "ListConnections" => ("connection_operations.listconnections", true),
            "GET" | "Get" => ("partition_operations.get", true),
            "Update" => ("partition_operations.update", false),
            "ExportToTmdlFolder" => ("database_operations.exporttotmdlfolder", true),
            unexpected => panic!("fake tool payload has unsupported operation {unexpected}"),
        };
        json!({
            "_meta": {
                "annotations": {
                    "readOnlyHint": read_only,
                    "title": title
                }
            },
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&payload).expect("serialize fake payload")
            }],
            "isError": false
        })
    }

    #[test]
    fn update_response_requires_the_exact_pinned_empty_object() {
        assert_eq!(
            parse_tool_payload(&fake_tool_result(json!({})), "Update")
                .expect("exact beta.11 Update response"),
            json!({})
        );
        for drifted in [
            json!([]),
            json!({"operation": "Update"}),
            json!({"data": {}}),
        ] {
            assert!(parse_tool_payload(&fake_tool_result(drifted), "Update").is_err());
        }
        let mut extra_result_key = fake_tool_result(json!({}));
        extra_result_key["structuredContent"] = json!({});
        assert!(parse_tool_payload(&extra_result_key, "Update").is_err());
        let mut extra_content_key = fake_tool_result(json!({}));
        extra_content_key["content"][0]["annotations"] = json!({});
        assert!(parse_tool_payload(&extra_content_key, "Update").is_err());
        let mut missing_error_flag = fake_tool_result(json!({}));
        missing_error_flag
            .as_object_mut()
            .expect("tool result")
            .remove("isError");
        assert!(parse_tool_payload(&missing_error_flag, "Update").is_err());
        let mut missing_metadata = fake_tool_result(json!({}));
        missing_metadata
            .as_object_mut()
            .expect("tool result")
            .remove("_meta");
        assert!(parse_tool_payload(&missing_metadata, "Update").is_err());
        let mut extra_annotation = fake_tool_result(json!({}));
        extra_annotation["_meta"]["annotations"]["destructiveHint"] = json!(false);
        assert!(parse_tool_payload(&extra_annotation, "Update").is_err());
        let mut wrong_annotation = fake_tool_result(json!({}));
        wrong_annotation["_meta"]["annotations"]["readOnlyHint"] = json!(true);
        assert!(parse_tool_payload(&wrong_annotation, "Update").is_err());
    }

    #[test]
    fn partition_readback_requires_the_exact_closed_shape() {
        let valid = json!({
            "operation": "GET",
            "message": "Retrieved one partition",
            "results": [{
                "index": 0,
                "itemIdentifier": "Fact.Fact",
                "message": "Retrieved partition",
                "warnings": [],
                "data": {
                    "annotations": [],
                    "attributes": "",
                    "dataView": "",
                    "description": "",
                    "errorMessage": "",
                    "name": "Fact",
                    "tableName": "Fact",
                    "sourceType": "M",
                    "expression": "let Source = 1 in Source",
                    "extendedProperties": [],
                    "mode": "import",
                    "modifiedTime": "2026-07-17T00:00:00Z",
                    "state": "Ready"
                }
            }],
            "summary": {
                "executionTime": "00:00:00.001",
                "failureCount": 0,
                "successCount": 1,
                "totalItems": 1
            },
            "warnings": []
        });
        assert_eq!(
            exact_partition_readback(&valid, "Fact", "Fact").expect("closed Get response"),
            "let Source = 1 in Source"
        );

        let mut extra_result = valid.clone();
        extra_result["results"]
            .as_array_mut()
            .expect("results")
            .push(valid["results"][0].clone());
        let mut extra_payload_field = valid.clone();
        extra_payload_field["unexpected"] = json!(true);
        let mut extra_result_field = valid.clone();
        extra_result_field["results"][0]["unexpected"] = json!(true);
        let mut extra_data_field = valid.clone();
        extra_data_field["results"][0]["data"]["unexpected"] = json!(true);
        let mut extra_summary_field = valid.clone();
        extra_summary_field["summary"]["unexpected"] = json!(true);
        let mut warning = valid.clone();
        warning["warnings"] = json!(["drift"]);
        let mut wrong_count = valid.clone();
        wrong_count["summary"]["totalItems"] = json!(2);
        let mut wrong_operation_casing = valid.clone();
        wrong_operation_casing["operation"] = json!("Get");
        for drifted in [
            extra_result,
            extra_payload_field,
            extra_result_field,
            extra_data_field,
            extra_summary_field,
            warning,
            wrong_count,
            wrong_operation_casing,
        ] {
            assert!(exact_partition_readback(&drifted, "Fact", "Fact").is_err());
        }
    }

    fn write_fake_tmdl_folder(definition: &Path) {
        assert!(definition.is_dir(), "ordinary export definition target");
        std::fs::create_dir(definition.join("tables")).expect("export tables");
        std::fs::write(
            definition.join("database.tmdl"),
            "database Synthetic\n\tcompatibilityLevel: 1600\n",
        )
        .expect("export database");
        std::fs::write(
            definition.join("model.tmdl"),
            "model Model\n\tculture: en-US\n",
        )
        .expect("export model");
        std::fs::write(
            definition.join("tables").join("Synthetic.tmdl"),
            "table Synthetic\n",
        )
        .expect("export table");
    }

    fn copy_model_fixture(target: &Path) {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("conformance")
            .join("microsoft")
            .join("modeling-mcp")
            .join("Synthetic.SemanticModel");
        std::fs::create_dir_all(target.join("definition").join("tables"))
            .expect("model fixture directories");
        for relative in [
            Path::new("definition.pbism"),
            Path::new("definition").join("database.tmdl").as_path(),
            Path::new("definition").join("model.tmdl").as_path(),
            Path::new("definition")
                .join("tables")
                .join("Synthetic.tmdl")
                .as_path(),
        ] {
            let target_file = target.join(relative);
            if let Some(parent) = target_file.parent() {
                std::fs::create_dir_all(parent).expect("fixture parent");
            }
            std::fs::copy(fixture.join(relative), target_file).expect("copy model fixture");
        }
    }

    fn staged_request(temp: &tempfile::TempDir) -> StagedPartitionReplacementRequest {
        let source = temp.path().join("source");
        let stage = temp.path().join("stage").join("Synthetic.SemanticModel");
        let workflow = temp.path().join("workflow");
        copy_model_fixture(&source);
        copy_model_fixture(&stage);
        std::fs::create_dir(&workflow).expect("workflow directory");
        let expected = staged_partition_source_fingerprint(&stage, "Synthetic", "Synthetic")
            .expect("before fingerprint");
        StagedPartitionReplacementRequest {
            source_root: source,
            staged_semantic_model_root: stage,
            fresh_export_root: workflow.join("mcp-export"),
            workflow_root: workflow,
            replacements: vec![StagedPartitionReplacement {
                table: "Synthetic".to_string(),
                partition: "Synthetic".to_string(),
                expected_before_sha256: expected,
                complete_m_expression:
                    "let\n\tSource = #table(type table [Value = Int64.Type], {{2}})\nin\n\tSource"
                        .to_string(),
            }],
        }
    }

    fn run_fake(
        request: &StagedPartitionReplacementRequest,
        fake: &mut FakeModelClient,
    ) -> StagedModelResult {
        let prepared = PreparedModelRun::new(request, true).expect("prepared fake model run");
        let core = run_prepared_model(fake, &prepared);
        let cleanup = ModelCleanupEvidence {
            children_reaped: true,
            pumps_joined: true,
            forced: false,
        };
        let core = materialize_verified(&prepared, core);
        finish_model_run(&prepared, core, Some(cleanup))
    }

    #[test]
    fn staged_model_requires_consent_before_path_or_process_mutation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        assert!(!request.fresh_export_root.exists());
        let failure = match PreparedModelRun::new(&request, false) {
            Ok(_) => panic!("consent was not required"),
            Err(failure) => failure,
        };
        assert_eq!(failure.phase, "consent");
        assert!(!request.fresh_export_root.exists());
    }

    #[test]
    fn staged_model_rejects_credential_like_m_before_export_preparation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        request.replacements[0].complete_m_expression =
            "let apiKey = \"unique-secret-value\" in apiKey".to_string();
        let failure = match PreparedModelRun::new(&request, true) {
            Ok(_) => panic!("credential-like M expression was accepted"),
            Err(failure) => failure,
        };
        assert_eq!(failure.phase, "credential-scan");
        assert!(!request.fresh_export_root.exists());
    }

    #[test]
    fn late_preparation_failure_releases_export_reservation_for_retry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        let expected = request.replacements[0].expected_before_sha256.clone();
        request.replacements[0].expected_before_sha256 = sha256_bytes(b"drifted source");
        let failure = match PreparedModelRun::new(&request, true) {
            Ok(_) => panic!("drifted before fingerprint was accepted"),
            Err(failure) => failure,
        };
        assert_eq!(failure.phase, "prepare");
        assert!(!request.fresh_export_root.exists());
        assert!(
            !request
                .workflow_root
                .join(".mcp-export.powerbi-cli-quarantine")
                .exists()
        );

        request.replacements[0].expected_before_sha256 = expected;
        let prepared =
            PreparedModelRun::new(&request, true).expect("retry after late preparation failure");
        assert!(prepared.paths.export_root.join("definition").is_dir());
    }

    #[test]
    fn staged_model_write_error_never_echoes_complete_m() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let sentinel = "UNIQUE_COMPLETE_M_SENTINEL_SHOULD_NOT_ESCAPE";
        let mut fake = FakeModelClient::new(definition);
        fake.fail_at = Some("update");
        fake.failure_message = Some(format!("vendor echoed M: {sentinel}"));
        let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
            panic!("write error unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "write");
        assert!(!failure.error.message().contains(sentinel));
        assert!(failure.error.message().contains("vendorDetailSha256="));
    }

    #[test]
    fn staged_model_success_has_exact_order_and_only_native_source_materialization() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let result = run_fake(&request, &mut fake);
        assert_eq!(fake.calls, ["connect", "list", "update", "get", "export"]);
        let StagedModelResult::Succeeded(success) = result else {
            panic!("fake staged workflow did not succeed")
        };
        assert!(success.source.byte_identical);
        assert!(!success.stage_definition.byte_identical);
        assert_eq!(
            success.stage_definition.after_sha256,
            success.expected_stage_sha256
        );
        let [replacement] = success.replacements.as_slice() else {
            panic!("expected one replacement")
        };
        assert_eq!(replacement.requested_sha256, replacement.readback_sha256);
        assert_eq!(replacement.readback_sha256, replacement.materialized_sha256);
        let table = std::fs::read_to_string(
            request
                .staged_semantic_model_root
                .join("definition")
                .join("tables")
                .join("Synthetic.tmdl"),
        )
        .expect("materialized table");
        assert!(table.contains("{{2}}"));
        assert_eq!(
            table.lines().last(),
            Some("\t\tannotation PBI_NavigationStepName = Navigation")
        );
    }

    #[test]
    fn staged_model_failures_quarantine_export_and_leave_source_and_stage_identical() {
        let cases = [
            (
                Some("update"),
                McpFailureKind::Backend,
                false,
                FakeExportShape::Valid,
                "write",
            ),
            (
                None,
                McpFailureKind::Backend,
                true,
                FakeExportShape::Valid,
                "readback",
            ),
            (
                Some("get"),
                McpFailureKind::Cancelled,
                false,
                FakeExportShape::Valid,
                "readback",
            ),
            (
                Some("export"),
                McpFailureKind::Backend,
                false,
                FakeExportShape::Valid,
                "export",
            ),
            (
                None,
                McpFailureKind::Backend,
                false,
                FakeExportShape::RootTmdl,
                "export-proof",
            ),
        ];
        for (fail_at, fail_kind, mismatch, export_shape, expected_phase) in cases {
            let temp = tempfile::tempdir().expect("tempdir");
            let request = staged_request(&temp);
            let definition =
                std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                    .expect("canonical definition");
            let source_snapshot =
                SourceTreeSnapshot::capture(&request.source_root).expect("source snapshot");
            let stage_snapshot = SourceTreeSnapshot::capture(&definition).expect("stage snapshot");
            let mut fake = FakeModelClient::new(definition);
            fake.fail_at = fail_at;
            fake.fail_kind = fail_kind;
            fake.readback_mismatch = mismatch;
            fake.export_shape = export_shape;
            let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
                panic!("failure case {expected_phase} unexpectedly succeeded")
            };
            assert_eq!(failure.phase, expected_phase);
            assert!(
                source_snapshot
                    .verify()
                    .expect("source proof")
                    .byte_identical
            );
            assert!(stage_snapshot.verify().expect("stage proof").byte_identical);
            assert!(
                request
                    .fresh_export_root
                    .join(".powerbi-cli-failure-only")
                    .is_file()
            );
        }
    }

    #[test]
    fn staged_model_exact_tree_proof_rejects_unrelated_concurrent_change() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition.clone());
        let prepared = PreparedModelRun::new(&request, true).expect("prepared model run");
        let core = run_prepared_model(&mut fake, &prepared);
        std::fs::write(
            definition.join("model.tmdl"),
            "model Model\n\tculture: de-CH\n",
        )
        .expect("concurrent stage mutation");
        let core = materialize_verified(&prepared, core);
        let StagedModelResult::Failed(failure) = finish_model_run(&prepared, core, None) else {
            panic!("unrelated concurrent stage mutation escaped the exact tree proof")
        };
        assert_eq!(failure.phase, "post-cleanup-stage-proof");
        assert!(
            prepared
                .source_snapshot
                .verify()
                .expect("source proof")
                .byte_identical
        );
        assert!(
            !prepared
                .stage_snapshot
                .verify()
                .expect("stage proof")
                .byte_identical
        );
    }

    #[test]
    fn staged_model_rechecks_source_before_the_first_native_write() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let prepared = PreparedModelRun::new(&request, true).expect("prepared model run");
        let core = run_prepared_model(&mut fake, &prepared);
        std::fs::write(
            request.source_root.join("definition").join("model.tmdl"),
            "model Model\n\tculture: de-CH\n",
        )
        .expect("concurrent source mutation");
        let CoreModelOutcome::Failed(failure) = materialize_verified(&prepared, core) else {
            panic!("concurrent source mutation escaped the pre-write proof")
        };
        assert_eq!(failure.phase, "post-cleanup-source-proof");
        assert!(
            prepared
                .stage_snapshot
                .verify()
                .expect("stage proof")
                .byte_identical,
            "source drift must be rejected before the first staged write"
        );
    }

    #[test]
    fn staged_model_exact_tree_proof_supports_two_distinct_tmdl_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        let tables = request
            .staged_semantic_model_root
            .join("definition")
            .join("tables");
        let synthetic =
            std::fs::read_to_string(tables.join("Synthetic.tmdl")).expect("synthetic table");
        std::fs::write(
            tables.join("Other.tmdl"),
            synthetic.replace("Synthetic", "Other"),
        )
        .expect("second table");
        let expected = staged_partition_source_fingerprint(
            &request.staged_semantic_model_root,
            "Other",
            "Other",
        )
        .expect("second before fingerprint");
        request.replacements.push(StagedPartitionReplacement {
            table: "Other".to_string(),
            partition: "Other".to_string(),
            expected_before_sha256: expected,
            complete_m_expression:
                "let\n\tSource = #table(type table [Value = Int64.Type], {{3}})\nin\n\tSource"
                    .to_string(),
        });
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let StagedModelResult::Succeeded(success) = run_fake(&request, &mut fake) else {
            panic!("two-file staged workflow did not succeed")
        };
        assert_eq!(success.replacements.len(), 2);
        assert_eq!(
            success.stage_definition.after_sha256,
            success.expected_stage_sha256
        );
        assert!(
            std::fs::read_to_string(tables.join("Synthetic.tmdl"))
                .expect("synthetic after")
                .contains("{{2}}")
        );
        assert!(
            std::fs::read_to_string(tables.join("Other.tmdl"))
                .expect("other after")
                .contains("{{3}}")
        );
    }

    #[test]
    fn staged_model_composes_two_replacements_in_one_tmdl_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut request = staged_request(&temp);
        let table_path = request
            .staged_semantic_model_root
            .join("definition")
            .join("tables")
            .join("Synthetic.tmdl");
        let mut table = std::fs::read_to_string(&table_path).expect("synthetic table");
        table.push_str(
            "\n\tpartition Other = m\n\t\tmode: import\n\t\tsource =\n\t\t\tlet\n\t\t\t\tSource = #table(type table [Value = Int64.Type], {{10}})\n\t\t\tin\n\t\t\t\tSource\n",
        );
        std::fs::write(&table_path, table).expect("second same-file partition");
        let expected = staged_partition_source_fingerprint(
            &request.staged_semantic_model_root,
            "Synthetic",
            "Other",
        )
        .expect("second before fingerprint");
        request.replacements.push(StagedPartitionReplacement {
            table: "Synthetic".to_string(),
            partition: "Other".to_string(),
            expected_before_sha256: expected,
            complete_m_expression:
                "let\n\tSource = #table(type table [Value = Int64.Type], {{3}})\nin\n\tSource"
                    .to_string(),
        });
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        let StagedModelResult::Succeeded(success) = run_fake(&request, &mut fake) else {
            panic!("same-file staged workflow did not succeed")
        };
        assert_eq!(success.replacements.len(), 2);
        let materialized = std::fs::read_to_string(table_path).expect("materialized table");
        assert!(materialized.contains("{{2}}"));
        assert!(materialized.contains("{{3}}"));
        assert!(!materialized.contains("{{10}}"));
    }

    #[test]
    fn staged_model_rechecks_fresh_export_and_exact_connection_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let prepared = PreparedModelRun::new(&request, true).expect("prepared model run");
        std::fs::write(request.fresh_export_root.join("intruder.txt"), "occupied")
            .expect("occupy export after preparation");
        let mut fake = FakeModelClient::new(prepared.paths.definition_dir.clone());
        let core = run_prepared_model(&mut fake, &prepared);
        let StagedModelResult::Failed(failure) = finish_model_run(&prepared, core, None) else {
            panic!("occupied export unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "export-guard");
        assert!(!fake.calls.contains(&"export"));

        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        fake.duplicate_connection = true;
        let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
            panic!("duplicate connection unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "connection");

        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let definition =
            std::fs::canonicalize(request.staged_semantic_model_root.join("definition"))
                .expect("canonical definition");
        let mut fake = FakeModelClient::new(definition);
        fake.duplicate_path_different_name = true;
        let StagedModelResult::Failed(failure) = run_fake(&request, &mut fake) else {
            panic!("duplicate canonical path with another name unexpectedly succeeded")
        };
        assert_eq!(failure.phase, "connection");
    }

    #[test]
    #[ignore = "requires the exact installed Microsoft Modeling MCP package"]
    fn exact_real_disposable_offline_workflow() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = staged_request(&temp);
        let tool = crate::microsoft::resolve_installed_component(MicrosoftComponent::ModelingMcp)
            .expect("installed Modeling MCP");
        let success = match execute_staged_partition_replacements(&tool, &request, true) {
            StagedModelResult::Succeeded(success) => success,
            StagedModelResult::Failed(failure) => panic!(
                "exact offline workflow failed at {}: {}",
                failure.phase,
                failure.error.message()
            ),
        };
        assert!(success.cleanup.children_reaped);
        assert!(success.cleanup.pumps_joined);
        assert!(success.source.byte_identical);
        assert_eq!(
            success.stage_definition.after_sha256,
            success.expected_stage_sha256
        );
        assert_eq!(
            success.export.export_root,
            std::fs::canonicalize(&request.fresh_export_root).expect("canonical export")
        );
        assert!(success.export.file_count >= 3);
        assert!(
            request
                .fresh_export_root
                .join("definition")
                .join("database.tmdl")
                .is_file()
        );
        assert!(
            !request
                .fresh_export_root
                .join(".powerbi-cli-failure-only")
                .exists()
        );
    }
}
