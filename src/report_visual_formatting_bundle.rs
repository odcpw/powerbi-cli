use crate::pbir::{VisualRecord, VisualSelector, find_visual, load_report_snapshot, visual_detail};
use crate::project_io::{copy_project_dir, write_json_atomic, write_json_pretty};
use crate::report_visual_formatting::formatting_summary_from_visual_json;
use crate::safety_scan::{CREDENTIAL_NEEDLES, SafetyScan, formatting_safety};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const VISUAL_FORMATTING_BUNDLE_SCHEMA: &str = "powerbi-cli.report.visuals.formatting-bundle.v1";

#[derive(Debug, Default)]
struct ExtractOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    out: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MutationMode {
    DryRun,
    InPlace,
    OutDir,
}

#[derive(Debug, Default)]
struct ApplyOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    bundle: Option<PathBuf>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
    allow_literal_text: bool,
    allow_cross_type: bool,
}

#[derive(Debug, Clone)]
struct FormattingPayload {
    visual_objects: Value,
    top_level_objects: Value,
}

pub(crate) fn extract_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_extract_args(args)?;
    let project = required_project(options.project, "report visuals formatting extract")?;
    require_visual_selector(&options.selector, "report visuals formatting extract")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals formatting extract",
    )?;
    let visual_path = visual_path(visual, "report visuals formatting extract")?;
    let visual_json = read_json_value(visual_path)?;
    let payload = formatting_payload_from_visual_json(&visual_json);
    if payload_is_empty(&payload) {
        return Err(
            CliError::invalid_args("source visual has no formatting objects to extract")
                .with_hint(
                    "Run formatting list to find visuals with formatObjectContainerCount > 0.",
                )
                .with_suggested_command(format!(
                    "powerbi-cli report visuals formatting list --project {} --json",
                    command_arg(&resolved.project_dir)
                )),
        );
    }
    let summary = formatting_summary_from_visual_json(&visual_json, false);
    let safety = payload_safety_json(&payload, &summary);
    let bundle = json!({
        "schema": VISUAL_FORMATTING_BUNDLE_SCHEMA,
        "bundleVersion": 1,
        "sourceFingerprint": fingerprint_value(&payload.to_json()),
        "source": {
            "projectDir": canonical_display(&resolved.project_dir),
            "pbip": canonical_display(&resolved.pbip_path),
            "reportDir": canonical_display(&resolved.report_dir),
            "visual": visual_detail(visual)
        },
        "formatting": payload.to_json(),
        "summary": summary,
        "safety": safety
    });
    if let Some(out) = &options.out {
        write_json_pretty(out, &bundle)?;
    }

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.formatting.extract.v1",
        "ok": true,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "out": options.out.as_ref().map(|path| canonical_display(path)),
        "source": {
            "visual": visual_detail(visual),
            "path": canonical_display(visual_path),
            "fingerprint": bundle["sourceFingerprint"].clone()
        },
        "bundle": bundle,
        "next": [
            "powerbi-cli report visuals formatting apply --project <target-project> --handle <target-visual-handle> --bundle <formatting-bundle.json> --dry-run --json",
            "powerbi-cli report visuals formatting show --project <target-project> --handle <target-visual-handle> --include-raw --json"
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

pub(crate) fn apply_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_apply_args(args)?;
    let source_project =
        required_project(options.project.clone(), "report visuals formatting apply")?;
    require_visual_selector(&options.selector, "report visuals formatting apply")?;
    let bundle_path = options.bundle.as_ref().ok_or_else(|| {
        CliError::invalid_args("report visuals formatting apply requires --bundle <formatting-bundle.json>")
            .with_hint("Create a bundle with `report visuals formatting extract` first.")
            .with_suggested_command(
                "powerbi-cli report visuals formatting extract --project <source-project> --handle <source-visual-handle> --out visual-formatting-bundle.json --json",
            )
    })?;
    let mode = require_mode(options.mode, "report visuals formatting apply")?;
    let bundle = read_formatting_bundle(bundle_path)?;
    let payload = payload_from_bundle(&bundle)?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, apply_formatting)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let target_visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals formatting apply",
    )?;
    validate_bundle_for_target(
        &bundle,
        &payload,
        target_visual,
        options.allow_literal_text,
        options.allow_cross_type,
        bundle_path,
    )?;

    let target_visual_path = visual_path(target_visual, "report visuals formatting apply")?;
    let mut target_json = read_json_value(target_visual_path)?;
    let before_payload = formatting_payload_from_visual_json(&target_json);
    let before_summary = formatting_summary_from_payload(&before_payload, false);
    apply_payload_to_visual_json(&mut target_json, &payload)?;
    let after_payload = formatting_payload_from_visual_json(&target_json);
    let after_summary = formatting_summary_from_payload(&after_payload, false);

    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        write_json_atomic(target_visual_path, &target_json)?;
    }
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(&target_resolved)?)
    };
    let validation_ok = validation
        .as_ref()
        .map(|report| report.errors.is_empty())
        .unwrap_or(true);
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&target_resolved.project_dir);
    let readback = format!(
        "powerbi-cli report visuals formatting show --project {} --handle {} --json",
        project_arg,
        shell_arg(&target_visual.handle)
    );
    let raw_review = format!(
        "powerbi-cli report visuals formatting show --project {} --handle {} --include-raw --json",
        project_arg,
        shell_arg(&target_visual.handle)
    );
    let visual_readback = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&target_visual.handle)
    );
    let wireframe = format!(
        "powerbi-cli report wireframe export {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let inspect = format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let before_change = if options.include_raw {
        before_payload.to_json()
    } else {
        before_summary.clone()
    };
    let after_change = if options.include_raw {
        after_payload.to_json()
    } else {
        after_summary.clone()
    };

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.formatting.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "apply-formatting",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "source": {
            "kind": "bundle",
            "path": canonical_display(bundle_path),
            "fingerprint": bundle["sourceFingerprint"].clone(),
            "visual": bundle.pointer("/source/visual").cloned().unwrap_or(Value::Null)
        },
        "target": visual_detail(target_visual),
        "formattingPlan": {
            "strategy": "replace",
            "visualTypeMatch": bundle.pointer("/source/visual/visualType").and_then(Value::as_str) == Some(target_visual.visual_type.as_str()),
            "rawIncluded": options.include_raw,
            "before": before_summary,
            "after": after_summary,
            "safety": payload_safety_json(&payload, &formatting_summary_from_payload(&payload, false))
        },
        "changes": [{
            "kind": "pbir.visual.formatting",
            "action": "replace",
            "path": canonical_display(target_visual_path),
            "jsonPointers": ["/visual/objects", "/objects"],
            "rawIncluded": options.include_raw,
            "before": before_change,
            "after": after_change
        }],
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
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
        })),
        "readbackCommand": readback,
        "rawReviewCommand": raw_review,
        "visualReadbackCommand": visual_readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, raw_review, visual_readback, wireframe, inspect, validate]
    }))
}

impl FormattingPayload {
    fn to_json(&self) -> Value {
        json!({
            "visualObjects": self.visual_objects.clone(),
            "topLevelObjects": self.top_level_objects.clone()
        })
    }
}

fn formatting_payload_from_visual_json(visual_json: &Value) -> FormattingPayload {
    FormattingPayload {
        visual_objects: visual_json
            .pointer("/visual/objects")
            .cloned()
            .unwrap_or(Value::Null),
        top_level_objects: visual_json.get("objects").cloned().unwrap_or(Value::Null),
    }
}

fn formatting_summary_from_payload(payload: &FormattingPayload, include_raw: bool) -> Value {
    let mut visual_json = json!({ "visual": {} });
    apply_payload_to_visual_json(&mut visual_json, payload)
        .expect("virtual visual payload has required visual object");
    formatting_summary_from_visual_json(&visual_json, include_raw)
}

fn payload_is_empty(payload: &FormattingPayload) -> bool {
    value_is_missing_or_empty_object(&payload.visual_objects)
        && value_is_missing_or_empty_object(&payload.top_level_objects)
}

fn value_is_missing_or_empty_object(value: &Value) -> bool {
    value.is_null() || value.as_object().is_some_and(|object| object.is_empty())
}

fn payload_from_bundle(bundle: &Value) -> CliResult<FormattingPayload> {
    let formatting = bundle["formatting"].as_object().ok_or_else(|| {
        CliError::validation_failed("formatting bundle is missing formatting object")
    })?;
    let visual_objects = bundle_field(formatting.get("visualObjects"), "visualObjects")?;
    let top_level_objects = bundle_field(formatting.get("topLevelObjects"), "topLevelObjects")?;
    Ok(FormattingPayload {
        visual_objects,
        top_level_objects,
    })
}

fn bundle_field(value: Option<&Value>, name: &str) -> CliResult<Value> {
    let Some(value) = value else {
        return Ok(Value::Null);
    };
    if value.is_null() || value.is_object() {
        return Ok(value.clone());
    }
    Err(CliError::validation_failed(format!(
        "formatting bundle {name} must be a JSON object or null"
    )))
}

fn payload_safety_json(payload: &FormattingPayload, summary: &Value) -> Value {
    let scan = scan_formatting_payload(payload);
    json!({
        "rawPbir": true,
        "copiesData": false,
        "containsSelectors": scan.contains_selectors,
        "containsDataSelectors": scan.contains_data_selectors,
        "containsLiteralText": scan.contains_literal_text,
        "containsColors": scan.contains_colors,
        "containsExternalUris": scan.contains_external_uris,
        "containsCredentialLikeText": scan.contains_credential_like_text,
        "containsUnsupportedShapes": summary["unsupportedContainerCount"].as_u64().unwrap_or_default() > 0,
        "literalValueCount": summary["literalValueCount"],
        "note": "Formatting bundles preserve raw PBIR style objects. Applying a bundle writes /visual/objects only; root-level /objects is read-only because current Desktop rejects it in enhanced PBIR visual containers."
    })
}

fn scan_formatting_payload(payload: &FormattingPayload) -> SafetyScan {
    formatting_safety(
        [&payload.visual_objects, &payload.top_level_objects],
        CREDENTIAL_NEEDLES,
        false,
    )
}

fn validate_bundle_for_target(
    bundle: &Value,
    payload: &FormattingPayload,
    target: &VisualRecord,
    allow_literal_text: bool,
    allow_cross_type: bool,
    bundle_path: &Path,
) -> CliResult<()> {
    if payload_is_empty(payload) {
        return Err(CliError::invalid_args("formatting bundle contains no formatting objects")
            .with_hint("Extract from a visual whose formatObjectContainerCount is greater than zero.")
            .with_suggested_command(
                "powerbi-cli report visuals formatting list --project <project-dir-or.pbip> --json",
            ));
    }
    if !value_is_missing_or_empty_object(&payload.top_level_objects) {
        return Err(CliError::unsupported_feature(
            "formatting bundle contains root-level visual-container objects, which this Power BI Desktop build rejects in enhanced PBIR visual.json files",
        )
        .with_hint("Extract/apply formatting that lives under /visual/objects. Root-level /objects is read-only for compatibility inspection.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --include-raw --json",
        ));
    }
    let source_type = bundle
        .pointer("/source/visual/visualType")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "formatting bundle is missing source visualType: {}",
                bundle_path.display()
            ))
        })?;
    if !allow_cross_type && source_type != target.visual_type {
        return Err(CliError::invalid_args(format!(
            "formatting bundle source visualType {source_type} does not match target visualType {}",
            target.visual_type
        ))
        .with_hint("Use a same-type target visual, or pass --allow-cross-type after reviewing the dry-run plan.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting apply --project <target-project> --handle <target-visual-handle> --bundle <formatting-bundle.json> --dry-run --json",
        ));
    }
    let summary = formatting_summary_from_payload(payload, false);
    if summary["unsupportedContainerCount"]
        .as_u64()
        .unwrap_or_default()
        > 0
    {
        return Err(CliError::validation_failed(format!(
            "formatting bundle contains unsupported object shapes: {}",
            bundle_path.display()
        )));
    }
    let scan = scan_formatting_payload(payload);
    if scan.contains_external_uris {
        return Err(CliError::validation_failed(format!(
            "formatting bundle contains external URI text: {}",
            bundle_path.display()
        )));
    }
    if scan.contains_credential_like_text {
        return Err(CliError::validation_failed(format!(
            "formatting bundle contains credential-like text: {}",
            bundle_path.display()
        )));
    }
    if scan.contains_data_selectors {
        return Err(CliError::validation_failed(format!(
            "formatting bundle contains data-bound selectors: {}",
            bundle_path.display()
        )));
    }
    if scan.contains_literal_text && !allow_literal_text {
        return Err(CliError::invalid_args(
            "formatting bundle contains literal text such as copied title or alt-text values",
        )
        .with_hint("Review the bundle, then pass --allow-literal-text if copying those display strings is intentional.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting apply --project <target-project> --handle <target-visual-handle> --bundle <formatting-bundle.json> --allow-literal-text --dry-run --json",
        ));
    }
    Ok(())
}

fn apply_payload_to_visual_json(
    visual_json: &mut Value,
    payload: &FormattingPayload,
) -> CliResult<()> {
    let root = visual_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed("visual.json root must be a JSON object")
            .with_hint("Run `validate --strict` before applying formatting.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })?;
    {
        let visual = root
            .get_mut("visual")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                CliError::validation_failed("visual.json has no visual object")
                    .with_hint("Run `validate --strict` before applying formatting.")
                    .with_suggested_command(
                        "powerbi-cli validate --strict <project-dir-or.pbip> --json",
                    )
            })?;
        if payload.visual_objects.is_null() {
            visual.remove("objects");
        } else {
            visual.insert("objects".to_string(), payload.visual_objects.clone());
        }
    }
    root.remove("objects");
    Ok(())
}

fn read_formatting_bundle(path: &Path) -> CliResult<Value> {
    let value = read_json_value(path)?;
    if value["schema"].as_str() != Some(VISUAL_FORMATTING_BUNDLE_SCHEMA) {
        return Err(CliError::validation_failed(format!(
            "unsupported visual formatting bundle schema in {}",
            path.display()
        )));
    }
    if value["bundleVersion"].as_u64() != Some(1) {
        return Err(CliError::validation_failed(format!(
            "unsupported visual formatting bundle version in {}",
            path.display()
        )));
    }
    Ok(value)
}

fn visual_path<'a>(visual: &'a VisualRecord, command: &str) -> CliResult<&'a PathBuf> {
    visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual has no visual.json path in inspect output: {}",
            visual.handle
        ))
        .with_hint("Run `validate --strict` before applying formatting.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle {} --json",
            shell_arg(&visual.handle)
        ))
    })
}

fn target_project(
    source_resolved: &ResolvedProject,
    mode: MutationMode,
    out_dir: Option<&Path>,
) -> CliResult<ResolvedProject> {
    match (mode, out_dir) {
        (MutationMode::DryRun | MutationMode::InPlace, _) => Ok(source_resolved.clone()),
        (MutationMode::OutDir, Some(out_dir)) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)
        }
        (MutationMode::OutDir, None) => {
            Err(CliError::invalid_args("--out-dir requires a directory"))
        }
    }
}

fn require_mode(mode: Option<MutationMode>, command: &str) -> CliResult<MutationMode> {
    mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --dry-run, --in-place, or --out-dir <dir>"
        ))
        .with_hint("Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --bundle <formatting-bundle.json> --dry-run --json"
        ))
    })
}

fn set_mode(
    current: &mut Option<MutationMode>,
    next: MutationMode,
    command: &str,
) -> CliResult<()> {
    if current.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one output mode: --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --bundle <formatting-bundle.json> --dry-run --json"
        )));
    }
    *current = Some(next);
    Ok(())
}

fn mode_name(mode: MutationMode) -> &'static str {
    match mode {
        MutationMode::DryRun => "dry-run",
        MutationMode::InPlace => "in-place",
        MutationMode::OutDir => "out-dir",
    }
}

fn parse_extract_args(args: &[String]) -> CliResult<ExtractOptions> {
    let mut options = ExtractOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" => options.selector.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" => {
                let value = take_value(args, &mut i, "--visual")?;
                if value.starts_with("visual:") {
                    options.selector.handle = Some(value);
                } else {
                    options.selector.visual = Some(value);
                }
            }
            "--out" | "--out-file" => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals formatting extract flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report visuals formatting extract --project <project-dir-or.pbip> --handle <visual-handle> --out visual-formatting-bundle.json --json`.")
                .with_suggested_command(
                    "powerbi-cli report visuals formatting extract --project <project-dir-or.pbip> --handle <visual-handle> --out visual-formatting-bundle.json --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_apply_args(args: &[String]) -> CliResult<ApplyOptions> {
    let mut options = ApplyOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" => options.selector.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" => {
                let value = take_value(args, &mut i, "--visual")?;
                if value.starts_with("visual:") {
                    options.selector.handle = Some(value);
                } else {
                    options.selector.visual = Some(value);
                }
            }
            "--bundle" | "--style-bundle" | "--formatting-bundle" => {
                options.bundle = Some(PathBuf::from(take_value(args, &mut i, "--bundle")?));
            }
            "--include-raw" => {
                options.include_raw = true;
                i += 1;
            }
            "--allow-literal-text" => {
                options.allow_literal_text = true;
                i += 1;
            }
            "--allow-cross-type" => {
                options.allow_cross_type = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report visuals formatting apply",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report visuals formatting apply",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report visuals formatting apply",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals formatting apply flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals formatting apply\"` for exact flags.")
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report visuals formatting apply\"",
                ));
            }
        }
    }
    Ok(options)
}

fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass the PBIP project directory or the .pbip file explicitly with `--project`.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --json"
        ))
    })
}

fn require_visual_selector(selector: &VisualSelector, command: &str) -> CliResult<()> {
    if selector.handle.is_some() || (selector.page.is_some() && selector.visual.is_some()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{command} requires --handle or --page plus --visual"
    ))
    .with_hint("Use `report visuals list` to get stable visual handles.")
    .with_suggested_command(format!(
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --json"
    )))
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli --json capabilities --for report` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for report")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn fingerprint_value(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    format!("fnv64:{}", hash_hex(&text))
}

fn hash_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
