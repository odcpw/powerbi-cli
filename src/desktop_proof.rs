//! Versioned, embedded Power BI Desktop proof records.

use crate::{CliError, CliResult};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const DESKTOP_PROOF_SCHEMA: &str = "powerbi-cli.desktop-proof.v1";

include!(concat!(env!("OUT_DIR"), "/desktop_proof_records.rs"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) enum ProofLevel {
    #[serde(rename = "unit-smoke")]
    UnitSmoke,
    #[serde(rename = "schema-golden")]
    SchemaGolden,
    #[serde(rename = "desktop-golden-pending")]
    DesktopGoldenPending,
    #[serde(rename = "manual-desktop-canvas-refresh")]
    ManualDesktopCanvasRefresh,
    #[serde(rename = "desktop-canvas-refresh")]
    DesktopCanvasRefresh,
}

impl ProofLevel {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "unit-smoke" => Some(Self::UnitSmoke),
            "schema-golden" => Some(Self::SchemaGolden),
            "desktop-golden-pending" => Some(Self::DesktopGoldenPending),
            "manual-desktop-canvas-refresh" => Some(Self::ManualDesktopCanvasRefresh),
            "desktop-canvas-refresh" => Some(Self::DesktopCanvasRefresh),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::UnitSmoke => "unit-smoke",
            Self::SchemaGolden => "schema-golden",
            Self::DesktopGoldenPending => "desktop-golden-pending",
            Self::ManualDesktopCanvasRefresh => "manual-desktop-canvas-refresh",
            Self::DesktopCanvasRefresh => "desktop-canvas-refresh",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopProofRecord {
    pub(crate) schema: String,
    pub(crate) fixture: String,
    pub(crate) date: String,
    pub(crate) desktop_version: Option<String>,
    pub(crate) commands: Vec<String>,
    pub(crate) signals: DesktopProofSignals,
    pub(crate) proof_level: ProofLevel,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub(crate) struct DesktopProofSignals {
    pub(crate) feature_ids: Vec<String>,
    pub(crate) placeholder: bool,
    pub(crate) schema_golden: bool,
    pub(crate) desktop_reference_present: bool,
    pub(crate) current_artifact: bool,
    pub(crate) desktop_opened: bool,
    pub(crate) canvas_rendered: bool,
    pub(crate) refresh_completed: bool,
    pub(crate) issue_dialogs_absent: bool,
    pub(crate) expected_values_matched: bool,
    pub(crate) save_reopen_matched: bool,
    pub(crate) manual_review: bool,
    pub(crate) automated: bool,
    pub(crate) notes: Vec<String>,
    pub(crate) evidence: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedDesktopProofRecord {
    pub(crate) source: &'static str,
    pub(crate) record: DesktopProofRecord,
}

pub(crate) fn embedded_desktop_proof_records() -> CliResult<Vec<LoadedDesktopProofRecord>> {
    EMBEDDED_DESKTOP_PROOF_RECORDS
        .iter()
        .map(|(source, text)| {
            parse_desktop_proof_record(source, text)
                .map(|record| LoadedDesktopProofRecord { source, record })
        })
        .collect()
}

pub(crate) fn parse_desktop_proof_record(
    source: &'static str,
    text: &str,
) -> CliResult<DesktopProofRecord> {
    let record: DesktopProofRecord = serde_json::from_str(text).map_err(|error| {
        invalid_record(source, format!("does not match desktop-proof.v1: {error}"))
    })?;
    validate_record(source, &record)?;
    Ok(record)
}

fn validate_record(source: &str, record: &DesktopProofRecord) -> CliResult<()> {
    if record.schema != DESKTOP_PROOF_SCHEMA {
        return Err(invalid_record(
            source,
            format!(
                "schema must be {DESKTOP_PROOF_SCHEMA}, got {}",
                record.schema
            ),
        ));
    }
    validate_fixture(source, &record.fixture)?;
    validate_date(source, &record.date)?;
    if record
        .desktop_version
        .as_deref()
        .is_some_and(|version| version.trim().is_empty())
    {
        return Err(invalid_record(
            source,
            "desktopVersion must be null or a non-empty string",
        ));
    }
    if record.commands.is_empty() {
        return Err(invalid_record(source, "commands must not be empty"));
    }
    for (index, command) in record.commands.iter().enumerate() {
        let command = command.trim();
        if command.is_empty() || !command.contains("powerbi-cli ") {
            return Err(invalid_record(
                source,
                format!("commands[{index}] must be a non-empty executable powerbi-cli command"),
            ));
        }
    }
    validate_signal_coherence(source, record)?;
    if record.signals.feature_ids.is_empty() {
        return Err(invalid_record(
            source,
            "signals.featureIds must not be empty",
        ));
    }
    let mut unique_feature_ids = BTreeSet::new();
    for (index, feature_id) in record.signals.feature_ids.iter().enumerate() {
        if feature_id.trim().is_empty() {
            return Err(invalid_record(
                source,
                format!("signals.featureIds[{index}] must not be empty"),
            ));
        }
        if !unique_feature_ids.insert(feature_id) {
            return Err(invalid_record(
                source,
                format!("signals.featureIds contains duplicate `{feature_id}`"),
            ));
        }
    }

    let supported = supported_level(record);
    if record.proof_level > supported {
        return Err(invalid_record(
            source,
            format!(
                "proofLevel {} exceeds the evidence-supported level {}; window/title/screenshot signals alone cannot prove a rendered and refreshed canvas",
                record.proof_level.as_str(),
                supported.as_str()
            ),
        ));
    }
    Ok(())
}

fn supported_level(record: &DesktopProofRecord) -> ProofLevel {
    let signals = &record.signals;
    let mut supported = ProofLevel::UnitSmoke;
    if signals.schema_golden {
        supported = ProofLevel::SchemaGolden;
    }
    if signals.placeholder || (signals.schema_golden && signals.desktop_reference_present) {
        supported = ProofLevel::DesktopGoldenPending;
    }

    let desktop_canvas_refresh = signals.current_artifact
        && record.desktop_version.is_some()
        && signals.desktop_opened
        && signals.canvas_rendered
        && signals.refresh_completed
        && signals.issue_dialogs_absent
        && signals.expected_values_matched
        && (signals.manual_review || signals.automated);
    if desktop_canvas_refresh {
        supported = ProofLevel::ManualDesktopCanvasRefresh;
        if signals.automated {
            supported = ProofLevel::DesktopCanvasRefresh;
        }
    }
    supported
}

fn validate_signal_coherence(source: &str, record: &DesktopProofRecord) -> CliResult<()> {
    let signals = &record.signals;
    if signals.placeholder
        && (record.desktop_version.is_some()
            || signals.current_artifact
            || signals.desktop_opened
            || signals.canvas_rendered
            || signals.refresh_completed
            || signals.issue_dialogs_absent
            || signals.expected_values_matched
            || signals.save_reopen_matched
            || signals.manual_review
            || signals.automated)
    {
        return Err(invalid_record(
            source,
            "signals.placeholder cannot be combined with Desktop execution evidence",
        ));
    }
    if signals.canvas_rendered && !signals.desktop_opened {
        return Err(invalid_record(
            source,
            "signals.canvasRendered requires signals.desktopOpened=true",
        ));
    }
    if signals.refresh_completed && !signals.desktop_opened {
        return Err(invalid_record(
            source,
            "signals.refreshCompleted requires signals.desktopOpened=true",
        ));
    }
    if signals.expected_values_matched && !(signals.canvas_rendered && signals.refresh_completed) {
        return Err(invalid_record(
            source,
            "signals.expectedValuesMatched requires rendered-canvas and completed-refresh signals",
        ));
    }
    if signals.save_reopen_matched && !(signals.current_artifact && signals.desktop_opened) {
        return Err(invalid_record(
            source,
            "signals.saveReopenMatched requires the current artifact to have been opened",
        ));
    }
    if (signals.manual_review || signals.automated) && !signals.desktop_opened {
        return Err(invalid_record(
            source,
            "manualReview/automated signals require signals.desktopOpened=true",
        ));
    }
    Ok(())
}

fn validate_fixture(source: &str, fixture: &str) -> CliResult<()> {
    let fixture = fixture.trim();
    if fixture.is_empty()
        || fixture.starts_with('/')
        || fixture.starts_with('\\')
        || fixture.contains('\\')
        || fixture
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(invalid_record(
            source,
            "fixture must be a non-empty relative identifier without traversal",
        ));
    }
    Ok(())
}

fn validate_date(source: &str, date: &str) -> CliResult<()> {
    let parts = date.split('-').collect::<Vec<_>>();
    let parse = |index: usize, width: usize| {
        parts
            .get(index)
            .filter(|part| {
                part.len() == width && part.chars().all(|character| character.is_ascii_digit())
            })
            .and_then(|part| part.parse::<u32>().ok())
    };
    let Some(year) = parse(0, 4) else {
        return Err(invalid_record(source, "date must use YYYY-MM-DD"));
    };
    let Some(month) = parse(1, 2) else {
        return Err(invalid_record(source, "date must use YYYY-MM-DD"));
    };
    let Some(day) = parse(2, 2) else {
        return Err(invalid_record(source, "date must use YYYY-MM-DD"));
    };
    if parts.len() != 3 || year == 0 || !(1..=12).contains(&month) {
        return Err(invalid_record(
            source,
            "date must be a valid YYYY-MM-DD date",
        ));
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > max_day {
        return Err(invalid_record(
            source,
            "date must be a valid YYYY-MM-DD date",
        ));
    }
    Ok(())
}

fn invalid_record(source: &str, message: impl AsRef<str>) -> CliError {
    CliError::validation_failed(format!(
        "invalid embedded Desktop proof record {source}: {}",
        message.as_ref()
    ))
    .with_hint(
        "Keep the record at or below the strongest level supported by its explicit signals; Desktop launch/window/screenshot observations are not canvas-refresh proof.",
    )
    .with_suggested_command("powerbi-cli features list --json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(level: &str, signals: &str, desktop_version: &str) -> String {
        format!(
            r#"{{
                "schema":"powerbi-cli.desktop-proof.v1",
                "fixture":"archetypes/test",
                "date":"2026-09-04",
                "desktopVersion":{desktop_version},
                "commands":["powerbi-cli validate --strict build/test --json"],
                "signals":{signals},
                "proofLevel":"{level}"
            }}"#
        )
    }

    fn record_value() -> Value {
        serde_json::from_str(&record(
            "unit-smoke",
            r#"{"featureIds":["report.visuals.generated"]}"#,
            "null",
        ))
        .expect("record value")
    }

    fn expect_invalid(value: Value, expected: &str) {
        let text = serde_json::to_string(&value).expect("serialize invalid record");
        let error =
            parse_desktop_proof_record("invalid.json", &text).expect_err("record must be refused");
        assert!(
            error.message.contains(expected),
            "expected {expected:?} in {:?}",
            error.message
        );
    }

    #[test]
    fn placeholder_record_stays_desktop_golden_pending_without_desktop() {
        let text = record(
            "desktop-golden-pending",
            r#"{"featureIds":["report.visuals.generated"],"placeholder":true}"#,
            "null",
        );
        let parsed = parse_desktop_proof_record("placeholder.json", &text).expect("placeholder");
        assert_eq!(parsed.proof_level, ProofLevel::DesktopGoldenPending);
    }

    #[test]
    fn complete_automated_signals_support_desktop_canvas_refresh() {
        let text = record(
            "desktop-canvas-refresh",
            r#"{"featureIds":["report.visuals.generated"],"schemaGolden":true,"desktopReferencePresent":true,"currentArtifact":true,"desktopOpened":true,"canvasRendered":true,"refreshCompleted":true,"issueDialogsAbsent":true,"expectedValuesMatched":true,"automated":true}"#,
            r#""2.200.1.0""#,
        );
        let parsed = parse_desktop_proof_record("proven.json", &text).expect("proven");
        assert_eq!(parsed.proof_level, ProofLevel::DesktopCanvasRefresh);
    }

    #[test]
    fn launch_and_window_signals_cannot_claim_canvas_refresh() {
        let text = record(
            "desktop-canvas-refresh",
            r#"{"featureIds":["desktop.window-evidence"],"desktopOpened":true,"issueDialogsAbsent":true,"automated":true}"#,
            r#""2.200.1.0""#,
        );
        let error = parse_desktop_proof_record("overclaim.json", &text)
            .expect_err("window evidence must not prove the canvas");
        assert_eq!(error.code, "validation_failed");
        assert!(
            error
                .message
                .contains("exceeds the evidence-supported level")
        );
    }

    #[test]
    fn manual_claim_requires_current_artifact_and_expected_values() {
        let text = record(
            "manual-desktop-canvas-refresh",
            r#"{"featureIds":["report.visuals.generated"],"schemaGolden":true,"desktopReferencePresent":true,"desktopOpened":true,"canvasRendered":true,"refreshCompleted":true,"issueDialogsAbsent":true}"#,
            r#""2.200.1.0""#,
        );
        let error = parse_desktop_proof_record("stale.json", &text)
            .expect_err("stale bytes must not retain a current manual claim");
        assert!(error.message.contains("desktop-golden-pending"));

        let no_review = record(
            "manual-desktop-canvas-refresh",
            r#"{"featureIds":["report.visuals.generated"],"schemaGolden":true,"desktopReferencePresent":true,"currentArtifact":true,"desktopOpened":true,"canvasRendered":true,"refreshCompleted":true,"issueDialogsAbsent":true,"expectedValuesMatched":true}"#,
            r#""2.200.1.0""#,
        );
        let error = parse_desktop_proof_record("unreviewed.json", &no_review)
            .expect_err("unreviewed signals must not support a manual claim");
        assert!(error.message.contains("desktop-golden-pending"));
    }

    #[test]
    fn record_contract_refuses_invalid_identity_commands_and_feature_links() {
        let mut value = record_value();
        value["schema"] = json!("powerbi-cli.desktop-proof.v2");
        expect_invalid(value, "schema must be");

        let mut value = record_value();
        value["fixture"] = json!("../outside");
        expect_invalid(value, "without traversal");

        let mut value = record_value();
        value["desktopVersion"] = json!("  ");
        expect_invalid(value, "desktopVersion must be null or a non-empty string");

        let mut value = record_value();
        value["commands"] = json!([]);
        expect_invalid(value, "commands must not be empty");

        let mut value = record_value();
        value["commands"] = json!(["validate build/test"]);
        expect_invalid(value, "executable powerbi-cli command");

        let mut value = record_value();
        value["signals"]["featureIds"] = json!([]);
        expect_invalid(value, "signals.featureIds must not be empty");

        let mut value = record_value();
        value["signals"]["featureIds"] = json!(["  "]);
        expect_invalid(value, "signals.featureIds[0] must not be empty");

        let mut value = record_value();
        value["signals"]["featureIds"] =
            json!(["report.visuals.generated", "report.visuals.generated"]);
        expect_invalid(value, "contains duplicate");
    }

    #[test]
    fn record_contract_refuses_contradictory_evidence_signals() {
        let mut value = record_value();
        value["signals"] = json!({
            "featureIds": ["report.visuals.generated"],
            "placeholder": true,
            "desktopOpened": true
        });
        expect_invalid(value, "placeholder cannot be combined");

        let mut value = record_value();
        value["signals"]["canvasRendered"] = json!(true);
        expect_invalid(value, "canvasRendered requires");

        let mut value = record_value();
        value["signals"]["refreshCompleted"] = json!(true);
        expect_invalid(value, "refreshCompleted requires");

        let mut value = record_value();
        value["signals"]["desktopOpened"] = json!(true);
        value["signals"]["expectedValuesMatched"] = json!(true);
        expect_invalid(value, "expectedValuesMatched requires");

        let mut value = record_value();
        value["signals"]["saveReopenMatched"] = json!(true);
        expect_invalid(value, "saveReopenMatched requires");

        let mut value = record_value();
        value["signals"]["manualReview"] = json!(true);
        expect_invalid(value, "manualReview/automated signals require");
    }

    #[test]
    fn embedded_desktop_proof_index_is_sorted_and_all_records_validate() {
        let records = embedded_desktop_proof_records().expect("embedded records");
        assert!(!records.is_empty());
        let paths = records
            .iter()
            .map(|record| record.source)
            .collect::<Vec<_>>();
        let mut sorted = paths.clone();
        sorted.sort_unstable();
        assert_eq!(paths, sorted);
        assert!(
            records
                .iter()
                .all(|record| record.record.schema == DESKTOP_PROOF_SCHEMA)
        );
    }

    #[test]
    fn record_shape_rejects_unknown_fields_and_invalid_dates() {
        let unknown = record(
            "unit-smoke",
            r#"{"featureIds":["report.visuals.generated"],"guessed":true}"#,
            "null",
        );
        assert!(
            parse_desktop_proof_record("unknown.json", &unknown)
                .expect_err("unknown signal")
                .message
                .contains("unknown field")
        );

        let invalid_date = record(
            "unit-smoke",
            r#"{"featureIds":["report.visuals.generated"]}"#,
            "null",
        )
        .replace("2026-09-04", "2026-02-30");
        assert!(
            parse_desktop_proof_record("date.json", &invalid_date)
                .expect_err("invalid date")
                .message
                .contains("valid YYYY-MM-DD")
        );
    }
}
