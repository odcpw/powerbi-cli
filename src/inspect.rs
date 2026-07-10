use crate::partitions::partition_summary_json;
use crate::relationship_tmdl::{RelationshipRecord, load_relationship_document};
use crate::tmdl::{column_handle, parse_table_document, table_handle};
use crate::{
    CliError, CliResult, ResolvedProject, ValidationReport, canonical_display, read_json_value,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn deep_inspect(
    resolved: &ResolvedProject,
    report: &ValidationReport,
) -> CliResult<Value> {
    let mut handles = Vec::new();
    let project_name = resolved
        .pbip_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("PowerBIProject");
    let project_handle = format!("project:{project_name}");
    handles.push(handle_entry(
        &project_handle,
        "project",
        project_name,
        Some(&resolved.pbip_path),
    ));

    let model = inspect_model(resolved, &mut handles)?;
    let report_detail = inspect_report(resolved, &mut handles)?;

    Ok(json!({
        "schema": "powerbi-cli.inspect.deep.v1",
        "project": {
            "handle": project_handle,
            "name": project_name,
            "projectDir": canonical_display(&resolved.project_dir),
            "pbip": canonical_display(&resolved.pbip_path),
            "reportDir": canonical_display(&resolved.report_dir),
            "semanticModelDir": canonical_display(&resolved.semantic_model_dir)
        },
        "counts": {
            "jsonFilesChecked": report.json_files_checked,
            "pages": report.pages,
            "visuals": report.visuals,
            "boundVisuals": report.bound_visuals,
            "tables": report.tables,
            "measures": report.measures,
            "relationships": report.relationships
        },
        "handles": handles,
        "model": model,
        "report": report_detail
    }))
}

fn inspect_model(resolved: &ResolvedProject, handles: &mut Vec<Value>) -> CliResult<Value> {
    let definition_dir = resolved.semantic_model_dir.join("definition");
    let tables_dir = definition_dir.join("tables");
    let mut tables = Vec::new();
    if tables_dir.is_dir() {
        for path in sorted_child_files(&tables_dir)? {
            if path.extension().and_then(|value| value.to_str()) == Some("tmdl") {
                tables.push(inspect_table_tmdl(&path, handles)?);
            }
        }
    }

    Ok(json!({
        "handle": "model:semantic",
        "definitionDir": canonical_display(&definition_dir),
        "tables": tables,
        "relationships": inspect_relationships(resolved, &definition_dir.join("relationships.tmdl"), handles)?
    }))
}

fn inspect_table_tmdl(path: &Path, handles: &mut Vec<Value>) -> CliResult<Value> {
    let table_document = parse_table_document(path.to_path_buf())?;
    let table_name = table_document.table.clone();

    let table_handle_value = table_handle(&table_name);
    handles.push(handle_entry(
        &table_handle_value,
        "table",
        &table_name,
        Some(path),
    ));
    let column_values = table_document
        .columns
        .iter()
        .map(|column| {
            let handle = column.handle();
            handles.push(handle_entry(&handle, "column", &column.name, Some(path)));
            json!({
                "handle": handle,
                "name": column.name,
                "isCalculated": column.is_calculated(),
                "expression": column.expression,
                "properties": {
                    "lineageTag": column.lineage_tag,
                    "dataType": column.data_type,
                    "formatString": column.format_string,
                    "summarizeBy": column.summarize_by,
                    "sourceColumn": column.source_column,
                    "displayFolder": column.display_folder,
                    "description": column.description,
                    "isHidden": column.is_hidden,
                    "isKey": column.is_key
                },
                "lineRange": {
                    "start": column.start_line + 1,
                    "end": column.end_line
                }
            })
        })
        .collect::<Vec<_>>();
    let measure_values = table_document
        .measures
        .into_iter()
        .map(|measure| {
            let handle = measure.handle();
            handles.push(handle_entry(&handle, "measure", &measure.name, Some(path)));
            json!({
                "handle": handle,
                "name": measure.name,
                "expression": measure.expression,
                "properties": {
                    "lineageTag": measure.lineage_tag,
                    "formatString": measure.format_string,
                    "displayFolder": measure.display_folder,
                    "description": measure.description
                },
                "lineRange": {
                    "start": measure.start_line + 1,
                    "end": measure.end_line
                }
            })
        })
        .collect::<Vec<_>>();
    let partition_values = table_document
        .partitions
        .iter()
        .map(|partition| {
            let handle = partition.handle();
            handles.push(handle_entry(
                &handle,
                "partition",
                &partition.name,
                Some(path),
            ));
            partition_summary_json(partition)
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "handle": table_handle_value,
        "name": table_name,
        "path": canonical_display(path),
        "columns": column_values,
        "measures": measure_values,
        "partitions": partition_values
    }))
}

fn inspect_relationships(
    resolved: &ResolvedProject,
    path: &Path,
    handles: &mut Vec<Value>,
) -> CliResult<Vec<Value>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let document = load_relationship_document(resolved)?;
    Ok(document
        .relationships
        .iter()
        .map(|relationship| inspect_relationship(relationship, handles))
        .collect())
}

fn inspect_relationship(relationship: &RelationshipRecord, handles: &mut Vec<Value>) -> Value {
    let handle = relationship.handle();
    handles.push(handle_entry(
        &handle,
        "relationship",
        &relationship.name,
        Some(&relationship.path),
    ));
    json!({
        "handle": handle,
        "name": relationship.name,
        "fromTable": relationship.from_table,
        "fromColumn": relationship.from_column,
        "toTable": relationship.to_table,
        "toColumn": relationship.to_column,
        "from": {
            "table": relationship.from_table,
            "column": relationship.from_column,
            "columnHandle": column_handle(&relationship.from_table, &relationship.from_column)
        },
        "to": {
            "table": relationship.to_table,
            "column": relationship.to_column,
            "columnHandle": column_handle(&relationship.to_table, &relationship.to_column)
        },
        "properties": {
            "crossFilteringBehavior": relationship.cross_filtering_behavior,
            "isActive": relationship.is_active
        },
        "path": canonical_display(&relationship.path),
        "lineRange": {
            "start": relationship.start_line + 1,
            "end": relationship.end_line
        },
        "block": relationship.block
    })
}

fn inspect_report(resolved: &ResolvedProject, handles: &mut Vec<Value>) -> CliResult<Value> {
    let pages_dir = resolved.report_dir.join("definition").join("pages");
    let pages_json_path = pages_dir.join("pages.json");
    let mut pages = Vec::new();
    if pages_json_path.is_file() {
        let pages_json = read_json_value(&pages_json_path)?;
        let page_order = pages_json["pageOrder"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let active_page_name = pages_json["activePageName"].as_str();
        for (index, page_name) in page_order.iter().enumerate() {
            pages.push(inspect_page(
                &pages_dir,
                page_name,
                index,
                active_page_name,
                handles,
            )?);
        }
    }

    Ok(json!({
        "handle": "report:main",
        "definitionDir": canonical_display(&resolved.report_dir.join("definition")),
        "pages": pages
    }))
}

fn inspect_page(
    pages_dir: &Path,
    page_name: &str,
    index: usize,
    active_page_name: Option<&str>,
    handles: &mut Vec<Value>,
) -> CliResult<Value> {
    let page_dir = pages_dir.join(page_name);
    let page_json_path = page_dir.join("page.json");
    let page_json = read_json_value(&page_json_path)?;
    let display_name = page_json["displayName"].as_str().unwrap_or(page_name);
    let handle = format!("page:{page_name}");
    handles.push(handle_entry(
        &handle,
        "page",
        display_name,
        Some(&page_json_path),
    ));

    let visuals_dir = page_dir.join("visuals");
    let visuals = if visuals_dir.is_dir() {
        sorted_child_dirs(&visuals_dir)?
            .into_iter()
            .map(|path| inspect_visual(page_name, &path, handles))
            .collect::<CliResult<Vec<_>>>()?
    } else {
        Vec::new()
    };

    Ok(json!({
        "handle": handle,
        "name": page_name,
        "displayName": display_name,
        "ordinal": index,
        "width": page_json["width"],
        "height": page_json["height"],
        "displayOption": page_json["displayOption"],
        "type": page_json["type"],
        "visibility": page_json["visibility"],
        "pageBinding": page_json["pageBinding"],
        "isActive": active_page_name == Some(page_name),
        "path": canonical_display(&page_json_path),
        "visuals": visuals
    }))
}

fn inspect_visual(
    page_name: &str,
    visual_dir: &Path,
    handles: &mut Vec<Value>,
) -> CliResult<Value> {
    let visual_json_path = visual_dir.join("visual.json");
    let visual_json = read_json_value(&visual_json_path)?;
    let visual_name = visual_json["name"]
        .as_str()
        .or_else(|| visual_dir.file_name().and_then(|value| value.to_str()))
        .unwrap_or("Visual");
    let visual_type = visual_json["visual"]["visualType"]
        .as_str()
        .unwrap_or("unknown");
    let title = visual_title(&visual_json).unwrap_or_else(|| {
        annotation_value(&visual_json, "powerbi-cli.placeholderTitle")
            .unwrap_or(visual_name)
            .to_string()
    });
    let handle = format!("visual:{page_name}:{visual_name}");
    handles.push(handle_entry(
        &handle,
        "visual",
        &title,
        Some(&visual_json_path),
    ));

    Ok(json!({
        "handle": handle,
        "name": visual_name,
        "title": title,
        "visualType": visual_type,
        "path": canonical_display(&visual_json_path),
        "position": visual_json["position"],
        "bindings": visual_bindings(&visual_json)
    }))
}

fn visual_bindings(visual_json: &Value) -> Vec<Value> {
    let mut bindings = Vec::new();
    if let Some(query_state) = visual_json["visual"]["query"]["queryState"].as_object() {
        for (role, role_value) in query_state {
            if let Some(projections) = role_value["projections"].as_array() {
                for projection in projections {
                    bindings.push(json!({
                        "role": role,
                        "queryRef": projection["queryRef"],
                        "nativeQueryRef": projection["nativeQueryRef"],
                        "displayName": projection["displayName"],
                        "kind": projection_kind(projection),
                        "table": projection_table(projection),
                        "field": projection_field(projection),
                        "column": projection_column(projection),
                        "measure": projection_measure(projection)
                    }));
                }
            }
        }
    }
    bindings
}

fn visual_title(visual_json: &Value) -> Option<String> {
    visual_json
        .pointer("/visual/objects/title/0/properties/text/expr/Literal/Value")
        .and_then(Value::as_str)
        .map(decode_text_literal)
        .filter(|value| !value.trim().is_empty())
}

fn decode_text_literal(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2 {
        return trimmed[1..trimmed.len() - 1].replace("''", "'");
    }
    trimmed.to_string()
}

fn projection_kind(projection: &Value) -> &'static str {
    if projection["field"]["Measure"].is_object() {
        "measure"
    } else if projection["field"]["Column"].is_object() {
        "column"
    } else {
        "unknown"
    }
}

fn projection_table(projection: &Value) -> Value {
    projection["field"]["Measure"]["Expression"]["SourceRef"]["Entity"]
        .as_str()
        .or_else(|| projection["field"]["Column"]["Expression"]["SourceRef"]["Entity"].as_str())
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

fn projection_field(projection: &Value) -> Value {
    projection["field"]["Measure"]["Property"]
        .as_str()
        .or_else(|| projection["field"]["Column"]["Property"].as_str())
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

fn projection_column(projection: &Value) -> Value {
    projection["field"]["Column"]["Property"]
        .as_str()
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

fn projection_measure(projection: &Value) -> Value {
    projection["field"]["Measure"]["Property"]
        .as_str()
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

fn annotation_value<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    value["annotations"]
        .as_array()?
        .iter()
        .find(|annotation| annotation["name"].as_str() == Some(name))
        .and_then(|annotation| annotation["value"].as_str())
}

fn handle_entry(handle: &str, kind: &str, name: &str, path: Option<&Path>) -> Value {
    json!({
        "handle": handle,
        "kind": kind,
        "name": name,
        "path": path.map(canonical_display)
    })
}

fn sorted_child_files(dir: &Path) -> CliResult<Vec<PathBuf>> {
    sorted_children(dir, |path| path.is_file())
}

fn sorted_child_dirs(dir: &Path) -> CliResult<Vec<PathBuf>> {
    sorted_children(dir, |path| path.is_dir())
}

fn sorted_children(dir: &Path, predicate: impl Fn(&Path) -> bool) -> CliResult<Vec<PathBuf>> {
    let mut paths = fs::read_dir(dir)
        .map_err(|err| CliError::file_not_found(format!("read {}: {err}", dir.display())))?
        .map(|entry| crate::read_dir_entry(dir, entry, "inspect directory children"))
        .collect::<CliResult<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| predicate(path))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
    Ok(paths)
}
