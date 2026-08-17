//! Workflow verification command for checking receipts and staged output evidence.

use super::*;

fn parse_verify_args(args: &[String]) -> CliResult<PathBuf> {
    let mut plan = None;
    parse_pairs(args, |flag, value| match flag {
        "--plan" => set_once(&mut plan, PathBuf::from(value), flag),
        _ => Err(CliError::invalid_args(format!(
            "unknown workflow verify flag: {flag}"
        ))),
    })?;
    plan.ok_or_else(|| CliError::invalid_args("workflow verify requires --plan"))
}

pub(super) fn workflow_verify(args: &[String]) -> CliResult<Value> {
    let plan_path = parse_verify_args(args)?;
    let plan = load_plan(&plan_path)?;
    verify_plan_inputs(&plan)?;
    let output_dir = canonical_plain_directory(Path::new(&plan.output_dir), "workflow output")?;
    let incomplete = output_dir.join(WORKFLOW_INCOMPLETE_FILE);
    match fs::symlink_metadata(&incomplete) {
        Ok(metadata) if metadata_is_link_or_reparse(&metadata) => {
            return Err(CliError::validation_failed(
                "workflow incomplete marker is a link or reparse point",
            ));
        }
        Ok(_) => {
            return Err(CliError::validation_failed(
                "workflow output is marked incomplete",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::unexpected(format!(
                "inspect workflow incomplete marker: {error}"
            )));
        }
    }
    let receipt_path = output_dir.join(WORKFLOW_RECEIPT_FILE);
    let receipt: WorkflowReceipt =
        read_json_bounded(&receipt_path, MAX_PROFILE_BYTES, "workflow receipt")?;
    if receipt.schema != WORKFLOW_RECEIPT_SCHEMA
        || receipt.receipt_checksum != receipt_checksum(&receipt)?
        || receipt.plan_fingerprint != plan.plan_fingerprint
        || receipt.source_closure_sha256 != plan.source.closure_sha256
    {
        return Err(CliError::validation_failed(
            "workflow receipt identity or checksum does not match the plan",
        ));
    }
    validate_receipt_claims(&plan, &receipt, &output_dir)?;
    let output_hash = hash_workflow_output(&output_dir)?.sha256;
    if receipt.output_tree_sha256 != output_hash {
        return Err(CliError::validation_failed(
            "workflow output hash does not match the receipt claim",
        ));
    }
    let staged_pbip = output_dir.join(&plan.source.pbip_relative);
    let validation = validate_command(&[
        "--strict".to_string(),
        "--backend".to_string(),
        "all".to_string(),
        staged_pbip.to_string_lossy().into_owned(),
    ])?;
    let validation_now = validation_claim(&validation)?;
    if validation_now != receipt.validation {
        return Err(CliError::validation_failed(
            "workflow validation evidence drifted from the receipt claim",
        ));
    }
    let output_after_validation = hash_workflow_output(&output_dir)?.sha256;
    if output_after_validation != output_hash {
        return Err(CliError::validation_failed(
            "a validation backend changed the workflow output during verification",
        ));
    }
    Ok(json!({
        "schema": "powerbi-cli.workflow-verify.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "planFingerprint": plan.plan_fingerprint,
        "receiptChecksum": receipt.receipt_checksum,
        "outputTreeSha256": output_hash,
        "validation": validation_now,
        "sourceInputsUnchanged": true,
        "receiptClaimsValid": true,
        "evidenceClaimsValid": true
    }))
}
