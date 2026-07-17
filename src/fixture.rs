use crate::inspect::deep_inspect;
use crate::lint::lint_project;
use crate::pbir_filters::{FilterOwner, ReportFilterRecord, list_report_filters};
use crate::pbir_interactions::{
    InteractionVisualRef, ReportInteractionRecord, interaction_semantics, list_report_interactions,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct NormalizeOptions {
    project: Option<PathBuf>,
    out: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct VerifyOptions {
    project: Option<PathBuf>,
    expected: Option<PathBuf>,
    write_actual: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct InteractionFixtureSummary {
    by_page: BTreeMap<String, Vec<Value>>,
    explicit_count: usize,
    unsupported_count: usize,
    stale_reference_count: usize,
}

pub(crate) fn fixture_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args("fixture requires a subcommand: normalize or verify")
                .with_hint("Run `powerbi-cli fixture normalize <project-dir-or.pbip> --json`.")
                .with_suggested_command(
                    "powerbi-cli fixture normalize <project-dir-or.pbip> --json",
                ),
        );
    };

    match action.as_str() {
        "normalize" | "normalise" | "summary" => normalize_command(rest),
        "verify" => verify_command(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown fixture command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for fixture` for supported fixture commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for fixture")),
    }
}

fn normalize_command(args: &[String]) -> CliResult<Value> {
    let options = parse_normalize_args(args)?;
    let project = required_project(options.project, "fixture normalize")?;
    let summary = normalized_project_summary(&project)?;
    let output = with_normalize_verification(summary);
    if let Some(path) = options.out {
        write_json_file(&path, &output)?;
    }
    Ok(output)
}

fn verify_command(args: &[String]) -> CliResult<Value> {
    let options = parse_verify_args(args)?;
    let project = required_project(options.project, "fixture verify")?;
    let expected_path = options.expected.ok_or_else(|| {
        CliError::invalid_args("fixture verify requires --expected <summary.json>")
            .with_hint("Generate an expected summary with `fixture normalize --out <file>`.")
            .with_suggested_command(
                "powerbi-cli fixture verify <project-dir-or.pbip> --expected testdata/golden/sales.summary.json --json",
            )
    })?;
    let summary = normalized_project_summary(&project)?;
    let actual_for_compare = with_normalize_verification(summary.clone());
    let expected = read_json_value(&expected_path)?;
    let differences = diff_json(&expected, &actual_for_compare, "");
    let same = differences.is_empty();
    let actual_written = if same {
        None
    } else if let Some(actual_path) = options.write_actual {
        write_json_file(&actual_path, &actual_for_compare)?;
        Some(actual_path)
    } else {
        None
    };
    let mut output = summary;
    let object = output
        .as_object_mut()
        .ok_or_else(|| CliError::unexpected("fixture summary was not a JSON object"))?;
    object.insert("ok".to_string(), Value::Bool(same));
    object.insert(
        "exitCode".to_string(),
        Value::from(if same {
            EXIT_SUCCESS
        } else {
            EXIT_VALIDATION_FAILED
        }),
    );
    object.insert(
        "verification".to_string(),
        json!({
            "mode": "verify",
            "expected": canonical_display(&expected_path),
            "actualWritten": actual_written.as_ref().map(|path| canonical_display(path)),
            "actual": (!same).then_some(actual_for_compare),
            "same": same,
            "differences": differences
        }),
    );
    Ok(output)
}

fn normalized_project_summary(project: &Path) -> CliResult<Value> {
    let resolved = resolve_project(project)?;
    let validation = validate_project(&resolved)?;
    if !validation.errors.is_empty() {
        return Err(CliError::validation_failed(
            "fixture normalize requires a locally valid PBIP project",
        )
        .with_hint("Run `validate --strict` and fix errors before capturing a golden summary.")
        .with_suggested_command(format!(
            "powerbi-cli validate --strict {} --json",
            command_arg(&resolved.project_dir)
        )));
    }
    let deep = deep_inspect(&resolved, &validation)?;
    let lint = lint_project(&resolved, &validation)?;
    let interactions = report_interaction_summary(&resolved)?;
    let pbir = pbir_summary(&resolved)?;
    let mut summary = json!({
        "schema": "powerbi-cli.fixture.summary.v1",
        "summaryVersion": 2,
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "project": {
            "name": deep["project"]["name"],
            "pbipName": resolved.pbip_path.file_name().and_then(|value| value.to_str()).unwrap_or("project.pbip")
        },
        "counts": summary_counts(&deep, &validation, &lint, &interactions),
        "model": model_summary(&deep["model"]),
        "report": report_summary(&deep["report"], &interactions.by_page)?,
        "pbir": pbir,
        "validation": {
            "warnings": validation.warnings,
            "errors": validation.errors
        },
        "lint": {
            "counts": lint["counts"],
            "findings": lint_findings_summary(&lint)
        },
        "next": [
            "Review this summary before committing it as a golden fixture.",
            "Run `powerbi-cli fixture verify <project> --expected <summary.json> --json` in CI.",
            "Run `powerbi-cli desktop open-check <project> --json` on an opt-in Windows Desktop oracle machine."
        ]
    });
    let canonical = serde_json::to_string(&summary).map_err(|err| {
        CliError::unexpected(format!("serialize fixture summary for fingerprint: {err}"))
    })?;
    summary["fingerprint"] = Value::String(fingerprint_hex(&canonical));
    Ok(summary)
}

fn with_normalize_verification(mut summary: Value) -> Value {
    summary["verification"] = json!({
        "mode": "normalize",
        "expected": Value::Null,
        "actualWritten": Value::Null,
        "same": Value::Null,
        "differences": []
    });
    summary
}

fn summary_counts(
    deep: &Value,
    validation: &crate::ValidationReport,
    lint: &Value,
    interactions: &InteractionFixtureSummary,
) -> Value {
    let columns = deep["model"]["tables"]
        .as_array()
        .map(|tables| {
            tables
                .iter()
                .map(|table| {
                    table["columns"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or_default()
                })
                .sum::<usize>()
        })
        .unwrap_or_default();
    json!({
        "jsonFilesChecked": validation.json_files_checked,
        "tables": validation.tables,
        "columns": columns,
        "measures": validation.measures,
        "relationships": validation.relationships,
        "pages": validation.pages,
        "visuals": validation.visuals,
        "boundVisuals": validation.bound_visuals,
        "validationWarnings": validation.warnings.len(),
        "validationErrors": validation.errors.len(),
        "lintErrors": lint["counts"]["errors"],
        "lintWarnings": lint["counts"]["warnings"],
        "lintInfo": lint["counts"]["info"],
        "explicitInteractions": interactions.explicit_count,
        "unsupportedInteractions": interactions.unsupported_count,
        "staleInteractionVisualReferences": interactions.stale_reference_count
    })
}

fn model_summary(model: &Value) -> Value {
    let tables = model["tables"]
        .as_array()
        .map(|items| items.iter().map(table_summary).collect::<Vec<_>>())
        .unwrap_or_default();
    let relationships = model["relationships"]
        .as_array()
        .map(|items| {
            let mut values = items.iter().map(relationship_summary).collect::<Vec<_>>();
            values.sort_by_key(|value| value_key(value, "name"));
            values
        })
        .unwrap_or_default();
    json!({
        "tables": tables,
        "relationships": relationships
    })
}

fn table_summary(table: &Value) -> Value {
    json!({
        "name": table["name"],
        "columns": table["columns"].as_array().map(|items| items.iter().map(column_summary).collect::<Vec<_>>()).unwrap_or_default(),
        "measures": table["measures"].as_array().map(|items| items.iter().map(measure_summary).collect::<Vec<_>>()).unwrap_or_default(),
        "partitions": table["partitions"].as_array().map(|items| items.iter().map(partition_summary).collect::<Vec<_>>()).unwrap_or_default()
    })
}

fn column_summary(column: &Value) -> Value {
    json!({
        "name": column["name"],
        "dataType": column["properties"]["dataType"],
        "isCalculated": column["isCalculated"],
        "isHidden": column["properties"]["isHidden"],
        "isKey": column["properties"]["isKey"],
        "summarizeBy": column["properties"]["summarizeBy"],
        "formatString": column["properties"]["formatString"]
    })
}

fn measure_summary(measure: &Value) -> Value {
    json!({
        "name": measure["name"],
        "expression": measure["expression"],
        "formatString": measure["properties"]["formatString"],
        "displayFolder": measure["properties"]["displayFolder"]
    })
}

fn partition_summary(partition: &Value) -> Value {
    json!({
        "name": partition["name"],
        "mode": partition["mode"],
        "sourceKind": partition["sourceKind"],
        "safeForHome": partition["offlineSafety"]["safeForHome"]
    })
}

fn relationship_summary(relationship: &Value) -> Value {
    json!({
        "name": relationship["name"],
        "from": format!("{}.{}", string_value(&relationship["fromTable"]), string_value(&relationship["fromColumn"])),
        "to": format!("{}.{}", string_value(&relationship["toTable"]), string_value(&relationship["toColumn"])),
        "crossFilteringBehavior": relationship["properties"]["crossFilteringBehavior"],
        "isActive": relationship["properties"]["isActive"]
    })
}

fn report_summary(
    report: &Value,
    interactions_by_page: &BTreeMap<String, Vec<Value>>,
) -> CliResult<Value> {
    let pages = report["pages"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|page| page_summary(page, interactions_by_page))
                .collect::<CliResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(json!({
        "interactionSemantics": interaction_semantics(),
        "pages": pages
    }))
}

fn page_summary(
    page: &Value,
    interactions_by_page: &BTreeMap<String, Vec<Value>>,
) -> CliResult<Value> {
    let visuals = page["visuals"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(ordinal, visual)| visual_summary(ordinal, visual))
                .collect::<CliResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let interactions = interactions_by_page
        .get(page["handle"].as_str().unwrap_or_default())
        .cloned()
        .unwrap_or_default();
    let mut summary = json!({
        "ordinal": page["ordinal"],
        "name": page["name"],
        "displayName": page["displayName"],
        "width": page["width"],
        "height": page["height"],
        "displayOption": page["displayOption"],
        "isActive": page["isActive"],
        "visuals": visuals,
        "interactionCount": interactions.len(),
        "interactions": interactions
    });
    let drillthrough = drillthrough_page_summary(page);
    if drillthrough["enabled"].as_bool().unwrap_or_default() {
        summary["drillthrough"] = drillthrough;
    }
    Ok(summary)
}

fn drillthrough_page_summary(page: &Value) -> Value {
    let binding = &page["pageBinding"];
    let parameters = binding["parameters"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|parameter| {
                    json!({
                        "name": parameter["name"],
                        "boundFilter": parameter["boundFilter"],
                        "target": field_expr_summary(&parameter["fieldExpr"])
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let enabled = page["type"].as_str() == Some("Drillthrough")
        || binding["type"].as_str() == Some("Drillthrough")
        || !parameters.is_empty();
    json!({
        "enabled": enabled,
        "pageType": page["type"],
        "visibility": page["visibility"],
        "binding": if binding.is_object() {
            json!({
                "name": binding["name"],
                "type": binding["type"],
                "referenceScope": binding["referenceScope"],
                "acceptsFilterContext": binding["acceptsFilterContext"],
                "parameters": parameters
            })
        } else {
            Value::Null
        }
    })
}

fn field_expr_summary(value: &Value) -> Value {
    if let Some(column) = find_column_expr(value) {
        return column;
    }
    json!({
        "kind": "unknown",
        "table": Value::Null,
        "column": Value::Null
    })
}

fn find_column_expr(value: &Value) -> Option<Value> {
    match value {
        Value::Object(object) => {
            if let Some(column) = object.get("Column").and_then(Value::as_object) {
                let table = column["Expression"]["SourceRef"]["Entity"]
                    .as_str()
                    .map(ToOwned::to_owned);
                let field = column["Property"].as_str().map(ToOwned::to_owned);
                return Some(json!({
                    "kind": "column",
                    "table": table,
                    "column": field
                }));
            }
            object.values().find_map(find_column_expr)
        }
        Value::Array(items) => items.iter().find_map(find_column_expr),
        _ => None,
    }
}

fn report_interaction_summary(resolved: &ResolvedProject) -> CliResult<InteractionFixtureSummary> {
    let (records, _) = list_report_interactions(resolved)?;
    let mut summary = InteractionFixtureSummary::default();
    for record in records {
        summary.explicit_count += 1;
        if record.unsupported {
            summary.unsupported_count += 1;
        }
        if stale_visual_reference(&record) {
            summary.stale_reference_count += 1;
        }
        summary
            .by_page
            .entry(record.page_handle.clone())
            .or_default()
            .push(interaction_summary(&record));
    }
    for values in summary.by_page.values_mut() {
        values.sort_by_key(|value| value["ordinal"].as_u64().unwrap_or_default());
    }
    Ok(summary)
}

fn pbir_summary(resolved: &ResolvedProject) -> CliResult<Value> {
    let (mut filters, _) = list_report_filters(resolved)?;
    filters.sort_by_key(filter_sort_key);
    Ok(json!({
        "reportDefinitionVersion": report_definition_version(resolved)?,
        "filters": {
            "counts": filter_counts(&filters),
            "items": filters.iter().map(filter_fixture_summary).collect::<Vec<_>>()
        }
    }))
}

fn report_definition_version(resolved: &ResolvedProject) -> CliResult<Value> {
    let path = resolved.report_dir.join("definition").join("version.json");
    if !path.is_file() {
        return Ok(Value::Null);
    }
    let value = read_json_value(&path)?;
    Ok(value["version"].clone())
}

fn filter_counts(filters: &[ReportFilterRecord]) -> Value {
    let report = filters
        .iter()
        .filter(|filter| filter.scope.as_str() == "report")
        .count();
    let page = filters
        .iter()
        .filter(|filter| filter.scope.as_str() == "page")
        .count();
    let visual = filters
        .iter()
        .filter(|filter| filter.scope.as_str() == "visual")
        .count();
    json!({
        "total": filters.len(),
        "report": report,
        "page": page,
        "visual": visual,
        "unsupported": filters.iter().filter(|filter| filter.unsupported).count(),
        "literals": filters.iter().map(|filter| filter.literal_count).sum::<usize>()
    })
}

fn filter_fixture_summary(filter: &ReportFilterRecord) -> Value {
    json!({
        "scope": filter.scope.as_str(),
        "owner": filter_owner_summary(&filter.owner),
        "ordinal": filter.ordinal,
        "name": filter.name,
        "filterType": filter.filter_type,
        "unsupported": filter.unsupported,
        "target": filter.target,
        "conditionSummary": filter.condition_summary,
        "literalCount": filter.literal_count,
        "desktopSafeName": filter.name.as_ref().is_some_and(|name| !name.trim().is_empty() && name.len() <= 50),
        "categoricalVersion": filter.raw["filter"]["Version"],
        "fromCount": filter.raw["filter"]["From"].as_array().map(Vec::len).unwrap_or_default(),
        "whereCount": filter.raw["filter"]["Where"].as_array().map(Vec::len).unwrap_or_default(),
        "whereUsesSourceAlias": where_uses_source_alias(&filter.raw)
    })
}

fn filter_owner_summary(owner: &FilterOwner) -> Value {
    match owner {
        FilterOwner::Report { .. } => json!({
            "kind": "report",
            "handle": "report:main",
            "name": "report",
            "displayName": "Report"
        }),
        FilterOwner::Page {
            handle,
            name,
            display_name,
            ordinal,
            ..
        } => json!({
            "kind": "page",
            "handle": handle,
            "name": name,
            "displayName": display_name,
            "ordinal": ordinal
        }),
        FilterOwner::Visual {
            handle,
            name,
            title,
            visual_type,
            page_handle,
            page_name,
            page_display_name,
            page_ordinal,
            ..
        } => json!({
            "kind": "visual",
            "handle": handle,
            "name": name,
            "title": title,
            "visualType": visual_type,
            "page": {
                "handle": page_handle,
                "name": page_name,
                "displayName": page_display_name,
                "ordinal": page_ordinal
            }
        }),
    }
}

fn filter_sort_key(filter: &ReportFilterRecord) -> (u8, String, usize, String) {
    (
        match filter.scope.as_str() {
            "report" => 0,
            "page" => 1,
            "visual" => 2,
            _ => 3,
        },
        filter_owner_key(&filter.owner),
        filter.ordinal,
        filter.name.clone().unwrap_or_default(),
    )
}

fn filter_owner_key(owner: &FilterOwner) -> String {
    match owner {
        FilterOwner::Report { .. } => "report".to_string(),
        FilterOwner::Page { name, .. } => format!("page:{name}"),
        FilterOwner::Visual {
            page_name, name, ..
        } => format!("visual:{page_name}:{name}"),
    }
}

fn where_uses_source_alias(filter: &Value) -> bool {
    let aliases = filter["filter"]["From"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["Name"].as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    source_ref_uses_alias(&filter["filter"]["Where"], &aliases)
}

fn source_ref_uses_alias(value: &Value, aliases: &[String]) -> bool {
    match value {
        Value::Object(object) => {
            if let Some(source_ref) = object.get("SourceRef").and_then(Value::as_object)
                && let Some(source) = source_ref.get("Source").and_then(Value::as_str)
                && aliases.iter().any(|alias| alias == source)
            {
                return true;
            }
            object
                .values()
                .any(|child| source_ref_uses_alias(child, aliases))
        }
        Value::Array(items) => items
            .iter()
            .any(|child| source_ref_uses_alias(child, aliases)),
        _ => false,
    }
}

fn interaction_summary(record: &ReportInteractionRecord) -> Value {
    json!({
        "ordinal": record.ordinal,
        "interactionType": record.interaction_type,
        "unsupported": record.unsupported,
        "staleVisualReference": stale_visual_reference(record),
        "sourceName": record.source_name,
        "targetName": record.target_name,
        "source": fixture_visual_ref(record.source_visual.as_ref(), &record.source_name),
        "target": fixture_visual_ref(record.target_visual.as_ref(), &record.target_name)
    })
}

fn stale_visual_reference(record: &ReportInteractionRecord) -> bool {
    (!record.source_name.is_empty() && record.source_visual.is_none())
        || (!record.target_name.is_empty() && record.target_visual.is_none())
}

fn fixture_visual_ref(visual: Option<&InteractionVisualRef>, raw_name: &str) -> Value {
    if let Some(visual) = visual {
        json!({
            "found": true,
            "handle": visual.handle,
            "name": visual.name,
            "title": visual.title,
            "visualType": visual.visual_type
        })
    } else {
        json!({
            "found": false,
            "handle": Value::Null,
            "name": raw_name,
            "title": Value::Null,
            "visualType": Value::Null
        })
    }
}

fn visual_summary(ordinal: usize, visual: &Value) -> CliResult<Value> {
    Ok(json!({
        "ordinal": ordinal,
        "name": visual["name"],
        "visualType": visual["visualType"],
        "title": visual["title"],
        "position": {
            "x": visual["position"]["x"],
            "y": visual["position"]["y"],
            "z": visual["position"]["z"],
            "width": visual["position"]["width"],
            "height": visual["position"]["height"],
            "tabOrder": visual["position"]["tabOrder"]
        },
        "bindingCount": visual["bindings"].as_array().map(Vec::len).unwrap_or_default(),
        "bindings": visual["bindings"].as_array().map(|items| items.iter().map(binding_summary).collect::<Vec<_>>()).unwrap_or_default(),
        "fingerprints": visual_raw_fingerprints(visual)?
    }))
}

fn visual_raw_fingerprints(visual: &Value) -> CliResult<Value> {
    let path = visual["path"].as_str().ok_or_else(|| {
        CliError::unexpected("deep inspect visual summary did not include visual.json path")
    })?;
    let raw = read_json_value(Path::new(path))?;
    Ok(json!({
        "visualJson": fingerprint_value(&raw)?,
        "visual": fingerprint_value(&raw["visual"])?,
        "queryState": fingerprint_value(&raw["visual"]["query"]["queryState"])?,
        "objects": fingerprint_value(&raw["visual"]["objects"])?,
        "visualContainerObjects": fingerprint_value(&raw["visual"]["visualContainerObjects"])?,
        "position": fingerprint_value(&raw["position"])?
    }))
}

fn binding_summary(binding: &Value) -> Value {
    json!({
        "role": binding["role"],
        "kind": binding["kind"],
        "table": binding["table"],
        "field": binding["field"],
        "column": binding["column"],
        "measure": binding["measure"]
    })
}

fn lint_findings_summary(lint: &Value) -> Vec<Value> {
    lint["findings"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|finding| {
                    json!({
                        "code": finding["code"],
                        "severity": finding["severity"],
                        "handle": finding["handle"]
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn parse_normalize_args(args: &[String]) -> CliResult<NormalizeOptions> {
    let mut options = NormalizeOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                set_project(
                    &mut options.project,
                    PathBuf::from(take_value(args, &mut i, "--project")?),
                    "fixture normalize",
                )?;
            }
            "--out" | "--out-file" => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown fixture normalize flag: {other}"
                ))
                .with_hint("Run `powerbi-cli fixture normalize <project-dir-or.pbip> --json`.")
                .with_suggested_command(
                    "powerbi-cli fixture normalize <project-dir-or.pbip> --json",
                ));
            }
            positional => {
                set_project(
                    &mut options.project,
                    PathBuf::from(positional),
                    "fixture normalize",
                )?;
                i += 1;
            }
        }
    }
    Ok(options)
}

fn parse_verify_args(args: &[String]) -> CliResult<VerifyOptions> {
    let mut options = VerifyOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                set_project(
                    &mut options.project,
                    PathBuf::from(take_value(args, &mut i, "--project")?),
                    "fixture verify",
                )?;
            }
            "--expected" => {
                options.expected = Some(PathBuf::from(take_value(args, &mut i, "--expected")?));
            }
            "--write-actual" => {
                options.write_actual =
                    Some(PathBuf::from(take_value(args, &mut i, "--write-actual")?));
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown fixture verify flag: {other}"
                ))
                .with_hint("Run `powerbi-cli fixture verify <project-dir-or.pbip> --expected <summary.json> --json`.")
                .with_suggested_command("powerbi-cli fixture verify <project-dir-or.pbip> --expected <summary.json> --json"));
            }
            positional => {
                set_project(
                    &mut options.project,
                    PathBuf::from(positional),
                    "fixture verify",
                )?;
                i += 1;
            }
        }
    }
    Ok(options)
}

fn set_project(current: &mut Option<PathBuf>, next: PathBuf, command: &str) -> CliResult<()> {
    if current.is_some() {
        return Err(
            CliError::invalid_args(format!("{command} accepts exactly one project path"))
                .with_hint("Use either a positional project path or --project, not both.")
                .with_suggested_command(format!(
                    "powerbi-cli {command} <project-dir-or.pbip> --json"
                )),
        );
    }
    *current = Some(next);
    Ok(())
}

fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    project.ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires <project-dir-or.pbip>"))
            .with_hint("Pass a PBIP project directory or .pbip file.")
            .with_suggested_command(format!(
                "powerbi-cli {command} <project-dir-or.pbip> --json"
            ))
    })
}

fn diff_json(expected: &Value, actual: &Value, path: &str) -> Vec<Value> {
    let mut diffs = Vec::new();
    diff_json_inner(expected, actual, path, &mut diffs);
    diffs.truncate(25);
    diffs
}

fn diff_json_inner(expected: &Value, actual: &Value, path: &str, diffs: &mut Vec<Value>) {
    if diffs.len() >= 25 {
        return;
    }
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            for key in sorted_keys(expected_map, actual_map) {
                let child_path = format!("{path}/{}", escape_json_pointer(&key));
                match (expected_map.get(&key), actual_map.get(&key)) {
                    (Some(left), Some(right)) => diff_json_inner(left, right, &child_path, diffs),
                    (Some(left), None) => diffs.push(json!({
                        "path": child_path,
                        "expected": left,
                        "actual": Value::Null
                    })),
                    (None, Some(right)) => diffs.push(json!({
                        "path": child_path,
                        "expected": Value::Null,
                        "actual": right
                    })),
                    (None, None) => {}
                }
                if diffs.len() >= 25 {
                    return;
                }
            }
        }
        (Value::Array(expected_items), Value::Array(actual_items)) => {
            let max_len = expected_items.len().max(actual_items.len());
            for index in 0..max_len {
                let child_path = format!("{path}/{index}");
                match (expected_items.get(index), actual_items.get(index)) {
                    (Some(left), Some(right)) => diff_json_inner(left, right, &child_path, diffs),
                    (Some(left), None) => diffs.push(json!({
                        "path": child_path,
                        "expected": left,
                        "actual": Value::Null
                    })),
                    (None, Some(right)) => diffs.push(json!({
                        "path": child_path,
                        "expected": Value::Null,
                        "actual": right
                    })),
                    (None, None) => {}
                }
                if diffs.len() >= 25 {
                    return;
                }
            }
        }
        _ if expected == actual => {}
        _ => diffs.push(json!({
            "path": if path.is_empty() { "/" } else { path },
            "expected": expected,
            "actual": actual
        })),
    }
}

fn sorted_keys(left: &Map<String, Value>, right: &Map<String, Value>) -> Vec<String> {
    let mut keys = left.keys().chain(right.keys()).cloned().collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    keys
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn write_json_file(path: &Path, value: &Value) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    fs::write(path, text)
        .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))
}

fn value_key(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn string_value(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn fingerprint_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn fingerprint_value(value: &Value) -> CliResult<String> {
    let canonical = serde_json::to_string(value).map_err(|err| {
        CliError::unexpected(format!(
            "serialize JSON subtree for fixture fingerprint: {err}"
        ))
    })?;
    Ok(fingerprint_hex(&canonical))
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli --json capabilities --for fixture` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for fixture")
    })?;
    *index += 2;
    Ok(value.clone())
}
