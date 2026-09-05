//! Bounded, score-ordered template alternatives over the primary plan's visuals.
use crate::design::grid;
use crate::planner_rules::{self, TemplateChoice};
use crate::{CliError, CliResult};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(crate) struct Candidate {
    pub(crate) spec: Value,
    pub(crate) structure_hash: String,
    pub(crate) score: i64,
    pub(crate) decision_diff: Vec<Value>,
}

pub(crate) fn generate(
    schema: &Value,
    primary: &Value,
    shape: &Value,
    intent: &Value,
    context: &Value,
    count: usize,
) -> CliResult<Vec<Candidate>> {
    let rules = planner_rules::catalog()?.variants;
    if count == 0 || count > rules.max_count {
        return Err(argument_error(format!(
            "--variants must be between 1 and {}",
            rules.max_count
        )));
    }
    let pages = primary["pages"]
        .as_array()
        .ok_or_else(|| argument_error("primary plan has no pages"))?;
    let mut combinations = vec![(Vec::<Value>::new(), Vec::<Value>::new(), 0i64)];
    for (page_index, page) in pages.iter().enumerate() {
        let mut page_context = context.clone();
        page_context["variantVisualCount"] = json!(page["visuals"].as_array().map_or(0, Vec::len));
        let choices = planner_rules::variant_choices(shape, intent, &page_context)?;
        let mut options = Vec::new();
        for choice in choices {
            if let Some(placed) = place_page(page, &choice)? {
                options.push((placed, choice));
            }
        }
        let mut expanded = Vec::new();
        for (prefix, decisions, score) in combinations {
            for (placed, choice) in &options {
                let mut next_pages = prefix.clone();
                next_pages.push(placed.clone());
                let mut next_decisions = decisions.clone();
                next_decisions.push(json!({
                    "pointer": format!("/pages/{page_index}/template"),
                    "before": page.get("template").cloned().unwrap_or(Value::Null),
                    "after": choice.template, "ruleId": choice.rule_id,
                    "score": choice.score, "reason": choice.reason,
                    "visualCount": page["visuals"].as_array().map_or(0, Vec::len)
                }));
                for (index, visual) in placed["visuals"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    next_decisions.push(json!({
                        "pointer": format!("/pages/{page_index}/visuals/{index}"),
                        "before": {"layout": page["visuals"][index]["layout"], "slot": page["visuals"][index]["slot"]},
                        "after": {"slot": visual["slot"]},
                        "ruleId": choice.rule_id,
                        "reason": "Resolve this visual through a compatible named slot in the selected template"
                    }));
                }
                expanded.push((next_pages, next_decisions, score + choice.score));
            }
        }
        expanded.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| structural_hash(&json!(a.0)).cmp(&structural_hash(&json!(b.0))))
        });
        expanded.truncate(rules.max_count * rules.choices.len());
        combinations = expanded;
    }
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    let mut refusals = Vec::new();
    for (pages, decision_diff, score) in combinations {
        let mut spec = primary.clone();
        spec["schema"] = json!("powerbi-cli.dashboard.v2");
        spec["pages"] = json!(pages);
        if let Some(required) = spec["proof"].get("required").cloned() {
            spec["proof"] = json!({"desktop":{"level":required}});
        }
        let hash = structural_hash(&spec["pages"]);
        if !seen.insert(hash.clone()) {
            continue;
        }
        match validate_candidate(schema, &spec) {
            Ok(()) => candidates.push(Candidate {
                spec,
                structure_hash: hash,
                score,
                decision_diff,
            }),
            Err(error) => refusals.push(error.message),
        }
        if candidates.len() == count {
            return Ok(candidates);
        }
    }
    Err(CliError::new(crate::rules::PLAN_VARIANTS_INSUFFICIENT, crate::EXIT_VALIDATION_FAILED,
        format!("requested {count} distinct variants but only {} compiler-valid template combinations exist; {}", candidates.len(), refusals.join("; ")))
        .with_pointer("/variants")
        .with_hint("Request fewer variants or supply more schema/profile/intent evidence; no candidate files were written.")
        .with_suggested_command("powerbi-cli report plan --schema <schema.json> --intent <intent.json> --variants 1 --out <dashboard.json> --json"))
}

pub(crate) fn argument_error(message: impl Into<String>) -> CliError {
    CliError::invalid_args(message)
        .with_pointer("/variants")
        .with_hint(
            "Provide a positive --variants count within the catalog limit and an --out filename.",
        )
        .with_suggested_command("powerbi-cli capabilities --for 'report plan' --json")
}

fn place_page(page: &Value, choice: &TemplateChoice) -> CliResult<Option<Value>> {
    let template = grid::template(&choice.template)?;
    let mut placed = page.clone();
    placed["template"] = json!(choice.template);
    let mut used = BTreeSet::new();
    for visual in placed["visuals"].as_array_mut().into_iter().flatten() {
        let kind = visual["type"].as_str().unwrap_or("");
        let family = match kind {
            "card" => "card",
            "tableEx" | "matrix" | "pivotTable" => "table",
            _ => "chart",
        };
        let slot = template
            .slots
            .iter()
            .filter(|slot| {
                !used.contains(&slot.name) && slot.name != "heading" && slot.name != "rail"
            })
            .filter(|slot| {
                slot.min_family.as_deref() == Some(family)
                    || slot.preferred_families.iter().any(|value| value == kind)
            })
            .min_by_key(|slot| {
                (
                    !slot.preferred_families.iter().any(|value| value == kind),
                    slot.row,
                    slot.col,
                    &slot.name,
                )
            });
        let Some(slot) = slot else { return Ok(None) };
        used.insert(slot.name.clone());
        visual["slot"] = json!(slot.name);
        visual
            .as_object_mut()
            .expect("planner visual object")
            .remove("layout");
    }
    Ok(Some(placed))
}

pub(crate) fn validate_candidate(schema: &Value, spec: &Value) -> CliResult<()> {
    // This helper uses the same compiler and schema validation as spec validate
    // with --schema; it does not strip or bypass any unsupported section.
    let (compiled, _) =
        crate::report_build::compile_dashboard_for_explain_with_profile(schema, spec, None)?;
    let validation = crate::schema::validate_schema_value(&compiled);
    if !validation.errors.is_empty() {
        return Err(CliError::validation_failed(validation.errors.join("; ")));
    }
    Ok(())
}

fn structural_hash(pages: &Value) -> String {
    let structure = pages
        .as_array()
        .into_iter()
        .flatten()
        .map(|page| {
            json!({
                "template":page["template"], "size":page["size"],
                "visuals":page["visuals"].as_array().into_iter().flatten().map(|visual| json!({
                    "type":visual["type"], "bindings":visual["bindings"], "slot":visual["slot"],
                    "layout":visual["layout"], "topnGuard":visual["topnGuard"]
                })).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&structure).expect("JSON structure"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incompatible_visuals_refuse_instead_of_duplicating_candidates() {
        let primary = json!({"pages":[{"visuals":(0..30).map(|_| json!({"type":"card"})).collect::<Vec<_>>() }]});
        let result = generate(
            &json!({}),
            &primary,
            &json!({}),
            &json!({}),
            &json!({"measureCount":2}),
            1,
        );
        let error = result.err().expect("no template can fit thirty cards");
        assert_eq!(error.code, crate::rules::PLAN_VARIANTS_INSUFFICIENT);
    }

    #[test]
    fn variant_catalog_filters_evidence_and_orders_scores() {
        let choices =
            planner_rules::variant_choices(&json!({}), &json!({}), &json!({"measureCount":1}))
                .unwrap();
        assert_eq!(
            choices
                .iter()
                .map(|choice| choice.template.as_str())
                .collect::<Vec<_>>(),
            ["overview", "kpi-strip-trend-breakdown"]
        );
        assert!(
            choices
                .windows(2)
                .all(|pair| pair[0].score >= pair[1].score)
        );
        assert!(
            planner_rules::variant_choices(&json!({}), &json!({}), &json!({}))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn variant_catalog_rejects_invalid_bounds_and_template_names() {
        let mut catalog = planner_rules::catalog().unwrap();
        catalog.variants.max_count = 0;
        assert!(planner_rules::parse_catalog(&serde_json::to_string(&catalog).unwrap()).is_err());
        catalog.variants.max_count = 8;
        catalog.variants.choices[0].template = "not-a-template".into();
        assert!(planner_rules::parse_catalog(&serde_json::to_string(&catalog).unwrap()).is_err());
    }
    #[test]
    fn structure_hash_ignores_labels_but_tracks_templates_and_bindings() {
        let first = json!([{"id":"a","template":"overview","visuals":[{"type":"card","title":"A","bindings":[]}]}]);
        let mut second = first.clone();
        second[0]["visuals"][0]["title"] = json!("B");
        assert_eq!(structural_hash(&first), structural_hash(&second));
        second[0]["template"] = json!("comparison");
        assert_ne!(structural_hash(&first), structural_hash(&second));
        second = first.clone();
        second[0]["visuals"][0]["bindings"] = json!([{"role":"Values","field":"Events[Total]"}]);
        assert_ne!(structural_hash(&first), structural_hash(&second));
    }
}
