//! Dashboard template resolution, named slots, and generated heading textboxes.

use super::*;

pub(super) fn resolve_page_layout(
    page_index: usize,
    page: &Map<String, Value>,
    defaults_applied: &mut Vec<Value>,
) -> CliResult<Option<ResolvedPageLayout>> {
    let heading = page_text(page, "heading", page_index)?;
    let subtitle = page_text(page, "subtitle", page_index)?;
    let explicit_template = match page.get("template") {
        None => None,
        Some(value) => {
            let name = value.as_str().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].template must be a non-empty string"
                ))
                .with_pointer(format!("/pages/{page_index}/template"))
            })?;
            if name.trim().is_empty() {
                return Err(CliError::invalid_args(format!(
                    "pages[{page_index}].template must be a non-empty string"
                ))
                .with_pointer(format!("/pages/{page_index}/template")));
            }
            Some(name)
        }
    };
    let needs_layout = explicit_template.is_some() || heading.is_some() || subtitle.is_some();
    if !needs_layout {
        return Ok(None);
    }
    let template_name = explicit_template.unwrap_or("overview");
    if explicit_template.is_none() {
        defaults_applied.push(json!({
            "pointer": format!("/pages/{page_index}/template"),
            "field": "pages[].template",
            "value": template_name,
            "reason": "heading and subtitle visuals require a heading band; the deterministic overview template supplies one"
        }));
    }
    let template = resolve_template_for_page(page_index, template_name)?;
    let page_size = page_size_for_spec(page_index, page)?;
    let positions = grid::resolve(&template, page_size, None).map_err(|error| {
        let mut error = error;
        error.prepend_pointer(&format!("/pages/{page_index}"));
        error
    })?;
    Ok(Some(ResolvedPageLayout {
        template,
        positions,
    }))
}

fn page_text(
    page: &Map<String, Value>,
    field: &str,
    page_index: usize,
) -> CliResult<Option<String>> {
    let Some(value) = page.get(field) else {
        return Ok(None);
    };
    let text = value.as_str().ok_or_else(|| {
        CliError::invalid_args(format!("pages[{page_index}].{field} must be a string"))
            .with_pointer(format!("/pages/{page_index}/{field}"))
    })?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(text.to_string()))
}

fn page_size_for_spec(page_index: usize, page: &Map<String, Value>) -> CliResult<PageSize> {
    let size = page.get("size").and_then(Value::as_object);
    let value = |field: &str, default: f64| -> CliResult<f64> {
        let Some(raw) = size.and_then(|size| size.get(field)) else {
            return Ok(default);
        };
        raw.as_f64().ok_or_else(|| {
            CliError::invalid_args(format!(
                "pages[{page_index}].size.{field} must be a finite positive number"
            ))
            .with_pointer(format!("/pages/{page_index}/size/{field}"))
        })
    };
    Ok(PageSize {
        width: value("width", PageSize::STANDARD.width)?,
        height: value("height", PageSize::STANDARD.height)?,
    })
}

fn resolve_template_for_page(page_index: usize, name: &str) -> CliResult<Template> {
    match grid::template(name) {
        Ok(template) => Ok(template),
        Err(_) => {
            let names = grid::catalog()?
                .templates
                .into_iter()
                .map(|template| template.name)
                .collect::<Vec<_>>();
            Err(CliError::invalid_args(format!(
                "pages[{page_index}] references unknown layout template `{name}`"
            ))
            .with_pointer(format!("/pages/{page_index}/template"))
            .with_hint(format!(
                "Choose one of the named templates: {}.",
                names.join(", ")
            ))
            .with_suggested_command(
                "powerbi-cli report layout auto --project <project-dir-or.pbip> --template overview --dry-run --json",
            ))
        }
    }
}

pub(super) fn layout_metadata(layout: &ResolvedPageLayout) -> Value {
    let slots = layout
        .template
        .slots
        .iter()
        .map(|slot| {
            json!({
                "name": slot.name,
                "col": slot.col,
                "row": slot.row,
                "colSpan": slot.col_span,
                "rowSpan": slot.row_span,
                "preferredFamilies": slot.preferred_families,
                "minFamily": slot.min_family,
                "position": layout.positions.get(&slot.name).map(slot_position_json).unwrap_or(Value::Null)
            })
        })
        .collect::<Vec<_>>();
    json!({
        "template": {
            "name": layout.template.name,
            "rail": layout.template.rail.map(|rail| rail.as_str()),
            "headingBand": layout.template.heading_band,
            "budget": layout.template.budget
        },
        "slots": slots,
        "sectionDividers": {
            "status": "feature_pending",
            "warningCode": "feature_pending"
        }
    })
}

fn slot_position_json(slot: &SlotPosition) -> Value {
    json!({
        "x": slot.x,
        "y": slot.y,
        "width": slot.width,
        "height": slot.height
    })
}

pub(super) fn append_heading_visuals(
    page_index: usize,
    page: &Map<String, Value>,
    layout: Option<&ResolvedPageLayout>,
    style: Option<&Value>,
    visuals: &mut Vec<Value>,
) -> CliResult<()> {
    let heading = page_text(page, "heading", page_index)?;
    let subtitle = page_text(page, "subtitle", page_index)?;
    if heading.is_none() && subtitle.is_none() {
        return Ok(());
    }
    let layout = layout.ok_or_else(|| {
        CliError::unexpected(format!(
            "pages[{page_index}] heading/subtitle layout was not resolved"
        ))
    })?;
    let heading_slot = layout.positions.get("heading").ok_or_else(|| {
        CliError::unexpected(format!(
            "layout template {} does not define a heading slot",
            layout.template.name
        ))
    })?;
    let page_id = page
        .get("id")
        .or_else(|| page.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("page");
    let mut existing_names = visuals
        .iter()
        .filter_map(|visual| visual.get("name").and_then(Value::as_str))
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
    let items = [("heading", heading, true), ("subtitle", subtitle, false)]
        .into_iter()
        .filter_map(|(kind, text, bold)| text.map(|text| (kind, text, bold)))
        .collect::<Vec<_>>();
    let count = items.len();
    let base_height = if count > 1 {
        round_layout(heading_slot.height / count as f64)
    } else {
        heading_slot.height
    };
    let (family, scale) = typography_tokens(style)?;
    for (index, (kind, text, bold)) in items.into_iter().enumerate() {
        let y = if count > 1 {
            round_layout(heading_slot.y + base_height * index as f64)
        } else {
            heading_slot.y
        };
        let height = if count > 1 && index + 1 == count {
            round_layout(heading_slot.y + heading_slot.height - y)
        } else {
            base_height
        };
        let mut name = visual_name(&format!("__layout_{kind}_{page_id}"));
        let mut suffix = 2;
        while !existing_names.insert(name.to_ascii_lowercase()) {
            name = visual_name(&format!("__layout_{kind}_{page_id}_{suffix}"));
            suffix += 1;
        }
        let size = round_layout(if bold { 20.0 } else { 12.0 } * scale);
        if !size.is_finite() || size <= 0.0 {
            return Err(CliError::invalid_args(
                "style.tokens.typography.scale must produce a finite positive font size",
            )
            .with_pointer("/style/tokens/typography/scale"));
        }
        visuals.push(json!({
            "name": name,
            "visualType": "textbox",
            "title": text,
            "text": text,
            "x": heading_slot.x,
            "y": y,
            "width": heading_slot.width,
            "height": height,
            "bindings": [],
            "textStyle": {
                "fontFamily": family,
                "fontSize": size,
                "fontWeight": if bold { "bold" } else { "normal" }
            },
            "generatedSlot": "heading",
            "generatedKind": kind
        }));
    }
    Ok(())
}

fn typography_tokens(style: Option<&Value>) -> CliResult<(String, f64)> {
    let typography = style
        .and_then(Value::as_object)
        .and_then(|style| style.get("tokens"))
        .and_then(Value::as_object)
        .and_then(|tokens| tokens.get("typography"));
    let Some(typography) = typography else {
        return Ok(("Segoe UI".to_string(), 1.0));
    };
    let typography = typography.as_object().ok_or_else(|| {
        CliError::invalid_args("style.tokens.typography must be an object")
            .with_pointer("/style/tokens/typography")
    })?;
    let family = match typography.get("family") {
        None => "Segoe UI".to_string(),
        Some(value) => {
            let family = value.as_str().ok_or_else(|| {
                CliError::invalid_args("style.tokens.typography.family must be a string")
                    .with_pointer("/style/tokens/typography/family")
            })?;
            if family.trim().is_empty() {
                return Err(CliError::invalid_args(
                    "style.tokens.typography.family must not be empty",
                )
                .with_pointer("/style/tokens/typography/family"));
            }
            family.trim().to_string()
        }
    };
    let scale = match typography.get("scale") {
        None => 1.0,
        Some(value) => {
            let scale = value.as_f64().ok_or_else(|| {
                CliError::invalid_args("style.tokens.typography.scale must be a number")
                    .with_pointer("/style/tokens/typography/scale")
            })?;
            if !scale.is_finite() || scale <= 0.0 {
                return Err(CliError::invalid_args(
                    "style.tokens.typography.scale must be a finite positive number",
                )
                .with_pointer("/style/tokens/typography/scale"));
            }
            scale
        }
    };
    Ok((family, scale))
}

fn round_layout(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub(super) fn resolve_visual_slot(
    page_index: usize,
    visual_index: usize,
    page: &Map<String, Value>,
    visual: &Map<String, Value>,
    layout: Option<&ResolvedPageLayout>,
    used_slots: &mut BTreeSet<String>,
    warnings: &mut Vec<Value>,
) -> CliResult<Option<SlotPosition>> {
    let Some(slot_value) = visual.get("slot") else {
        return Ok(None);
    };
    let pointer = format!("/pages/{page_index}/visuals/{visual_index}/slot");
    let Some(slot_name) = slot_value.as_str() else {
        return Err(spec_missing_input(
            pointer,
            "visuals[].slot",
            "a visual slot must name one of the page template slots",
            json!({"slot": "primary"}),
        ));
    };
    let slot_name = slot_name.trim();
    if slot_name.is_empty() {
        return Err(spec_missing_input(
            pointer,
            "visuals[].slot",
            "a visual slot must name one of the page template slots",
            json!({"slot": "primary"}),
        ));
    }
    let Some(layout) = layout else {
        return Err(spec_missing_input(
            pointer,
            "visuals[].slot",
            format!(
                "slot `{slot_name}` cannot be resolved without pages[].template; choose a named template first"
            ),
            json!({"template": "overview", "slot": slot_name}),
        ));
    };
    let Some(slot) = layout
        .template
        .slots
        .iter()
        .find(|candidate| candidate.name.eq_ignore_ascii_case(slot_name))
    else {
        let available = layout
            .template
            .slots
            .iter()
            .map(|slot| slot.name.as_str())
            .collect::<Vec<_>>();
        return Err(spec_missing_input(
            pointer,
            "visuals[].slot",
            format!(
                "slot `{slot_name}` is not defined by page template `{}`; available slots: {}",
                layout.template.name,
                available.join(", ")
            ),
            json!({"slot": available.first().copied().unwrap_or("primary")}),
        )
        .with_hint(format!(
            "Use one of the `{}` template slots: {}.",
            layout.template.name,
            available.join(", ")
        )));
    };
    let key = slot.name.to_ascii_lowercase();
    if !used_slots.insert(key) {
        return Err(CliError::invalid_args(format!(
            "pages[{page_index}] uses template slot `{}` more than once",
            slot.name
        ))
        .with_pointer(format!("/pages/{page_index}/visuals/{visual_index}/slot"))
        .with_hint(
            "Assign each template slot to at most one visual, or use explicit layout coordinates.",
        ));
    }
    let visual_type = visual
        .get("type")
        .or_else(|| visual.get("visualType"))
        .and_then(Value::as_str)
        .unwrap_or("card");
    let visual_type = if visual_type.eq_ignore_ascii_case("textbox") {
        "textbox".to_string()
    } else {
        canonical_visual_type(visual_type)?
    };
    if !preferred_family_matches(&visual_type, &slot.preferred_families) {
        let page_id = page
            .get("id")
            .or_else(|| page.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("page");
        let visual_id = visual
            .get("id")
            .or_else(|| visual.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("visual");
        warnings.push(json!({
            "code": crate::rules::DESIGN_SLOT_FAMILY_MISMATCH,
            "pointer": format!("/pages/{page_index}/visuals/{visual_index}/slot"),
            "message": format!("visual {} ({}) is assigned to slot {} preferred for {}", visual_handle(page_id, visual_id), visual_type, slot.name, slot.preferred_families.join(", ")),
            "visual": visual_handle(page_id, visual_id),
            "slot": slot.name,
            "preferredFamilies": slot.preferred_families
        }));
    }
    Ok(layout.positions.get(&slot.name).copied())
}

fn preferred_family_matches(visual_type: &str, preferred: &[String]) -> bool {
    let visual_type = visual_type.to_ascii_lowercase();
    preferred.iter().any(|family| {
        let family = family.to_ascii_lowercase();
        family == visual_type
            || (family == "chart"
                && matches!(
                    visual_type.as_str(),
                    "linechart"
                        | "areachart"
                        | "stackedareachart"
                        | "barchart"
                        | "clusteredbarchart"
                        | "columnchart"
                        | "clusteredcolumnchart"
                        | "combochart"
                        | "lineclusteredcolumncombochart"
                ))
            || (family == "areachart" && visual_type == "stackedareachart")
            || (family == "barchart" && visual_type == "clusteredbarchart")
            || (family == "columnchart" && visual_type == "clusteredcolumnchart")
            || (family == "combochart" && visual_type == "lineclusteredcolumncombochart")
            || (family == "table" && matches!(visual_type.as_str(), "tableex" | "table"))
            || (family == "matrix" && matches!(visual_type.as_str(), "matrix" | "pivottable"))
            || (family == "card" && visual_type == "kpi")
    })
}
