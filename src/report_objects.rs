use crate::cli_support::{required_project, shell_arg, take_value};
use crate::inspect::deep_inspect;
use crate::pbir_bookmarks::{bookmark_record_json, list_report_bookmarks};
use crate::pbir_filters::{FilterOwner, filter_record_json, list_report_filters};
use crate::pbir_interactions::{interaction_record_json, list_report_interactions};
use crate::pbir_slicers::{list_report_slicers, slicer_record_json};
use crate::{
    CliError, CliResult, ResolvedProject, canonical_display, command_arg, read_json_value,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct ObjectOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    kind: Option<String>,
    name_contains: Option<String>,
    title_contains: Option<String>,
    visual_type: Option<String>,
    path_contains: Option<String>,
    selector: Option<String>,
    include_raw: bool,
}

#[derive(Debug, Clone)]
enum ObjectPredicate {
    Handle(String),
    Kind(String),
    NameContains(String),
    TitleContains(String),
    VisualType(String),
    PathContains(String),
}

pub(crate) fn objects_command(command: &str, args: &[String]) -> CliResult<Value> {
    match command {
        "tree" => tree_command(args),
        "find" => find_command(args),
        "cat" => cat_command(args),
        "query" => query_command(args),
        _ => Err(
            CliError::invalid_args(format!("unknown report object command: {command}"))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report tree\"`.")
                .with_suggested_command("powerbi-cli --json capabilities --for \"report tree\""),
        ),
    }
}

fn tree_command(args: &[String]) -> CliResult<Value> {
    let options = parse_object_args("report tree", args)?;
    let project = required_project(options.project, "report tree")?;
    let (context, objects) = load_object_context(&project)?;
    Ok(json!({
        "schema": "powerbi-cli.report.objects.tree.v1",
        "ok": context.validation_ok,
        "projectDir": context.project_dir,
        "pbip": context.pbip,
        "reportDir": context.report_dir,
        "counts": object_counts(&objects),
        "objects": objects.iter().map(strip_object_detail).collect::<Vec<_>>(),
        "tree": object_tree(&objects),
        "warnings": context.warnings,
        "errors": context.errors,
        "next": [
            format!("powerbi-cli report find --project {} --kind visual --json", command_arg(&context.project_path)),
            format!("powerbi-cli report cat --project {} --handle <handle> --json", command_arg(&context.project_path)),
            format!("powerbi-cli report query --project {} --selector kind:visual --json", command_arg(&context.project_path)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&context.project_path))
        ]
    }))
}

fn find_command(args: &[String]) -> CliResult<Value> {
    let options = parse_object_args("report find", args)?;
    let predicates = find_predicates(&options)?;
    let project = required_project(options.project, "report find")?;
    let (context, objects) = load_object_context(&project)?;
    let matches = objects
        .iter()
        .filter(|object| {
            predicates
                .iter()
                .all(|predicate| matches_predicate(object, predicate))
        })
        .map(strip_object_detail)
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.report.objects.find.v1",
        "ok": context.validation_ok,
        "projectDir": context.project_dir,
        "pbip": context.pbip,
        "reportDir": context.report_dir,
        "predicates": predicates_json(&predicates),
        "counts": {
            "objects": objects.len(),
            "matched": matches.len()
        },
        "objects": matches,
        "warnings": context.warnings,
        "errors": context.errors,
        "next": [
            format!("powerbi-cli report cat --project {} --handle <handle> --json", command_arg(&context.project_path)),
            format!("powerbi-cli report tree --project {} --json", command_arg(&context.project_path))
        ]
    }))
}

fn cat_command(args: &[String]) -> CliResult<Value> {
    let options = parse_object_args("report cat", args)?;
    let project = required_project(options.project, "report cat")?;
    let handle = options.handle.as_deref().ok_or_else(|| {
        CliError::invalid_args("report cat requires --handle <handle>")
            .with_hint("Run `report tree` or `report find` to discover stable handles.")
            .with_suggested_command(
                "powerbi-cli report tree --project <project-dir-or.pbip> --json",
            )
    })?;
    let (context, objects) = load_object_context(&project)?;
    let object = object_by_handle(&objects, handle)?;
    Ok(json!({
        "schema": "powerbi-cli.report.objects.cat.v1",
        "ok": context.validation_ok,
        "projectDir": context.project_dir,
        "pbip": context.pbip,
        "reportDir": context.report_dir,
        "handle": handle,
        "object": object,
        "raw": raw_for_object(&object, options.include_raw)?,
        "rawIncluded": options.include_raw,
        "warnings": context.warnings,
        "errors": context.errors,
        "next": [
            format!("powerbi-cli report find --project {} --kind {} --json", command_arg(&context.project_path), shell_arg(object["kind"].as_str().unwrap_or("object"))),
            format!("powerbi-cli validate --strict {} --json", command_arg(&context.project_path))
        ]
    }))
}

fn query_command(args: &[String]) -> CliResult<Value> {
    let options = parse_object_args("report query", args)?;
    let project = required_project(options.project, "report query")?;
    let selector = options.selector.as_deref().ok_or_else(|| {
        CliError::invalid_args("report query requires --selector <selector>")
            .with_hint(
                "Supported selectors: handle:<handle>, kind:<kind>, visualType:<type>, title~:<text>, name~:<text>, path~:<text>.",
            )
            .with_suggested_command(
                "powerbi-cli report query --project <project-dir-or.pbip> --selector kind:visual --json",
            )
    })?;
    let predicate = parse_selector(selector)?;
    let (context, objects) = load_object_context(&project)?;
    let matches = objects
        .iter()
        .filter(|object| matches_predicate(object, &predicate))
        .map(strip_object_detail)
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.report.objects.query.v1",
        "ok": context.validation_ok,
        "projectDir": context.project_dir,
        "pbip": context.pbip,
        "reportDir": context.report_dir,
        "selector": selector,
        "predicate": predicate_json(&predicate),
        "counts": {
            "objects": objects.len(),
            "matched": matches.len()
        },
        "objects": matches,
        "warnings": context.warnings,
        "errors": context.errors,
        "next": [
            format!("powerbi-cli report cat --project {} --handle <handle> --json", command_arg(&context.project_path)),
            format!("powerbi-cli report tree --project {} --json", command_arg(&context.project_path))
        ]
    }))
}

struct ObjectContext {
    project_path: PathBuf,
    project_dir: String,
    pbip: String,
    report_dir: String,
    validation_ok: bool,
    warnings: Value,
    errors: Value,
}

fn load_object_context(project: &Path) -> CliResult<(ObjectContext, Vec<Value>)> {
    let resolved = resolve_project(project)?;
    let validation = validate_project(&resolved)?;
    let deep = deep_inspect(&resolved, &validation)?;
    let mut objects = flatten_objects(&deep);
    append_report_metadata_objects(&resolved, &mut objects)?;
    Ok((
        ObjectContext {
            project_path: resolved.project_dir.clone(),
            project_dir: canonical_display(&resolved.project_dir),
            pbip: canonical_display(&resolved.pbip_path),
            report_dir: canonical_display(&resolved.report_dir),
            validation_ok: validation.errors.is_empty(),
            warnings: Value::Array(validation.warnings.into_iter().map(Value::String).collect()),
            errors: Value::Array(validation.errors.into_iter().map(Value::String).collect()),
        },
        objects,
    ))
}

fn flatten_objects(deep: &Value) -> Vec<Value> {
    let mut objects = Vec::new();
    let project = &deep["project"];
    push_object(
        &mut objects,
        "project",
        project["handle"].as_str().unwrap_or("project:main"),
        project["name"].as_str().unwrap_or("project"),
        None,
        project["pbip"].as_str(),
        project.clone(),
    );

    let model = &deep["model"];
    push_object(
        &mut objects,
        "model",
        model["handle"].as_str().unwrap_or("model:semantic"),
        "semantic model",
        Some(project["handle"].as_str().unwrap_or("project:main")),
        None,
        model.clone(),
    );
    if let Some(tables) = model["tables"].as_array() {
        for table in tables {
            let table_handle = table["handle"].as_str().unwrap_or("table:unknown");
            push_object(
                &mut objects,
                "table",
                table_handle,
                table["name"].as_str().unwrap_or("table"),
                Some("model:semantic"),
                table["path"].as_str(),
                table.clone(),
            );
            push_children(&mut objects, table, "columns", "column", table_handle);
            push_children(&mut objects, table, "measures", "measure", table_handle);
            push_children(&mut objects, table, "partitions", "partition", table_handle);
        }
    }
    push_children(
        &mut objects,
        model,
        "relationships",
        "relationship",
        "model:semantic",
    );

    let report = &deep["report"];
    push_object(
        &mut objects,
        "report",
        report["handle"].as_str().unwrap_or("report:main"),
        "report",
        Some(project["handle"].as_str().unwrap_or("project:main")),
        None,
        report.clone(),
    );
    if let Some(pages) = report["pages"].as_array() {
        for page in pages {
            let page_handle = page["handle"].as_str().unwrap_or("page:unknown");
            push_object(
                &mut objects,
                "page",
                page_handle,
                page["displayName"]
                    .as_str()
                    .or_else(|| page["name"].as_str())
                    .unwrap_or("page"),
                Some("report:main"),
                page["path"].as_str(),
                page.clone(),
            );
            if let Some(visuals) = page["visuals"].as_array() {
                for visual in visuals {
                    push_visual_with_bindings(&mut objects, visual, page_handle);
                }
            }
        }
    }
    objects
}

fn append_report_metadata_objects(
    resolved: &ResolvedProject,
    objects: &mut Vec<Value>,
) -> CliResult<()> {
    let (filters, _) = list_report_filters(resolved)?;
    for record in filters {
        let parent = filter_parent_handle(&record.owner);
        let name = record
            .display_name
            .clone()
            .or_else(|| record.name.clone())
            .unwrap_or_else(|| record.handle.clone());
        let path = canonical_display(&record.path);
        let detail = filter_record_json(&record, false);
        push_object(
            objects,
            "filter",
            &record.handle,
            &name,
            Some(&parent),
            Some(&path),
            detail,
        );
    }

    let (slicers, _) = list_report_slicers(resolved)?;
    for record in slicers {
        let path = record.path.as_ref().map(|path| canonical_display(path));
        let detail = slicer_record_json(&record, false);
        push_object(
            objects,
            "slicer",
            &record.handle,
            &record.title,
            Some(&record.visual_handle),
            path.as_deref(),
            detail,
        );
    }

    let (bookmarks, _, _) = list_report_bookmarks(resolved)?;
    for record in bookmarks {
        let path = canonical_display(&record.path);
        let detail = bookmark_record_json(&record, false);
        push_object(
            objects,
            "bookmark",
            &record.handle,
            &record.display_name,
            Some("report:main"),
            Some(&path),
            detail,
        );
    }

    let (interactions, _) = list_report_interactions(resolved)?;
    for record in interactions {
        let path = canonical_display(&record.path);
        let name = format!(
            "{} -> {} ({})",
            record.source_name, record.target_name, record.interaction_type
        );
        let detail = interaction_record_json(&record, false);
        push_object(
            objects,
            "interaction",
            &record.handle,
            &name,
            Some(&record.page_handle),
            Some(&path),
            detail,
        );
    }

    Ok(())
}

fn filter_parent_handle(owner: &FilterOwner) -> String {
    match owner {
        FilterOwner::Report { .. } => "report:main".to_string(),
        FilterOwner::Page { handle, .. } => handle.clone(),
        FilterOwner::Visual { handle, .. } => handle.clone(),
    }
}

fn push_visual_with_bindings(objects: &mut Vec<Value>, visual: &Value, page_handle: &str) {
    let visual_handle = visual["handle"].as_str().unwrap_or("visual:unknown");
    push_object(
        objects,
        "visual",
        visual_handle,
        visual["name"]
            .as_str()
            .or_else(|| visual["title"].as_str())
            .unwrap_or("visual"),
        Some(page_handle),
        visual["path"].as_str(),
        visual.clone(),
    );
    let Some(bindings) = visual["bindings"].as_array() else {
        return;
    };
    for (index, binding) in bindings.iter().enumerate() {
        let handle = format!("binding:{visual_handle}:{index}");
        let mut detail = binding.clone();
        if let Some(object) = detail.as_object_mut() {
            object.insert("handle".to_string(), Value::String(handle.clone()));
            object.insert(
                "visualHandle".to_string(),
                Value::String(visual_handle.to_string()),
            );
            object.insert("ordinal".to_string(), Value::from(index));
        }
        let name = binding_name(binding, index);
        push_object(
            objects,
            "binding",
            &handle,
            &name,
            Some(visual_handle),
            visual["path"].as_str(),
            detail,
        );
    }
}

fn binding_name(binding: &Value, index: usize) -> String {
    let role = binding["role"].as_str().unwrap_or("field");
    if let (Some(table), Some(field)) = (binding["table"].as_str(), binding["field"].as_str()) {
        format!("{role}: {table}[{field}]")
    } else if let Some(display_name) = binding["displayName"].as_str() {
        format!("{role}: {display_name}")
    } else if let Some(query_ref) = binding["queryRef"].as_str() {
        format!("{role}: {query_ref}")
    } else {
        format!("{role}: binding {index}")
    }
}

fn push_children(
    objects: &mut Vec<Value>,
    parent: &Value,
    field: &str,
    kind: &str,
    parent_handle: &str,
) {
    if let Some(items) = parent[field].as_array() {
        for item in items {
            let handle = item["handle"].as_str().unwrap_or(kind);
            let name = item["name"]
                .as_str()
                .or_else(|| item["title"].as_str())
                .unwrap_or(kind);
            push_object(
                objects,
                kind,
                handle,
                name,
                Some(parent_handle),
                item["path"].as_str(),
                item.clone(),
            );
        }
    }
}

fn push_object(
    objects: &mut Vec<Value>,
    kind: &str,
    handle: &str,
    name: &str,
    parent: Option<&str>,
    path: Option<&str>,
    object: Value,
) {
    objects.push(json!({
        "handle": handle,
        "kind": kind,
        "name": name,
        "title": object["title"].clone(),
        "visualType": object["visualType"].clone(),
        "parentHandle": parent,
        "path": path,
        "object": object
    }));
}

fn strip_object_detail(value: &Value) -> Value {
    json!({
        "handle": value["handle"],
        "kind": value["kind"],
        "name": value["name"],
        "title": value["title"],
        "visualType": value["visualType"],
        "parentHandle": value["parentHandle"],
        "path": value["path"]
    })
}

fn object_tree(objects: &[Value]) -> Vec<Value> {
    objects
        .iter()
        .filter(|object| object["parentHandle"].is_null())
        .map(|object| tree_node(object, objects))
        .collect()
}

fn tree_node(object: &Value, objects: &[Value]) -> Value {
    let handle = object["handle"].as_str().unwrap_or_default();
    json!({
        "handle": object["handle"],
        "kind": object["kind"],
        "name": object["name"],
        "title": object["title"],
        "visualType": object["visualType"],
        "children": objects
            .iter()
            .filter(|candidate| candidate["parentHandle"].as_str() == Some(handle))
            .map(|child| tree_node(child, objects))
            .collect::<Vec<_>>()
    })
}

fn object_counts(objects: &[Value]) -> Value {
    let mut counts = serde_json::Map::new();
    counts.insert("objects".to_string(), Value::from(objects.len()));
    for object in objects {
        if let Some(kind) = object["kind"].as_str() {
            let next = counts.get(kind).and_then(Value::as_u64).unwrap_or(0) + 1;
            counts.insert(kind.to_string(), Value::from(next));
        }
    }
    Value::Object(counts)
}

fn object_by_handle(objects: &[Value], handle: &str) -> CliResult<Value> {
    let matches = objects
        .iter()
        .filter(|object| object["handle"].as_str() == Some(handle))
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [object] => Ok(object.clone()),
        [] => Err(
            CliError::invalid_args(format!("report object not found: {handle}"))
                .with_hint("Run `report tree` or `report find` to discover stable handles.")
                .with_suggested_command(
                    "powerbi-cli report tree --project <project-dir-or.pbip> --json",
                ),
        ),
        _ => Err(CliError::validation_failed(format!(
            "duplicate report object handle found: {handle}"
        ))),
    }
}

fn raw_for_object(object: &Value, include_raw: bool) -> CliResult<Value> {
    if !include_raw {
        return Ok(Value::Null);
    }
    let Some(path) = object["path"].as_str() else {
        return Ok(json!({
            "available": false,
            "reason": "object has no backing file path"
        }));
    };
    let path = PathBuf::from(path);
    let raw = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => json!({
            "available": true,
            "kind": "json",
            "path": canonical_display(&path),
            "value": read_json_value(&path)?
        }),
        _ => json!({
            "available": true,
            "kind": "text",
            "path": canonical_display(&path),
            "text": fs::read_to_string(&path)
                .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?
        }),
    };
    Ok(raw)
}

fn parse_object_args(command: &str, args: &[String]) -> CliResult<ObjectOptions> {
    let mut options = ObjectOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--kind" => options.kind = Some(take_value(args, &mut i, "--kind")?),
            "--name-contains" | "--nameContains" => {
                options.name_contains = Some(take_value(args, &mut i, "--name-contains")?);
            }
            "--title-contains" | "--titleContains" => {
                options.title_contains = Some(take_value(args, &mut i, "--title-contains")?);
            }
            "--visual-type" | "--visualType" => {
                options.visual_type = Some(take_value(args, &mut i, "--visual-type")?);
            }
            "--path-contains" | "--pathContains" => {
                options.path_contains = Some(take_value(args, &mut i, "--path-contains")?);
            }
            "--selector" => options.selector = Some(take_value(args, &mut i, "--selector")?),
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other if !other.starts_with('-') && options.project.is_none() => {
                options.project = Some(PathBuf::from(other));
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!("unknown {command} flag: {other}"))
                    .with_hint(format!(
                        "Run `powerbi-cli --json capabilities --for \"{command}\"` for exact usage."
                    ))
                    .with_suggested_command(format!(
                        "powerbi-cli --json capabilities --for \"{command}\""
                    )));
            }
        }
    }
    Ok(options)
}

fn find_predicates(options: &ObjectOptions) -> CliResult<Vec<ObjectPredicate>> {
    let mut predicates = Vec::new();
    if let Some(handle) = &options.handle {
        predicates.push(ObjectPredicate::Handle(handle.clone()));
    }
    if let Some(kind) = &options.kind {
        predicates.push(ObjectPredicate::Kind(kind.clone()));
    }
    if let Some(value) = &options.name_contains {
        predicates.push(ObjectPredicate::NameContains(value.clone()));
    }
    if let Some(value) = &options.title_contains {
        predicates.push(ObjectPredicate::TitleContains(value.clone()));
    }
    if let Some(value) = &options.visual_type {
        predicates.push(ObjectPredicate::VisualType(value.clone()));
    }
    if let Some(value) = &options.path_contains {
        predicates.push(ObjectPredicate::PathContains(value.clone()));
    }
    if predicates.is_empty() {
        return Err(CliError::invalid_args("report find requires at least one filter")
            .with_hint(
                "Use --kind, --handle, --visual-type, --name-contains, --title-contains, or --path-contains.",
            )
            .with_suggested_command(
                "powerbi-cli report find --project <project-dir-or.pbip> --kind visual --json",
            ));
    }
    Ok(predicates)
}

fn parse_selector(selector: &str) -> CliResult<ObjectPredicate> {
    if let Some(value) = selector.strip_prefix("handle:") {
        return Ok(ObjectPredicate::Handle(nonempty_selector(value, selector)?));
    }
    if let Some(value) = selector.strip_prefix("kind:") {
        return Ok(ObjectPredicate::Kind(nonempty_selector(value, selector)?));
    }
    if let Some(value) = selector.strip_prefix("visualType:") {
        return Ok(ObjectPredicate::VisualType(nonempty_selector(
            value, selector,
        )?));
    }
    if let Some(value) = selector.strip_prefix("title~:") {
        return Ok(ObjectPredicate::TitleContains(nonempty_selector(
            value, selector,
        )?));
    }
    if let Some(value) = selector.strip_prefix("name~:") {
        return Ok(ObjectPredicate::NameContains(nonempty_selector(
            value, selector,
        )?));
    }
    if let Some(value) = selector.strip_prefix("path~:") {
        return Ok(ObjectPredicate::PathContains(nonempty_selector(
            value, selector,
        )?));
    }
    Err(CliError::invalid_args(format!("invalid report query selector: {selector}"))
        .with_hint(
            "Supported selectors: handle:<handle>, kind:<kind>, visualType:<type>, title~:<text>, name~:<text>, path~:<text>.",
        )
        .with_suggested_command(
            "powerbi-cli report query --project <project-dir-or.pbip> --selector kind:visual --json",
        ))
}

fn nonempty_selector(value: &str, selector: &str) -> CliResult<String> {
    if value.trim().is_empty() {
        return Err(CliError::invalid_args(format!(
            "empty report query selector value: {selector}"
        )));
    }
    Ok(value.to_string())
}

fn matches_predicate(object: &Value, predicate: &ObjectPredicate) -> bool {
    match predicate {
        ObjectPredicate::Handle(value) => object["handle"].as_str() == Some(value),
        ObjectPredicate::Kind(value) => object["kind"]
            .as_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(value)),
        ObjectPredicate::NameContains(value) => {
            contains_case_insensitive(object["name"].as_str(), value)
        }
        ObjectPredicate::TitleContains(value) => {
            contains_case_insensitive(object["title"].as_str(), value)
        }
        ObjectPredicate::VisualType(value) => object["visualType"]
            .as_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(value)),
        ObjectPredicate::PathContains(value) => {
            contains_case_insensitive(object["path"].as_str(), value)
        }
    }
}

fn contains_case_insensitive(haystack: Option<&str>, needle: &str) -> bool {
    haystack
        .map(|haystack| {
            haystack
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        })
        .unwrap_or(false)
}

fn predicates_json(predicates: &[ObjectPredicate]) -> Vec<Value> {
    predicates.iter().map(predicate_json).collect()
}

fn predicate_json(predicate: &ObjectPredicate) -> Value {
    match predicate {
        ObjectPredicate::Handle(value) => json!({"kind": "handle", "value": value}),
        ObjectPredicate::Kind(value) => json!({"kind": "kind", "value": value}),
        ObjectPredicate::NameContains(value) => json!({"kind": "nameContains", "value": value}),
        ObjectPredicate::TitleContains(value) => json!({"kind": "titleContains", "value": value}),
        ObjectPredicate::VisualType(value) => json!({"kind": "visualType", "value": value}),
        ObjectPredicate::PathContains(value) => json!({"kind": "pathContains", "value": value}),
    }
}
