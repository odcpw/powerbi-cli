use crate::cli_support::{
    MutationMode, mode_name, require_mode_with_contract, required_project,
    set_report_visual_mode as set_mode, shell_arg, take_report_value as take_value, target_project,
};
use crate::pbir::{VisualRecord, VisualSelector, find_visual, load_report_snapshot, visual_detail};
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

const SET_OBJECT: &str = "report visuals set-object";
const SET_DISPLAY_NAME: &str = "report visuals set-display-name";
const DISPLAY_NAME_ROLES: &[&str] = &[
    "Values", "Category", "Series", "X", "Y", "Y2", "Size", "Rows", "Columns", "Tooltips",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyType {
    Bool,
    Double,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectHome {
    VisualObjects,
    VisualContainerObjects,
}

#[derive(Debug, Clone, Copy)]
struct ObjectProperty {
    object: &'static str,
    property: &'static str,
    value_type: PropertyType,
    home: ObjectHome,
}

const OBJECT_CATALOG: &[ObjectProperty] = &[
    ObjectProperty {
        object: "labels",
        property: "show",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "labels",
        property: "fontSize",
        value_type: PropertyType::Double,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "categoryLabels",
        property: "show",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "categoryLabels",
        property: "fontSize",
        value_type: PropertyType::Double,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "categoryLabels",
        property: "wordWrap",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "categoryAxis",
        property: "show",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "categoryAxis",
        property: "showAxisTitle",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "valueAxis",
        property: "show",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "valueAxis",
        property: "showAxisTitle",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualObjects,
    },
    ObjectProperty {
        object: "title",
        property: "show",
        value_type: PropertyType::Bool,
        home: ObjectHome::VisualContainerObjects,
    },
    ObjectProperty {
        object: "title",
        property: "text",
        value_type: PropertyType::String,
        home: ObjectHome::VisualContainerObjects,
    },
];

#[derive(Debug, Default)]
struct ObjectOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    object: Option<String>,
    property: Option<String>,
    value: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct DisplayNameOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    role: Option<String>,
    index: Option<usize>,
    display_name: Option<String>,
    clear: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn set_object(args: &[String]) -> CliResult<Value> {
    let options = parse_object_args(args)?;
    let source_project = required_project(options.project.clone(), SET_OBJECT)?;
    require_visual_selector(&options.selector, SET_OBJECT)?;
    let spec = resolve_object_property(options.object.as_deref(), options.property.as_deref())?;
    let raw_value = options.value.as_deref().ok_or_else(|| {
        CliError::invalid_args(format!("{SET_OBJECT} requires --value <raw>"))
            .with_hint("Pass a typed value: true|false for bools, a number for doubles, or free text for strings.")
            .with_suggested_command(set_object_usage())
    })?;
    let encoded = encode_property_value(spec, raw_value)?;
    let mode = require_mode_with_contract(
        options.mode,
        SET_OBJECT,
        "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.",
        set_object_usage(),
    )?;

    crate::cli_support::preflight_out_dir(args, set_object)?;
    let source_resolved = resolve_project(&source_project)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(&snapshot.pages, &options.selector, SET_OBJECT)?;
    let visual_path = visual_json_path(visual, SET_OBJECT)?;
    let mut visual_json = read_json_value(visual_path)?;
    let json_pointer = object_property_pointer(spec);
    let before = property_at(&visual_json, spec)
        .cloned()
        .unwrap_or(Value::Null);
    upsert_object_property(&mut visual_json, spec, encoded.clone())?;
    let after = property_at(&visual_json, spec)
        .cloned()
        .unwrap_or_else(|| encoded.clone());

    finish_visual_mutation(MutationResult {
        mode,
        target_resolved: &target_resolved,
        visual,
        visual_path,
        visual_json: &visual_json,
        schema: "powerbi-cli.report.visuals.objectMutation.v1",
        action: "set-object",
        changes: json!([{
            "kind": "pbir.visual.objectProperty",
            "action": "set-object",
            "path": canonical_display(visual_path),
            "object": spec.object,
            "property": spec.property,
            "jsonPointer": json_pointer,
            "before": before,
            "after": after
        }]),
        plan: json!({
            "object": spec.object,
            "property": spec.property,
            "jsonPointer": json_pointer,
            "before": before,
            "after": after
        }),
    })
}

pub(crate) fn set_display_name(args: &[String]) -> CliResult<Value> {
    let options = parse_display_name_args(args)?;
    let source_project = required_project(options.project.clone(), SET_DISPLAY_NAME)?;
    require_visual_selector(&options.selector, SET_DISPLAY_NAME)?;
    let role = require_display_name_role(options.role.as_deref())?;
    require_display_name_intent(&options)?;
    let index = options.index.unwrap_or(0);
    let mode = require_mode_with_contract(
        options.mode,
        SET_DISPLAY_NAME,
        "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.",
        set_display_name_usage(),
    )?;

    crate::cli_support::preflight_out_dir(args, set_display_name)?;
    let source_resolved = resolve_project(&source_project)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(&snapshot.pages, &options.selector, SET_DISPLAY_NAME)?;
    let visual_path = visual_json_path(visual, SET_DISPLAY_NAME)?;
    let mut visual_json = read_json_value(visual_path)?;
    let json_pointer = format!("/visual/query/queryState/{role}/projections/{index}/displayName");
    let before = projection_display_name(&visual_json, role, index);
    apply_display_name(
        &mut visual_json,
        role,
        index,
        options.display_name.as_deref(),
        options.clear,
    )?;
    let after = if options.clear {
        Value::Null
    } else {
        Value::String(options.display_name.clone().unwrap_or_default())
    };
    let action = if options.clear {
        "clear-display-name"
    } else {
        "set-display-name"
    };

    finish_visual_mutation(MutationResult {
        mode,
        target_resolved: &target_resolved,
        visual,
        visual_path,
        visual_json: &visual_json,
        schema: "powerbi-cli.report.visuals.displayNameMutation.v1",
        action,
        changes: json!([{
            "kind": "pbir.visual.displayName",
            "action": action,
            "path": canonical_display(visual_path),
            "role": role,
            "index": index,
            "jsonPointer": json_pointer,
            "before": before,
            "after": after
        }]),
        plan: json!({
            "role": role,
            "index": index,
            "jsonPointer": json_pointer,
            "clear": options.clear,
            "before": before,
            "after": after
        }),
    })
}

struct MutationResult<'a> {
    mode: MutationMode,
    target_resolved: &'a ResolvedProject,
    visual: &'a VisualRecord,
    visual_path: &'a Path,
    visual_json: &'a Value,
    schema: &'a str,
    action: &'a str,
    changes: Value,
    plan: Value,
}

fn finish_visual_mutation(result: MutationResult<'_>) -> CliResult<Value> {
    let dry_run = matches!(result.mode, MutationMode::DryRun);
    if !dry_run {
        write_json_atomic(result.visual_path, result.visual_json)?;
    }
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(result.target_resolved)?)
    };
    let validation_ok = validation
        .as_ref()
        .map(|report| report.errors.is_empty())
        .unwrap_or(true);
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&result.target_resolved.project_dir);
    let readback = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&result.visual.handle)
    );
    let inspect = format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&result.target_resolved.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&result.target_resolved.project_dir)
    );
    Ok(json!({
        "schema": result.schema,
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": result.action,
        "dryRun": dry_run,
        "mode": mode_name(result.mode),
        "projectDir": canonical_display(&result.target_resolved.project_dir),
        "pbip": canonical_display(&result.target_resolved.pbip_path),
        "reportDir": canonical_display(&result.target_resolved.report_dir),
        "target": visual_detail(result.visual),
        "plan": result.plan,
        "changes": result.changes,
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors,
            "counts": {
                "tables": report.tables,
                "relationships": report.relationships,
                "measures": report.measures,
                "pages": report.pages,
                "visuals": report.visuals,
                "boundVisuals": report.bound_visuals
            }
        })),
        "readbackCommand": readback,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, inspect, validate]
    }))
}

fn resolve_object_property(
    object: Option<&str>,
    property: Option<&str>,
) -> CliResult<&'static ObjectProperty> {
    let object = object.ok_or_else(|| {
        CliError::invalid_args(format!("{SET_OBJECT} requires --object <name>"))
            .with_hint(supported_pairs_hint())
            .with_suggested_command(set_object_usage())
    })?;
    let property = property.ok_or_else(|| {
        CliError::invalid_args(format!("{SET_OBJECT} requires --property <name>"))
            .with_hint(supported_pairs_hint())
            .with_suggested_command(set_object_usage())
    })?;
    OBJECT_CATALOG
        .iter()
        .find(|spec| spec.object == object && spec.property == property)
        .ok_or_else(|| {
            CliError::unsupported_feature(format!(
                "unsupported visual object/property `{object}.{property}`; supported pairs are: {}",
                supported_pairs().join(", ")
            ))
            .with_hint(supported_pairs_hint())
            .with_suggested_command(set_object_usage())
        })
}

fn encode_property_value(spec: &ObjectProperty, raw: &str) -> CliResult<Value> {
    match spec.value_type {
        PropertyType::Bool => {
            let value = parse_bool_value(raw).ok_or_else(|| {
                CliError::invalid_args(format!(
                    "--value for {object}.{property} must be true or false, got {raw}",
                    object = spec.object,
                    property = spec.property
                ))
                .with_hint(
                    "Boolean object properties accept only `--value true` or `--value false`.",
                )
                .with_suggested_command(set_object_usage())
            })?;
            Ok(literal_expression(if value { "true" } else { "false" }))
        }
        PropertyType::Double => {
            if parse_bool_value(raw).is_some() {
                return Err(CliError::invalid_args(format!(
                    "--value for {object}.{property} must be a number, got boolean {raw}",
                    object = spec.object,
                    property = spec.property
                ))
                .with_hint("Font sizes are PBIR doubles encoded as `<n>D`.")
                .with_suggested_command(format!(
                    "powerbi-cli report visuals set-object --project <project-dir-or.pbip> --handle <visual-handle> --object {} --property {} --value 20 --dry-run --json",
                    spec.object, spec.property
                )));
            }
            let parsed = raw.parse::<f64>().ok().filter(|value| value.is_finite());
            let parsed = parsed.ok_or_else(|| {
                CliError::invalid_args(format!(
                    "--value for {object}.{property} must be a finite number, got {raw}",
                    object = spec.object,
                    property = spec.property
                ))
                .with_hint("Pass a plain number such as `20` or `11.5`.")
                .with_suggested_command(set_object_usage())
            })?;
            Ok(literal_expression(&encode_double_literal(parsed)))
        }
        PropertyType::String => Ok(literal_expression(&encode_text_literal(raw))),
    }
}

fn literal_expression(value: &str) -> Value {
    json!({ "expr": { "Literal": { "Value": value } } })
}

fn encode_text_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn encode_double_literal(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < i64::MAX as f64 {
        format!("{}D", value as i64)
    } else {
        format!("{value}D")
    }
}

fn parse_bool_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn object_property_pointer(spec: &ObjectProperty) -> String {
    format!(
        "{}/{}/0/properties/{}",
        object_home_pointer(spec.home),
        spec.object,
        spec.property
    )
}

fn object_home_pointer(home: ObjectHome) -> &'static str {
    match home {
        ObjectHome::VisualObjects => "/visual/objects",
        ObjectHome::VisualContainerObjects => "/visual/visualContainerObjects",
    }
}

fn object_home_key(home: ObjectHome) -> &'static str {
    match home {
        ObjectHome::VisualObjects => "objects",
        ObjectHome::VisualContainerObjects => "visualContainerObjects",
    }
}

fn property_at<'a>(visual_json: &'a Value, spec: &ObjectProperty) -> Option<&'a Value> {
    visual_json.pointer(&object_property_pointer(spec))
}

fn upsert_object_property(
    visual_json: &mut Value,
    spec: &ObjectProperty,
    encoded: Value,
) -> CliResult<()> {
    let properties = ensure_object_slot_properties(visual_json, spec)?;
    properties.insert(spec.property.to_string(), encoded);
    Ok(())
}

fn ensure_object_slot_properties<'a>(
    visual_json: &'a mut Value,
    spec: &ObjectProperty,
) -> CliResult<&'a mut Map<String, Value>> {
    let root = json_object_mut(visual_json, "visual.json root")?;
    let visual = root
        .entry("visual".to_string())
        .or_insert_with(|| json!({}));
    let visual = json_object_mut(visual, "visual.json visual")?;
    let home_key = object_home_key(spec.home);
    let home_pointer = object_home_pointer(spec.home);
    let objects = visual
        .entry(home_key.to_string())
        .or_insert_with(|| json!({}));
    let objects = json_object_mut(objects, home_pointer)?;
    let cards = objects
        .entry(spec.object.to_string())
        .or_insert_with(|| json!([{ "properties": {} }]));
    let cards = cards.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "{home_pointer}/{} must be an array before set-object can patch it",
            spec.object
        ))
        .with_hint("Use `report visuals formatting show --include-raw` to inspect this visual before editing raw PBIR.")
        .with_suggested_command(
            "powerbi-cli report visuals formatting show --project <project-dir-or.pbip> --handle <visual-handle> --include-raw --json",
        )
    })?;
    if cards.is_empty() {
        cards.push(json!({ "properties": {} }));
    }
    let card = json_object_mut(&mut cards[0], "formatting card")?;
    let properties = card
        .entry("properties".to_string())
        .or_insert_with(|| json!({}));
    json_object_mut(properties, "formatting properties")
}

fn require_display_name_role(role: Option<&str>) -> CliResult<&str> {
    let role = role.ok_or_else(|| {
        CliError::invalid_args(format!("{SET_DISPLAY_NAME} requires --role <role>"))
            .with_hint(format!(
                "Supported roles are: {}.",
                DISPLAY_NAME_ROLES.join(", ")
            ))
            .with_suggested_command(set_display_name_usage())
    })?;
    DISPLAY_NAME_ROLES
        .iter()
        .copied()
        .find(|supported| *supported == role)
        .ok_or_else(|| {
            CliError::invalid_args(format!(
                "unsupported projection role `{role}`; supported roles are: {}",
                DISPLAY_NAME_ROLES.join(", ")
            ))
            .with_hint("Use a Desktop PBIR queryState role that already exists on the visual.")
            .with_suggested_command(set_display_name_usage())
        })
}

fn require_display_name_intent(options: &DisplayNameOptions) -> CliResult<()> {
    if options.clear && options.display_name.is_some() {
        return Err(CliError::invalid_args(
            "choose either --display-name <text> or --clear, not both",
        )
        .with_hint("Use --display-name to set a projection label, or --clear to remove it.")
        .with_suggested_command(set_display_name_usage()));
    }
    if options.clear {
        return Ok(());
    }
    let Some(display_name) = options.display_name.as_deref() else {
        return Err(CliError::invalid_args(format!(
            "{SET_DISPLAY_NAME} requires --display-name <text> or --clear"
        ))
        .with_hint("Start with `--dry-run` and specify a display name or --clear.")
        .with_suggested_command(set_display_name_usage()));
    };
    if display_name.trim().is_empty() {
        return Err(CliError::invalid_args("--display-name must not be empty")
            .with_hint("Pass visible text, or use --clear to remove an existing displayName.")
            .with_suggested_command(set_display_name_usage()));
    }
    Ok(())
}

fn projection_display_name(visual_json: &Value, role: &str, index: usize) -> Value {
    visual_json
        .pointer(&format!(
            "/visual/query/queryState/{role}/projections/{index}/displayName"
        ))
        .cloned()
        .unwrap_or(Value::Null)
}

fn apply_display_name(
    visual_json: &mut Value,
    role: &str,
    index: usize,
    display_name: Option<&str>,
    clear: bool,
) -> CliResult<()> {
    let projection = projection_object_mut(visual_json, role, index)?;
    if clear {
        projection.remove("displayName");
        return Ok(());
    }
    projection.insert(
        "displayName".to_string(),
        Value::String(display_name.unwrap_or_default().to_string()),
    );
    Ok(())
}

fn projection_object_mut<'a>(
    visual_json: &'a mut Value,
    role: &str,
    index: usize,
) -> CliResult<&'a mut Map<String, Value>> {
    let present = present_role_summary(visual_json);
    let query_state = visual_json
        .pointer_mut("/visual/query/queryState")
        .and_then(Value::as_object_mut);
    let Some(query_state) = query_state else {
        return Err(missing_projection_error(role, index, &present));
    };
    let Some(role_value) = query_state.get_mut(role) else {
        return Err(missing_projection_error(role, index, &present));
    };
    let Some(projections) = role_value
        .get_mut("projections")
        .and_then(Value::as_array_mut)
    else {
        return Err(missing_projection_error(role, index, &present));
    };
    if index >= projections.len() {
        return Err(missing_projection_error(role, index, &present));
    }
    json_object_mut(
        &mut projections[index],
        &format!("queryState.{role}.projections[{index}]"),
    )
}

fn present_role_summary(visual_json: &Value) -> String {
    let Some(query_state) = visual_json
        .pointer("/visual/query/queryState")
        .and_then(Value::as_object)
    else {
        return "none".to_string();
    };
    let mut roles = query_state
        .iter()
        .map(|(role, value)| {
            let count = value
                .get("projections")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            (role.clone(), count)
        })
        .collect::<Vec<_>>();
    roles.sort_by(|left, right| left.0.cmp(&right.0));
    if roles.is_empty() {
        "none".to_string()
    } else {
        roles
            .into_iter()
            .map(|(role, count)| format!("{role} ({count})"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn missing_projection_error(role: &str, index: usize, present: &str) -> CliError {
    CliError::invalid_args(format!(
        "visual has no queryState.{role}.projections[{index}]; present roles: {present}"
    ))
    .with_hint(format!(
        "Supported roles are: {}. Use `report visuals show` to inspect the current projections.",
        DISPLAY_NAME_ROLES.join(", ")
    ))
    .with_suggested_command(
        "powerbi-cli report visuals show --project <project-dir-or.pbip> --handle <visual-handle> --json",
    )
}

fn parse_object_args(args: &[String]) -> CliResult<ObjectOptions> {
    let mut options = ObjectOptions::default();
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
            "--object" => options.object = Some(take_value(args, &mut i, "--object")?),
            "--property" => options.property = Some(take_value(args, &mut i, "--property")?),
            "--value" => options.value = Some(take_value(args, &mut i, "--value")?),
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals set-object flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report visuals set-object\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report visuals set-object\"",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_display_name_args(args: &[String]) -> CliResult<DisplayNameOptions> {
    let mut options = DisplayNameOptions::default();
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
            "--role" => options.role = Some(take_value(args, &mut i, "--role")?),
            "--index" => {
                let value = take_value(args, &mut i, "--index")?;
                let parsed = value.parse::<usize>().map_err(|_| {
                    CliError::invalid_args("--index must be a nonnegative integer")
                        .with_hint("Projection indexes are zero-based.")
                        .with_suggested_command(set_display_name_usage())
                })?;
                options.index = Some(parsed);
            }
            "--display-name" | "--displayName" => {
                options.display_name = Some(take_value(args, &mut i, "--display-name")?);
            }
            "--clear" => {
                options.clear = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals set-display-name flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report visuals set-display-name\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report visuals set-display-name\"",
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

fn visual_json_path<'a>(visual: &'a VisualRecord, command: &str) -> CliResult<&'a PathBuf> {
    visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual has no path in inspect output: {}",
            visual.handle
        ))
        .with_hint("Run `validate --strict` before mutating this report.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle {} --json",
            shell_arg(&visual.handle)
        ))
    })
}

fn json_object_mut<'a>(value: &'a mut Value, label: &str) -> CliResult<&'a mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{label} must be a JSON object"))
            .with_hint("Run `validate --strict` before editing this visual.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })
}

fn supported_pairs() -> Vec<String> {
    OBJECT_CATALOG
        .iter()
        .map(|spec| format!("{}.{}", spec.object, spec.property))
        .collect()
}

fn supported_pairs_hint() -> String {
    format!(
        "Supported object/property pairs: {}.",
        supported_pairs().join(", ")
    )
}

fn set_object_usage() -> &'static str {
    "powerbi-cli report visuals set-object --project <project-dir-or.pbip> --handle <visual-handle> --object categoryLabels --property fontSize --value 20 --dry-run --json"
}

fn set_display_name_usage() -> &'static str {
    "powerbi-cli report visuals set-display-name --project <project-dir-or.pbip> --handle <visual-handle> --role Values --display-name <text> --dry-run --json"
}
