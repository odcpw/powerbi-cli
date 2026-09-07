//! In-process composition of the existing profile, planner, compiler and readbacks.
use crate::cli_support::{MutationMode, require_mode, set_mode, take_report_value};
use crate::input_safety::{InputKind, read_utf8};
use crate::ops::{SnapshotOptions, Transaction};
use crate::project_io::write_json_pretty;
use crate::{CliError, CliResult, canonical_display, command_arg, resolve_project};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Default)]
struct Options {
    schema: Option<String>,
    rows: Option<String>,
    profile: Option<String>,
    intent: Option<String>,
    objective: Option<String>,
    style: Option<String>,
    output: Option<PathBuf>,
    project: Option<PathBuf>,
    artifacts: Option<PathBuf>,
    confirm: Option<String>,
    mode: Option<MutationMode>,
}

pub(crate) fn compose_command(args: &[String]) -> CliResult<Value> {
    let options = parse(args)?;
    let mode = require_mode(options.mode, "report compose")?;
    let schema = options.schema.as_deref().ok_or_else(|| {
        crate::report_build::spec_missing_input_with_command(
            "/schema",
            "schema",
            "compose requires a schema manifest",
            json!({"--schema":"<schema.json>"}),
            "powerbi-cli schema validate <schema.json> --json",
        )
    })?;
    let existing = options
        .project
        .as_deref()
        .map(resolve_project)
        .transpose()?;
    if mode == MutationMode::InPlace && existing.is_none() {
        return Err(invalid(
            "--in-place requires --project <project-dir-or.pbip>",
        ));
    }
    if mode == MutationMode::OutDir && existing.is_some() {
        return Err(invalid(
            "--project is only used with --dry-run or --in-place",
        ));
    }
    let target = options
        .output
        .clone()
        .or_else(|| existing.as_ref().map(|project| project.project_dir.clone()));
    let confirm = existing
        .as_ref()
        .map(|project| format!("compose:{}", canonical_display(&project.project_dir)));
    if mode == MutationMode::InPlace && options.confirm != confirm {
        return Err(invalid("in-place composition replaces the project; supply the confirmation token from --dry-run")
            .with_hint(format!("Required --confirm token: {}", confirm.as_deref().unwrap())));
    }
    if mode == MutationMode::OutDir {
        require_absent(
            target
                .as_deref()
                .ok_or_else(|| invalid("--out-dir requires a path"))?,
        )?;
    }
    let artifacts = options.artifacts.clone().or_else(|| {
        target
            .as_ref()
            .map(|path| PathBuf::from(format!("{}-compose", path.display())))
    });
    if mode != MutationMode::DryRun {
        require_absent(artifacts.as_deref().expect("write target"))?;
    }
    if let (Some(target), Some(artifacts)) = (&target, &artifacts) {
        let target = resolved_output_path(target)?;
        let artifacts = resolved_output_path(artifacts)?;
        if target.starts_with(&artifacts) || artifacts.starts_with(&target) {
            return Err(invalid(
                "project and compose artifacts must be separate, non-nested directories",
            ));
        }
    }
    // The existing build and readback stages run against an invocation-owned
    // temporary project even in dry-run mode. Nothing is published until success.
    let mut work =
        Some(tempfile::tempdir().map_err(|error| CliError::unexpected(error.to_string()))?);
    let work_root = work
        .as_ref()
        .expect("working directory")
        .path()
        .to_path_buf();
    let sidecar = if mode == MutationMode::DryRun {
        work_root.join("artifacts")
    } else {
        artifacts.clone().expect("artifacts")
    };
    if let Some(parent) = sidecar.parent() {
        std::fs::create_dir_all(parent).map_err(|error| CliError::unexpected(error.to_string()))?;
    }
    std::fs::create_dir(&sidecar).map_err(|error| CliError::unexpected(error.to_string()))?;
    let project = work_root.join("project");
    let profile_path = sidecar.join("profile.json");
    let spec_path = sidecar.join("spec.json");
    let mut stages = Vec::new();
    let mut documents = Vec::new();
    let profile = if let Some(path) = &options.profile {
        let start = Instant::now();
        let result = crate::profile::profile_command(&["validate".into(), path.clone()])?;
        if result["ok"] == false {
            return Ok(result);
        }
        stage(&mut stages, "profile validate", start, "profile.json");
        crate::profile::load_profile_value(Path::new(path))?
    } else {
        let mut args = vec!["infer".into(), "--schema".into(), schema.into()];
        if let Some(rows) = &options.rows {
            args.extend(["--rows".into(), rows.clone()]);
        }
        let start = Instant::now();
        let result = crate::profile::profile_command(&args)?;
        stage(&mut stages, "profile infer", start, "profile.json");
        result["profile"].clone()
    };
    record(&sidecar, &mut documents, "profile.json", profile)?;
    let mut args = vec![
        "--schema".into(),
        schema.into(),
        "--profile".into(),
        path(&profile_path),
    ];
    if let Some(intent) = &options.intent {
        args.extend(["--intent".into(), intent.clone()]);
    }
    if let Some(objective) = &options.objective {
        args.extend(["--objective".into(), objective.clone()]);
    }
    let start = Instant::now();
    let plan = crate::report_plan::plan_command(&args)?;
    stage(&mut stages, "report plan", start, "plan.json");
    let mut spec = plan["specV2"].clone();
    if let Some(style) = &options.style {
        spec["style"] = if style.ends_with(".json") {
            let value: Value =
                serde_json::from_str(&read_utf8(Path::new(style), InputKind::JsonArtifact)?)
                    .map_err(|error| invalid(&format!("invalid style JSON: {error}")))?;
            json!({"tokens":value})
        } else {
            json!({"tokens":{"preset":style}})
        };
    }
    record(&sidecar, &mut documents, "plan.json", plan.clone())?;
    record(&sidecar, &mut documents, "spec.json", spec)?;
    let args = vec![
        "--schema".into(),
        schema.into(),
        "--profile".into(),
        path(&profile_path),
        "--spec".into(),
        path(&spec_path),
    ];
    let start = Instant::now();
    let mut validate_args = vec!["validate".into()];
    validate_args.extend(args.clone());
    let validation = crate::report_build::spec_command(&validate_args)?;
    // Value-returning validators retain their native failure document unchanged.
    if validation["ok"] == false {
        return Ok(validation);
    }
    stage(&mut stages, "report spec validate", start, "validate.json");
    record(&sidecar, &mut documents, "validate.json", validation)?;
    let start = Instant::now();
    let explain = crate::report_spec_explain::explain_command(&args)?;
    stage(&mut stages, "report spec explain", start, "explain.json");
    record(
        &sidecar,
        &mut documents,
        "ops.json",
        explain["plan"].clone(),
    )?;
    record(&sidecar, &mut documents, "explain.json", explain)?;
    let start = Instant::now();
    let mut build_args = args;
    build_args.extend(["--out-dir".into(), path(&project)]);
    let build = retain_failed_work(
        crate::report_build::build_command(&build_args),
        &mut work,
        mode,
    )?;
    if build["ok"] == false {
        return Ok(build);
    }
    stage(&mut stages, "report build", start, "build.json");
    record(&sidecar, &mut documents, "build.json", build.clone())?;
    let start = Instant::now();
    let triage = retain_failed_work(
        crate::triage::triage_command(&[path(&project)]),
        &mut work,
        mode,
    )?;
    if triage["ok"] == false {
        return Ok(triage);
    }
    stage(&mut stages, "triage", start, "triage.json");
    record(&sidecar, &mut documents, "triage.json", triage.clone())?;
    let start = Instant::now();
    let wireframes = retain_failed_work(
        crate::report_wireframe::wireframe_export(&[
            path(&project),
            "--format".into(),
            "svg".into(),
            "--out".into(),
            path(&sidecar.join("wireframes")),
        ]),
        &mut work,
        mode,
    )?;
    if wireframes["ok"] == false {
        return Ok(wireframes);
    }
    stage(
        &mut stages,
        "report wireframe export",
        start,
        "wireframes.json",
    );
    record(
        &sidecar,
        &mut documents,
        "wireframes.json",
        wireframes.clone(),
    )?;
    record(
        &sidecar,
        &mut documents,
        "scorecard.json",
        triage["scorecard"].clone(),
    )?;
    let display_target = target
        .as_deref()
        .map(canonical_display)
        .unwrap_or("<project-dir>".into());
    let display_artifacts = artifacts
        .as_deref()
        .map(canonical_display)
        .unwrap_or("<project-dir>-compose".into());
    // Artifacts render the canonical spelling while arguments may carry the
    // raw one (Windows 8.3 short names, symlinked temp roots); redact both so
    // dry-run responses never leak a scratch path.
    let replacements = [
        (canonical_display(&sidecar), display_artifacts.clone()),
        (path(&sidecar), display_artifacts.clone()),
        (canonical_display(&project), display_target.clone()),
        (path(&project), display_target.clone()),
    ];
    for (name, document) in &mut documents {
        remap(document, &replacements);
        write_json_pretty(&sidecar.join(name), document)?;
    }
    let mut snapshot = None;
    if mode != MutationMode::DryRun {
        let mut transaction = Transaction::begin(resolve_project(&project)?)?;
        if let Some(source) = existing {
            transaction.source = source;
            snapshot = transaction
                .commit_in_place(SnapshotOptions::default())?
                .snapshot_dir;
        } else {
            transaction.commit_out_dir(target.as_deref().expect("target"), false)?;
        }
    }
    let mut response = json!({
        "schema":"powerbi-cli.report.compose.v1", "ok":true, "exitCode":0,
        "dryRun":mode == MutationMode::DryRun, "changed":mode != MutationMode::DryRun,
        "projectDir":display_target, "artifactsDir":display_artifacts,
        "stages":stages, "decisions":plan["decisions"], "scorecard":triage["scorecard"],
        "proofPlan":build["proofPlan"], "wireframes":wireframes["artifacts"],
        "confirmToken":confirm, "snapshotDir":snapshot.map(|p| canonical_display(&p)),
        "artifacts":documents.iter().map(|(name,value)| (name.clone(),value.clone())).collect::<serde_json::Map<_,_>>(),
        "next":[format!("powerbi-cli validate {} --json", command_arg(Path::new(&display_target))),
            format!("powerbi-cli triage {} --json", command_arg(Path::new(&display_target)))]
    });
    remap(&mut response, &replacements);
    Ok(response)
}

fn path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
// Native build/readback failures can contain commands addressing the staged
// project. Keep that invocation-owned working tree in write mode so those
// commands still work; successful runs and dry-runs clean it automatically.
fn retain_failed_work(
    result: CliResult<Value>,
    work: &mut Option<tempfile::TempDir>,
    mode: MutationMode,
) -> CliResult<Value> {
    if mode != MutationMode::DryRun
        && !result.as_ref().is_ok_and(|value| value["ok"] != false)
        && let Some(directory) = work.take()
    {
        let _retained_path = directory.keep();
    }
    result
}
fn invalid(message: &str) -> CliError {
    CliError::invalid_args(message)
        .with_hint("Inspect report compose capabilities; preview with --dry-run and choose fresh, separate output and artifact directories.")
        .with_suggested_command("powerbi-cli capabilities --for 'report compose' --json")
}
fn require_absent(path: &Path) -> CliResult<()> {
    if path.symlink_metadata().is_ok() {
        return Err(invalid(&format!(
            "compose output already exists: {}; choose a new output/artifacts directory",
            path.display()
        )));
    }
    Ok(())
}
fn resolved_output_path(path: &Path) -> CliResult<PathBuf> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|error| CliError::unexpected(error.to_string()));
    }
    let name = path
        .file_name()
        .ok_or_else(|| invalid("output must name a directory"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(resolved_output_path(parent)?.join(name))
}
fn stage(stages: &mut Vec<Value>, name: &str, start: Instant, artifact: &str) {
    stages
        .push(json!({"name":name,"ok":true,"ms":start.elapsed().as_millis(),"artifact":artifact}));
}
fn record(
    root: &Path,
    documents: &mut Vec<(String, Value)>,
    name: &str,
    value: Value,
) -> CliResult<()> {
    write_json_pretty(&root.join(name), &value)?;
    documents.push((name.into(), value));
    Ok(())
}
fn remap(value: &mut Value, replacements: &[(String, String)]) {
    match value {
        Value::String(text) => {
            for (from, to) in replacements {
                *text = text.replace(from, to);
            }
        }
        Value::Array(items) => {
            for item in items {
                remap(item, replacements);
            }
        }
        Value::Object(items) => {
            for item in items.values_mut() {
                remap(item, replacements);
            }
        }
        _ => {}
    }
}
fn parse(args: &[String]) -> CliResult<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--dry-run" | "--in-place" => {
                set_mode(
                    &mut options.mode,
                    if flag == "--dry-run" {
                        MutationMode::DryRun
                    } else {
                        MutationMode::InPlace
                    },
                    "report compose",
                )?;
                index += 1;
            }
            "--schema" | "--rows" | "--profile" | "--intent" | "--objective" | "--style"
            | "--out-dir" | "--project" | "--artifacts-dir" | "--confirm" | "--template-set"
            | "--variants" => {
                let value = take_report_value(args, &mut index, flag)?;
                match flag {
                    "--schema" => options.schema = Some(value),
                    "--rows" => options.rows = Some(value),
                    "--profile" => options.profile = Some(value),
                    "--intent" => options.intent = Some(value),
                    "--objective" => options.objective = Some(value),
                    "--style" => options.style = Some(value),
                    "--project" => options.project = Some(value.into()),
                    "--artifacts-dir" => options.artifacts = Some(value.into()),
                    "--confirm" => options.confirm = Some(value),
                    "--out-dir" => {
                        set_mode(&mut options.mode, MutationMode::OutDir, "report compose")?;
                        options.output = Some(value.into());
                    }
                    "--template-set" if value == "default" => {}
                    "--variants" if value == "1" => {}
                    _ => {
                        return Err(CliError::unsupported_feature(
                            "compose supports template-set default and one deterministic variant",
                        )
                        .with_hint("Use --template-set default and --variants 1; additional composition variants are not compiled.")
                        .with_suggested_command(
                            "powerbi-cli capabilities --for 'report compose' --json",
                        ));
                    }
                }
            }
            _ => return Err(invalid(&format!("unknown report compose flag: {flag}"))),
        }
    }
    if options.rows.is_some() && options.profile.is_some() {
        return Err(invalid(
            "choose --rows for inference or an existing --profile, not both",
        ));
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_failures_remain_unchanged_and_keep_write_mode_recovery_paths() {
        let parent = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir_in(parent.path()).unwrap();
        let retained = directory.path().to_path_buf();
        let mut work = Some(directory);
        let native = json!({"ok":false,"exitCode":10,"errors":[{"code":"unsupported_feature"}]});
        assert_eq!(
            retain_failed_work(Ok(native.clone()), &mut work, MutationMode::OutDir).unwrap(),
            native
        );
        assert!(work.is_none());
        assert!(retained.exists());
        let mut dry_work = Some(tempfile::tempdir_in(parent.path()).unwrap());
        let dry_path = dry_work.as_ref().unwrap().path().to_path_buf();
        let error = retain_failed_work(
            Err(CliError::invalid_args("native failure")),
            &mut dry_work,
            MutationMode::DryRun,
        )
        .unwrap_err();
        assert_eq!(error.message, "native failure");
        drop(dry_work);
        assert!(!dry_path.exists());
    }

    #[test]
    fn output_resolution_detects_nested_paths_through_dot_segments() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("existing")).unwrap();
        assert_eq!(
            resolved_output_path(&root.path().join("existing/../new")).unwrap(),
            root.path().canonicalize().unwrap().join("new")
        );
    }
}
