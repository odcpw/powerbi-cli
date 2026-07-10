use crate::project_io::{copy_project_dir, write_text_atomic_validated};
use crate::relationship_tmdl::{
    RelationshipDefinition, RelationshipMutationPlan, RelationshipRecord, RelationshipSelector,
    add_relationship_plan, default_relationship_name, delete_relationship_plan, find_relationship,
    load_relationship_document, load_relationships_and_tables, normalize_cross_filtering_behavior,
    replace_relationship_plan,
};
use crate::tmdl::column_handle;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(crate) fn relationships_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model relationships requires a subcommand: list, show, add, update, delete",
        )
        .with_hint(
            "Run `powerbi-cli model relationships list --project <project-dir-or.pbip> --json`.",
        )
        .with_suggested_command(
            "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" => list_relationships(rest),
        "show" => show_relationship(rest),
        "add" => mutate_relationship(Action::Add, rest),
        "update" => mutate_relationship(Action::Update, rest),
        "delete" => mutate_relationship(Action::Delete, rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown model relationships command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for relationships` for supported relationship commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for relationships")),
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
    selector: RelationshipSelector,
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
    selector: RelationshipSelector,
    from_table: Option<String>,
    from_column: Option<String>,
    to_table: Option<String>,
    to_column: Option<String>,
    cross_filtering_behavior: Option<String>,
    is_active: Option<bool>,
    mode: Option<MutationMode>,
    confirm: Option<String>,
}

fn list_relationships(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "model relationships list")?;
    let resolved = resolve_project(&project)?;
    let doc = load_relationship_document(&resolved)?;
    let mut relationships = doc
        .relationships
        .iter()
        .filter(|relationship| {
            options.table.as_ref().is_none_or(|table| {
                relationship.from_table.eq_ignore_ascii_case(table)
                    || relationship.to_table.eq_ignore_ascii_case(table)
            })
        })
        .map(relationship_json)
        .collect::<Vec<_>>();
    relationships.sort_by(|left, right| {
        left["handle"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["handle"].as_str().unwrap_or_default())
    });

    Ok(json!({
        "schema": "powerbi-cli.model.relationships.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "filter": {
            "table": options.table
        },
        "counts": {
            "relationships": relationships.len()
        },
        "relationships": relationships,
        "next": [
            format!("powerbi-cli model relationships show --project {} --handle <relationship-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn show_relationship(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "model relationships show")?;
    let resolved = resolve_project(&project)?;
    let doc = load_relationship_document(&resolved)?;
    let record = find_relationship(&doc, &options.selector)?;

    Ok(json!({
        "schema": "powerbi-cli.model.relationships.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "relationship": relationship_json(record),
        "block": record.block,
        "next": [
            format!("powerbi-cli model relationships update --project {} --handle {} --cross-filtering-behavior oneDirection --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&record.handle())),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn mutate_relationship(action: Action, args: &[String]) -> CliResult<Value> {
    let options = parse_mutation_args(action, args)?;
    let source_project = required_project(options.project.clone(), "model relationships mutation")?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = options.mode.as_ref().ok_or_else(|| {
        CliError::invalid_args(format!(
            "model relationships {} requires --dry-run, --in-place, or --out-dir <dir>",
            action.as_str()
        ))
        .with_hint("Start with `--dry-run`; use `--in-place` only when the plan is correct.")
        .with_suggested_command(format!(
            "powerbi-cli model relationships {} --project <project-dir-or.pbip> --dry-run --json",
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

    let (doc, tables) = load_relationships_and_tables(&target_resolved)?;
    let plan = build_mutation_plan(action, &doc, &tables, &options)?;
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
            "powerbi-cli model relationships list --project {} --json",
            project_arg
        ),
        Action::Add | Action::Update => format!(
            "powerbi-cli model relationships show --project {} --handle {} --json",
            project_arg,
            shell_arg(&plan.handle)
        ),
    };
    let inspect = format!("powerbi-cli inspect --deep {} --json", project_arg);
    let validate = format!("powerbi-cli validate --strict {} --json", project_arg);

    Ok(json!({
        "schema": "powerbi-cli.model.relationships.mutation.v1",
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
            "name": plan.name,
            "path": canonical_display(&plan.path)
        },
        "changes": [{
            "kind": "tmdl.relationship",
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
    doc: &crate::relationship_tmdl::RelationshipDocument,
    tables: &[crate::tmdl::TableDocument],
    options: &MutationOptions,
) -> CliResult<RelationshipMutationPlan> {
    match action {
        Action::Add => {
            let from_table = options.from_table.clone().expect("validated from table");
            let from_column = options.from_column.clone().expect("validated from column");
            let to_table = options.to_table.clone().expect("validated to table");
            let to_column = options.to_column.clone().expect("validated to column");
            let name = options.selector.name.clone().unwrap_or_else(|| {
                default_relationship_name(&from_table, &from_column, &to_table, &to_column)
            });
            let definition = RelationshipDefinition {
                name,
                from_table,
                from_column,
                to_table,
                to_column,
                cross_filtering_behavior: options
                    .cross_filtering_behavior
                    .clone()
                    .unwrap_or_else(|| "oneDirection".to_string()),
                is_active: options.is_active.unwrap_or(true),
            };
            add_relationship_plan(doc, tables, definition)
        }
        Action::Update => {
            let existing = find_relationship(doc, &options.selector)?;
            let definition = RelationshipDefinition {
                name: existing.name.clone(),
                from_table: options
                    .from_table
                    .clone()
                    .unwrap_or_else(|| existing.from_table.clone()),
                from_column: options
                    .from_column
                    .clone()
                    .unwrap_or_else(|| existing.from_column.clone()),
                to_table: options
                    .to_table
                    .clone()
                    .unwrap_or_else(|| existing.to_table.clone()),
                to_column: options
                    .to_column
                    .clone()
                    .unwrap_or_else(|| existing.to_column.clone()),
                cross_filtering_behavior: options
                    .cross_filtering_behavior
                    .clone()
                    .unwrap_or_else(|| existing.cross_filtering_behavior.clone()),
                is_active: options.is_active.unwrap_or(existing.is_active),
            };
            replace_relationship_plan(doc, tables, &options.selector, definition)
        }
        Action::Delete => {
            let existing = find_relationship(doc, &options.selector)?;
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
                    "powerbi-cli model relationships delete --project <project-dir-or.pbip> --handle {} --dry-run --json",
                    shell_arg(&existing.handle())
                )));
            }
            delete_relationship_plan(doc, &options.selector)
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
                    "unknown model relationships list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli model relationships list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli model relationships list --project <project-dir-or.pbip> --json",
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
            "--name" => options.selector.name = Some(take_value(args, &mut i, "--name")?),
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model relationships show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli model relationships show --project <project-dir-or.pbip> --handle <relationship-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli model relationships show --project <project-dir-or.pbip> --handle <relationship-handle> --json",
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
            "--name" => options.selector.name = Some(take_value(args, &mut i, "--name")?),
            "--from-table" => options.from_table = Some(take_value(args, &mut i, "--from-table")?),
            "--from-column" => {
                options.from_column = Some(take_value(args, &mut i, "--from-column")?);
            }
            "--to-table" => options.to_table = Some(take_value(args, &mut i, "--to-table")?),
            "--to-column" => options.to_column = Some(take_value(args, &mut i, "--to-column")?),
            "--cross-filtering-behavior" | "--cross-filter" => {
                let value = take_value(args, &mut i, "--cross-filtering-behavior")?;
                options.cross_filtering_behavior =
                    Some(normalize_cross_filtering_behavior(&value)?);
            }
            "--active" => {
                options.is_active = Some(true);
                i += 1;
            }
            "--inactive" => {
                options.is_active = Some(false);
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
                    "unknown model relationships {} flag: {other}",
                    action.as_str()
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for relationships` for exact flags.",
                )
                .with_suggested_command("powerbi-cli --json capabilities --for relationships"));
            }
        }
    }

    if !matches!(action, Action::Delete) && options.confirm.is_some() {
        return Err(CliError::invalid_args(format!(
            "--confirm is only valid for model relationships delete, not {}",
            action.as_str()
        ))
        .with_hint(
            "Remove --confirm or use the delete action with an exact relationship handle.",
        ));
    }

    if matches!(action, Action::Add) {
        require_add_endpoint(&options.from_table, "--from-table")?;
        require_add_endpoint(&options.from_column, "--from-column")?;
        require_add_endpoint(&options.to_table, "--to-table")?;
        require_add_endpoint(&options.to_column, "--to-column")?;
    }
    if matches!(action, Action::Update | Action::Delete) {
        require_selector(&options.selector, action.as_str())?;
    }
    if matches!(action, Action::Update)
        && (options.from_table.is_some()
            || options.from_column.is_some()
            || options.to_table.is_some()
            || options.to_column.is_some())
    {
        return Err(CliError::invalid_args(
            "model relationships update does not rewire endpoints yet",
        )
        .with_hint(
            "For endpoint changes, delete the old relationship and add a new relationship so the diff is explicit.",
        )
        .with_suggested_command(
            "powerbi-cli model relationships delete --project <project-dir-or.pbip> --handle <relationship-handle> --dry-run --json",
        ));
    }
    if matches!(action, Action::Update)
        && options.from_table.is_none()
        && options.from_column.is_none()
        && options.to_table.is_none()
        && options.to_column.is_none()
        && options.cross_filtering_behavior.is_none()
        && options.is_active.is_none()
    {
        return Err(CliError::invalid_args(
            "model relationships update requires at least one field to change",
        )
        .with_hint(
            "Pass an endpoint flag, `--cross-filtering-behavior`, `--active`, or `--inactive`.",
        )
        .with_suggested_command(
            "powerbi-cli model relationships update --project <project-dir-or.pbip> --handle <relationship-handle> --cross-filtering-behavior bothDirections --dry-run --json",
        ));
    }

    Ok(options)
}

fn relationship_json(relationship: &RelationshipRecord) -> Value {
    json!({
        "handle": relationship.handle(),
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
        }
    })
}

fn set_mode(current: &mut Option<MutationMode>, next: MutationMode) -> CliResult<()> {
    if current.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one output mode: --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.")
        .with_suggested_command(
            "powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> --dry-run --json",
        ));
    }
    *current = Some(next);
    Ok(())
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli --json capabilities --for relationships` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for relationships")
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

fn require_selector(selector: &RelationshipSelector, action: &str) -> CliResult<()> {
    if selector.handle.is_some() || selector.name.is_some() {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "model relationships {action} requires --handle or --name"
    ))
    .with_hint("Use handles from `powerbi-cli model relationships list --project <project> --json`.")
    .with_suggested_command(format!(
        "powerbi-cli model relationships {action} --project <project-dir-or.pbip> --handle <relationship-handle> --dry-run --json"
    )))
}

fn require_add_endpoint(value: &Option<String>, flag: &str) -> CliResult<()> {
    if value.as_ref().is_some_and(|value| !value.trim().is_empty()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "model relationships add requires {flag}"
    ))
    .with_hint("Pass explicit endpoint flags so agents do not swap relationship direction.")
    .with_suggested_command(
        "powerbi-cli model relationships add --project <project-dir-or.pbip> --from-table <table> --from-column <column> --to-table <table> --to-column <column> --dry-run --json",
    ))
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
