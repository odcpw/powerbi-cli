//! Versioned, embedded visual-formatting catalog.
//!
//! The catalog is deliberately data-driven so every set-object key has one
//! auditable provenance record. `build.rs` emits the `include_str!` embedding;
//! this module owns the strict schema and deterministic validation used by the
//! command and by the typed SetObject kernel.

use crate::{CliError, CliResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::OnceLock;

pub(crate) const FORMATTING_CATALOG_SCHEMA: &str = "powerbi-cli.formatting-catalog.v1";
pub(crate) const FORMATTING_CATALOG_SOURCE: &str = "testdata/formatting-catalog.v1.json";
pub(crate) const FORMATTING_CATALOG_OUTPUT_SCHEMA: &str =
    "powerbi-cli.report.visuals.formattingCatalog.v1";

include!(concat!(env!("OUT_DIR"), "/formatting_catalog.rs"));

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FormattingCatalogDocument {
    pub(crate) schema: String,
    pub(crate) entries: Vec<FormattingCatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FormattingCatalogEntry {
    pub(crate) object: String,
    pub(crate) property: String,
    pub(crate) encoding: FormattingEncoding,
    pub(crate) visual_types: Vec<String>,
    pub(crate) container: FormattingContainer,
    pub(crate) reference: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) enum FormattingEncoding {
    #[serde(rename = "bool", alias = "boolean")]
    Bool,
    #[serde(rename = "double")]
    Double,
    #[serde(rename = "string", alias = "text")]
    String,
}

impl FormattingEncoding {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Double => "double",
            Self::String => "string",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) enum FormattingContainer {
    #[serde(rename = "objects")]
    Objects,
    #[serde(rename = "visualContainerObjects")]
    VisualContainerObjects,
}

impl FormattingContainer {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Objects => "objects",
            Self::VisualContainerObjects => "visualContainerObjects",
        }
    }
}

static EMBEDDED_FORMATTING_CATALOG: OnceLock<Result<FormattingCatalogDocument, String>> =
    OnceLock::new();

/// Parse and validate one catalog document. This is public within the crate
/// so unit tests can exercise the same strict schema used by the embedded
/// source without writing a replacement file to disk.
pub(crate) fn parse_formatting_catalog(
    source: &str,
    text: &str,
) -> CliResult<FormattingCatalogDocument> {
    let document: FormattingCatalogDocument = serde_json::from_str(text).map_err(|error| {
        invalid_catalog(
            source,
            format!("does not match formatting-catalog.v1: {error}"),
        )
    })?;
    validate_document(source, &document)?;
    Ok(document)
}

pub(crate) fn embedded_formatting_catalog() -> CliResult<&'static FormattingCatalogDocument> {
    match EMBEDDED_FORMATTING_CATALOG.get_or_init(|| {
        parse_formatting_catalog(FORMATTING_CATALOG_SOURCE, EMBEDDED_FORMATTING_CATALOG_TEXT)
            .map_err(|error| error.message)
    }) {
        Ok(document) => Ok(document),
        Err(message) => Err(CliError::validation_failed(message.clone())),
    }
}

pub(crate) fn formatting_catalog_entries() -> CliResult<&'static [FormattingCatalogEntry]> {
    Ok(&embedded_formatting_catalog()?.entries)
}

pub(crate) fn formatting_catalog_json() -> CliResult<Value> {
    let catalog = embedded_formatting_catalog()?;
    let entries = catalog
        .entries
        .iter()
        .map(|entry| {
            json!({
                "object": entry.object,
                "property": entry.property,
                "encoding": entry.encoding.as_str(),
                "visualTypes": entry.visual_types,
                "container": entry.container.as_str(),
                "reference": entry.reference
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": FORMATTING_CATALOG_OUTPUT_SCHEMA,
        "catalogSchema": catalog.schema,
        "source": FORMATTING_CATALOG_SOURCE,
        "entryCount": entries.len(),
        "entries": entries,
        "notes": [
            "This is the complete curated set-object surface; no additional formatting properties are implied.",
            "visualTypes=[\"*\"] means the existing set-object behavior accepts the key on any visual; future entries must narrow applicability when a Desktop fixture proves it.",
            "Each entry reference must identify a Desktop-authored fixture or dated pilot observation before a new property is added."
        ],
        "next": [
            "powerbi-cli report visuals set-object --project <project-dir-or.pbip> --handle <visual-handle> --object categoryLabels --property fontSize --value 20 --dry-run --json",
            "powerbi-cli --json capabilities --for \"report visuals set-object\""
        ]
    }))
}

fn validate_document(source: &str, document: &FormattingCatalogDocument) -> CliResult<()> {
    if document.schema != FORMATTING_CATALOG_SCHEMA {
        return Err(invalid_catalog(
            source,
            format!(
                "schema must be {FORMATTING_CATALOG_SCHEMA}, got {}",
                document.schema
            ),
        ));
    }
    if document.entries.is_empty() {
        return Err(invalid_catalog(source, "entries must not be empty"));
    }

    let mut keys = BTreeSet::new();
    for (index, entry) in document.entries.iter().enumerate() {
        if entry.object.trim().is_empty() {
            return Err(invalid_catalog(
                source,
                format!("entries[{index}].object must not be empty"),
            ));
        }
        if entry.property.trim().is_empty() {
            return Err(invalid_catalog(
                source,
                format!("entries[{index}].property must not be empty"),
            ));
        }
        if !keys.insert((entry.object.as_str(), entry.property.as_str())) {
            return Err(invalid_catalog(
                source,
                format!(
                    "entries[{index}] duplicates {}.{}",
                    entry.object, entry.property
                ),
            ));
        }
        if entry.visual_types.is_empty() {
            return Err(invalid_catalog(
                source,
                format!("entries[{index}].visualTypes must not be empty"),
            ));
        }
        for (visual_index, visual_type) in entry.visual_types.iter().enumerate() {
            if visual_type.trim().is_empty() {
                return Err(invalid_catalog(
                    source,
                    format!("entries[{index}].visualTypes[{visual_index}] must not be empty"),
                ));
            }
        }
        if entry.reference.trim().is_empty() {
            return Err(invalid_catalog(
                source,
                format!("entries[{index}].reference must not be empty"),
            ));
        }
    }
    Ok(())
}

fn invalid_catalog(source: &str, message: impl AsRef<str>) -> CliError {
    CliError::validation_failed(format!(
        "invalid embedded formatting catalog {source}: {}",
        message.as_ref()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_strict_sorted_and_contains_exactly_the_migrated_entries() {
        let catalog = embedded_formatting_catalog().expect("embedded formatting catalog");
        assert_eq!(catalog.schema, FORMATTING_CATALOG_SCHEMA);
        assert_eq!(catalog.entries.len(), 11);
        let keys = catalog
            .entries
            .iter()
            .map(|entry| format!("{}.{}", entry.object, entry.property))
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "labels.show",
                "labels.fontSize",
                "categoryLabels.show",
                "categoryLabels.fontSize",
                "categoryLabels.wordWrap",
                "categoryAxis.show",
                "categoryAxis.showAxisTitle",
                "valueAxis.show",
                "valueAxis.showAxisTitle",
                "title.show",
                "title.text"
            ]
        );
        assert!(
            catalog
                .entries
                .iter()
                .all(|entry| entry.visual_types == ["*"])
        );
    }

    #[test]
    fn catalog_schema_rejects_unknown_fields_and_duplicate_keys() {
        let unknown = r#"{
            "schema":"powerbi-cli.formatting-catalog.v1",
            "entries":[],
            "unexpected":true
        }"#;
        let error = parse_formatting_catalog("unknown.json", unknown)
            .expect_err("unknown catalog fields must be rejected");
        assert!(error.message.contains("unknown field"));

        let duplicate = r#"{
            "schema":"powerbi-cli.formatting-catalog.v1",
            "entries":[
                {"object":"labels","property":"show","encoding":"bool","visualTypes":["*"],"container":"objects","reference":"fixture"},
                {"object":"labels","property":"show","encoding":"bool","visualTypes":["*"],"container":"objects","reference":"fixture"}
            ]
        }"#;
        let error = parse_formatting_catalog("duplicate.json", duplicate)
            .expect_err("duplicate catalog keys must be rejected");
        assert!(error.message.contains("duplicates labels.show"));
    }
}
