use crate::cli_support::{
    MutationMode, mode_name, preflight_out_dir, required_project, set_mode, take_value,
    target_project,
};
use crate::project_io::write_text_atomic_validated;
use crate::tmdl::{
    ColumnDefinition, ColumnRecord, ColumnSelector, MutationPlan, add_column_plan,
    column_selector_parts, delete_column_plan, find_column, find_column_by_selector,
    load_table_documents, replace_column_plan, same_name, set_column_sort_by_plan,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default)]
struct SetSortByOptions {
    project: Option<PathBuf>,
    table: Option<String>,
    column: Option<String>,
    by: Option<String>,
    clear: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn columns_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model columns requires a subcommand: list, show, add, update, delete, set-sort-by",
        )
        .with_hint("Use model columns list/show for readback or add/update/delete for guarded TMDL column CRUD.")
        .with_suggested_command(
            "powerbi-cli model columns list --project <project-dir-or.pbip> --json",
        ));
    };
    match action.as_str() {
        "list" => list_columns(rest),
        "show" => show_column(rest),
        "add" => mutate_column(ColumnAction::Add, rest),
        "update" => mutate_column(ColumnAction::Update, rest),
        "delete" => mutate_column(ColumnAction::Delete, rest),
        "set-sort-by" | "setSortBy" => set_sort_by(rest),
        other => Err(CliError::invalid_args(format!(
            "unknown model columns command: {other}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"model columns\"` for exact usage.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"model columns\"")),
    }
}

fn set_sort_by(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    let project = required_project(options.project.clone(), "model columns set-sort-by")?;
    let table = options.table.as_deref().ok_or_else(|| {
        CliError::invalid_args("model columns set-sort-by requires --table <table>")
            .with_suggested_command(example_command())
    })?;
    let column = options.column.as_deref().ok_or_else(|| {
        CliError::invalid_args("model columns set-sort-by requires --column <column>")
            .with_suggested_command(example_command())
    })?;
    if options.by.is_some() == options.clear {
        return Err(CliError::invalid_args(
            "model columns set-sort-by requires exactly one of --by <sort-column> or --clear",
        )
        .with_hint("Use --by to set sortByColumn, or --clear to remove the property.")
        .with_suggested_command(example_command()));
    }
    let mode = options.mode.ok_or_else(|| {
        CliError::invalid_args(
            "model columns set-sort-by requires --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with --dry-run and inspect the exact TMDL block change.")
        .with_suggested_command(example_command())
    })?;

    let source_resolved = resolve_project(&project)?;
    preflight_out_dir(args, set_sort_by)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let docs = load_table_documents(&target_resolved)?;
    let target = find_column(&docs, table, column)?;
    let canonical_sort_by = if let Some(by) = options.by.as_deref() {
        let sort = find_column(&docs, &target.table, by)?;
        if target.name.eq_ignore_ascii_case(&sort.name) {
            return Err(CliError::invalid_args(format!(
                "column {} cannot sort by itself",
                target.handle()
            ))
            .with_hint("Choose a different column in the same table or use --clear.")
            .with_suggested_command(example_command()));
        }
        Some(sort.name.clone())
    } else {
        None
    };
    let previous_sort_by = target.sort_by_column.clone();
    let target_table = target.table.clone();
    let target_name = target.name.clone();
    let plan = set_column_sort_by_plan(
        &docs,
        &target_table,
        &target_name,
        canonical_sort_by.as_deref(),
    )?;
    let dry_run = mode == MutationMode::DryRun;
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        let (validation, modified) = write_text_atomic_validated(
            &plan.path,
            &plan.new_text,
            || validate_project(&target_resolved),
            |report| report.errors.is_empty(),
        )?;
        (Some(validation), modified)
    };
    let validation_ok = validation
        .as_ref()
        .is_none_or(|report| report.errors.is_empty());
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&target_resolved.project_dir);
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");

    Ok(json!({
        "schema": "powerbi-cli.model.columns.setSortBy.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": if options.clear { "clear" } else { "set" },
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({
            "performed": true,
            "projectModified": false,
            "reason": "post-mutation validation failed; the original TMDL file was restored"
        })),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "semanticModelDir": canonical_display(&target_resolved.semantic_model_dir),
        "target": {
            "handle": plan.handle,
            "table": target_table,
            "column": target_name,
            "sortByColumn": canonical_sort_by,
            "previousSortByColumn": previous_sort_by,
            "path": canonical_display(&plan.path)
        },
        "changes": [{
            "kind": "tmdl.column.sortByColumn",
            "action": if options.clear { "remove" } else { "set" },
            "path": canonical_display(&plan.path),
            "before": plan.before_block,
            "after": plan.after_block
        }],
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors
        })),
        "readbackCommand": inspect,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [inspect, validate]
    }))
}

fn parse_args(args: &[String]) -> CliResult<SetSortByOptions> {
    let mut options = SetSortByOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut index, "--project")?));
            }
            "--table" => options.table = Some(take_value(args, &mut index, "--table")?),
            "--column" => options.column = Some(take_value(args, &mut index, "--column")?),
            "--by" => options.by = Some(take_value(args, &mut index, "--by")?),
            "--clear" => {
                options.clear = true;
                index += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "model columns set-sort-by",
                )?;
                index += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "model columns set-sort-by",
                )?;
                index += 1;
            }
            "--out-dir" | "--out" => {
                options.out_dir = Some(PathBuf::from(take_value(args, &mut index, "--out-dir")?));
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "model columns set-sort-by",
                )?;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model columns set-sort-by flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"model columns\"` for exact usage.")
                .with_suggested_command(example_command()));
            }
        }
    }
    Ok(options)
}

fn example_command() -> &'static str {
    "powerbi-cli model columns set-sort-by --project <project-dir-or.pbip> --table <table> --column <column> --by <sort-column> --dry-run --json"
}

#[derive(Debug, Default)]
struct ColumnListOptions {
    project: Option<PathBuf>,
    table: Option<String>,
}

#[derive(Debug, Default)]
struct ColumnShowOptions {
    project: Option<PathBuf>,
    selector: ColumnSelector,
}

#[derive(Debug, Clone, Copy)]
enum ColumnAction {
    Add,
    Update,
    Delete,
}

impl ColumnAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Default)]
struct ColumnMutationOptions {
    project: Option<PathBuf>,
    selector: ColumnSelector,
    expression: Option<String>,
    data_type: Option<String>,
    format_string: Option<String>,
    summarize_by: Option<String>,
    sort_by_column: Option<String>,
    clear_sort_by: bool,
    source_column: Option<String>,
    display_folder: Option<String>,
    description: Option<String>,
    is_hidden: Option<bool>,
    is_key: Option<bool>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    confirm: Option<String>,
}

fn list_columns(args: &[String]) -> CliResult<Value> {
    let options = parse_column_list_args(args)?;
    let project = required_project(options.project, "model columns list")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let mut columns = docs
        .iter()
        .filter(|doc| {
            options
                .table
                .as_deref()
                .is_none_or(|table| same_name(table, &doc.table))
        })
        .flat_map(|doc| doc.columns.iter().map(column_json))
        .collect::<Vec<_>>();
    columns.sort_by(|left, right| {
        left["handle"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["handle"].as_str().unwrap_or_default())
    });

    let next_show = format!(
        "powerbi-cli model columns show --project {} --handle <column-handle> --json",
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
    Ok(json!({
        "schema": "powerbi-cli.model.columns.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "filter": {"table": options.table},
        "counts": {"tables": docs.len(), "columns": columns.len()},
        "columns": columns,
        "next": [next_show, inspect, validate]
    }))
}

fn show_column(args: &[String]) -> CliResult<Value> {
    let options = parse_column_show_args(args)?;
    let project = required_project(options.project, "model columns show")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let record = find_column_by_selector(&docs, &options.selector)?;
    let handle = record.handle();
    let update = format!(
        "powerbi-cli model columns update --project {} --handle {} --data-type <type> --dry-run --json",
        command_arg(&resolved.project_dir),
        crate::cli_support::shell_arg(&handle)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );
    Ok(json!({
        "schema": "powerbi-cli.model.columns.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "column": column_json(record),
        "block": record.block,
        "next": [update, validate]
    }))
}

fn mutate_column(action: ColumnAction, args: &[String]) -> CliResult<Value> {
    let options = parse_column_mutation_args(action, args)?;
    let source_project = required_project(
        options.project.clone(),
        &format!("model columns {}", action.as_str()),
    )?;
    let mode = options.mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "model columns {} requires --dry-run, --in-place, or --out-dir <dir>",
            action.as_str()
        ))
        .with_hint("Start with --dry-run and inspect the exact TMDL block change.")
        .with_suggested_command(format!(
            "powerbi-cli model columns {} --project <project-dir-or.pbip> --table <table> --name <column> --dry-run --json",
            action.as_str()
        ))
    })?;
    if matches!(mode, MutationMode::OutDir) {
        preflight_out_dir(args, |dry_args| mutate_column(action, dry_args))?;
    }
    let source_resolved = resolve_project(&source_project)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let docs = load_table_documents(&target_resolved)?;
    let plan = build_column_plan(action, &docs, &options, mode)?;

    let dry_run = mode == MutationMode::DryRun;
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        let (validation, modified) = write_text_atomic_validated(
            &plan.path,
            &plan.new_text,
            || validate_project(&target_resolved),
            |report| report.errors.is_empty(),
        )?;
        (Some(validation), modified)
    };
    let validation_ok = validation
        .as_ref()
        .is_none_or(|report| report.errors.is_empty());
    let exit_code = if validation_ok {
        EXIT_SUCCESS
    } else {
        EXIT_VALIDATION_FAILED
    };
    let project_arg = command_arg(&target_resolved.project_dir);
    let readback = if matches!(action, ColumnAction::Delete) {
        format!(
            "powerbi-cli model columns list --project {} --table {} --json",
            project_arg,
            crate::cli_support::shell_arg(&plan.table)
        )
    } else {
        format!(
            "powerbi-cli model columns show --project {} --handle {} --json",
            project_arg,
            crate::cli_support::shell_arg(&plan.handle)
        )
    };
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.model.columns.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": action.as_str(),
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({
            "performed": true,
            "projectModified": false,
            "reason": "post-mutation validation failed; the original TMDL file was restored"
        })),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "semanticModelDir": canonical_display(&target_resolved.semantic_model_dir),
        "target": {
            "handle": plan.handle,
            "table": plan.table,
            "name": plan.name,
            "path": canonical_display(&plan.path)
        },
        "changes": [{
            "kind": "tmdl.column",
            "action": action.as_str(),
            "path": canonical_display(&plan.path),
            "before": plan.before_block,
            "after": plan.after_block
        }],
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors,
            "counts": {"tables": report.tables, "measures": report.measures, "relationships": report.relationships, "pages": report.pages, "visuals": report.visuals}
        })),
        "readbackCommand": readback,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, inspect, validate]
    }))
}

fn build_column_plan(
    action: ColumnAction,
    docs: &[crate::tmdl::TableDocument],
    options: &ColumnMutationOptions,
    mode: MutationMode,
) -> CliResult<MutationPlan> {
    match action {
        ColumnAction::Add => {
            let table = options.selector.table.as_deref().expect("validated table");
            let name = options.selector.name.clone().expect("validated name");
            let expression = options.expression.clone();
            if expression.is_some() && options.source_column.is_some() {
                return Err(CliError::invalid_args(
                    "model columns add cannot combine --expression with --source-column",
                )
                .with_hint("Calculated columns use --expression; source columns use --source-column or its default.")
                .with_suggested_command(format!(
                    "powerbi-cli model columns add --project <project-dir-or.pbip> --table {} --name {} --expression <dax> --data-type string --dry-run --json",
                    crate::cli_support::shell_arg(table),
                    crate::cli_support::shell_arg(&name)
                )));
            }
            validate_sort_reference(docs, table, &name, options.sort_by_column.as_deref())?;
            add_column_plan(
                docs,
                table,
                ColumnDefinition {
                    name,
                    expression,
                    data_type: Some(normalize_column_type(
                        options.data_type.as_deref().unwrap_or("string"),
                    )?),
                    lineage_tag: None,
                    format_string: options.format_string.clone(),
                    summarize_by: options.summarize_by.clone(),
                    sort_by_column: options.sort_by_column.clone(),
                    source_column: options.source_column.clone(),
                    display_folder: options.display_folder.clone(),
                    description: options.description.clone(),
                    is_hidden: options.is_hidden.unwrap_or(false),
                    is_key: options.is_key.unwrap_or(false),
                },
            )
        }
        ColumnAction::Update => {
            let existing = find_column_by_selector(docs, &options.selector)?;
            let expression = options
                .expression
                .clone()
                .or_else(|| existing.expression.clone());
            if expression.is_some() && options.source_column.is_some() {
                return Err(CliError::invalid_args(
                    "calculated columns cannot have --source-column",
                )
                .with_hint("Clear the expression or omit --source-column when updating a calculated column."));
            }
            let expression_is_calculated = expression.is_some();
            validate_sort_reference(
                docs,
                &existing.table,
                &existing.name,
                if options.clear_sort_by {
                    None
                } else {
                    options.sort_by_column.as_deref()
                },
            )?;
            replace_column_plan(
                docs,
                &options.selector,
                ColumnDefinition {
                    name: existing.name.clone(),
                    expression,
                    data_type: Some(normalize_column_type(
                        options
                            .data_type
                            .as_deref()
                            .or(existing.data_type.as_deref())
                            .unwrap_or("string"),
                    )?),
                    lineage_tag: existing.lineage_tag.clone(),
                    format_string: options
                        .format_string
                        .clone()
                        .or_else(|| existing.format_string.clone()),
                    summarize_by: options
                        .summarize_by
                        .clone()
                        .or_else(|| existing.summarize_by.clone()),
                    sort_by_column: if options.clear_sort_by {
                        None
                    } else {
                        options
                            .sort_by_column
                            .clone()
                            .or_else(|| existing.sort_by_column.clone())
                    },
                    // Supplying an expression changes a base column into a
                    // calculated column; carrying a sourceColumn into that
                    // shape would produce an invalid mixed block.  A base
                    // column update with no expression still preserves its
                    // existing sourceColumn unless explicitly replaced.
                    source_column: if expression_is_calculated {
                        options.source_column.clone()
                    } else {
                        options
                            .source_column
                            .clone()
                            .or_else(|| existing.source_column.clone())
                    },
                    display_folder: options
                        .display_folder
                        .clone()
                        .or_else(|| existing.display_folder.clone()),
                    description: options
                        .description
                        .clone()
                        .or_else(|| existing.description.clone()),
                    is_hidden: options.is_hidden.unwrap_or(existing.is_hidden),
                    is_key: options.is_key.unwrap_or(existing.is_key),
                },
            )
        }
        ColumnAction::Delete => {
            let existing = find_column_by_selector(docs, &options.selector)?;
            if mode == MutationMode::InPlace
                && options.confirm.as_deref() != Some(existing.handle().as_str())
            {
                return Err(CliError::invalid_args(format!(
                    "in-place delete requires --confirm {}",
                    existing.handle()
                ))
                .with_hint("Run delete with --dry-run first, then rerun with the exact confirm handle.")
                .with_suggested_command(format!(
                    "powerbi-cli model columns delete --project <project-dir-or.pbip> --handle {} --dry-run --json",
                    crate::cli_support::shell_arg(&existing.handle())
                )));
            }
            delete_column_plan(docs, &options.selector)
        }
    }
}

fn validate_sort_reference(
    docs: &[crate::tmdl::TableDocument],
    table: &str,
    column: &str,
    sort_by: Option<&str>,
) -> CliResult<()> {
    let Some(sort_by) = sort_by else {
        return Ok(());
    };
    if same_name(column, sort_by) {
        return Err(CliError::invalid_args(format!(
            "column {} cannot sort by itself",
            column_handle_for_error(table, column)
        ))
        .with_hint("Choose a different column in the same table or omit --sort-by."));
    }
    let _ = find_column(docs, table, sort_by)?;
    Ok(())
}

fn column_handle_for_error(table: &str, column: &str) -> String {
    format!(
        "column:{}:{}",
        table.replace('%', "%25").replace(':', "%3A"),
        column.replace('%', "%25").replace(':', "%3A")
    )
}

fn parse_column_list_args(args: &[String]) -> CliResult<ColumnListOptions> {
    let mut options = ColumnListOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?))
            }
            "--table" => options.table = Some(take_value(args, &mut i, "--table")?),
            other => return Err(CliError::invalid_args(format!(
                "unknown model columns list flag: {other}"
            ))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for \"model columns\"` for exact usage.",
            )
            .with_suggested_command(
                "powerbi-cli model columns list --project <project-dir-or.pbip> --json",
            )),
        }
    }
    Ok(options)
}

fn parse_column_show_args(args: &[String]) -> CliResult<ColumnShowOptions> {
    let mut options = ColumnShowOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?)),
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--table" => options.selector.table = Some(take_value(args, &mut i, "--table")?),
            "--name" => options.selector.name = Some(take_value(args, &mut i, "--name")?),
            other => return Err(CliError::invalid_args(format!("unknown model columns show flag: {other}"))
                .with_hint("Use --handle or --table plus --name from `model columns list`.")
                .with_suggested_command("powerbi-cli model columns show --project <project-dir-or.pbip> --handle <column-handle> --json")),
        }
    }
    if options.selector.handle.is_some()
        && (options.selector.table.is_some() || options.selector.name.is_some())
    {
        return Err(CliError::invalid_args(
            "model columns show accepts one selector: --handle or --table plus --name",
        )
        .with_hint("Use the exact stable handle returned by `model columns list`, or pass --table and --name together.")
        .with_suggested_command(
            "powerbi-cli model columns show --project <project-dir-or.pbip> --handle <column-handle> --json",
        ));
    }
    if options.selector.handle.is_some() {
        let _ = column_selector_parts(&options.selector)?;
    }
    if options.selector.handle.is_none()
        && (options.selector.table.is_none() || options.selector.name.is_none())
    {
        return Err(CliError::invalid_args(
            "model columns show requires --handle or --table plus --name",
        )
        .with_hint("Use a stable handle from `model columns list`.")
        .with_suggested_command(
            "powerbi-cli model columns list --project <project-dir-or.pbip> --json",
        ));
    }
    Ok(options)
}

fn parse_column_mutation_args(
    action: ColumnAction,
    args: &[String],
) -> CliResult<ColumnMutationOptions> {
    let mut options = ColumnMutationOptions::default();
    let mut expression_source = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?))
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--table" => options.selector.table = Some(take_value(args, &mut i, "--table")?),
            "--name" => options.selector.name = Some(take_value(args, &mut i, "--name")?),
            "--expression" => {
                set_expression_source(&mut expression_source, "--expression")?;
                options.expression = Some(take_value(args, &mut i, "--expression")?);
            }
            "--expression-file" => {
                set_expression_source(&mut expression_source, "--expression-file")?;
                let path = take_value(args, &mut i, "--expression-file")?;
                options.expression = Some(read_column_expression_file(&path)?);
            }
            "--data-type" | "--datatype" => {
                options.data_type = Some(take_value(args, &mut i, "--data-type")?)
            }
            "--format-string" => {
                options.format_string = Some(take_value(args, &mut i, "--format-string")?)
            }
            "--summarize-by" => {
                options.summarize_by = Some(take_value(args, &mut i, "--summarize-by")?)
            }
            "--sort-by" | "--sort-by-column" => {
                options.sort_by_column = Some(take_value(args, &mut i, "--sort-by")?)
            }
            "--clear-sort-by" => {
                options.clear_sort_by = true;
                i += 1;
            }
            "--source-column" => {
                options.source_column = Some(take_value(args, &mut i, "--source-column")?)
            }
            "--display-folder" => {
                options.display_folder = Some(take_value(args, &mut i, "--display-folder")?)
            }
            "--description" => {
                options.description = Some(take_value(args, &mut i, "--description")?)
            }
            "--hidden" => {
                if options.is_hidden.is_some() {
                    return Err(CliError::invalid_args(
                        "--hidden and --visible are mutually exclusive",
                    )
                    .with_hint("Pass exactly one visibility flag.")
                    .with_suggested_command(
                        "powerbi-cli model columns update --project <project-dir-or.pbip> --handle <column-handle> --hidden --dry-run --json",
                    ));
                }
                options.is_hidden = Some(true);
                i += 1;
            }
            "--visible" => {
                if options.is_hidden.is_some() {
                    return Err(CliError::invalid_args(
                        "--hidden and --visible are mutually exclusive",
                    )
                    .with_hint("Pass exactly one visibility flag.")
                    .with_suggested_command(
                        "powerbi-cli model columns update --project <project-dir-or.pbip> --handle <column-handle> --visible --dry-run --json",
                    ));
                }
                options.is_hidden = Some(false);
                i += 1;
            }
            "--key" => {
                if options.is_key.is_some() {
                    return Err(CliError::invalid_args(
                        "--key and --not-key are mutually exclusive",
                    )
                    .with_hint("Pass exactly one key flag.")
                    .with_suggested_command(
                        "powerbi-cli model columns update --project <project-dir-or.pbip> --handle <column-handle> --key --dry-run --json",
                    ));
                }
                options.is_key = Some(true);
                i += 1;
            }
            "--not-key" => {
                if options.is_key.is_some() {
                    return Err(CliError::invalid_args(
                        "--key and --not-key are mutually exclusive",
                    )
                    .with_hint("Pass exactly one key flag.")
                    .with_suggested_command(
                        "powerbi-cli model columns update --project <project-dir-or.pbip> --handle <column-handle> --not-key --dry-run --json",
                    ));
                }
                options.is_key = Some(false);
                i += 1;
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    &format!("model columns {}", action.as_str()),
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    &format!("model columns {}", action.as_str()),
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                options.out_dir = Some(PathBuf::from(take_value(args, &mut i, "--out-dir")?));
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    &format!("model columns {}", action.as_str()),
                )?;
            }
            other => return Err(CliError::invalid_args(format!(
                "unknown model columns {} flag: {other}",
                action.as_str()
            ))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for \"model columns\"` for exact usage.",
            )
            .with_suggested_command(
                "powerbi-cli model columns list --project <project-dir-or.pbip> --json",
            )),
        }
    }

    if options.selector.handle.is_some() {
        let _ = column_selector_parts(&options.selector)?;
    }
    if options.selector.handle.is_some()
        && (options.selector.table.is_some() || options.selector.name.is_some())
    {
        return Err(CliError::invalid_args(
            "model columns command accepts one selector: --handle or --table plus --name",
        )
        .with_hint("Use the exact stable handle returned by `model columns list`, or pass --table and --name together.")
        .with_suggested_command(
            "powerbi-cli model columns list --project <project-dir-or.pbip> --json",
        ));
    }
    if !matches!(action, ColumnAction::Delete) && options.confirm.is_some() {
        return Err(CliError::invalid_args(format!(
            "--confirm is only valid for model columns delete, not {}",
            action.as_str()
        ))
        .with_hint("Remove --confirm or use delete with the exact column handle."));
    }
    match action {
        ColumnAction::Add => {
            if options.selector.handle.is_some() {
                return Err(CliError::invalid_args(
                    "model columns add accepts --table and --name, not --handle",
                )
                .with_hint("A new column has no handle until it is added.")
                .with_suggested_command(
                    "powerbi-cli model columns add --project <project-dir-or.pbip> --table <table> --name <column> --data-type string --dry-run --json",
                ));
            }
            if options.selector.table.is_none() || options.selector.name.is_none() {
                return Err(CliError::invalid_args(
                    "model columns add requires --table and --name",
                )
                .with_hint("Pass --table <table> --name <column> and choose an output mode.")
                .with_suggested_command("powerbi-cli model columns add --project <project-dir-or.pbip> --table <table> --name <column> --data-type string --dry-run --json"));
            }
            if options
                .expression
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err(CliError::invalid_args(
                    "column expression must not be empty",
                ));
            }
            validate_column_name(
                options.selector.name.as_deref().expect("validated name"),
                "--name",
            )?;
            if options.clear_sort_by {
                return Err(CliError::invalid_args(
                    "--clear-sort-by is only valid for model columns update",
                )
                .with_hint("Use --sort-by when adding a column, or use update --clear-sort-by to remove an existing sort reference."));
            }
        }
        ColumnAction::Update => {
            if options.clear_sort_by && options.sort_by_column.is_some() {
                return Err(CliError::invalid_args(
                    "model columns update requires exactly one of --sort-by or --clear-sort-by",
                )
                .with_hint(
                    "Set a sort column or clear the existing sortByColumn property, not both.",
                ));
            }
            if options.selector.handle.is_none()
                && (options.selector.table.is_none() || options.selector.name.is_none())
            {
                return Err(CliError::invalid_args(format!(
                    "model columns {} requires --handle or --table plus --name",
                    action.as_str()
                ))
                .with_hint("Use a stable handle from `model columns list`.")
                .with_suggested_command(format!(
                    "powerbi-cli model columns {} --project <project-dir-or.pbip> --handle <column-handle> --dry-run --json",
                    action.as_str()
                )));
            }
            if options.selector.handle.is_none() {
                validate_column_name(
                    options.selector.name.as_deref().expect("validated name"),
                    "--name",
                )?;
            }
            if matches!(action, ColumnAction::Update)
                && options.expression.is_none()
                && options.data_type.is_none()
                && options.format_string.is_none()
                && options.summarize_by.is_none()
                && options.sort_by_column.is_none()
                && !options.clear_sort_by
                && options.source_column.is_none()
                && options.display_folder.is_none()
                && options.description.is_none()
                && options.is_hidden.is_none()
                && options.is_key.is_none()
            {
                return Err(CliError::invalid_args(
                    "model columns update requires at least one field to change",
                )
                .with_hint("Pass --expression, --data-type, --format-string, --summarize-by, --sort-by, --source-column, --description, --hidden, --visible, --key, or --not-key.")
                .with_suggested_command("powerbi-cli model columns update --project <project-dir-or.pbip> --handle <column-handle> --data-type string --dry-run --json"));
            }
        }
        ColumnAction::Delete => {
            if options.expression.is_some()
                || options.data_type.is_some()
                || options.format_string.is_some()
                || options.summarize_by.is_some()
                || options.sort_by_column.is_some()
                || options.clear_sort_by
                || options.source_column.is_some()
                || options.display_folder.is_some()
                || options.description.is_some()
                || options.is_hidden.is_some()
                || options.is_key.is_some()
            {
                return Err(CliError::invalid_args(
                    "model columns delete accepts only a selector, output mode, and optional --confirm",
                )
                .with_hint("Remove column property flags; use model columns update to change metadata."));
            }
            if options.selector.handle.is_none()
                && (options.selector.table.is_none() || options.selector.name.is_none())
            {
                return Err(CliError::invalid_args(
                    "model columns delete requires --handle or --table plus --name",
                )
                .with_hint("Use a stable handle from `model columns list`.")
                .with_suggested_command(
                    "powerbi-cli model columns delete --project <project-dir-or.pbip> --handle <column-handle> --dry-run --json",
                ));
            }
            if options.confirm.is_some() && options.mode != Some(MutationMode::InPlace) {
                return Err(CliError::invalid_args(
                    "--confirm is only valid with --in-place model columns delete",
                )
                .with_hint(
                    "Use --dry-run first, then pass the exact handle with --in-place --confirm.",
                ));
            }
        }
    }
    Ok(options)
}

fn set_expression_source(current: &mut Option<&'static str>, next: &'static str) -> CliResult<()> {
    if let Some(current) = current {
        return Err(CliError::invalid_args(format!(
            "{current} and {next} are mutually exclusive"
        ))
        .with_hint("Pass the DAX expression either inline or in one UTF-8 file, not both.")
        .with_suggested_command(
            "powerbi-cli model columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression-file <path> --data-type string --dry-run --json",
        ));
    }
    *current = Some(next);
    Ok(())
}

fn validate_column_name(name: &str, flag: &str) -> CliResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed != name || name.chars().count() > 100 {
        return Err(CliError::invalid_args(format!(
            "{flag} must be a non-empty name of at most 100 characters without surrounding whitespace"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(CliError::invalid_args(format!(
            "{flag} must not contain control characters"
        )));
    }
    Ok(())
}

fn column_json(column: &ColumnRecord) -> Value {
    json!({
        "handle": column.handle(),
        "table": column.table,
        "name": column.name,
        "isCalculated": column.is_calculated(),
        "expression": column.expression,
        "properties": {
            "lineageTag": column.lineage_tag,
            "dataType": column.data_type,
            "formatString": column.format_string,
            "summarizeBy": column.summarize_by,
            "sortByColumn": column.sort_by_column,
            "sourceColumn": column.source_column,
            "displayFolder": column.display_folder,
            "description": column.description,
            "isHidden": column.is_hidden,
            "isKey": column.is_key
        },
        "path": canonical_display(&column.path),
        "lineRange": {"start": column.start_line + 1, "end": column.end_line}
    })
}

fn normalize_column_type(value: &str) -> CliResult<String> {
    let normalized = match value.trim().to_ascii_lowercase().as_str() {
        "string" | "text" => "string",
        "int64" | "integer" | "whole" | "whole-number" => "int64",
        "double" => "double",
        "decimal" | "currency" => "decimal",
        "date" | "datetime" | "date-time" => "dateTime",
        "boolean" | "bool" => "boolean",
        other => {
            return Err(CliError::unsupported_feature(format!(
                "unsupported model column data type: {other}"
            ))
            .with_hint("Use string, int64, double, decimal, dateTime, or boolean.")
            .with_suggested_command("powerbi-cli model columns add --project <project-dir-or.pbip> --table <table> --name <column> --data-type string --dry-run --json"));
        }
    };
    Ok(normalized.to_string())
}

fn read_column_expression_file(path: &str) -> CliResult<String> {
    let text = if path == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
            .map_err(|err| CliError::unexpected(format!("read expression from stdin: {err}")))?;
        text
    } else {
        fs::read_to_string(path).map_err(|err| {
            CliError::file_not_found(format!("read expression file {path}: {err}"))
        })?
    };
    let expression = text
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if expression.trim().is_empty() {
        return Err(CliError::invalid_args("expression file is empty"));
    }
    Ok(expression)
}
