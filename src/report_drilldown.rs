use crate::cli_support::{
    MutationMode, mode_name, required_project, set_mode, shell_arg, take_value, target_project,
};
use crate::pbir::{VisualSelector, find_visual, load_report_snapshot, visual_detail};
use crate::pbir_bindings::{
    VisualBindingKind, VisualBindingResolved, binding_summary, set_binding_status_annotation,
    visual_query_json,
};
use crate::project_io::write_json_atomic;
use crate::tmdl::{load_table_documents, same_name};
use crate::visual_catalog::{VisualBindingFamily, binding_family};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct HierarchyOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    fields: Vec<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

struct HierarchyPlan {
    visual_json: Value,
    before: Value,
    after: Value,
    controls_before: Value,
    controls_after: Value,
    fields: Vec<Value>,
    bindings: Vec<Value>,
}

pub(crate) fn drilldown_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report drilldown requires a subcommand: set-hierarchy",
        )
        .with_hint("Set a hierarchy axis on an existing line, area, bar, column, or combo chart.")
        .with_suggested_command(
            "powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> --handle <visual-handle> --field 'Table[Year]' --field 'Table[Month]' --dry-run --json",
        ));
    };
    match action.as_str() {
        "set-hierarchy" | "setHierarchy" | "hierarchy" | "set" => set_hierarchy(rest),
        other => Err(CliError::invalid_args(format!(
            "unknown report drilldown command: {other}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report drilldown\"` for exact usage.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report drilldown\"")),
    }
}

fn set_hierarchy(args: &[String]) -> CliResult<Value> {
    let options = parse_hierarchy_args(args)?;
    let source_project =
        required_project(options.project.clone(), "report drilldown set-hierarchy")?;
    require_visual_selector(&options.selector, "report drilldown set-hierarchy")?;
    require_hierarchy_fields(&options)?;
    let mode = require_drilldown_mode(options.mode, "report drilldown set-hierarchy")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, set_hierarchy)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report drilldown set-hierarchy",
    )?;
    let visual_path = visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!("visual has no path: {}", visual.handle))
    })?;
    let plan = build_hierarchy_plan(&target_resolved, visual_path, &visual.visual_type, &options)?;

    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(visual_path, &plan.visual_json)?;
    }

    drilldown_response(
        &target_resolved,
        mode,
        visual_path,
        visual,
        &plan,
        options.include_raw,
    )
}

fn build_hierarchy_plan(
    resolved: &ResolvedProject,
    visual_path: &std::path::Path,
    visual_type: &str,
    options: &HierarchyOptions,
) -> CliResult<HierarchyPlan> {
    let mut visual_json = read_json_value(visual_path)?;
    ensure_category_chart(visual_type)?;
    let docs = load_table_documents(resolved)?;
    let resolved_bindings = options
        .fields
        .iter()
        .map(|field| {
            let (table, column) = parse_field_ref(field)?;
            resolve_column_ref(&docs, &table, &column)?;
            Ok(VisualBindingResolved {
                role: "Category".to_string(),
                table,
                field: column,
                kind: VisualBindingKind::Column,
                data_type: None,
                display_name: None,
                format_string: None,
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    for (index, binding) in resolved_bindings.iter().enumerate() {
        if resolved_bindings[..index].iter().any(|previous| {
            same_name(&previous.table, &binding.table) && same_name(&previous.field, &binding.field)
        }) {
            return Err(CliError::invalid_args(format!(
                "duplicate drilldown hierarchy field: {}[{}]",
                binding.table, binding.field
            )));
        }
    }

    let before = category_state(&visual_json, options.include_raw);
    let controls_before = drill_control_state(&visual_json, options.include_raw);
    let mut projections =
        visual_query_json(visual_type, &resolved_bindings)["queryState"]["Category"]["projections"]
            .clone();
    let projection_items = projections.as_array_mut().ok_or_else(|| {
        CliError::validation_failed("generated hierarchy projections are not an array")
    })?;
    if let Some(first) = projection_items.first_mut().and_then(Value::as_object_mut) {
        // Desktop persists the current hierarchy level through the projection's
        // active marker. Start every authored hierarchy at its first level.
        first.insert("active".to_string(), Value::Bool(true));
    }
    replace_category_projections(&mut visual_json, visual_type, projections)?;
    enable_drill_controls(&mut visual_json)?;
    visual_json["howCreated"] = Value::String("DraggedToFieldWell".to_string());
    set_binding_status_annotation(&mut visual_json, "bound");
    let after = category_state(&visual_json, options.include_raw);
    let controls_after = drill_control_state(&visual_json, options.include_raw);
    let fields = resolved_bindings
        .iter()
        .map(|binding| {
            json!({
                "table": binding.table,
                "column": binding.field,
                "field": format!("{}[{}]", binding.table, binding.field),
                "queryRef": format!("{}.{}", binding.table, binding.field)
            })
        })
        .collect::<Vec<_>>();
    let bindings = resolved_bindings
        .iter()
        .map(binding_summary)
        .collect::<Vec<_>>();

    Ok(HierarchyPlan {
        visual_json,
        before,
        after,
        controls_before,
        controls_after,
        fields,
        bindings,
    })
}

fn replace_category_projections(
    visual_json: &mut Value,
    visual_type: &str,
    projections: Value,
) -> CliResult<()> {
    let visual = visual_json["visual"].as_object_mut().ok_or_else(|| {
        CliError::validation_failed("visual.json has no visual object")
            .with_hint("Run `validate --strict` before mutating this report.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })?;
    let query = visual
        .entry("query".to_string())
        .or_insert_with(|| json!({ "queryState": {} }));
    let query_object = query
        .as_object_mut()
        .ok_or_else(|| CliError::validation_failed("visual.query is not an object"))?;
    let query_state = query_object
        .entry("queryState".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let query_state_object = query_state
        .as_object_mut()
        .ok_or_else(|| CliError::validation_failed("visual.query.queryState is not an object"))?;
    let has_role = |role: &str| {
        query_state_object
            .get(role)
            .and_then(|value| value["projections"].as_array())
            .is_some_and(|items| !items.is_empty())
    };
    let has_required_values = match visual_type {
        "lineClusteredColumnComboChart" | "lineStackedColumnComboChart" => {
            has_role("Y") || has_role("Y2")
        }
        _ => has_role("Y"),
    };
    if !has_required_values {
        return Err(CliError::invalid_args(
            "report drilldown set-hierarchy requires the chart's existing numeric bindings",
        )
        .with_hint("Bind the chart first, then set the hierarchy. Scatter charts require X and Y; combo charts require Y or Y2; other category charts require Y.")
        .with_suggested_command(
            "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle <visual-handle> --binding \"role=Category,table=<table>,column=<column>\" --binding \"role=Y,table=<table>,measure=<measure>\" --dry-run --json",
        ));
    }
    query_state_object.insert(
        "Category".to_string(),
        json!({ "projections": projections }),
    );
    Ok(())
}

const DRILL_CONTROL_PROPERTIES: &[(&str, &str)] = &[
    ("show", "visible"),
    ("showDrillRoleSelector", "roleSelector"),
    ("showDrillUpButton", "drillUp"),
    ("showDrillToggleButton", "drillMode"),
    ("showDrillDownLevelButton", "nextLevel"),
    ("showDrillDownExpandButton", "expandLevel"),
];

fn enable_drill_controls(visual_json: &mut Value) -> CliResult<()> {
    let visual = visual_json["visual"].as_object_mut().ok_or_else(|| {
        CliError::validation_failed("visual.json has no visual object")
            .with_hint("Run `validate --strict` before mutating this report.")
            .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })?;
    let container_objects = visual
        .entry("visualContainerObjects".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            CliError::validation_failed("visual.visualContainerObjects is not an object")
        })?;
    let header = container_objects
        .entry("visualHeader".to_string())
        .or_insert_with(|| Value::Array(vec![json!({ "properties": {} })]))
        .as_array_mut()
        .ok_or_else(|| CliError::validation_failed("visualHeader formatting is not an array"))?;
    if header.is_empty() {
        header.push(json!({ "properties": {} }));
    }
    let card = header[0].as_object_mut().ok_or_else(|| {
        CliError::validation_failed("visualHeader formatting card is not an object")
    })?;
    let properties = card
        .entry("properties".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| CliError::validation_failed("visualHeader properties is not an object"))?;
    for (property, _) in DRILL_CONTROL_PROPERTIES {
        properties.insert(
            (*property).to_string(),
            json!({ "expr": { "Literal": { "Value": "true" } } }),
        );
    }
    Ok(())
}

fn drill_control_state(visual_json: &Value, include_raw: bool) -> Value {
    let header = &visual_json["visual"]["visualContainerObjects"]["visualHeader"];
    let properties = &header[0]["properties"];
    let mut summary = Map::new();
    for (property, label) in DRILL_CONTROL_PROPERTIES {
        let value = properties[*property]["expr"]["Literal"]["Value"]
            .as_str()
            .and_then(|value| value.parse::<bool>().ok());
        summary.insert(
            (*label).to_string(),
            value.map(Value::Bool).unwrap_or(Value::Null),
        );
    }
    let mut state = Value::Object(summary);
    if include_raw {
        state["raw"] = header.clone();
    }
    state
}

fn category_state(visual_json: &Value, include_raw: bool) -> Value {
    let category = visual_json["visual"]["query"]["queryState"]["Category"].clone();
    let projections = category["projections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut value = json!({
        "projectionCount": projections.len(),
        "fields": projections.iter().map(|projection| json!({
            "queryRef": projection["queryRef"],
            "nativeQueryRef": projection["nativeQueryRef"],
            "displayName": projection["displayName"]
        })).collect::<Vec<_>>()
    });
    if include_raw {
        value["raw"] = category;
    }
    value
}

fn drilldown_response(
    resolved: &ResolvedProject,
    mode: MutationMode,
    visual_path: &std::path::Path,
    visual: &crate::pbir::VisualRecord,
    plan: &HierarchyPlan,
    include_raw: bool,
) -> CliResult<Value> {
    let dry_run = matches!(mode, MutationMode::DryRun);
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(resolved)?)
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
    let project = command_arg(&resolved.project_dir);
    let readback = format!(
        "powerbi-cli report visuals show --project {project} --handle {} --json",
        shell_arg(&visual.handle)
    );
    let wireframe = format!(
        "powerbi-cli report wireframe export {} --json",
        command_arg(&resolved.project_dir)
    );
    let inspect = format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&resolved.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );
    let mut change = json!({
        "kind": "pbir.visual.bindings",
        "action": "set-drilldown-hierarchy",
        "path": canonical_display(visual_path),
        "jsonPointer": "/visual/query/queryState/Category",
        "before": plan.before,
        "after": plan.after
    });
    if include_raw {
        change["rawIncluded"] = Value::Bool(true);
    }
    let controls_change = json!({
        "kind": "pbir.visual.formatting",
        "action": "enable-drill-controls",
        "path": canonical_display(visual_path),
        "jsonPointer": "/visual/visualContainerObjects/visualHeader",
        "before": plan.controls_before,
        "after": plan.controls_after
    });
    Ok(json!({
        "schema": "powerbi-cli.report.drilldown.hierarchyMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "set-hierarchy",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "target": visual_detail(visual),
        "hierarchyPlan": {
            "fields": plan.fields,
            "bindings": plan.bindings,
            "before": plan.before,
            "after": plan.after,
            "controls": {
                "before": plan.controls_before,
                "after": plan.controls_after
            }
        },
        "changes": [change, controls_change],
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
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, wireframe, inspect, validate]
    }))
}

fn ensure_category_chart(visual_type: &str) -> CliResult<()> {
    if matches!(
        visual_type,
        "lineClusteredColumnComboChart" | "lineStackedColumnComboChart"
    ) {
        return Ok(());
    }
    match binding_family(visual_type)? {
        VisualBindingFamily::CategoryY => Ok(()),
        _ => Err(CliError::invalid_args(format!(
            "drilldown hierarchy is supported for category-axis charts, not {visual_type}"
        ))
        .with_hint(
            "Use a line, area, bar, column, or combo chart with a Category field well. Power BI scatter Category accepts only one projection.",
        )
        .with_suggested_command(
            "powerbi-cli report visuals catalog --visual-type lineChart --json",
        )),
    }
}

fn resolve_column_ref(
    docs: &[crate::tmdl::TableDocument],
    table: &str,
    column: &str,
) -> CliResult<()> {
    let table_doc = docs
        .iter()
        .find(|doc| same_name(&doc.table, table))
        .ok_or_else(|| {
            CliError::validation_failed(format!("table not found for hierarchy field: {table}"))
                .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json")
        })?;
    if table_doc
        .columns
        .iter()
        .any(|candidate| same_name(&candidate.name, column))
    {
        Ok(())
    } else {
        Err(CliError::validation_failed(format!(
            "column not found for hierarchy field: {table}[{column}]"
        ))
        .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json"))
    }
}

fn parse_hierarchy_args(args: &[String]) -> CliResult<HierarchyOptions> {
    let mut options = HierarchyOptions::default();
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
            "--field" | "--target" | "--category" | "--axis" => {
                options.fields.push(take_value(args, &mut i, "--field")?);
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report drilldown set-hierarchy",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report drilldown set-hierarchy",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report drilldown set-hierarchy",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report drilldown set-hierarchy flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report drilldown set-hierarchy\"`.")
                .with_suggested_command(
                    "powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> --handle <visual-handle> --field 'Table[Year]' --field 'Table[Month]' --dry-run --json",
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
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --field 'Table[Year]' --field 'Table[Month]' --dry-run --json"
    )))
}

fn require_hierarchy_fields(options: &HierarchyOptions) -> CliResult<()> {
    if options.fields.len() >= 2 {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "report drilldown set-hierarchy requires at least two --field values",
    )
    .with_hint("Use drill levels from broad to narrow, for example Year then Month.")
    .with_suggested_command(
        "powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> --handle <visual-handle> --field 'DimDate[FiscalYear]' --field 'DimDate[Month]' --dry-run --json",
    ))
}

fn require_drilldown_mode(mode: Option<MutationMode>, command: &str) -> CliResult<MutationMode> {
    mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --dry-run, --in-place, or --out-dir <dir>"
        ))
        .with_hint("Start with `--dry-run`; use `--in-place` or `--out-dir` after reviewing the hierarchy plan.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --field 'Table[Year]' --field 'Table[Month]' --dry-run --json"
        ))
    })
}

fn parse_field_ref(target: &str) -> CliResult<(String, String)> {
    let target = target.trim();
    if let Some((table, rest)) = target.split_once('[')
        && let Some(column) = rest.strip_suffix(']')
        && !table.trim().is_empty()
        && !column.trim().is_empty()
    {
        return Ok((table.trim().to_string(), column.trim().to_string()));
    }
    if let Some((table, column)) = target.split_once('.')
        && !table.trim().is_empty()
        && !column.trim().is_empty()
    {
        return Ok((table.trim().to_string(), column.trim().to_string()));
    }
    Err(CliError::invalid_args(format!(
        "invalid hierarchy field syntax: {target}"
    ))
    .with_hint("Use `Table[Column]` or `Table.Column`.")
    .with_suggested_command(
        "powerbi-cli report drilldown set-hierarchy --project <project-dir-or.pbip> --handle <visual-handle> --field 'DimDate[FiscalYear]' --field 'DimDate[Month]' --dry-run --json",
    ))
}
