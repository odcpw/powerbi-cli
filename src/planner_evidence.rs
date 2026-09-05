//! Planner evidence requirements and recovery diagnostics, before layout or writes.
use crate::report_build::spec_missing_input_with_command;
use crate::{CliError, CliResult, command_arg};
use serde_json::{Value, json};
use std::path::Path;
pub(crate) struct Evidence {
    pub(crate) tables: Vec<String>,
    pub(crate) date_count: usize,
    pub(crate) measure_count: usize,
    pub(crate) intent_signal_count: usize,
    pub(crate) fact_override: Option<String>,
}
pub(crate) fn validate(
    evidence: Evidence,
    shape: &Value,
    schema_path: &Path,
) -> CliResult<Vec<String>> {
    let thresholds = crate::planner_rules::catalog()?.evidence_thresholds;
    let fields_command = format!(
        "powerbi-cli report spec fields --schema {} --json",
        command_arg(schema_path)
    );
    if evidence.intent_signal_count < thresholds.minimum_intent_signals {
        return Err(plan_missing_input(
            "/intent/questions",
            "intent.questions",
            "the intent document contains no report question, KPI, comparison, period, alert, or archetype",
            json!({"questions": ["Which categories contribute most to the total?"]}),
            &format!(
                "powerbi-cli report plan --schema {} --intent <intent.json> --json",
                command_arg(schema_path)
            ),
        ));
    }
    if evidence.date_count < thresholds.minimum_date_columns {
        return Err(plan_missing_input(
            "/schema/tables",
            "schema.tables[].columns[].type=date",
            "no supported date axis is available; declare a date column before choosing a layout",
            json!({"columns": [{"name": "Date", "dataType": "date"}]}),
            &fields_command,
        ));
    }
    if evidence.measure_count < thresholds.minimum_measures {
        return Err(plan_missing_input(
            "/intent/kpis/0/measure",
            "intent.kpis[0].measure",
            "no declared measure is available; declare a schema measure and bind the intended KPI instead of guessing a SUM",
            json!({"kpis": [{"name": "Revenue", "measure": "FactSales[Revenue]"}]}),
            &fields_command,
        ));
    }
    let tables = evidence.tables;
    let mut facts = shape["facts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|fact| fact["table"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    if let Some(selection) = evidence.fact_override {
        if tables.contains(&selection) {
            facts = vec![selection];
        } else {
            facts.clear();
        }
    } else if facts.is_empty() && tables.len() == 1 {
        facts = tables.clone();
    }
    if facts.is_empty() || facts.len() > thresholds.maximum_fact_candidates {
        return Err(plan_missing_input(
            "/intent/model/factTable", "intent.model.factTable",
            format!("select one schema table in the intent file; candidate tables: {}", tables.join(", ")),
            json!({"model": {"factTable": tables.first()}, "candidates": tables}),
            &format!("powerbi-cli profile infer --schema {} --rows <rows.csv|rows.json> --out <profile.json> --json", command_arg(schema_path)),
        ).with_suggested_command(format!(
            "powerbi-cli report plan --schema {} --profile <profile.json> --intent <intent.json> --json", command_arg(schema_path)
        )));
    }
    Ok(facts)
}
pub(crate) fn plan_missing_input(
    pointer: impl Into<String>,
    field: impl Into<String>,
    reason: impl Into<String>,
    example: Value,
    command: &str,
) -> CliError {
    let candidates = example
        .get("candidates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut error = spec_missing_input_with_command(pointer, field, reason, example, command)
        .with_candidates(candidates);
    error.code = "plan.missing_input";
    error.message = error.message.replace("dashboard-spec", "planner");
    error.hint = Some("Supply the named schema or intent field and rerun report plan; no default layout is applied.".into());
    error
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fact_override_must_name_a_real_schema_table() {
        for (selection, expected) in [("Events", true), ("Absent", false)] {
            let evidence = Evidence {
                tables: vec!["Events".into(), "Other".into()],
                date_count: 1,
                measure_count: 1,
                intent_signal_count: 1,
                fact_override: Some(selection.into()),
            };
            let result = validate(evidence, &json!({"facts":[]}), Path::new("schema.json"));
            assert_eq!(result.is_ok(), expected);
            if let Err(error) = result {
                assert_eq!(error.code, "plan.missing_input");
                assert_eq!(error.pointer(), Some("/intent/model/factTable"));
                assert_eq!(
                    error.candidates().unwrap(),
                    &vec![json!("Events"), json!("Other")]
                );
            }
        }
    }
}
