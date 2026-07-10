use crate::inspect::deep_inspect;
use crate::{
    CliError, CliResult, ResolvedProject, ValidationReport, canonical_display, validate_project,
};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct ReportSnapshot {
    pub(crate) validation: ValidationReport,
    pub(crate) pages: Vec<PageRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct PageRecord {
    pub(crate) handle: String,
    pub(crate) name: String,
    pub(crate) display_name: String,
    pub(crate) ordinal: usize,
    pub(crate) width: Value,
    pub(crate) height: Value,
    pub(crate) display_option: Value,
    pub(crate) page_type: Value,
    pub(crate) visibility: Value,
    pub(crate) page_binding: Value,
    pub(crate) is_active: bool,
    pub(crate) path: Option<PathBuf>,
    pub(crate) visuals: Vec<VisualRecord>,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualRecord {
    pub(crate) handle: String,
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) visual_type: String,
    pub(crate) path: Option<PathBuf>,
    pub(crate) position: Value,
    pub(crate) bindings: Vec<Value>,
    pub(crate) page_handle: String,
    pub(crate) page_name: String,
    pub(crate) page_display_name: String,
    pub(crate) page_ordinal: usize,
}

#[derive(Debug, Default)]
pub(crate) struct PageSelector {
    pub(crate) handle: Option<String>,
    pub(crate) name: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct VisualSelector {
    pub(crate) handle: Option<String>,
    pub(crate) page: Option<String>,
    pub(crate) visual: Option<String>,
}

pub(crate) fn load_report_snapshot(resolved: &ResolvedProject) -> CliResult<ReportSnapshot> {
    let validation = validate_project(resolved)?;
    let deep = deep_inspect(resolved, &validation)?;
    let pages = deep["report"]["pages"]
        .as_array()
        .map(|items| items.iter().map(page_record).collect::<CliResult<Vec<_>>>())
        .transpose()?
        .unwrap_or_default();
    Ok(ReportSnapshot { validation, pages })
}

pub(crate) fn page_summary(page: &PageRecord) -> Value {
    json!({
        "handle": page.handle,
        "name": page.name,
        "displayName": page.display_name,
        "ordinal": page.ordinal,
        "width": page.width,
        "height": page.height,
        "displayOption": page.display_option,
        "type": page.page_type,
        "visibility": page.visibility,
        "pageBinding": page.page_binding,
        "isActive": page.is_active,
        "path": page.path.as_ref().map(|path| canonical_display(path)),
        "visualCount": page.visuals.len(),
        "visualHandles": page.visuals.iter().map(|visual| visual.handle.clone()).collect::<Vec<_>>()
    })
}

pub(crate) fn page_detail(page: &PageRecord) -> Value {
    let mut value = page_summary(page);
    value["visuals"] = Value::Array(page.visuals.iter().map(visual_detail).collect());
    value
}

pub(crate) fn visual_list_item(visual: &VisualRecord) -> Value {
    json!({
        "handle": visual.handle,
        "name": visual.name,
        "title": visual.title,
        "visualType": visual.visual_type,
        "page": {
            "handle": visual.page_handle,
            "name": visual.page_name,
            "displayName": visual.page_display_name,
            "ordinal": visual.page_ordinal
        },
        "path": visual.path.as_ref().map(|path| canonical_display(path)),
        "position": visual.position,
        "bindingCount": visual.bindings.len()
    })
}

pub(crate) fn visual_detail(visual: &VisualRecord) -> Value {
    let mut value = visual_list_item(visual);
    value["bindings"] = Value::Array(visual.bindings.clone());
    value
}

pub(crate) fn find_page<'a>(
    pages: &'a [PageRecord],
    selector: &PageSelector,
    command: &str,
) -> CliResult<&'a PageRecord> {
    let matches = pages
        .iter()
        .filter(|page| page_matches(page, selector))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [page] => Ok(*page),
        [] => Err(CliError::invalid_args("page not found")
            .with_hint("Use `report pages list` to get stable page handles.")
            .with_suggested_command(page_selector_suggestion(command))),
        _ => Err(
            CliError::invalid_args("page selector matched multiple pages")
                .with_hint("Use the exact page handle instead of display name.")
                .with_suggested_command(page_selector_suggestion(command)),
        ),
    }
}

fn page_selector_suggestion(command: &str) -> String {
    match command {
        "report visuals add" => "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --dry-run --json".to_string(),
        "report interactions set" => "powerbi-cli report interactions set --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --type DataFilter --dry-run --json".to_string(),
        "report interactions disable" => "powerbi-cli report interactions disable --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json".to_string(),
        "report pages add" => "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --before <page-handle> --dry-run --json".to_string(),
        "report pages reorder" => "powerbi-cli report pages reorder --project <project-dir-or.pbip> --order <page-handle>,<page-handle> --dry-run --json".to_string(),
        _ => format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <page-handle> --json"
        ),
    }
}

pub(crate) fn find_visual<'a>(
    pages: &'a [PageRecord],
    selector: &VisualSelector,
    command: &str,
) -> CliResult<&'a VisualRecord> {
    if let Some(handle) = selector.handle.as_deref() {
        let matches = pages
            .iter()
            .flat_map(|page| page.visuals.iter())
            .filter(|visual| visual.handle == handle)
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [visual] => Ok(*visual),
            [] => Err(visual_not_found(command)),
            _ => Err(visual_ambiguous(command)),
        };
    }

    let page = selector.page.as_ref().ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --handle or --page plus --visual"
        ))
        .with_hint("Use `report visuals list` to get stable visual handles.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --json"
        ))
    })?;
    let visual_name = selector.visual.as_ref().ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires --visual when --page is used"))
            .with_hint("Use `report visuals list` to get stable visual names and handles.")
            .with_suggested_command(format!(
                "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-name> --visual <visual-name> --json"
            ))
    })?;
    let page_selector = PageSelector {
        handle: page.starts_with("page:").then(|| page.clone()),
        name: (!page.starts_with("page:")).then(|| page.clone()),
    };
    let page = find_page(pages, &page_selector, "report pages show")?;
    let matches = page
        .visuals
        .iter()
        .filter(|visual| {
            name_matches(&visual.name, visual_name) || name_matches(&visual.title, visual_name)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [visual] => Ok(*visual),
        [] => Err(visual_not_found(command)),
        _ => Err(visual_ambiguous(command)),
    }
}

pub(crate) fn all_visuals(pages: &[PageRecord]) -> Vec<&VisualRecord> {
    pages
        .iter()
        .flat_map(|page| page.visuals.iter())
        .collect::<Vec<_>>()
}

pub(crate) fn visuals_for_page<'a>(
    pages: &'a [PageRecord],
    page: Option<&str>,
) -> CliResult<Vec<&'a VisualRecord>> {
    if let Some(page) = page {
        let selector = PageSelector {
            handle: page.starts_with("page:").then(|| page.to_string()),
            name: (!page.starts_with("page:")).then(|| page.to_string()),
        };
        return Ok(find_page(pages, &selector, "report pages show")?
            .visuals
            .iter()
            .collect::<Vec<_>>());
    }
    Ok(all_visuals(pages))
}

fn page_record(value: &Value) -> CliResult<PageRecord> {
    let handle = required_string(value, "handle", "page")?;
    let name = required_string(value, "name", "page")?;
    let display_name = value["displayName"].as_str().unwrap_or(&name).to_string();
    let ordinal = value["ordinal"].as_u64().unwrap_or_default() as usize;
    let display_option = value["displayOption"].clone();
    let page_type = value["type"].clone();
    let visibility = value["visibility"].clone();
    let page_binding = value["pageBinding"].clone();
    let is_active = value["isActive"].as_bool().unwrap_or_default();
    let path = value["path"].as_str().map(PathBuf::from);
    let visuals = value["visuals"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|visual| visual_record(visual, &handle, &name, &display_name, ordinal))
                .collect::<CliResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(PageRecord {
        handle,
        name,
        display_name,
        ordinal,
        width: value["width"].clone(),
        height: value["height"].clone(),
        display_option,
        page_type,
        visibility,
        page_binding,
        is_active,
        path,
        visuals,
    })
}

fn visual_record(
    value: &Value,
    page_handle: &str,
    page_name: &str,
    page_display_name: &str,
    page_ordinal: usize,
) -> CliResult<VisualRecord> {
    Ok(VisualRecord {
        handle: required_string(value, "handle", "visual")?,
        name: required_string(value, "name", "visual")?,
        title: value["title"]
            .as_str()
            .unwrap_or_else(|| value["name"].as_str().unwrap_or("Visual"))
            .to_string(),
        visual_type: value["visualType"]
            .as_str()
            .unwrap_or("unknown")
            .to_string(),
        path: value["path"].as_str().map(PathBuf::from),
        position: value["position"].clone(),
        bindings: value["bindings"].as_array().cloned().unwrap_or_default(),
        page_handle: page_handle.to_string(),
        page_name: page_name.to_string(),
        page_display_name: page_display_name.to_string(),
        page_ordinal,
    })
}

fn required_string(value: &Value, field: &str, kind: &str) -> CliResult<String> {
    value[field].as_str().map(ToOwned::to_owned).ok_or_else(|| {
        CliError::validation_failed(format!("deep inspect {kind} is missing {field}"))
    })
}

fn page_matches(page: &PageRecord, selector: &PageSelector) -> bool {
    selector
        .handle
        .as_ref()
        .is_some_and(|handle| page.handle == *handle)
        || selector.name.as_ref().is_some_and(|name| {
            name_matches(&page.name, name) || name_matches(&page.display_name, name)
        })
}

fn name_matches(actual: &str, expected: &str) -> bool {
    actual == expected || actual.eq_ignore_ascii_case(expected)
}

fn visual_not_found(command: &str) -> CliError {
    CliError::invalid_args("visual not found")
        .with_hint("Use `report visuals list` to get stable visual handles.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --json"
        ))
}

fn visual_ambiguous(command: &str) -> CliError {
    CliError::invalid_args("visual selector matched multiple visuals")
        .with_hint("Use the exact visual handle instead of title or name.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --json"
        ))
}
