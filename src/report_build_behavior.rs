//! Pure dashboard visual-behavior lowering to the registered mutation kernels.
use crate::ops::{MutationPayload, Op};
use crate::report_build::{FieldKind, FieldRef, ModelIndex};
use crate::{CliError, CliResult};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub(crate) fn compile(
    spec: &Map<String, Value>,
    compiled: &Value,
    model: &ModelIndex,
) -> CliResult<(Vec<Op>, Vec<String>)> {
    let mut operations = Vec::new();
    let mut pointers = Vec::new();
    for (pi, page) in spec
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (vi, visual) in page
            .get("visuals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if !["sort", "drilldown", "topnGuard"]
                .iter()
                .any(|key| visual.get(*key).is_some())
            {
                continue;
            }
            let generated = &compiled["pages"][pi]["visuals"][vi];
            let page_name = scaffold_page_name(&compiled["pages"][pi], pi);
            let visual_name = scaffold_visual_name(generated, vi);
            let handle = format!("visual:{}:{}", page_name, visual_name);
            let kind = generated["visualType"].as_str().unwrap();
            let base = format!("/pages/{pi}/visuals/{vi}");
            let bindings = generated["bindings"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            // Sorting precedes hierarchy replacement so the latter preserves
            // both the sort definition and its own active-level markers.
            if let Some(sort) = visual.get("sort") {
                let pointer = format!("{base}/sort");
                let field = resolve(
                    model,
                    sort.get("field"),
                    FieldKind::Measure,
                    &format!("{pointer}/field"),
                )?;
                if !matches!(
                    kind,
                    "pieChart" | "donutChart" | "lineClusteredColumnComboChart"
                ) {
                    return Err(sort_refusal(&pointer));
                }
                let direction = sort
                    .get("direction")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        invalid(
                            &format!("{pointer}/direction"),
                            "sort.direction requires Descending",
                        )
                    })?;
                if !matches!(
                    direction.trim().to_ascii_lowercase().as_str(),
                    "desc" | "descending"
                ) {
                    return Err(sort_refusal(&format!("{pointer}/direction")));
                }
                if bindings.iter().any(|b| b.get("sortDirection").is_some()) {
                    return Err(invalid(
                        &pointer,
                        "sort conflicts with an explicit binding sortDirection",
                    ));
                }
                let mut sorted = bindings.clone();
                let binding = sorted
                    .iter_mut()
                    .find(|b| matches_field(b, "measure", &field))
                    .ok_or_else(|| {
                        invalid(
                            &format!("{pointer}/field"),
                            "sort.field must be a projected measure",
                        )
                    })?;
                binding["sortDirection"] = json!("Descending");
                operations.push(Op::SetBindings(payload(json!({"handle": handle, "bindingsJson": serde_json::to_string(&sorted).expect("bindings serialize")}))));
                pointers.push(pointer);
            }
            let mut hierarchy = Vec::new();
            if let Some(drilldown) = visual.get("drilldown") {
                let pointer = format!("{base}/drilldown");
                if !matches!(
                    kind,
                    "lineChart"
                        | "areaChart"
                        | "stackedAreaChart"
                        | "barChart"
                        | "columnChart"
                        | "clusteredBarChart"
                        | "clusteredColumnChart"
                        | "lineClusteredColumnComboChart"
                ) {
                    return Err(CliError::unsupported_feature(format!("drilldown hierarchy requires a line, area, bar, column, or combo chart, not {kind}; scatter Category accepts only one projection"))
                        .with_pointer(&pointer).with_hint("Use a category-axis chart with at least two model columns.")
                        .with_suggested_command("powerbi-cli report visuals catalog --visual-type lineChart --json"));
                }
                let fields = drilldown
                    .get("fields")
                    .and_then(Value::as_array)
                    .filter(|fields| fields.len() >= 2)
                    .ok_or_else(|| {
                        invalid(
                            &format!("{pointer}/fields"),
                            "drilldown.fields requires at least two columns",
                        )
                    })?;
                let mut seen = BTreeSet::new();
                for (i, field) in fields.iter().enumerate() {
                    let fp = format!("{pointer}/fields/{i}");
                    let field = resolve(model, Some(field), FieldKind::Column, &fp)?;
                    let reference = reference(&field);
                    if !seen.insert(reference.to_ascii_lowercase()) {
                        return Err(invalid(&fp, "duplicate drilldown hierarchy field"));
                    }
                    if bindings
                        .iter()
                        .any(|b| b["role"] != "Category" && matches_field(b, "column", &field))
                    {
                        return Err(invalid(
                            &fp,
                            "hierarchy field is already projected in another role",
                        ));
                    }
                    hierarchy.push(reference);
                }
                operations.push(Op::SetDrilldownHierarchy(payload(
                    json!({"handle": handle, "field": hierarchy}),
                )));
                pointers.push(pointer);
            }
            if let Some(guard) = visual.get("topnGuard") {
                let pointer = format!("{base}/topnGuard");
                let measure = resolve(
                    model,
                    guard.get("orderBy"),
                    FieldKind::Measure,
                    &format!("{pointer}/orderBy"),
                )?;
                let top = guard
                    .get("top")
                    .and_then(Value::as_u64)
                    .filter(|top| *top > 0)
                    .ok_or_else(|| {
                        invalid(
                            &format!("{pointer}/top"),
                            "topnGuard.top requires a positive integer",
                        )
                    })?;
                let axis = hierarchy
                    .first()
                    .cloned()
                    .or_else(|| {
                        bindings
                            .iter()
                            .find(|b| b["role"] == "Category" && b.get("column").is_some())
                            .map(|b| {
                                format!(
                                    "{}[{}]",
                                    b["table"].as_str().unwrap(),
                                    b["column"].as_str().unwrap()
                                )
                            })
                    })
                    .ok_or_else(|| {
                        invalid(&pointer, "topnGuard requires a Category column to rank")
                    })?;
                operations.push(Op::SetTopNGuard(payload(json!({"handle": handle, "field": axis, "orderBy": reference(&measure), "top": top}))));
                pointers.push(pointer);
            }
        }
    }
    Ok((operations, pointers))
}

// Explain does not run the scaffold writer, so materialize its default names
// before declaring handles referenced by the lowered behavior operations.
pub(crate) fn materialize_scaffold_names(schema: &mut Value) {
    if let Some(pages) = schema["pages"].as_array_mut() {
        for (pi, page) in pages.iter_mut().enumerate() {
            page["name"] = json!(scaffold_page_name(page, pi));
            if let Some(visuals) = page["visuals"].as_array_mut() {
                for (vi, visual) in visuals.iter_mut().enumerate() {
                    visual["name"] = json!(scaffold_visual_name(visual, vi));
                }
            }
        }
    }
}

fn scaffold_page_name(page: &Value, index: usize) -> String {
    page["name"].as_str().map(str::to_owned).unwrap_or_else(|| {
        crate::scaffold::object_name(
            "ReportSection",
            page["displayName"].as_str().unwrap_or("Page"),
            index,
        )
    })
}

fn scaffold_visual_name(visual: &Value, index: usize) -> String {
    visual["name"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            crate::scaffold::object_name(
                "VisualContainer",
                visual["title"]
                    .as_str()
                    .unwrap_or(&format!("Visual {}", index + 1)),
                index,
            )
        })
}

fn payload(value: Value) -> MutationPayload {
    serde_json::from_value(value).expect("compiler mutation payload is an object")
}

fn matches_field(binding: &Value, kind: &str, field: &FieldRef) -> bool {
    binding["table"]
        .as_str()
        .is_some_and(|table| table.eq_ignore_ascii_case(&field.table))
        && binding[kind]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(&field.name))
}

fn resolve(
    model: &ModelIndex,
    value: Option<&Value>,
    kind: FieldKind,
    pointer: &str,
) -> CliResult<FieldRef> {
    let text = value
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| invalid(pointer, "requires a nonempty Table[Field] reference"))?;
    let field = model.resolve_field(text).map_err(|error| {
        error
            .with_pointer(pointer)
            .with_hint("Use report spec fields to select an existing, unambiguous model field.")
            .with_suggested_command("powerbi-cli report visuals catalog --json")
    })?;
    if field.kind != kind {
        return Err(invalid(
            pointer,
            &format!("expected a model {kind:?} reference"),
        ));
    }
    Ok(field)
}

fn reference(field: &FieldRef) -> String {
    format!("{}[{}]", field.table, field.name)
}

fn invalid(pointer: &str, message: &str) -> CliError {
    CliError::invalid_args(message).with_pointer(pointer)
        .with_hint("Use report spec fields to select model fields and report visuals catalog to inspect supported roles.")
        .with_suggested_command("powerbi-cli report visuals catalog --json")
}

fn sort_refusal(pointer: &str) -> CliError {
    CliError::unsupported_feature("visual sort is fixture-gated by pbi-t4-pbir-catalog-expansion-sn2.1; only a single descending projected measure on combo, pie, or donut is proven")
        .with_pointer(pointer).with_hint("Use Descending on a projected measure in combo/pie/donut; ascending and table/matrix sorts need Desktop evidence.")
        .with_suggested_command("powerbi-cli report visuals catalog --json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_lowering_resolves_case_insensitively_and_preserves_other_binding_metadata() {
        let schema = json!({"tables":[{"name":"Fact","columns":[],"measures":[{"name":"Total"}]}]});
        let model = ModelIndex::from_schema(&schema);
        let bindings = json!([{"role":"Y","table":"Fact","measure":"Total","displayName":"Revenue","formatString":"0.0"}]);
        let compiled = json!({"pages":[{"name":"Page","visuals":[{"name":"Share","visualType":"pieChart","bindings":bindings}]}]});
        let spec =
            json!({"pages":[{"visuals":[{"sort":{"field":"fact[total]","direction":"desc"}}]}]});
        let (ops, pointers) = compile(spec.as_object().unwrap(), &compiled, &model).unwrap();
        assert_eq!(pointers, ["/pages/0/visuals/0/sort"]);
        let Op::SetBindings(payload) = &ops[0] else {
            panic!("expected SetBindings")
        };
        let sorted: Value =
            serde_json::from_str(payload.fields["bindingsJson"].as_str().unwrap()).unwrap();
        let mut expected = bindings;
        expected[0]["sortDirection"] = json!("Descending");
        assert_eq!(sorted, expected);
    }
}
