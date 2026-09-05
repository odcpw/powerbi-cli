//! Materialize the catalog's narrative flow using the shared grid boundary.

use super::*;
use crate::design::grid::{self, PageSize, SlotPosition};
use crate::report_build::spec_missing_input_with_command;

pub(super) fn build(
    legacy: &Value,
    intent: &Intent,
    rules: &RulePlan,
    model: &PlanModel<'_>,
    shape: &Value,
    context: &Value,
) -> CliResult<(Value, Value)> {
    let rule = rules
        .catalog
        .rules
        .iter()
        .find(|rule| rule.proposal.narrative_flow.is_some())
        .ok_or_else(|| CliError::unexpected("planner catalog has no narrative flow rule"))?;
    let flow = rule
        .proposal
        .narrative_flow
        .as_ref()
        .expect("selected flow rule");
    let primary = context["primaryMeasure"].as_str().ok_or_else(|| {
        CliError::validation_failed("narrative plan requires a resolved primary measure")
    })?;
    let mut categories = model
        .category_columns
        .iter()
        .map(|field| field.reference.clone())
        .collect::<Vec<_>>();
    categories.sort();
    categories.dedup();
    let mut high = shape["highCardinality"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let table = entry["table"].as_str()?;
            let column = entry["column"].as_str()?;
            model.has_column(table, column).then(|| {
                (
                    entry["distinct"].as_u64().unwrap_or(0),
                    field_reference(table, column),
                )
            })
        })
        .collect::<Vec<_>>();
    high.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let drill_target = high.first().map(|(_, field)| field.clone());
    categories.truncate(flow.max_breakdowns);
    if let Some(target) = &drill_target
        && !categories.contains(target)
    {
        if categories.len() == flow.max_breakdowns {
            categories.pop();
        }
        categories.push(target.clone());
        categories.sort();
    }
    let rail = rail_field(model, intent)?;
    let proposal = |archetype: &str| {
        rules
            .proposals
            .iter()
            .find(|item| item["archetype"] == archetype)
    };
    let mut pages = Vec::new();
    let mut ordering = Vec::new();
    let mut drill_source = None;
    for stage in &flow.page_order {
        let template = if stage == "overview" {
            flow.overview_templates
                .get(shape["kind"].as_str().unwrap_or_default())
                .unwrap_or(&flow.templates[stage])
        } else if stage == "detail" {
            flow.detail_templates
                .get(shape["kind"].as_str().unwrap_or_default())
                .unwrap_or(&flow.templates[stage])
        } else {
            &flow.templates[stage]
        };
        let mut candidates = Vec::new();
        match stage.as_str() {
            "overview" => {
                let mut visuals = vec![
                    json!({"id":"primary_kpi", "type":"card", "title":"Primary KPI", "slot":"kpi.1", "bindings":[{"role":"Values", "field":primary}]}),
                ];
                if let Some(trend) =
                    proposal("time-series").and_then(|p| proposal_visual(p, "primary", 0))
                {
                    visuals.push(trend);
                }
                if let Some(category) = categories.first() {
                    visuals.push(ranking(category, primary, "secondary"));
                }
                candidates.push(("overview".to_string(), "Overview".to_string(), visuals));
            }
            "trend" => {
                if let Some(visual) =
                    proposal("time-series").and_then(|p| proposal_visual(p, "primary", 0))
                {
                    candidates.push((stage.clone(), "Trend".to_string(), vec![visual]));
                }
            }
            "breakdown" => {
                for (index, category) in categories.iter().enumerate() {
                    let id = format!("breakdown_{}", index + 1);
                    if drill_target.as_ref() == Some(category) {
                        drill_source = Some(id.clone());
                    }
                    candidates.push((
                        id,
                        format!("Breakdown: {category}"),
                        vec![ranking(category, primary, "primary")],
                    ));
                }
            }
            "comparison" | "exceptions" => {
                let archetype = if stage == "comparison" {
                    "comparison"
                } else {
                    "exception-list"
                };
                if let Some(mut visual) =
                    proposal(archetype).and_then(|p| proposal_visual(p, "primary", 0))
                {
                    // Colors and threshold expressions remain proposals until their
                    // compiler is available; never emit speculative formatting.
                    visual
                        .as_object_mut()
                        .expect("visual")
                        .remove("conditionalFormatting");
                    if stage == "exceptions" {
                        for binding in visual["bindings"].as_array_mut().expect("bindings") {
                            binding["role"] = "Values".into();
                        }
                    }
                    candidates.push((
                        stage.clone(),
                        page_display_name(stage).to_string(),
                        vec![visual],
                    ));
                }
            }
            "detail" | "drillthrough-detail" => {
                if stage == "detail" || drill_target.is_some() {
                    let detail_template = grid::template(template)?;
                    let slot = detail_template
                        .slots
                        .iter()
                        .find(|slot| {
                            slot.preferred_families
                                .iter()
                                .any(|family| family == "tableEx")
                        })
                        .ok_or_else(|| {
                            CliError::unexpected("narrative detail template has no table slot")
                        })?;
                    let bindings = context["detailColumns"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|field| json!({"role":"Values", "field":field}))
                        .collect::<Vec<_>>();
                    candidates.push((stage.clone(), if stage == "detail" { "Detail" } else { "Drillthrough detail" }.to_string(), vec![json!({
                        "id":"detail_table", "type":"tableEx", "title":"Detail", "slot":slot.name, "bindings":bindings
                    })]));
                }
            }
            _ => {
                return Err(CliError::unexpected(format!(
                    "unknown narrative stage {stage}"
                )));
            }
        }
        for (id, title, mut visuals) in candidates {
            place_with_rail(template, &flow.rail_template, &mut visuals, rail.as_deref())?;
            let mut page = json!({"id":id, "displayName":title, "template":template, "heading":title, "visuals":visuals});
            if stage == "drillthrough-detail" {
                page["drillthrough"] = json!({"target":drill_target, "hidden":true});
            }
            ordering.push(json!({"page":id, "stage":stage, "template":template, "ruleId":rule.id, "position":pages.len()}));
            pages.push(page);
        }
    }
    let mut spec = json!({"schema":"powerbi-cli.dashboard.v2", "report":legacy["report"], "pages":pages,
        "proof":{"desktop":{"level":"desktop-golden-pending"}}});
    if let Some(model) = legacy.get("model") {
        spec["model"] = model.clone();
    }
    if let Some(field) = &rail {
        spec["layout"] = json!({"rail":{"side":"left", "width":2, "slicers":[{"field":field,"title":field,"mode":"Dropdown"}]}});
    }
    let explanation = json!({
        "kind":"narrative-flow", "ruleId":rule.id, "score":rule.score, "reason":rule.summary,
        "pageOrder":ordering.iter().map(|item| item["page"].clone()).collect::<Vec<_>>(),
        "activePage":"overview", "ordering":ordering,
        "rail":{"field":rail, "replicatedOn":if rail.is_some() { pages.len() } else { 0 }, "placement":"compiled-shared-rail"},
        "drillthrough":drill_target.as_ref().map(|field| json!({"sourcePage":drill_source,"targetPage":"drillthrough-detail","target":field,"distinct":high[0].0,"reason":"highest observed categorical cardinality; source breakdown binds the same column"})),
        "omittedStages":flow.page_order.iter().filter(|stage| !ordering.iter().any(|item| item["stage"] == stage.as_str())).collect::<Vec<_>>()
    });
    Ok((spec, explanation))
}

fn ranking(field: &str, measure: &str, slot: &str) -> Value {
    json!({"id":"breakdown", "type":"barChart", "title":format!("By {field}"), "slot":slot,
        "bindings":[{"role":"Category", "field":field},{"role":"Y", "field":measure}]})
}

fn rail_field(model: &PlanModel<'_>, intent: &Intent) -> CliResult<Option<String>> {
    if let Some(requested) = intent.filter_dimensions.first() {
        let matches = model
            .all_schema_columns()
            .into_iter()
            .filter(|field| {
                field.reference.eq_ignore_ascii_case(requested)
                    || field.name.eq_ignore_ascii_case(requested)
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            if !requested.contains('[') {
                // Free-text dimensions retain the legacy unconsumed-intent
                // warning; only explicit field references are strict inputs.
                return Ok(model
                    .category_columns
                    .first()
                    .or_else(|| model.date_columns.first())
                    .map(|field| field.reference.clone()));
            }
            return Err(spec_missing_input_with_command(
                "/filterDimensions/0",
                "filterDimensions",
                "the rail field must resolve to one schema column",
                json!({"filterDimensions":["Table[Column]"]}),
                "powerbi-cli report spec fields --schema <schema.json> --json",
            ));
        }
        return Ok(Some(matches[0].reference.clone()));
    }
    Ok(model
        .category_columns
        .first()
        .or_else(|| model.date_columns.first())
        .map(|field| field.reference.clone()))
}

/// Reserve the shared left rail using grid geometry. The slicer compiler
/// materializes the rail; other templates have content scaled into its
/// remaining area through explicit-layout precedence.
pub(crate) fn place_with_rail(
    template: &str,
    rail_template: &str,
    visuals: &mut [Value],
    rail: Option<&str>,
) -> CliResult<()> {
    let positions = grid::resolve(&grid::template(template)?, PageSize::STANDARD, None)?;
    let Some(_) = rail else {
        return Ok(());
    };
    let fallback = grid::resolve(&grid::template(rail_template)?, PageSize::STANDARD, None)?;
    if positions
        .get("rail")
        .is_none_or(|rail| rail.x != fallback["rail"].x)
    {
        let content_left = fallback["heading"].x;
        let content_width = fallback["heading"].width;
        for visual in visuals.iter_mut() {
            let slot = visual["slot"].as_str().unwrap_or_default();
            let original = positions.get(slot).ok_or_else(|| {
                CliError::unexpected(format!("narrative template {template} has no slot {slot}"))
            })?;
            let mapped = SlotPosition {
                x: round(content_left + (original.x - 24.0) * content_width / 1232.0),
                width: round(original.width * content_width / 1232.0),
                y: original.y,
                height: original.height,
            };
            visual["layout"] = json!(mapped);
        }
    }
    Ok(())
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_rail_preserves_nonoverlapping_content_and_uses_grid_geometry() {
        let mut visuals = vec![ranking("Dim[Name]", "Fact[Revenue]", "primary")];
        place_with_rail("time-series", "overview", &mut visuals, Some("Dim[Name]")).unwrap();
        let content = &visuals[0]["layout"];
        let rail = grid::resolve(
            &grid::template("overview").unwrap(),
            PageSize::STANDARD,
            None,
        )
        .unwrap()["rail"];
        assert!(content["x"].as_f64().unwrap() >= rail.x + rail.width);
        assert_eq!(visuals.len(), 1, "slicer emission belongs to the compiler");
    }
}
