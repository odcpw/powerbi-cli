use crate::pbir::{
    VisualSelector, find_visual, load_report_snapshot, visual_detail, visual_list_item,
    visuals_for_page,
};
use crate::report_conditional_formatting::conditional_formatting_command;
use crate::report_visual_formatting_bundle::{apply_formatting, extract_formatting};
use crate::report_visual_formatting_color::set_color_formatting;
use crate::report_visual_formatting_text::set_text_formatting;
use crate::safety_scan::count_literals;
use crate::{
    CliError, CliResult, canonical_display, command_arg, read_json_value, resolve_project,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
    page: Option<String>,
    include_raw: bool,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    include_raw: bool,
}

pub(crate) fn formatting_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report visuals formatting requires a subcommand: list, show, conditional-formatting, extract, apply, set-text, or set-color",
        )
        .with_hint(
            "Start with the read-only inventory before extracting or applying style bundles.",
        )
        .with_suggested_command(
            "powerbi-cli report visuals formatting list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" => list_formatting(rest),
        "show" | "get" => show_formatting(rest),
        "extract" | "export" | "clone" => extract_formatting(rest),
        "apply" | "import" => apply_formatting(rest),
        "conditional-formatting" | "conditional" | "cf" => conditional_formatting_command(rest),
        "set-text" | "text" | "title" | "set-title" => set_text_formatting(rest),
        "set-color" | "set-colour" | "color" | "colour" => set_color_formatting(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown report visuals formatting command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals formatting\"` for supported visual formatting commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report visuals formatting\"")),
    }
}

fn list_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "report visuals formatting list")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visuals = visuals_for_page(&snapshot.pages, options.page.as_deref())?;
    let rows = visuals
        .into_iter()
        .map(|visual| {
            let mut row = visual_list_item(visual);
            row["formatting"] = formatting_summary(visual.path.as_ref(), options.include_raw)?;
            Ok(row)
        })
        .collect::<CliResult<Vec<_>>>()?;
    let totals = formatting_totals(&rows);

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.formatting.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "filter": {
            "page": options.page
        },
        "rawIncluded": options.include_raw,
        "counts": {
            "visuals": rows.len(),
            "visualsWithFormatting": totals.visuals_with_formatting,
            "formatObjectContainers": totals.containers,
            "formatCards": totals.cards,
            "formatProperties": totals.properties,
            "unsupportedContainers": totals.unsupported_containers,
            "literalValues": totals.literal_values
        },
        "visuals": rows,
        "next": [
            format!("powerbi-cli report visuals formatting show --project {} --handle <visual-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report themes show --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

fn show_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "report visuals formatting show")?;
    require_visual_selector(&options.selector, "report visuals formatting show")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals formatting show",
    )?;
    let formatting = formatting_summary(visual.path.as_ref(), options.include_raw)?;

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.formatting.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "rawIncluded": options.include_raw,
        "visual": visual_detail(visual),
        "formatting": formatting,
        "next": [
            format!("powerbi-cli report visuals formatting list --project {} --page {} --json", command_arg(&resolved.project_dir), shell_arg(&visual.page_handle)),
            format!("powerbi-cli report visuals show --project {} --handle {} --json", command_arg(&resolved.project_dir), shell_arg(&visual.handle)),
            format!("powerbi-cli report themes show --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

#[derive(Debug, Default)]
struct FormattingTotals {
    visuals_with_formatting: usize,
    containers: u64,
    cards: u64,
    properties: u64,
    unsupported_containers: u64,
    literal_values: u64,
}

fn formatting_totals(rows: &[Value]) -> FormattingTotals {
    let mut totals = FormattingTotals::default();
    for row in rows {
        let formatting = &row["formatting"];
        let containers = formatting["formatObjectContainerCount"]
            .as_u64()
            .unwrap_or_default();
        if containers > 0 {
            totals.visuals_with_formatting += 1;
        }
        totals.containers += containers;
        totals.cards += formatting["formatCardCount"].as_u64().unwrap_or_default();
        totals.properties += formatting["formatPropertyCount"]
            .as_u64()
            .unwrap_or_default();
        totals.unsupported_containers += formatting["unsupportedContainerCount"]
            .as_u64()
            .unwrap_or_default();
        totals.literal_values += formatting["literalValueCount"].as_u64().unwrap_or_default();
    }
    totals
}

pub(crate) fn formatting_summary(
    visual_path: Option<&PathBuf>,
    include_raw: bool,
) -> CliResult<Value> {
    let Some(visual_path) = visual_path else {
        return Ok(empty_formatting_summary(
            include_raw,
            Some("visual has no visual.json path"),
        ));
    };
    let visual_json = read_json_value(visual_path)?;
    Ok(formatting_summary_from_visual_json(
        &visual_json,
        include_raw,
    ))
}

pub(crate) fn formatting_summary_from_visual_json(visual_json: &Value, include_raw: bool) -> Value {
    let mut containers = Vec::new();
    append_object_summaries(
        &mut containers,
        "visual.visualContainerObjects",
        visual_json.pointer("/visual/visualContainerObjects"),
        include_raw,
    );
    append_object_summaries(
        &mut containers,
        "visual.objects",
        visual_json.pointer("/visual/objects"),
        include_raw,
    );
    append_object_summaries(
        &mut containers,
        "objects",
        visual_json.get("objects"),
        include_raw,
    );
    let object_names = containers
        .iter()
        .filter_map(|container| container["objectName"].as_str())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let sources = containers
        .iter()
        .filter_map(|container| container["source"].as_str())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let format_card_count = containers
        .iter()
        .map(|container| container["cardCount"].as_u64().unwrap_or_default())
        .sum::<u64>();
    let format_property_count = containers
        .iter()
        .map(|container| container["propertyCount"].as_u64().unwrap_or_default())
        .sum::<u64>();
    let unsupported_container_count = containers
        .iter()
        .filter(|container| container["unsupportedShape"].as_bool().unwrap_or_default())
        .count();
    let literal_value_count = containers
        .iter()
        .map(|container| container["literalValueCount"].as_u64().unwrap_or_default())
        .sum::<u64>();

    json!({
        "rawIncluded": include_raw,
        "formatObjectContainerCount": containers.len(),
        "formatCardCount": format_card_count,
        "formatPropertyCount": format_property_count,
        "unsupportedContainerCount": unsupported_container_count,
        "literalValueCount": literal_value_count,
        "sources": sources,
        "objectNames": object_names,
        "containers": containers,
        "safety": {
            "rawIncluded": include_raw,
            "dataValueRisk": "low",
            "mayContainLiteralTextOrColors": literal_value_count > 0,
            "note": "Raw PBIR shared visual-container and visual-specific formatting objects can contain literal text, colors, and display strings; raw payloads are omitted unless --include-raw is passed."
        }
    })
}

fn empty_formatting_summary(include_raw: bool, note: Option<&str>) -> Value {
    json!({
        "rawIncluded": include_raw,
        "formatObjectContainerCount": 0,
        "formatCardCount": 0,
        "formatPropertyCount": 0,
        "unsupportedContainerCount": 0,
        "literalValueCount": 0,
        "sources": [],
        "objectNames": [],
        "containers": [],
        "safety": {
            "rawIncluded": include_raw,
            "dataValueRisk": "low",
            "mayContainLiteralTextOrColors": false,
            "note": note.unwrap_or("No PBIR formatting objects were found.")
        }
    })
}

fn append_object_summaries(
    output: &mut Vec<Value>,
    source: &str,
    objects: Option<&Value>,
    include_raw: bool,
) {
    let Some(Value::Object(map)) = objects else {
        return;
    };
    let mut names = map.keys().collect::<Vec<_>>();
    names.sort();
    for object_name in names {
        if let Some(value) = map.get(object_name) {
            output.push(object_summary(source, object_name, value, include_raw));
        }
    }
}

fn object_summary(source: &str, object_name: &str, value: &Value, include_raw: bool) -> Value {
    let (cards, unsupported_shape) = object_cards(value);
    let mut property_names = BTreeSet::new();
    let mut card_summaries = Vec::new();
    let mut selector_count = 0;
    for (ordinal, card) in cards.iter().enumerate() {
        let properties = card
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| {
                let mut keys = properties.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys
            })
            .unwrap_or_default();
        for property in &properties {
            property_names.insert(property.clone());
        }
        let has_selector = card.get("selector").is_some();
        if has_selector {
            selector_count += 1;
        }
        card_summaries.push(json!({
            "ordinal": ordinal,
            "shape": value_kind(card),
            "propertyCount": properties.len(),
            "propertyNames": properties,
            "hasSelector": has_selector
        }));
    }
    let property_names = property_names.into_iter().collect::<Vec<_>>();
    let literal_value_count = count_literals(value);
    let mut summary = json!({
        "source": source,
        "objectName": object_name,
        "shape": value_kind(value),
        "unsupportedShape": unsupported_shape,
        "cardCount": cards.len(),
        "propertyCount": property_names.len(),
        "selectorCount": selector_count,
        "literalValueCount": literal_value_count,
        "propertyNames": property_names,
        "cards": card_summaries
    });
    if include_raw {
        summary["raw"] = value.clone();
    }
    summary
}

fn object_cards(value: &Value) -> (Vec<&Value>, bool) {
    match value {
        Value::Array(items) => (items.iter().collect(), false),
        Value::Object(map) if map.contains_key("properties") => (vec![value], false),
        _ => (Vec::new(), true),
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn parse_list_args(args: &[String]) -> CliResult<ListOptions> {
    let mut options = ListOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--include-raw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals formatting list flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report visuals formatting list --project <project-dir-or.pbip> --json`.")
                .with_suggested_command(
                    "powerbi-cli report visuals formatting list --project <project-dir-or.pbip> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_show_args(args: &[String]) -> CliResult<ShowOptions> {
    let mut options = ShowOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" => options.selector.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" => {
                let value = take_value(args, &mut i, "--visual")?;
                if value.starts_with("visual:") {
                    options.selector.handle = Some(value);
                } else {
                    options.selector.visual = Some(value);
                }
            }
            "--include-raw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals formatting show flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --json`.")
                .with_suggested_command(
                    "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn require_visual_selector(selector: &VisualSelector, command: &str) -> CliResult<()> {
    if selector.handle.is_some() || (selector.page.is_some() && selector.visual.is_some()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{command} requires --handle or --page plus --visual"
    ))
    .with_hint("Use `report visuals list` to get stable visual handles.")
    .with_suggested_command(format!(
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --json"
    )))
}

fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass the PBIP project directory or the .pbip file explicitly with `--project`.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --json"
        ))
    })
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli --json capabilities --for report` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for report")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
