//! Versioned, data-driven planner rules with deterministic explanations.
//!
//! The rule catalog is deliberately kept outside Rust code.  `build.rs`
//! embeds the checked-in JSON, while this module provides the strict schema,
//! validation, and one deterministic evaluation boundary used by `report
//! plan`.  Rules emit slot-agnostic proposals; a later layout engine can
//! replace the slot assignment without changing rule semantics.

use crate::{CliError, CliResult, EXIT_VALIDATION_FAILED};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub(crate) const PLANNER_RULES_SCHEMA: &str = "powerbi-cli.planner-rules.v1";
pub(crate) const PLANNER_RULES_VERSION: u32 = 1;

include!(concat!(env!("OUT_DIR"), "/planner_rule_catalog.rs"));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuleCatalog {
    pub(crate) schema: String,
    pub(crate) version: u32,
    pub(crate) rules: Vec<PlannerRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PlannerRule {
    pub(crate) id: String,
    pub(crate) summary: String,
    pub(crate) kind: String,
    pub(crate) score: i64,
    pub(crate) conditions: Vec<RuleCondition>,
    pub(crate) proposal: RuleProposal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuleCondition {
    pub(crate) signal: String,
    pub(crate) operator: String,
    pub(crate) value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuleProposal {
    pub(crate) archetype: String,
    pub(crate) template: String,
    pub(crate) visual_family: Option<String>,
    pub(crate) bindings: Vec<RuleBinding>,
    pub(crate) priority: i64,
    pub(crate) size_class: String,
    pub(crate) semantic_color: Option<String>,
    pub(crate) page: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuleBinding {
    pub(crate) role: String,
    pub(crate) source: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RulePlan {
    pub(crate) catalog: RuleCatalog,
    pub(crate) explanations: Vec<Value>,
    pub(crate) proposals: Vec<Value>,
}

/// Parse and strictly validate the embedded planner catalog.
pub(crate) fn catalog() -> CliResult<RuleCatalog> {
    parse_catalog(EMBEDDED_PLANNER_RULE_CATALOG).map_err(|message| {
        CliError::new(
            "planner.rule_catalog.invalid",
            EXIT_VALIDATION_FAILED,
            format!("embedded planner rule catalog is invalid: {message}"),
        )
        .with_hint("The bundled planner-rules.v1 catalog must pass its strict schema validation.")
    })
}

/// Strict parser exposed to unit tests and future catalog tooling.
pub(crate) fn parse_catalog(text: &str) -> Result<RuleCatalog, String> {
    let catalog = serde_json::from_str::<RuleCatalog>(text)
        .map_err(|error| format!("JSON does not match planner-rules.v1: {error}"))?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}

pub(crate) fn validate_catalog(catalog: &RuleCatalog) -> Result<(), String> {
    if catalog.schema != PLANNER_RULES_SCHEMA {
        return Err(format!(
            "schema must be {PLANNER_RULES_SCHEMA}, got {}",
            catalog.schema
        ));
    }
    if catalog.version != PLANNER_RULES_VERSION {
        return Err(format!(
            "version must be {PLANNER_RULES_VERSION}, got {}",
            catalog.version
        ));
    }
    if catalog.rules.is_empty() {
        return Err("rules must not be empty".to_string());
    }
    let mut ids = BTreeSet::new();
    for (index, rule) in catalog.rules.iter().enumerate() {
        let path = format!("rules[{index}]");
        if !valid_identifier(&rule.id) {
            return Err(format!(
                "{path}.id must be a non-empty dotted identifier, got `{}`",
                rule.id
            ));
        }
        if !ids.insert(rule.id.as_str()) {
            return Err(format!("{path}.id duplicates `{}`", rule.id));
        }
        if rule.summary.trim().is_empty() {
            return Err(format!("{path}.summary must not be empty"));
        }
        if !matches!(rule.kind.as_str(), "visual" | "measure" | "page") {
            return Err(format!(
                "{path}.kind must be visual, measure, or page, got `{}`",
                rule.kind
            ));
        }
        if !(0..=100).contains(&rule.score) {
            return Err(format!(
                "{path}.score must be between 0 and 100, got {}",
                rule.score
            ));
        }
        if rule.conditions.is_empty() {
            return Err(format!("{path}.conditions must not be empty"));
        }
        for (condition_index, condition) in rule.conditions.iter().enumerate() {
            let condition_path = format!("{path}.conditions[{condition_index}]");
            if !valid_identifier(&condition.signal) {
                return Err(format!(
                    "{condition_path}.signal must be a non-empty identifier"
                ));
            }
            if !matches!(
                condition.operator.as_str(),
                "eq" | "neq" | "gt" | "gte" | "lt" | "lte" | "exists"
            ) {
                return Err(format!(
                    "{condition_path}.operator `{}` is not supported",
                    condition.operator
                ));
            }
            if condition.operator == "exists" && !condition.value.is_boolean() {
                return Err(format!("{condition_path}.value must be boolean for exists"));
            }
            if condition.value.is_array() || condition.value.is_object() {
                return Err(format!(
                    "{condition_path}.value must be a scalar JSON value"
                ));
            }
        }
        validate_proposal(&path, &rule.kind, &rule.proposal)?;
    }
    Ok(())
}

fn validate_proposal(path: &str, kind: &str, proposal: &RuleProposal) -> Result<(), String> {
    for (field, value) in [
        ("archetype", proposal.archetype.as_str()),
        ("template", proposal.template.as_str()),
        ("sizeClass", proposal.size_class.as_str()),
    ] {
        if value.trim().is_empty() || value.chars().any(char::is_whitespace) {
            return Err(format!("{path}.proposal.{field} must be a non-empty token"));
        }
    }
    if !matches!(
        proposal.size_class.as_str(),
        "compact" | "half" | "wide" | "full"
    ) {
        return Err(format!(
            "{path}.proposal.sizeClass must be compact, half, wide, or full"
        ));
    }
    if !(0..=100).contains(&proposal.priority) {
        return Err(format!(
            "{path}.proposal.priority must be between 0 and 100"
        ));
    }
    if let Some(family) = proposal.visual_family.as_deref()
        && (family.trim().is_empty() || family.chars().any(char::is_whitespace))
    {
        return Err(format!(
            "{path}.proposal.visualFamily must be a non-empty token when present"
        ));
    }
    if let Some(color) = proposal.semantic_color.as_deref()
        && !matches!(color, "good" | "bad" | "neutral" | "warning" | "emphasis")
    {
        return Err(format!(
            "{path}.proposal.semanticColor must name a semantic token"
        ));
    }
    if let Some(page) = proposal.page.as_deref()
        && (page.trim().is_empty() || page.chars().any(char::is_whitespace))
    {
        return Err(format!("{path}.proposal.page must be a non-empty token"));
    }
    if kind == "visual" && proposal.visual_family.is_none() {
        return Err(format!(
            "{path}.proposal.visualFamily is required for {kind} rules"
        ));
    }
    for (index, binding) in proposal.bindings.iter().enumerate() {
        if binding.role.trim().is_empty() || binding.role.chars().any(char::is_whitespace) {
            return Err(format!(
                "{path}.proposal.bindings[{index}].role must be a token"
            ));
        }
        if !valid_identifier(&binding.source) {
            return Err(format!(
                "{path}.proposal.bindings[{index}].source must be a non-empty identifier"
            ));
        }
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        })
}

pub(crate) fn catalog_json() -> CliResult<Value> {
    let catalog = catalog()?;
    serde_json::to_value(catalog).map_err(|error| {
        CliError::new(
            "planner.rule_catalog.invalid",
            EXIT_VALIDATION_FAILED,
            format!("planner rule catalog cannot be serialized: {error}"),
        )
    })
}

pub(crate) fn rule_ids() -> CliResult<Vec<String>> {
    Ok(catalog()?.rules.into_iter().map(|rule| rule.id).collect())
}

/// Evaluate every catalog rule in file order.  The context is a deterministic
/// JSON object assembled by `report_plan`; shape and intent are passed
/// separately so this boundary cannot accidentally ignore either input.
pub(crate) fn evaluate(shape: &Value, intent: &Value, context: &Value) -> CliResult<RulePlan> {
    let catalog = catalog()?;
    let mut explanations = Vec::new();
    let mut proposals = Vec::new();
    for rule in &catalog.rules {
        let evidence = rule
            .conditions
            .iter()
            .map(|condition| {
                let actual = signal_value(condition.signal.as_str(), shape, intent, context);
                let matched =
                    condition_matches(condition.operator.as_str(), &actual, &condition.value);
                json!({
                    "signal": condition.signal,
                    "operator": condition.operator,
                    "expected": condition.value,
                    "actual": actual,
                    "matched": matched
                })
            })
            .collect::<Vec<_>>();
        if !evidence
            .iter()
            .all(|item| item["matched"].as_bool().unwrap_or(false))
        {
            continue;
        }
        let proposal = proposal_value(rule, &evidence, shape, intent, context);
        explanations.push(json!({
            "ruleId": rule.id,
            "score": rule.score,
            "summary": rule.summary,
            "evidence": evidence,
            "proposal": proposal
        }));
        proposals.push(proposal);
    }
    // Keep score ties deterministic while retaining catalog order as the
    // final tie-breaker.  Overview is intentionally last in the catalog but
    // appears first in the slot-agnostic page plan below.
    proposals.sort_by(|left, right| {
        right["priority"]
            .as_i64()
            .unwrap_or_default()
            .cmp(&left["priority"].as_i64().unwrap_or_default())
            .then_with(|| {
                left["ruleId"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["ruleId"].as_str().unwrap_or_default())
            })
    });
    Ok(RulePlan {
        catalog,
        explanations,
        proposals,
    })
}

fn signal_value(signal: &str, shape: &Value, intent: &Value, context: &Value) -> Value {
    if let Some(value) = context.get(signal) {
        return value.clone();
    }
    match signal {
        "shapeKind" => shape.get("kind").cloned().unwrap_or(Value::Null),
        "factCount" => Value::from(
            shape
                .get("facts")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        ),
        "highCardinalityCount" => Value::from(
            shape
                .get("highCardinality")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        ),
        "targetCount" => Value::from(
            intent
                .get("kpis")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|kpi| kpi.get("target").is_some_and(|target| !target.is_null()))
                .count(),
        ),
        "alertCount" => Value::from(
            intent
                .get("alerts")
                .and_then(Value::as_array)
                .map_or(0, Vec::len),
        ),
        _ => Value::Null,
    }
}

fn condition_matches(operator: &str, actual: &Value, expected: &Value) -> bool {
    match operator {
        "exists" => {
            expected.as_bool().unwrap_or(false)
                == (!actual.is_null() && actual != &Value::Bool(false))
        }
        "eq" => scalar_equal(actual, expected),
        "neq" => !scalar_equal(actual, expected),
        "gt" => numeric_pair(actual, expected).is_some_and(|(left, right)| left > right),
        "gte" => numeric_pair(actual, expected).is_some_and(|(left, right)| left >= right),
        "lt" => numeric_pair(actual, expected).is_some_and(|(left, right)| left < right),
        "lte" => numeric_pair(actual, expected).is_some_and(|(left, right)| left <= right),
        _ => false,
    }
}

fn scalar_equal(left: &Value, right: &Value) -> bool {
    if let Some((left, right)) = numeric_pair(left, right) {
        return (left - right).abs() < f64::EPSILON;
    }
    left == right
}

fn numeric_pair(left: &Value, right: &Value) -> Option<(f64, f64)> {
    Some((left.as_f64()?, right.as_f64()?))
}

fn proposal_value(
    rule: &PlannerRule,
    evidence: &[Value],
    shape: &Value,
    intent: &Value,
    context: &Value,
) -> Value {
    let mut proposal = Map::new();
    proposal.insert("kind".to_string(), Value::String(rule.kind.clone()));
    proposal.insert("ruleId".to_string(), Value::String(rule.id.clone()));
    proposal.insert(
        "ruleIds".to_string(),
        Value::Array(vec![Value::String(rule.id.clone())]),
    );
    proposal.insert("score".to_string(), Value::from(rule.score));
    proposal.insert(
        "archetype".to_string(),
        Value::String(rule.proposal.archetype.clone()),
    );
    proposal.insert(
        "template".to_string(),
        Value::String(rule.proposal.template.clone()),
    );
    if let Some(family) = rule.proposal.visual_family.as_deref() {
        proposal.insert(
            "visualFamily".to_string(),
            Value::String(family.to_string()),
        );
    }
    proposal.insert("priority".to_string(), Value::from(rule.proposal.priority));
    proposal.insert(
        "sizeClass".to_string(),
        Value::String(rule.proposal.size_class.clone()),
    );
    if let Some(color) = rule.proposal.semantic_color.as_deref() {
        proposal.insert(
            "semanticColor".to_string(),
            Value::String(color.to_string()),
        );
    }
    if let Some(page) = rule.proposal.page.as_deref() {
        proposal.insert("page".to_string(), Value::String(page.to_string()));
    }
    let bindings = rule
        .proposal
        .bindings
        .iter()
        .map(|binding| {
            let value = signal_value(&binding.source, shape, intent, context);
            if let Some(fields) = value.as_array() {
                json!({"role": binding.role, "fields": fields, "source": binding.source})
            } else {
                json!({"role": binding.role, "field": value, "source": binding.source})
            }
        })
        .collect::<Vec<_>>();
    proposal.insert("bindings".to_string(), Value::Array(bindings));
    proposal.insert("evidence".to_string(), Value::Array(evidence.to_vec()));
    Value::Object(proposal)
}

impl RulePlan {
    pub(crate) fn to_value(&self) -> Value {
        json!({
            "schema": self.catalog.schema,
            "version": self.catalog.version,
            "rules": self.explanations,
            "proposals": self.proposals
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn embedded_catalog_passes_strict_validation_and_has_unique_ids() {
        let catalog = catalog().expect("embedded planner catalog");
        validate_catalog(&catalog).expect("strict catalog validation");
        let ids = catalog
            .rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<Vec<_>>();
        let unique = ids.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), unique.len());
        assert!(ids.iter().any(|id| id.ends_with("time-series")));
    }

    #[test]
    fn strict_catalog_rejects_unknown_fields() {
        let invalid = r#"{
            "schema":"powerbi-cli.planner-rules.v1",
            "version":1,
            "rules":[],
            "extra":true
        }"#;
        let error = parse_catalog(invalid).expect_err("unknown field must be rejected");
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn condition_comparisons_are_typed_and_deterministic() {
        assert!(condition_matches("gte", &json!(2), &json!(1)));
        assert!(condition_matches("eq", &json!(2), &json!(2.0)));
        assert!(!condition_matches("eq", &json!("2"), &json!(2)));
        assert!(condition_matches("exists", &json!(true), &json!(true)));
        assert!(!condition_matches("exists", &Value::Null, &json!(true)));
    }

    #[test]
    fn every_catalog_rule_can_fire_with_a_minimal_context() {
        let intent =
            json!({"kpis":[{"name":"Revenue","target":100}], "alerts":[{"measure":"Revenue"}]});
        let base_context = |declared_measure_count| {
            json!({
                "hasTimeAxis": true,
                "measureCount": 2,
                "declaredMeasureCount": declared_measure_count,
                "categoryCount": 1,
                "columnCount": 6,
                "targetCount": 1,
                "alertCount": 1,
                "highCardinalityCount": 1,
                "primaryMeasure": "Fact[Revenue]",
                "secondaryMeasure": "Fact[Target]",
                "primaryCategory": "Dim[Name]",
                "timeAxis": "DimDate[Date]",
                "detailColumns": ["Fact[Revenue]"],
                "alertColumns": ["Dim[Name]"]
            })
        };
        let mut scores = BTreeMap::new();
        for (kind, declared_measure_count) in [
            ("star", 0),
            ("flat", 2),
            ("snowflake", 2),
            ("multi-fact", 2),
            ("ambiguous", 2),
        ] {
            let shape = json!({
                "kind": kind,
                "facts": [{"table": "Fact"}],
                "highCardinality": [{"column": "Customer"}]
            });
            let plan = evaluate(&shape, &intent, &base_context(declared_measure_count))
                .expect("evaluate catalog");
            for rule in &plan.explanations {
                let id = rule["ruleId"].as_str().expect("rule id").to_owned();
                let score = rule["score"].as_i64().expect("rule score");
                scores.insert(id, score);
            }
        }
        let catalog = catalog().expect("embedded planner catalog");
        for rule in &catalog.rules {
            assert!(
                scores.contains_key(rule.id.as_str()),
                "rule did not fire: {}",
                rule.id
            );
            assert_eq!(
                scores[rule.id.as_str()],
                rule.score,
                "score for {}",
                rule.id
            );
        }
    }

    #[test]
    fn catalog_scores_are_stable_and_documented() {
        let shape = json!({"kind":"star", "facts":[{"table":"Fact"}], "highCardinality":[{"column":"Customer"}]});
        let intent =
            json!({"kpis":[{"name":"Revenue","target":100}], "alerts":[{"measure":"Revenue"}]});
        let context = json!({
            "hasTimeAxis": true,
            "measureCount": 2,
            "declaredMeasureCount": 2,
            "categoryCount": 1,
            "columnCount": 6,
            "targetCount": 1,
            "alertCount": 1,
            "highCardinalityCount": 1,
            "primaryMeasure": "Fact[Revenue]",
            "secondaryMeasure": "Fact[Target]",
            "primaryCategory": "Dim[Name]",
            "timeAxis": "DimDate[Date]",
            "detailColumns": ["Fact[Revenue]"],
            "alertColumns": ["Dim[Name]"]
        });
        let plan = evaluate(&shape, &intent, &context).expect("evaluate catalog");
        let catalog = catalog().expect("embedded planner catalog");
        let expected = [
            ("planner.time-series", 92),
            ("planner.category-ranking", 84),
            ("planner.scatter-focus", 88),
            ("planner.detail-table", 78),
            ("planner.measure-target", 86),
            ("planner.measure-total", 74),
            ("planner.alert-exception-list", 89),
            ("planner.high-cardinality-drillthrough", 72),
            ("planner.shape-flat-template", 61),
            ("planner.shape-snowflake-template", 61),
            ("planner.shape-multi-fact-template", 61),
            ("planner.shape-ambiguous-template", 61),
            ("planner.overview", 60),
        ];
        for (id, score) in expected {
            let rule = catalog
                .rules
                .iter()
                .find(|rule| rule.id == id)
                .unwrap_or_else(|| panic!("catalog missing {id}"));
            assert_eq!(rule.score, score, "score changed for {id}");
        }
        for (id, score) in expected {
            let explanation = plan.explanations.iter().find(|item| item["ruleId"] == id);
            if id == "planner.detail-table" || id.starts_with("planner.shape-") {
                assert!(
                    explanation.is_none(),
                    "shape/detail rule should not fire for this star context"
                );
            } else {
                assert_eq!(explanation.expect("fired rule")["score"], score);
            }
        }
        let readme = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"));
        let skill = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/skills/powerbi-cli/SKILL.md"
        ));
        for rule in &catalog.rules {
            assert!(readme.contains(&rule.id), "README drifted for {}", rule.id);
            assert!(skill.contains(&rule.id), "SKILL.md drifted for {}", rule.id);
        }
    }
}
