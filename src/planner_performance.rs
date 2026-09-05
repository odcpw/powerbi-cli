//! Profile-backed guard proposals; planning never mutates a project.
use crate::planner_rules::PerformanceRules;
use crate::report_build::{page_name, visual_name};
use crate::{CliError, CliResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuardOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    threshold: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top: Option<u64>,
}

pub(crate) fn parse_overrides(value: Option<&Value>) -> CliResult<Option<GuardOverrides>> {
    let Some(value) = value else { return Ok(None) };
    let fail = |message: String| {
        CliError::invalid_args(message)
            .with_pointer("/guards")
            .with_hint("Use guards: {threshold: 200, top: 50} with positive integer values.")
            .with_suggested_command("powerbi-cli capabilities --for 'report plan' --json")
    };
    let parsed: GuardOverrides = serde_json::from_value(value.clone())
        .map_err(|error| fail(format!("invalid intent guards: {error}")))?;
    if parsed.threshold == Some(0) || parsed.top == Some(0) {
        return Err(fail(
            "intent guards threshold and top must be positive".to_string(),
        ));
    }
    Ok(Some(parsed))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(
    schema: &Value,
    profile: Option<&Value>,
    shape: &Value,
    spec: &mut Value,
    overrides: Option<&GuardOverrides>,
    project: Option<&Path>,
    rules: &PerformanceRules,
    decisions: &mut Vec<Value>,
) -> CliResult<Value> {
    let threshold = overrides
        .and_then(|item| item.threshold)
        .unwrap_or(rules.threshold);
    let top = overrides.and_then(|item| item.top).unwrap_or(rules.top);
    let mut measures = BTreeSet::new();
    for table in schema["tables"].as_array().into_iter().flatten() {
        for measure in table["measures"].as_array().into_iter().flatten() {
            if let (Some(table), Some(name)) = (table["name"].as_str(), measure["name"].as_str()) {
                measures.insert(format!("{table}[{name}]"));
            }
        }
    }
    for measure in spec["model"]["measures"].as_array().into_iter().flatten() {
        if let (Some(table), Some(name)) = (measure["table"].as_str(), measure["name"].as_str()) {
            measures.insert(format!("{table}[{name}]"));
        }
    }
    let mut ops = Vec::new();
    let mut commands = Vec::new();
    let mut generated_measure = None;
    for (pi, page) in spec["pages"]
        .as_array_mut()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let page_id = page["id"].as_str().unwrap_or("overview").to_string();
        for (vi, visual) in page["visuals"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let bindings = visual["bindings"].as_array().cloned().unwrap_or_default();
            let candidate = bindings
                .iter()
                .filter(|binding| matches!(binding["role"].as_str(), Some("Category" | "Rows")))
                .filter_map(|binding| {
                    let field = binding["field"].as_str()?;
                    let count = distinct_count(profile?, field)?;
                    (count > threshold).then_some((field.to_string(), count))
                })
                .max_by_key(|(_, count)| *count);
            let Some((field, count)) = candidate else {
                continue;
            };
            let order_by = bindings
                .iter()
                .filter_map(|binding| binding["field"].as_str())
                .find(|field| {
                    measures
                        .iter()
                        .any(|measure| measure.eq_ignore_ascii_case(field))
                })
                .map(ToOwned::to_owned);
            let (order_by, strategy) = if let Some(order_by) = order_by {
                (order_by, "first-bound-measure")
            } else {
                let facts = shape["facts"].as_array().filter(|facts| facts.len() == 1);
                let table = facts.and_then(|facts| facts[0]["table"].as_str()).ok_or_else(|| {
                    crate::report_build::spec_missing_input(
                        format!("/pages/{pi}/visuals/{vi}/topnGuard/orderBy"),
                        "topnGuard.orderBy",
                        "high-cardinality grouping needs a bound measure or one unambiguous fact table",
                        json!({"orderBy": "Facts[Count]"}),
                    )
                })?;
                let name = "Planner Guard Count";
                let reference = format!("{table}[{name}]");
                if measures
                    .iter()
                    .any(|measure| measure.eq_ignore_ascii_case(&reference))
                {
                    return Err(crate::report_build::spec_missing_input(
                        format!("/pages/{pi}/visuals/{vi}/topnGuard/orderBy"),
                        "topnGuard.orderBy",
                        "generated guard measure name already exists; bind an explicit ranking measure",
                        json!({"orderBy": reference}),
                    ));
                }
                generated_measure = Some(json!({
                    "table": table, "name": name,
                    "expression": format!("COUNTROWS('{}')", table.replace('\'', "''")),
                    "formatString": "#,0"
                }));
                (reference, "generated-countrows")
            };
            visual["topnGuard"] = json!({"orderBy": order_by, "top": top});
            let handle = format!(
                "visual:{}:{}",
                page_name(&page_id),
                visual_name(visual["id"].as_str().unwrap_or("visual"))
            );
            ops.push(guard_operation(&handle, &field, &order_by, top)?);
            commands.push(format!(
                "powerbi-cli report visuals set-topn-guard --project <project-dir> --handle {} --field {} --order-by {} --top {top} --dry-run --json",
                crate::cli_support::shell_arg(&handle), crate::cli_support::shell_arg(&field),
                crate::cli_support::shell_arg(&order_by)));
            decisions.push(json!({
                "kind": "performance-guard", "ruleId": crate::rules::PLANNER_CARDINALITY_GUARD,
                "handle": handle, "field": field, "distinctCount": count,
                "threshold": threshold, "top": top, "orderBy": order_by,
                "rankingStrategy": strategy,
                "shape": shape["kind"],
                "reason": "Category/Rows distinct count exceeds the configured threshold; rank by the first bound measure or a cheap COUNTROWS measure on the single classified fact table"
            }));
        }
    }
    if let Some(measure) = generated_measure {
        if !spec["model"].is_object() {
            spec["model"] = json!({});
        }
        if !spec["model"]["measures"].is_array() {
            spec["model"]["measures"] = json!([]);
        }
        spec["model"]["measures"]
            .as_array_mut()
            .expect("measure array")
            .push(measure);
    }
    let mut findings = Vec::new();
    if let Some(project) = project {
        let resolved = crate::resolve_project(project)?;
        for finding in crate::lint::planner_buffer_findings(&resolved)? {
            if finding["referenceCount"].as_u64().unwrap_or(0) < rules.buffer_reuse_threshold {
                continue;
            }
            decisions.push(json!({
                "kind": "performance-recommendation", "ruleId": crate::rules::M_UNBUFFERED_REUSE,
                "handle": finding["handle"], "step": finding["step"],
                "referenceCount": finding["referenceCount"],
                "threshold": rules.buffer_reuse_threshold, "shape": shape["kind"],
                "recommendation": "Review query folding and repeated evaluation before adding Table.Buffer",
                "reason": finding["message"], "mutatesProject": false,
                "analysisBoundary": "heuristic"
            }));
            findings.push(finding);
        }
    }
    Ok(json!({
        "threshold": threshold, "top": top,
        "bufferReuseThreshold": rules.buffer_reuse_threshold,
        "bufferAnalysis": if project.is_some() { "analyzed" } else { "project-not-supplied" },
        "target": "specV2", "ops": {"schema": "powerbi-cli.ops.v1", "ops": ops},
        "kernelAvailable": crate::ops::registered_kernel_tags().contains(&"setTopNGuard"),
        "previewCommands": commands,
        "findings": findings
    }))
}

// Single serialization boundary shared with the SetTopNGuard command payload.
fn guard_operation(handle: &str, field: &str, order_by: &str, top: u64) -> CliResult<Value> {
    let payload = json!({"op": "setTopNGuard", "handle": handle, "field": field,
        "orderBy": order_by, "top": top});
    if crate::ops::registered_kernel_tags().contains(&"setTopNGuard") {
        let operation: crate::ops::Op = serde_json::from_value(payload).map_err(|error| {
            CliError::unexpected(format!("invalid generated TopN operation: {error}"))
        })?;
        return serde_json::to_value(operation)
            .map_err(|error| CliError::unexpected(format!("serialize TopN operation: {error}")));
    }
    Ok(payload)
}

fn distinct_count(profile: &Value, field: &str) -> Option<u64> {
    let (table, column) = crate::report_filter_shapes::parse_field_reference(field).ok()?;
    profile["tables"].as_array()?.iter().find(|item| {
        item["name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(&table))
    })?["columns"]
        .as_array()?
        .iter()
        .find(|item| {
            item["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(&column))
        })?["distinctCount"]
        .as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbound_high_cardinality_rows_generate_cheap_count_for_one_fact_only() {
        let schema = json!({"tables":[{"name":"Facts", "measures":[]}]});
        let profile =
            json!({"tables":[{"name":"Facts","columns":[{"name":"Group","distinctCount":201}]}]});
        let original = json!({"pages":[{"id":"detail","visuals":[{
            "id":"matrix","bindings":[{"role":"Rows","field":"Facts[Group]"}]
        }]}]});
        let rules = crate::planner_rules::catalog().unwrap().performance;
        let mut spec = original.clone();
        let mut decisions = Vec::new();
        let result = plan(
            &schema,
            Some(&profile),
            &json!({"kind":"flat","facts":[{"table":"Facts"}]}),
            &mut spec,
            None,
            None,
            &rules,
            &mut decisions,
        )
        .unwrap();
        assert_eq!(
            spec["model"]["measures"][0]["expression"],
            "COUNTROWS('Facts')"
        );
        assert_eq!(decisions[0]["rankingStrategy"], "generated-countrows");
        assert_eq!(
            result["ops"]["ops"][0]["orderBy"],
            "Facts[Planner Guard Count]"
        );
        let error = plan(
            &schema,
            Some(&profile),
            &json!({"kind":"multi-fact","facts":[{"table":"A"},{"table":"B"}]}),
            &mut original.clone(),
            None,
            None,
            &rules,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.code, "spec.missing_input");
        let error = plan(
            &json!({"tables":[{"name":"Facts","measures":[{"name":"planner guard count"}]}]}),
            Some(&profile),
            &json!({"kind":"flat","facts":[{"table":"Facts"}]}),
            &mut original.clone(),
            None,
            None,
            &rules,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.code, "spec.missing_input");
    }

    #[test]
    fn absent_cardinality_never_invents_a_guard() {
        let mut spec = json!({"pages":[{"id":"detail","visuals":[{
            "id":"matrix","bindings":[{"role":"Rows","field":"Facts[Group]"}]
        }]}]});
        let rules = crate::planner_rules::catalog().unwrap().performance;
        let result = plan(
            &json!({}),
            None,
            &json!({}),
            &mut spec,
            None,
            None,
            &rules,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(result["ops"]["ops"], json!([]));
        assert!(spec["pages"][0]["visuals"][0].get("topnGuard").is_none());
    }
    #[test]
    fn guards_reject_unknown_noninteger_zero_and_negative_values() {
        for value in [
            json!({"top": 0}),
            json!({"threshold": -1}),
            json!({"top": 1.5}),
            json!({"other": 1}),
            json!(null),
            json!([]),
        ] {
            let error = parse_overrides(Some(&value)).unwrap_err();
            assert_eq!(error.code, "invalid_args");
            assert_eq!(error.pointer(), Some("/guards"));
            assert!(error.hint.is_some());
            assert!(!error.suggested_commands.is_empty());
        }
    }
}
