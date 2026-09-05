//! Read-only dashboard-spec compilation preview.
//!
//! `report spec explain` deliberately stops at the typed operation plan. It
//! validates and compiles the supported portion of a dashboard specification,
//! but never opens a transaction or writes a project/artifact.

use crate::json_composition::normalize_spec_file;
use crate::ops::{
    AddMeasure, AddVisual, Op, OpPlan, ProjectIndex, SetInteraction, measure_handle, page_handle,
    visual_handle,
};
use crate::profile::{load_profile_value, profile_summary, validate_profile_value};
use crate::report_build::compiled_schema_for_explain;
use crate::report_spec_schema::{
    DASHBOARD_V1, DASHBOARD_V2, SpecVersion, UncompiledSection, style_is_supported_typography,
    uncompiled_v2_sections, validate_known_fields,
};
use crate::schema::{load_schema_value, validate_schema_value};
use crate::{CliError, CliResult, EXIT_SUCCESS, canonical_display, command_arg};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct ExplainOptions {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
    spec: Option<PathBuf>,
}

#[derive(Debug)]
struct OpEntry {
    operation: Op,
    pointer: String,
    summary: String,
}

pub(crate) fn explain_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    let schema_path = options.schema.ok_or_else(|| {
        CliError::invalid_args("report spec explain requires --schema <schema.json>")
            .with_suggested_command(
                "powerbi-cli report spec explain --schema <schema.json> --spec <dashboard.json> --json",
            )
    })?;
    let spec_path = options.spec.ok_or_else(|| {
        CliError::invalid_args("report spec explain requires --spec <dashboard.json>")
            .with_suggested_command(
                "powerbi-cli report spec explain --schema <schema.json> --spec <dashboard.json> --json",
            )
    })?;

    let schema = load_schema_value(&schema_path)?;
    let schema_validation = validate_schema_value(&schema);
    if !schema_validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "schema is not valid: {}",
            schema_validation.errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli schema validate {} --json",
            command_arg(&schema_path)
        )));
    }

    let normalized = normalize_spec_file(&spec_path)?;
    let spec = normalized.value;
    let version = validate_known_fields(&spec)?;
    let profile = load_optional_profile(options.profile.as_deref())?;
    let unsupported = collect_unsupported_sections(&spec, version)?;
    let sanitized = sanitize_for_compile(&spec, version);
    let compiled_schema = compiled_schema_for_explain(&schema, &sanitized)?;
    let compiled_validation = validate_schema_value(&compiled_schema);
    if !compiled_validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "compiled dashboard schema is invalid: {}",
            compiled_validation.errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli report spec validate --schema {} --spec {} --json",
            command_arg(&schema_path),
            command_arg(&spec_path)
        )));
    }
    let (entries, index) = compile_operations(&spec, &compiled_schema, &schema)?;
    let plan = build_plan_json(&entries, &index)?;
    let layout = layout_json(&spec, &compiled_schema);
    let defaults = defaults_json(&spec, &compiled_schema);
    let unsupported_json = unsupported
        .iter()
        .map(|item| {
            json!({
                "section": item.section,
                "pointer": item.pointer,
                "owningBead": item.owning_bead,
                "suggestedCommand": item.suggested_command,
                "message": format!("recognized section is not compiled yet; owning bead: {}", item.owning_bead)
            })
        })
        .collect::<Vec<_>>();
    let warnings = unsupported
        .iter()
        .map(|item| {
            json!({
                "code": "report.spec.uncompiled_section",
                "message": format!("{} is previewed without applying; owning bead: {}", item.section, item.owning_bead),
                "pointer": item.pointer,
                "owningBead": item.owning_bead
            })
        })
        .collect::<Vec<_>>();
    let proof_plan = proof_plan(&schema_path, options.profile.as_deref(), &spec_path);
    let next = proof_plan["commands"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    Ok(json!({
        "schema": "powerbi-cli.report.spec.explain.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "schemaPath": canonical_display(&schema_path),
        "profilePath": options.profile.as_ref().map(|path| canonical_display(path)),
        "specPath": canonical_display(&spec_path),
        "normalizedFrom": normalized.normalized_from,
        "specVersion": match version {
            SpecVersion::V1 => DASHBOARD_V1,
            SpecVersion::V2 => DASHBOARD_V2,
        },
        "profileSummary": profile.as_ref().map(profile_summary),
        "plan": plan,
        "handles": handles_json(&entries),
        "layout": layout,
        "defaults": defaults,
        "proofPlan": proof_plan,
        "unsupportedSections": unsupported_json,
        "warnings": warnings,
        "next": next
    }))
}

fn parse_args(args: &[String]) -> CliResult<ExplainOptions> {
    let mut options = ExplainOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--schema" => options.schema = Some(take_value(args, &mut index, "--schema")?),
            "--profile" => options.profile = Some(take_value(args, &mut index, "--profile")?),
            "--spec" => options.spec = Some(take_value(args, &mut index, "--spec")?),
            value if value.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown report spec explain flag: {value}"
                ))
                .with_hint("Run `powerbi-cli report spec explain --schema <schema.json> --spec <dashboard.json> --json`.")
                .with_suggested_command(
                    "powerbi-cli report spec explain --schema <schema.json> --spec <dashboard.json> --json",
                ));
            }
            value => {
                if options.spec.is_some() {
                    return Err(CliError::invalid_args(
                        "report spec explain accepts exactly one spec path",
                    )
                    .with_suggested_command(
                        "powerbi-cli report spec explain --schema <schema.json> --spec <dashboard.json> --json",
                    ));
                }
                options.spec = Some(PathBuf::from(value));
                index += 1;
            }
        }
    }
    Ok(options)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<PathBuf> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value")).with_suggested_command(
            "powerbi-cli report spec explain --schema <schema.json> --spec <dashboard.json> --json",
        )
    })?;
    *index += 2;
    Ok(PathBuf::from(value))
}

fn load_optional_profile(path: Option<&Path>) -> CliResult<Option<Value>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let profile = load_profile_value(path)?;
    let errors = validate_profile_value(&profile);
    if !errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "profile is not valid: {}",
            errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli profile validate {} --json",
            command_arg(path)
        )));
    }
    Ok(Some(profile))
}

fn collect_unsupported_sections(
    spec: &Value,
    version: SpecVersion,
) -> CliResult<Vec<UncompiledSection>> {
    let mut sections = if version == SpecVersion::V2 {
        uncompiled_v2_sections(spec)?
    } else {
        collect_v1_unsupported(spec)
    };
    sections.sort_by(|left, right| left.pointer.cmp(&right.pointer));
    Ok(sections)
}

fn collect_v1_unsupported(spec: &Value) -> Vec<UncompiledSection> {
    const MODEL_BEAD: &str = "pbi-t3-compiler-completeness-1qi.5";
    const STYLE_BEAD: &str = "pbi-t3-compiler-completeness-1qi.6";
    const VISUAL_BEHAVIOR_BEAD: &str = "pbi-t3-compiler-completeness-1qi.4";
    const STYLE_COMMAND: &str = "powerbi-cli report themes apply-preset --project <project-dir> --preset <preset> --dry-run --json";
    const MODEL_COMMAND: &str = "powerbi-cli --json capabilities --for model";
    const VISUAL_COMMAND: &str = "powerbi-cli --json capabilities --for report";
    let Some(root) = spec.as_object() else {
        return Vec::new();
    };
    let mut sections = Vec::new();
    if root.contains_key("style") {
        sections.push(UncompiledSection {
            section: "style".into(),
            pointer: "/style".into(),
            owning_bead: STYLE_BEAD,
            suggested_command: STYLE_COMMAND,
        });
    }
    if root
        .get("model")
        .and_then(Value::as_object)
        .is_some_and(|model| model.contains_key("relationships"))
    {
        sections.push(UncompiledSection {
            section: "model.relationships".into(),
            pointer: "/model/relationships".into(),
            owning_bead: MODEL_BEAD,
            suggested_command: MODEL_COMMAND,
        });
    }
    if let Some(pages) = root.get("pages").and_then(Value::as_array) {
        for (page_index, page) in pages.iter().enumerate() {
            let Some(page) = page.as_object() else {
                continue;
            };
            let page_pointer = format!("/pages/{page_index}");
            if let Some(visuals) = page.get("visuals").and_then(Value::as_array) {
                for (visual_index, visual) in visuals.iter().enumerate() {
                    let Some(visual) = visual.as_object() else {
                        continue;
                    };
                    if visual.contains_key("drilldown") {
                        sections.push(UncompiledSection {
                            section: format!(
                                "pages[{page_index}].visuals[{visual_index}].drilldown"
                            ),
                            pointer: format!("{page_pointer}/visuals/{visual_index}/drilldown"),
                            owning_bead: VISUAL_BEHAVIOR_BEAD,
                            suggested_command: VISUAL_COMMAND,
                        });
                    }
                }
            }
        }
    }
    sections
}

fn sanitize_for_compile(spec: &Value, version: SpecVersion) -> Value {
    let Some(root) = spec.as_object() else {
        return spec.clone();
    };
    let mut sanitized = root.clone();
    match version {
        SpecVersion::V1 => {
            sanitized.remove("style");
            if let Some(model) = sanitized.get_mut("model").and_then(Value::as_object_mut) {
                model.remove("relationships");
            }
            sanitize_pages(&mut sanitized, &[], &["drilldown"]);
        }
        SpecVersion::V2 => {
            for field in ["layout", "filters", "proof"] {
                sanitized.remove(field);
            }
            if sanitized
                .get("style")
                .is_some_and(|style| !style_is_supported_typography(style))
            {
                sanitized.remove("style");
            }
            if let Some(model) = sanitized.get_mut("model").and_then(Value::as_object_mut) {
                for field in [
                    "measurePatterns",
                    "calculatedColumns",
                    "relationships",
                    "staticTables",
                    "dateTable",
                    "sortBy",
                    "formatStrings",
                ] {
                    model.remove(field);
                }
                if let Some(measures) = model.get_mut("measures").and_then(Value::as_array_mut) {
                    for measure in measures {
                        if let Some(measure) = measure.as_object_mut() {
                            measure.remove("expressionFile");
                            measure.remove("formatStringExpression");
                        }
                    }
                }
            }
            sanitize_pages(
                &mut sanitized,
                &["filters", "slicers", "drillthrough", "tooltipFor"],
                &[
                    "sort",
                    "drilldown",
                    "topnGuard",
                    "filters",
                    "subtitle",
                    "format",
                    "conditionalFormatting",
                ],
            );
        }
    }
    Value::Object(sanitized)
}

fn sanitize_pages(root: &mut Map<String, Value>, page_fields: &[&str], visual_fields: &[&str]) {
    let Some(pages) = root.get_mut("pages").and_then(Value::as_array_mut) else {
        return;
    };
    for page in pages {
        let Some(page) = page.as_object_mut() else {
            continue;
        };
        for field in page_fields {
            page.remove(*field);
        }
        if let Some(visuals) = page.get_mut("visuals").and_then(Value::as_array_mut) {
            for visual in visuals {
                let Some(visual) = visual.as_object_mut() else {
                    continue;
                };
                for field in visual_fields {
                    visual.remove(*field);
                }
            }
        }
    }
}

fn compile_operations(
    spec: &Value,
    compiled_schema: &Value,
    source_schema: &Value,
) -> CliResult<(Vec<OpEntry>, ProjectIndex)> {
    let mut entries = Vec::new();
    let mut seeded = BTreeSet::from(["report:main".to_string()]);
    seeded.extend(schema_measure_handles(source_schema));
    if let Some(pages) = compiled_schema["pages"].as_array() {
        seeded.extend(
            pages
                .iter()
                .filter_map(|page| page["name"].as_str().map(page_handle)),
        );
    }

    let mut declared = seeded.clone();
    if let Some(measures) = spec
        .get("model")
        .and_then(Value::as_object)
        .and_then(|model| model.get("measures"))
        .and_then(Value::as_array)
    {
        for (index, value) in measures.iter().enumerate() {
            let Some(measure) = value.as_object() else {
                continue;
            };
            let table = required_string(measure, "table", "dashboard spec model measure")?;
            let name = required_string(measure, "name", "dashboard spec model measure")?;
            let handle = measure_handle(&table, &name);
            if !declared.insert(handle.clone()) {
                continue;
            }
            let Some(expression) = measure.get("expression").and_then(Value::as_str) else {
                continue;
            };
            entries.push(OpEntry {
                operation: Op::AddMeasure(AddMeasure {
                    handle,
                    table,
                    name,
                    expression: expression.to_string(),
                    format_string: optional_string(measure, "formatString")?,
                    format_string_definition: optional_string(measure, "formatStringExpression")?,
                    description: optional_string(measure, "description")?,
                    display_folder: optional_string(measure, "displayFolder")?,
                }),
                pointer: format!("/model/measures/{index}"),
                summary: "declare dashboard-spec measure".to_string(),
            });
        }
    }

    let raw_pages = spec.get("pages").and_then(Value::as_array);
    let compiled_pages = compiled_schema["pages"].as_array();
    if let (Some(raw_pages), Some(compiled_pages)) = (raw_pages, compiled_pages) {
        for (page_index, raw_page) in raw_pages.iter().enumerate() {
            let Some(raw_page) = raw_page.as_object() else {
                continue;
            };
            let Some(compiled_page) = compiled_pages.get(page_index).and_then(Value::as_object)
            else {
                continue;
            };
            let page = compiled_page
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| raw_page.get("id").and_then(Value::as_str))
                .or_else(|| raw_page.get("name").and_then(Value::as_str))
                .unwrap_or("page");
            let page_handle_value = page_handle(page);
            let raw_visuals = raw_page.get("visuals").and_then(Value::as_array);
            let compiled_visuals = compiled_page.get("visuals").and_then(Value::as_array);
            if let Some(compiled_visuals) = compiled_visuals {
                for (visual_index, compiled_visual) in compiled_visuals.iter().enumerate() {
                    let Some(compiled_visual) = compiled_visual.as_object() else {
                        continue;
                    };
                    let raw_visual = raw_visuals
                        .and_then(|visuals| visuals.get(visual_index))
                        .and_then(Value::as_object);
                    let visual_id = raw_visual
                        .and_then(|visual| {
                            visual
                                .get("id")
                                .or_else(|| visual.get("name"))
                                .and_then(Value::as_str)
                        })
                        .or_else(|| compiled_visual.get("name").and_then(Value::as_str))
                        .unwrap_or("visual");
                    let pointer = if raw_visual.is_some() {
                        format!("/pages/{page_index}/visuals/{visual_index}")
                    } else {
                        format!("/pages/{page_index}/generatedVisuals/{visual_index}")
                    };
                    let handle = visual_handle(page, visual_id);
                    let visual_type = compiled_visual
                        .get("visualType")
                        .and_then(Value::as_str)
                        .unwrap_or("card")
                        .to_string();
                    let position = position_value(compiled_visual);
                    let name = compiled_visual
                        .get("name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    entries.push(OpEntry {
                        operation: Op::AddVisual(AddVisual {
                            handle,
                            page: page_handle_value.clone(),
                            visual_type,
                            name,
                            title: compiled_visual
                                .get("title")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned),
                            mode: compiled_visual
                                .get("mode")
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned),
                            single_select: compiled_visual
                                .get("singleSelect")
                                .and_then(Value::as_bool),
                            position,
                            bindings: compiled_visual
                                .get("bindings")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default(),
                        }),
                        pointer,
                        summary: if raw_visual.is_some() {
                            "declare compiled report visual".to_string()
                        } else {
                            "declare compiler-generated layout visual".to_string()
                        },
                    });
                }
            }
            if let Some(interactions) = compiled_page.get("interactions").and_then(Value::as_array)
            {
                for (interaction_index, interaction) in interactions.iter().enumerate() {
                    let Some(interaction) = interaction.as_object() else {
                        continue;
                    };
                    let Some(source) = interaction.get("source").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(target) = interaction.get("target").and_then(Value::as_str) else {
                        continue;
                    };
                    entries.push(OpEntry {
                        operation: Op::SetInteraction(SetInteraction {
                            page: page_handle_value.clone(),
                            source: visual_handle(page, source),
                            target: visual_handle(page, target),
                            interaction_type: interaction
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or("DataFilter")
                                .to_string(),
                        }),
                        pointer: format!("/pages/{page_index}/interactions/{interaction_index}"),
                        summary: "set compiled visual interaction".to_string(),
                    });
                }
            }
        }
    }

    let index = ProjectIndex::new(seeded);
    Ok((entries, index))
}

fn schema_measure_handles(schema: &Value) -> BTreeSet<String> {
    schema["tables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .flat_map(|table| {
            let table_name = table
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            table
                .get("measures")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(move |measure| {
                    measure
                        .get("name")
                        .and_then(Value::as_str)
                        .map(|name| measure_handle(table_name, name))
                })
        })
        .collect()
}

fn build_plan_json(entries: &[OpEntry], index: &ProjectIndex) -> CliResult<Value> {
    let operations = entries
        .iter()
        .map(|entry| entry.operation.clone())
        .collect::<Vec<_>>();
    let plan = OpPlan::new(operations);
    let validated = plan.validate(index).map_err(|error| {
        error
            .as_cli_error()
            .with_hint("Review the ordered operation plan and stable handles before applying it.")
    })?;
    let mut operation_json = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let stage = validated
            .ops
            .get(index)
            .map(|operation| operation.stage)
            .unwrap_or_else(|| entry.operation.stage());
        let serialized = serde_json::to_value(&entry.operation)
            .map_err(|error| CliError::unexpected(format!("serialize operation plan: {error}")))?;
        operation_json.push(json!({
            "index": index,
            "stage": stage.number(),
            "stageName": stage.name(),
            "op": entry.operation.tag(),
            "handle": entry.operation.declared_handle(),
            "summary": entry.summary,
            "pointer": entry.pointer,
            "operation": serialized
        }));
    }
    let stages = validated
        .stages
        .iter()
        .map(|stage| {
            let ops = stage
                .operations
                .iter()
                .filter_map(|index| operation_json.get(*index).cloned())
                .collect::<Vec<_>>();
            json!({
                "index": stage.stage,
                "name": stage.name,
                "ops": ops,
                "operationIndexes": stage.operations
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": crate::ops::OPS_SCHEMA,
        "ops": operation_json,
        "stages": stages,
        "operationCount": entries.len()
    }))
}

fn handles_json(entries: &[OpEntry]) -> Value {
    let declared = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let handle = entry.operation.declared_handle()?;
            Some(json!({
                "handle": handle,
                "kind": declared_kind(entry.operation.tag()),
                "pointer": entry.pointer,
                "opIndex": index
            }))
        })
        .collect::<Vec<_>>();
    let references = entries
        .iter()
        .enumerate()
        .flat_map(|(index, entry)| {
            entry
                .operation
                .references()
                .into_iter()
                .map(move |reference| {
                    json!({
                        "field": reference.field,
                        "handle": reference.handle,
                        "opIndex": index,
                        "pointer": format!("{}/{}", entry.pointer, reference.field)
                    })
                })
        })
        .collect::<Vec<_>>();
    json!({"declared": declared, "references": references})
}

fn declared_kind(tag: &str) -> &'static str {
    match tag {
        "addMeasure" => "measure",
        "addRelationship" => "relationship",
        "addVisual" => "visual",
        "addFilter" => "filter",
        _ => "operation",
    }
}

fn layout_json(spec: &Value, compiled_schema: &Value) -> Value {
    let mut pages = Vec::new();
    let raw_pages = spec.get("pages").and_then(Value::as_array);
    let compiled_pages = compiled_schema["pages"].as_array();
    if let (Some(raw_pages), Some(compiled_pages)) = (raw_pages, compiled_pages) {
        for (page_index, raw_page) in raw_pages.iter().enumerate() {
            let Some(raw_page) = raw_page.as_object() else {
                continue;
            };
            let Some(compiled_page) = compiled_pages.get(page_index).and_then(Value::as_object)
            else {
                continue;
            };
            let page = compiled_page
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| raw_page.get("id").and_then(Value::as_str))
                .unwrap_or("page");
            let slots = raw_page
                .get("visuals")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(visual_index, raw_visual)| {
                    let raw_visual = raw_visual.as_object()?;
                    let compiled_visual = compiled_page
                        .get("visuals")
                        .and_then(Value::as_array)?
                        .get(visual_index)?;
                    let visual_id = raw_visual
                        .get("id")
                        .or_else(|| raw_visual.get("name"))
                        .and_then(Value::as_str)
                        .or_else(|| compiled_visual.get("name").and_then(Value::as_str))
                        .unwrap_or("visual");
                    let position = position_value(compiled_visual.as_object()?)?;
                    let mut assignment = json!({
                        "visual": visual_handle(page, visual_id),
                        "pointer": format!("/pages/{page_index}/visuals/{visual_index}"),
                        "coordinates": position,
                        "x": position["x"],
                        "y": position["y"],
                        "width": position["width"],
                        "height": position["height"]
                    });
                    if let Some(slot) = raw_visual.get("slot") {
                        assignment["slot"] = slot.clone();
                    }
                    Some(assignment)
                })
                .collect::<Vec<_>>();
            let mut page_layout = json!({
                "page": page_handle(page),
                "pointer": format!("/pages/{page_index}"),
                "slots": slots
            });
            if let Some(layout) = compiled_page.get("layout").and_then(Value::as_object) {
                if let Some(template) = layout.get("template") {
                    page_layout["template"] = template.clone();
                }
                if let Some(resolved_slots) = layout.get("slots") {
                    page_layout["resolvedSlots"] = resolved_slots.clone();
                }
                if let Some(compiled_visuals) =
                    compiled_page.get("visuals").and_then(Value::as_array)
                {
                    let raw_len = raw_page
                        .get("visuals")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len);
                    let headings = compiled_visuals
                        .iter()
                        .enumerate()
                        .skip(raw_len)
                        .filter_map(|(visual_index, visual)| {
                            let visual = visual.as_object()?;
                            let position = position_value(visual)?;
                            Some(json!({
                                "kind": visual.get("generatedKind"),
                                "visual": visual.get("name").and_then(Value::as_str).map(|name| visual_handle(page, name)),
                                "text": visual.get("text"),
                                "pointer": format!("/pages/{page_index}/generatedVisuals/{}", visual_index.saturating_sub(raw_len)),
                                "coordinates": position
                            }))
                        })
                        .collect::<Vec<_>>();
                    page_layout["headings"] = Value::Array(headings);
                }
            }
            pages.push(page_layout);
        }
    }
    json!({"available": true, "unavailable": Value::Null, "pages": pages})
}

fn defaults_json(spec: &Value, compiled_schema: &Value) -> Value {
    let mut per_visual = Vec::new();
    let raw_pages = spec.get("pages").and_then(Value::as_array);
    let compiled_pages = compiled_schema["pages"].as_array();
    if let (Some(raw_pages), Some(compiled_pages)) = (raw_pages, compiled_pages) {
        for (page_index, raw_page) in raw_pages.iter().enumerate() {
            let Some(raw_page) = raw_page.as_object() else {
                continue;
            };
            let Some(compiled_page) = compiled_pages.get(page_index).and_then(Value::as_object)
            else {
                continue;
            };
            let page = compiled_page
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| raw_page.get("id").and_then(Value::as_str))
                .unwrap_or("page");
            let Some(raw_visuals) = raw_page.get("visuals").and_then(Value::as_array) else {
                continue;
            };
            let Some(compiled_visuals) = compiled_page.get("visuals").and_then(Value::as_array)
            else {
                continue;
            };
            for (visual_index, raw_visual) in raw_visuals.iter().enumerate() {
                let Some(raw_visual) = raw_visual.as_object() else {
                    continue;
                };
                let Some(compiled_visual) = compiled_visuals.get(visual_index) else {
                    continue;
                };
                let visual_id = raw_visual
                    .get("id")
                    .or_else(|| raw_visual.get("name"))
                    .and_then(Value::as_str)
                    .or_else(|| compiled_visual.get("name").and_then(Value::as_str))
                    .unwrap_or("visual");
                let defaulted_fields = ["x", "y", "width", "height"]
                    .into_iter()
                    .filter(|field| {
                        raw_visual.get(*field).is_none()
                            && raw_visual
                                .get("layout")
                                .and_then(Value::as_object)
                                .and_then(|layout| layout.get(*field))
                                .is_none()
                    })
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                per_visual.push(json!({
                    "handle": visual_handle(page, visual_id),
                    "pointer": format!("/pages/{page_index}/visuals/{visual_index}"),
                    "visualType": compiled_visual.get("visualType"),
                    "defaulted": !defaulted_fields.is_empty(),
                    "defaultedFields": defaulted_fields,
                    "position": position_value(compiled_visual.as_object().unwrap_or(&Map::new()))
                }));
            }
            for (visual_index, compiled_visual) in
                compiled_visuals.iter().enumerate().skip(raw_visuals.len())
            {
                let Some(compiled_visual) = compiled_visual.as_object() else {
                    continue;
                };
                let Some(name) = compiled_visual.get("name").and_then(Value::as_str) else {
                    continue;
                };
                per_visual.push(json!({
                    "handle": visual_handle(page, name),
                    "pointer": format!("/pages/{page_index}/generatedVisuals/{}", visual_index.saturating_sub(raw_visuals.len())),
                    "visualType": compiled_visual.get("visualType"),
                    "generated": true,
                    "defaulted": true,
                    "defaultedFields": ["text", "textStyle", "x", "y", "width", "height"],
                    "position": position_value(compiled_visual)
                }));
            }
        }
    }
    json!({"perVisual": per_visual})
}

fn position_value(visual: &Map<String, Value>) -> Option<Value> {
    let mut position = Map::new();
    for field in ["x", "y", "width", "height"] {
        if let Some(value) = visual.get(field) {
            position.insert(field.to_string(), value.clone());
        }
    }
    (!position.is_empty()).then_some(Value::Object(position))
}

fn proof_plan(schema: &Path, profile: Option<&Path>, spec: &Path) -> Value {
    let profile_arg = profile
        .map(|path| format!(" --profile {}", command_arg(path)))
        .unwrap_or_default();
    let build = format!(
        "powerbi-cli report build --schema {}{} --spec {} --out-dir <project-dir> --json",
        command_arg(schema),
        profile_arg,
        command_arg(spec)
    );
    let commands = vec![
        build,
        "powerbi-cli validate --strict <project-dir> --json".to_string(),
        "powerbi-cli handoff check <project-dir> --json".to_string(),
        "powerbi-cli desktop open-check <project-dir> --json".to_string(),
    ];
    json!({
        "commands": commands,
        "unavailable": [
            {
                "level": "desktop-canvas-refresh",
                "reason": "Power BI Desktop is the compatibility oracle; explain is a unit-smoke preview and does not launch Desktop."
            }
        ]
    })
}

fn required_string(object: &Map<String, Value>, field: &str, owner: &str) -> CliResult<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::invalid_args(format!("{owner} requires {field}")))
}

fn optional_string(object: &Map<String, Value>, field: &str) -> CliResult<Option<String>> {
    match object.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| {
                CliError::invalid_args(format!("dashboard spec field {field} must be a string"))
            }),
    }
}
