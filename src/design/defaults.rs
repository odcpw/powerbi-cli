//! Data-driven visual formatting defaults.
//!
//! The defaults catalog is intentionally separate from the formatting catalog:
//! this file describes design policy (which family gets which value), while
//! `report_visual_objects` remains the single PBIR encoding boundary.  Every
//! active entry therefore resolves to one catalog key and is applied through
//! the same SetObject patch used by the command and operation kernel.

use crate::formatting_catalog::{FormattingCatalogEntry, formatting_catalog_entries};
use crate::ops::SetObject;
use crate::report_visual_objects::{apply_set_object_value_to_visual_json, encode_catalog_value};
use crate::{CliError, CliResult};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

pub(crate) const DESIGN_DEFAULTS_SCHEMA: &str = "powerbi-cli.design-defaults.v1";
pub(crate) const DESIGN_DEFAULTS_SOURCE: &str = "src/design/defaults.json";

const DESIGN_DEFAULTS_TEXT: &str = include_str!("defaults.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesignDefaultsDocument {
    pub(crate) schema: String,
    pub(crate) families: Vec<DesignFamily>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesignFamily {
    pub(crate) family: String,
    pub(crate) visual_types: Vec<String>,
    pub(crate) defaults: Vec<DesignDefault>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesignDefault {
    pub(crate) token: String,
    #[serde(default, alias = "key")]
    pub(crate) catalog_key: Option<String>,
    #[serde(default)]
    pub(crate) value: Option<Value>,
    #[serde(default)]
    pub(crate) inert: bool,
    #[serde(default)]
    pub(crate) owner: Option<String>,
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct EffectiveDefault {
    pub(crate) family: String,
    pub(crate) token: String,
    pub(crate) catalog_key: Option<String>,
    pub(crate) object: Option<String>,
    pub(crate) property: Option<String>,
    pub(crate) value: Option<Value>,
    pub(crate) encoded_value: Option<Value>,
    pub(crate) source: String,
    pub(crate) inert: bool,
    pub(crate) owner: Option<String>,
    pub(crate) reason: Option<String>,
}

static EMBEDDED_DESIGN_DEFAULTS: OnceLock<Result<DesignDefaultsDocument, String>> = OnceLock::new();

/// Parse one defaults document and validate every active entry against the
/// embedded formatting catalog.  Keeping this parser callable from tests
/// makes malformed catalog data fail before a project can be written.
pub(crate) fn parse_design_defaults(source: &str, text: &str) -> CliResult<DesignDefaultsDocument> {
    let document: DesignDefaultsDocument = serde_json::from_str(text).map_err(|error| {
        invalid_defaults(
            source,
            format!("does not match design-defaults.v1: {error}"),
        )
    })?;
    validate_document_against_catalog(source, &document, formatting_catalog_entries()?)?;
    Ok(document)
}

pub(crate) fn embedded_design_defaults() -> CliResult<&'static DesignDefaultsDocument> {
    match EMBEDDED_DESIGN_DEFAULTS.get_or_init(|| {
        parse_design_defaults(DESIGN_DEFAULTS_SOURCE, DESIGN_DEFAULTS_TEXT)
            .map_err(|error| error.message)
    }) {
        Ok(document) => Ok(document),
        Err(message) => Err(CliError::validation_failed(message.clone())),
    }
}

pub(crate) fn catalog_json() -> CliResult<Value> {
    let document = embedded_design_defaults()?;
    let families = document
        .families
        .iter()
        .map(|family| {
            json!({
                "family": family.family,
                "visualTypes": family.visual_types,
                "defaults": family.defaults.iter().map(default_json).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": DESIGN_DEFAULTS_SCHEMA,
        "source": DESIGN_DEFAULTS_SOURCE,
        "familyCount": families.len(),
        "families": families,
        "mergeOrder": ["catalog", "style.tokens", "style.defaults", "visuals[].format"],
        "notes": [
            "Active entries are encoded by the same SetObject path as report visuals set-object.",
            "Inert entries document design intent that awaits a Desktop-referenced formatting catalog key."
        ]
    }))
}

/// Resolve catalog and style values for one visual.  The returned list is
/// ordered by catalog key so repeated runs produce byte-identical output.
pub(crate) fn effective_defaults(
    visual_type: &str,
    style: Option<&Value>,
    visual_format: Option<&Map<String, Value>>,
) -> CliResult<Vec<EffectiveDefault>> {
    let catalog = embedded_design_defaults()?;
    let mut by_key = BTreeMap::<String, EffectiveDefault>::new();
    let normalized_visual = visual_type.trim().to_ascii_lowercase();

    for family in &catalog.families {
        if !family.visual_types.iter().any(|candidate| {
            candidate == "*" || candidate.trim().eq_ignore_ascii_case(&normalized_visual)
        }) {
            continue;
        }
        for default in &family.defaults {
            let key = default
                .catalog_key
                .as_deref()
                .unwrap_or(default.token.as_str())
                .to_string();
            by_key.insert(
                key,
                effective_from_default(family, default, "catalog", None)?,
            );
        }
    }

    if let Some(style) = style {
        let style_object = style.as_object().ok_or_else(|| {
            CliError::invalid_args("style must be an object when design defaults are enabled")
                .with_pointer("/style")
        })?;
        if let Some(tokens) = style_object.get("tokens") {
            let tokens = tokens.as_object().ok_or_else(|| {
                CliError::invalid_args("style.tokens must be an object")
                    .with_pointer("/style/tokens")
            })?;
            // `formatting` is the explicit v2 shape.  Direct catalog keys are
            // accepted as a compatibility convenience for machine-generated
            // specs that already flatten tokens.
            apply_override_map(
                &mut by_key,
                visual_type,
                tokens,
                "style.tokens",
                "/style/tokens",
            )?;
            if let Some(formatting) = tokens.get("formatting") {
                let formatting = formatting.as_object().ok_or_else(|| {
                    CliError::invalid_args("style.tokens.formatting must be an object")
                        .with_pointer("/style/tokens/formatting")
                })?;
                apply_override_map(
                    &mut by_key,
                    visual_type,
                    formatting,
                    "style.tokens",
                    "/style/tokens/formatting",
                )?;
            }
        }
        if let Some(defaults) = style_object.get("defaults") {
            let defaults = defaults.as_object().ok_or_else(|| {
                CliError::invalid_args("style.defaults must be an object")
                    .with_pointer("/style/defaults")
            })?;
            apply_override_map(
                &mut by_key,
                visual_type,
                defaults,
                "style.defaults",
                "/style/defaults",
            )?;
        }
    }

    if let Some(visual_format) = visual_format {
        apply_override_map(
            &mut by_key,
            visual_type,
            visual_format,
            "visuals[].format",
            "/pages/*/visuals/*/format",
        )?;
    }

    Ok(by_key.into_values().collect())
}

/// Apply active resolved defaults to a generated PBIR visual.  This helper is
/// intentionally a thin adapter over `SetObject`: no second object/property
/// encoder or PBIR path is allowed to emerge in the compiler.
pub(crate) fn apply_visual_defaults(
    visual_json: &mut Value,
    visual_type: &str,
    style: Option<&Value>,
    visual_format: Option<&Map<String, Value>>,
    enabled: bool,
) -> CliResult<Vec<EffectiveDefault>> {
    if !enabled {
        return Ok(Vec::new());
    }
    let effective = effective_defaults(visual_type, style, visual_format)?;
    for default in &effective {
        if default.inert {
            continue;
        }
        let operation = SetObject {
            visual: "<compiled-visual>".to_string(),
            object: default.object.clone().unwrap_or_default(),
            property: default.property.clone().unwrap_or_default(),
            value: default.encoded_value.clone().ok_or_else(|| {
                CliError::unexpected("active design default has no encoded value")
            })?,
        };
        apply_set_object_value_to_visual_json(visual_json, &operation)?;
    }
    Ok(effective)
}

pub(crate) fn effective_json(defaults: &[EffectiveDefault]) -> Vec<Value> {
    defaults
        .iter()
        .map(|default| {
            let mut value = json!({
                "family": default.family,
                "token": default.token,
                "catalogKey": default.catalog_key,
                "value": default.value,
                "source": default.source,
                "inert": default.inert
            });
            if let Some(object) = default.object.as_deref() {
                value["object"] = Value::String(object.to_string());
            }
            if let Some(property) = default.property.as_deref() {
                value["property"] = Value::String(property.to_string());
            }
            if let Some(encoded) = default.encoded_value.as_ref() {
                value["encodedValue"] = encoded.clone();
                if let (Some(object), Some(property)) =
                    (default.object.as_deref(), default.property.as_deref())
                {
                    value["operation"] = json!({
                        "op": "setObject",
                        "visual": "<visual-handle>",
                        "object": object,
                        "property": property,
                        "value": encoded
                    });
                }
            }
            if let Some(owner) = default.owner.as_deref() {
                value["owner"] = Value::String(owner.to_string());
            }
            if let Some(reason) = default.reason.as_deref() {
                value["reason"] = Value::String(reason.to_string());
            }
            value
        })
        .collect()
}

fn default_json(default: &DesignDefault) -> Value {
    let mut value = json!({
        "token": default.token,
        "catalogKey": default.catalog_key,
        "value": default.value,
        "inert": default.inert
    });
    if let Some(owner) = default.owner.as_deref() {
        value["owner"] = Value::String(owner.to_string());
    }
    if let Some(reason) = default.reason.as_deref() {
        value["reason"] = Value::String(reason.to_string());
    }
    value
}

fn effective_from_default(
    family: &DesignFamily,
    default: &DesignDefault,
    source: &str,
    value_override: Option<&Value>,
) -> CliResult<EffectiveDefault> {
    if default.inert {
        return Ok(EffectiveDefault {
            family: family.family.clone(),
            token: default.token.clone(),
            catalog_key: default.catalog_key.clone(),
            object: None,
            property: None,
            value: value_override.cloned().or_else(|| default.value.clone()),
            encoded_value: None,
            source: source.to_string(),
            inert: true,
            owner: default.owner.clone(),
            reason: default.reason.clone(),
        });
    }
    let key = default.catalog_key.as_deref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "design default {}.{} is active but has no catalogKey",
            family.family, default.token
        ))
    })?;
    let entry = catalog_entry(key, &family.family)?;
    let raw = value_override
        .cloned()
        .or_else(|| default.value.clone())
        .ok_or_else(|| {
            CliError::validation_failed(format!("design default {key} is active but has no value"))
        })?;
    let encoded = encode_catalog_value(entry, &raw)
        .map_err(|error| error.with_pointer(format!("/families/{}/defaults", family.family)))?;
    let (object, property) = split_key(key).ok_or_else(|| {
        CliError::validation_failed(format!("design default catalog key is malformed: {key}"))
    })?;
    Ok(EffectiveDefault {
        family: family.family.clone(),
        token: default.token.clone(),
        catalog_key: Some(key.to_string()),
        object: Some(object.to_string()),
        property: Some(property.to_string()),
        value: Some(raw),
        encoded_value: Some(encoded),
        source: source.to_string(),
        inert: false,
        owner: None,
        reason: default.reason.clone(),
    })
}

fn apply_override_map(
    by_key: &mut BTreeMap<String, EffectiveDefault>,
    visual_type: &str,
    values: &Map<String, Value>,
    source: &str,
    pointer: &str,
) -> CliResult<()> {
    for (key, value) in values {
        if key == "palette"
            || key == "semantic"
            || key == "typography"
            || key == "surfaces"
            || key == "spacing"
            || key == "numberFormats"
            || key == "formatting"
        {
            continue;
        }
        let entry = formatting_entry_for_visual(key, visual_type).ok_or_else(|| {
            CliError::invalid_args(format!(
                "{source} override `{key}` is not a formatting-catalog key for visual type {visual_type}"
            ))
            .with_pointer(format!("{pointer}/{key}"))
            .with_hint("Use a key from `powerbi-cli report visuals catalog --formatting --json`.")
        })?;
        let existing = by_key.get(key);
        if existing.is_some_and(|default| default.inert) {
            return Err(CliError::unsupported_feature(format!(
                "{source} override `{key}` targets an inert design default"
            ))
            .with_pointer(format!("{pointer}/{key}"))
            .with_hint("Wait for the owning Desktop-referenced T4 formatting catalog bead."));
        }
        let encoded = encode_catalog_value(entry, value)
            .map_err(|error| error.with_pointer(format!("{pointer}/{key}")))?;
        let (object, property) = split_key(key).ok_or_else(|| {
            CliError::invalid_args(format!(
                "formatting catalog key must be object.property: {key}"
            ))
        })?;
        let family = existing
            .map(|default| default.family.clone())
            .unwrap_or_else(|| "style".to_string());
        by_key.insert(
            key.clone(),
            EffectiveDefault {
                family,
                token: key.clone(),
                catalog_key: Some(key.clone()),
                object: Some(object.to_string()),
                property: Some(property.to_string()),
                value: Some(value.clone()),
                encoded_value: Some(encoded),
                source: source.to_string(),
                inert: false,
                owner: None,
                reason: None,
            },
        );
    }
    Ok(())
}

fn validate_document_against_catalog(
    source: &str,
    document: &DesignDefaultsDocument,
    formatting_catalog: &[FormattingCatalogEntry],
) -> CliResult<()> {
    if document.schema != DESIGN_DEFAULTS_SCHEMA {
        return Err(invalid_defaults(
            source,
            format!(
                "schema must be {DESIGN_DEFAULTS_SCHEMA}, got {}",
                document.schema
            ),
        ));
    }
    if document.families.is_empty() {
        return Err(invalid_defaults(source, "families must not be empty"));
    }
    let mut family_names = BTreeSet::new();
    for (family_index, family) in document.families.iter().enumerate() {
        if family.family.trim().is_empty() {
            return Err(invalid_defaults(
                source,
                format!("families[{family_index}].family must not be empty"),
            ));
        }
        if !family_names.insert(family.family.to_ascii_lowercase()) {
            return Err(invalid_defaults(
                source,
                format!(
                    "families[{family_index}] duplicates family {}",
                    family.family
                ),
            ));
        }
        if family.visual_types.is_empty() {
            return Err(invalid_defaults(
                source,
                format!("families[{family_index}].visualTypes must not be empty"),
            ));
        }
        if family.defaults.is_empty() {
            return Err(invalid_defaults(
                source,
                format!("families[{family_index}].defaults must not be empty"),
            ));
        }
        let mut keys = BTreeSet::new();
        for (default_index, default) in family.defaults.iter().enumerate() {
            if default.token.trim().is_empty() {
                return Err(invalid_defaults(
                    source,
                    format!(
                        "families[{family_index}].defaults[{default_index}].token must not be empty"
                    ),
                ));
            }
            let key = default
                .catalog_key
                .as_deref()
                .unwrap_or(default.token.as_str());
            if !keys.insert(key.to_string()) {
                return Err(invalid_defaults(
                    source,
                    format!("families[{family_index}] duplicates default key {key}"),
                ));
            }
            if default.inert {
                if default.catalog_key.is_some() {
                    return Err(invalid_defaults(
                        source,
                        format!(
                            "families[{family_index}].defaults[{default_index}] inert entries must not name catalogKey"
                        ),
                    ));
                }
                let owner = default.owner.as_deref().unwrap_or_default();
                if !owner.starts_with("pbi-t4-") {
                    return Err(invalid_defaults(
                        source,
                        format!(
                            "families[{family_index}].defaults[{default_index}] inert entries require an owning pbi-t4 bead"
                        ),
                    ));
                }
                continue;
            }
            let catalog_key = default.catalog_key.as_deref().ok_or_else(|| {
                invalid_defaults(
                    source,
                    format!("families[{family_index}].defaults[{default_index}] active entries require catalogKey"),
                )
            })?;
            let entry = formatting_catalog
                .iter()
                .find(|entry| format!("{}.{}", entry.object, entry.property) == catalog_key)
                .ok_or_else(|| {
                    invalid_defaults(
                        source,
                        format!("families[{family_index}].defaults[{default_index}] references unknown formatting catalog key {catalog_key}"),
                    )
                })?;
            if entry.reference.trim().is_empty() {
                return Err(invalid_defaults(
                    source,
                    format!(
                        "formatting catalog key {catalog_key} has no Desktop-referenced evidence"
                    ),
                ));
            }
            let value = default.value.as_ref().ok_or_else(|| {
                invalid_defaults(
                    source,
                    format!("families[{family_index}].defaults[{default_index}] active entries require value"),
                )
            })?;
            encode_catalog_value(entry, value).map_err(|error| {
                invalid_defaults(
                    source,
                    format!(
                        "families[{family_index}].defaults[{default_index}] value is invalid: {}",
                        error.message
                    ),
                )
            })?;
        }
    }
    Ok(())
}

fn formatting_entry_for_visual(
    key: &str,
    visual_type: &str,
) -> Option<&'static FormattingCatalogEntry> {
    formatting_catalog_entries().ok()?.iter().find(|entry| {
        format!("{}.{}", entry.object, entry.property) == key
            && entry
                .visual_types
                .iter()
                .any(|candidate| candidate == "*" || candidate.eq_ignore_ascii_case(visual_type))
    })
}

fn catalog_entry(key: &str, family: &str) -> CliResult<&'static FormattingCatalogEntry> {
    formatting_catalog_entries()?
        .iter()
        .find(|entry| format!("{}.{}", entry.object, entry.property) == key)
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "design family {family} references unknown formatting catalog key {key}"
            ))
        })
}

fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('.')
        .filter(|(object, property)| !object.is_empty() && !property.is_empty())
}

fn invalid_defaults(source: &str, message: impl AsRef<str>) -> CliError {
    CliError::validation_failed(format!(
        "invalid embedded design defaults {source}: {}",
        message.as_ref()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_are_complete_against_formatting_catalog() {
        let document = embedded_design_defaults().expect("embedded design defaults");
        let keys = formatting_catalog_entries()
            .expect("formatting catalog")
            .iter()
            .map(|entry| format!("{}.{}", entry.object, entry.property))
            .collect::<BTreeSet<_>>();
        for family in &document.families {
            for default in &family.defaults {
                if default.inert {
                    assert!(
                        default
                            .owner
                            .as_deref()
                            .is_some_and(|owner| owner.starts_with("pbi-t4-"))
                    );
                } else {
                    assert!(keys.contains(default.catalog_key.as_ref().expect("catalog key")));
                }
            }
        }
    }

    #[test]
    fn unknown_active_catalog_key_and_missing_evidence_are_rejected() {
        let unknown = r#"{
            "schema":"powerbi-cli.design-defaults.v1",
            "families":[{"family":"card","visualTypes":["card"],"defaults":[{"token":"bad","catalogKey":"missing.key","value":true}]}]
        }"#;
        let error = parse_design_defaults("unknown.json", unknown)
            .expect_err("unknown catalog key must be rejected");
        assert!(error.message.contains("unknown formatting catalog key"));

        let inert = r#"{
            "schema":"powerbi-cli.design-defaults.v1",
            "families":[{"family":"card","visualTypes":["card"],"defaults":[{"token":"pending","inert":true,"owner":"pbi-t3-owner"}]}]
        }"#;
        let error = parse_design_defaults("inert.json", inert)
            .expect_err("inert entries must be owned by T4");
        assert!(error.message.contains("owning pbi-t4 bead"));

        let known_key = r#"{
            "schema":"powerbi-cli.design-defaults.v1",
            "families":[{"family":"card","visualTypes":["card"],"defaults":[{"token":"title.show","catalogKey":"title.show","value":true}]}]
        }"#;
        let document: DesignDefaultsDocument =
            serde_json::from_str(known_key).expect("known-key defaults document");
        let mut catalog = formatting_catalog_entries()
            .expect("formatting catalog")
            .to_vec();
        catalog
            .iter_mut()
            .find(|entry| entry.object == "title" && entry.property == "show")
            .expect("title.show catalog entry")
            .reference
            .clear();
        let error = validate_document_against_catalog("missing-evidence.json", &document, &catalog)
            .expect_err("catalog entries without evidence must be rejected");
        assert!(error.message.contains("has no Desktop-referenced evidence"));
    }

    #[test]
    fn style_and_visual_overrides_follow_documented_precedence() {
        let style = json!({
            "tokens": {"formatting": {"labels.show": false}},
            "defaults": {"labels.show": true}
        });
        let visual = Map::from_iter([(String::from("labels.show"), Value::Bool(false))]);
        let values =
            effective_defaults("card", Some(&style), Some(&visual)).expect("resolve defaults");
        let labels = values
            .iter()
            .find(|value| value.catalog_key.as_deref() == Some("labels.show"))
            .expect("labels default");
        assert_eq!(labels.source, "visuals[].format");
        assert_eq!(labels.value, Some(Value::Bool(false)));
    }
}
