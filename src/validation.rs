//! Native PBIP, PBIR, and TMDL project validation.

use crate::pbir_bindings::visual_query_state_errors;
use crate::pbir_visual_factory::{BETWEEN_SLICER_MIN_HEIGHT, SLICER_MIN_HEIGHT};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    lint, microsoft, read_json_value, relationship_tmdl, resolve_project, tmdl, walkdir_entry,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Default)]
pub(crate) struct ValidationReport {
    pub(crate) errors: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) json_files_checked: usize,
    pub(crate) pages: usize,
    pub(crate) visuals: usize,
    pub(crate) bound_visuals: usize,
    pub(crate) tables: usize,
    pub(crate) measures: usize,
    pub(crate) relationships: usize,
}

pub(crate) fn validate_command(args: &[String]) -> CliResult<Value> {
    let options = parse_validate_args(args)?;
    if options.strict && options.backend == ValidationBackend::MicrosoftReport {
        return Err(CliError::invalid_args(
            "--strict is a native lint option and cannot be used with --backend microsoft-report",
        )
        .with_hint(
            "Use --backend all to run strict native lint and the official validator together.",
        )
        .with_suggested_command(
            "powerbi-cli validate <project-dir-or.pbip> --strict --backend all --json",
        ));
    }
    let resolved = resolve_project(&options.path)?;
    match options.backend {
        ValidationBackend::Native => native_validation_output(&resolved, options.strict),
        ValidationBackend::MicrosoftReport => {
            let official = microsoft::validate_official_report(&resolved)?;
            let ok = official["ok"].as_bool().unwrap_or(false);
            Ok(json!({
                "schema": "powerbi-cli.validate.microsoft-report.v1",
                "ok": ok,
                "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
                "strict": false,
                "backend": "microsoft-report",
                "projectDir": canonical_display(&resolved.project_dir),
                "pbip": canonical_display(&resolved.pbip_path),
                "reportDir": canonical_display(&resolved.report_dir),
                "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
                "counts": official["counts"],
                "warnings": official["warnings"],
                "errors": official["errors"],
                "validators": {
                    "microsoftReport": official
                }
            }))
        }
        ValidationBackend::All => {
            let mut output = native_validation_output(&resolved, options.strict)?;
            let native_ok = output["ok"].as_bool().unwrap_or(false);
            let official = microsoft::validate_official_report(&resolved)?;
            let official_ok = official["ok"].as_bool().unwrap_or(false);
            let overall_ok = native_ok && official_ok;
            output["ok"] = Value::Bool(overall_ok);
            output["exitCode"] = Value::from(if overall_ok {
                EXIT_SUCCESS
            } else {
                EXIT_VALIDATION_FAILED
            });
            output["backend"] = Value::String("all".to_string());
            output["schema"] = Value::String("powerbi-cli.validate.all.v1".to_string());
            output["validators"] = json!({
                "native": {
                    "id": "native",
                    "ok": native_ok,
                    "strict": options.strict
                },
                "microsoftReport": official
            });
            Ok(output)
        }
    }
}

fn native_validation_output(resolved: &ResolvedProject, strict: bool) -> CliResult<Value> {
    let report = validate_project(resolved)?;
    let ok = report.errors.is_empty();
    let lint = if strict && ok {
        Some(lint::lint_project(resolved, &report)?)
    } else {
        None
    };
    let lint_ok = lint
        .as_ref()
        .and_then(|value| value["ok"].as_bool())
        .unwrap_or(true);
    let overall_ok = ok && lint_ok;
    let mut output = json!({
        "ok": overall_ok,
        "exitCode": if overall_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "strict": strict,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "counts": {
            "jsonFilesChecked": report.json_files_checked,
            "pages": report.pages,
            "visuals": report.visuals,
            "boundVisuals": report.bound_visuals,
            "tables": report.tables,
            "measures": report.measures,
            "relationships": report.relationships
        },
        "warnings": report.warnings,
        "errors": report.errors
    });
    if let Some(lint) = lint {
        output["lint"] = lint;
    }
    output["backend"] = Value::String("native".to_string());
    output["validators"] = json!({
        "native": {
            "id": "native",
            "ok": overall_ok,
            "strict": strict
        }
    });
    Ok(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationBackend {
    Native,
    MicrosoftReport,
    All,
}

impl ValidationBackend {
    fn parse(value: &str) -> CliResult<Self> {
        match value {
            "native" => Ok(Self::Native),
            "microsoft-report" => Ok(Self::MicrosoftReport),
            "all" => Ok(Self::All),
            _ => Err(
                CliError::invalid_args(format!("unknown validation backend: {value}"))
                    .with_hint("Use native, microsoft-report, or all.")
                    .with_suggested_command(
                        "powerbi-cli validate <project-dir-or.pbip> --backend all --json",
                    ),
            ),
        }
    }
}

#[derive(Debug)]
struct ValidateOptions {
    path: PathBuf,
    strict: bool,
    backend: ValidationBackend,
}

fn parse_validate_args(args: &[String]) -> CliResult<ValidateOptions> {
    let mut path = None;
    let mut strict = false;
    let mut backend = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--strict" => {
                if strict {
                    return Err(CliError::invalid_args(
                        "--strict may be specified only once",
                    ));
                }
                strict = true;
                index += 1;
            }
            "--backend" => {
                if backend.is_some() {
                    return Err(CliError::invalid_args(
                        "--backend may be specified only once",
                    ));
                }
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::invalid_args("--backend requires a value"))?;
                backend = Some(ValidationBackend::parse(value)?);
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown validate flag: {other}"))
                        .with_hint(
                            "Run `powerbi-cli validate --strict <project-dir-or.pbip> --json`.",
                        )
                        .with_suggested_command(
                            "powerbi-cli validate --strict <project-dir-or.pbip> --json",
                        ),
                );
            }
            other => {
                if path.is_some() {
                    return Err(CliError::invalid_args("validate accepts exactly one path")
                        .with_hint("Run `powerbi-cli validate <project-dir-or.pbip> --json`.")
                        .with_suggested_command(
                            "powerbi-cli validate <project-dir-or.pbip> --json",
                        ));
                }
                path = Some(PathBuf::from(other));
                index += 1;
            }
        }
    }
    path.map(|path| ValidateOptions {
        path,
        strict,
        backend: backend.unwrap_or(ValidationBackend::Native),
    })
    .ok_or_else(|| {
        CliError::invalid_args("validate requires a path")
            .with_hint("Run `powerbi-cli validate <project-dir-or.pbip> --json`.")
            .with_suggested_command("powerbi-cli validate <project-dir-or.pbip> --json")
    })
}

pub(crate) fn validate_project(resolved: &ResolvedProject) -> CliResult<ValidationReport> {
    validate_project_with_runtime_policy(resolved, false)
}

pub(crate) fn validate_desktop_runtime_project(
    resolved: &ResolvedProject,
) -> CliResult<ValidationReport> {
    validate_project_with_runtime_policy(resolved, true)
}

fn validate_project_with_runtime_policy(
    resolved: &ResolvedProject,
    allow_desktop_runtime_files: bool,
) -> CliResult<ValidationReport> {
    let mut report = ValidationReport::default();
    required_file(&resolved.pbip_path, &mut report);
    required_file(&resolved.report_dir.join("definition.pbir"), &mut report);
    required_file(
        &resolved.semantic_model_dir.join("definition.pbism"),
        &mut report,
    );
    required_file(
        &resolved.report_dir.join("definition").join("version.json"),
        &mut report,
    );
    required_file(
        &resolved.report_dir.join("definition").join("report.json"),
        &mut report,
    );
    required_file(
        &resolved
            .report_dir
            .join("definition")
            .join("pages")
            .join("pages.json"),
        &mut report,
    );
    required_file(
        &resolved
            .semantic_model_dir
            .join("definition")
            .join("database.tmdl"),
        &mut report,
    );
    required_file(
        &resolved
            .semantic_model_dir
            .join("definition")
            .join("model.tmdl"),
        &mut report,
    );
    required_file(
        &resolved
            .semantic_model_dir
            .join("definition")
            .join("relationships.tmdl"),
        &mut report,
    );

    check_json_files(resolved, &mut report, allow_desktop_runtime_files)?;
    check_report_theme(resolved, &mut report)?;
    check_report_pages(resolved, &mut report)?;
    check_report_filter_configs(resolved, &mut report)?;
    check_semantic_model(resolved, &mut report)?;
    check_offline_hazards(resolved, &mut report, allow_desktop_runtime_files)?;
    Ok(report)
}

fn required_file(path: &Path, report: &mut ValidationReport) {
    if !path.is_file() {
        report
            .errors
            .push(format!("missing required file: {}", path.display()));
    }
}

fn check_json_files(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
    allow_desktop_runtime_files: bool,
) -> CliResult<()> {
    check_json_file(&resolved.pbip_path, report)?;
    for artifact_dir in [&resolved.report_dir, &resolved.semantic_model_dir] {
        check_json_files_in(artifact_dir, report, allow_desktop_runtime_files)?;
    }
    Ok(())
}

fn check_json_files_in(
    artifact_dir: &Path,
    report: &mut ValidationReport,
    allow_desktop_runtime_files: bool,
) -> CliResult<()> {
    // Required-file checks already report a missing artifact as structured
    // validation output. Do not turn that expected validation failure into an
    // exit-70 filesystem traversal error.
    if !artifact_dir.is_dir() {
        return Ok(());
    }
    let entries = WalkDir::new(artifact_dir)
        .into_iter()
        .filter_entry(|entry| {
            !allow_desktop_runtime_files
                || entry.depth() != 1
                || !entry.file_type().is_dir()
                || !entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(".pbi")
        });
    for entry in entries {
        let entry = walkdir_entry(artifact_dir, entry, "walk selected artifact JSON files")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if is_json_like(path) {
            check_json_file(path, report)?;
        }
    }
    Ok(())
}

fn check_json_file(path: &Path, report: &mut ValidationReport) -> CliResult<()> {
    report.json_files_checked += 1;
    if has_utf8_bom(path)? {
        report
            .errors
            .push(format!("JSON-like file has UTF-8 BOM: {}", path.display()));
    }
    if let Err(err) = read_json_value(path) {
        report.errors.push(err.message);
    }
    Ok(())
}

fn is_json_like(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    file_name == ".platform"
        || matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("json" | "pbip" | "pbir" | "pbism")
        )
}

fn has_utf8_bom(path: &Path) -> CliResult<bool> {
    let bytes = fs::read(path)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?;
    Ok(bytes.starts_with(&[0xEF, 0xBB, 0xBF]))
}

fn check_report_theme(resolved: &ResolvedProject, report: &mut ValidationReport) -> CliResult<()> {
    let report_json_path = resolved.report_dir.join("definition").join("report.json");
    if !report_json_path.is_file() {
        return Ok(());
    }
    let report_json = read_json_value(&report_json_path)?;
    let Some(theme_collection) = report_json.get("themeCollection") else {
        report.errors.push(format!(
            "{} is missing required themeCollection",
            report_json_path.display()
        ));
        return Ok(());
    };
    let Some(theme_collection) = theme_collection.as_object() else {
        report.errors.push(format!(
            "{} themeCollection must be an object",
            report_json_path.display()
        ));
        return Ok(());
    };
    let Some(custom_theme) = theme_collection.get("customTheme") else {
        return Ok(());
    };
    let Some(custom_theme) = custom_theme.as_object() else {
        report.errors.push(format!(
            "{} themeCollection.customTheme must be an object",
            report_json_path.display()
        ));
        return Ok(());
    };
    if custom_theme.contains_key("resource") {
        report.errors.push(format!(
            "{} themeCollection.customTheme.resource is not valid PBIR report schema metadata; use customTheme.name/reportVersionAtImport/type plus report resourcePackages",
            report_json_path.display()
        ));
    }
    for field in ["name", "type"] {
        if custom_theme
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            report.errors.push(format!(
                "{} themeCollection.customTheme.{field} must be a non-empty string",
                report_json_path.display()
            ));
        }
    }
    let theme_version_valid = match report_schema_major(&report_json) {
        Some(3) => custom_theme
            .get("reportVersionAtImport")
            .is_some_and(valid_theme_version_object),
        Some(2) => custom_theme
            .get("reportVersionAtImport")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        _ => false,
    };
    if !theme_version_valid {
        report.errors.push(format!(
            "{} themeCollection.customTheme.reportVersionAtImport must match the report schema version",
            report_json_path.display()
        ));
    }
    let theme_type = custom_theme.get("type").and_then(Value::as_str);
    if !matches!(theme_type, Some("RegisteredResources" | "SharedResources")) {
        report.errors.push(format!(
            "{} themeCollection.customTheme.type must be RegisteredResources or SharedResources",
            report_json_path.display()
        ));
    }
    if theme_type == Some("RegisteredResources") {
        check_registered_theme_resource_package(
            &report_json_path,
            &report_json,
            custom_theme,
            report,
        );
    }
    Ok(())
}

fn check_registered_theme_resource_package(
    report_json_path: &Path,
    report_json: &Value,
    custom_theme: &serde_json::Map<String, Value>,
    report: &mut ValidationReport,
) {
    let theme_name = custom_theme
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !theme_name.to_ascii_lowercase().ends_with(".json") {
        report.errors.push(format!(
            "{} RegisteredResources customTheme name `{theme_name}` must include the .json extension",
            report_json_path.display()
        ));
    }
    let Some(packages) = report_json
        .get("resourcePackages")
        .and_then(Value::as_array)
    else {
        report.errors.push(format!(
            "{} RegisteredResources customTheme requires report resourcePackages",
            report_json_path.display()
        ));
        return;
    };
    let Some(package) = packages.iter().find(|package| {
        package["name"].as_str() == Some("RegisteredResources")
            && package["type"].as_str() == Some("RegisteredResources")
    }) else {
        report.errors.push(format!(
            "{} RegisteredResources customTheme requires a RegisteredResources resource package",
            report_json_path.display()
        ));
        return;
    };
    let Some(items) = package["items"].as_array() else {
        report.errors.push(format!(
            "{} RegisteredResources resource package must contain an items array",
            report_json_path.display()
        ));
        return;
    };
    let has_theme_item = items.iter().any(|item| {
        item["type"].as_str() == Some("CustomTheme") && item["name"].as_str() == Some(theme_name)
    });
    if !has_theme_item {
        report.errors.push(format!(
            "{} RegisteredResources customTheme `{theme_name}` has no matching CustomTheme resource package item",
            report_json_path.display()
        ));
        return;
    }
    let theme_item = items.iter().find(|item| {
        item["type"].as_str() == Some("CustomTheme") && item["name"].as_str() == Some(theme_name)
    });
    let Some(theme_item) = theme_item else {
        return;
    };
    let item_path = theme_item["path"].as_str().unwrap_or_default();
    if item_path != theme_name || item_path.contains('/') || item_path.contains('\\') {
        report.errors.push(format!(
            "{} CustomTheme resource path `{item_path}` must be the same filename as `{theme_name}`",
            report_json_path.display()
        ));
        return;
    }
    let Some(report_dir) = report_json_path.parent().and_then(Path::parent) else {
        return;
    };
    let theme_path = report_dir
        .join("StaticResources")
        .join("RegisteredResources")
        .join(item_path);
    if !theme_path.is_file() {
        report.errors.push(format!(
            "{} references missing CustomTheme file {}",
            report_json_path.display(),
            theme_path.display()
        ));
        return;
    }
    match read_json_value(&theme_path) {
        Ok(theme_json) if theme_json["name"].as_str() == Some(theme_name) => {}
        Ok(theme_json) => report.errors.push(format!(
            "{} theme name `{}` does not match report customTheme name `{theme_name}`",
            theme_path.display(),
            theme_json["name"].as_str().unwrap_or_default()
        )),
        Err(err) => report.errors.push(format!(
            "{} could not be read for theme validation: {}",
            theme_path.display(),
            err.message
        )),
    }
}

fn check_report_pages(resolved: &ResolvedProject, report: &mut ValidationReport) -> CliResult<()> {
    let pages_dir = resolved.report_dir.join("definition").join("pages");
    let pages_json_path = pages_dir.join("pages.json");
    if !pages_json_path.exists() {
        return Ok(());
    }
    let pages_json = read_json_value(&pages_json_path)?;
    let mut page_order = Vec::new();
    let mut seen_pages = BTreeSet::new();
    match pages_json["pageOrder"].as_array() {
        Some(items) => {
            for item in items {
                let Some(page_name) = item.as_str() else {
                    report.errors.push(format!(
                        "{} pageOrder contains a non-string entry",
                        pages_json_path.display()
                    ));
                    continue;
                };
                if !seen_pages.insert(page_name.to_string()) {
                    report.errors.push(format!(
                        "{} pageOrder contains duplicate page: {}",
                        pages_json_path.display(),
                        page_name
                    ));
                }
                page_order.push(page_name.to_string());
            }
        }
        None => report.errors.push(format!(
            "{} has no pageOrder array",
            pages_json_path.display()
        )),
    }
    if page_order.is_empty() {
        report.warnings.push(format!(
            "{} has no pageOrder entries",
            pages_json_path.display()
        ));
    }
    if let Some(active_page_name) = pages_json["activePageName"].as_str()
        && !seen_pages.contains(active_page_name)
    {
        report.errors.push(format!(
            "{} activePageName references a page not in pageOrder: {}",
            pages_json_path.display(),
            active_page_name
        ));
    }
    for page_name in &page_order {
        let page_dir = pages_dir.join(page_name);
        let page_json_path = page_dir.join("page.json");
        if !page_json_path.is_file() {
            report.errors.push(format!(
                "pageOrder references missing page.json: {}",
                page_json_path.display()
            ));
            continue;
        }
        report.pages += 1;
        let page_json = read_json_value(&page_json_path)?;
        if page_json["name"].as_str() != Some(page_name) {
            report.errors.push(format!(
                "{} name does not match page folder {}",
                page_json_path.display(),
                page_name
            ));
        }
        check_positive_page_number(&page_json_path, &page_json, "width", report);
        check_positive_page_number(&page_json_path, &page_json, "height", report);
        let visuals_dir = page_dir.join("visuals");
        if visuals_dir.is_dir() {
            for visual_entry in fs::read_dir(&visuals_dir).map_err(|err| {
                CliError::unexpected(format!("read visuals dir {}: {err}", visuals_dir.display()))
            })? {
                let visual_entry = visual_entry.map_err(|err| {
                    CliError::unexpected(format!(
                        "read visual entry {}: {err}",
                        visuals_dir.display()
                    ))
                })?;
                if !visual_entry
                    .file_type()
                    .map_err(|err| {
                        CliError::unexpected(format!(
                            "read visual entry type {}: {err}",
                            visual_entry.path().display()
                        ))
                    })?
                    .is_dir()
                {
                    continue;
                }
                let visual_json = visual_entry.path().join("visual.json");
                if !visual_json.is_file() {
                    report.errors.push(format!(
                        "visual directory is missing visual.json: {}. Remove the empty visual directory or restore its visual.json before retrying",
                        visual_entry.path().display()
                    ));
                    continue;
                }
                report.visuals += 1;
                let visual = read_json_value(&visual_json)?;
                report
                    .errors
                    .extend(visual_query_state_errors(&visual_json, &visual));
                check_visual_minimum_size(&visual_json, &visual, report);
                if visual["visual"]["query"]["queryState"]
                    .as_object()
                    .is_some_and(|query_state| {
                        query_state.values().any(|role| {
                            role["projections"]
                                .as_array()
                                .is_some_and(|projections| !projections.is_empty())
                        })
                    })
                {
                    report.bound_visuals += 1;
                }
            }
        }
    }
    for entry in fs::read_dir(&pages_dir).map_err(|err| {
        CliError::unexpected(format!("read pages dir {}: {err}", pages_dir.display()))
    })? {
        let entry = entry.map_err(|err| {
            CliError::unexpected(format!(
                "read pages dir entry {}: {err}",
                pages_dir.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !seen_pages.contains(name) && path.join("page.json").is_file() {
            report.warnings.push(format!(
                "page directory is not referenced by pageOrder: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn check_visual_minimum_size(
    visual_json_path: &Path,
    visual: &Value,
    report: &mut ValidationReport,
) {
    if visual["visual"]["visualType"].as_str() != Some("slicer") {
        return;
    }
    let is_between = visual
        .pointer("/visual/objects/data/0/properties/mode/expr/Literal/Value")
        .and_then(Value::as_str)
        == Some("'Between'");
    let minimum = if is_between {
        BETWEEN_SLICER_MIN_HEIGHT
    } else {
        SLICER_MIN_HEIGHT
    };
    let qualifier = if is_between {
        "Between slicer"
    } else {
        "slicer"
    };
    let Some(height) = visual["position"]["height"].as_f64() else {
        report.errors.push(format!(
            "{} {qualifier} position.height must be a number of at least {minimum}",
            visual_json_path.display(),
        ));
        return;
    };
    if height < minimum {
        report.errors.push(format!(
            "{} {qualifier} height {height} is below the Power BI minimum of {minimum}",
            visual_json_path.display(),
        ));
    }
}

fn check_positive_page_number(
    page_json_path: &Path,
    page_json: &Value,
    field: &str,
    report: &mut ValidationReport,
) {
    if !page_json[field].as_f64().is_some_and(|value| value > 0.0) {
        report.errors.push(format!(
            "{} has invalid nonpositive or missing page {}",
            page_json_path.display(),
            field
        ));
    }
}

fn check_report_filter_configs(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
) -> CliResult<()> {
    let definition_dir = resolved.report_dir.join("definition");
    if !definition_dir.is_dir() {
        return Ok(());
    }
    for entry in WalkDir::new(&definition_dir) {
        let entry = walkdir_entry(
            &definition_dir,
            entry,
            "walk report definition filter configurations",
        )?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let file_name = path.file_name().and_then(|value| value.to_str());
        if !matches!(file_name, Some("report.json" | "page.json" | "visual.json")) {
            continue;
        }
        let value = read_json_value(path)?;
        check_filter_config(path, &value, report);
    }
    Ok(())
}

fn check_filter_config(path: &Path, value: &Value, report: &mut ValidationReport) {
    let Some(filter_config) = value.get("filterConfig") else {
        return;
    };
    let Some(filter_config) = filter_config.as_object() else {
        report
            .errors
            .push(format!("{} filterConfig is not an object", path.display()));
        return;
    };
    let Some(filters) = filter_config.get("filters") else {
        return;
    };
    let Some(filters) = filters.as_array() else {
        report.errors.push(format!(
            "{} filterConfig.filters is not an array",
            path.display()
        ));
        return;
    };
    for (index, filter) in filters.iter().enumerate() {
        check_filter_config_entry(path, index, filter, report);
    }
}

fn check_filter_config_entry(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    match filter.get("name") {
        Some(Value::String(name)) if name.trim().is_empty() || name.len() > 50 => {
            report.errors.push(format!(
                "{} filterConfig.filters[{index}] name must be between 1 and 50 characters for Power BI Desktop",
                path.display()
            ));
        }
        Some(Value::String(_)) => {}
        None => report.errors.push(format!(
            "{} filterConfig.filters[{index}] is missing required name",
            path.display()
        )),
        Some(_) => report.errors.push(format!(
            "{} filterConfig.filters[{index}] name is not a string",
            path.display()
        )),
    }
    if filter["howCreated"].as_str() == Some("powerbi-cli") {
        report.errors.push(format!(
            "{} filterConfig.filters[{index}] has invalid howCreated \"powerbi-cli\"; use a Power BI value such as \"User\"",
            path.display()
        ));
    }
    let Some(filter_type) = filter["type"].as_str() else {
        return;
    };
    if !matches!(
        filter_type,
        "Categorical" | "Advanced" | "TopN" | "RelativeDate"
    ) {
        return;
    }
    check_filter_field(path, index, filter, report);
    if filter_type == "Categorical"
        && filter["howCreated"].as_str() == Some("Drillthrough")
        && filter.get("filter").is_none()
    {
        return;
    }
    let Some(body) = filter.get("filter").and_then(Value::as_object) else {
        // Desktop materializes one field-well placeholder per visual binding when a
        // report is saved. These entries deliberately carry only name, field, and
        // type; they are metadata, not active filter predicates.
        return;
    };
    if filter_type == "Categorical" && body.contains_key("values") {
        report.errors.push(format!(
            "{} categorical filterConfig.filters[{index}] uses legacy filter.values; expected filter.Version, filter.From, and filter.Where",
            path.display()
        ));
    }
    if filter["filter"]["Version"].as_i64() != Some(2) {
        report.errors.push(format!(
            "{} {filter_type} filterConfig.filters[{index}] is missing filter.Version = 2",
            path.display(),
        ));
    }
    let from = filter["filter"]["From"].as_array();
    if from.is_none_or(|items| items.is_empty()) {
        report.errors.push(format!(
            "{} {filter_type} filterConfig.filters[{index}] is missing non-empty filter.From",
            path.display(),
        ));
    }
    let where_clauses = filter["filter"]["Where"].as_array();
    if where_clauses.is_none_or(|items| items.is_empty()) {
        report.errors.push(format!(
            "{} {filter_type} filterConfig.filters[{index}] is missing non-empty filter.Where",
            path.display(),
        ));
    }
    let aliases = filter_from_aliases(filter);
    if let Some(where_clauses) = where_clauses {
        for clause in where_clauses {
            check_filter_where_source_refs(path, index, clause, &aliases, report);
        }
    }
    match filter_type {
        "Categorical" => check_categorical_filter_shape(path, index, filter, report),
        "Advanced" => check_advanced_filter_shape(path, index, filter, report),
        "TopN" => check_topn_filter_shape(path, index, filter, report),
        "RelativeDate" => check_relative_date_filter_shape(path, index, filter, report),
        _ => {}
    }
}

fn check_filter_field(path: &Path, index: usize, filter: &Value, report: &mut ValidationReport) {
    let field = filter
        .pointer("/field/Column")
        .or_else(|| filter.pointer("/field/Measure"));
    let valid = field.is_some_and(|field| {
        field
            .pointer("/Expression/SourceRef/Entity")
            .and_then(Value::as_str)
            .is_some_and(|entity| !entity.is_empty())
            && field
                .get("Property")
                .and_then(Value::as_str)
                .is_some_and(|property| !property.is_empty())
            && field.pointer("/Expression/SourceRef/Source").is_none()
    });
    if !valid {
        report.errors.push(format!(
            "{} filterConfig.filters[{index}] field must be a Column or Measure with top-level SourceRef.Entity and a Property",
            path.display()
        ));
    }
}

fn check_categorical_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let Some(in_condition) = filter.pointer("/filter/Where/0/Condition/In") else {
        report.errors.push(format!(
            "{} categorical filterConfig.filters[{index}] must use Where[0].Condition.In",
            path.display()
        ));
        return;
    };
    if in_condition["Expressions"]
        .as_array()
        .is_none_or(|items| items.is_empty())
        || !in_condition["Values"].is_array()
    {
        report.errors.push(format!(
            "{} categorical filterConfig.filters[{index}] In condition requires non-empty Expressions and a Values array",
            path.display()
        ));
    }
}

fn check_advanced_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let Some(condition) = filter.pointer("/filter/Where/0/Condition") else {
        report.errors.push(format!(
            "{} Advanced filterConfig.filters[{index}] is missing Where[0].Condition",
            path.display()
        ));
        return;
    };
    if !valid_advanced_condition(condition) {
        report.errors.push(format!(
            "{} Advanced filterConfig.filters[{index}] has an invalid or empty Where[0].Condition expression",
            path.display()
        ));
    }
}

fn valid_advanced_condition(condition: &Value) -> bool {
    if let Some(comparison) = condition.get("Comparison") {
        return comparison["ComparisonKind"].as_i64().is_some()
            && comparison.get("Left").is_some_and(Value::is_object)
            && comparison.get("Right").is_some_and(Value::is_object);
    }
    for operator in ["And", "Or"] {
        if let Some(binary) = condition.get(operator) {
            return binary.get("Left").is_some_and(valid_advanced_condition)
                && binary.get("Right").is_some_and(valid_advanced_condition);
        }
    }
    condition
        .as_object()
        .is_some_and(|condition| !condition.is_empty())
}

fn check_topn_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let subquery = filter["filter"]["From"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["Type"].as_i64() == Some(2)
                    && item.pointer("/Expression/Subquery/Query").is_some()
            })
        })
        .and_then(|item| item.pointer("/Expression/Subquery/Query"));
    let Some(query) = subquery else {
        report.errors.push(format!(
            "{} TopN filterConfig.filters[{index}] requires a Type 2 subquery source in filter.From",
            path.display()
        ));
        return;
    };
    let query_from = query["From"].as_array();
    let query_select = query["Select"].as_array();
    let query_order_by = query["OrderBy"].as_array();
    let top = query["Top"].as_u64();
    let direction = query
        .pointer("/OrderBy/0/Direction")
        .and_then(Value::as_i64);
    let has_measure = query.pointer("/OrderBy/0/Expression/Measure").is_some();
    if query["Version"].as_i64() != Some(2)
        || query_from.is_none_or(|items| items.is_empty())
        || query_select.is_none_or(|items| items.is_empty())
        || query_order_by.is_none_or(|items| items.is_empty())
        || top.is_none_or(|top| top == 0)
        || !matches!(direction, Some(1 | 2))
        || !has_measure
    {
        report.errors.push(format!(
            "{} TopN filterConfig.filters[{index}] subquery requires Version 2, From, Select, measure OrderBy with Direction 1 or 2, and positive Top",
            path.display()
        ));
    }
    let query_aliases = query_from
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["Name"].as_str().map(ToOwned::to_owned))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    if let Some(select) = query_select {
        for expression in select {
            check_filter_where_source_refs(path, index, expression, &query_aliases, report);
        }
    }
    if let Some(order_by) = query_order_by {
        for expression in order_by {
            check_filter_where_source_refs(path, index, expression, &query_aliases, report);
        }
    }
    let topn_alias = filter["filter"]["From"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["Type"].as_i64() == Some(2)
                    && item.pointer("/Expression/Subquery/Query").is_some()
            })
        })
        .and_then(|item| item["Name"].as_str());
    let table_alias = filter
        .pointer("/filter/Where/0/Condition/In/Table/SourceRef/Source")
        .and_then(Value::as_str);
    if filter
        .pointer("/filter/Where/0/Condition/In/Expressions")
        .and_then(Value::as_array)
        .is_none_or(|items| items.is_empty())
        || table_alias.is_none()
        || table_alias != topn_alias
    {
        report.errors.push(format!(
            "{} TopN filterConfig.filters[{index}] Where must use In.Expressions and reference the Type 2 subquery alias through In.Table.SourceRef.Source",
            path.display()
        ));
    }
}

fn check_relative_date_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let Some(between) = filter.pointer("/filter/Where/0/Condition/Between") else {
        report.errors.push(format!(
            "{} RelativeDate filterConfig.filters[{index}] must use Where[0].Condition.Between",
            path.display()
        ));
        return;
    };
    if between.pointer("/Expression/Column").is_none()
        || !contains_expression_key(&between["LowerBound"], "DateSpan")
        || !contains_expression_key(&between["UpperBound"], "DateSpan")
        || !contains_expression_key(&between["LowerBound"], "Now")
        || !contains_expression_key(&between["UpperBound"], "Now")
    {
        report.errors.push(format!(
            "{} RelativeDate filterConfig.filters[{index}] Between requires a column Expression and DateSpan bounds derived from Now",
            path.display()
        ));
    }
}

fn contains_expression_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(key)
                || object
                    .values()
                    .any(|value| contains_expression_key(value, key))
        }
        Value::Array(items) => items
            .iter()
            .any(|value| contains_expression_key(value, key)),
        _ => false,
    }
}

fn filter_from_aliases(filter: &Value) -> BTreeSet<String> {
    filter["filter"]["From"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["Name"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn check_filter_where_source_refs(
    path: &Path,
    index: usize,
    value: &Value,
    aliases: &BTreeSet<String>,
    report: &mut ValidationReport,
) {
    match value {
        Value::Object(object) => {
            if let Some(source_ref) = object.get("SourceRef").and_then(Value::as_object) {
                if source_ref.get("Entity").and_then(Value::as_str).is_some()
                    && source_ref.get("Source").is_none()
                {
                    report.warnings.push(format!(
                        "{} filterConfig.filters[{index}] Where SourceRef uses Entity instead of Source alias",
                        path.display()
                    ));
                }
                if let Some(source) = source_ref.get("Source").and_then(Value::as_str)
                    && !aliases.is_empty()
                    && !aliases.contains(source)
                {
                    report.warnings.push(format!(
                        "{} filterConfig.filters[{index}] Where SourceRef.Source \"{source}\" is not present in filter.From",
                        path.display()
                    ));
                }
            }
            for child in object.values() {
                check_filter_where_source_refs(path, index, child, aliases, report);
            }
        }
        Value::Array(items) => {
            for child in items {
                check_filter_where_source_refs(path, index, child, aliases, report);
            }
        }
        _ => {}
    }
}

fn check_semantic_model(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
) -> CliResult<()> {
    let definition = resolved.semantic_model_dir.join("definition");
    let tables_dir = definition.join("tables");
    if !tables_dir.is_dir() {
        report.errors.push(format!(
            "missing TMDL tables directory: {}",
            tables_dir.display()
        ));
        return Ok(());
    }
    for entry in fs::read_dir(&tables_dir)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", tables_dir.display())))?
    {
        let entry = entry.map_err(|err| {
            CliError::unexpected(format!("read {} entry: {err}", tables_dir.display()))
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("tmdl") {
            report.tables += 1;
            let text = fs::read_to_string(&path)
                .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?;
            report.measures += text
                .lines()
                .filter(|line| line.trim_start().starts_with("measure "))
                .count();
            if !text.contains("partition ") {
                report
                    .warnings
                    .push(format!("table has no partition block: {}", path.display()));
            }
            if text.contains("Sql.Database(") || text.contains("Odbc.DataSource(") {
                report.warnings.push(format!(
                    "table partition appears to contain a real connector, review before taking home: {}",
                    path.display()
                ));
            }
        }
    }
    if report.tables == 0 {
        report.errors.push(format!(
            "semantic model contains no table .tmdl files: {}",
            tables_dir.display()
        ));
    }
    check_relationships(resolved, report)?;
    Ok(())
}

fn check_relationships(resolved: &ResolvedProject, report: &mut ValidationReport) -> CliResult<()> {
    let relationships_path = resolved
        .semantic_model_dir
        .join("definition")
        .join("relationships.tmdl");
    if !relationships_path.is_file() {
        return Ok(());
    }

    let (relationship_doc, tables) = relationship_tmdl::load_relationships_and_tables(resolved)?;
    report.relationships = relationship_doc.relationships.len();
    for relationship in &relationship_doc.relationships {
        if !tmdl_column_exists(&tables, &relationship.from_table, &relationship.from_column) {
            report.errors.push(format!(
                "relationship references missing from column {}.{}: {}",
                relationship.from_table,
                relationship.from_column,
                relationship.handle()
            ));
        }
        if !tmdl_column_exists(&tables, &relationship.to_table, &relationship.to_column) {
            report.errors.push(format!(
                "relationship references missing to column {}.{}: {}",
                relationship.to_table,
                relationship.to_column,
                relationship.handle()
            ));
        }
    }
    check_variation_references(&tables, &relationship_doc.relationships, report)?;
    Ok(())
}

fn check_variation_references(
    tables: &[tmdl::TableDocument],
    relationships: &[relationship_tmdl::RelationshipRecord],
    report: &mut ValidationReport,
) -> CliResult<()> {
    let relationship_names = relationships
        .iter()
        .map(|relationship| relationship.name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let table_names = tables
        .iter()
        .map(|table| table.table.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut hierarchy_names = BTreeMap::<String, BTreeSet<String>>::new();
    for table in tables {
        let text = fs::read_to_string(&table.path)
            .map_err(|err| CliError::unexpected(format!("read {}: {err}", table.path.display())))?;
        let names = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("hierarchy "))
            .map(unquote_tmdl_reference)
            .map(|name| name.to_ascii_lowercase())
            .collect();
        hierarchy_names.insert(table.table.to_ascii_lowercase(), names);
    }

    for table in tables {
        let text = fs::read_to_string(&table.path)
            .map_err(|err| CliError::unexpected(format!("read {}: {err}", table.path.display())))?;
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("relationship:") {
                let name = unquote_tmdl_reference(value.trim());
                if !name.is_empty() && !relationship_names.contains(&name.to_ascii_lowercase()) {
                    report.errors.push(format!(
                        "{}:{} variation references missing relationship: {}",
                        table.path.display(),
                        index + 1,
                        name
                    ));
                }
            }
            if let Some(value) = trimmed.strip_prefix("defaultHierarchy:")
                && let Some((table_name, hierarchy_name)) = hierarchy_reference(value.trim())
            {
                let table_key = table_name.to_ascii_lowercase();
                if !table_names.contains(&table_key) {
                    report.errors.push(format!(
                        "{}:{} variation defaultHierarchy references missing table: {}",
                        table.path.display(),
                        index + 1,
                        table_name
                    ));
                } else if !hierarchy_names
                    .get(&table_key)
                    .is_some_and(|names| names.contains(&hierarchy_name.to_ascii_lowercase()))
                {
                    report.errors.push(format!(
                        "{}:{} variation defaultHierarchy references missing hierarchy: {}.{}",
                        table.path.display(),
                        index + 1,
                        table_name,
                        hierarchy_name
                    ));
                }
            }
        }
    }
    Ok(())
}

fn hierarchy_reference(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    if value.starts_with('\'') {
        let bytes = value.as_bytes();
        let mut index = 1;
        while index < bytes.len() {
            if bytes[index] == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                if bytes.get(index + 1) != Some(&b'.') {
                    return None;
                }
                return Some((
                    unquote_tmdl_reference(&value[..=index]),
                    unquote_tmdl_reference(&value[index + 2..]),
                ));
            }
            index += 1;
        }
        None
    } else {
        value.split_once('.').map(|(table, hierarchy)| {
            (
                unquote_tmdl_reference(table),
                unquote_tmdl_reference(hierarchy),
            )
        })
    }
}

fn unquote_tmdl_reference(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value[1..value.len() - 1].replace("''", "'")
    } else {
        value.to_string()
    }
}

fn tmdl_column_exists(tables: &[tmdl::TableDocument], table: &str, column: &str) -> bool {
    tables.iter().any(|document| {
        tmdl::same_name(&document.table, table)
            && document
                .columns
                .iter()
                .any(|record| tmdl::same_name(&record.name, column))
    })
}

fn check_offline_hazards(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
    allow_desktop_runtime_files: bool,
) -> CliResult<()> {
    for artifact_dir in [&resolved.report_dir, &resolved.semantic_model_dir] {
        if !artifact_dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(artifact_dir) {
            let entry = walkdir_entry(
                artifact_dir,
                entry,
                "walk selected artifact offline-safety inputs",
            )?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let normalized = path
                .strip_prefix(artifact_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            let desktop_runtime_file = path
                .strip_prefix(artifact_dir)
                .ok()
                .and_then(|relative| relative.components().next())
                .is_some_and(|component| {
                    component
                        .as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(".pbi")
                });
            if (!allow_desktop_runtime_files || !desktop_runtime_file)
                && (normalized.ends_with(".pbi/cache.abf")
                    || normalized.ends_with("cache.abf")
                    || normalized.ends_with("localsettings.json")
                    || normalized.ends_with(".pbix")
                    || normalized.ends_with(".pbit"))
            {
                report.errors.push(format!(
                    "offline-unsafe data/cache/local file present: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn report_schema_major(report_json: &Value) -> Option<u64> {
    report_json
        .get("$schema")?
        .as_str()?
        .rsplit_once("/report/")?
        .1
        .split('/')
        .next()?
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn valid_theme_version_object(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 3
        && ["visual", "page", "report"].into_iter().all(|field| {
            object
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(is_three_part_numeric_version)
        })
}

fn is_three_part_numeric_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .into_iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}
