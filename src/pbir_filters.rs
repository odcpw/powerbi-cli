use crate::pbir::{PageRecord, VisualRecord, load_report_snapshot};
use crate::{
    CliError, CliResult, ResolvedProject, canonical_display, command_arg, read_json_value,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FilterScope {
    All,
    Report,
    Page,
    Visual,
}

impl FilterScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Report => "report",
            Self::Page => "page",
            Self::Visual => "visual",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FilterArrayOrigin {
    FilterConfig,
    Legacy,
}

impl FilterArrayOrigin {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FilterConfig => "filterConfig",
            Self::Legacy => "legacy",
        }
    }

    fn handle_suffix(self) -> &'static str {
        match self {
            Self::FilterConfig => "",
            Self::Legacy => "#legacy",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum FilterHandleIdentity {
    Name,
    Fingerprint,
}

impl FilterHandleIdentity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Fingerprint => "fingerprint",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReportFilterRecord {
    pub(crate) handle: String,
    pub(crate) handle_identity: FilterHandleIdentity,
    pub(crate) handle_ambiguous: bool,
    pub(crate) scope: FilterScope,
    pub(crate) ordinal: usize,
    pub(crate) array_origin: FilterArrayOrigin,
    pub(crate) name: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) filter_type: String,
    pub(crate) unsupported: bool,
    pub(crate) path: PathBuf,
    pub(crate) json_pointer: String,
    pub(crate) owner: FilterOwner,
    pub(crate) target: Value,
    pub(crate) condition_summary: String,
    pub(crate) fingerprint: String,
    pub(crate) may_contain_data_values: bool,
    pub(crate) literal_count: usize,
    pub(crate) raw: Value,
}

#[derive(Debug, Clone)]
pub(crate) enum FilterOwner {
    Report {
        path: PathBuf,
    },
    Page {
        handle: String,
        name: String,
        display_name: String,
        ordinal: usize,
        path: PathBuf,
    },
    Visual {
        handle: String,
        name: String,
        title: String,
        visual_type: String,
        path: PathBuf,
        page_handle: String,
        page_name: String,
        page_display_name: String,
        page_ordinal: usize,
    },
}

pub(crate) fn list_report_filters(
    resolved: &ResolvedProject,
) -> CliResult<(Vec<ReportFilterRecord>, crate::ValidationReport)> {
    let snapshot = load_report_snapshot(resolved)?;
    let mut filters = Vec::new();
    let report_json_path = resolved.report_dir.join("definition").join("report.json");
    if report_json_path.is_file() {
        let report_json = read_json_value(&report_json_path)?;
        collect_filters_from_value(
            &mut filters,
            FilterScope::Report,
            FilterOwner::Report {
                path: report_json_path.clone(),
            },
            &report_json_path,
            &report_json,
        );
    }

    for page in &snapshot.pages {
        if let Some(page_path) = page.path.as_ref() {
            let page_json = read_json_value(page_path)?;
            collect_filters_from_value(
                &mut filters,
                FilterScope::Page,
                page_owner(page, page_path),
                page_path,
                &page_json,
            );
        }
        for visual in &page.visuals {
            if let Some(visual_path) = visual.path.as_ref() {
                let visual_json = read_json_value(visual_path)?;
                collect_filters_from_value(
                    &mut filters,
                    FilterScope::Visual,
                    visual_owner(visual, visual_path),
                    visual_path,
                    &visual_json,
                );
            }
        }
    }

    Ok((filters, snapshot.validation))
}

pub(crate) fn filter_record_json(record: &ReportFilterRecord, include_raw: bool) -> Value {
    let mut value = json!({
        "handle": record.handle,
        "handleIdentity": record.handle_identity.as_str(),
        "handleAmbiguous": record.handle_ambiguous,
        "scope": record.scope.as_str(),
        "ordinal": record.ordinal,
        "arrayOrigin": record.array_origin.as_str(),
        "name": record.name,
        "displayName": record.display_name,
        "filterType": record.filter_type,
        "unsupported": record.unsupported,
        "target": record.target,
        "conditionSummary": record.condition_summary,
        "isActive": true,
        "path": canonical_display(&record.path),
        "jsonPointer": record.json_pointer,
        "fingerprint": record.fingerprint,
        "owner": owner_json(&record.owner),
        "safety": safety_json(record, include_raw)
    });
    if let Some(page) = owner_page_json(&record.owner) {
        value["page"] = page;
    }
    if let Some(visual) = owner_visual_json(&record.owner) {
        value["visual"] = visual;
    }
    if include_raw {
        value["raw"] = record.raw.clone();
    }
    value
}

pub(crate) fn owner_matches_page(owner: &FilterOwner, page: &str) -> bool {
    match owner {
        FilterOwner::Page {
            handle,
            name,
            display_name,
            ..
        } => matches_name_or_handle(handle, name, display_name, page),
        FilterOwner::Visual {
            page_handle,
            page_name,
            page_display_name,
            ..
        } => matches_name_or_handle(page_handle, page_name, page_display_name, page),
        FilterOwner::Report { .. } => false,
    }
}

pub(crate) fn owner_matches_visual(owner: &FilterOwner, visual: &str) -> bool {
    match owner {
        FilterOwner::Visual {
            handle,
            name,
            title,
            ..
        } => matches_name_or_handle(handle, name, title, visual),
        _ => false,
    }
}

fn collect_filters_from_value(
    out: &mut Vec<ReportFilterRecord>,
    scope: FilterScope,
    owner: FilterOwner,
    path: &Path,
    value: &Value,
) {
    let first_record = out.len();
    for (origin, base_pointer, items) in filter_arrays(value) {
        for (index, raw) in items.iter().enumerate() {
            out.push(filter_record(
                scope,
                owner.clone(),
                path,
                format!("{base_pointer}/{index}"),
                index,
                origin,
                raw,
            ));
        }
    }
    disambiguate_filter_handles(&mut out[first_record..]);
}

fn filter_arrays(value: &Value) -> Vec<(FilterArrayOrigin, &'static str, &[Value])> {
    let mut arrays = Vec::new();
    if let Some(items) = value["filterConfig"]["filters"].as_array() {
        arrays.push((
            FilterArrayOrigin::FilterConfig,
            "/filterConfig/filters",
            items.as_slice(),
        ));
    }
    if let Some(items) = value["filters"].as_array() {
        arrays.push((FilterArrayOrigin::Legacy, "/filters", items.as_slice()));
    }
    arrays
}

fn filter_record(
    scope: FilterScope,
    owner: FilterOwner,
    path: &Path,
    json_pointer: String,
    ordinal: usize,
    array_origin: FilterArrayOrigin,
    raw: &Value,
) -> ReportFilterRecord {
    let filter_type = raw["type"].as_str().unwrap_or("unknown").to_string();
    let target = filter_target(raw);
    let condition_summary = condition_summary(&filter_type, &target, raw);
    let fingerprint = filter_fingerprint(raw);
    let name = raw["name"].as_str().map(ToOwned::to_owned);
    let (handle_identity, identity) = filter_identity(name.as_deref(), &fingerprint);
    let may_contain_data_values = raw.get("filter").is_some() || contains_filter_value_key(raw);
    let literal_count = raw.get("filter").map(count_literals).unwrap_or_default();
    ReportFilterRecord {
        handle: filter_handle(scope, &owner, handle_identity, &identity, array_origin),
        handle_identity,
        handle_ambiguous: false,
        scope,
        ordinal,
        array_origin,
        name,
        display_name: raw["displayName"].as_str().map(ToOwned::to_owned),
        unsupported: !known_filter_type(&filter_type),
        filter_type,
        path: path.to_path_buf(),
        json_pointer,
        owner,
        target,
        condition_summary,
        fingerprint,
        may_contain_data_values,
        literal_count,
        raw: raw.clone(),
    }
}

fn page_owner(page: &PageRecord, path: &Path) -> FilterOwner {
    FilterOwner::Page {
        handle: page.handle.clone(),
        name: page.name.clone(),
        display_name: page.display_name.clone(),
        ordinal: page.ordinal,
        path: path.to_path_buf(),
    }
}

fn visual_owner(visual: &VisualRecord, path: &Path) -> FilterOwner {
    FilterOwner::Visual {
        handle: visual.handle.clone(),
        name: visual.name.clone(),
        title: visual.title.clone(),
        visual_type: visual.visual_type.clone(),
        path: path.to_path_buf(),
        page_handle: visual.page_handle.clone(),
        page_name: visual.page_name.clone(),
        page_display_name: visual.page_display_name.clone(),
        page_ordinal: visual.page_ordinal,
    }
}

fn filter_handle(
    scope: FilterScope,
    owner: &FilterOwner,
    handle_identity: FilterHandleIdentity,
    identity: &str,
    origin: FilterArrayOrigin,
) -> String {
    let identity = match handle_identity {
        FilterHandleIdentity::Name => encode_handle_component(identity),
        FilterHandleIdentity::Fingerprint => format!(
            "@{}",
            encode_handle_component(identity.strip_prefix('@').unwrap_or(identity))
        ),
    };
    let suffix = origin.handle_suffix();
    match (scope, owner) {
        (FilterScope::Report, _) => format!("filter:report:main:{identity}{suffix}"),
        (FilterScope::Page, FilterOwner::Page { name, .. }) => {
            format!(
                "filter:page:{}:{identity}{suffix}",
                encode_handle_component(name)
            )
        }
        (
            FilterScope::Visual,
            FilterOwner::Visual {
                page_name, name, ..
            },
        ) => {
            format!(
                "filter:visual:{}:{}:{identity}{suffix}",
                encode_handle_component(page_name),
                encode_handle_component(name)
            )
        }
        _ => format!("filter:{}:unknown:{identity}{suffix}", scope.as_str()),
    }
}

pub(crate) fn named_filter_handle(
    scope: FilterScope,
    page_name: Option<&str>,
    visual_name: Option<&str>,
    name: &str,
    origin: FilterArrayOrigin,
) -> String {
    let identity = encode_handle_component(name);
    let suffix = origin.handle_suffix();
    match scope {
        FilterScope::Report => format!("filter:report:main:{identity}{suffix}"),
        FilterScope::Page => format!(
            "filter:page:{}:{identity}{suffix}",
            encode_handle_component(page_name.unwrap_or("page"))
        ),
        FilterScope::Visual => format!(
            "filter:visual:{}:{}:{identity}{suffix}",
            encode_handle_component(page_name.unwrap_or("page")),
            encode_handle_component(visual_name.unwrap_or("visual"))
        ),
        FilterScope::All => unreachable!("filter handles cannot use all scope"),
    }
}

pub(crate) fn refreshed_filter_handle(record: &ReportFilterRecord, raw: &Value) -> String {
    let fingerprint = filter_fingerprint(raw);
    let (handle_identity, identity) = filter_identity(raw["name"].as_str(), &fingerprint);
    filter_handle(
        record.scope,
        &record.owner,
        handle_identity,
        &identity,
        record.array_origin,
    )
}

pub(crate) fn filter_fingerprint(raw: &Value) -> String {
    let canonical = serde_json::to_string(raw).unwrap_or_default();
    format!("fnv64:{}", fingerprint_hex(&canonical))
}

pub(crate) fn select_filter_by_handle<'a>(
    records: &'a [ReportFilterRecord],
    handle: &str,
    project_dir: &Path,
    for_mutation: bool,
) -> CliResult<&'a ReportFilterRecord> {
    let matches = records
        .iter()
        .filter(|record| record.handle == handle)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] if for_mutation && record.handle_ambiguous => Err(CliError::invalid_args(
            format!("filter handle is ambiguous and cannot be mutated safely: {handle}"),
        )
        .with_hint(
            "This owner contains duplicate filter names or fingerprints. Give the filters unique names, then run `report filters list` again.",
        )
        .with_suggested_command(format!(
            "powerbi-cli report filters list --project {} --include-raw --json",
            command_arg(project_dir)
        ))),
        [record] => Ok(record),
        [] if looks_like_ordinal_filter_handle(handle) => Err(CliError::invalid_args(format!(
            "legacy ordinal filter handle is no longer supported: {handle}"
        ))
        .with_hint(
            "Ordinal handles can retarget after deletion. Re-list filters and use the current name- or fingerprint-based handle.",
        )
        .with_suggested_command(format!(
            "powerbi-cli report filters list --project {} --json",
            command_arg(project_dir)
        ))),
        [] => Err(CliError::invalid_args(format!(
            "filter not found or filter handle is stale: {handle}"
        ))
        .with_hint(
            "The filter may have been renamed, removed, or changed. Run `report filters list` and use the exact current handle.",
        )
        .with_suggested_command(format!(
            "powerbi-cli report filters list --project {} --json",
            command_arg(project_dir)
        ))),
        _ => Err(CliError::invalid_args(format!(
            "filter handle matched multiple filters and cannot be resolved safely: {handle}"
        ))
        .with_hint("Run `report filters list --include-raw` and repair duplicate identities.")),
    }
}

fn filter_identity(name: Option<&str>, fingerprint: &str) -> (FilterHandleIdentity, String) {
    match name {
        Some(name) => (FilterHandleIdentity::Name, name.to_string()),
        None => (
            FilterHandleIdentity::Fingerprint,
            format!(
                "@{}",
                fingerprint
                    .strip_prefix("fnv64:")
                    .unwrap_or(fingerprint)
                    .chars()
                    .take(12)
                    .collect::<String>()
            ),
        ),
    }
}

fn disambiguate_filter_handles(records: &mut [ReportFilterRecord]) {
    let mut counts = HashMap::<String, usize>::new();
    for record in records.iter() {
        *counts.entry(record.handle.clone()).or_default() += 1;
    }
    let mut occurrences = HashMap::<String, usize>::new();
    for record in records {
        if counts.get(&record.handle).copied().unwrap_or_default() <= 1 {
            continue;
        }
        record.handle_ambiguous = true;
        let occurrence = occurrences.entry(record.handle.clone()).or_default();
        *occurrence += 1;
        record.handle = append_duplicate_ordinal(&record.handle, record.array_origin, *occurrence);
    }
}

fn append_duplicate_ordinal(handle: &str, origin: FilterArrayOrigin, ordinal: usize) -> String {
    let suffix = origin.handle_suffix();
    let stem = handle.strip_suffix(suffix).unwrap_or(handle);
    format!("{stem}~{ordinal}{suffix}")
}

fn encode_handle_component(value: &str) -> String {
    let numeric_only = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    let mut encoded = String::new();
    for (index, byte) in value.bytes().enumerate() {
        let safe = byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.');
        if safe && !(numeric_only && index == 0) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn looks_like_ordinal_filter_handle(handle: &str) -> bool {
    let parts = handle.split(':').collect::<Vec<_>>();
    let expected_len = match parts.get(1).copied() {
        Some("report") => 3,
        Some("page") => 4,
        Some("visual") => 5,
        _ => return false,
    };
    parts.len() == expected_len
        && parts
            .last()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn owner_json(owner: &FilterOwner) -> Value {
    match owner {
        FilterOwner::Report { path } => json!({
            "kind": "report",
            "handle": "report:main",
            "name": "report",
            "displayName": "Report",
            "path": canonical_display(path)
        }),
        FilterOwner::Page {
            handle,
            name,
            display_name,
            ordinal,
            path,
        } => json!({
            "kind": "page",
            "handle": handle,
            "name": name,
            "displayName": display_name,
            "ordinal": ordinal,
            "path": canonical_display(path)
        }),
        FilterOwner::Visual {
            handle,
            name,
            title,
            visual_type,
            path,
            page_handle,
            page_name,
            page_display_name,
            page_ordinal,
        } => json!({
            "kind": "visual",
            "handle": handle,
            "name": name,
            "title": title,
            "visualType": visual_type,
            "path": canonical_display(path),
            "page": {
                "handle": page_handle,
                "name": page_name,
                "displayName": page_display_name,
                "ordinal": page_ordinal
            }
        }),
    }
}

fn owner_page_json(owner: &FilterOwner) -> Option<Value> {
    match owner {
        FilterOwner::Page {
            handle,
            name,
            display_name,
            ordinal,
            path,
        } => Some(json!({
            "handle": handle,
            "name": name,
            "displayName": display_name,
            "ordinal": ordinal,
            "path": canonical_display(path)
        })),
        FilterOwner::Visual {
            page_handle,
            page_name,
            page_display_name,
            page_ordinal,
            ..
        } => Some(json!({
            "handle": page_handle,
            "name": page_name,
            "displayName": page_display_name,
            "ordinal": page_ordinal
        })),
        FilterOwner::Report { .. } => None,
    }
}

fn owner_visual_json(owner: &FilterOwner) -> Option<Value> {
    match owner {
        FilterOwner::Visual {
            handle,
            name,
            title,
            visual_type,
            path,
            ..
        } => Some(json!({
            "handle": handle,
            "name": name,
            "title": title,
            "visualType": visual_type,
            "path": canonical_display(path)
        })),
        _ => None,
    }
}

fn safety_json(record: &ReportFilterRecord, raw_included: bool) -> Value {
    let findings = if record.may_contain_data_values {
        vec![json!({
            "code": "filter.possible_persisted_values",
            "severity": "warning",
            "message": "Power BI filter metadata can persist selected values from the semantic model; review raw filter JSON before sharing outside the work environment."
        })]
    } else {
        Vec::new()
    };
    json!({
        "dataValueRisk": if record.may_contain_data_values { "possible" } else { "none-detected" },
        "mayContainDataValues": record.may_contain_data_values,
        "literalCountInFilterDefinition": record.literal_count,
        "rawIncluded": raw_included,
        "findings": findings
    })
}

pub(crate) fn filter_target(raw: &Value) -> Value {
    find_field(raw).unwrap_or_else(|| {
        json!({
            "kind": "unknown",
            "table": Value::Null,
            "column": Value::Null,
            "measure": Value::Null,
            "field": Value::Null
        })
    })
}

fn find_field(value: &Value) -> Option<Value> {
    match value {
        Value::Object(object) => {
            if let Some(column) = object.get("Column").and_then(Value::as_object) {
                let table = column["Expression"]["SourceRef"]["Entity"]
                    .as_str()
                    .map(ToOwned::to_owned);
                let field = column["Property"].as_str().map(ToOwned::to_owned);
                return Some(json!({
                    "kind": "column",
                    "table": table,
                    "column": field,
                    "measure": Value::Null,
                    "field": field
                }));
            }
            if let Some(measure) = object.get("Measure").and_then(Value::as_object) {
                let table = measure["Expression"]["SourceRef"]["Entity"]
                    .as_str()
                    .map(ToOwned::to_owned);
                let field = measure["Property"].as_str().map(ToOwned::to_owned);
                return Some(json!({
                    "kind": "measure",
                    "table": table,
                    "column": Value::Null,
                    "measure": field,
                    "field": field
                }));
            }
            object.values().find_map(find_field)
        }
        Value::Array(items) => items.iter().find_map(find_field),
        _ => None,
    }
}

fn condition_summary(filter_type: &str, target: &Value, raw: &Value) -> String {
    let target_text = match target["kind"].as_str() {
        Some("column") => match (target["table"].as_str(), target["column"].as_str()) {
            (Some(table), Some(column)) => format!(" on {table}[{column}]"),
            _ => " on unknown column".to_string(),
        },
        Some("measure") => match (target["table"].as_str(), target["measure"].as_str()) {
            (Some(table), Some(measure)) => format!(" on {table}[{measure}]"),
            _ => " on unknown measure".to_string(),
        },
        _ => String::new(),
    };
    let definition = if raw.get("filter").is_some() {
        " with persisted filter definition"
    } else {
        ""
    };
    format!("{filter_type} filter{target_text}{definition}")
}

fn known_filter_type(filter_type: &str) -> bool {
    matches!(
        filter_type,
        "Categorical"
            | "Range"
            | "Advanced"
            | "Passthrough"
            | "TopN"
            | "Include"
            | "Exclude"
            | "RelativeDate"
            | "Tuple"
            | "RelativeTime"
    )
}

fn contains_filter_value_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "value" | "values" | "literal" | "literals" | "condition" | "conditions"
            ) || contains_filter_value_key(value)
        }),
        Value::Array(items) => items.iter().any(contains_filter_value_key),
        _ => false,
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
