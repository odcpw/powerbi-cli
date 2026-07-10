use crate::pbir::{PageRecord, VisualRecord, load_report_snapshot};
use crate::{CliResult, ResolvedProject, canonical_display, read_json_value};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct ReportInteractionRecord {
    pub(crate) handle: String,
    pub(crate) ordinal: usize,
    pub(crate) interaction_type: String,
    pub(crate) unsupported: bool,
    pub(crate) source_name: String,
    pub(crate) target_name: String,
    pub(crate) source_visual: Option<InteractionVisualRef>,
    pub(crate) target_visual: Option<InteractionVisualRef>,
    pub(crate) page_handle: String,
    pub(crate) page_name: String,
    pub(crate) page_display_name: String,
    pub(crate) page_ordinal: usize,
    pub(crate) page_visual_count: usize,
    pub(crate) path: PathBuf,
    pub(crate) json_pointer: String,
    pub(crate) fingerprint: String,
    pub(crate) raw: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct InteractionVisualRef {
    pub(crate) handle: String,
    pub(crate) name: String,
    pub(crate) title: String,
    pub(crate) visual_type: String,
    pub(crate) path: Option<PathBuf>,
}

pub(crate) fn list_report_interactions(
    resolved: &ResolvedProject,
) -> CliResult<(Vec<ReportInteractionRecord>, crate::ValidationReport)> {
    let snapshot = load_report_snapshot(resolved)?;
    let mut interactions = Vec::new();
    for page in &snapshot.pages {
        if let Some(page_path) = page.path.as_ref() {
            let page_json = read_json_value(page_path)?;
            collect_interactions(&mut interactions, page, page_path, &page_json);
        }
    }
    Ok((interactions, snapshot.validation))
}

pub(crate) fn interaction_record_json(
    record: &ReportInteractionRecord,
    include_raw: bool,
) -> Value {
    let mut value = json!({
        "handle": record.handle,
        "ordinal": record.ordinal,
        "interactionType": record.interaction_type,
        "unsupported": record.unsupported,
        "page": {
            "handle": record.page_handle,
            "name": record.page_name,
            "displayName": record.page_display_name,
            "ordinal": record.page_ordinal,
            "visualCount": record.page_visual_count
        },
        "sourceName": record.source_name,
        "targetName": record.target_name,
        "source": visual_ref_json(record.source_visual.as_ref(), &record.source_name),
        "target": visual_ref_json(record.target_visual.as_ref(), &record.target_name),
        "path": canonical_display(&record.path),
        "jsonPointer": record.json_pointer,
        "fingerprint": record.fingerprint,
        "semantics": interaction_semantics(),
        "safety": safety_json(include_raw)
    });
    if include_raw {
        value["raw"] = record.raw.clone();
    }
    value
}

pub(crate) fn interaction_matches_page(record: &ReportInteractionRecord, page: &str) -> bool {
    matches_name_or_handle(
        &record.page_handle,
        &record.page_name,
        &record.page_display_name,
        page,
    )
}

pub(crate) fn interaction_matches_source(record: &ReportInteractionRecord, source: &str) -> bool {
    visual_selector_matches(record.source_visual.as_ref(), &record.source_name, source)
}

pub(crate) fn interaction_matches_target(record: &ReportInteractionRecord, target: &str) -> bool {
    visual_selector_matches(record.target_visual.as_ref(), &record.target_name, target)
}

pub(crate) fn interaction_matches_handle(record: &ReportInteractionRecord, handle: &str) -> bool {
    record.handle == handle
}

pub(crate) fn known_interaction_type(value: &str) -> bool {
    matches!(
        value,
        "Default" | "DataFilter" | "HighlightFilter" | "NoFilter"
    )
}

pub(crate) fn interaction_semantics() -> Value {
    json!({
        "mode": "explicit-overrides",
        "missingRowsMean": "When a source/target visual pair is absent from visualInteractions, Power BI uses the target visual's default interaction behavior.",
        "supportedTypes": ["Default", "DataFilter", "HighlightFilter", "NoFilter"]
    })
}

fn collect_interactions(
    out: &mut Vec<ReportInteractionRecord>,
    page: &PageRecord,
    page_path: &Path,
    page_json: &Value,
) {
    let Some(items) = page_json["visualInteractions"].as_array() else {
        return;
    };
    for (index, raw) in items.iter().enumerate() {
        out.push(interaction_record_from_raw(page, page_path, index, raw));
    }
}

pub(crate) fn interaction_record_from_raw(
    page: &PageRecord,
    page_path: &Path,
    ordinal: usize,
    raw: &Value,
) -> ReportInteractionRecord {
    let source_name = raw["source"].as_str().unwrap_or("").to_string();
    let target_name = raw["target"].as_str().unwrap_or("").to_string();
    let interaction_type = raw["type"].as_str().unwrap_or("unknown").to_string();
    let canonical = serde_json::to_string(raw).unwrap_or_default();
    let source_visual = page
        .visuals
        .iter()
        .find(|visual| visual.name == source_name)
        .map(visual_ref);
    let target_visual = page
        .visuals
        .iter()
        .find(|visual| visual.name == target_name)
        .map(visual_ref);
    let unsupported = source_name.is_empty()
        || target_name.is_empty()
        || !known_interaction_type(&interaction_type);
    ReportInteractionRecord {
        handle: format!("interaction:{}:{ordinal}", page.name),
        ordinal,
        interaction_type,
        unsupported,
        source_name,
        target_name,
        source_visual,
        target_visual,
        page_handle: page.handle.clone(),
        page_name: page.name.clone(),
        page_display_name: page.display_name.clone(),
        page_ordinal: page.ordinal,
        page_visual_count: page.visuals.len(),
        path: page_path.to_path_buf(),
        json_pointer: format!("/visualInteractions/{ordinal}"),
        fingerprint: format!("fnv64:{}", fingerprint_hex(&canonical)),
        raw: raw.clone(),
    }
}

fn visual_ref(visual: &VisualRecord) -> InteractionVisualRef {
    InteractionVisualRef {
        handle: visual.handle.clone(),
        name: visual.name.clone(),
        title: visual.title.clone(),
        visual_type: visual.visual_type.clone(),
        path: visual.path.clone(),
    }
}

fn visual_ref_json(visual: Option<&InteractionVisualRef>, raw_name: &str) -> Value {
    if let Some(visual) = visual {
        json!({
            "found": true,
            "handle": visual.handle,
            "name": visual.name,
            "title": visual.title,
            "visualType": visual.visual_type,
            "path": visual.path.as_ref().map(|path| canonical_display(path))
        })
    } else {
        json!({
            "found": false,
            "name": raw_name,
            "handle": Value::Null,
            "title": Value::Null,
            "visualType": Value::Null,
            "path": Value::Null
        })
    }
}

fn safety_json(raw_included: bool) -> Value {
    json!({
        "dataValueRisk": "none-detected",
        "mayContainDataValues": false,
        "rawIncluded": raw_included,
        "findings": []
    })
}

fn visual_selector_matches(
    visual: Option<&InteractionVisualRef>,
    raw_name: &str,
    selector: &str,
) -> bool {
    if let Some(visual) = visual {
        visual.handle == selector
            || visual.name == selector
            || visual.title == selector
            || visual.name.eq_ignore_ascii_case(selector)
            || visual.title.eq_ignore_ascii_case(selector)
    } else {
        raw_name == selector || raw_name.eq_ignore_ascii_case(selector)
    }
}

fn matches_name_or_handle(handle: &str, name: &str, display: &str, expected: &str) -> bool {
    handle == expected
        || name == expected
        || display == expected
        || name.eq_ignore_ascii_case(expected)
        || display.eq_ignore_ascii_case(expected)
}

fn fingerprint_hex(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
