use crate::cli_support::{
    MutationMode, mode_name, preflight_out_dir, required_project, set_mode, target_project,
};
use crate::input_safety::{InputKind, read_utf8, read_utf8_stream, validate_text};
use crate::project_io::{write_text_atomic, write_text_atomic_validated};
use crate::safety_scan::contains_credential_like_text_str;
use crate::tmdl::{
    ColumnDefinition, ColumnRecord, TableDocument, column_block_lines, find_table,
    load_table_documents, same_name, table_handle,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::static_tables::static_tables_command;

pub(crate) fn tables_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model tables requires a subcommand: list, show, add, add-calculated, rename, delete, add-static",
        )
        .with_hint("Use model tables list/show for readback or add/add-calculated/rename/delete for guarded TMDL table CRUD.")
        .with_suggested_command(
            "powerbi-cli model tables list --project <project-dir-or.pbip> --json",
        ));
    };
    match action.as_str() {
        "list" => list_tables(rest),
        "show" => show_table(rest),
        "add" => mutate_table(TableAction::Add, rest),
        "add-calculated" | "addCalculated" => mutate_table(TableAction::AddCalculated, rest),
        "rename" | "update" => mutate_table(TableAction::Rename, rest),
        "delete" => mutate_table(TableAction::Delete, rest),
        "add-static" | "addStatic" | "add-selector" | "addSelector" => static_tables_command(args),
        other => Err(
            CliError::invalid_args(format!("unknown model tables command: {other}"))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"model tables\"` for exact usage.",
                )
                .with_suggested_command(
                    "powerbi-cli model tables list --project <project-dir-or.pbip> --json",
                ),
        ),
    }
}

#[derive(Debug, Default)]
struct TableListOptions {
    project: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct TableShowOptions {
    project: Option<PathBuf>,
    table: Option<String>,
    handle: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum TableAction {
    Add,
    AddCalculated,
    Rename,
    Delete,
}

impl TableAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::AddCalculated => "add-calculated",
            Self::Rename => "rename",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Default)]
struct TableMutationOptions {
    project: Option<PathBuf>,
    table: Option<String>,
    new_name: Option<String>,
    handle: Option<String>,
    columns: Vec<ColumnDefinitionInput>,
    columns_json_seen: bool,
    data_type: Option<String>,
    expression: Option<String>,
    rename_references: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    confirm: Option<String>,
}

#[derive(Debug, Clone)]
struct ColumnDefinitionInput {
    definition: ColumnDefinition,
}

fn list_tables(args: &[String]) -> CliResult<Value> {
    let options = parse_table_list_args(args)?;
    let project = required_project(options.project, "model tables list")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let mut tables = docs.iter().map(table_json).collect::<Vec<_>>();
    tables.sort_by(|left, right| {
        left["handle"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["handle"].as_str().unwrap_or_default())
    });
    let project_arg = command_arg(&resolved.project_dir);
    let show = format!(
        "powerbi-cli model tables show --project {project_arg} --handle <table-handle> --json"
    );
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.model.tables.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "counts": {"tables": tables.len(), "columns": docs.iter().map(|doc| doc.columns.len()).sum::<usize>(), "measures": docs.iter().map(|doc| doc.measures.len()).sum::<usize>(), "partitions": docs.iter().map(|doc| doc.partitions.len()).sum::<usize>()},
        "tables": tables,
        "next": [show, inspect, validate]
    }))
}

fn show_table(args: &[String]) -> CliResult<Value> {
    let options = parse_table_show_args(args)?;
    let project = required_project(options.project, "model tables show")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let table_name = if let Some(handle) = options.handle {
        parse_table_handle(&handle)?
    } else {
        options.table.expect("validated table")
    };
    let doc = find_table(&docs, &table_name)?;
    let project_arg = command_arg(&resolved.project_dir);
    let rename = format!(
        "powerbi-cli model tables rename --project {project_arg} --handle {} --new-name <table> --dry-run --json",
        crate::cli_support::shell_arg(&table_handle(&doc.table))
    );
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.model.tables.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "table": table_json(doc),
        "block": read_utf8(&doc.path, InputKind::ProjectText)?,
        "next": [rename, validate]
    }))
}

fn mutate_table(action: TableAction, args: &[String]) -> CliResult<Value> {
    let options = parse_table_mutation_args(action, args)?;
    let command = format!("model tables {}", action.as_str());
    let source_project = required_project(options.project.clone(), &command)?;
    let mode = options.mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --dry-run, --in-place, or --out-dir <dir>"
        ))
        .with_hint("Start with --dry-run and inspect the exact TMDL change.")
        .with_suggested_command(format!(
            "powerbi-cli model tables {} --project <project-dir-or.pbip> --dry-run --json",
            action.as_str()
        ))
    })?;
    if mode == MutationMode::OutDir {
        preflight_out_dir(args, |dry_args| mutate_table(action, dry_args))?;
    }
    let source_resolved = resolve_project(&source_project)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let docs = load_table_documents(&target_resolved)?;
    match action {
        TableAction::Add => add_table(&target_resolved, &docs, &options, mode),
        TableAction::AddCalculated => add_calculated_table(&target_resolved, &docs, &options, mode),
        TableAction::Rename => rename_table(&target_resolved, &docs, &options, mode),
        TableAction::Delete => delete_table(&target_resolved, &docs, &options, mode),
    }
}

fn add_calculated_table(
    resolved: &crate::ResolvedProject,
    docs: &[TableDocument],
    options: &TableMutationOptions,
    mode: MutationMode,
) -> CliResult<Value> {
    let table = options.table.as_deref().expect("validated table");
    if docs.iter().any(|doc| same_name(&doc.table, table)) {
        return Err(CliError::invalid_args(format!(
            "semantic model table already exists: {table}"
        ))
        .with_hint("Choose a new table name; this command never replaces an existing table.")
        .with_suggested_command(format!(
            "powerbi-cli model tables list --project {} --json",
            command_arg(&resolved.project_dir)
        )));
    }
    let path = resolved
        .semantic_model_dir
        .join("definition")
        .join("tables")
        .join(format!("{table}.tmdl"));
    if path.exists() {
        return Err(CliError::invalid_args(format!(
            "table target already exists: {}",
            path.display()
        ))
        .with_hint("The command never overwrites an existing table file."));
    }
    let expression = options.expression.as_deref().expect("validated expression");
    let text = calculated_table_tmdl(table, expression, &options.columns);
    let dry_run = mode == MutationMode::DryRun;
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        let (validation, modified) = write_text_atomic_validated(
            &path,
            &text,
            || validate_project(resolved),
            |report| report.errors.is_empty(),
        )?;
        (Some(validation), modified)
    };
    let validation_ok = validation
        .as_ref()
        .is_none_or(|report| report.errors.is_empty());
    let project_arg = command_arg(&resolved.project_dir);
    let show = format!(
        "powerbi-cli model tables show --project {project_arg} --handle {} --json",
        crate::cli_support::shell_arg(&table_handle(table))
    );
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.model.tables.mutation.v1",
        "ok": validation_ok,
        "exitCode": if validation_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "action": "add-calculated",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({"performed": true, "projectModified": false, "reason": "post-mutation validation failed; the new calculated table file was removed"})),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "target": {"handle": table_handle(table), "table": table, "path": canonical_display(&path), "partitionKind": "calculated"},
        "changes": [{"kind": "tmdl.calculatedTable", "action": "add", "path": canonical_display(&path), "before": Value::Null, "after": text}],
        "validation": validation.map(validation_json),
        "readbackCommand": show,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [show, inspect, validate]
    }))
}

fn add_table(
    resolved: &crate::ResolvedProject,
    docs: &[TableDocument],
    options: &TableMutationOptions,
    mode: MutationMode,
) -> CliResult<Value> {
    let table = options.table.as_deref().expect("validated table");
    if docs.iter().any(|doc| same_name(&doc.table, table)) {
        return Err(CliError::invalid_args(format!(
            "semantic model table already exists: {table}"
        ))
        .with_hint("Choose a new table name; this command never replaces an existing table.")
        .with_suggested_command(format!(
            "powerbi-cli model tables list --project {} --json",
            command_arg(&resolved.project_dir)
        )));
    }
    let path = resolved
        .semantic_model_dir
        .join("definition")
        .join("tables")
        .join(format!("{table}.tmdl"));
    if path.exists() {
        return Err(CliError::invalid_args(format!(
            "table target already exists: {}",
            path.display()
        ))
        .with_hint("The command never overwrites an existing table file."));
    }
    let definitions = if options.columns.is_empty() {
        vec![ColumnDefinitionInput {
            definition: ColumnDefinition {
                name: "Value".to_string(),
                expression: None,
                data_type: Some(normalize_table_type(
                    options.data_type.as_deref().unwrap_or("string"),
                )?),
                lineage_tag: None,
                format_string: None,
                summarize_by: None,
                sort_by_column: None,
                source_column: None,
                display_folder: None,
                description: None,
                is_hidden: false,
                is_key: false,
            },
        }]
    } else {
        options.columns.clone()
    };
    let text = table_tmdl(table, &definitions);
    let dry_run = mode == MutationMode::DryRun;
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        let (validation, modified) = write_text_atomic_validated(
            &path,
            &text,
            || validate_project(resolved),
            |report| report.errors.is_empty(),
        )?;
        (Some(validation), modified)
    };
    let validation_ok = validation
        .as_ref()
        .is_none_or(|report| report.errors.is_empty());
    let project_arg = command_arg(&resolved.project_dir);
    let show = format!(
        "powerbi-cli model tables show --project {project_arg} --handle {} --json",
        crate::cli_support::shell_arg(&table_handle(table))
    );
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.model.tables.mutation.v1",
        "ok": validation_ok,
        "exitCode": if validation_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "action": "add",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({"performed": true, "projectModified": false, "reason": "post-mutation validation failed; the new TMDL table file was removed"})),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "target": {"handle": table_handle(table), "table": table, "path": canonical_display(&path)},
        "changes": [{"kind": "tmdl.table", "action": "add", "path": canonical_display(&path), "before": Value::Null, "after": text}],
        "validation": validation.map(validation_json),
        "readbackCommand": show,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [show, inspect, validate]
    }))
}

fn rename_table(
    resolved: &crate::ResolvedProject,
    docs: &[TableDocument],
    options: &TableMutationOptions,
    mode: MutationMode,
) -> CliResult<Value> {
    let old_name = if let Some(handle) = options.handle.as_deref() {
        parse_table_handle(handle)?
    } else {
        options.table.clone().expect("validated table")
    };
    let new_name = options.new_name.as_deref().expect("validated new name");
    let doc = find_table(docs, &old_name)?;
    if same_name(&doc.table, new_name) {
        return Err(
            CliError::invalid_args("new table name must differ from the current name")
                .with_hint("Pass a different --new-name value."),
        );
    }
    if docs.iter().any(|other| same_name(&other.table, new_name)) {
        return Err(CliError::invalid_args(format!(
            "semantic model table already exists: {new_name}"
        ))
        .with_hint("Choose a new table name."));
    }
    if let Some(line) = doc.unsupported_table_line() {
        return Err(CliError::unsupported_feature(format!(
            "table rename would drop unsupported TMDL line: {line}"
        ))
        .with_hint("This table contains Desktop-authored table metadata this writer does not model; rename it in Desktop or recreate the generated table.")
        .with_suggested_command(format!(
            "powerbi-cli model tables show --project {} --handle {} --json",
            command_arg(&resolved.project_dir),
            crate::cli_support::shell_arg(&table_handle(&doc.table))
        )));
    }
    let references = collect_table_references(resolved, &old_name)?;
    if !references.is_empty() && !options.rename_references {
        return Err(CliError::validation_failed(format!(
            "table {} is referenced by {}; rerun with --rename-references to update all references",
            old_name,
            references.join(", ")
        ))
        .with_hint("Rename references updates relationship endpoints, DAX expressions, and variation table references in the same guarded mutation.")
        .with_suggested_command(format!(
            "powerbi-cli model tables rename --project {} --handle {} --new-name {} --rename-references --dry-run --json",
            command_arg(&resolved.project_dir),
            crate::cli_support::shell_arg(&table_handle(&doc.table)),
            crate::cli_support::shell_arg(new_name)
        )));
    }
    let old_path = doc.path.clone();
    let new_path = old_path
        .parent()
        .ok_or_else(|| {
            CliError::unexpected(format!("table path has no parent: {}", old_path.display()))
        })?
        .join(format!("{new_name}.tmdl"));
    if new_path.exists() {
        return Err(CliError::invalid_args(format!(
            "table target already exists: {}",
            new_path.display()
        )));
    }
    let renamed_text = doc.table_header_replaced(new_name)?;
    let mut changes = BTreeMap::new();
    changes.insert(old_path.clone(), renamed_text.clone());
    if options.rename_references {
        for path in referenced_tmdl_paths(resolved)? {
            let text = if path == old_path {
                renamed_text.clone()
            } else {
                read_utf8(&path, InputKind::ProjectText)?
            };
            let replaced = replace_table_references(&text, &old_name, new_name);
            if replaced != text {
                changes.insert(path, replaced);
            }
        }
    }
    let before_files = changes
        .keys()
        .map(|path| {
            let text = read_utf8(path, InputKind::ProjectText)?;
            Ok((path.clone(), text))
        })
        .collect::<CliResult<BTreeMap<_, _>>>()?;
    let dry_run = mode == MutationMode::DryRun;
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        for (path, text) in &changes {
            if let Err(error) = write_text_atomic(path, text) {
                let _ = restore_files(&before_files);
                return Err(error);
            }
        }
        if let Err(error) = fs::rename(&old_path, &new_path) {
            restore_files(&before_files)?;
            return Err(CliError::unexpected(format!(
                "rename {} to {}: {error}",
                old_path.display(),
                new_path.display()
            )));
        }
        match validate_project(resolved) {
            Ok(report) if report.errors.is_empty() => (Some(report), true),
            Ok(report) => {
                let _ = fs::rename(&new_path, &old_path);
                restore_files(&before_files)?;
                (Some(report), false)
            }
            Err(error) => {
                let _ = fs::rename(&new_path, &old_path);
                restore_files(&before_files)?;
                return Err(error);
            }
        }
    };
    let validation_ok = validation
        .as_ref()
        .is_none_or(|report| report.errors.is_empty());
    let project_arg = command_arg(&resolved.project_dir);
    let show = format!(
        "powerbi-cli model tables show --project {project_arg} --handle {} --json",
        crate::cli_support::shell_arg(&table_handle(new_name))
    );
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let change_values = changes
        .iter()
        .map(|(path, after)| {
            let display_path = if path == &old_path { &new_path } else { path };
            json!({
                "kind": "tmdl.table",
                "action": "rename",
                "path": canonical_display(display_path),
                "before": before_files.get(path),
                "after": after
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "schema": "powerbi-cli.model.tables.mutation.v1",
        "ok": validation_ok,
        "exitCode": if validation_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "action": "rename",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({"performed": true, "projectModified": false, "reason": "post-mutation validation failed; the original table file and references were restored"})),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "target": {"handle": table_handle(new_name), "previousHandle": table_handle(&old_name), "table": new_name, "previousTable": old_name, "path": canonical_display(&new_path), "referencesUpdated": options.rename_references, "references": references},
        "changes": change_values,
        "validation": validation.map(validation_json),
        "readbackCommand": show,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [show, inspect, validate]
    }))
}

fn delete_table(
    resolved: &crate::ResolvedProject,
    docs: &[TableDocument],
    options: &TableMutationOptions,
    mode: MutationMode,
) -> CliResult<Value> {
    let table_name = if let Some(handle) = options.handle.as_deref() {
        parse_table_handle(handle)?
    } else {
        options.table.clone().expect("validated table")
    };
    let doc = find_table(docs, &table_name)?;
    let references = collect_table_references(resolved, &doc.table)?;
    if !references.is_empty() {
        return Err(CliError::validation_failed(format!(
            "table {} cannot be deleted because it is referenced by {}",
            doc.table,
            references.join(", ")
        ))
        .with_hint("Delete or rewire relationships and DAX references before deleting the table.")
        .with_suggested_command(format!(
            "powerbi-cli model tables show --project {} --handle {} --json",
            command_arg(&resolved.project_dir),
            crate::cli_support::shell_arg(&table_handle(&doc.table))
        )));
    }
    if mode == MutationMode::InPlace
        && options.confirm.as_deref() != Some(table_handle(&doc.table).as_str())
    {
        return Err(CliError::invalid_args(format!(
            "in-place delete requires --confirm {}",
            table_handle(&doc.table)
        ))
        .with_hint(
            "Run delete with --dry-run first, then rerun with the exact confirm table handle.",
        )
        .with_suggested_command(format!(
            "powerbi-cli model tables delete --project {} --handle {} --dry-run --json",
            command_arg(&resolved.project_dir),
            crate::cli_support::shell_arg(&table_handle(&doc.table))
        )));
    }
    let before = read_utf8(&doc.path, InputKind::ProjectText)?;
    let dry_run = mode == MutationMode::DryRun;
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        fs::remove_file(&doc.path)
            .map_err(|err| CliError::unexpected(format!("remove {}: {err}", doc.path.display())))?;
        match validate_project(resolved) {
            Ok(report) if report.errors.is_empty() => (Some(report), true),
            Ok(report) => {
                write_text_atomic(&doc.path, &before)?;
                (Some(report), false)
            }
            Err(error) => {
                write_text_atomic(&doc.path, &before)?;
                return Err(error);
            }
        }
    };
    let validation_ok = validation
        .as_ref()
        .is_none_or(|report| report.errors.is_empty());
    let project_arg = command_arg(&resolved.project_dir);
    let list = format!("powerbi-cli model tables list --project {project_arg} --json");
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.model.tables.mutation.v1",
        "ok": validation_ok,
        "exitCode": if validation_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "action": "delete",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({"performed": true, "projectModified": false, "reason": "post-mutation validation failed; the original table file was restored"})),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "target": {"handle": table_handle(&doc.table), "table": doc.table, "path": canonical_display(&doc.path)},
        "changes": [{"kind": "tmdl.table", "action": "delete", "path": canonical_display(&doc.path), "before": before, "after": Value::Null}],
        "validation": validation.map(validation_json),
        "readbackCommand": list,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [list, inspect, validate]
    }))
}

fn table_json(doc: &TableDocument) -> Value {
    json!({
        "handle": table_handle(&doc.table),
        "name": doc.table,
        "path": canonical_display(&doc.path),
        "counts": {"columns": doc.columns.len(), "measures": doc.measures.len(), "partitions": doc.partitions.len()},
        "columns": doc.columns.iter().map(column_summary).collect::<Vec<_>>(),
        "measures": doc.measures.iter().map(|measure| json!({"handle": measure.handle(), "name": measure.name, "expression": measure.expression})).collect::<Vec<_>>(),
        "partitions": doc.partitions.iter().map(|partition| json!({"handle": partition.handle(), "name": partition.name, "expressionKind": partition.expression_kind, "mode": partition.mode, "sourceKind": partition.source_kind})).collect::<Vec<_>>()
    })
}

fn column_summary(column: &ColumnRecord) -> Value {
    json!({
        "handle": column.handle(),
        "name": column.name,
        "isCalculated": column.is_calculated(),
        "dataType": column.data_type,
        "sourceColumn": column.source_column,
        "sortByColumn": column.sort_by_column,
        "isHidden": column.is_hidden,
        "isKey": column.is_key
    })
}

fn table_tmdl(table: &str, columns: &[ColumnDefinitionInput]) -> String {
    let mut lines = Vec::new();
    lines.push(format!("table {}", crate::tmdl::tmdl_object_name(table)));
    lines.push(format!(
        "    lineageTag: {}",
        stable_guid(&format!("table:{table}"))
    ));
    lines.push(String::new());
    for input in columns {
        lines.extend(column_block_lines(table, &input.definition));
    }
    lines.push(format!(
        "    partition {} = m",
        crate::tmdl::tmdl_object_name(table)
    ));
    lines.push("        mode: import".to_string());
    lines.push("        source =".to_string());
    lines.push("            let".to_string());
    lines.push("                Source = #table(".to_string());
    let typed = columns
        .iter()
        .map(|input| {
            format!(
                "{} = {}",
                m_identifier(&input.definition.name),
                m_type(input.definition.data_type.as_deref().unwrap_or("string"))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("                    type table [{typed}],"));
    lines.push("                    {}".to_string());
    lines.push("                )".to_string());
    lines.push("            in".to_string());
    lines.push("                Source".to_string());
    lines.join("\n") + "\n"
}

fn calculated_table_tmdl(
    table: &str,
    expression: &str,
    columns: &[ColumnDefinitionInput],
) -> String {
    let mut lines = Vec::new();
    lines.push(format!("table {}", crate::tmdl::tmdl_object_name(table)));
    lines.push(format!(
        "    lineageTag: {}",
        stable_guid(&format!("calculated-table:{table}"))
    ));
    lines.push(String::new());
    for input in columns {
        lines.extend(column_block_lines(table, &input.definition));
    }
    lines.push(format!(
        "    partition {} = calculated",
        crate::tmdl::tmdl_object_name(table)
    ));
    lines.push("        mode: import".to_string());
    if expression.contains('\n') || expression.contains('\r') {
        lines.push("        source =".to_string());
        for line in expression.replace("\r\n", "\n").replace('\r', "\n").lines() {
            lines.push(format!("            {}", line.trim_end()));
        }
    } else {
        lines.push(format!("        source = {}", expression.trim()));
    }
    lines.join("\n") + "\n"
}

fn parse_table_list_args(args: &[String]) -> CliResult<TableListOptions> {
    let mut options = TableListOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(crate::cli_support::take_value(
                    args,
                    &mut i,
                    "--project",
                )?))
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model tables list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli model tables list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli model tables list --project <project-dir-or.pbip> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_table_show_args(args: &[String]) -> CliResult<TableShowOptions> {
    let mut options = TableShowOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(crate::cli_support::take_value(
                    args,
                    &mut i,
                    "--project",
                )?))
            }
            "--handle" => {
                options.handle = Some(crate::cli_support::take_value(args, &mut i, "--handle")?)
            }
            "--table" | "--name" => {
                options.table = Some(crate::cli_support::take_value(args, &mut i, "--table")?)
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model tables show flag: {other}"
                ))
                .with_hint("Use --handle or --table from `model tables list`.")
                .with_suggested_command(
                    "powerbi-cli model tables list --project <project-dir-or.pbip> --json",
                ));
            }
        }
    }
    if options.handle.is_some() == options.table.is_some() {
        return Err(CliError::invalid_args("model tables show requires exactly one of --handle or --table")
            .with_hint("Use a stable table handle from `model tables list`.")
            .with_suggested_command("powerbi-cli model tables show --project <project-dir-or.pbip> --handle <table-handle> --json"));
    }
    if let Some(handle) = options.handle.as_deref() {
        let _ = parse_table_handle(handle)?;
    }
    Ok(options)
}

fn parse_table_mutation_args(
    action: TableAction,
    args: &[String],
) -> CliResult<TableMutationOptions> {
    let mut options = TableMutationOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(crate::cli_support::take_value(
                    args,
                    &mut i,
                    "--project",
                )?))
            }
            "--table" => {
                options.table = Some(crate::cli_support::take_value(args, &mut i, "--table")?)
            }
            "--name" => {
                options.table = Some(crate::cli_support::take_value(args, &mut i, "--name")?)
            }
            "--handle" => {
                options.handle = Some(crate::cli_support::take_value(args, &mut i, "--handle")?)
            }
            "--new-name" | "--rename-to" => {
                options.new_name = Some(crate::cli_support::take_value(args, &mut i, "--new-name")?)
            }
            "--column" => {
                if options.columns_json_seen {
                    return Err(CliError::invalid_args(
                        "--column cannot be combined with --columns-json",
                    )
                    .with_hint(
                        "Use repeated --column flags or one --columns-json array, not both.",
                    ));
                }
                let name = crate::cli_support::take_value(args, &mut i, "--column")?;
                options.columns.push(ColumnDefinitionInput {
                    definition: ColumnDefinition {
                        name,
                        expression: None,
                        data_type: Some("string".to_string()),
                        lineage_tag: None,
                        format_string: None,
                        summarize_by: None,
                        sort_by_column: None,
                        source_column: None,
                        display_folder: None,
                        description: None,
                        is_hidden: false,
                        is_key: false,
                    },
                });
            }
            "--columns-json" => {
                if options.columns_json_seen {
                    return Err(CliError::invalid_args("pass --columns-json only once"));
                }
                if !options.columns.is_empty() {
                    return Err(CliError::invalid_args(
                        "--columns-json cannot be combined with --column",
                    )
                    .with_hint(
                        "Use repeated --column flags or one --columns-json array, not both.",
                    ));
                }
                options.columns_json_seen = true;
                let raw = crate::cli_support::take_value(args, &mut i, "--columns-json")?;
                options.columns = parse_columns_json(&raw)?;
            }
            "--data-type" | "--datatype" => {
                options.data_type =
                    Some(crate::cli_support::take_value(args, &mut i, "--data-type")?)
            }
            "--expression" => {
                if options.expression.is_some() {
                    return Err(CliError::invalid_args(
                        "--expression and --expression-file are mutually exclusive",
                    ));
                }
                options.expression = Some(crate::cli_support::take_value(
                    args,
                    &mut i,
                    "--expression",
                )?);
            }
            "--expression-file" => {
                if options.expression.is_some() {
                    return Err(CliError::invalid_args(
                        "--expression and --expression-file are mutually exclusive",
                    ));
                }
                let path = crate::cli_support::take_value(args, &mut i, "--expression-file")?;
                options.expression = Some(read_expression_file(&path)?);
            }
            "--rename-references" | "--update-references" => {
                options.rename_references = true;
                i += 1;
            }
            "--confirm" => {
                options.confirm = Some(crate::cli_support::take_value(args, &mut i, "--confirm")?)
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    &format!("model tables {}", action.as_str()),
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    &format!("model tables {}", action.as_str()),
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                options.out_dir = Some(PathBuf::from(crate::cli_support::take_value(
                    args,
                    &mut i,
                    "--out-dir",
                )?));
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    &format!("model tables {}", action.as_str()),
                )?;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model tables {} flag: {other}",
                    action.as_str()
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"model tables\"` for exact usage.",
                )
                .with_suggested_command(
                    "powerbi-cli model tables list --project <project-dir-or.pbip> --json",
                ));
            }
        }
    }
    match action {
        TableAction::Add => {
            if options.table.is_none() {
                return Err(CliError::invalid_args("model tables add requires --table <table>").with_suggested_command("powerbi-cli model tables add --project <project-dir-or.pbip> --table <table> --column <column> --dry-run --json"));
            }
            if options.handle.is_some() || options.new_name.is_some() {
                return Err(CliError::invalid_args(
                    "model tables add does not accept --handle or --new-name",
                ));
            }
            if options.rename_references {
                return Err(CliError::invalid_args(
                    "--rename-references is only valid for model tables rename",
                )
                .with_hint(
                    "Add creates a new table; use rename when updating an existing table name.",
                ));
            }
            if options.confirm.is_some() {
                return Err(CliError::invalid_args(
                    "--confirm is only valid for model tables delete",
                ));
            }
            if options.expression.is_some() {
                return Err(CliError::invalid_args(
                    "--expression is only valid for model tables add-calculated",
                ));
            }
            if let Some(data_type) = options.data_type.as_deref() {
                let normalized = normalize_table_type(data_type)?;
                for input in &mut options.columns {
                    input.definition.data_type = Some(normalized.clone());
                }
            }
            validate_column_definitions(&options.columns)?;
        }
        TableAction::AddCalculated => {
            if options.table.is_none() {
                return Err(CliError::invalid_args("model tables add-calculated requires --table <table>")
                    .with_suggested_command("powerbi-cli model tables add-calculated --project <project-dir-or.pbip> --table <table> --expression <dax> --dry-run --json"));
            }
            if options.handle.is_some() || options.new_name.is_some() {
                return Err(CliError::invalid_args(
                    "model tables add-calculated does not accept --handle or --new-name",
                ));
            }
            if options.rename_references || options.confirm.is_some() || options.data_type.is_some()
            {
                return Err(CliError::invalid_args(
                    "model tables add-calculated accepts --table, --expression, optional columns, and an output mode",
                ));
            }
            let expression = options.expression.as_deref().ok_or_else(|| {
                CliError::invalid_args(
                    "model tables add-calculated requires --expression or --expression-file",
                )
                .with_hint("Provide the calculated table DAX as inline text or a bounded UTF-8 file.")
                .with_suggested_command("powerbi-cli model tables add-calculated --project <project-dir-or.pbip> --table <table> --expression \"FILTER('FactSales', 'FactSales'[Revenue] > 0)\" --dry-run --json")
            })?;
            validate_text(expression, InputKind::SourceText)?;
            if expression.trim().is_empty() {
                return Err(CliError::invalid_args(
                    "calculated table expression must not be empty",
                ));
            }
            if contains_credential_like_text_str(expression) {
                return Err(CliError::invalid_args(
                    "calculated table expression contains credential-like text",
                )
                .with_hint("Remove credentials and keep the project offline-safe."));
            }
            validate_column_definitions(&options.columns)?;
        }
        TableAction::Rename => {
            if options.handle.is_none() && options.table.is_none() {
                return Err(CliError::invalid_args("model tables rename requires --handle or --table").with_suggested_command("powerbi-cli model tables rename --project <project-dir-or.pbip> --handle <table-handle> --new-name <table> --dry-run --json"));
            }
            if options.new_name.is_none() {
                return Err(CliError::invalid_args("model tables rename requires --new-name").with_suggested_command("powerbi-cli model tables rename --project <project-dir-or.pbip> --handle <table-handle> --new-name <table> --dry-run --json"));
            }
            if options.handle.is_some() && options.table.is_some() {
                return Err(CliError::invalid_args(
                    "model tables rename accepts one selector: --handle or --table",
                ));
            }
            validate_table_name(
                options.new_name.as_deref().expect("validated new name"),
                "--new-name",
            )?;
            if options.confirm.is_some() {
                return Err(CliError::invalid_args(
                    "--confirm is only valid for model tables delete",
                ));
            }
            if !options.columns.is_empty()
                || options.data_type.is_some()
                || options.expression.is_some()
            {
                return Err(CliError::invalid_args(
                    "model tables rename accepts only a table selector, --new-name, --rename-references, and an output mode",
                )
                .with_hint("Use model tables add to define columns for a new table."));
            }
        }
        TableAction::Delete => {
            if options.handle.is_none() && options.table.is_none() {
                return Err(CliError::invalid_args("model tables delete requires --handle or --table").with_suggested_command("powerbi-cli model tables delete --project <project-dir-or.pbip> --handle <table-handle> --dry-run --json"));
            }
            if options.handle.is_some() && options.table.is_some() {
                return Err(CliError::invalid_args(
                    "model tables delete accepts one selector: --handle or --table",
                ));
            }
            if options.new_name.is_some() || options.rename_references {
                return Err(CliError::invalid_args(
                    "--new-name and --rename-references are only valid for table rename",
                ));
            }
            if !options.columns.is_empty()
                || options.data_type.is_some()
                || options.expression.is_some()
            {
                return Err(CliError::invalid_args(
                    "model tables delete accepts only a table selector, output mode, and optional --confirm",
                )
                .with_hint("Use model tables add or rename to change table metadata."));
            }
            if let Some(handle) = options.handle.as_deref() {
                let _ = parse_table_handle(handle)?;
            }
            if options.confirm.is_some() && options.mode != Some(MutationMode::InPlace) {
                return Err(CliError::invalid_args(
                    "--confirm is only valid with --in-place model tables delete",
                )
                .with_hint(
                    "Use --dry-run first, then pass the exact handle with --in-place --confirm.",
                ));
            }
        }
    }
    if let Some(table) = options.table.as_deref() {
        validate_table_name(table, "--table")?;
    }
    Ok(options)
}

fn validate_column_definitions(columns: &[ColumnDefinitionInput]) -> CliResult<()> {
    let mut names = std::collections::BTreeSet::new();
    for input in columns {
        let definition = &input.definition;
        validate_column_name(&definition.name, "column name")?;
        if !names.insert(definition.name.to_ascii_lowercase()) {
            return Err(CliError::invalid_args(format!(
                "column names must be unique: {}",
                definition.name
            )));
        }
        if definition.expression.is_some() && definition.source_column.is_some() {
            return Err(CliError::invalid_args(format!(
                "calculated column {} cannot combine expression with sourceColumn",
                definition.name
            ))
            .with_hint("Use expression for a calculated column or sourceColumn for a base column, not both."));
        }
        if let Some(sort_by) = definition.sort_by_column.as_deref() {
            if same_name(&definition.name, sort_by) {
                return Err(CliError::invalid_args(format!(
                    "column {} cannot sort by itself",
                    definition.name
                ))
                .with_hint("Choose a different column in the new table or omit sortByColumn."));
            }
            if !columns
                .iter()
                .any(|candidate| same_name(&candidate.definition.name, sort_by))
            {
                return Err(CliError::invalid_args(format!(
                    "column {} sortByColumn target does not exist in the new table: {}",
                    definition.name, sort_by
                ))
                .with_hint("Define the sort column in --columns-json before referencing it."));
            }
        }
    }
    Ok(())
}

fn parse_columns_json(raw: &str) -> CliResult<Vec<ColumnDefinitionInput>> {
    let value: Value = serde_json::from_str(raw).map_err(|err| {
        CliError::invalid_args(format!("--columns-json is not valid JSON: {err}"))
    })?;
    let items = value
        .as_array()
        .ok_or_else(|| CliError::invalid_args("--columns-json must be a JSON array"))?;
    let mut result = Vec::new();
    for item in items {
        let definition = if let Some(name) = item.as_str() {
            ColumnDefinition {
                name: name.to_string(),
                expression: None,
                data_type: Some("string".to_string()),
                lineage_tag: None,
                format_string: None,
                summarize_by: None,
                sort_by_column: None,
                source_column: None,
                display_folder: None,
                description: None,
                is_hidden: false,
                is_key: false,
            }
        } else {
            let object = item.as_object().ok_or_else(|| {
                CliError::invalid_args("--columns-json items must be strings or objects")
            })?;
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| CliError::invalid_args("column objects require a string name"))?;
            ColumnDefinition {
                name: name.to_string(),
                expression: object
                    .get("expression")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                data_type: Some(normalize_table_type(
                    object
                        .get("dataType")
                        .or_else(|| object.get("data_type"))
                        .and_then(Value::as_str)
                        .unwrap_or("string"),
                )?),
                lineage_tag: object
                    .get("lineageTag")
                    .or_else(|| object.get("lineage_tag"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                format_string: object
                    .get("formatString")
                    .or_else(|| object.get("format_string"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                summarize_by: object
                    .get("summarizeBy")
                    .or_else(|| object.get("summarize_by"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                sort_by_column: object
                    .get("sortByColumn")
                    .or_else(|| object.get("sort_by_column"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                source_column: object
                    .get("sourceColumn")
                    .or_else(|| object.get("source_column"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                display_folder: object
                    .get("displayFolder")
                    .or_else(|| object.get("display_folder"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                description: object
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                is_hidden: object
                    .get("isHidden")
                    .or_else(|| object.get("is_hidden"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                is_key: object
                    .get("isKey")
                    .or_else(|| object.get("is_key"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        };
        validate_column_name(&definition.name, "column name")?;
        result.push(ColumnDefinitionInput { definition });
    }
    if result.is_empty() {
        return Err(CliError::invalid_args(
            "--columns-json must contain at least one column",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for input in &result {
        if !seen.insert(input.definition.name.to_ascii_lowercase()) {
            return Err(CliError::invalid_args(format!(
                "column names must be unique: {}",
                input.definition.name
            )));
        }
    }
    Ok(result)
}

fn parse_table_handle(handle: &str) -> CliResult<String> {
    let Some(encoded) = handle.strip_prefix("table:") else {
        return Err(CliError::invalid_args(format!("invalid table handle: {handle}"))
            .with_hint("Table handles look like `table:<table>`; literal `%` and `:` are encoded as `%25` and `%3A`.")
            .with_suggested_command("powerbi-cli model tables list --project <project-dir-or.pbip> --json"));
    };
    if encoded.is_empty() || encoded.contains(':') {
        return Err(CliError::invalid_args(format!(
            "invalid table handle: {handle}"
        )));
    }
    decode_handle_component(encoded)
        .map_err(|_| CliError::invalid_args(format!("invalid table handle: {handle}")))
}

fn decode_handle_component(value: &str) -> Result<String, ()> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            result.push(ch);
            continue;
        }
        let first = chars.next().ok_or(())?;
        let second = chars.next().ok_or(())?;
        match (first, second) {
            ('2', '5') => result.push('%'),
            ('3', 'A' | 'a') => result.push(':'),
            _ => return Err(()),
        }
    }
    Ok(result)
}

fn validate_table_name(name: &str, flag: &str) -> CliResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed != name || name.chars().count() > 100 {
        return Err(CliError::invalid_args(format!(
            "{flag} must be a non-empty name of at most 100 characters without surrounding whitespace"
        )));
    }
    if name.ends_with('.')
        || name
            .chars()
            .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
    {
        return Err(CliError::invalid_args(format!(
            "{flag} is not a portable table name: {name}"
        )));
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err(CliError::invalid_args(format!(
            "{flag} uses a filesystem-reserved name: {name}"
        )));
    }
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
            "{flag} must not contain control characters: {name}"
        )));
    }
    Ok(())
}

fn normalize_table_type(value: &str) -> CliResult<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "string" | "text" => Ok("string".to_string()),
        "int64" | "integer" | "whole" | "whole-number" => Ok("int64".to_string()),
        "double" => Ok("double".to_string()),
        "decimal" | "currency" => Ok("decimal".to_string()),
        "date" | "datetime" | "date-time" => Ok("dateTime".to_string()),
        "boolean" | "bool" => Ok("boolean".to_string()),
        other => Err(CliError::unsupported_feature(format!("unsupported model table column data type: {other}"))
            .with_hint("Use string, int64, double, decimal, dateTime, or boolean.")
            .with_suggested_command("powerbi-cli model tables add --project <project-dir-or.pbip> --table <table> --column <column> --data-type string --dry-run --json")),
    }
}

fn stable_guid(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let mut other = 0xcbf29ce484222325_u64;
    for byte in format!("{value}:powerbi-cli").as_bytes() {
        other ^= u64::from(*byte);
        other = other.wrapping_mul(0x100000001b3);
    }
    let hex = format!("{hash:016x}{other:016x}");
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[16..19],
        &hex[19..31]
    )
}

fn read_expression_file(path: &str) -> CliResult<String> {
    let text = if path == "-" {
        read_utf8_stream(
            &mut io::stdin(),
            InputKind::SourceText,
            "calculated table expression",
        )?
    } else {
        read_utf8(std::path::Path::new(path), InputKind::SourceText)?
    };
    let expression = text
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if expression.trim().is_empty() {
        return Err(CliError::invalid_args("calculated table expression file is empty")
            .with_hint("Provide a DAX table expression.")
            .with_suggested_command("powerbi-cli model tables add-calculated --project <project-dir-or.pbip> --table <table> --expression-file <dax.txt> --dry-run --json"));
    }
    Ok(expression)
}

fn m_identifier(value: &str) -> String {
    if value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        value.to_string()
    } else {
        format!("#\"{}\"", value.replace('"', "\"\""))
    }
}

fn m_type(value: &str) -> &'static str {
    match value {
        // The offline safety parser intentionally accepts the canonical M
        // scalar type names used by scaffolded tables (`number`, `date`, ...)
        // rather than the richer `Int64.Type` alias.
        "int64" => "number",
        "double" => "number",
        "decimal" => "number",
        "dateTime" => "datetime",
        "boolean" => "logical",
        _ => "text",
    }
}

fn validation_json(report: crate::ValidationReport) -> Value {
    json!({"ok": report.errors.is_empty(), "warnings": report.warnings, "errors": report.errors, "counts": {"tables": report.tables, "measures": report.measures, "relationships": report.relationships, "pages": report.pages, "visuals": report.visuals}})
}

fn referenced_tmdl_paths(resolved: &crate::ResolvedProject) -> CliResult<Vec<PathBuf>> {
    let tables_dir = resolved
        .semantic_model_dir
        .join("definition")
        .join("tables");
    let mut paths = fs::read_dir(&tables_dir)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", tables_dir.display())))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|err| CliError::unexpected(format!("read table entry: {err}")))
        })
        .collect::<CliResult<Vec<_>>>()?;
    paths.retain(|path| path.extension().and_then(|value| value.to_str()) == Some("tmdl"));
    let relationships = resolved
        .semantic_model_dir
        .join("definition")
        .join("relationships.tmdl");
    if relationships.is_file() {
        paths.push(relationships);
    }
    paths.sort();
    Ok(paths)
}

fn reference_patterns(table: &str) -> Vec<String> {
    let quoted = format!("'{}'", table.replace('\'', "''"));
    let mut patterns = vec![format!("{quoted}["), format!("{quoted}.")];
    if is_simple_identifier(table) {
        patterns.extend([format!("{table}["), format!("{table}.")]);
    }
    patterns
}

fn is_simple_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn collect_table_references(
    resolved: &crate::ResolvedProject,
    table: &str,
) -> CliResult<Vec<String>> {
    let patterns = reference_patterns(table);
    let mut references = Vec::new();
    for path in referenced_tmdl_paths(resolved)? {
        let text = read_utf8(&path, InputKind::ProjectText)?;
        for (index, line) in text.lines().enumerate() {
            if patterns.iter().any(|pattern| line.contains(pattern)) {
                references.push(format!("{}:{}", path.display(), index + 1));
            }
        }
    }
    references.sort();
    references.dedup();
    Ok(references)
}

fn replace_table_references(text: &str, old: &str, new: &str) -> String {
    let old_quoted = format!("'{}'", old.replace('\'', "''"));
    let new_quoted = format!("'{}'", new.replace('\'', "''"));
    let mut result = text.replace(&format!("{old_quoted}["), &format!("{new_quoted}["));
    result = result.replace(&format!("{old_quoted}."), &format!("{new_quoted}."));
    if is_simple_identifier(old) {
        result = result.replace(&format!("{old}["), &format!("{new}["));
        result = result.replace(&format!("{old}."), &format!("{new}."));
    }
    result
}

fn restore_files(files: &BTreeMap<PathBuf, String>) -> CliResult<()> {
    for (path, text) in files {
        write_text_atomic(path, text)?;
    }
    Ok(())
}
