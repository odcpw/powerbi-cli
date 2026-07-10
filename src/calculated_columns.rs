use crate::project_io::{copy_project_dir, write_text_atomic_validated};
use crate::tmdl::{
    CalculatedColumnDefinition, ColumnRecord, ColumnSelector, MutationPlan, TableDocument,
    add_calculated_column_plan, column_selector_parts, delete_calculated_column_plan,
    find_calculated_column, load_table_documents, replace_calculated_column_plan, same_name,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

pub(crate) fn calculated_columns_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model calculated-columns requires a subcommand: list, show, add, update, delete",
        )
        .with_hint(
            "Run `powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json`.",
        )
        .with_suggested_command(
            "powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" => list_calculated_columns(rest),
        "show" => show_calculated_column(rest),
        "add" => mutate_calculated_column(Action::Add, rest),
        "update" => mutate_calculated_column(Action::Update, rest),
        "delete" => mutate_calculated_column(Action::Delete, rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown model calculated-columns command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for calculated-columns` for supported commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for calculated-columns")),
    }
}

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
    table: Option<String>,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    selector: ColumnSelector,
}

#[derive(Debug, Clone, Copy)]
enum Action {
    Add,
    Update,
    Delete,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug)]
enum MutationMode {
    DryRun,
    InPlace,
    OutDir(PathBuf),
}

#[derive(Debug, Default)]
struct MutationOptions {
    project: Option<PathBuf>,
    selector: ColumnSelector,
    expression: Option<String>,
    data_type: Option<String>,
    format_string: Option<String>,
    summarize_by: Option<String>,
    display_folder: Option<String>,
    description: Option<String>,
    is_hidden: Option<bool>,
    mode: Option<MutationMode>,
    confirm: Option<String>,
}

fn list_calculated_columns(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "model calculated-columns list")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let mut columns = Vec::new();
    for doc in &docs {
        if options
            .table
            .as_ref()
            .is_none_or(|table| same_name(table, &doc.table))
        {
            columns.extend(
                doc.columns
                    .iter()
                    .filter(|column| column.is_calculated())
                    .map(calculated_column_json),
            );
        }
    }
    columns.sort_by(|left, right| {
        left["handle"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["handle"].as_str().unwrap_or_default())
    });

    Ok(json!({
        "schema": "powerbi-cli.model.calculatedColumns.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "filter": {
            "table": options.table
        },
        "counts": {
            "tables": docs.len(),
            "calculatedColumns": columns.len()
        },
        "calculatedColumns": columns,
        "next": [
            format!("powerbi-cli model calculated-columns show --project {} --handle <column-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn show_calculated_column(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "model calculated-columns show")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let record = find_calculated_column(&docs, &options.selector)?;

    Ok(json!({
        "schema": "powerbi-cli.model.calculatedColumns.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "calculatedColumn": calculated_column_json(record),
        "block": record.block,
        "next": [
            format!("powerbi-cli model calculated-columns update --project {} --handle {} --expression <dax> --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&record.handle())),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn mutate_calculated_column(action: Action, args: &[String]) -> CliResult<Value> {
    let options = parse_mutation_args(action, args)?;
    let source_project =
        required_project(options.project.clone(), "model calculated-columns mutation")?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = options.mode.as_ref().ok_or_else(|| {
        CliError::invalid_args(format!(
            "model calculated-columns {} requires --dry-run, --in-place, or --out-dir <dir>",
            action.as_str()
        ))
        .with_hint("Start with `--dry-run`; use `--in-place` only when the plan is correct.")
        .with_suggested_command(format!(
            "powerbi-cli model calculated-columns {} --project <project-dir-or.pbip> --dry-run --json",
            action.as_str()
        ))
    })?;

    let target_resolved = match mode {
        MutationMode::DryRun | MutationMode::InPlace => source_resolved,
        MutationMode::OutDir(out_dir) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)?
        }
    };

    let docs = load_table_documents(&target_resolved)?;
    let plan = build_mutation_plan(action, &docs, &options)?;
    let dry_run = matches!(mode, MutationMode::DryRun);
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        let (validation, project_modified) = write_text_atomic_validated(
            &plan.path,
            &plan.new_text,
            || validate_project(&target_resolved),
            |report| report.errors.is_empty(),
        )?;
        (Some(validation), project_modified)
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
    let readback = match action {
        Action::Delete => format!(
            "powerbi-cli model calculated-columns list --project {} --table {} --json",
            project_arg,
            shell_arg(&plan.table)
        ),
        Action::Add | Action::Update => format!(
            "powerbi-cli model calculated-columns show --project {} --handle {} --json",
            project_arg,
            shell_arg(&plan.handle)
        ),
    };
    let inspect = format!("powerbi-cli inspect --deep {} --json", project_arg);
    let validate = format!("powerbi-cli validate --strict {} --json", project_arg);

    Ok(json!({
        "schema": "powerbi-cli.model.calculatedColumns.mutation.v1",
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
            "kind": "tmdl.calculatedColumn",
            "action": action.as_str(),
            "path": canonical_display(&plan.path),
            "before": plan.before_block,
            "after": plan.after_block
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
                "visuals": report.visuals
            }
        })),
        "readbackCommand": readback,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, inspect, validate]
    }))
}

fn build_mutation_plan(
    action: Action,
    docs: &[TableDocument],
    options: &MutationOptions,
) -> CliResult<MutationPlan> {
    match action {
        Action::Add => {
            let table_name = options.selector.table.as_deref().expect("validated table");
            let data_type = normalize_column_data_type(
                options.data_type.as_deref().expect("validated data type"),
            )?;
            let definition = CalculatedColumnDefinition {
                name: options.selector.name.clone().expect("validated name"),
                expression: options.expression.clone().expect("validated expression"),
                data_type: data_type.tmdl.to_string(),
                lineage_tag: None,
                format_string: options
                    .format_string
                    .clone()
                    .or_else(|| data_type.default_format_string.map(ToOwned::to_owned)),
                summarize_by: options.summarize_by.clone(),
                display_folder: options.display_folder.clone(),
                description: options.description.clone(),
                is_hidden: options.is_hidden.unwrap_or(false),
            };
            add_calculated_column_plan(docs, table_name, definition)
        }
        Action::Update => {
            let existing = find_calculated_column(docs, &options.selector)?;
            let normalized_data_type = options
                .data_type
                .as_deref()
                .map(normalize_column_data_type)
                .transpose()?;
            let definition = CalculatedColumnDefinition {
                name: existing.name.clone(),
                expression: options
                    .expression
                    .clone()
                    .unwrap_or_else(|| existing.expression.clone().unwrap_or_default()),
                data_type: normalized_data_type
                    .map(|data_type| data_type.tmdl.to_string())
                    .or_else(|| existing.data_type.clone())
                    .unwrap_or_else(|| "string".to_string()),
                lineage_tag: existing.lineage_tag.clone(),
                format_string: options
                    .format_string
                    .clone()
                    .or_else(|| existing.format_string.clone())
                    .or_else(|| {
                        normalized_data_type
                            .and_then(|data_type| data_type.default_format_string)
                            .map(ToOwned::to_owned)
                    }),
                summarize_by: options
                    .summarize_by
                    .clone()
                    .or_else(|| existing.summarize_by.clone()),
                display_folder: options
                    .display_folder
                    .clone()
                    .or_else(|| existing.display_folder.clone()),
                description: options
                    .description
                    .clone()
                    .or_else(|| existing.description.clone()),
                is_hidden: options.is_hidden.unwrap_or(existing.is_hidden),
            };
            replace_calculated_column_plan(docs, &options.selector, definition)
        }
        Action::Delete => {
            let existing = find_calculated_column(docs, &options.selector)?;
            if matches!(options.mode, Some(MutationMode::InPlace))
                && options.confirm.as_deref() != Some(existing.handle().as_str())
            {
                return Err(CliError::invalid_args(format!(
                    "in-place delete requires --confirm {}",
                    existing.handle()
                ))
                .with_hint(
                    "Run delete with `--dry-run` first, then rerun with the exact confirm handle.",
                )
                .with_suggested_command(format!(
                    "powerbi-cli model calculated-columns delete --project <project-dir-or.pbip> --handle {} --dry-run --json",
                    shell_arg(&existing.handle())
                )));
            }
            delete_calculated_column_plan(docs, &options.selector)
        }
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
            "--table" => {
                options.table = Some(take_value(args, &mut i, "--table")?);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model calculated-columns list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli model calculated-columns list --project <project-dir-or.pbip> --json",
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
            "--table" => options.selector.table = Some(take_value(args, &mut i, "--table")?),
            "--name" => options.selector.name = Some(take_value(args, &mut i, "--name")?),
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model calculated-columns show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli model calculated-columns show --project <project-dir-or.pbip> --handle <column-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli model calculated-columns show --project <project-dir-or.pbip> --handle <column-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_mutation_args(action: Action, args: &[String]) -> CliResult<MutationOptions> {
    let mut options = MutationOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.selector.handle = Some(take_value(args, &mut i, "--handle")?),
            "--table" => options.selector.table = Some(take_value(args, &mut i, "--table")?),
            "--name" => options.selector.name = Some(take_value(args, &mut i, "--name")?),
            "--expression" => options.expression = Some(take_value(args, &mut i, "--expression")?),
            "--expression-file" => {
                let path = take_value(args, &mut i, "--expression-file")?;
                options.expression = Some(read_expression_file(&path)?);
            }
            "--data-type" | "--datatype" => {
                options.data_type = Some(take_value(args, &mut i, "--data-type")?);
            }
            "--format-string" => {
                options.format_string = Some(take_value(args, &mut i, "--format-string")?);
            }
            "--summarize-by" => {
                options.summarize_by = Some(take_value(args, &mut i, "--summarize-by")?);
            }
            "--display-folder" => {
                options.display_folder = Some(take_value(args, &mut i, "--display-folder")?);
            }
            "--description" => {
                options.description = Some(take_value(args, &mut i, "--description")?);
            }
            "--hidden" => {
                options.is_hidden = Some(true);
                i += 1;
            }
            "--visible" => {
                options.is_hidden = Some(false);
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
                set_mode(&mut options.mode, MutationMode::OutDir(out_dir))?;
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model calculated-columns {} flag: {other}",
                    action.as_str()
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for calculated-columns` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for calculated-columns",
                ));
            }
        }
    }

    if options.selector.handle.is_some() {
        let _ = column_selector_parts(&options.selector)?;
    }
    if !matches!(action, Action::Delete) && options.confirm.is_some() {
        return Err(CliError::invalid_args(format!(
            "--confirm is only valid for model calculated-columns delete, not {}",
            action.as_str()
        ))
        .with_hint("Remove --confirm or use the delete action with an exact column handle."));
    }

    if matches!(action, Action::Add) {
        if options.selector.table.is_none() || options.selector.name.is_none() {
            return Err(CliError::invalid_args(
                "model calculated-columns add requires --table and --name",
            )
            .with_hint(
                "Run add with `--table <table> --name <column> --expression <dax> --data-type <type> --dry-run` first.",
            )
            .with_suggested_command(
                "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression <dax> --data-type string --dry-run --json",
            ));
        }
        if options
            .expression
            .as_ref()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(CliError::invalid_args(
                "model calculated-columns add requires --expression or --expression-file",
            )
            .with_hint("Use `--expression-file <path>` when shell quoting DAX is awkward.")
            .with_suggested_command(
                "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression-file <path> --data-type string --dry-run --json",
            ));
        }
        if options
            .data_type
            .as_ref()
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(CliError::invalid_args(
                "model calculated-columns add requires --data-type",
            )
            .with_hint("Use one of: string, int64, double, decimal, date, dateTime, boolean.")
            .with_suggested_command(
                "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression <dax> --data-type string --dry-run --json",
            ));
        }
    }
    if matches!(action, Action::Update | Action::Delete) {
        require_selector(&options.selector, action.as_str())?;
    }
    if matches!(action, Action::Update)
        && options.expression.is_none()
        && options.data_type.is_none()
        && options.format_string.is_none()
        && options.summarize_by.is_none()
        && options.display_folder.is_none()
        && options.description.is_none()
        && options.is_hidden.is_none()
    {
        return Err(CliError::invalid_args(
            "model calculated-columns update requires at least one field to change",
        )
        .with_hint(
            "Pass `--expression`, `--data-type`, `--format-string`, `--summarize-by`, `--description`, `--hidden`, or `--visible`.",
        )
        .with_suggested_command(
            "powerbi-cli model calculated-columns update --project <project-dir-or.pbip> --handle <column-handle> --expression <dax> --dry-run --json",
        ));
    }

    Ok(options)
}

fn calculated_column_json(column: &ColumnRecord) -> Value {
    json!({
        "handle": column.handle(),
        "table": column.table,
        "name": column.name,
        "expression": column.expression,
        "properties": {
            "lineageTag": column.lineage_tag,
            "dataType": column.data_type,
            "formatString": column.format_string,
            "summarizeBy": column.summarize_by,
            "displayFolder": column.display_folder,
            "description": column.description,
            "isHidden": column.is_hidden
        },
        "path": canonical_display(&column.path),
        "lineRange": {
            "start": column.start_line + 1,
            "end": column.end_line
        }
    })
}

#[derive(Debug, Clone, Copy)]
struct NormalizedColumnDataType {
    tmdl: &'static str,
    default_format_string: Option<&'static str>,
}

fn normalize_column_data_type(value: &str) -> CliResult<NormalizedColumnDataType> {
    let normalized = match value.trim().to_ascii_lowercase().as_str() {
        "string" | "text" => NormalizedColumnDataType {
            tmdl: "string",
            default_format_string: None,
        },
        "int64" | "integer" | "whole" | "whole-number" => NormalizedColumnDataType {
            tmdl: "int64",
            default_format_string: None,
        },
        "double" => NormalizedColumnDataType {
            tmdl: "double",
            default_format_string: None,
        },
        "decimal" | "currency" => NormalizedColumnDataType {
            tmdl: "decimal",
            default_format_string: None,
        },
        "date" => NormalizedColumnDataType {
            tmdl: "dateTime",
            default_format_string: Some("Short Date"),
        },
        "datetime" | "date-time" => NormalizedColumnDataType {
            tmdl: "dateTime",
            default_format_string: None,
        },
        "boolean" | "bool" => NormalizedColumnDataType {
            tmdl: "boolean",
            default_format_string: None,
        },
        other => {
            return Err(CliError::unsupported_feature(format!(
                "unsupported calculated column data type: {other}"
            ))
            .with_hint("Use string, int64, double, decimal, date, dateTime, or boolean. `date` emits TMDL dateTime with a Short Date format unless --format-string is supplied.")
            .with_suggested_command(
                "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression <dax> --data-type string --dry-run --json",
            ));
        }
    };
    Ok(normalized)
}

fn set_mode(current: &mut Option<MutationMode>, next: MutationMode) -> CliResult<()> {
    if current.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one output mode: --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.")
        .with_suggested_command(
            "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression <dax> --data-type string --dry-run --json",
        ));
    }
    *current = Some(next);
    Ok(())
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for calculated-columns` for exact usage.",
            )
            .with_suggested_command("powerbi-cli --json capabilities --for calculated-columns")
    })?;
    *index += 2;
    Ok(value.clone())
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

fn require_selector(selector: &ColumnSelector, action: &str) -> CliResult<()> {
    if selector.handle.is_some() {
        return Ok(());
    }
    if selector.table.is_some() && selector.name.is_some() {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "model calculated-columns {action} requires --handle or --table plus --name"
    ))
    .with_hint(
        "Use handles from `powerbi-cli model calculated-columns list --project <project> --json`.",
    )
    .with_suggested_command(format!(
        "powerbi-cli model calculated-columns {action} --project <project-dir-or.pbip> --handle <column-handle> --dry-run --json"
    )))
}

fn read_expression_file(path: &str) -> CliResult<String> {
    let text = if path == "-" {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
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
        return Err(CliError::invalid_args("expression file is empty")
            .with_hint("Provide a DAX expression, for example `IF('FactSales'[Revenue] > 0, \"Has Revenue\", \"No Revenue\")`.")
            .with_suggested_command(
                "powerbi-cli model calculated-columns add --project <project-dir-or.pbip> --table <table> --name <column> --expression-file <path> --data-type string --dry-run --json",
            ));
    }
    Ok(expression)
}

fn mode_name(mode: &MutationMode) -> &'static str {
    match mode {
        MutationMode::DryRun => "dry-run",
        MutationMode::InPlace => "in-place",
        MutationMode::OutDir(_) => "out-dir",
    }
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
