use crate::cli_support::{
    MutationMode, mode_name, preflight_out_dir, required_project, set_mode, take_value,
    target_project,
};
use crate::project_io::write_text_atomic_validated;
use crate::tmdl::{find_column, load_table_documents, set_column_sort_by_plan};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
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
            "model columns requires a subcommand: set-sort-by",
        )
        .with_hint("Set or clear one column's same-table TMDL sortByColumn property.")
        .with_suggested_command(
            "powerbi-cli model columns set-sort-by --project <project-dir-or.pbip> --table <table> --column <column> --by <sort-column> --dry-run --json",
        ));
    };
    match action.as_str() {
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
