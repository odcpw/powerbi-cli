use crate::pbir::{VisualRecord, load_report_snapshot};
use crate::{CliResult, ResolvedProject, canonical_display, read_json_value};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(crate) struct ReportSlicerRecord {
    pub(crate) handle: String,
    pub(crate) visual_handle: String,
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
    pub(crate) state: Value,
    pub(crate) fingerprint: Option<String>,
    pub(crate) may_contain_data_values: bool,
    pub(crate) literal_count: usize,
    pub(crate) raw: Option<Value>,
}

pub(crate) fn list_report_slicers(
    resolved: &ResolvedProject,
) -> CliResult<(Vec<ReportSlicerRecord>, crate::ValidationReport)> {
    let snapshot = load_report_snapshot(resolved)?;
    let mut slicers = Vec::new();
    for visual in snapshot.pages.iter().flat_map(|page| page.visuals.iter()) {
        if is_slicer_visual_type(&visual.visual_type) {
            slicers.push(slicer_record(visual)?);
        }
    }
    Ok((slicers, snapshot.validation))
}

pub(crate) fn slicer_record_json(record: &ReportSlicerRecord, include_raw: bool) -> Value {
    let mut value = json!({
        "handle": record.handle,
        "visualHandle": record.visual_handle,
        "name": record.name,
        "title": record.title,
        "visualType": record.visual_type,
        "page": {
            "handle": record.page_handle,
            "name": record.page_name,
            "displayName": record.page_display_name,
            "ordinal": record.page_ordinal
        },
        "path": record.path.as_ref().map(|path| canonical_display(path)),
        "position": record.position,
        "bindingCount": record.bindings.len(),
        "bindings": record.bindings,
        "target": first_binding_target(&record.bindings),
        "targets": binding_targets(&record.bindings),
        "state": record.state,
        "fingerprint": record.fingerprint,
        "safety": safety_json(record, include_raw)
    });
    if include_raw && let Some(raw) = &record.raw {
        value["raw"] = raw.clone();
    }
    value
}

pub(crate) fn slicer_matches_page(record: &ReportSlicerRecord, page: &str) -> bool {
    matches_name_or_handle(
        &record.page_handle,
        &record.page_name,
        &record.page_display_name,
        page,
    )
}

pub(crate) fn slicer_matches_handle_or_visual(record: &ReportSlicerRecord, selector: &str) -> bool {
    record.handle == selector
        || record.visual_handle == selector
        || record.name == selector
        || record.title == selector
        || record.name.eq_ignore_ascii_case(selector)
        || record.title.eq_ignore_ascii_case(selector)
}

fn slicer_record(visual: &VisualRecord) -> CliResult<ReportSlicerRecord> {
    let raw = visual
        .path
        .as_ref()
        .map(|path| read_json_value(path))
        .transpose()?;
    let canonical = raw
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok());
    let may_contain_data_values = raw.as_ref().is_some_and(slicer_may_contain_data_values);
    let literal_count = raw
        .as_ref()
        .map(count_sensitive_literals)
        .unwrap_or_default();
    Ok(ReportSlicerRecord {
        handle: format!("slicer:{}:{}", visual.page_name, visual.name),
        visual_handle: visual.handle.clone(),
        name: visual.name.clone(),
        title: visual.title.clone(),
        visual_type: visual.visual_type.clone(),
        path: visual.path.clone(),
        position: visual.position.clone(),
        bindings: visual.bindings.clone(),
        page_handle: visual.page_handle.clone(),
        page_name: visual.page_name.clone(),
        page_display_name: visual.page_display_name.clone(),
        page_ordinal: visual.page_ordinal,
        state: slicer_state_summary_from_bindings(&visual.bindings, raw.as_ref()),
        fingerprint: canonical.map(|text| format!("fnv64:{}", fingerprint_hex(&text))),
        may_contain_data_values,
        literal_count,
        raw,
    })
}

pub(crate) fn is_slicer_visual_type(visual_type: &str) -> bool {
    visual_type.to_ascii_lowercase().contains("slicer")
}

pub(crate) fn slicer_state_summary_from_bindings(bindings: &[Value], raw: Option<&Value>) -> Value {
    let mut query_roles = BTreeSet::new();
    let mut filter_config_filters = 0;
    let mut legacy_filters = 0;
    let mut has_visual_objects = false;
    let mut has_selection_state = false;
    let mut has_cached_display_state = false;
    if let Some(raw) = raw {
        if let Some(query_state) = raw["visual"]["query"]["queryState"].as_object() {
            query_roles.extend(query_state.keys().cloned());
        }
        filter_config_filters = raw["filterConfig"]["filters"]
            .as_array()
            .map(|items| items.len())
            .unwrap_or_default();
        legacy_filters = raw["filters"]
            .as_array()
            .map(|items| items.len())
            .unwrap_or_default();
        has_visual_objects = raw["visual"]["objects"].is_object() || raw["objects"].is_object();
        has_selection_state = contains_any_key(
            raw,
            &[
                "selection",
                "selected",
                "selectedValue",
                "selectedValues",
                "filterExpressionMetadata",
            ],
        );
        has_cached_display_state = contains_any_key(
            raw,
            &[
                "cachedDisplayNames",
                "cachedFilterDisplayItems",
                "cachedValueItems",
            ],
        );
    }
    json!({
        "fieldCount": bindings.len(),
        "queryRoles": query_roles.into_iter().collect::<Vec<_>>(),
        "filterConfigFilters": filter_config_filters,
        "legacyFilters": legacy_filters,
        "hasVisualObjects": has_visual_objects,
        "hasSelectionState": has_selection_state,
        "hasCachedDisplayState": has_cached_display_state
    })
}

fn slicer_may_contain_data_values(raw: &Value) -> bool {
    contains_any_key(
        raw,
        &[
            "filter",
            "filters",
            "selection",
            "selected",
            "selectedValue",
            "selectedValues",
            "filterExpressionMetadata",
            "cachedDisplayNames",
            "cachedFilterDisplayItems",
            "cachedValueItems",
            "valueMap",
            "identities",
        ],
    )
}

fn safety_json(record: &ReportSlicerRecord, raw_included: bool) -> Value {
    let findings = if record.may_contain_data_values {
        vec![json!({
            "code": "slicer.possible_persisted_values",
            "severity": "warning",
            "message": "Power BI slicer visual metadata can persist selected values from the semantic model; review raw slicer visual JSON before sharing outside the work environment."
        })]
    } else {
        Vec::new()
    };
    json!({
        "dataValueRisk": if record.may_contain_data_values { "possible" } else { "none-detected" },
        "mayContainDataValues": record.may_contain_data_values,
        "literalCountInSlicerState": record.literal_count,
        "rawIncluded": raw_included,
        "findings": findings
    })
}

fn first_binding_target(bindings: &[Value]) -> Value {
    bindings.first().map(binding_target).unwrap_or(Value::Null)
}

fn binding_targets(bindings: &[Value]) -> Value {
    Value::Array(bindings.iter().map(binding_target).collect())
}

fn binding_target(binding: &Value) -> Value {
    json!({
        "role": binding["role"],
        "kind": binding["kind"],
        "table": binding["table"],
        "field": binding["field"],
        "column": binding["column"],
        "measure": binding["measure"],
        "queryRef": binding["queryRef"],
        "nativeQueryRef": binding["nativeQueryRef"],
        "displayName": binding["displayName"]
    })
}

fn contains_any_key(value: &Value, keys: &[&str]) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            keys.iter()
                .any(|expected| key.eq_ignore_ascii_case(expected))
                || contains_any_key(value, keys)
        }),
        Value::Array(items) => items.iter().any(|item| contains_any_key(item, keys)),
        _ => false,
    }
}

fn count_sensitive_literals(value: &Value) -> usize {
    match value {
        Value::Object(object) => object
            .iter()
            .map(|(key, value)| {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "filter"
                        | "filters"
                        | "selection"
                        | "selected"
                        | "selectedvalue"
                        | "selectedvalues"
                        | "filterexpressionmetadata"
                        | "cacheddisplaynames"
                        | "cachedfilterdisplayitems"
                        | "cachedvalueitems"
                        | "valuemap"
                        | "identities"
                ) {
                    count_literals(value)
                } else {
                    count_sensitive_literals(value)
                }
            })
            .sum(),
        Value::Array(items) => items.iter().map(count_sensitive_literals).sum(),
        _ => 0,
    }
}

fn count_literals(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Bool(_) | Value::Number(_) | Value::String(_) => 1,
        Value::Array(items) => items.iter().map(count_literals).sum(),
        Value::Object(object) => object.values().map(count_literals).sum(),
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
