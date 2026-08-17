//! Workflow execution command and run-only staged-copy orchestration.

use super::shared::*;
use super::*;

pub(super) fn workflow_run(args: &[String]) -> CliResult<Value> {
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

pub(super) fn copy_claimed_files(
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

pub(super) fn copy_resources(plan: &WorkflowPlan, output: &OwnedWorkflowOutput) -> CliResult<()> {
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
