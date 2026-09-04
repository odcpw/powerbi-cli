//! Canonical lint and audit rule registry.
//!
//! Finding producers use the public constants below rather than repeating rule
//! identifiers. This keeps CLI explanation, capabilities, and emitted findings
//! on one contract.

use crate::{CliError, CliResult};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleFamily {
    Validation,
    Report,
    Model,
    Dax,
    M,
    Audit,
    Handoff,
    /// Reserved for the future typed design-lint implementation.
    Design,
}

impl RuleFamily {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Report => "report",
            Self::Model => "model",
            Self::Dax => "dax",
            Self::M => "m",
            Self::Audit => "audit",
            Self::Handoff => "handoff",
            Self::Design => "design",
        }
    }
}

const RULE_FAMILIES: &[RuleFamily] = &[
    RuleFamily::Validation,
    RuleFamily::Report,
    RuleFamily::Model,
    RuleFamily::Dax,
    RuleFamily::M,
    RuleFamily::Audit,
    RuleFamily::Handoff,
    RuleFamily::Design,
];

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleDefinition {
    pub(crate) id: &'static str,
    pub(crate) family: RuleFamily,
    pub(crate) severity: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) remediation: &'static str,
    pub(crate) sanitize_action: Option<&'static str>,
    pub(crate) since: &'static str,
}

impl RuleDefinition {
    pub(crate) fn to_json(self) -> Value {
        json!({
            "id": self.id,
            "family": self.family.as_str(),
            "severity": self.severity,
            "summary": self.summary,
            "remediation": self.remediation,
            "sanitizeAction": self.sanitize_action,
            "since": self.since
        })
    }

    pub(crate) fn example_finding(self) -> Value {
        json!({
            "code": self.id,
            "ruleId": self.id,
            "severity": self.severity,
            "message": self.summary,
            "handle": "<affected-handle>",
            "path": "<affected-path>"
        })
    }
}

macro_rules! define_rules {
    ($(
        $name:ident => ($id:literal, $family:ident, $severity:literal, $summary:literal, $remediation:literal, $sanitize:expr)
    ),+ $(,)?) => {
        $(#[allow(dead_code)] pub(crate) const $name: &str = $id;)+

        pub(crate) const RULES: &[RuleDefinition] = &[
            $(RuleDefinition {
                id: $id,
                family: RuleFamily::$family,
                severity: $severity,
                summary: $summary,
                remediation: $remediation,
                sanitize_action: $sanitize,
                since: "0.1.0",
            },)+
        ];
    };
}

define_rules! {
    SPEC_MISSING_INPUT => ("spec.missing_input", Validation, "error", "A required dashboard-spec input is missing or cannot be inferred safely.", "Provide the field named by the RFC 6901 pointer, using `report spec fields` to inspect valid model candidates.", None),
    VALIDATION_STRUCTURE => ("validation.structure", Validation, "error", "The project fails native PBIP/PBIR/TMDL structural validation.", "Run `powerbi-cli validate <project> --json`, repair every reported structural error, and lint again.", None),
    VALIDATION_WARNING => ("validation.warning", Validation, "warning", "Native project validation reported a non-fatal compatibility warning.", "Review the corresponding validation warning and use Power BI Desktop when compatibility proof is required.", None),
    VALIDATION_MISSING_FILE => ("validation.missing_file", Validation, "error", "A required PBIP/PBIR/TMDL project file is missing.", "Restore the generated project file or regenerate the project before opening it in Desktop.", None),
    VALIDATION_FILE_READ => ("validation.file_read", Validation, "error", "A native validation input could not be read.", "Restore a readable project input and run native validation again.", None),
    VALIDATION_INVALID_JSON => ("validation.invalid_json", Validation, "error", "A PBIP/PBIR/TMDL JSON input is not valid JSON.", "Repair or regenerate the JSON file at the reported pointer.", None),
    VALIDATION_UTF8_BOM => ("validation.utf8_bom", Validation, "error", "A JSON-like project input contains a UTF-8 BOM rejected by the native contract.", "Rewrite the file as UTF-8 without a byte-order mark.", None),
    VALIDATION_THEME_SHAPE => ("validation.theme_shape", Validation, "error", "Report theme metadata does not match the PBIR schema.", "Repair themeCollection metadata using the report theme command or a Desktop round trip.", None),
    VALIDATION_THEME_RESOURCE => ("validation.theme_resource", Validation, "error", "A registered report theme resource is missing or inconsistent.", "Keep the customTheme metadata, resource package item, and JSON resource filename in sync.", None),
    VALIDATION_PAGE_ORDER => ("validation.page_order", Validation, "error", "Report page order metadata is missing, malformed, or inconsistent.", "Repair pages.json pageOrder and activePageName entries.", None),
    VALIDATION_PAGE_ORDER_EMPTY => ("validation.page_order_empty", Validation, "warning", "The report has no entries in pageOrder.", "Add an intentional report page or remove the empty report metadata.", None),
    VALIDATION_PAGE_SHAPE => ("validation.page_shape", Validation, "error", "A report page metadata file does not match its folder or geometry contract.", "Repair page.json name and positive width/height values.", None),
    VALIDATION_PAGE_UNREFERENCED => ("validation.page_unreferenced", Validation, "warning", "A page directory is not referenced by pages.json pageOrder.", "Add the page to pageOrder or remove the stale page directory.", None),
    VALIDATION_VISUAL_SHAPE => ("validation.visual_shape", Validation, "error", "A report visual metadata file is missing or has invalid geometry.", "Restore visual.json or repair the visual position metadata.", None),
    VALIDATION_QUERY_STATE => ("validation.query_state", Validation, "error", "A visual query state uses an unsupported or Desktop-incompatible role.", "Reapply the visual bindings with the report visuals command and validate again.", None),
    VALIDATION_FILTER_SHAPE => ("validation.filter_shape", Validation, "error", "Report filter metadata does not match the PBIR filter contract.", "Repair the filterConfig entry using the report filter commands.", None),
    VALIDATION_FILTER_SOURCE_REF => ("validation.filter_source_ref", Validation, "warning", "A filter source reference may not resolve to its declared alias.", "Review filter.From aliases and use Source references that exist in the same filter.", None),
    VALIDATION_MODEL_TABLE => ("validation.model_table", Validation, "error", "The semantic model has no usable table definitions.", "Restore the TMDL tables directory and at least one table file.", None),
    VALIDATION_MODEL_PARTITION => ("validation.model_partition", Validation, "warning", "A semantic-model table has no partition block.", "Add a credential-free dummy or model-derived partition before handoff.", None),
    VALIDATION_MODEL_CONNECTOR => ("validation.model_connector", Validation, "warning", "A semantic-model partition appears to contain a real connector.", "Review the connector before taking the project to a locked-down machine.", None),
    VALIDATION_RELATIONSHIP => ("validation.relationship", Validation, "error", "A semantic-model relationship references a missing endpoint.", "Repair relationship table/column names against the generated TMDL tables.", None),
    VALIDATION_VARIATION => ("validation.variation", Validation, "error", "A TMDL variation references missing model metadata.", "Repair the variation relationship, table, or hierarchy reference.", None),
    VALIDATION_OFFLINE_UNSAFE_FILE => ("validation.offline_unsafe_file", Validation, "error", "An offline-unsafe cache, local, or binary file is present in the project.", "Remove runtime data and cache artifacts before sharing the source project.", None),
    PBIR_REPORT_DEFINITION_VERSION => ("pbir.report_definition_version", Report, "error", "The PBIR report definition version is not the Desktop round-trip-proven version.", "Regenerate the report with the current CLI or migrate it to the version named in the finding before opening it in Desktop.", None),
    BPA_REPORT_DUPLICATE_PAGE_TITLE => ("bpa.report.duplicate_page_title", Report, "warning", "Multiple report pages have the same normalized display title.", "Give each page a distinct display name with `report pages update`.", None),
    REPORT_PAGE_EMPTY => ("report.page_empty", Report, "warning", "A report page contains no visuals.", "Add an intentional visual or delete the empty page with the guarded page command.", None),
    REPORT_VISUAL_MISSING_TITLE => ("report.visual_missing_title", Report, "warning", "A visual has no visible title text.", "Set a sentence-case title with `report visuals formatting set-text`, unless the visual intentionally uses a separate heading.", None),
    BPA_REPORT_DUPLICATE_VISUAL_TITLE => ("bpa.report.duplicate_visual_title", Report, "warning", "Multiple visuals on one page share the same normalized title.", "Rename the visuals so each title states its distinct purpose.", None),
    REPORT_VISUAL_UNBOUND => ("report.visual_unbound", Report, "info", "A visual has no field bindings.", "Bind the required fields with `report visuals set-bindings` or remove the unused visual.", None),
    PBIR_VISUAL_ALT_TEXT_LEGACY_LOCATION => ("pbir.visual_alt_text_legacy_location", Report, "warning", "Visual alt text is stored at a legacy PBIR location rejected by the Microsoft validator.", "Remove the rejected property with `report visuals formatting set-text --clear-alt-text`.", None),
    PBIR_VISUAL_ALT_TEXT_UNSUPPORTED_LOCATION => ("pbir.visual_alt_text_unsupported_location", Report, "warning", "Visual alt text is stored at a PBIR location rejected by the Microsoft validator.", "Remove the rejected property with `report visuals formatting set-text --clear-alt-text`; authoring remains fixture-gated.", None),
    REPORT_VISUAL_OUTSIDE_PAGE => ("report.visual_outside_page", Report, "warning", "A visual extends outside its page bounds.", "Move or resize it with `report visuals set-position` or re-run the layout command.", None),
    MODEL_TABLE_WITHOUT_COLUMNS => ("model.table_without_columns", Model, "error", "A semantic-model table has no columns.", "Add the intended columns or remove the invalid table definition.", None),
    MODEL_TABLE_WITHOUT_PARTITION => ("model.table_without_partition", Model, "warning", "A semantic-model table has no partition.", "Add a credential-free dummy or model-derived partition before handoff.", None),
    MODEL_RELATIONSHIP_COMMENT_UNSUPPORTED => ("model.relationship_comment_unsupported", Model, "error", "A TMDL comment above a relationship is rejected by older supported Desktop builds.", "Delete the comment above the relationship and keep explanatory prose outside TMDL.", None),
    PLATFORM_UNKNOWN_METADATA_PROPERTY => ("platform.unknown_metadata_property", Validation, "warning", "A .platform file contains metadata outside the proven schema.", "Keep only the documented `type` and `displayName` metadata properties.", None),
    DAX_REFERENCE_MISSING_COLUMN => ("dax.reference_missing_column", Dax, "error", "A DAX expression references a column absent from the semantic model.", "Correct the table/column reference using handles returned by `inspect --deep`.", None),
    DAX_REFERENCE_MISSING_MEASURE => ("dax.reference_missing_measure", Dax, "error", "A DAX expression references a measure that cannot be resolved.", "Create the measure or correct the reference using `model measures list`.", None),
    DAX_REFERENCE_SELF => ("dax.reference_self", Dax, "error", "A DAX measure directly references itself.", "Remove the self-reference or split the calculation into non-cyclic helper measures.", None),
    DAX_REFERENCE_AMBIGUOUS_MEASURE => ("dax.reference_ambiguous_measure", Dax, "warning", "An unqualified DAX measure reference resolves to multiple measures.", "Rename duplicate measures or qualify the reference so it resolves uniquely.", None),
    DAX_TABLE_VARIABLE_SCALAR_IF => ("dax.table_variable_scalar_if", Dax, "error", "A variable assigned by scalar IF is passed directly to a table-argument function.", "Branch around the table-consuming calculation instead of choosing table expressions with scalar IF.", None),
    DAX_DEPENDENCY_CYCLE => ("dax.dependency_cycle", Dax, "error", "The measure dependency graph contains a cycle.", "Break the cycle by extracting acyclic base measures or rewriting one dependency.", None),
    M_UNBUFFERED_REUSE => ("m.unbuffered_reuse", M, "warning", "A table-producing M step is reused by later steps without Table.Buffer.", "Review query folding and, when repeated evaluation is real, buffer the shared step deliberately.", None),
    M_UNTYPED_EXPANSION => ("m.untyped_expansion", M, "warning", "Table.ExpandTableColumn emits a numeric model column without a matching type conversion.", "Apply `Table.TransformColumnTypes` to expanded numeric columns before loading them.", None),
    FILTER_POSSIBLE_PERSISTED_VALUES => ("filter.possible_persisted_values", Audit, "warning", "Report filter metadata may contain persisted data values.", "Clear the filter through `report sanitize apply` after reviewing its plan.", Some("clear-filter-values")),
    SLICER_POSSIBLE_PERSISTED_VALUES => ("slicer.possible_persisted_values", Audit, "warning", "Slicer metadata may contain persisted selections or data values.", "Clear slicer state through `report sanitize apply` after reviewing its plan.", Some("clear-slicer-selections")),
    BOOKMARK_POSSIBLE_PERSISTED_VALUES => ("bookmark.possible_persisted_values", Audit, "warning", "Bookmark metadata may contain persisted data values.", "Review the bookmark evidence and remove unsafe captured state before sharing the project.", None),
    BOOKMARK_POSSIBLE_PERSISTED_STATE => ("bookmark.possible_persisted_state", Audit, "warning", "Bookmark state may persist captured report state or data values.", "Review the bookmark manually; mutation stays blocked until a Desktop-backed shape is proven.", None),
    INTERACTION_UNSUPPORTED_OR_STALE => ("interaction.unsupported_or_stale", Audit, "warning", "A visual interaction is unsupported or references a missing visual.", "Use the explicit report interaction commands to repair or remove the stale override.", None),
    PROJECT_VALIDATION_ERROR => ("project.validation_error", Handoff, "error", "Project validation failed during handoff audit.", "Run native validation, repair the reported error, and repeat the handoff audit.", None),
    PROJECT_VALIDATION_WARNING => ("project.validation_warning", Handoff, "warning", "Project validation produced a warning during handoff audit.", "Review the native validation warning before handing off the project.", None),
    HANDOFF_OFFLINE_UNSAFE_FILE => ("handoff.offline_unsafe_file", Handoff, "error", "Validation found an offline-unsafe file in the project.", "Remove the unsafe artifact and keep only PBIP/PBIR/TMDL source metadata.", None),
    HANDOFF_TABLE_WITHOUT_PARTITION => ("handoff.table_without_partition", Handoff, "error", "A table has no partition that can be rebound safely.", "Add a credential-free dummy partition or an explicitly supported work-source partition.", None),
    HANDOFF_SOURCE_TEMPLATE_STORE_INVALID => ("handoff.source_template_store_invalid", Handoff, "error", "The source-template sidecar cannot be parsed or validated.", "Repair or regenerate the source-template store with `source-template` commands.", None),
    PARTITION_SOURCE_MISSING => ("partition.source_missing", Handoff, "error", "A partition has no source expression.", "Add a complete credential-free partition source.", None),
    PARTITION_REAL_CONNECTOR_POSTGRES => ("partition.real_connector.postgres", Handoff, "error", "A partition uses PostgreSQL.Database and is unsafe for offline handoff.", "Replace it with a dummy partition for offline use, or audit explicitly for the work target.", None),
    PARTITION_REAL_CONNECTOR_SQL => ("partition.real_connector.sql", Handoff, "error", "A partition uses Sql.Database and is unsafe for offline handoff.", "Replace it with a dummy partition for offline use, or audit explicitly for the work target.", None),
    PARTITION_REAL_CONNECTOR_ODBC => ("partition.real_connector.odbc", Handoff, "error", "A partition uses Odbc.DataSource and is unsafe for offline handoff.", "Replace it with a dummy partition for offline use, or audit explicitly for the work target.", None),
    PARTITION_REAL_CONNECTOR_WEB => ("partition.real_connector.web", Handoff, "error", "A partition uses Web.Contents and is unsafe for offline handoff.", "Replace it with a dummy partition for offline use, or audit explicitly for the work target.", None),
    PARTITION_REAL_CONNECTOR_FILE => ("partition.real_connector.file", Handoff, "error", "A partition reads an external file and is unsafe for offline handoff.", "Replace it with a generated dummy table before offline handoff.", None),
    PARTITION_REAL_CONNECTOR_SHAREPOINT => ("partition.real_connector.sharepoint", Handoff, "error", "A partition uses SharePoint.Files and is unsafe for offline handoff.", "Replace it with a generated dummy table for offline use, or audit explicitly for the work target and authenticate only in Power BI Desktop.", None),
    PARTITION_DUMMY_TABLE_SHAPE_UNVERIFIED => ("partition.dummy_table_shape_unverified", Handoff, "warning", "A #table partition does not match the proven generated shape.", "Regenerate the dummy partition from the schema or correct its columns and row arity.", None),
    PARTITION_PII_SUSPECT_LITERAL => ("partition.pii_suspect_literal", Handoff, "warning", "Dummy partition literals may contain personal or long free-text values.", "Replace suspect values with synthetic placeholders before offline handoff.", None),
    PARTITION_SOURCE_UNKNOWN => ("partition.source_unknown", Handoff, "warning", "A partition source is not a recognized safe dummy or supported connector.", "Classify or replace the source explicitly; do not rely on an unknown M expression.", None),
    PARTITION_CREDENTIAL_LIKE_TEXT => ("partition.credential_like_text", Handoff, "error", "A partition source contains credential-like text.", "Remove the secret or credential parameter and configure authentication only on the work machine.", None),
    PARTITION_MODEL_DERIVED => ("partition.model_derived", Handoff, "warning", "A partition is explicitly annotated as model-derived rather than offline dummy data.", "Review it for the work target; offline handoff still requires a generated dummy source.", None),
    SOURCE_TEMPLATE_CREDENTIAL_LIKE_TEXT => ("sourceTemplate.credential_like_text", Handoff, "error", "A source template contains credential-like text.", "Remove credentials and retain only placeholders plus non-secret source identifiers.", None),
    SOURCE_TEMPLATE_ODBC_DSN_ATTRIBUTES => ("sourceTemplate.odbc_dsn_attributes", Handoff, "error", "An ODBC DSN contains inline connection attributes.", "Use a bare DSN name and configure attributes and credentials in the work-machine ODBC manager.", None),
    SOURCE_TEMPLATE_SPECIFIC_VALUES => ("sourceTemplate.specific_values", Handoff, "warning", "A source template stores specific source identifiers instead of placeholders.", "Replace machine- or environment-specific identifiers with registered placeholders.", None),
    HANDOFF_PARTITION_NOT_DUMMY => ("handoff.partition_not_dummy", Handoff, "error", "An offline handoff partition is not a generated dummy #table.", "Replace the source with a schema-shaped dummy partition before offline handoff.", None),
    HANDOFF_PARTITION_SOURCE_UNRECOGNIZED => ("handoff.partition_source_unrecognized", Handoff, "error", "A work-target partition is neither dummy, model-derived, nor a recognized connector.", "Use a supported connector or declare and review the intended model-derived source.", None),
    HANDOFF_POWERBI_CACHE_FOLDER => ("handoff.powerbi_cache_folder", Handoff, "error", "The project contains a Power BI runtime cache folder.", "Delete the .pbi runtime directory before packaging or handoff.", None),
    HANDOFF_ANALYSIS_SERVICES_CACHE => ("handoff.analysis_services_cache", Handoff, "error", "The project contains an Analysis Services cache file.", "Remove the .abf cache; source projects must never carry imported model data.", None),
    HANDOFF_BINARY_POWERBI_FILE => ("handoff.binary_powerbi_file", Handoff, "error", "The source project contains a PBIX or PBIT binary.", "Keep binary documents outside the offline-safe PBIP source project.", None),
    HANDOFF_LOCAL_SETTINGS_FILE => ("handoff.local_settings_file", Handoff, "error", "The project contains localSettings.json runtime state.", "Remove localSettings.json before handoff.", None),
    HANDOFF_EMBEDDED_DATA_FILE => ("handoff.embedded_data_file", Handoff, "error", "The project contains an embedded data file.", "Remove CSV, workbook, parquet, DuckDB, or SQLite data from the source project.", None),
    HANDOFF_CREDENTIAL_LIKE_TEXT => ("handoff.credential_like_text", Handoff, "error", "A handoff text file contains credential-like content.", "Remove or redact credentials and configure authentication only on the locked-down machine.", None),
    HANDOFF_PII_SUSPECT_TEXT => ("handoff.pii_suspect_text", Handoff, "warning", "A handoff text file contains PII-suspect row literals.", "Review and replace possible real rows with synthetic values.", None),
    HANDOFF_TEXT_SCAN_FAILED => ("handoff.text_scan_failed", Handoff, "error", "A handoff text file could not be read for safety scanning.", "Restore readable source text or remove the unreadable file before handoff.", None),
    DAX_FORMAT_MISSING => ("dax.format_missing", Dax, "warning", "A measure has no static or dynamic format string, so its display unit is implicit.", "Set a deliberate --format-string or --format-string-definition on the measure, then re-run DAX lint.", None),
    DAX_FORMAT_INVALID => ("dax.format_invalid", Dax, "warning", "A measure format string is not a balanced, supported custom format pattern.", "Replace the formatString with a balanced Power BI custom format such as #,##0.00, 0.0%, or Short Date.", None),
    MODEL_KEY_NOT_HIDDEN => ("model.key_not_hidden", Model, "warning", "A relationship endpoint marked as a model key remains visible to report authors.", "Hide relationship key columns with the model column visibility control while leaving the relationship endpoint intact.", None),
    MODEL_RELATIONSHIP_DIRECTION_SUSPECT => ("model.relationship_direction_suspect", Model, "warning", "A many-to-one fact-to-dimension relationship uses both-direction filtering.", "Prefer oneDirection from the fact table to the dimension; use bothDirections only with an explicit, reviewed ambiguity requirement.", None),
    MODEL_COLUMN_UNUSED => ("model.column_unused", Model, "warning", "A model column is not referenced by a visual, measure, or relationship.", "Remove the column or document its intended use; otherwise hide or omit it before handoff to keep the model focused.", None),
    M_DUPLICATE_STEP_NAME => ("m.duplicate_step_name", M, "error", "An M let expression defines the same step name more than once, which can surface as a cyclic-reference refresh error in Power BI Desktop.", "Rename or remove the duplicate M step; lint reports the first and duplicate source positions, including quoted identifiers, before Desktop handoff.", None),
}

pub(crate) fn all_rules() -> &'static [RuleDefinition] {
    RULES
}

pub(crate) fn rules_for_family(family: RuleFamily) -> impl Iterator<Item = RuleDefinition> {
    RULES
        .iter()
        .copied()
        .filter(move |rule| rule.family == family)
}

pub(crate) fn find_rule(id: &str) -> Option<RuleDefinition> {
    RULES.iter().copied().find(|rule| rule.id == id)
}

pub(crate) fn rule_ids() -> Vec<&'static str> {
    RULES.iter().map(|rule| rule.id).collect()
}

pub(crate) fn rule_family_names() -> Vec<&'static str> {
    RULE_FAMILIES
        .iter()
        .copied()
        .map(RuleFamily::as_str)
        .collect()
}

pub(crate) fn rule_definitions_json() -> Vec<Value> {
    RULES.iter().copied().map(RuleDefinition::to_json).collect()
}

pub(crate) fn ensure_finding_ids_registered(findings: &[Value], field: &str) -> CliResult<()> {
    for finding in findings {
        let Some(id) = finding.get(field).and_then(Value::as_str) else {
            continue;
        };
        if find_rule(id).is_none() {
            return Err(CliError::unexpected(format!(
                "{field} `{id}` is emitted but absent from the lint rule registry"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_registry() -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for rule in RULES {
        if !ids.insert(rule.id) {
            return Err(format!("duplicate lint rule id: {}", rule.id));
        }
        if rule.id.trim().is_empty()
            || rule.summary.trim().is_empty()
            || rule.remediation.trim().is_empty()
            || rule.since.trim().is_empty()
        {
            return Err(format!("undocumented lint rule: {}", rule.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        RULES, RuleFamily, ensure_finding_ids_registered, find_rule, rules_for_family,
        validate_registry,
    };
    use serde_json::json;
    use std::collections::BTreeSet;

    #[test]
    fn every_registered_rule_has_unique_complete_documentation() {
        validate_registry().expect("valid documented registry");
        let unique_ids = RULES.iter().map(|rule| rule.id).collect::<BTreeSet<_>>();
        assert_eq!(RULES.len(), unique_ids.len());
    }

    #[test]
    fn design_lint_has_a_typed_empty_extension_point() {
        assert_eq!(rules_for_family(RuleFamily::Design).count(), 0);
    }

    #[test]
    fn every_emitted_finding_id_must_be_registered() {
        let registered = vec![json!({"code": RULES[0].id})];
        ensure_finding_ids_registered(&registered, "code").expect("registered finding");

        let ad_hoc = vec![json!({"code": "future.ad_hoc_rule"})];
        let error = ensure_finding_ids_registered(&ad_hoc, "code")
            .expect_err("ad-hoc finding must be rejected");
        assert_eq!(error.code, "unexpected");
        assert!(error.message.contains("future.ad_hoc_rule"));
    }

    #[test]
    fn native_validation_codes_are_registered_for_explanation() {
        for id in [
            "validation.missing_file",
            "validation.file_read",
            "validation.invalid_json",
            "validation.utf8_bom",
            "validation.theme_shape",
            "validation.theme_resource",
            "validation.page_order",
            "validation.page_order_empty",
            "validation.page_shape",
            "validation.page_unreferenced",
            "validation.visual_shape",
            "validation.query_state",
            "validation.filter_shape",
            "validation.filter_source_ref",
            "validation.model_table",
            "validation.model_partition",
            "validation.model_connector",
            "validation.relationship",
            "validation.variation",
            "validation.offline_unsafe_file",
        ] {
            assert!(
                find_rule(id).is_some(),
                "missing native validation rule {id}"
            );
        }
    }
}
