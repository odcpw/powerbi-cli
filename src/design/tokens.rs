//! Versioned design-token catalog and report style token compiler.
//!
//! The token surface deliberately stays data driven.  A small, strict JSON
//! catalog is embedded at build time and user supplied token objects are
//! merged with one of the catalog sets before being validated and lowered to
//! the registered-resource theme shape Power BI consumes.

use crate::cli_support::take_value;
use crate::ops::OpOutcome;
use crate::pbir::load_report_snapshot;
use crate::pbir_themes::list_report_themes;
use crate::report_themes::apply_theme_bundle_operation;
use crate::{
    CliError, CliResult, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display, command_arg,
    read_json_value, resolve_project,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

include!(concat!(env!("OUT_DIR"), "/design_tokens.rs"));

const CATALOG_SCHEMA: &str = "powerbi-cli.tokens.v1";
const REPORT_TOKENS_SHOW_SCHEMA: &str = "powerbi-cli.report.style.tokens.show.v1";
const REPORT_TOKENS_DERIVE_SCHEMA: &str = "powerbi-cli.report.style.tokens.derive.v1";
const REGISTERED_RESOURCES_PACKAGE: &str = "RegisteredResources";
const REPORT_VERSION_AT_IMPORT: &str = "2.0.0";
const AA_RATIO: f64 = 4.5;
const TOKEN_FIELDS: &[&str] = &[
    "preset",
    "id",
    "name",
    "summary",
    "palette",
    "semantic",
    "ramps",
    "typography",
    "surfaces",
    "spacing",
    "numberFormats",
    "textClasses",
    "visualDefaults",
    "allowContrastBelowAA",
];

#[derive(Debug, Clone)]
pub(crate) struct CompiledTokens {
    pub(crate) tokens: Value,
    pub(crate) theme_bundle: Value,
    pub(crate) warnings: Vec<Value>,
    pub(crate) allow_contrast_below_aa: bool,
}

pub(crate) fn tokens_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report style tokens requires a subcommand: show or derive",
        )
        .with_hint("Use `report style tokens show --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli report style tokens show --project <project-dir-or.pbip> --json",
        ));
    };
    match action.as_str() {
        "show" | "list" => show_tokens(rest),
        "derive" | "from-report" => derive_tokens(rest),
        other => Err(CliError::invalid_args(format!(
            "unknown report style tokens command: {other}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report style tokens\"`.")
        .with_suggested_command(
            "powerbi-cli report style tokens show --project <project-dir-or.pbip> --json",
        )),
    }
}

#[derive(Debug, Default)]
struct TokenCommandOptions {
    project: Option<PathBuf>,
    preset: Option<String>,
}

fn show_tokens(args: &[String]) -> CliResult<Value> {
    let options = parse_token_command_args(args, "report style tokens show")?;
    let project_path = options.project.ok_or_else(|| {
        CliError::invalid_args("report style tokens show requires --project <project-dir-or.pbip>")
            .with_hint("Pass a PBIP project directory or .pbip path to inspect the active theme.")
            .with_suggested_command(
                "powerbi-cli report style tokens show --project <project-dir-or.pbip> --json",
            )
    })?;
    let project = resolve_project(&project_path)?;
    let catalog = token_catalog()?;
    let sets = catalog["sets"].as_array().cloned().unwrap_or_default();
    let selected_id = options
        .preset
        .as_deref()
        .map(ToOwned::to_owned)
        .or_else(|| active_token_id(&project).ok().flatten())
        .unwrap_or_else(|| "corporate-neutral".to_string());
    let selected = find_set(&sets, &selected_id)?;
    let validation = crate::validate_project(&project)?;
    let project_arg = command_arg(&project.project_dir);
    Ok(json!({
        "schema": REPORT_TOKENS_SHOW_SCHEMA,
        "ok": validation.errors.is_empty(),
        "projectDir": canonical_display(&project.project_dir),
        "pbip": canonical_display(&project.pbip_path),
        "reportDir": canonical_display(&project.report_dir),
        "catalog": {
            "schema": CATALOG_SCHEMA,
            "version": catalog["version"],
            "sets": sets.iter().map(token_set_summary).collect::<Vec<_>>()
        },
        "selected": {
            "id": selected["id"],
            "tokens": selected,
            "theme": compile_theme(selected)?
        },
        "validation": {
            "ok": validation.errors.is_empty(),
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "next": [
            format!("powerbi-cli report style tokens derive --project {project_arg} --json"),
            format!("powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir {project_arg} --json"),
            format!("powerbi-cli validate --strict {project_arg} --json")
        ]
    }))
}

fn derive_tokens(args: &[String]) -> CliResult<Value> {
    let options = parse_token_command_args(args, "report style tokens derive")?;
    if options.preset.is_some() {
        return Err(CliError::invalid_args(
            "report style tokens derive does not accept --preset/--id",
        )
        .with_hint(
            "Derive samples the active report theme; use show --preset to inspect a built-in set.",
        )
        .with_suggested_command(
            "powerbi-cli report style tokens derive --project <project-dir-or.pbip> --json",
        ));
    }
    let project_path = options.project.ok_or_else(|| {
        CliError::invalid_args(
            "report style tokens derive requires --project <project-dir-or.pbip>",
        )
        .with_hint("Pass the PBIP project directory or .pbip path whose theme should be sampled.")
        .with_suggested_command(
            "powerbi-cli report style tokens derive --project <project-dir-or.pbip> --json",
        )
    })?;
    let project = resolve_project(&project_path)?;
    let report_path = project.report_dir.join("definition").join("report.json");
    let report_json = read_json_value(&report_path)?;
    let resources = list_report_themes(&project)?;
    let active_name = report_json
        .pointer("/themeCollection/customTheme/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let resource = resources
        .iter()
        .find(|item| item.name == active_name || item.relative_path.ends_with(active_name))
        .or_else(|| resources.iter().find(|item| item.registered))
        .or_else(|| resources.first());
    let theme = resource
        .map(|item| item.theme.clone())
        .unwrap_or_else(|| json!({}));
    let snapshot = load_report_snapshot(&project)?;
    let visual_colors = collect_visual_colors(&snapshot);
    let catalog = token_catalog()?;
    let sets = catalog["sets"]
        .as_array()
        .ok_or_else(|| CliError::unexpected("embedded design token catalog has no sets array"))?;
    let fallback = find_set(sets, "corporate-neutral")?;
    let mut palette_values = theme["dataColors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|value| is_hex_color(value))
        .map(|value| value.to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    palette_values.extend(visual_colors.iter().cloned());
    let palette = palette_values.into_iter().collect::<Vec<_>>();
    let mut tokens = fallback.clone();
    if !palette.is_empty() {
        tokens["palette"] = Value::Array(palette.into_iter().map(Value::String).collect());
    }
    for field in ["background", "foreground"] {
        if let Some(value) = theme.get(field).filter(|value| value.is_string()) {
            let surface = if field == "background" {
                "page"
            } else {
                "foreground"
            };
            if surface == "page" {
                tokens["surfaces"]["page"] = value.clone();
            }
            if surface == "foreground" {
                tokens["textClasses"]["title"]["color"] = value.clone();
            }
        }
    }
    if let Some(text_classes) = theme.get("textClasses").filter(|value| value.is_object()) {
        tokens["textClasses"] = text_classes.clone();
    }
    if let Some(visual_defaults) = theme.get("visualStyles").filter(|value| value.is_object()) {
        // Theme resources are formatting-only, but malformed or hand-authored
        // resources can still carry free-form text keys. Preserve formatting
        // values while dropping report-literal fields before returning a
        // derived token proposal.
        tokens["visualDefaults"] = sanitize_visual_defaults(visual_defaults);
    }
    canonicalize_object(&mut tokens);
    let token_validation = validate_token_set(&tokens, "/tokens")?;
    let project_arg = command_arg(&project.project_dir);
    Ok(json!({
        "schema": REPORT_TOKENS_DERIVE_SCHEMA,
        "ok": true,
        "projectDir": canonical_display(&project.project_dir),
        "pbip": canonical_display(&project.pbip_path),
        "reportDir": canonical_display(&project.report_dir),
        "source": {
            "themeHandle": resource.map(|item| item.handle.clone()),
            "themeName": resource.map(|item| item.name.clone()),
            "themeFingerprint": resource.map(|item| fingerprint(&item.theme)),
            "visualCount": snapshot.pages.iter().map(|page| page.visuals.len()).sum::<usize>(),
            "sampledColorCount": visual_colors.len()
        },
        "tokens": token_validation.normalized,
        "next": [
            format!("powerbi-cli report style tokens show --project {project_arg} --json"),
            "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json".to_string()
        ]
    }))
}

fn parse_token_command_args(args: &[String], command: &str) -> CliResult<TokenCommandOptions> {
    let mut options = TokenCommandOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut index, "--project")?));
            }
            "--preset" | "--id" => {
                options.preset = Some(take_value(args, &mut index, "--preset")?);
            }
            other => {
                return Err(
                    CliError::invalid_args(format!("unknown {command} flag: {other}"))
                        .with_hint(
                            "Run `powerbi-cli --json capabilities --for \"report style tokens\"`.",
                        )
                        .with_suggested_command(format!(
                            "powerbi-cli {command} --project <project-dir-or.pbip> --json"
                        )),
                );
            }
        }
    }
    Ok(options)
}

/// Compile a dashboard-spec `style.tokens` object.  Missing fields inherit the
/// corporate-neutral set, while every supplied key is checked against the
/// strict token vocabulary before any project is touched.
pub(crate) fn compile_tokens(value: &Value) -> CliResult<CompiledTokens> {
    let object = value.as_object().ok_or_else(|| {
        CliError::invalid_args("style.tokens must be an object")
            .with_pointer("/style/tokens")
            .with_field("style.tokens")
    })?;
    for key in object.keys() {
        if !TOKEN_FIELDS.iter().any(|field| field == key) {
            return Err(
                CliError::invalid_args(format!("unknown style token field `{key}`"))
                    .with_pointer(format!("/style/tokens/{key}"))
                    .with_field("style.tokens")
                    .with_hint(
                        "Use `report style tokens show` to inspect the versioned token catalog.",
                    ),
            );
        }
    }
    let catalog = token_catalog()?;
    let sets = catalog["sets"]
        .as_array()
        .ok_or_else(|| CliError::unexpected("embedded design token catalog has no sets array"))?;
    let preset = object
        .get("preset")
        .and_then(Value::as_str)
        .unwrap_or("corporate-neutral");
    let base = find_set(sets, preset)?;
    let mut normalized = base.clone();
    let mut overrides = object.clone();
    overrides.remove("preset");
    expand_typography_scale(&mut overrides, base)?;
    merge_values(&mut normalized, Value::Object(overrides));
    canonicalize_object(&mut normalized);
    let validation = validate_token_set(&normalized, "/style/tokens")?;
    let allow = validation
        .normalized
        .get("allowContrastBelowAA")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let failures = contrast_failures(&validation.normalized)?;
    let warnings = if failures.is_empty() {
        Vec::new()
    } else if allow {
        failures
            .iter()
            .map(|failure| {
                json!({
                    "code": crate::rules::DESIGN_CONTRAST_BELOW_AA,
                    "severity": "warning",
                    "message": failure.message,
                    "pair": failure.pair,
                    "ratio": failure.ratio
                })
            })
            .collect()
    } else {
        let failure = &failures[0];
        return Err(CliError::new(
            crate::rules::DESIGN_CONTRAST_BELOW_AA,
            EXIT_VALIDATION_FAILED,
            format!(
                "WCAG AA contrast failed for {} (ratio {:.2}; required {:.1}:1)",
                failure.pair, failure.ratio, AA_RATIO
            ),
        )
        .with_pointer("/style/tokens")
        .with_field("style.tokens")
        .with_reason(format!(
            "foreground/background pair {} is below WCAG AA",
            failure.pair
        ))
        .with_hint(
            "Choose a higher-contrast token or set allowContrastBelowAA:true to record a warning.",
        ));
    };
    let theme_bundle = compile_theme_value(&validation.normalized)?;
    Ok(CompiledTokens {
        tokens: validation.normalized,
        theme_bundle,
        warnings,
        allow_contrast_below_aa: allow,
    })
}

/// Dashboard v2 expresses typography scale as a multiplier, while the token
/// catalog stores the resolved point size for each text class. Expand the
/// shorthand at this boundary so both contracts remain strict and the theme
/// receives deterministic, fully resolved sizes.
fn expand_typography_scale(overrides: &mut Map<String, Value>, base: &Value) -> CliResult<()> {
    let Some(typography) = overrides
        .get_mut("typography")
        .and_then(Value::as_object_mut)
    else {
        return Ok(());
    };
    let Some(multiplier) = typography.get("scale").and_then(Value::as_f64) else {
        return Ok(());
    };
    if !multiplier.is_finite() || multiplier <= 0.0 {
        return Err(
            CliError::invalid_args("typography.scale must be a finite positive number")
                .with_pointer("/style/tokens/typography/scale")
                .with_field("typography.scale"),
        );
    }
    let base_scale = base["typography"]["scale"].as_object().ok_or_else(|| {
        CliError::unexpected("built-in design token typography scale must be an object")
    })?;
    let mut resolved = Map::new();
    for (class, size) in base_scale {
        let point_size = size.as_f64().ok_or_else(|| {
            CliError::unexpected("built-in design token typography sizes must be numbers")
        })? * multiplier;
        let number = serde_json::Number::from_f64(point_size).ok_or_else(|| {
            CliError::invalid_args("typography.scale must produce finite positive point sizes")
                .with_pointer("/style/tokens/typography/scale")
                .with_field("typography.scale")
        })?;
        resolved.insert(class.clone(), Value::Number(number));
    }
    typography.insert("scale".to_string(), Value::Object(resolved));
    Ok(())
}

/// Validate the supplied token object without running the contrast policy.
/// The dashboard-spec walker calls this after its field walk so dynamic maps
/// such as text classes and visual defaults still receive the same structural
/// checks as report build.
pub(crate) fn validate_style_tokens_shape(value: &Value) -> CliResult<()> {
    let object = value.as_object().ok_or_else(|| {
        CliError::invalid_args("style.tokens must be an object")
            .with_pointer("/style/tokens")
            .with_field("style.tokens")
    })?;
    for key in object.keys() {
        if !TOKEN_FIELDS.iter().any(|field| field == key) {
            return Err(
                CliError::invalid_args(format!("unknown style token field `{key}`"))
                    .with_pointer(format!("/style/tokens/{key}"))
                    .with_field("style.tokens"),
            );
        }
    }
    let catalog = token_catalog()?;
    let sets = catalog["sets"]
        .as_array()
        .ok_or_else(|| CliError::unexpected("embedded design token catalog has no sets array"))?;
    let preset = object
        .get("preset")
        .and_then(Value::as_str)
        .unwrap_or("corporate-neutral");
    let base = find_set(sets, preset)?;
    let mut normalized = base.clone();
    let mut overrides = object.clone();
    overrides.remove("preset");
    expand_typography_scale(&mut overrides, base)?;
    merge_values(&mut normalized, Value::Object(overrides));
    validate_token_set(&normalized, "/style/tokens")?;
    Ok(())
}

/// Lower normalized style tokens into the registered-resource theme bundle
/// consumed by the existing report theme mutation boundary.
pub(crate) fn compile_theme(tokens: &Value) -> CliResult<Value> {
    compile_theme_value(&validate_token_set(tokens, "/tokens")?.normalized)
}

fn compile_theme_value(tokens: &Value) -> CliResult<Value> {
    let id = tokens["id"].as_str().unwrap_or("corporate-neutral");
    let resource_name = format!("powerbi-cli-tokens-{id}.json");
    let relative_path = format!("StaticResources/RegisteredResources/{resource_name}");
    let mut visual_defaults = tokens["visualDefaults"].clone();
    let compact_above = tokens["numberFormats"]["compactAbove"].clone();
    set_display_units(&mut visual_defaults, &compact_above);
    let text_classes = compiled_text_classes(tokens);
    let foreground = text_classes["title"]
        .get("color")
        .filter(|value| value.is_string())
        .cloned()
        .unwrap_or_else(|| tokens["semantic"]["neutral"].clone());
    let theme_json = json!({
        "name": resource_name,
        "dataColors": tokens["palette"],
        "background": tokens["surfaces"]["page"],
        "foreground": foreground,
        "tableAccent": tokens["palette"].as_array().and_then(|colors| colors.first()).cloned().unwrap_or(Value::Null),
        "textClasses": text_classes,
        "visualStyles": visual_defaults
    });
    let theme_collection = json!({
        "customTheme": {
            "name": resource_name,
            "reportVersionAtImport": REPORT_VERSION_AT_IMPORT,
            "type": REGISTERED_RESOURCES_PACKAGE
        }
    });
    Ok(json!({
        "schema": crate::pbir_themes::THEME_BUNDLE_SCHEMA,
        "bundleVersion": 1,
        "sourceFingerprint": format!("tokens:{id}"),
        "theme": {
            "handle": format!("theme:tokens-{id}"),
            "state": "referenced",
            "name": tokens["name"],
            "themeCollection": theme_collection,
            "registeredThemes": [{
                "name": tokens["name"],
                "relativePath": relative_path,
                "themeJson": theme_json
            }]
        },
        "themeCollection": theme_collection,
        "registeredThemes": [{
            "handle": format!("theme:tokens-{id}"),
            "name": tokens["name"],
            "relativePath": relative_path,
            "registered": true,
            "themeJson": theme_json,
            "safety": crate::pbir_themes::theme_safety_json(&crate::pbir_themes::theme_safety(&theme_json))
        }],
        "safety": {
            "containsExternalUris": false,
            "containsBinaryAssets": false,
            "copiesData": false,
            "themeCollection": crate::pbir_themes::theme_safety_json(&crate::pbir_themes::theme_safety(&theme_collection))
        }
    }))
}

fn compiled_text_classes(tokens: &Value) -> Value {
    let mut classes = tokens["textClasses"].clone();
    let family = tokens["typography"]["family"].clone();
    let scale = tokens["typography"]["scale"].as_object();
    if let Some(classes_object) = classes.as_object_mut() {
        for (class, value) in classes_object {
            let Some(class_object) = value.as_object_mut() else {
                continue;
            };
            class_object.insert("fontFamily".to_string(), family.clone());
            if let Some(size) = scale.and_then(|scale| scale.get(class)) {
                class_object.insert("fontSize".to_string(), size.clone());
            }
        }
    }
    canonicalize_object(&mut classes);
    classes
}

/// Apply a compiled token theme through the one existing registered-resource
/// mutation boundary.  Build and future style commands can share this helper
/// without reimplementing PBIR resource package semantics.
pub(crate) fn apply_compiled_theme(
    compiled: &CompiledTokens,
    project: &ResolvedProject,
) -> CliResult<OpOutcome> {
    let outcome = apply_theme_bundle_operation(&compiled.theme_bundle, project)?;
    if compiled.allow_contrast_below_aa {
        record_contrast_waiver(project, &compiled.warnings)?;
    }
    Ok(outcome)
}

fn record_contrast_waiver(project: &ResolvedProject, warnings: &[Value]) -> CliResult<()> {
    let path = project.project_dir.join("POWERBI_HANDOFF.md");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let marker = "## Design token contrast waiver";
    if existing.contains(marker) {
        return Ok(());
    }
    let details = warnings
        .iter()
        .map(|warning| {
            let pair = warning["pair"].as_str().unwrap_or("unspecified pair");
            let message = warning["message"].as_str().unwrap_or("contrast below AA");
            format!("- `{pair}`: {message}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let suffix = format!(
        "\n{marker}\n\nThis build used `style.tokens.allowContrastBelowAA: true`; review the named `design.contrast_below_aa` warnings before Desktop handoff.\n\n{details}\n"
    );
    fs::write(&path, format!("{existing}{suffix}"))
        .map_err(|error| CliError::unexpected(format!("write {}: {error}", path.display())))
}

#[derive(Debug)]
struct TokenValidation {
    normalized: Value,
}

fn validate_token_set(value: &Value, pointer: &str) -> CliResult<TokenValidation> {
    let object = value.as_object().ok_or_else(|| {
        CliError::invalid_args("design token set must be an object")
            .with_pointer(pointer)
            .with_field("style.tokens")
    })?;
    let required = [
        "id",
        "name",
        "summary",
        "palette",
        "semantic",
        "ramps",
        "typography",
        "surfaces",
        "spacing",
        "numberFormats",
        "textClasses",
        "visualDefaults",
    ];
    for field in required {
        if !object.contains_key(field) {
            return Err(
                CliError::invalid_args(format!("design token set is missing `{field}`"))
                    .with_pointer(format!("{pointer}/{field}")),
            );
        }
    }
    for key in object.keys() {
        if !TOKEN_FIELDS.iter().any(|field| field == key) {
            return Err(
                CliError::invalid_args(format!("unknown design token field `{key}`"))
                    .with_pointer(format!("{pointer}/{key}")),
            );
        }
    }
    string_field(object, "id", pointer)?;
    validate_token_id(object["id"].as_str().unwrap_or_default(), pointer)?;
    string_field(object, "name", pointer)?;
    string_field(object, "summary", pointer)?;
    let palette = object["palette"]
        .as_array()
        .ok_or_else(|| token_type_error(pointer, "palette", "an array of hexadecimal colors"))?;
    if palette.is_empty() {
        return Err(token_type_error(
            pointer,
            "palette",
            "a non-empty array of colors",
        ));
    }
    for (index, color) in palette.iter().enumerate() {
        parse_color(color, &format!("{pointer}/palette/{index}"))?;
    }
    validate_color_map(
        &object["semantic"],
        pointer,
        "semantic",
        &["good", "bad", "neutral", "warning", "emphasis"],
    )?;
    validate_ramps(&object["ramps"], pointer)?;
    validate_typography(&object["typography"], pointer)?;
    validate_color_map(
        &object["surfaces"],
        pointer,
        "surfaces",
        &["page", "card", "border", "alt"],
    )?;
    let spacing = object["spacing"]
        .as_object()
        .ok_or_else(|| token_type_error(pointer, "spacing", "an object with unit"))?;
    if spacing.keys().any(|key| key != "unit") {
        return Err(token_type_error(
            pointer,
            "spacing",
            "an object with unit only",
        ));
    }
    if spacing["unit"].as_f64().is_none_or(|unit| unit <= 0.0) {
        return Err(token_type_error(
            pointer,
            "spacing.unit",
            "a positive number",
        ));
    }
    validate_number_formats(&object["numberFormats"], pointer)?;
    validate_text_classes(&object["textClasses"], pointer)?;
    if !object["visualDefaults"].is_object() {
        return Err(token_type_error(pointer, "visualDefaults", "an object"));
    }
    if crate::safety_scan::contains_external_uri(&object["visualDefaults"])
        || crate::safety_scan::contains_credential_like_text(
            &object["visualDefaults"],
            crate::safety_scan::CREDENTIAL_NEEDLES,
            true,
        )
    {
        return Err(CliError::invalid_args(
            "visualDefaults must not contain external URI or credential-like text",
        )
        .with_pointer(format!("{pointer}/visualDefaults"))
        .with_field("visualDefaults"));
    }
    if let Some(value) = object.get("allowContrastBelowAA")
        && !value.is_boolean()
    {
        return Err(token_type_error(
            pointer,
            "allowContrastBelowAA",
            "a boolean",
        ));
    }
    let mut normalized = value.clone();
    canonicalize_object(&mut normalized);
    Ok(TokenValidation { normalized })
}

fn validate_catalog(value: &Value) -> CliResult<()> {
    let object = value.as_object().ok_or_else(|| {
        CliError::unexpected("embedded design token catalog root must be an object")
    })?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "schema" | "version" | "sets"))
    {
        return Err(CliError::unexpected(
            "embedded design token catalog contains unknown fields",
        ));
    }
    if object["schema"] != CATALOG_SCHEMA || object["version"] != 1 {
        return Err(CliError::unexpected(
            "embedded design token catalog schema/version mismatch",
        ));
    }
    let sets = object["sets"].as_array().ok_or_else(|| {
        CliError::unexpected("embedded design token catalog sets must be an array")
    })?;
    let mut ids = BTreeSet::new();
    for (index, set) in sets.iter().enumerate() {
        let validation = validate_token_set(set, &format!("/sets/{index}"))?;
        let id = validation.normalized["id"].as_str().unwrap_or_default();
        if !ids.insert(id.to_string()) {
            return Err(CliError::unexpected(format!(
                "duplicate design token set id: {id}"
            )));
        }
    }
    for id in ["corporate-neutral", "high-contrast", "dark", "print"] {
        if !ids.contains(id) {
            return Err(CliError::unexpected(format!(
                "catalog missing built-in token set: {id}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn token_catalog() -> CliResult<Value> {
    let value = serde_json::from_str::<Value>(EMBEDDED_DESIGN_TOKEN_CATALOG).map_err(|error| {
        CliError::unexpected(format!("parse embedded design token catalog: {error}"))
    })?;
    validate_catalog(&value)?;
    Ok(value)
}

fn token_set_summary(value: &Value) -> Value {
    json!({
        "id": value["id"],
        "name": value["name"],
        "summary": value["summary"],
        "palette": value["palette"],
        "semantic": value["semantic"],
        "ramps": value["ramps"],
        "typography": value["typography"],
        "surfaces": value["surfaces"],
        "spacing": value["spacing"],
        "numberFormats": value["numberFormats"],
        "textClasses": value["textClasses"],
        "visualDefaults": value["visualDefaults"]
    })
}

fn find_set<'a>(sets: &'a [Value], id: &str) -> CliResult<&'a Value> {
    sets.iter()
        .find(|set| {
            set["id"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(id))
        })
        .ok_or_else(|| {
            CliError::invalid_args(format!("unknown design token set: {id}"))
                .with_hint(
                    "Use `report style tokens show --project <project> --json` to list built-ins.",
                )
                .with_suggested_command(
                    "powerbi-cli report style tokens show --project <project-dir-or.pbip> --json",
                )
        })
}

fn active_token_id(project: &ResolvedProject) -> CliResult<Option<String>> {
    let report = read_json_value(&project.report_dir.join("definition").join("report.json"))?;
    let name = report
        .pointer("/themeCollection/customTheme/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let prefix = "powerbi-cli-tokens-";
    Ok(name
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(".json"))
        .map(ToOwned::to_owned))
}

fn string_field(object: &Map<String, Value>, field: &str, pointer: &str) -> CliResult<String> {
    object[field]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| token_type_error(pointer, field, "a non-empty string"))
}

fn validate_token_id(id: &str, pointer: &str) -> CliResult<()> {
    if id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        && !id.contains("..")
    {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "design token id must contain only ASCII letters, numbers, '.', '_' or '-'",
    )
    .with_pointer(format!("{pointer}/id"))
    .with_field("id"))
}

fn token_type_error(pointer: &str, field: &str, expected: &str) -> CliError {
    CliError::invalid_args(format!("{field} must be {expected}"))
        .with_pointer(format!("{pointer}/{}", field.replace('.', "/")))
        .with_field(field)
}

fn validate_color_map(
    value: &Value,
    pointer: &str,
    field: &str,
    required: &[&str],
) -> CliResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| token_type_error(pointer, field, "an object of colors"))?;
    for key in object.keys() {
        if !required.iter().any(|expected| expected == key) {
            return Err(token_type_error(
                &format!("{pointer}/{field}"),
                key,
                "one of the documented semantic/surface color names",
            ));
        }
    }
    for key in required {
        if !object.contains_key(*key) {
            return Err(
                CliError::invalid_args(format!("{field} is missing `{key}`"))
                    .with_pointer(format!("{pointer}/{field}/{key}")),
            );
        }
        parse_color(&object[*key], &format!("{pointer}/{field}/{key}"))?;
    }
    Ok(())
}

fn validate_ramps(value: &Value, pointer: &str) -> CliResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| token_type_error(pointer, "ramps", "an object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "sequential" | "diverging") {
            return Err(token_type_error(
                pointer,
                "ramps",
                "sequential/diverging arrays only",
            ));
        }
    }
    for field in ["sequential", "diverging"] {
        let values = object[field]
            .as_array()
            .ok_or_else(|| token_type_error(&format!("{pointer}/ramps"), field, "an array"))?;
        if values.len() < 2 {
            return Err(CliError::invalid_args(format!(
                "ramps.{field} requires at least two colors"
            ))
            .with_pointer(format!("{pointer}/ramps/{field}")));
        }
        for (index, color) in values.iter().enumerate() {
            parse_color(color, &format!("{pointer}/ramps/{field}/{index}"))?;
        }
    }
    Ok(())
}

fn validate_typography(value: &Value, pointer: &str) -> CliResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| token_type_error(pointer, "typography", "an object"))?;
    for key in object.keys() {
        if !matches!(key.as_str(), "family" | "scale") {
            return Err(token_type_error(
                pointer,
                "typography",
                "family and scale only",
            ));
        }
    }
    if object["family"]
        .as_str()
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(token_type_error(
            pointer,
            "typography.family",
            "a non-empty string",
        ));
    }
    if !object["scale"].is_object() {
        return Err(token_type_error(
            pointer,
            "typography.scale",
            "an object of point sizes",
        ));
    }
    if let Some(scale) = object["scale"].as_object() {
        for (class, size) in scale {
            if size.as_f64().is_none_or(|size| size <= 0.0) {
                return Err(CliError::invalid_args(format!(
                    "typography.scale.{class} must be a positive number"
                ))
                .with_pointer(format!("{pointer}/typography/scale/{class}")));
            }
        }
    }
    Ok(())
}

fn validate_number_formats(value: &Value, pointer: &str) -> CliResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| token_type_error(pointer, "numberFormats", "an object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "currency" | "percent" | "integer" | "compactAbove"
        ) {
            return Err(token_type_error(
                pointer,
                "numberFormats",
                "documented format names only",
            ));
        }
    }
    for field in ["currency", "percent", "integer"] {
        if object[field]
            .as_str()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(token_type_error(
                &format!("{pointer}/numberFormats"),
                field,
                "a non-empty format string",
            ));
        }
    }
    if object["compactAbove"]
        .as_f64()
        .is_none_or(|value| value < 0.0)
    {
        return Err(token_type_error(
            pointer,
            "numberFormats.compactAbove",
            "a number",
        ));
    }
    Ok(())
}

fn validate_text_classes(value: &Value, pointer: &str) -> CliResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| token_type_error(pointer, "textClasses", "an object"))?;
    for (class, item) in object {
        let item = item.as_object().ok_or_else(|| {
            token_type_error(&format!("{pointer}/textClasses"), class, "an object")
        })?;
        if !item["fontSize"].is_number() {
            return Err(token_type_error(
                &format!("{pointer}/textClasses"),
                &format!("{class}.fontSize"),
                "a number",
            ));
        }
        parse_color(
            &item["color"],
            &format!("{pointer}/textClasses/{class}/color"),
        )?;
    }
    Ok(())
}

#[derive(Debug)]
struct ContrastFailure {
    pair: String,
    ratio: f64,
    message: String,
}

fn contrast_failures(tokens: &Value) -> CliResult<Vec<ContrastFailure>> {
    let mut foregrounds = Vec::new();
    if let Some(foreground) = tokens["textClasses"]["title"].get("color") {
        foregrounds.push(("textClasses.title.color".to_string(), foreground));
    }
    for field in ["good", "bad", "neutral", "warning", "emphasis"] {
        foregrounds.push((format!("semantic.{field}"), &tokens["semantic"][field]));
    }
    if let Some(classes) = tokens["textClasses"].as_object() {
        for (class, value) in classes {
            if class == "title" {
                continue;
            }
            if let Some(color) = value.get("color") {
                foregrounds.push((format!("textClasses.{class}.color"), color));
            }
        }
    }
    let mut visual_colors = Vec::new();
    collect_color_values(
        &tokens["visualDefaults"],
        "visualDefaults",
        &mut visual_colors,
    );
    for (path, color) in visual_colors {
        foregrounds.push((path, color));
    }
    let mut failures = Vec::new();
    for (surface, background_value) in [
        ("page", &tokens["surfaces"]["page"]),
        ("card", &tokens["surfaces"]["card"]),
        ("alt", &tokens["surfaces"]["alt"]),
    ] {
        let background = parse_color(
            background_value,
            &format!("/style/tokens/surfaces/{surface}"),
        )?;
        for (foreground_path, color) in &foregrounds {
            let foreground = parse_color(color, &format!("/style/tokens/{foreground_path}"))?;
            let ratio = contrast_ratio(foreground, background);
            if ratio < AA_RATIO {
                let pair = format!("{foreground_path} vs surfaces.{surface}");
                failures.push(ContrastFailure {
                    message: format!(
                        "{pair} has contrast ratio {:.2}:1 against the {surface} surface",
                        ratio
                    ),
                    pair,
                    ratio,
                });
            }
        }
    }
    Ok(failures)
}

fn collect_color_values<'a>(value: &'a Value, path: &str, output: &mut Vec<(String, &'a Value)>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if key.to_ascii_lowercase().contains("color") && child.is_string() {
                    output.push((child_path.clone(), child));
                }
                collect_color_values(child, &child_path, output);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                collect_color_values(child, &format!("{path}[{index}]"), output);
            }
        }
        _ => {}
    }
}

fn parse_color(value: &Value, pointer: &str) -> CliResult<[f64; 3]> {
    let raw = value.as_str().ok_or_else(|| {
        CliError::invalid_args(format!(
            "color at {pointer} must be a #RGB or #RRGGBB string"
        ))
        .with_pointer(pointer)
    })?;
    let hex = raw.strip_prefix('#').ok_or_else(|| {
        CliError::invalid_args(format!("color at {pointer} must start with #"))
            .with_pointer(pointer)
    })?;
    let expanded = if hex.len() == 3 {
        hex.chars().flat_map(|ch| [ch, ch]).collect::<String>()
    } else {
        hex.to_string()
    };
    if expanded.len() != 6 || !expanded.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(CliError::invalid_args(format!(
            "color at {pointer} must be a #RGB or #RRGGBB string"
        ))
        .with_pointer(pointer));
    }
    let mut rgb = [0.0; 3];
    for (index, chunk) in expanded.as_bytes().chunks(2).enumerate() {
        let text = std::str::from_utf8(chunk).expect("hex is ASCII");
        let value = u8::from_str_radix(text, 16).expect("hex was validated");
        rgb[index] = f64::from(value) / 255.0;
    }
    Ok(rgb)
}

pub(crate) fn contrast_ratio(foreground: [f64; 3], background: [f64; 3]) -> f64 {
    let foreground = relative_luminance(foreground);
    let background = relative_luminance(background);
    let (lighter, darker) = if foreground > background {
        (foreground, background)
    } else {
        (background, foreground)
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(rgb: [f64; 3]) -> f64 {
    let linear = rgb.map(|channel| {
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
}

fn set_display_units(value: &mut Value, compact_above: &Value) {
    if let Some(object) = value.as_object_mut() {
        for visual in object.values_mut() {
            if let Some(visual_object) = visual.as_object_mut() {
                for (slot, properties) in visual_object {
                    if slot.eq_ignore_ascii_case("labels")
                        && let Some(items) = properties.as_array_mut()
                    {
                        for item in items {
                            if let Some(item_object) = item.as_object_mut() {
                                item_object
                                    .insert("displayUnits".to_string(), compact_above.clone());
                            }
                        }
                    }
                }
            }
        }
    }
}

fn merge_values(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(target), Value::Object(source)) => {
            for (key, value) in source {
                if let Some(existing) = target.get_mut(&key) {
                    merge_values(existing, value);
                } else {
                    target.insert(key, value);
                }
            }
        }
        (target, source) => *target = source,
    }
}

fn canonicalize_object(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for child in object.values_mut() {
                canonicalize_object(child);
            }
            let sorted = object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>();
            *object = sorted.into_iter().collect();
        }
        Value::Array(items) => {
            for child in items {
                canonicalize_object(child);
            }
        }
        _ => {}
    }
}

fn is_hex_color(value: &str) -> bool {
    let hex = value.strip_prefix('#').unwrap_or_default();
    matches!(hex.len(), 3 | 6) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn collect_visual_colors(snapshot: &crate::pbir::ReportSnapshot) -> BTreeSet<String> {
    let mut colors = BTreeSet::new();
    for visual in snapshot.pages.iter().flat_map(|page| page.visuals.iter()) {
        let Some(path) = visual.path.as_ref() else {
            continue;
        };
        let Ok(value) = read_json_value(path) else {
            continue;
        };
        collect_report_format_colors(&value, &mut colors);
    }
    colors
}

fn collect_report_format_colors(value: &Value, colors: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (child_key, child) in object {
                let lower = child_key.to_ascii_lowercase();
                let allowed = lower.contains("color")
                    || matches!(lower.as_str(), "background" | "fill" | "outline");
                if allowed
                    && let Some(text) = child.as_str()
                    && is_hex_color(text)
                {
                    colors.insert(text.to_ascii_uppercase());
                }
                collect_report_format_colors(child, colors);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_report_format_colors(child, colors);
            }
        }
        _ => {}
    }
}

fn sanitize_visual_defaults(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut sanitized = Map::new();
            for (key, child) in object {
                let lower = key.to_ascii_lowercase();
                // These keys carry report-authored literal text rather than
                // reusable formatting. A `title` formatting object remains
                // useful, but a scalar title value is omitted below.
                if matches!(
                    lower.as_str(),
                    "text" | "displayname" | "name" | "titletext" | "query" | "expression"
                ) || (lower == "title" && child.is_string())
                {
                    continue;
                }
                sanitized.insert(key.clone(), sanitize_visual_defaults(child));
            }
            Value::Object(sanitized)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(sanitize_visual_defaults)
                .collect::<Vec<_>>(),
        ),
        _ => value.clone(),
    }
}

fn fingerprint(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_strict_and_contains_required_sets() {
        let catalog = token_catalog().expect("catalog");
        assert_eq!(catalog["schema"], CATALOG_SCHEMA);
        assert_eq!(catalog["sets"].as_array().expect("sets").len(), 4);
    }

    #[test]
    fn wcag_ratio_matches_known_black_white_pair() {
        let black = parse_color(&json!("#000000"), "/black").expect("black");
        let white = parse_color(&json!("#ffffff"), "/white").expect("white");
        assert!((contrast_ratio(black, white) - 21.0).abs() < 0.01);
    }

    #[test]
    fn every_builtin_token_set_passes_contrast() {
        let catalog = token_catalog().expect("catalog");
        for set in catalog["sets"].as_array().expect("sets") {
            let compiled = compile_tokens(set).expect("built-in contrast");
            assert!(compiled.warnings.is_empty(), "{}", set["id"]);
        }
    }

    #[test]
    fn low_contrast_requires_explicit_waiver() {
        let value = json!({
            "preset": "corporate-neutral",
            "surfaces": {"page": "#ffffff"},
            "semantic": {"good": "#ffffff"}
        });
        let error = compile_tokens(&value).expect_err("contrast must fail");
        assert_eq!(error.code, "design.contrast_below_aa");
        assert!(error.message.contains("semantic.good"));
        let allowed = json!({
            "preset": "corporate-neutral",
            "allowContrastBelowAA": true,
            "surfaces": {"page": "#ffffff"},
            "semantic": {"good": "#ffffff"}
        });
        let compiled = compile_tokens(&allowed).expect("waiver");
        assert_eq!(compiled.warnings[0]["code"], "design.contrast_below_aa");
    }
}
