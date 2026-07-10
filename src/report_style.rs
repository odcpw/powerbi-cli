use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::pbir::{load_report_snapshot, visual_list_item};
use crate::project_io::{write_json_atomic, write_json_pretty};
use crate::safety_scan::{STYLE_CREDENTIAL_NEEDLES, formatting_safety};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const STYLE_BUNDLE_SCHEMA: &str = "powerbi-cli.report.style-bundle.v1";

#[derive(Debug, Default)]
struct ProjectOptions {
    project: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct ExtractOptions {
    project: Option<PathBuf>,
    out: Option<PathBuf>,
    include_literal_text: bool,
}

#[derive(Debug, Default)]
struct ApplyOptions {
    project: Option<PathBuf>,
    bundle: Option<PathBuf>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    allow_literal_text: bool,
}

#[derive(Debug, Default)]
struct DiffOptions {
    left: Option<PathBuf>,
    right: Option<PathBuf>,
}

pub(crate) fn style_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report style requires a subcommand: inspect, extract, diff, or apply",
        )
        .with_hint("Use style bundles for schema-independent master-format extraction and application.")
        .with_suggested_command(
            "powerbi-cli report style extract --project <project-dir-or.pbip> --out style.json --json",
        ));
    };
    match action.as_str() {
        "inspect" | "show" => inspect_style(rest),
        "extract" | "export" => extract_style(rest),
        "diff" => diff_style(rest),
        "apply" | "import" => apply_style(rest),
        other => Err(CliError::invalid_args(format!(
            "unknown report style command: {other}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report style\"` for supported style commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report style\"")),
    }
}

fn inspect_style(args: &[String]) -> CliResult<Value> {
    let options = parse_project_args(args, "report style inspect")?;
    let project = required_project(options.project, "report style inspect")?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let bundle = style_bundle(&resolved, false)?;
    let project_arg = command_arg(&resolved.project_dir);
    Ok(json!({
        "schema": "powerbi-cli.report.style.inspect.v1",
        "ok": validation.errors.is_empty(),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "summary": style_summary(&bundle),
        "style": bundle,
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli report style extract --project {project_arg} --out style.json --json"),
            "powerbi-cli report style apply --project <target-project> --bundle style.json --dry-run --json".to_string()
        ]
    }))
}

fn extract_style(args: &[String]) -> CliResult<Value> {
    let options = parse_extract_args(args)?;
    let project = required_project(options.project, "report style extract")?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let bundle = style_bundle(&resolved, options.include_literal_text)?;
    if let Some(out) = &options.out {
        write_json_pretty(out, &bundle)?;
    }
    Ok(json!({
        "schema": "powerbi-cli.report.style.extract.v1",
        "ok": validation.errors.is_empty(),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "out": options.out.as_ref().map(|path| canonical_display(path)),
        "summary": style_summary(&bundle),
        "bundle": bundle,
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            "powerbi-cli report style inspect --project <project-dir-or.pbip> --json",
            "powerbi-cli report style apply --project <target-project> --bundle style.json --dry-run --json"
        ]
    }))
}

fn diff_style(args: &[String]) -> CliResult<Value> {
    let options = parse_diff_args(args)?;
    let left = read_style_bundle(&required_path(options.left, "left style bundle")?)?;
    let right = read_style_bundle(&required_path(options.right, "right style bundle")?)?;
    let left_summary = style_summary(&left);
    let right_summary = style_summary(&right);
    Ok(json!({
        "schema": "powerbi-cli.report.style.diff.v1",
        "ok": true,
        "left": left_summary,
        "right": right_summary,
        "diff": {
            "sameFingerprint": fingerprint_value(&left) == fingerprint_value(&right),
            "leftFingerprint": fingerprint_value(&left),
            "rightFingerprint": fingerprint_value(&right),
            "themeCollectionChanged": left["themeCollection"] != right["themeCollection"],
            "visualStyleCountDelta": right["visualStyles"].as_array().map(Vec::len).unwrap_or_default() as i64
                - left["visualStyles"].as_array().map(Vec::len).unwrap_or_default() as i64
        }
    }))
}

fn apply_style(args: &[String]) -> CliResult<Value> {
    let options = parse_apply_args(args)?;
    let source_project = required_project(options.project.clone(), "report style apply")?;
    let bundle_path = options.bundle.as_ref().ok_or_else(|| {
        CliError::invalid_args("report style apply requires --bundle <style-bundle.json>")
            .with_hint("Create a bundle with `report style extract` first.")
            .with_suggested_command(
                "powerbi-cli report style extract --project <source-project> --out style.json --json",
            )
    })?;
    let bundle = read_style_bundle(bundle_path)?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = require_mode(options.mode, "report style apply")?;
    crate::cli_support::preflight_out_dir(args, apply_style)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let mut changes = Vec::new();
    let mut applied = Vec::new();
    let mut skipped = Vec::new();

    let report_path = target_resolved
        .report_dir
        .join("definition")
        .join("report.json");
    let mut report_json = read_json_value(&report_path)?;
    let before_theme = report_json["themeCollection"].clone();
    let after_theme = bundle["themeCollection"].clone();
    if !after_theme.is_null() && before_theme != after_theme {
        report_json["themeCollection"] = after_theme.clone();
        changes.push(json!({
            "kind": "pbir.report.themeCollection",
            "path": canonical_display(&report_path),
            "before": before_theme,
            "after": after_theme
        }));
        applied.push(json!({"kind": "themeCollection", "target": "report"}));
    }

    let visual_styles = bundle["visualStyles"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let style_by_key = visual_styles
        .iter()
        .filter_map(|style| {
            Some((
                format!(
                    "{}:{}",
                    style["visualType"].as_str()?,
                    style["ordinalWithinType"].as_u64()?
                ),
                style.clone(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let mut target_ordinals: BTreeMap<String, u64> = BTreeMap::new();
    for visual in snapshot.pages.iter().flat_map(|page| page.visuals.iter()) {
        let ordinal = target_ordinals
            .entry(visual.visual_type.clone())
            .or_default();
        let key = format!("{}:{}", visual.visual_type, *ordinal);
        *ordinal += 1;
        let Some(style) = style_by_key.get(&key) else {
            skipped.push(json!({
                "handle": visual.handle,
                "visualType": visual.visual_type,
                "reason": "no matching visualType/ordinal style entry"
            }));
            continue;
        };
        let payload = &style["formatting"];
        let safety = style_safety(payload);
        if safety["containsLiteralText"].as_bool().unwrap_or(false) && !options.allow_literal_text {
            skipped.push(json!({
                "handle": visual.handle,
                "visualType": visual.visual_type,
                "reason": "style entry contains literal text; rerun with --allow-literal-text after review"
            }));
            continue;
        }
        if !payload_top_level_objects_empty(payload) {
            skipped.push(json!({
                "handle": visual.handle,
                "visualType": visual.visual_type,
                "reason": "style entry contains root-level visual-container objects; current Desktop rejects /objects at visual.json root in enhanced PBIR"
            }));
            continue;
        }
        let Some(visual_path) = visual.path.as_ref() else {
            skipped.push(json!({
                "handle": visual.handle,
                "reason": "visual has no visual.json path"
            }));
            continue;
        };
        let mut visual_json = read_json_value(visual_path)?;
        let before = formatting_payload_from_visual(&visual_json);
        set_formatting_payload(&mut visual_json, payload)?;
        let after = formatting_payload_from_visual(&visual_json);
        if before != after {
            changes.push(json!({
                "kind": "pbir.visual.formatting",
                "target": visual_list_item(visual),
                "path": canonical_display(visual_path),
                "before": before,
                "after": after
            }));
            applied.push(json!({
                "kind": "visualFormatting",
                "handle": visual.handle,
                "matchedBy": "visualTypeOrdinal",
                "visualType": visual.visual_type
            }));
        }
    }

    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        if changes
            .iter()
            .any(|change| change["kind"] == "pbir.report.themeCollection")
        {
            write_json_atomic(&report_path, &report_json)?;
        }
        for change in changes
            .iter()
            .filter(|change| change["kind"] == "pbir.visual.formatting")
        {
            let path = PathBuf::from(change["path"].as_str().unwrap_or_default());
            let mut visual_json = read_json_value(&path)?;
            set_formatting_payload(&mut visual_json, &change["after"])?;
            write_json_atomic(&path, &visual_json)?;
        }
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
    let readback = format!("powerbi-cli report style inspect --project {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let handoff = format!("powerbi-cli handoff check {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.report.style.apply.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "apply-style",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "bundle": canonical_display(bundle_path),
        "source": {
            "fingerprint": fingerprint_value(&bundle),
            "allowLiteralText": options.allow_literal_text
        },
        "counts": {
            "changes": changes.len(),
            "applied": applied.len(),
            "skipped": skipped.len()
        },
        "applied": applied,
        "skipped": skipped,
        "changes": changes,
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
        "validateCommand": validate,
        "handoffCheckCommand": handoff,
        "next": [readback, validate, handoff]
    }))
}

fn style_bundle(resolved: &ResolvedProject, include_literal_text: bool) -> CliResult<Value> {
    let report_path = resolved.report_dir.join("definition").join("report.json");
    let report_json = read_json_value(&report_path)?;
    let snapshot = load_report_snapshot(resolved)?;
    let mut ordinals: BTreeMap<String, u64> = BTreeMap::new();
    let mut visual_styles = Vec::new();
    for visual in snapshot.pages.iter().flat_map(|page| page.visuals.iter()) {
        let ordinal = ordinals.entry(visual.visual_type.clone()).or_default();
        let payload = visual
            .path
            .as_ref()
            .map(|path| read_json_value(path).map(|json| formatting_payload_from_visual(&json)))
            .transpose()?
            .unwrap_or_else(|| json!({ "visualObjects": null, "topLevelObjects": null }));
        let safety = style_safety(&payload);
        visual_styles.push(json!({
            "handle": visual.handle,
            "name": visual.name,
            "title": visual.title,
            "visualType": visual.visual_type,
            "ordinalWithinType": *ordinal,
            "page": {
                "handle": visual.page_handle,
                "name": visual.page_name,
                "displayName": visual.page_display_name
            },
            "formatting": payload,
            "safety": safety,
            "review": {
                "literalTextIncludedForFidelity": safety["containsLiteralText"].as_bool().unwrap_or(false),
                "extractFlagIncludedLiteralText": include_literal_text,
                "applyRequiresAllowLiteralText": safety["containsLiteralText"].as_bool().unwrap_or(false)
            }
        }));
        *ordinal += 1;
    }
    let bundle = json!({
        "schema": STYLE_BUNDLE_SCHEMA,
        "source": {
            "projectDir": canonical_display(&resolved.project_dir),
            "pbip": canonical_display(&resolved.pbip_path),
            "reportDir": canonical_display(&resolved.report_dir)
        },
        "themeCollection": report_json["themeCollection"].clone(),
        "visualStyles": visual_styles,
        "policy": {
            "matching": "visualType+ordinalWithinType",
            "schemaIndependent": true,
            "copiesBindings": false,
            "copiesData": false,
            "conditionalFormattingAuthoring": "fixture-gated"
        }
    });
    Ok(bundle)
}

fn style_summary(bundle: &Value) -> Value {
    let styles = bundle["visualStyles"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    json!({
        "schema": bundle["schema"],
        "fingerprint": fingerprint_value(bundle),
        "hasThemeCollection": !bundle["themeCollection"].is_null(),
        "visualStyles": styles.len(),
        "visualTypes": styles.iter().filter_map(|style| style["visualType"].as_str()).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>(),
        "stylesWithLiteralText": styles.iter().filter(|style| style["safety"]["containsLiteralText"].as_bool().unwrap_or(false)).count(),
        "stylesWithDataSelectors": styles.iter().filter(|style| style["safety"]["containsDataSelectors"].as_bool().unwrap_or(false)).count()
    })
}

fn formatting_payload_from_visual(visual_json: &Value) -> Value {
    json!({
        "visualObjects": visual_json.pointer("/visual/objects").cloned().unwrap_or(Value::Null),
        "topLevelObjects": visual_json.get("objects").cloned().unwrap_or(Value::Null)
    })
}

fn set_formatting_payload(visual_json: &mut Value, payload: &Value) -> CliResult<()> {
    let root = visual_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed("visual.json root must be an object before applying style")
    })?;
    let visual = root
        .entry("visual".to_string())
        .or_insert_with(|| json!({}));
    let visual = visual.as_object_mut().ok_or_else(|| {
        CliError::validation_failed("/visual must be an object before applying style")
    })?;
    match &payload["visualObjects"] {
        Value::Null => {
            visual.remove("objects");
        }
        other => {
            visual.insert("objects".to_string(), other.clone());
        }
    }
    root.remove("objects");
    Ok(())
}

fn payload_top_level_objects_empty(payload: &Value) -> bool {
    let value = &payload["topLevelObjects"];
    value.is_null() || value.as_object().is_some_and(|object| object.is_empty())
}

fn read_style_bundle(path: &Path) -> CliResult<Value> {
    let value = read_json_value(path)?;
    if value["schema"].as_str() != Some(STYLE_BUNDLE_SCHEMA) {
        return Err(CliError::validation_failed(format!(
            "style bundle schema mismatch in {}",
            path.display()
        ))
        .with_hint("Use a bundle created by `report style extract`.")
        .with_suggested_command(
            "powerbi-cli report style extract --project <source-project> --out style.json --json",
        ));
    }
    Ok(value)
}

fn style_safety(payload: &Value) -> Value {
    let scan = formatting_safety([payload], STYLE_CREDENTIAL_NEEDLES, true);
    json!({
        "containsSelectors": scan.contains_selectors,
        "containsDataSelectors": scan.contains_data_selectors,
        "containsLiteralText": scan.contains_literal_text,
        "containsConditionalFormattingSignals": scan.contains_conditional_formatting_signals,
        "containsCredentialLikeText": scan.contains_credential_like_text
    })
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

fn parse_project_args(args: &[String], command: &str) -> CliResult<ProjectOptions> {
    let mut options = ProjectOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            other => {
                return Err(CliError::invalid_args(format!("unknown {command} flag: {other}"))
                    .with_hint("Run `powerbi-cli --json capabilities --for \"report style\"` for exact usage.")
                    .with_suggested_command("powerbi-cli --json capabilities --for \"report style\""));
            }
        }
    }
    Ok(options)
}

fn parse_extract_args(args: &[String]) -> CliResult<ExtractOptions> {
    let mut options = ExtractOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--out" | "--out-file" => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            "--include-literal-text" => {
                options.include_literal_text = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report style extract flag: {other}"
                ))
                .with_suggested_command(
                    "powerbi-cli report style extract --project <project-dir-or.pbip> --out style.json --json",
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
            "--bundle" => {
                options.bundle = Some(PathBuf::from(take_value(args, &mut i, "--bundle")?))
            }
            "--allow-literal-text" => {
                options.allow_literal_text = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report style apply",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report style apply",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report style apply",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report style apply flag: {other}"
                ))
                .with_suggested_command(
                    "powerbi-cli report style apply --project <project-dir-or.pbip> --bundle style.json --dry-run --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_diff_args(args: &[String]) -> CliResult<DiffOptions> {
    let mut options = DiffOptions::default();
    for arg in args {
        if arg.starts_with('-') {
            return Err(
                CliError::invalid_args(format!("unknown report style diff flag: {arg}"))
                    .with_suggested_command(
                        "powerbi-cli report style diff before-style.json after-style.json --json",
                    ),
            );
        }
        if options.left.is_none() {
            options.left = Some(PathBuf::from(arg));
        } else if options.right.is_none() {
            options.right = Some(PathBuf::from(arg));
        } else {
            return Err(CliError::invalid_args(
                "report style diff accepts exactly two bundle paths",
            ));
        }
    }
    Ok(options)
}

fn required_path(path: Option<PathBuf>, label: &str) -> CliResult<PathBuf> {
    path.ok_or_else(|| {
        CliError::invalid_args(format!("missing {label}")).with_suggested_command(
            "powerbi-cli report style diff before-style.json after-style.json --json",
        )
    })
}

#[allow(dead_code)]
fn _shell_handle(handle: &str) -> String {
    shell_arg(handle)
}
