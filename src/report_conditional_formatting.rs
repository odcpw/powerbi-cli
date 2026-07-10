use crate::cli_support::{required_project, take_value};
use crate::pbir::{
    VisualSelector, find_visual, load_report_snapshot, visual_detail, visual_list_item,
    visuals_for_page,
};
use crate::{
    CliError, CliResult, canonical_display, command_arg, read_json_value, resolve_project,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone)]
struct ConditionalSignal {
    pointer: String,
    key: String,
    signal_type: String,
    value_kind: &'static str,
    raw: Value,
}

pub(crate) fn conditional_formatting_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report visuals formatting conditional-formatting requires list or show",
        )
        .with_hint("Conditional formatting authoring remains fixture-gated; use readback first.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> --json",
        ));
    };
    match action.as_str() {
        "list" | "ls" => list_conditional_formatting(rest),
        "show" | "get" => show_conditional_formatting(rest),
        "add" | "create" | "set" | "update" | "delete" | "remove" => Err(
            CliError::unsupported_feature(
                "conditional formatting authoring needs Desktop-authored PBIR fixtures before mutation commands are exposed",
            )
            .with_hint("Use list/show to inventory existing rules and attach fixture examples to the test corpus.")
            .with_suggested_command(
                "powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> --json",
            ),
        ),
        other => Err(CliError::invalid_args(format!(
            "unknown conditional-formatting command: {other}"
        ))
        .with_hint("Run list/show for conditional formatting readback.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> --json",
        )),
    }
}

fn list_conditional_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(
        options.project,
        "report visuals formatting conditional-formatting list",
    )?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visuals = visuals_for_page(&snapshot.pages, options.page.as_deref())?;
    let rows = visuals
        .into_iter()
        .map(|visual| {
            let mut row = visual_list_item(visual);
            let summary =
                visual_conditional_formatting_summary(visual.path.as_deref(), options.include_raw)?;
            row["conditionalFormatting"] = summary;
            Ok(row)
        })
        .collect::<CliResult<Vec<_>>>()?;
    let signal_count = rows
        .iter()
        .map(|row| {
            row["conditionalFormatting"]["signalCount"]
                .as_u64()
                .unwrap_or_default()
        })
        .sum::<u64>();
    let visuals_with_signals = rows
        .iter()
        .filter(|row| {
            row["conditionalFormatting"]["signalCount"]
                .as_u64()
                .unwrap_or_default()
                > 0
        })
        .count();
    let project_arg = command_arg(&resolved.project_dir);

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.conditionalFormatting.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "filter": {
            "page": options.page
        },
        "rawIncluded": options.include_raw,
        "counts": {
            "visuals": rows.len(),
            "visualsWithConditionalFormattingSignals": visuals_with_signals,
            "conditionalFormattingSignals": signal_count
        },
        "visuals": rows,
        "contract": {
            "readOnly": true,
            "authoring": "unsupported-until-desktop-authored-fixtures",
            "detection": "static PBIR JSON key/path scan for conditional/rule/gradient formatting signals"
        },
        "next": [
            format!("powerbi-cli report visuals formatting conditional-formatting show --project {project_arg} --handle <visual-handle> --json"),
            format!("powerbi-cli report visuals formatting list --project {project_arg} --json")
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

fn show_conditional_formatting(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(
        options.project,
        "report visuals formatting conditional-formatting show",
    )?;
    require_visual_selector(
        &options.selector,
        "report visuals formatting conditional-formatting show",
    )?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals formatting conditional-formatting show",
    )?;
    let summary =
        visual_conditional_formatting_summary(visual.path.as_deref(), options.include_raw)?;
    let project_arg = command_arg(&resolved.project_dir);

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.conditionalFormatting.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "rawIncluded": options.include_raw,
        "visual": visual_detail(visual),
        "conditionalFormatting": summary,
        "contract": {
            "readOnly": true,
            "authoring": "unsupported-until-desktop-authored-fixtures"
        },
        "next": [
            format!("powerbi-cli report visuals formatting conditional-formatting list --project {project_arg} --page {} --json", shell_arg(&visual.page_handle)),
            format!("powerbi-cli report visuals formatting show --project {project_arg} --handle {} --include-raw --json", shell_arg(&visual.handle))
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

fn visual_conditional_formatting_summary(
    visual_path: Option<&Path>,
    include_raw: bool,
) -> CliResult<Value> {
    let Some(visual_path) = visual_path else {
        return Ok(empty_summary(
            include_raw,
            Some("visual has no visual.json path"),
        ));
    };
    let visual_json = read_json_value(visual_path)?;
    let mut signals = Vec::new();
    collect_signals(&visual_json, "", "", &mut signals);
    let signal_types = signals
        .iter()
        .map(|signal| signal.signal_type.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let object_names = signals
        .iter()
        .filter_map(|signal| object_name_from_pointer(&signal.pointer))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let signal_json = signals
        .into_iter()
        .map(|signal| signal_json(signal, include_raw))
        .collect::<Vec<_>>();

    Ok(json!({
        "rawIncluded": include_raw,
        "signalCount": signal_json.len(),
        "signalTypes": signal_types,
        "formatObjectNames": object_names,
        "signals": signal_json,
        "safety": {
            "rawIncluded": include_raw,
            "dataValueRisk": "low",
            "note": "Conditional-formatting PBIR fragments can contain literal colors, thresholds, and display strings; raw payloads are omitted unless --include-raw is passed."
        }
    }))
}

fn empty_summary(include_raw: bool, note: Option<&str>) -> Value {
    json!({
        "rawIncluded": include_raw,
        "signalCount": 0,
        "signalTypes": [],
        "formatObjectNames": [],
        "signals": [],
        "safety": {
            "rawIncluded": include_raw,
            "dataValueRisk": "low",
            "note": note.unwrap_or("No conditional formatting signals were found.")
        }
    })
}

fn collect_signals(value: &Value, pointer: &str, key: &str, output: &mut Vec<ConditionalSignal>) {
    if let Some(signal_type) = signal_type(key, pointer, value) {
        output.push(ConditionalSignal {
            pointer: pointer.to_string(),
            key: key.to_string(),
            signal_type,
            value_kind: value_kind(value),
            raw: value.clone(),
        });
    }
    match value {
        Value::Object(map) => {
            for (child_key, child_value) in map {
                collect_signals(
                    child_value,
                    &format!("{}/{}", pointer, escape_pointer_segment(child_key)),
                    child_key,
                    output,
                );
            }
        }
        Value::Array(items) => {
            for (index, child_value) in items.iter().enumerate() {
                collect_signals(child_value, &format!("{pointer}/{index}"), key, output);
            }
        }
        _ => {}
    }
}

fn signal_type(key: &str, pointer: &str, value: &Value) -> Option<String> {
    let lower = key.to_ascii_lowercase();
    if lower.contains("conditional") {
        return Some("conditional".to_string());
    }
    if lower.contains("gradient") {
        return Some("gradient".to_string());
    }
    if (lower == "rule" || lower == "rules" || lower.ends_with("rule") || lower.ends_with("rules"))
        && is_formatting_pointer(pointer)
    {
        return Some("rule".to_string());
    }
    if is_rule_like_object(value) && is_formatting_pointer(pointer) {
        return Some("rule-like-object".to_string());
    }
    None
}

fn is_rule_like_object(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    let keys = map
        .keys()
        .map(|key| key.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let has_condition = keys.iter().any(|key| {
        matches!(
            key.as_str(),
            "condition" | "conditions" | "threshold" | "thresholds" | "min" | "max"
        )
    });
    let has_result = keys.iter().any(|key| {
        matches!(
            key.as_str(),
            "color" | "fill" | "fontcolor" | "background" | "value"
        )
    });
    has_condition && has_result
}

fn is_formatting_pointer(pointer: &str) -> bool {
    pointer.contains("/visual/objects") || pointer.contains("/objects")
}

fn signal_json(signal: ConditionalSignal, include_raw: bool) -> Value {
    let mut value = json!({
        "pointer": signal.pointer,
        "key": signal.key,
        "type": signal.signal_type,
        "valueKind": signal.value_kind
    });
    if include_raw {
        value["raw"] = signal.raw;
    }
    value
}

fn object_name_from_pointer(pointer: &str) -> Option<String> {
    for prefix in ["/visual/objects/", "/objects/"] {
        if let Some(tail) = pointer.strip_prefix(prefix) {
            return tail.split('/').next().map(unescape_pointer_segment);
        }
    }
    None
}

fn escape_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn unescape_pointer_segment(value: &str) -> String {
    value.replace("~1", "/").replace("~0", "~")
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
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown conditional-formatting list flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> --json`.")
                .with_suggested_command(
                    "powerbi-cli report visuals formatting conditional-formatting list --project <project-dir-or.pbip> --json",
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
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown conditional-formatting show flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report visuals formatting conditional-formatting show --project <project-dir-or.pbip> --handle <visual-handle> --json`.")
                .with_suggested_command(
                    "powerbi-cli report visuals formatting conditional-formatting show --project <project-dir-or.pbip> --handle <visual-handle> --json",
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

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
