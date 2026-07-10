use crate::cli_support::{
    MutationMode, mode_name, required_project, set_mode, shell_arg, take_value, target_project,
};
use crate::feature_catalog::unsupported_feature_error_with_message;
use crate::pbir::{PageRecord, PageSelector, find_page, load_report_snapshot, page_summary};
use crate::pbir_filters::filter_target;
use crate::project_io::write_json_atomic;
use crate::tmdl::{load_table_documents, same_name};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct SetOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    target: Option<String>,
    table: Option<String>,
    column: Option<String>,
    keep_all_filters: Option<bool>,
    keep_visible: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    include_raw: bool,
}

#[derive(Debug, Default)]
struct ClearOptions {
    project: Option<PathBuf>,
    selector: PageSelector,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    confirm: Option<String>,
    restore_visible: bool,
    include_raw: bool,
}

#[derive(Debug)]
struct ResolvedDrillthroughPage {
    page: PageRecord,
    path: PathBuf,
}

#[derive(Debug)]
struct DrillthroughSetPlan {
    page_json: Value,
    before: Value,
    after: Value,
    changes: Vec<Value>,
    binding_name: String,
    parameter_name: String,
    filter_name: String,
}

#[derive(Debug)]
struct DrillthroughClearPlan {
    page_json: Value,
    before: Value,
    after: Value,
    changes: Vec<Value>,
    removed_filters: usize,
}

pub(crate) fn drillthrough_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "report drillthrough requires a subcommand: set, show, clear",
        )
        .with_hint("Use `set` to mark a page as a same-report drillthrough target.")
        .with_suggested_command(
            "powerbi-cli report drillthrough set --project <project-dir-or.pbip> --page <page-handle> --target 'Table[Column]' --dry-run --json",
        ));
    };

    match action.as_str() {
        "set" | "add" | "create" => set_drillthrough(rest),
        "show" | "get" => show_drillthrough(rest),
        "clear" | "remove" | "delete" => clear_drillthrough(rest),
        "visual" | "button" | "action" | "actions" => Err(unsupported_drillthrough_variant(
            "visual drillthrough actions",
        )),
        other => Err(CliError::invalid_args(format!(
            "unknown report drillthrough command: {other}"
        ))
        .with_hint(
            "Run `powerbi-cli --json capabilities --for \"report drillthrough\"` for exact usage.",
        )
        .with_suggested_command("powerbi-cli --json capabilities --for \"report drillthrough\"")),
    }
}

fn set_drillthrough(args: &[String]) -> CliResult<Value> {
    let options = parse_set_args(args)?;
    let source_project = required_project(options.project.clone(), "report drillthrough set")?;
    require_page_selector(&options.selector, "report drillthrough set")?;
    let mode = require_drillthrough_mode(options.mode, "report drillthrough set")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, set_drillthrough)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let page = resolve_drillthrough_page(
        &target_resolved,
        &options.selector,
        "report drillthrough set",
    )?;
    let (table, column) = resolve_target_column(&target_resolved, &options)?;
    let plan = build_set_plan(&page, &table, &column, &options)?;

    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&page.path, &plan.page_json)?;
    }

    set_response(
        &target_resolved,
        mode,
        &page,
        &table,
        &column,
        &plan,
        options.include_raw,
    )
}

fn show_drillthrough(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "report drillthrough show")?;
    require_page_selector(&options.selector, "report drillthrough show")?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let page_record = find_page(
        &snapshot.pages,
        &options.selector,
        "report drillthrough show",
    )?;
    let path = page_path(page_record)?;
    let page_json = read_json_value(&path)?;
    let state = drillthrough_state(&page_json, options.include_raw);
    Ok(json!({
        "schema": "powerbi-cli.report.drillthrough.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "page": page_summary(page_record),
        "drillthrough": state,
        "readbackCommand": drillthrough_show_command(&resolved, &page_record.handle),
        "pageReadbackCommand": page_show_command(&resolved, &page_record.handle),
        "validateCommand": validate_command(&resolved),
        "next": [
            format!("powerbi-cli report drillthrough set --project {} --page {} --target 'Table[Column]' --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&page_record.handle)),
            format!("powerbi-cli report drillthrough clear --project {} --page {} --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&page_record.handle)),
            validate_command(&resolved)
        ],
        "warnings": snapshot.validation.warnings,
        "errors": snapshot.validation.errors
    }))
}

fn clear_drillthrough(args: &[String]) -> CliResult<Value> {
    let options = parse_clear_args(args)?;
    let source_project = required_project(options.project.clone(), "report drillthrough clear")?;
    require_page_selector(&options.selector, "report drillthrough clear")?;
    let mode = require_drillthrough_mode(options.mode, "report drillthrough clear")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, clear_drillthrough)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let page = resolve_drillthrough_page(
        &target_resolved,
        &options.selector,
        "report drillthrough clear",
    )?;
    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&page.page.handle) {
        return Err(CliError::invalid_args(
            "report drillthrough clear --in-place requires --confirm <page-handle>",
        )
        .with_hint("The confirm token must exactly match the target page handle.")
        .with_suggested_command(format!(
            "powerbi-cli report drillthrough clear --project {} --page {} --in-place --confirm {} --json",
            command_arg(&target_resolved.project_dir),
            shell_arg(&page.page.handle),
            shell_arg(&page.page.handle)
        )));
    }

    let plan = build_clear_plan(&page, options.restore_visible, options.include_raw)?;
    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&page.path, &plan.page_json)?;
    }

    clear_response(&target_resolved, mode, &page, &plan, options.include_raw)
}

fn parse_set_args(args: &[String]) -> CliResult<SetOptions> {
    let mut options = SetOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" | "--name" => {
                set_page_selector(&mut options.selector, take_value(args, &mut i, "--page")?);
            }
            "--target" => options.target = Some(take_value(args, &mut i, "--target")?),
            "--table" => options.table = Some(take_value(args, &mut i, "--table")?),
            "--column" => options.column = Some(take_value(args, &mut i, "--column")?),
            "--filter-name" | "--filterName" | "--name-filter" | "--display-name"
            | "--displayName" => {
                return Err(unsupported_drillthrough_variant(
                    "custom drillthrough filter names",
                ));
            }
            "--keep-all-filters" | "--keepAllFilters" => {
                options.keep_all_filters = Some(parse_bool(&take_value(
                    args,
                    &mut i,
                    "--keep-all-filters",
                )?)?);
            }
            "--no-keep-all-filters" | "--noKeepAllFilters" => {
                options.keep_all_filters = Some(false);
                i += 1;
            }
            "--keep-visible" | "--keepVisible" | "--no-hide-page" | "--noHidePage" => {
                options.keep_visible = true;
                i += 1;
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report drillthrough set",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report drillthrough set",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report drillthrough set",
                )?;
                options.out_dir = Some(out_dir);
            }
            "--cross-report" | "--crossReport" => {
                return Err(unsupported_drillthrough_variant(
                    "cross-report drillthrough",
                ));
            }
            "--visual" | "--button" | "--action" | "--source-visual" | "--sourceVisual" => {
                return Err(unsupported_drillthrough_variant(
                    "visual drillthrough actions",
                ));
            }
            other => return Err(unknown_flag("report drillthrough set", other)),
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
            "--page" | "--name" => {
                set_page_selector(&mut options.selector, take_value(args, &mut i, "--page")?);
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--no-raw" | "--noRaw" => {
                options.include_raw = false;
                i += 1;
            }
            other => return Err(unknown_flag("report drillthrough show", other)),
        }
    }
    Ok(options)
}

fn parse_clear_args(args: &[String]) -> CliResult<ClearOptions> {
    let mut options = ClearOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" | "--name" => {
                set_page_selector(&mut options.selector, take_value(args, &mut i, "--page")?);
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--restore-visible" | "--restoreVisible" => {
                options.restore_visible = true;
                i += 1;
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report drillthrough clear",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report drillthrough clear",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report drillthrough clear",
                )?;
                options.out_dir = Some(out_dir);
            }
            "--cross-report" | "--crossReport" => {
                return Err(unsupported_drillthrough_variant(
                    "cross-report drillthrough",
                ));
            }
            "--visual" | "--button" | "--action" | "--source-visual" | "--sourceVisual" => {
                return Err(unsupported_drillthrough_variant(
                    "visual drillthrough actions",
                ));
            }
            other => return Err(unknown_flag("report drillthrough clear", other)),
        }
    }
    Ok(options)
}

fn resolve_drillthrough_page(
    resolved: &ResolvedProject,
    selector: &PageSelector,
    command: &str,
) -> CliResult<ResolvedDrillthroughPage> {
    let snapshot = load_report_snapshot(resolved)?;
    let page = find_page(&snapshot.pages, selector, command)?.clone();
    let path = page_path(&page)?;
    Ok(ResolvedDrillthroughPage { page, path })
}

fn page_path(page: &PageRecord) -> CliResult<PathBuf> {
    page.path.clone().ok_or_else(|| {
        CliError::validation_failed(format!(
            "page {} does not have a page.json path",
            page.handle
        ))
    })
}

fn resolve_target_column(
    resolved: &ResolvedProject,
    options: &SetOptions,
) -> CliResult<(String, String)> {
    let (requested_table, requested_column) = requested_target(options)?;
    let docs = load_table_documents(resolved)?;
    let table = docs
        .iter()
        .find(|doc| same_name(&doc.table, &requested_table))
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "table not found for drillthrough target: {requested_table}"
            ))
            .with_hint("Run `inspect --deep` to discover canonical TMDL table names.")
            .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json")
        })?;
    let column = table
        .columns
        .iter()
        .find(|column| same_name(&column.name, &requested_column))
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "column not found for drillthrough target: {}[{}]",
                table.table, requested_column
            ))
            .with_hint("First-slice drillthrough supports model columns only, not measures or field parameters.")
            .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json")
        })?;
    Ok((table.table.clone(), column.name.clone()))
}

fn requested_target(options: &SetOptions) -> CliResult<(String, String)> {
    if let Some(target) = options.target.as_deref() {
        if options.table.is_some() || options.column.is_some() {
            return Err(CliError::invalid_args(
                "report drillthrough set accepts either --target or --table plus --column, not both",
            )
            .with_hint("Use --target 'Table[Column]' for compact input, or use --table and --column separately.")
            .with_suggested_command("powerbi-cli report drillthrough set --project <project-dir-or.pbip> --page <page-handle> --target 'DimCustomer[Segment]' --dry-run --json"));
        }
        return parse_target(target);
    }
    let table = options.table.clone().ok_or_else(|| {
        CliError::invalid_args("report drillthrough set requires --target or --table plus --column")
            .with_hint("Use a TMDL column, for example `DimDate[Year]`.")
            .with_suggested_command("powerbi-cli report drillthrough set --project <project-dir-or.pbip> --page <page-handle> --target 'DimCustomer[Segment]' --dry-run --json")
    })?;
    let column = options.column.clone().ok_or_else(|| {
        CliError::invalid_args("report drillthrough set requires --column when --table is used")
            .with_hint("Use TMDL column names from `inspect --deep`.")
            .with_suggested_command("powerbi-cli report drillthrough set --project <project-dir-or.pbip> --page <page-handle> --table DimCustomer --column Segment --dry-run --json")
    })?;
    Ok((table, column))
}

fn parse_target(target: &str) -> CliResult<(String, String)> {
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
        "invalid drillthrough target syntax: {target}"
    ))
    .with_hint("Use `Table[Column]` or `Table.Column`.")
    .with_suggested_command(
        "powerbi-cli report drillthrough set --project <project-dir-or.pbip> --page <page-handle> --target 'DimCustomer[Segment]' --dry-run --json",
    ))
}

fn build_set_plan(
    page: &ResolvedDrillthroughPage,
    table: &str,
    column: &str,
    options: &SetOptions,
) -> CliResult<DrillthroughSetPlan> {
    let mut page_json = read_json_value(&page.path)?;
    let before = drillthrough_state(&page_json, options.include_raw);
    let before_type = page_json.get("type").cloned().unwrap_or(Value::Null);
    let before_visibility = page_json.get("visibility").cloned().unwrap_or(Value::Null);
    let before_binding = page_json.get("pageBinding").cloned().unwrap_or(Value::Null);
    let before_filters = drillthrough_filter_summaries(&page_json, options.include_raw);
    let binding_name = generated_name("DrillthroughBinding", &page.page.name, table, column);
    let parameter_name = generated_name("DrillthroughParameter", &page.page.name, table, column);
    let filter_name = generated_name("DrillthroughFilter", &page.page.name, table, column);
    let field_expr = column_expr(table, column);
    let accepts_filter_context = if options.keep_all_filters.unwrap_or(true) {
        "Default"
    } else {
        "None"
    };
    let binding = json!({
        "name": binding_name,
        "type": "Drillthrough",
        "referenceScope": "Default",
        "parameters": [{
            "name": parameter_name,
            "boundFilter": filter_name,
            "fieldExpr": field_expr
        }],
        "acceptsFilterContext": accepts_filter_context
    });
    let drillthrough_filter = json!({
        "name": filter_name,
        "howCreated": "Drillthrough",
        "type": "Categorical",
        "field": field_expr
    });

    let root = page_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} is not a JSON object", page.path.display()))
    })?;
    // The Desktop-authored reference omits this root marker, but Desktop 2.155
    // accepted it and existing CLI output relies on it. The linked pageBinding
    // parameter and Drillthrough filter below are the operative Desktop shape.
    root.insert(
        "type".to_string(),
        Value::String("Drillthrough".to_string()),
    );
    if !options.keep_visible {
        root.insert(
            "visibility".to_string(),
            Value::String("HiddenInViewMode".to_string()),
        );
    }
    root.insert("pageBinding".to_string(), binding);
    replace_drillthrough_filters(root, drillthrough_filter)?;

    let after = drillthrough_state(&page_json, options.include_raw);
    let after_type = page_json.get("type").cloned().unwrap_or(Value::Null);
    let after_visibility = page_json.get("visibility").cloned().unwrap_or(Value::Null);
    let after_binding = page_json.get("pageBinding").cloned().unwrap_or(Value::Null);
    let after_filters = drillthrough_filter_summaries(&page_json, options.include_raw);
    let mut changes = Vec::new();
    push_change(
        &mut changes,
        "set",
        &page.path,
        "/type",
        before_type,
        after_type,
        options.include_raw,
    );
    if !options.keep_visible {
        push_change(
            &mut changes,
            "set",
            &page.path,
            "/visibility",
            before_visibility,
            after_visibility,
            options.include_raw,
        );
    }
    push_change(
        &mut changes,
        "set",
        &page.path,
        "/pageBinding",
        before_binding,
        after_binding,
        options.include_raw,
    );
    push_change(
        &mut changes,
        "set",
        &page.path,
        "/filterConfig/filters",
        before_filters,
        after_filters,
        options.include_raw,
    );

    Ok(DrillthroughSetPlan {
        page_json,
        before,
        after,
        changes,
        binding_name,
        parameter_name,
        filter_name,
    })
}

fn build_clear_plan(
    page: &ResolvedDrillthroughPage,
    restore_visible: bool,
    include_raw: bool,
) -> CliResult<DrillthroughClearPlan> {
    let mut page_json = read_json_value(&page.path)?;
    let before = drillthrough_state(&page_json, include_raw);
    let before_type = page_json.get("type").cloned().unwrap_or(Value::Null);
    let before_visibility = page_json.get("visibility").cloned().unwrap_or(Value::Null);
    let before_binding = page_json.get("pageBinding").cloned().unwrap_or(Value::Null);
    let before_filters = drillthrough_filter_summaries(&page_json, include_raw);
    let root = page_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} is not a JSON object", page.path.display()))
    })?;
    if root.get("type").and_then(Value::as_str) == Some("Drillthrough") {
        root.remove("type");
    }
    if root
        .get("pageBinding")
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        == Some("Drillthrough")
    {
        root.remove("pageBinding");
    }
    if restore_visible && root.get("visibility").and_then(Value::as_str) == Some("HiddenInViewMode")
    {
        root.insert(
            "visibility".to_string(),
            Value::String("AlwaysVisible".to_string()),
        );
    }
    let removed_filters = remove_drillthrough_filters(root)?;
    let after = drillthrough_state(&page_json, include_raw);
    let after_type = page_json.get("type").cloned().unwrap_or(Value::Null);
    let after_visibility = page_json.get("visibility").cloned().unwrap_or(Value::Null);
    let after_binding = page_json.get("pageBinding").cloned().unwrap_or(Value::Null);
    let after_filters = drillthrough_filter_summaries(&page_json, include_raw);
    let mut changes = Vec::new();
    push_change(
        &mut changes,
        "clear",
        &page.path,
        "/type",
        before_type,
        after_type,
        include_raw,
    );
    if restore_visible {
        push_change(
            &mut changes,
            "clear",
            &page.path,
            "/visibility",
            before_visibility,
            after_visibility,
            include_raw,
        );
    }
    push_change(
        &mut changes,
        "clear",
        &page.path,
        "/pageBinding",
        before_binding,
        after_binding,
        include_raw,
    );
    push_change(
        &mut changes,
        "clear",
        &page.path,
        "/filterConfig/filters",
        before_filters,
        after_filters,
        include_raw,
    );
    Ok(DrillthroughClearPlan {
        page_json,
        before,
        after,
        changes,
        removed_filters,
    })
}

fn remove_drillthrough_filters(root: &mut Map<String, Value>) -> CliResult<usize> {
    let Some(filter_config) = root.get_mut("filterConfig") else {
        return Ok(0);
    };
    let filter_config = filter_config
        .as_object_mut()
        .ok_or_else(|| CliError::validation_failed("page filterConfig is not an object"))?;
    let Some(filters) = filter_config.get_mut("filters") else {
        return Ok(0);
    };
    let filters = filters
        .as_array_mut()
        .ok_or_else(|| CliError::validation_failed("page /filterConfig/filters is not an array"))?;
    let before = filters.len();
    filters.retain(|filter| filter["howCreated"].as_str() != Some("Drillthrough"));
    Ok(before - filters.len())
}

fn replace_drillthrough_filters(
    root: &mut Map<String, Value>,
    paired_filter: Value,
) -> CliResult<()> {
    let filter_config = root
        .entry("filterConfig".to_string())
        .or_insert_with(|| json!({ "filters": [] }));
    let filter_config = filter_config
        .as_object_mut()
        .ok_or_else(|| CliError::validation_failed("page filterConfig is not an object"))?;
    let filters = filter_config
        .entry("filters".to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| CliError::validation_failed("page /filterConfig/filters is not an array"))?;
    filters.retain(|filter| filter["howCreated"].as_str() != Some("Drillthrough"));
    filters.push(paired_filter);
    Ok(())
}

fn drillthrough_state(page_json: &Value, include_raw: bool) -> Value {
    let page_binding = page_json.get("pageBinding").cloned().unwrap_or(Value::Null);
    let binding_summary = page_binding_summary(&page_binding, include_raw);
    let filters = drillthrough_filter_summaries(page_json, include_raw);
    let page_type = page_json.get("type").cloned().unwrap_or(Value::Null);
    let visibility = page_json.get("visibility").cloned().unwrap_or(Value::Null);
    let enabled = page_type.as_str() == Some("Drillthrough")
        || page_binding["type"].as_str() == Some("Drillthrough")
        || filters.as_array().is_some_and(|items| !items.is_empty());
    json!({
        "enabled": enabled,
        "pageType": page_type,
        "visibility": visibility,
        "binding": binding_summary,
        "filters": filters
    })
}

fn page_binding_summary(binding: &Value, include_raw: bool) -> Value {
    if !binding.is_object() {
        return Value::Null;
    }
    let parameters = binding["parameters"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|parameter| {
                    let mut summary = json!({
                        "name": parameter["name"],
                        "boundFilter": parameter["boundFilter"],
                        "fieldExpr": parameter["fieldExpr"],
                        "asAggregation": parameter["asAggregation"],
                        "qnaSingleSelectRequired": parameter["qnaSingleSelectRequired"],
                        "target": filter_target(&parameter["fieldExpr"])
                    });
                    if include_raw {
                        summary["raw"] = parameter.clone();
                    }
                    summary
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut summary = json!({
        "name": binding["name"],
        "type": binding["type"],
        "referenceScope": binding["referenceScope"],
        "acceptsFilterContext": binding["acceptsFilterContext"],
        "parameters": parameters
    });
    if include_raw {
        summary["raw"] = binding.clone();
    }
    summary
}

fn drillthrough_filter_summaries(page_json: &Value, include_raw: bool) -> Value {
    let filters = page_json["filterConfig"]["filters"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .enumerate()
                .filter(|(_, filter)| filter["howCreated"].as_str() == Some("Drillthrough"))
                .map(|(ordinal, filter)| {
                    let mut summary = json!({
                        "ordinal": ordinal,
                        "name": filter["name"],
                        "displayName": filter["displayName"],
                        "filterType": filter["type"],
                        "howCreated": filter["howCreated"],
                        "target": filter_target(filter),
                        "hasPersistedFilterDefinition": filter.get("filter").is_some()
                    });
                    if include_raw {
                        summary["raw"] = filter.clone();
                    }
                    summary
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(filters)
}

fn set_response(
    resolved: &ResolvedProject,
    mode: MutationMode,
    page: &ResolvedDrillthroughPage,
    table: &str,
    column: &str,
    plan: &DrillthroughSetPlan,
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
    let readback = (!dry_run).then(|| drillthrough_show_command(resolved, &page.page.handle));
    Ok(json!({
        "schema": "powerbi-cli.report.drillthrough.setMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "set",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "page": page_summary(&page.page),
        "target": {
            "kind": "column",
            "table": table,
            "column": column,
            "field": column
        },
        "drillthroughPlan": {
            "before": plan.before,
            "after": plan.after,
            "bindingName": plan.binding_name,
            "parameterName": plan.parameter_name,
            "filterName": plan.filter_name,
            "rawAfterIncluded": include_raw
        },
        "changes": plan.changes,
        "safety": {
            "dataValueRisk": "none-detected",
            "mayContainDataValues": false,
            "message": "Drillthrough field metadata stores table and column names, not selected data values."
        },
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
        "readbackCommand": readback.clone(),
        "pageReadbackCommand": page_show_command(resolved, &page.page.handle),
        "filterReadbackCommand": format!("powerbi-cli report filters list --project {} --scope page --page {} --json", command_arg(&resolved.project_dir), shell_arg(&page.page.handle)),
        "wireframeCommand": format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir)),
        "inspectCommand": format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
        "validateCommand": validate_command(resolved),
        "next": [
            readback.unwrap_or_else(|| format!("powerbi-cli report drillthrough show --project {} --page {} --json", command_arg(&resolved.project_dir), shell_arg(&page.page.handle))),
            page_show_command(resolved, &page.page.handle),
            validate_command(resolved)
        ]
    }))
}

fn clear_response(
    resolved: &ResolvedProject,
    mode: MutationMode,
    page: &ResolvedDrillthroughPage,
    plan: &DrillthroughClearPlan,
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
    let readback = (!dry_run).then(|| drillthrough_show_command(resolved, &page.page.handle));
    Ok(json!({
        "schema": "powerbi-cli.report.drillthrough.clearMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "clear",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "page": page_summary(&page.page),
        "drillthroughPlan": {
            "before": plan.before,
            "after": plan.after,
            "removedFilters": plan.removed_filters,
            "rawAfterIncluded": include_raw
        },
        "changes": plan.changes,
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
        "readbackCommand": readback.clone(),
        "pageReadbackCommand": page_show_command(resolved, &page.page.handle),
        "filterReadbackCommand": format!("powerbi-cli report filters list --project {} --scope page --page {} --json", command_arg(&resolved.project_dir), shell_arg(&page.page.handle)),
        "wireframeCommand": format!("powerbi-cli report wireframe export {} --json", command_arg(&resolved.project_dir)),
        "inspectCommand": format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
        "validateCommand": validate_command(resolved),
        "next": [
            readback.unwrap_or_else(|| format!("powerbi-cli report drillthrough show --project {} --page {} --json", command_arg(&resolved.project_dir), shell_arg(&page.page.handle))),
            page_show_command(resolved, &page.page.handle),
            validate_command(resolved)
        ]
    }))
}

fn push_change(
    changes: &mut Vec<Value>,
    action: &str,
    path: &Path,
    pointer: &str,
    before: Value,
    after: Value,
    include_raw: bool,
) {
    if before == after {
        return;
    }
    changes.push(json!({
        "kind": "pbir.drillthrough",
        "action": action,
        "path": canonical_display(path),
        "jsonPointer": pointer,
        "before": if include_raw { before.clone() } else { concise_value(before) },
        "after": if include_raw { after.clone() } else { concise_value(after) }
    }));
}

fn concise_value(value: Value) -> Value {
    value
}

fn column_expr(table: &str, column: &str) -> Value {
    json!({
        "Column": {
            "Expression": { "SourceRef": { "Entity": table } },
            "Property": column
        }
    })
}

fn generated_name(prefix: &str, page: &str, table: &str, column: &str) -> String {
    let seed = format!("{page}|{table}|{column}");
    format!("{prefix}_{}", fingerprint_hex(&seed, 12))
}

fn fingerprint_hex(text: &str, chars: usize) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let full = format!("{hash:016x}");
    full.chars().take(chars).collect()
}

fn set_page_selector(selector: &mut PageSelector, value: String) {
    if value.starts_with("page:") {
        selector.handle = Some(value);
    } else {
        selector.name = Some(value);
    }
}

fn require_page_selector(selector: &PageSelector, command: &str) -> CliResult<()> {
    if selector.handle.is_some() || selector.name.is_some() {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{command} requires --page <page-name-or-handle> or --handle <page-handle>"
    ))
    .with_hint("Use `report pages list` to get stable page handles.")
    .with_suggested_command(format!(
        "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-handle> --json"
    )))
}

fn require_drillthrough_mode(mode: Option<MutationMode>, command: &str) -> CliResult<MutationMode> {
    mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --dry-run, --in-place, or --out-dir <dir>"
        ))
        .with_hint("Start with `--dry-run`; use `--out-dir` or confirmed `--in-place` after review.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-handle> --target 'Table[Column]' --dry-run --json"
        ))
    })
}

fn parse_bool(value: &str) -> CliResult<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(CliError::invalid_args(format!(
            "expected boolean true/false, got {value}"
        ))),
    }
}

fn unknown_flag(command: &str, flag: &str) -> CliError {
    CliError::invalid_args(format!("unknown {command} flag: {flag}"))
        .with_hint(
            "Run `powerbi-cli --json capabilities --for \"report drillthrough\"` for exact usage.",
        )
        .with_suggested_command("powerbi-cli --json capabilities --for \"report drillthrough\"")
}

fn unsupported_drillthrough_variant(kind: &str) -> CliError {
    unsupported_feature_error_with_message(
        "report.drillthrough",
        format!(
            "{kind} is not implemented; first-slice support is same-report page drillthrough by one model column"
        ),
    )
}

fn drillthrough_show_command(resolved: &ResolvedProject, page_handle: &str) -> String {
    format!(
        "powerbi-cli report drillthrough show --project {} --page {} --json",
        command_arg(&resolved.project_dir),
        shell_arg(page_handle)
    )
}

fn page_show_command(resolved: &ResolvedProject, page_handle: &str) -> String {
    format!(
        "powerbi-cli report pages show --project {} --handle {} --json",
        command_arg(&resolved.project_dir),
        shell_arg(page_handle)
    )
}

fn validate_command(resolved: &ResolvedProject) -> String {
    format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    )
}
