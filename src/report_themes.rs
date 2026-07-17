use crate::pbir_themes::{
    THEME_BUNDLE_SCHEMA, list_report_themes, theme_record_json, theme_safety, theme_safety_json,
    write_theme_bundle, write_theme_json,
};
use crate::project_io::{copy_project_dir, write_json_atomic};
use crate::safety_scan::contains_external_uri as value_contains_external_uri;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    read_json_value, report_schema_major, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn themes_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report themes requires a subcommand: show, extract, apply, presets, or apply-preset",
        )
        .with_hint("Run `powerbi-cli report themes show --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli report themes show --project <project-dir-or.pbip> --json",
        ));
    };

    match normalize_action(action).as_str() {
        "show" => show_theme(rest),
        "extract" => extract_theme(rest),
        "apply" => apply_theme(rest),
        "presets" | "preset" => theme_presets(rest),
        "apply-preset" => apply_theme_preset(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown report themes command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report themes\"` for supported theme commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report themes\"")),
    }
}

#[derive(Debug, Default)]
struct ProjectOptions {
    project: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct ExtractOptions {
    project: Option<PathBuf>,
    out: Option<PathBuf>,
}

#[derive(Debug, Clone)]
enum MutationMode {
    DryRun,
    InPlace,
    OutDir(PathBuf),
}

#[derive(Debug, Default)]
struct ApplyOptions {
    project: Option<PathBuf>,
    bundle: Option<PathBuf>,
    mode: Option<MutationMode>,
}

#[derive(Debug, Default)]
struct PresetOptions {
    preset: Option<String>,
    include_bundle: bool,
}

#[derive(Debug, Default)]
struct ApplyPresetOptions {
    project: Option<PathBuf>,
    preset: Option<String>,
    mode: Option<MutationMode>,
}

#[derive(Debug, Clone, Copy)]
struct BuiltinThemePreset {
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    colors: &'static [&'static str],
    background: &'static str,
    foreground: &'static str,
    table_accent: &'static str,
}

const REGISTERED_RESOURCES_PACKAGE: &str = "RegisteredResources";
const CUSTOM_THEME_ITEM_TYPE: &str = "CustomTheme";
const REPORT_VERSION_AT_IMPORT: &str = "2.0.0";
const REPORT_VERSION_AT_IMPORT_VISUAL: &str = "2.10.0";
const REPORT_VERSION_AT_IMPORT_PAGE: &str = "2.3.1";
const REPORT_VERSION_AT_IMPORT_REPORT: &str = "3.4.0";

fn show_theme(args: &[String]) -> CliResult<Value> {
    let options = parse_project_args(args, "report themes show")?;
    let project = required_project(options.project, "report themes show")?;
    let resolved = resolve_project(&project)?;
    let report_json_path = report_json_path(&resolved);
    let report_json = read_json_value(&report_json_path)?;
    let theme_collection = report_json["themeCollection"].clone();
    let resources = list_report_themes(&resolved)?;

    Ok(json!({
        "schema": "powerbi-cli.report.themes.show.v1",
        "ok": true,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "theme": report_theme_json(&report_json_path, &theme_collection, &resources, false),
        "next": [
            format!("powerbi-cli report themes extract --project {} --out report-theme-bundle.json --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report themes apply --project {} --bundle report-theme-bundle.json --dry-run --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn extract_theme(args: &[String]) -> CliResult<Value> {
    let options = parse_extract_args(args)?;
    let project = required_project(options.project, "report themes extract")?;
    let resolved = resolve_project(&project)?;
    let report_json_path = report_json_path(&resolved);
    let report_json = read_json_value(&report_json_path)?;
    let theme_collection = report_json["themeCollection"].clone();
    let resources = list_report_themes(&resolved)?;
    let bundle = report_theme_bundle_json(&report_json_path, &theme_collection, &resources);
    if let Some(out) = &options.out {
        write_theme_bundle(out, &bundle)?;
    }

    Ok(json!({
        "schema": "powerbi-cli.report.themes.extract.v1",
        "ok": true,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "out": options.out.as_ref().map(|path| canonical_display(path)),
        "bundle": bundle,
        "next": [
            "powerbi-cli report themes apply --project <target-project> --bundle <theme-bundle.json> --dry-run --json",
            "powerbi-cli report themes show --project <target-project> --json"
        ]
    }))
}

fn apply_theme(args: &[String]) -> CliResult<Value> {
    let options = parse_apply_args(args)?;
    let source_project = required_project(options.project.clone(), "report themes apply")?;
    let bundle_path = options.bundle.as_ref().ok_or_else(|| {
        CliError::invalid_args("report themes apply requires --bundle <theme-bundle.json>")
            .with_hint("Create a bundle with `report themes extract` first.")
            .with_suggested_command(
                "powerbi-cli report themes extract --project <source-project> --out theme-bundle.json --json",
            )
    })?;
    let bundle = read_bundle(bundle_path)?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = options.mode.as_ref().ok_or_else(|| {
        CliError::invalid_args("report themes apply requires --dry-run, --in-place, or --out-dir <dir>")
            .with_hint("Start with `--dry-run`; use `--out-dir` or `--in-place` only after reviewing the raw theme bundle.")
            .with_suggested_command(
                "powerbi-cli report themes apply --project <project-dir-or.pbip> --bundle <theme-bundle.json> --dry-run --json",
            )
    })?;
    crate::cli_support::preflight_out_dir(args, apply_theme)?;
    let target_resolved = match mode {
        MutationMode::DryRun | MutationMode::InPlace => source_resolved,
        MutationMode::OutDir(out_dir) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)?
        }
    };

    let report_json_path = report_json_path(&target_resolved);
    let mut report_json = read_json_value(&report_json_path)?;
    let before_theme_collection = report_json["themeCollection"].clone();
    let after_theme_collection =
        normalized_theme_collection_for_bundle(&bundle["themeCollection"], &bundle, &report_json)?;
    validate_theme_collection(&after_theme_collection, bundle_path)?;
    let resource_changes = bundled_resource_changes(&target_resolved, &bundle)?;
    let before_resource_packages = report_json["resourcePackages"].clone();
    upsert_registered_resource_package(&mut report_json, &bundle)?;
    let after_resource_packages = report_json["resourcePackages"].clone();

    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        report_json["themeCollection"] = after_theme_collection.clone();
        write_json_atomic(&report_json_path, &report_json)?;
        for change in &resource_changes {
            let path = PathBuf::from(change["path"].as_str().unwrap_or_default());
            write_theme_json(&path, &change["after"])?;
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
    let readback = format!(
        "powerbi-cli report themes show --project {} --json",
        project_arg
    );
    let validate = format!("powerbi-cli validate --strict {} --json", project_arg);
    let handoff = format!("powerbi-cli handoff check {} --json", project_arg);

    Ok(json!({
        "schema": "powerbi-cli.report.themes.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "apply-theme",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "source": {
            "kind": "bundle",
            "path": canonical_display(bundle_path),
            "fingerprint": fingerprint_value(&bundle)
        },
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "bundle": canonical_display(bundle_path),
        "target": {
            "handle": "theme:report",
            "path": canonical_display(&report_json_path)
        },
        "changes": [{
            "kind": "pbir.report.themeCollection",
            "action": "replace",
            "path": canonical_display(&report_json_path),
            "before": before_theme_collection,
            "after": after_theme_collection
        }, {
            "kind": "pbir.report.resourcePackages",
            "action": "upsert-registered-theme-package",
            "path": canonical_display(&report_json_path),
            "before": before_resource_packages,
            "after": after_resource_packages
        }],
        "resourceChanges": resource_changes,
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

fn theme_presets(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Ok(theme_preset_list());
    };
    match action.as_str() {
        "list" | "ls" | "catalog" => Ok(theme_preset_list()),
        "show" | "get" => show_theme_preset(rest),
        other => Err(CliError::invalid_args(format!(
            "unknown report themes presets command: {other}"
        ))
        .with_hint("Run `powerbi-cli report themes presets list --json`.")
        .with_suggested_command("powerbi-cli report themes presets list --json")),
    }
}

fn theme_preset_list() -> Value {
    json!({
        "schema": "powerbi-cli.report.themes.presets.v1",
        "presets": BUILTIN_THEME_PRESETS.iter().map(|preset| json!({
            "id": preset.id,
            "name": preset.name,
            "summary": preset.summary,
            "dataColors": preset.colors,
            "background": preset.background,
            "foreground": preset.foreground,
            "tableAccent": preset.table_accent,
            "command": format!("powerbi-cli report themes apply-preset --project <project-dir-or.pbip> --preset {} --dry-run --json", preset.id)
        })).collect::<Vec<_>>(),
        "next": [
            "powerbi-cli report themes presets show --preset risk-dashboard --json",
            "powerbi-cli report themes apply-preset --project <project-dir-or.pbip> --preset risk-dashboard --dry-run --json"
        ]
    })
}

fn show_theme_preset(args: &[String]) -> CliResult<Value> {
    let options = parse_preset_args(args)?;
    let preset_id = options.preset.as_deref().unwrap_or("risk-dashboard");
    let preset = builtin_theme_preset(preset_id)?;
    let bundle = builtin_theme_bundle(preset);
    Ok(json!({
        "schema": "powerbi-cli.report.themes.preset.v1",
        "preset": {
            "id": preset.id,
            "name": preset.name,
            "summary": preset.summary,
            "dataColors": preset.colors,
            "background": preset.background,
            "foreground": preset.foreground,
            "tableAccent": preset.table_accent,
            "bundle": options.include_bundle.then_some(bundle.clone()),
            "fingerprint": fingerprint_value(&bundle)
        },
        "next": [
            format!("powerbi-cli report themes apply-preset --project <project-dir-or.pbip> --preset {} --dry-run --json", preset.id),
            "powerbi-cli report themes presets list --json".to_string()
        ]
    }))
}

fn apply_theme_preset(args: &[String]) -> CliResult<Value> {
    let options = parse_apply_preset_args(args)?;
    let source_project = required_project(options.project.clone(), "report themes apply-preset")?;
    let preset_id = options.preset.as_deref().unwrap_or("risk-dashboard");
    let preset = builtin_theme_preset(preset_id)?;
    let bundle = builtin_theme_bundle(preset);
    let source_resolved = resolve_project(&source_project)?;
    let mode = options.mode.as_ref().ok_or_else(|| {
        CliError::invalid_args("report themes apply-preset requires --dry-run, --in-place, or --out-dir <dir>")
            .with_hint("Start with `--dry-run`; use `--out-dir` or `--in-place` only after reviewing the theme changes.")
            .with_suggested_command(
                "powerbi-cli report themes apply-preset --project <project-dir-or.pbip> --preset risk-dashboard --dry-run --json",
            )
    })?;
    crate::cli_support::preflight_out_dir(args, apply_theme_preset)?;
    let target_resolved = match mode {
        MutationMode::DryRun | MutationMode::InPlace => source_resolved,
        MutationMode::OutDir(out_dir) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)?
        }
    };

    let report_json_path = report_json_path(&target_resolved);
    let mut report_json = read_json_value(&report_json_path)?;
    let before_theme_collection = report_json["themeCollection"].clone();
    let after_theme_collection =
        normalized_theme_collection_for_bundle(&bundle["themeCollection"], &bundle, &report_json)?;
    validate_theme_collection(&after_theme_collection, Path::new(preset.id))?;
    let resource_changes = bundled_resource_changes(&target_resolved, &bundle)?;
    let before_resource_packages = report_json["resourcePackages"].clone();
    upsert_registered_resource_package(&mut report_json, &bundle)?;
    let after_resource_packages = report_json["resourcePackages"].clone();

    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        report_json["themeCollection"] = after_theme_collection.clone();
        write_json_atomic(&report_json_path, &report_json)?;
        for change in &resource_changes {
            let path = PathBuf::from(change["path"].as_str().unwrap_or_default());
            write_theme_json(&path, &change["after"])?;
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
    let readback = format!(
        "powerbi-cli report themes show --project {} --json",
        project_arg
    );
    let validate = format!("powerbi-cli validate --strict {} --json", project_arg);
    let handoff = format!("powerbi-cli handoff check {} --json", project_arg);

    Ok(json!({
        "schema": "powerbi-cli.report.themes.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "apply-theme-preset",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "source": {
            "kind": "builtin-preset",
            "preset": preset.id,
            "name": preset.name,
            "fingerprint": fingerprint_value(&bundle)
        },
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": {
            "handle": "theme:report",
            "path": canonical_display(&report_json_path)
        },
        "changes": [{
            "kind": "pbir.report.themeCollection",
            "action": "replace",
            "path": canonical_display(&report_json_path),
            "before": before_theme_collection,
            "after": after_theme_collection
        }, {
            "kind": "pbir.report.resourcePackages",
            "action": "upsert-registered-theme-package",
            "path": canonical_display(&report_json_path),
            "before": before_resource_packages,
            "after": after_resource_packages
        }],
        "resourceChanges": resource_changes,
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

fn report_theme_json(
    report_json_path: &Path,
    theme_collection: &Value,
    resources: &[crate::pbir_themes::ReportThemeRecord],
    include_resources: bool,
) -> Value {
    json!({
        "handle": "theme:report",
        "state": theme_state(theme_collection, resources),
        "name": theme_name(theme_collection, resources),
        "fingerprint": fingerprint_value(&json!({
            "themeCollection": theme_collection,
            "registeredThemes": resources.iter().map(|resource| theme_record_json(resource, include_resources)).collect::<Vec<_>>()
        })),
        "path": canonical_display(report_json_path),
        "reportJsonPath": canonical_display(report_json_path),
        "themeCollection": theme_collection,
        "isEmpty": theme_collection.as_object().is_none_or(|object| object.is_empty()),
        "safety": theme_safety_json(&theme_safety(theme_collection)),
        "registeredThemes": resources.iter().map(|resource| theme_record_json(resource, include_resources)).collect::<Vec<_>>()
    })
}

fn report_theme_bundle_json(
    report_json_path: &Path,
    theme_collection: &Value,
    resources: &[crate::pbir_themes::ReportThemeRecord],
) -> Value {
    json!({
        "schema": THEME_BUNDLE_SCHEMA,
        "bundleVersion": 1,
        "sourceFingerprint": fingerprint_value(&json!({
            "themeCollection": theme_collection,
            "registeredThemes": resources.iter().map(|resource| theme_record_json(resource, true)).collect::<Vec<_>>()
        })),
        "theme": report_theme_json(report_json_path, theme_collection, resources, true),
        "themeCollection": theme_collection,
        "registeredThemes": resources.iter().map(|resource| theme_record_json(resource, true)).collect::<Vec<_>>(),
        "safety": {
            "containsExternalUris": value_contains_external_uri(theme_collection) || resources.iter().any(|resource| value_contains_external_uri(&resource.theme)),
            "containsBinaryAssets": false,
            "copiesData": false,
            "themeCollection": theme_safety_json(&theme_safety(theme_collection))
        }
    })
}

fn bundled_resource_changes(
    resolved: &crate::ResolvedProject,
    bundle: &Value,
) -> CliResult<Vec<Value>> {
    let mut changes = Vec::new();
    for resource in bundle["registeredThemes"].as_array().into_iter().flatten() {
        let relative = resource["relativePath"].as_str().ok_or_else(|| {
            CliError::validation_failed("theme bundle resource is missing relativePath")
        })?;
        let relative_path = clean_registered_resource_path(relative)?;
        let path = resolved.report_dir.join(&relative_path);
        let before = if path.exists() {
            Some(read_json_value(&path)?)
        } else {
            None
        };
        let mut after = resource["themeJson"].clone();
        let resource_name = relative_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                CliError::validation_failed(format!(
                    "theme bundle resource path has no file name: {relative}"
                ))
            })?;
        after["name"] = Value::String(resource_name.to_string());
        crate::pbir_themes::validate_theme_json(&after, &path)?;
        changes.push(json!({
            "kind": "pbir.report.registeredThemeResource",
            "action": if before.is_some() { "replace" } else { "add" },
            "path": canonical_display(&path),
            "relativePath": relative_path.to_string_lossy().replace('\\', "/"),
            "before": before,
            "after": after
        }));
    }
    changes.sort_by(|left, right| {
        left["relativePath"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["relativePath"].as_str().unwrap_or_default())
    });
    Ok(changes)
}

fn validate_theme_collection(value: &Value, path: &Path) -> CliResult<()> {
    if !value.is_object() {
        return Err(CliError::validation_failed(format!(
            "theme bundle themeCollection must be a JSON object: {}",
            path.display()
        )));
    }
    let safety = theme_safety(value);
    if safety
        .findings
        .iter()
        .any(|finding| finding.severity == "error")
    {
        return Err(CliError::validation_failed(format!(
            "theme bundle themeCollection contains credential-like text: {}",
            path.display()
        )));
    }
    Ok(())
}

fn read_bundle(path: &Path) -> CliResult<Value> {
    let text = fs::read_to_string(path).map_err(|err| {
        CliError::file_not_found(format!("read bundle {}: {err}", path.display()))
    })?;
    let value = serde_json::from_str::<Value>(&text).map_err(|err| {
        CliError::validation_failed(format!("parse theme bundle {}: {err}", path.display()))
    })?;
    if value["schema"] != THEME_BUNDLE_SCHEMA {
        return Err(CliError::validation_failed(format!(
            "unsupported theme bundle schema in {}",
            path.display()
        )));
    }
    validate_theme_collection(&value["themeCollection"], path)?;
    if value_contains_external_uri(&value) {
        return Err(CliError::validation_failed(format!(
            "theme bundle contains external URI references: {}",
            path.display()
        )));
    }
    Ok(value)
}

fn theme_state(
    theme_collection: &Value,
    resources: &[crate::pbir_themes::ReportThemeRecord],
) -> &'static str {
    if theme_collection
        .as_object()
        .is_none_or(|object| object.is_empty())
        && resources.is_empty()
    {
        "none"
    } else if resources.iter().any(|resource| resource.registered) {
        "referenced"
    } else if !theme_collection
        .as_object()
        .is_none_or(|object| object.is_empty())
    {
        "embedded"
    } else {
        "unknown"
    }
}

fn theme_name(
    theme_collection: &Value,
    resources: &[crate::pbir_themes::ReportThemeRecord],
) -> String {
    theme_collection["name"]
        .as_str()
        .or_else(|| {
            resources
                .iter()
                .find(|resource| resource.registered)
                .or_else(|| resources.first())
                .map(|resource| resource.name.as_str())
        })
        .unwrap_or("Power BI Theme")
        .to_string()
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
                return Err(
                    CliError::invalid_args(format!("unknown {command} flag: {other}"))
                        .with_hint(format!(
                            "Run `powerbi-cli {command} --project <project-dir-or.pbip> --json`."
                        ))
                        .with_suggested_command(format!(
                            "powerbi-cli {command} --project <project-dir-or.pbip> --json"
                        )),
                );
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
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report themes extract flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report themes extract --project <project-dir-or.pbip> --out theme-bundle.json --json`.")
                .with_suggested_command(
                    "powerbi-cli report themes extract --project <project-dir-or.pbip> --out theme-bundle.json --json",
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
            "--bundle" | "--style-bundle" | "--theme-bundle" => {
                options.bundle = Some(PathBuf::from(take_value(args, &mut i, "--bundle")?));
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir(out_dir))?;
            }
            "--theme" => {
                return Err(CliError::invalid_args(
                    "report themes apply expects --bundle, not a raw theme JSON file in this slice",
                )
                .with_hint("Use `report themes extract` to create a raw-preserving bundle from an existing PBIP.")
                .with_suggested_command(
                    "powerbi-cli report themes extract --project <source-project> --out theme-bundle.json --json",
                ));
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report themes apply flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report themes\"` for exact flags.",
                )
                .with_suggested_command("powerbi-cli --json capabilities --for \"report themes\""));
            }
        }
    }
    Ok(options)
}

fn parse_preset_args(args: &[String]) -> CliResult<PresetOptions> {
    let mut options = PresetOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--preset" => options.preset = Some(take_value(args, &mut i, "--preset")?),
            "--include-bundle" | "--includeBundle" => {
                options.include_bundle = true;
                i += 1;
            }
            other if !other.starts_with('-') && options.preset.is_none() => {
                options.preset = Some(other.to_string());
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report themes presets show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report themes presets show --preset risk-dashboard --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report themes presets show --preset risk-dashboard --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_apply_preset_args(args: &[String]) -> CliResult<ApplyPresetOptions> {
    let mut options = ApplyPresetOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--preset" | "--theme" | "--style" => {
                options.preset = Some(take_value(args, &mut i, "--preset")?);
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir(out_dir))?;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report themes apply-preset flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report themes apply-preset --project <project-dir-or.pbip> --preset risk-dashboard --dry-run --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report themes apply-preset --project <project-dir-or.pbip> --preset risk-dashboard --dry-run --json",
                ));
            }
        }
    }
    Ok(options)
}

fn clean_registered_resource_path(value: &str) -> CliResult<PathBuf> {
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') || normalized.contains("..") {
        return Err(CliError::validation_failed(format!(
            "theme bundle resource path must be relative inside the report folder: {value}"
        )));
    }
    let lowered = normalized.to_ascii_lowercase();
    if !lowered.starts_with("staticresources/registeredresources/") || !lowered.ends_with(".json") {
        return Err(CliError::validation_failed(format!(
            "theme bundle resource path must be a JSON file under StaticResources/RegisteredResources: {value}"
        )));
    }
    let mut path = PathBuf::new();
    for part in normalized.split('/') {
        if !part.is_empty() && part != "." {
            path.push(part);
        }
    }
    Ok(path)
}

const BUILTIN_THEME_PRESETS: &[BuiltinThemePreset] = &[
    BuiltinThemePreset {
        id: "risk-dashboard",
        name: "powerbi-cli Risk Dashboard",
        summary: "Readable operational dashboard palette with strong status contrast and neutral canvas colors.",
        colors: &[
            "#2563EB", "#DC2626", "#16A34A", "#F59E0B", "#7C3AED", "#0891B2", "#4B5563", "#DB2777",
        ],
        background: "#FFFFFF",
        foreground: "#111827",
        table_accent: "#2563EB",
    },
    BuiltinThemePreset {
        id: "neutral-ops",
        name: "powerbi-cli Neutral Ops",
        summary: "Low-noise operations palette for dense dashboards and repeated review.",
        colors: &[
            "#1F77B4", "#D62728", "#2CA02C", "#9467BD", "#8C564B", "#E377C2", "#7F7F7F", "#BCBD22",
        ],
        background: "#FFFFFF",
        foreground: "#1F2937",
        table_accent: "#1F77B4",
    },
];

fn builtin_theme_preset(id: &str) -> CliResult<BuiltinThemePreset> {
    let normalized = id.to_ascii_lowercase();
    BUILTIN_THEME_PRESETS
        .iter()
        .copied()
        .find(|preset| preset.id == normalized.as_str())
        .ok_or_else(|| {
            CliError::invalid_args(format!("unknown theme preset: {id}"))
                .with_hint("Run `report themes presets list` to discover built-in preset ids.")
                .with_suggested_command("powerbi-cli report themes presets list --json")
        })
}

fn builtin_theme_bundle(preset: BuiltinThemePreset) -> Value {
    let relative_path = format!(
        "StaticResources/RegisteredResources/powerbi-cli-{}.json",
        preset.id
    );
    let resource_name = format!("powerbi-cli-{}.json", preset.id);
    let theme_json = json!({
        "name": resource_name,
        "dataColors": preset.colors,
        "background": preset.background,
        "foreground": preset.foreground,
        "tableAccent": preset.table_accent,
        "visualStyles": {
            "*": {
                "*": {
                    "title": [{
                        "show": true,
                        "fontColor": { "solid": { "color": preset.foreground } },
                        "fontSize": 11
                    }],
                    "categoryAxis": [{
                        "labelColor": { "solid": { "color": preset.foreground } }
                    }],
                    "valueAxis": [{
                        "labelColor": { "solid": { "color": preset.foreground } }
                    }]
                }
            }
        }
    });
    let theme_collection = json!({
        "customTheme": {
            "name": resource_name,
            "reportVersionAtImport": REPORT_VERSION_AT_IMPORT,
            "type": REGISTERED_RESOURCES_PACKAGE
        }
    });
    json!({
        "schema": THEME_BUNDLE_SCHEMA,
        "bundleVersion": 1,
        "sourceFingerprint": format!("builtin:{}", preset.id),
        "theme": {
            "handle": format!("theme:builtin-{}", preset.id),
            "state": "referenced",
            "name": preset.name,
            "themeCollection": theme_collection,
            "registeredThemes": [{
                "name": preset.name,
                "relativePath": relative_path,
                "themeJson": theme_json
            }]
        },
        "themeCollection": theme_collection,
        "registeredThemes": [{
            "handle": format!("theme:builtin-{}", preset.id),
            "name": preset.name,
            "relativePath": relative_path,
            "registered": true,
            "themeJson": theme_json,
            "safety": theme_safety_json(&theme_safety(&theme_json))
        }],
        "safety": {
            "containsExternalUris": false,
            "containsBinaryAssets": false,
            "copiesData": false,
            "themeCollection": theme_safety_json(&theme_safety(&theme_collection))
        }
    })
}

fn normalized_theme_collection_for_bundle(
    value: &Value,
    bundle: &Value,
    report_json: &Value,
) -> CliResult<Value> {
    let mut theme_collection = if value.is_object() {
        value.clone()
    } else {
        json!({})
    };
    let Some(first_item) = registered_resource_package_items(bundle)?
        .into_iter()
        .next()
    else {
        return Ok(theme_collection);
    };
    let item_name = first_item["name"]
        .as_str()
        .unwrap_or("powerbi-cli-theme.json")
        .to_string();
    theme_collection["customTheme"] = json!({
        "name": item_name,
        "reportVersionAtImport": theme_version_at_import(report_json)?,
        "type": REGISTERED_RESOURCES_PACKAGE
    });
    Ok(theme_collection)
}

fn theme_version_at_import(report_json: &Value) -> CliResult<Value> {
    match report_schema_major(report_json) {
        Some(3) => Ok(json!({
            "visual": REPORT_VERSION_AT_IMPORT_VISUAL,
            "page": REPORT_VERSION_AT_IMPORT_PAGE,
            "report": REPORT_VERSION_AT_IMPORT_REPORT
        })),
        Some(2) => Ok(Value::String(REPORT_VERSION_AT_IMPORT.to_string())),
        _ => Err(CliError::validation_failed(
            "report $schema does not contain a supported report schema version",
        )),
    }
}

fn upsert_registered_resource_package(report_json: &mut Value, bundle: &Value) -> CliResult<()> {
    let items = registered_resource_package_items(bundle)?;
    if items.is_empty() {
        return Ok(());
    }
    if !report_json["resourcePackages"].is_array() {
        report_json["resourcePackages"] = Value::Array(Vec::new());
    }
    let packages = report_json["resourcePackages"]
        .as_array_mut()
        .expect("resourcePackages was just made an array");
    let package = if let Some(index) = packages.iter().position(|package| {
        package["name"].as_str() == Some(REGISTERED_RESOURCES_PACKAGE)
            && package["type"].as_str() == Some(REGISTERED_RESOURCES_PACKAGE)
    }) {
        &mut packages[index]
    } else {
        packages.push(json!({
            "name": REGISTERED_RESOURCES_PACKAGE,
            "type": REGISTERED_RESOURCES_PACKAGE,
            "items": []
        }));
        packages
            .last_mut()
            .expect("registered resource package was just pushed")
    };
    if !package["items"].is_array() {
        package["items"] = Value::Array(Vec::new());
    }
    let existing = package["items"]
        .as_array_mut()
        .expect("resource package items was just made an array");
    for item in items {
        let name = item["name"].as_str().unwrap_or_default();
        let path = item["path"].as_str().unwrap_or_default();
        if let Some(position) = existing.iter().position(|candidate| {
            candidate["name"].as_str() == Some(name) || candidate["path"].as_str() == Some(path)
        }) {
            existing[position] = item;
        } else {
            existing.push(item);
        }
    }
    Ok(())
}

fn registered_resource_package_items(bundle: &Value) -> CliResult<Vec<Value>> {
    let mut items = Vec::new();
    for resource in bundle["registeredThemes"].as_array().into_iter().flatten() {
        let relative = resource["relativePath"].as_str().ok_or_else(|| {
            CliError::validation_failed("theme bundle resource is missing relativePath")
        })?;
        let item_path = resource_package_item_path(relative)?;
        let item_name = item_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                CliError::validation_failed(format!(
                    "theme bundle resource path has no file name: {relative}"
                ))
            })?
            .to_string();
        let item = json!({
            "name": item_name,
            "path": item_path.to_string_lossy().replace('\\', "/"),
            "type": CUSTOM_THEME_ITEM_TYPE
        });
        if !items.iter().any(|existing: &Value| {
            existing["name"] == item["name"] || existing["path"] == item["path"]
        }) {
            items.push(item);
        }
    }
    Ok(items)
}

fn resource_package_item_path(relative: &str) -> CliResult<PathBuf> {
    let relative_path = clean_registered_resource_path(relative)?;
    let prefix = Path::new("StaticResources").join("RegisteredResources");
    Ok(relative_path
        .strip_prefix(&prefix)
        .unwrap_or(&relative_path)
        .to_path_buf())
}

fn report_json_path(resolved: &crate::ResolvedProject) -> PathBuf {
    resolved.report_dir.join("definition").join("report.json")
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

fn set_mode(current: &mut Option<MutationMode>, next: MutationMode) -> CliResult<()> {
    if current.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one output mode: --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.")
        .with_suggested_command(
            "powerbi-cli report themes apply --project <project-dir-or.pbip> --bundle <theme-bundle.json> --dry-run --json",
        ));
    }
    *current = Some(next);
    Ok(())
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for \"report themes\"` for exact usage.",
            )
            .with_suggested_command("powerbi-cli --json capabilities --for \"report themes\"")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn normalize_action(value: &str) -> String {
    match value {
        "get" => "show",
        "export" => "extract",
        "clone" => "extract",
        "import" => "apply",
        "applyPreset" => "apply-preset",
        other => other,
    }
    .to_string()
}

fn mode_name(mode: &MutationMode) -> &'static str {
    match mode {
        MutationMode::DryRun => "dry-run",
        MutationMode::InPlace => "in-place",
        MutationMode::OutDir(_) => "out-dir",
    }
}
