use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::pbir::{VisualRecord, VisualSelector, find_visual, load_report_snapshot, visual_detail};
use crate::pbir_filters::FilterScope;
use crate::project_io::write_json_atomic;
use crate::report_filter_shapes::{
    FilterSpec, ResolvedFilterColumn, TopNDirection, generated_filter_name, parse_field_reference,
    resolve_filter_column, resolve_filter_measure, validate_filter_name,
};
use crate::tmdl::same_name;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::PathBuf;

const COMMAND: &str = "report visuals set-topn-guard";

#[derive(Debug, Default)]
struct GuardOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    field: Option<String>,
    order_by: Option<String>,
    top: Option<u64>,
    direction: Option<TopNDirection>,
    display_name: Option<String>,
    name: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

struct GuardPlan {
    visual_json: Value,
    action: &'static str,
    name: String,
    display_name: String,
    json_pointer: String,
    before: Value,
    after: Value,
}

pub(crate) fn set_topn_guard(args: &[String]) -> CliResult<Value> {
    let options = parse_guard_args(args)?;
    let source_project = required_project(options.project.clone(), COMMAND)?;
    require_visual_selector(&options.selector)?;
    let field = options.field.as_deref().ok_or_else(|| {
        CliError::invalid_args(format!("{COMMAND} requires --field <Table.Column>"))
            .with_hint("Use the axis column the TopN guard should keep, for example DimCustomer.CustomerName.")
            .with_suggested_command(suggested_command())
    })?;
    let order_by = options.order_by.as_deref().ok_or_else(|| {
        CliError::invalid_args(format!("{COMMAND} requires --order-by <Table.Measure>"))
            .with_hint("Use the ranking measure, for example FactSales[Total Revenue].")
            .with_suggested_command(suggested_command())
    })?;
    let top = options.top.ok_or_else(|| {
        CliError::invalid_args(format!("{COMMAND} requires --top <N>"))
            .with_hint("Pass a positive integer row cap.")
            .with_suggested_command(suggested_command())
    })?;
    if let Some(name) = options.name.as_deref() {
        validate_filter_name(name)?;
    }
    let mode = require_mode(options.mode, COMMAND)?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, set_topn_guard)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(&snapshot.pages, &options.selector, COMMAND)?;
    let visual_path = visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!("visual has no path: {}", visual.handle))
    })?;

    let (table, column) = parse_field_reference(field)?;
    let column = resolve_filter_column(&target_resolved, &table, &column)?;
    let by = resolve_filter_measure(&target_resolved, order_by)?;
    let direction = options.direction.unwrap_or(TopNDirection::Top);
    let spec = FilterSpec::TopN {
        direction,
        count: top,
        by: by.clone(),
    };
    spec.validate_for(&column, FilterScope::Visual)?;
    let display_name = options
        .display_name
        .clone()
        .unwrap_or_else(|| format!("Top {top}"));
    let plan = apply_guard(
        visual_path,
        &column,
        &spec,
        options.name.as_deref(),
        &display_name,
    )?;

    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(visual_path, &plan.visual_json)?;
    }

    guard_response(
        &target_resolved,
        mode,
        visual,
        visual_path,
        &column,
        &spec,
        &plan,
    )
}

fn parse_guard_args(args: &[String]) -> CliResult<GuardOptions> {
    let mut options = GuardOptions::default();
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
            "--field" => set_once(
                &mut options.field,
                take_value(args, &mut i, "--field")?,
                "--field",
            )?,
            "--order-by" | "--orderBy" => {
                set_once(
                    &mut options.order_by,
                    take_value(args, &mut i, "--order-by")?,
                    "--order-by",
                )?;
            }
            "--top" => set_once(
                &mut options.top,
                take_positive_u64(args, &mut i, "--top")?,
                "--top",
            )?,
            "--direction" => {
                let value = parse_direction(&take_value(args, &mut i, "--direction")?)?;
                set_once(&mut options.direction, value, "--direction")?;
            }
            "--display-name" | "--displayName" => {
                set_once(
                    &mut options.display_name,
                    take_value(args, &mut i, "--display-name")?,
                    "--display-name",
                )?;
            }
            "--name" => set_once(
                &mut options.name,
                take_value(args, &mut i, "--name")?,
                "--name",
            )?,
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun, COMMAND)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace, COMMAND)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir, COMMAND)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown {COMMAND} flag: {other}"
                ))
                .with_hint(format!(
                    "Run `powerbi-cli --json capabilities --for \"{COMMAND}\"` for exact flags."
                ))
                .with_suggested_command(format!(
                    "powerbi-cli --json capabilities --for \"{COMMAND}\""
                )));
            }
        }
    }
    Ok(options)
}

fn apply_guard(
    visual_path: &std::path::Path,
    column: &ResolvedFilterColumn,
    spec: &FilterSpec,
    requested_name: Option<&str>,
    display_name: &str,
) -> CliResult<GuardPlan> {
    let mut visual_json = read_json_value(visual_path)?;
    let filters = ensure_visual_filters(&mut visual_json, visual_path)?;
    let match_index = find_existing_guard(filters, column, requested_name)?;
    let (action, name, before, ordinal) = match match_index {
        Some(index) => {
            let existing = &filters[index];
            let name = existing["name"]
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| generated_filter_name(FilterScope::Visual, column, spec));
            ("update", name, guard_summary(existing), index)
        }
        None => {
            let name = requested_name
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| generated_filter_name(FilterScope::Visual, column, spec));
            validate_filter_name(&name)?;
            if filters.iter().any(|existing| {
                existing["name"]
                    .as_str()
                    .is_some_and(|existing_name| existing_name.eq_ignore_ascii_case(&name))
            }) {
                return Err(CliError::invalid_args(format!(
                    "filter name already exists for this visual: {name}"
                ))
                .with_hint(
                    "Pass a unique --name or update the existing TopN guard on the same field.",
                )
                .with_suggested_command(suggested_command()));
            }
            ("create", name, Value::Null, filters.len())
        }
    };
    let filter = spec.to_pbir(&name, Some(display_name), column)?;
    let after = guard_summary(&filter);
    if match_index.is_some() {
        filters[ordinal] = filter;
    } else {
        filters.push(filter);
    }
    Ok(GuardPlan {
        visual_json,
        action,
        name,
        display_name: display_name.to_string(),
        json_pointer: format!("/filterConfig/filters/{ordinal}"),
        before,
        after,
    })
}

fn ensure_visual_filters<'a>(
    visual_json: &'a mut Value,
    visual_path: &std::path::Path,
) -> CliResult<&'a mut Vec<Value>> {
    let root = visual_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} is not a JSON object", visual_path.display()))
    })?;
    let filter_config = root
        .entry("filterConfig")
        .or_insert_with(|| json!({ "filters": [] }));
    if !filter_config.is_object() {
        return Err(CliError::validation_failed(format!(
            "{} filterConfig is not an object",
            visual_path.display()
        )));
    }
    let filter_config = filter_config.as_object_mut().expect("checked object");
    let filters = filter_config
        .entry("filters")
        .or_insert_with(|| Value::Array(Vec::new()));
    filters.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "{} /filterConfig/filters is not an array",
            visual_path.display()
        ))
    })
}

fn find_existing_guard(
    filters: &[Value],
    column: &ResolvedFilterColumn,
    requested_name: Option<&str>,
) -> CliResult<Option<usize>> {
    if let Some(name) = requested_name {
        let matches = filters
            .iter()
            .enumerate()
            .filter(|(_, filter)| {
                filter["name"]
                    .as_str()
                    .is_some_and(|existing| existing.eq_ignore_ascii_case(name))
            })
            .collect::<Vec<_>>();
        return match matches.as_slice() {
            [] => Ok(None),
            [(index, filter)] => {
                if filter["type"].as_str() != Some("TopN") {
                    return Err(CliError::invalid_args(format!(
                        "filter --name {name} exists but is not a TopN guard"
                    ))
                    .with_hint("Choose a new --name, or delete the existing non-TopN filter first.")
                    .with_suggested_command(
                        "powerbi-cli report filters list --project <project-dir-or.pbip> --scope visual --visual <visual-handle> --json",
                    ));
                }
                Ok(Some(*index))
            }
            _ => Err(CliError::invalid_args(format!(
                "filter --name {name} matched multiple filters on this visual"
            ))),
        };
    }

    let matches = filters
        .iter()
        .enumerate()
        .filter(|(_, filter)| {
            filter["type"].as_str() == Some("TopN") && field_matches(filter, column)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [(index, _)] => Ok(Some(*index)),
        _ => Err(CliError::invalid_args(
            "multiple TopN guards exist on the same field; pass --name to choose one",
        )
        .with_hint("Use `report filters list --scope visual` and pass the exact filter --name.")
        .with_suggested_command(
            "powerbi-cli report filters list --project <project-dir-or.pbip> --scope visual --visual <visual-handle> --json",
        )),
    }
}

fn field_matches(filter: &Value, column: &ResolvedFilterColumn) -> bool {
    let table = filter
        .pointer("/field/Column/Expression/SourceRef/Entity")
        .and_then(Value::as_str);
    let property = filter
        .pointer("/field/Column/Property")
        .and_then(Value::as_str);
    matches!(
        (table, property),
        (Some(table), Some(property))
            if same_name(table, &column.table) && same_name(property, &column.column)
    )
}

fn guard_summary(filter: &Value) -> Value {
    let query = filter.pointer("/filter/From/0/Expression/Subquery/Query");
    let top = query
        .and_then(|query| query.get("Top"))
        .cloned()
        .unwrap_or(Value::Null);
    let direction = match query.and_then(|query| query.pointer("/OrderBy/0/Direction")) {
        Some(Value::Number(number)) if number.as_u64() == Some(1) => "asc",
        _ => "desc",
    };
    let measure = query
        .and_then(|query| query.pointer("/OrderBy/0/Expression/Measure/Property"))
        .and_then(Value::as_str);
    let source = query
        .and_then(|query| {
            query.pointer("/OrderBy/0/Expression/Measure/Expression/SourceRef/Source")
        })
        .and_then(Value::as_str);
    let table = query.and_then(|query| {
        query["From"].as_array().and_then(|from| {
            from.iter().find_map(|entry| {
                (entry["Name"].as_str() == source).then(|| entry["Entity"].as_str())?
            })
        })
    });
    let order_by = match (table, measure) {
        (Some(table), Some(measure)) => format!("{table}[{measure}]"),
        _ => String::new(),
    };
    json!({
        "top": top,
        "orderBy": order_by,
        "direction": direction
    })
}

fn guard_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    visual: &VisualRecord,
    visual_path: &std::path::Path,
    column: &ResolvedFilterColumn,
    spec: &FilterSpec,
    plan: &GuardPlan,
) -> CliResult<Value> {
    let dry_run = matches!(mode, MutationMode::DryRun);
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(target_resolved)?)
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
    let project_arg = command_arg(&target_resolved.project_dir);
    let readback = format!(
        "powerbi-cli report filters list --project {project_arg} --scope visual --visual {} --json",
        shell_arg(&visual.handle)
    );
    let visual_readback = format!(
        "powerbi-cli report visuals show --project {project_arg} --handle {} --json",
        shell_arg(&visual.handle)
    );
    let wireframe = format!("powerbi-cli report wireframe export {project_arg} --json");
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let FilterSpec::TopN {
        direction,
        count,
        by,
    } = spec
    else {
        unreachable!("set-topn-guard only emits TopN specs");
    };

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.topnGuardMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": plan.action,
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": visual_detail(visual),
        "guard": {
            "name": plan.name,
            "displayName": plan.display_name,
            "field": format!("{}[{}]", column.table, column.column),
            "orderBy": format!("{}[{}]", by.table, by.measure),
            "top": count,
            "direction": direction_name(*direction),
            "jsonPointer": plan.json_pointer
        },
        "changes": [{
            "kind": "pbir.visual.topnGuard",
            "action": plan.action,
            "path": canonical_display(visual_path),
            "jsonPointer": plan.json_pointer,
            "before": plan.before,
            "after": plan.after
        }],
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
        "visualReadbackCommand": visual_readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, visual_readback, wireframe, inspect, validate]
    }))
}

fn parse_direction(value: &str) -> CliResult<TopNDirection> {
    match value.to_ascii_lowercase().as_str() {
        "desc" | "descending" | "top" => Ok(TopNDirection::Top),
        "asc" | "ascending" | "bottom" => Ok(TopNDirection::Bottom),
        other => Err(
            CliError::invalid_args(format!("invalid --direction {other}"))
                .with_hint("Use --direction desc or --direction asc.")
                .with_suggested_command(suggested_command()),
        ),
    }
}

fn direction_name(direction: TopNDirection) -> &'static str {
    match direction {
        TopNDirection::Top => "desc",
        TopNDirection::Bottom => "asc",
    }
}

fn take_positive_u64(args: &[String], index: &mut usize, flag: &str) -> CliResult<u64> {
    let raw = take_value(args, index, flag)?;
    let value = raw
        .parse::<u64>()
        .map_err(|_| CliError::invalid_args(format!("{flag} must be a positive whole number")))?;
    if value == 0 || value > i64::MAX as u64 {
        return Err(CliError::invalid_args(format!(
            "{flag} must be between 1 and {}",
            i64::MAX
        )));
    }
    Ok(value)
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> CliResult<()> {
    if slot.is_some() {
        return Err(CliError::invalid_args(format!(
            "{flag} may be passed only once"
        )));
    }
    *slot = Some(value);
    Ok(())
}

fn require_visual_selector(selector: &VisualSelector) -> CliResult<()> {
    if selector.handle.is_some() || (selector.page.is_some() && selector.visual.is_some()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{COMMAND} requires --handle or --page plus --visual"
    ))
    .with_hint("Use `report visuals list` to get stable visual handles.")
    .with_suggested_command(suggested_command()))
}

fn suggested_command() -> String {
    format!(
        "powerbi-cli {COMMAND} --project <project-dir-or.pbip> --handle <visual-handle> --field <Table.Column> --order-by <Table.Measure> --top <N> --dry-run --json"
    )
}
