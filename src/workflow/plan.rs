//! Workflow planning command, profile validation, and selected-source manifest construction.

use super::*;

pub(super) fn workflow_plan(args: &[String]) -> CliResult<Value> {
    let options = parse_plan_args(args)?;
    let plan_path = resolve_new_file_candidate(&options.plan_path, "workflow plan")?;
    let output_dir = resolve_new_directory_candidate(&options.output_dir)?;
    validate_credential_free_path(&plan_path, "workflow plan")?;
    validate_credential_free_path(&output_dir, "workflow output")?;
    let output_dir_text = unicode_path(&output_dir, "workflow output")?;
    let resolved = resolve_project(&options.project)?;
    let project_root = canonical_plain_directory(&resolved.project_dir, "project root")?;
    validate_credential_free_path(&project_root, "project root")?;
    if plan_path.starts_with(&project_root) {
        return Err(CliError::invalid_args(
            "workflow plan file must be outside the entire source project root",
        ));
    }
    if paths_overlap(&project_root, &output_dir) {
        return Err(CliError::invalid_args(
            "workflow output must not overlap the source project",
        ));
    }
    let selected_pbip = fs::canonicalize(&resolved.pbip_path)
        .map_err(|error| CliError::unexpected(format!("resolve selected PBIP: {error}")))?;
    let selected_report = fs::canonicalize(&resolved.report_dir)
        .map_err(|error| CliError::unexpected(format!("resolve selected Report: {error}")))?;
    let selected_model = fs::canonicalize(&resolved.semantic_model_dir).map_err(|error| {
        CliError::unexpected(format!("resolve selected SemanticModel: {error}"))
    })?;
    if plan_path == selected_pbip
        || plan_path.starts_with(&selected_report)
        || plan_path.starts_with(&selected_model)
    {
        return Err(CliError::invalid_args(
            "workflow plan file must be outside the selected PBIP artifact closure",
        ));
    }
    let profile_path = canonical_plain_file(&options.profile, "source profile", MAX_PROFILE_BYTES)?;
    validate_credential_free_path(&profile_path, "source profile")?;
    let profile_bytes = read_bounded(&profile_path, MAX_PROFILE_BYTES, "source profile")?;
    let profile_text = std::str::from_utf8(&profile_bytes)
        .map_err(|_| CliError::validation_failed("source profile must be UTF-8 JSON"))?;
    if contains_credential_like_text_str(profile_text) {
        return Err(CliError::validation_failed(
            "source profile contains credential-like content",
        ));
    }
    let profile: SourceProfile = serde_json::from_slice(&profile_bytes).map_err(|error| {
        CliError::validation_failed(format!(
            "parse source profile {}: {error}",
            profile_path.display()
        ))
    })?;
    validate_profile_shape(&profile)?;
    let profile_dir = profile_path.parent().expect("canonical file has parent");
    let resources = resolve_profile_resources(&profile, profile_dir, &options.resources)?;
    let templates = resolve_profile_templates(&profile, profile_dir)?;
    let source = source_manifest(&resolved, &project_root)?;

    for replacement in &profile.replacements {
        let actual = staged_partition_source_fingerprint(
            &resolved.semantic_model_dir,
            &replacement.table,
            &replacement.partition,
        )
        .map_err(|failure| CliError::validation_failed(failure.message().to_string()))?;
        if actual != replacement.expected_before_sha256 {
            return Err(CliError::validation_failed(format!(
                "partition source drift for {}.{}: expected {}, found {}",
                replacement.table,
                replacement.partition,
                replacement.expected_before_sha256,
                actual
            )));
        }
        let template = templates
            .get(&replacement.template)
            .ok_or_else(|| CliError::validation_failed("resolved template is missing"))?;
        let text = read_utf8_claim(template, MAX_TEMPLATE_BYTES, "M template")?;
        validate_template(&text, replacement)?;
    }

    let mut plan = WorkflowPlan {
        schema: WORKFLOW_PLAN_SCHEMA.to_string(),
        plan_fingerprint: String::new(),
        policy: WORKFLOW_POLICY.to_string(),
        profile_id: profile.profile_id.clone(),
        profile: claim_for_file(&profile_path, MAX_PROFILE_BYTES)?,
        source,
        templates,
        resources,
        replacements: profile
            .replacements
            .iter()
            .map(|item| PlannedReplacement {
                table: item.table.clone(),
                partition: item.partition.clone(),
                expected_before_sha256: item.expected_before_sha256.clone(),
                template: item.template.clone(),
                expected_connector: item.expected_connector.clone(),
                resources: item.resources.clone(),
            })
            .collect(),
        integration_lock_sha256: sha256_bytes(INTEGRATION_LOCK_BYTES),
        output_dir: output_dir_text,
    };
    plan.plan_fingerprint = plan_fingerprint(&plan)?;
    write_json_new_atomic(
        &plan_path,
        &serde_json::to_value(&plan).map_err(json_serialize_error)?,
    )?;
    Ok(json!({
        "schema": WORKFLOW_PLAN_SCHEMA,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "profileId": plan.profile_id,
        "plan": canonical_display(&plan_path),
        "planFingerprint": plan.plan_fingerprint,
        "selectedFiles": plan.source.files.len(),
        "resources": plan.resources.len(),
        "replacements": plan.replacements.len(),
        "outputDir": plan.output_dir,
        "next": [format!("powerbi-cli workflow run --plan {} --confirm {} --json", plan_path.display(), plan.plan_fingerprint)]
    }))
}

#[derive(Debug)]
struct PlanOptions {
    project: PathBuf,
    profile: PathBuf,
    plan_path: PathBuf,
    output_dir: PathBuf,
    resources: BTreeMap<String, PathBuf>,
}

fn parse_plan_args(args: &[String]) -> CliResult<PlanOptions> {
    let mut project = None;
    let mut profile = None;
    let mut plan_path = None;
    let mut output_dir = None;
    let mut resources = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
        match flag.as_str() {
            "--project" => set_once(&mut project, PathBuf::from(value), flag)?,
            "--profile" => set_once(&mut profile, PathBuf::from(value), flag)?,
            "--out" => set_once(&mut plan_path, PathBuf::from(value), flag)?,
            "--out-dir" => set_once(&mut output_dir, PathBuf::from(value), flag)?,
            "--resource" => {
                let (name, path) = value
                    .split_once('=')
                    .ok_or_else(|| CliError::invalid_args("--resource must use name=path"))?;
                validate_name(name, "resource")?;
                if resources
                    .insert(name.to_string(), PathBuf::from(path))
                    .is_some()
                {
                    return Err(CliError::invalid_args(format!(
                        "duplicate --resource override: {name}"
                    )));
                }
            }
            _ => {
                return Err(CliError::invalid_args(format!(
                    "unknown workflow plan flag: {flag}"
                )));
            }
        }
        index += 2;
    }
    Ok(PlanOptions {
        project: project
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --project"))?,
        profile: profile
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --profile"))?,
        plan_path: plan_path
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --out"))?,
        output_dir: output_dir
            .ok_or_else(|| CliError::invalid_args("workflow plan requires --out-dir"))?,
        resources,
    })
}
