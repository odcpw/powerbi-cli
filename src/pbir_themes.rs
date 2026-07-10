use crate::project_io::write_json_atomic;
use crate::safety_scan::{CREDENTIAL_NEEDLES, contains_credential_like_text};
use crate::{CliError, CliResult, ResolvedProject, canonical_display};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(crate) const THEME_BUNDLE_SCHEMA: &str = "powerbi-cli.report.theme-bundle.v1";

#[derive(Debug, Clone)]
pub(crate) struct ReportThemeRecord {
    pub(crate) handle: String,
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) relative_path: String,
    pub(crate) registered: bool,
    pub(crate) theme: Value,
    pub(crate) safety: ThemeSafety,
}

#[derive(Debug, Clone)]
pub(crate) struct ThemeSafety {
    pub(crate) status: String,
    pub(crate) findings: Vec<ThemeFinding>,
}

#[derive(Debug, Clone)]
pub(crate) struct ThemeFinding {
    pub(crate) code: String,
    pub(crate) severity: String,
    pub(crate) message: String,
}

pub(crate) fn list_report_themes(resolved: &ResolvedProject) -> CliResult<Vec<ReportThemeRecord>> {
    let report_json_path = resolved.report_dir.join("definition").join("report.json");
    let report_json_text = fs::read_to_string(&report_json_path).unwrap_or_default();
    let mut themes = Vec::new();

    for entry in WalkDir::new(&resolved.report_dir) {
        let entry =
            crate::walkdir_entry(&resolved.report_dir, entry, "walk report theme definitions")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let relative = relative_report_path(&resolved.report_dir, path);
        if !relative
            .to_ascii_lowercase()
            .contains("staticresources/registeredresources/")
        {
            continue;
        }
        let theme = read_theme_json(path)?;
        if !looks_like_theme_json(&theme, path) {
            continue;
        }
        let name = theme["name"]
            .as_str()
            .unwrap_or("Power BI Theme")
            .to_string();
        let registered = report_json_text.contains(&relative)
            || report_json_text.contains(&relative.replace('/', "\\"))
            || path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|file_name| report_json_text.contains(file_name))
            || report_json_text.contains(&name);
        let handle = format!("theme:{}", slug(&relative));
        themes.push(ReportThemeRecord {
            handle,
            name,
            path: path.to_path_buf(),
            relative_path: relative,
            registered,
            safety: theme_safety(&theme),
            theme,
        });
    }

    themes.sort_by(|left, right| left.handle.cmp(&right.handle));
    Ok(themes)
}

pub(crate) fn read_theme_json(path: &Path) -> CliResult<Value> {
    let text = fs::read_to_string(path)
        .map_err(|err| CliError::file_not_found(format!("read theme {}: {err}", path.display())))?;
    let value = serde_json::from_str(&text).map_err(|err| {
        CliError::validation_failed(format!("parse theme JSON {}: {err}", path.display()))
    })?;
    validate_theme_json(&value, path)?;
    Ok(value)
}

pub(crate) fn validate_theme_json(value: &Value, path: &Path) -> CliResult<()> {
    if !value.is_object() {
        return Err(CliError::validation_failed(format!(
            "theme JSON must be an object: {}",
            path.display()
        )));
    }
    if value["name"]
        .as_str()
        .is_none_or(|name| name.trim().is_empty())
    {
        return Err(CliError::validation_failed(format!(
            "theme JSON requires a non-empty name: {}",
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
            "theme JSON contains credential-like text: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn theme_record_json(record: &ReportThemeRecord, include_theme: bool) -> Value {
    let mut value = json!({
        "handle": record.handle,
        "name": record.name,
        "path": canonical_display(&record.path),
        "relativePath": record.relative_path,
        "registered": record.registered,
        "safety": theme_safety_json(&record.safety)
    });
    if include_theme {
        value["themeJson"] = record.theme.clone();
    }
    value
}

pub(crate) fn write_theme_json(path: &Path, theme: &Value) -> CliResult<()> {
    validate_theme_json(theme, path)?;
    write_json_pretty(path, theme)
}

pub(crate) fn write_theme_bundle(path: &Path, bundle: &Value) -> CliResult<()> {
    write_json_pretty(path, bundle)
}

pub(crate) fn theme_safety_json(safety: &ThemeSafety) -> Value {
    json!({
        "status": safety.status,
        "safeForOfflineHandoff": safety.status != "unsafe",
        "credentialFree": !safety.findings.iter().any(|finding| finding.code == "theme.credential_like_text"),
        "findings": safety.findings.iter().map(|finding| json!({
            "code": finding.code,
            "severity": finding.severity,
            "message": finding.message
        })).collect::<Vec<_>>()
    })
}

pub(crate) fn theme_safety(value: &Value) -> ThemeSafety {
    let mut findings = Vec::new();
    if contains_credential_like_text(value, CREDENTIAL_NEEDLES, true) {
        findings.push(ThemeFinding {
            code: "theme.credential_like_text".to_string(),
            severity: "error".to_string(),
            message: "theme JSON contains credential-like text".to_string(),
        });
    }
    let status = if findings.iter().any(|finding| finding.severity == "error") {
        "unsafe"
    } else {
        "safe"
    };
    ThemeSafety {
        status: status.to_string(),
        findings,
    }
}

fn write_json_pretty(path: &Path, value: &Value) -> CliResult<()> {
    if path.exists() {
        return write_json_atomic(path, value);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    fs::write(path, text)
        .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))
}

fn looks_like_theme_json(value: &Value, path: &Path) -> bool {
    if value["name"].as_str().is_none() {
        return false;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.contains_key("dataColors")
        || object.contains_key("visualStyles")
        || object.contains_key("textClasses")
        || object.contains_key("background")
        || object.contains_key("foreground")
        || object.contains_key("tableAccent")
    {
        return true;
    }
    value["$schema"]
        .as_str()
        .is_some_and(|schema| schema.to_ascii_lowercase().contains("reporttheme"))
        || path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|file_name| file_name.to_ascii_lowercase().contains("theme"))
}

fn relative_report_path(report_dir: &Path, path: &Path) -> String {
    path.strip_prefix(report_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "theme".to_string()
    } else {
        trimmed
    }
}
