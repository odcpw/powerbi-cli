use crate::pbir::{PageRecord, load_report_snapshot};
use crate::safety_scan::data_value_safety;
use crate::{CliResult, ResolvedProject, canonical_display, read_json_value};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::fs;
use std::path::{Path, PathBuf};

const BOOKMARK_DATA_VALUE_KEYS: &[&str] = &[
    "filter",
    "filters",
    "highlight",
    "selection",
    "values",
    "value",
    "cachedValueItems",
    "valueMap",
    "identities",
    "dataSourceVariables",
];

#[derive(Debug, Clone)]
pub(crate) struct BookmarkGroup {
    pub(crate) name: String,
    pub(crate) display_name: String,
    pub(crate) ordinal: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ReportBookmarkRecord {
    pub(crate) handle: String,
    pub(crate) ordinal: usize,
    pub(crate) name: String,
    pub(crate) display_name: String,
    pub(crate) schema: Option<String>,
    pub(crate) schema_version: Option<String>,
    pub(crate) path: PathBuf,
    pub(crate) fingerprint: String,
    pub(crate) group: Option<BookmarkGroup>,
    pub(crate) options: Value,
    pub(crate) state: Value,
    pub(crate) unsupported: bool,
    pub(crate) unsupported_reasons: Vec<String>,
    pub(crate) may_contain_data_values: bool,
    pub(crate) literal_count: usize,
    pub(crate) raw: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct BookmarksMetadata {
    pub(crate) path: Option<PathBuf>,
    pub(crate) items_count: usize,
    pub(crate) groups_count: usize,
    pub(crate) diagnostics: Vec<Value>,
    order: Vec<String>,
    groups_by_child: BTreeMap<String, BookmarkGroup>,
}

impl BookmarksMetadata {
    fn empty() -> Self {
        Self {
            path: None,
            items_count: 0,
            groups_count: 0,
            diagnostics: Vec::new(),
            order: Vec::new(),
            groups_by_child: BTreeMap::new(),
        }
    }
}

pub(crate) fn list_report_bookmarks(
    resolved: &ResolvedProject,
) -> CliResult<(
    Vec<ReportBookmarkRecord>,
    BookmarksMetadata,
    crate::ValidationReport,
)> {
    let snapshot = load_report_snapshot(resolved)?;
    let bookmarks_dir = resolved.report_dir.join("definition").join("bookmarks");
    if !bookmarks_dir.is_dir() {
        return Ok((Vec::new(), BookmarksMetadata::empty(), snapshot.validation));
    }

    let metadata = read_bookmarks_metadata(&bookmarks_dir)?;
    let mut diagnostics = metadata.diagnostics.clone();
    let pages_by_name = snapshot
        .pages
        .iter()
        .map(|page| (page.name.clone(), page))
        .collect::<BTreeMap<_, _>>();
    let mut by_name = BTreeMap::new();
    for path in bookmark_paths(&bookmarks_dir)? {
        let raw = read_json_value(&path)?;
        let name = raw["name"]
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| bookmark_name_from_path(&path));
        let file_stem = bookmark_name_from_path(&path);
        if name != file_stem {
            diagnostics.push(diagnostic(
                "bookmark.name_file_mismatch",
                "warning",
                format!("Bookmark file name `{file_stem}` does not match bookmark name `{name}`."),
                Some(&path),
            ));
        }
        match by_name.entry(name) {
            Entry::Occupied(entry) => {
                diagnostics.push(diagnostic(
                    "bookmark.duplicate_name",
                    "warning",
                    format!(
                        "Duplicate bookmark name `{}`; the first file wins.",
                        entry.key()
                    ),
                    Some(&path),
                ));
            }
            Entry::Vacant(entry) => {
                entry.insert((path, raw));
            }
        }
    }

    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    for name in &metadata.order {
        if let Some((path, raw)) = by_name.remove(name) {
            seen.insert(name.clone());
            records.push(bookmark_record(
                records.len(),
                path,
                raw,
                metadata.groups_by_child.get(name).cloned(),
                &pages_by_name,
            ));
        } else {
            diagnostics.push(diagnostic(
                "bookmark.metadata_missing_file",
                "warning",
                format!(
                    "bookmarks.json references `{name}`, but no matching bookmark file exists."
                ),
                metadata.path.as_deref(),
            ));
        }
    }
    for (name, (path, raw)) in by_name {
        if metadata.path.is_some() {
            diagnostics.push(diagnostic(
                "bookmark.file_not_in_metadata",
                "info",
                format!("Bookmark file `{name}` is not referenced by bookmarks.json metadata."),
                Some(&path),
            ));
        }
        if seen.insert(name) {
            records.push(bookmark_record(
                records.len(),
                path,
                raw,
                None,
                &pages_by_name,
            ));
        }
    }

    let metadata = BookmarksMetadata {
        diagnostics,
        ..metadata
    };
    Ok((records, metadata, snapshot.validation))
}

pub(crate) fn bookmark_record_json(record: &ReportBookmarkRecord, include_raw: bool) -> Value {
    let mut value = json!({
        "handle": record.handle,
        "ordinal": record.ordinal,
        "name": record.name,
        "displayName": record.display_name,
        "schema": record.schema,
        "schemaVersion": record.schema_version,
        "path": canonical_display(&record.path),
        "jsonPointer": "",
        "fingerprint": record.fingerprint,
        "group": record.group.as_ref().map(group_json),
        "options": record.options,
        "state": record.state,
        "unsupported": record.unsupported,
        "unsupportedReasons": record.unsupported_reasons,
        "safety": safety_json(record, include_raw)
    });
    if include_raw {
        value["raw"] = record.raw.clone();
    }
    value
}

pub(crate) fn bookmarks_metadata_json(metadata: &BookmarksMetadata) -> Value {
    json!({
        "path": metadata.path.as_ref().map(|path| canonical_display(path)),
        "items": metadata.items_count,
        "groups": metadata.groups_count,
        "orderedNames": metadata.order,
        "diagnostics": metadata.diagnostics
    })
}

fn read_bookmarks_metadata(bookmarks_dir: &Path) -> CliResult<BookmarksMetadata> {
    let path = bookmarks_dir.join("bookmarks.json");
    if !path.is_file() {
        let mut metadata = BookmarksMetadata::empty();
        metadata.diagnostics.push(diagnostic(
            "bookmark.metadata_missing",
            "info",
            "No bookmarks.json metadata file is present; bookmark files will be sorted by file name.",
            None,
        ));
        return Ok(metadata);
    }
    let raw = read_json_value(&path)?;
    let items = raw["items"].as_array().cloned().unwrap_or_default();
    let mut metadata = BookmarksMetadata {
        path: Some(path),
        items_count: items.len(),
        groups_count: 0,
        diagnostics: Vec::new(),
        order: Vec::new(),
        groups_by_child: BTreeMap::new(),
    };

    for item in items {
        if let Some(children) = item["children"].as_array() {
            let group_name = item["name"].as_str().unwrap_or("group").to_string();
            let group_display = item["displayName"]
                .as_str()
                .unwrap_or(&group_name)
                .to_string();
            let group = BookmarkGroup {
                name: group_name,
                display_name: group_display,
                ordinal: metadata.groups_count,
            };
            metadata.groups_count += 1;
            for child in children.iter().filter_map(Value::as_str) {
                metadata.order.push(child.to_string());
                metadata
                    .groups_by_child
                    .insert(child.to_string(), group.clone());
            }
        } else if let Some(name) = item["name"].as_str() {
            metadata.order.push(name.to_string());
        }
    }
    Ok(metadata)
}

fn bookmark_paths(bookmarks_dir: &Path) -> CliResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(bookmarks_dir).map_err(|err| {
        crate::CliError::unexpected(format!(
            "failed to read bookmarks directory {}: {err}",
            bookmarks_dir.display()
        ))
    })? {
        let entry = entry.map_err(|err| {
            crate::CliError::unexpected(format!(
                "failed to read bookmarks directory entry {}: {err}",
                bookmarks_dir.display()
            ))
        })?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".bookmark.json"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn bookmark_record(
    ordinal: usize,
    path: PathBuf,
    raw: Value,
    group: Option<BookmarkGroup>,
    pages_by_name: &BTreeMap<String, &PageRecord>,
) -> ReportBookmarkRecord {
    let fallback_name = bookmark_name_from_path(&path);
    let name = raw["name"].as_str().unwrap_or(&fallback_name).to_string();
    let display_name = raw["displayName"].as_str().unwrap_or(&name).to_string();
    let schema = raw["$schema"].as_str().map(ToOwned::to_owned);
    let schema_version = schema.as_deref().and_then(schema_version);
    let options = options_summary(&raw);
    let state = state_summary(&raw, pages_by_name);
    let unsupported_reasons = unsupported_reasons(&raw);
    let safety = data_value_safety(&raw, BOOKMARK_DATA_VALUE_KEYS);
    let may_contain_data_values = safety.may_contain_data_values;
    let literal_count = safety.literal_count;
    let canonical = serde_json::to_string(&raw).unwrap_or_default();

    ReportBookmarkRecord {
        handle: format!("bookmark:{name}"),
        ordinal,
        name,
        display_name,
        schema,
        schema_version,
        path,
        fingerprint: format!("fnv64:{}", fingerprint_hex(&canonical)),
        group,
        options,
        state,
        unsupported: !unsupported_reasons.is_empty(),
        unsupported_reasons,
        may_contain_data_values,
        literal_count,
        raw,
    }
}

fn options_summary(raw: &Value) -> Value {
    let options = &raw["options"];
    let target_visual_names = options["targetVisualNames"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "applyOnlyToTargetVisuals": options["applyOnlyToTargetVisuals"].as_bool(),
        "targetVisualNames": target_visual_names,
        "targetVisualCount": target_visual_names.len(),
        "suppressActiveSection": options["suppressActiveSection"].as_bool(),
        "suppressData": options["suppressData"].as_bool(),
        "suppressDisplay": options["suppressDisplay"].as_bool()
    })
}

fn state_summary(raw: &Value, pages_by_name: &BTreeMap<String, &PageRecord>) -> Value {
    let exploration = &raw["explorationState"];
    let active_section = exploration["activeSection"].as_str().map(ToOwned::to_owned);
    let active_page = active_section
        .as_ref()
        .and_then(|name| pages_by_name.get(name))
        .map(|page| {
            json!({
                "handle": page.handle,
                "name": page.name,
                "displayName": page.display_name,
                "ordinal": page.ordinal
            })
        });
    let sections = exploration["sections"].as_object();
    let section_count = sections.map(|sections| sections.len()).unwrap_or_default();
    let report_filter_states = count_filter_containers(&exploration["filters"]);
    let mut page_filter_states = 0;
    let mut visual_filter_states = 0;
    let mut visual_container_states = 0;
    let mut visual_group_states = 0;
    let mut highlight_states = 0;
    let mut visual_display_states = 0;
    let mut display_mode_counts = BTreeMap::<String, usize>::new();
    let mut formatting_states = usize::from(exploration.get("objects").is_some());
    let mut sort_states = 0;
    let mut has_parameters = false;
    let mut has_projections = false;

    if let Some(sections) = sections {
        for section in sections.values() {
            page_filter_states += count_filter_containers(&section["filters"]);
            if let Some(visuals) = section["visualContainers"].as_object() {
                visual_container_states += visuals.len();
                for visual in visuals.values() {
                    visual_filter_states += count_filter_containers(&visual["filters"]);
                    highlight_states += usize::from(visual.get("highlight").is_some());
                    let single_visual = &visual["singleVisual"];
                    visual_display_states += usize::from(single_visual.get("display").is_some());
                    if let Some(mode) = single_visual["display"]["mode"].as_str() {
                        *display_mode_counts.entry(mode.to_string()).or_default() += 1;
                    }
                    formatting_states += usize::from(single_visual.get("objects").is_some());
                    sort_states += usize::from(single_visual.get("orderBy").is_some());
                    has_parameters |= single_visual.get("parameters").is_some();
                    has_projections |= single_visual.get("activeProjections").is_some()
                        || single_visual.get("projections").is_some();
                }
            }
            if let Some(groups) = section["visualContainerGroups"].as_object() {
                visual_group_states += groups.len();
            }
        }
    }

    json!({
        "version": exploration["version"].as_str(),
        "activeSection": active_section,
        "activePage": active_page,
        "sectionCount": section_count,
        "reportFilterStates": report_filter_states,
        "pageFilterStates": page_filter_states,
        "visualFilterStates": visual_filter_states,
        "visualContainerStates": visual_container_states,
        "visualContainerGroupStates": visual_group_states,
        "highlightStates": highlight_states,
        "visualDisplayStates": visual_display_states,
        "displayModeCounts": display_mode_counts,
        "formattingStates": formatting_states,
        "sortStates": sort_states,
        "hasParameters": has_parameters,
        "hasProjections": has_projections,
        "hasDataSourceVariables": exploration["dataSourceVariables"].as_str().is_some()
    })
}

fn count_filter_containers(filters: &Value) -> usize {
    let Some(object) = filters.as_object() else {
        return 0;
    };
    let mut count = 0;
    count += object
        .get("byName")
        .and_then(Value::as_object)
        .map(|items| items.len())
        .unwrap_or_default();
    for key in ["byExpr", "byType", "byTransientState"] {
        count += object
            .get(key)
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or_default();
    }
    count
}

fn unsupported_reasons(raw: &Value) -> Vec<String> {
    let mut reasons = Vec::new();
    for key in ["$schema", "displayName", "name", "explorationState"] {
        if raw.get(key).is_none() {
            reasons.push(format!("missing required bookmark field `{key}`"));
        }
    }
    if raw["$schema"]
        .as_str()
        .is_some_and(|schema| !supported_bookmark_schema(schema))
    {
        reasons.push("unsupported bookmark schema URL".to_string());
    }
    if !raw["explorationState"].is_object() {
        reasons.push("explorationState is not an object".to_string());
    }
    reasons
}

fn safety_json(record: &ReportBookmarkRecord, raw_included: bool) -> Value {
    let findings = if record.may_contain_data_values {
        vec![json!({
            "code": "bookmark.possible_persisted_values",
            "severity": "warning",
            "message": "Power BI bookmark metadata can persist report, page, visual, filter, slicer, highlight, or selected data state; review raw bookmark JSON before sharing outside the work environment."
        })]
    } else {
        Vec::new()
    };
    json!({
        "dataValueRisk": if record.may_contain_data_values { "possible" } else { "none-detected" },
        "mayContainDataValues": record.may_contain_data_values,
        "literalCountInBookmarkState": record.literal_count,
        "rawIncluded": raw_included,
        "findings": findings
    })
}

fn group_json(group: &BookmarkGroup) -> Value {
    json!({
        "name": group.name,
        "displayName": group.display_name,
        "ordinal": group.ordinal
    })
}

fn bookmark_name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".bookmark.json"))
        .unwrap_or("bookmark")
        .to_string()
}

fn schema_version(schema: &str) -> Option<String> {
    schema
        .split("/bookmark/")
        .nth(1)
        .and_then(|tail| tail.split('/').next())
        .map(ToOwned::to_owned)
}

fn supported_bookmark_schema(schema: &str) -> bool {
    schema.starts_with(
        "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/",
    ) && schema.ends_with("/schema.json")
}

fn diagnostic(
    code: &'static str,
    severity: &'static str,
    message: impl Into<String>,
    path: Option<&Path>,
) -> Value {
    json!({
        "code": code,
        "severity": severity,
        "message": message.into(),
        "path": path.map(canonical_display)
    })
}

fn fingerprint_hex(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
