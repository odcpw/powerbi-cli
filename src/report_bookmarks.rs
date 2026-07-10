use crate::cli_support::{
    MutationMode, mode_name, require_mode, set_mode, shell_arg, target_project,
};
use crate::pbir_bookmarks::{
    ReportBookmarkRecord, bookmark_record_json, bookmarks_metadata_json, list_report_bookmarks,
};
use crate::project_io::{write_json_atomic, write_json_pretty};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
    include_raw: bool,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    include_raw: bool,
}

#[derive(Debug, Default)]
struct MutationOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    display_name: Option<String>,
    order: Vec<String>,
    confirm: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn bookmarks_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(
            CliError::invalid_args("report bookmarks requires a subcommand: list or show")
                .with_hint(
                    "Run `powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json",
                ),
        );
    };

    match action.as_str() {
        "list" | "ls" => list_bookmarks(rest),
        "show" | "get" => show_bookmark(rest),
        "set-display-name" | "display-name" | "rename" => set_display_name(rest),
        "delete" | "remove" => delete_bookmark(rest),
        "reorder" | "order" => reorder_bookmarks(rest),
        "add" | "create" | "update" | "set" | "capture" => Err(
            CliError::unsupported_feature(
                "bookmark state capture or creation is not implemented",
            )
            .with_hint("Use list/show plus metadata mutations. Capturing bookmark state requires Desktop-authored oracle fixtures first.")
            .with_suggested_command(
                "powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json",
            )
            .with_suggested_command(
                "powerbi-cli features list --for report.bookmark-mutations --json",
            ),
        ),
        _ => Err(CliError::invalid_args(format!(
            "unknown report bookmarks command: {action}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"report bookmarks\"` for supported bookmark commands.")
        .with_suggested_command("powerbi-cli --json capabilities --for \"report bookmarks\"")),
    }
}

fn set_display_name(args: &[String]) -> CliResult<Value> {
    let options = parse_mutation_args("report bookmarks set-display-name", args)?;
    let display_name = options.display_name.clone().ok_or_else(|| {
        CliError::invalid_args("report bookmarks set-display-name requires --display-name <text>")
            .with_hint("Use list/show to get the exact bookmark handle first.")
            .with_suggested_command(
                "powerbi-cli report bookmarks set-display-name --project <project-dir-or.pbip> --handle <bookmark-handle> --display-name <text> --dry-run --json",
            )
    })?;
    crate::cli_support::preflight_out_dir(args, set_display_name)?;
    let (target_resolved, mode) =
        prepare_bookmark_mutation(&options, "report bookmarks set-display-name")?;
    let (records, metadata, _) = list_report_bookmarks(&target_resolved)?;
    let record = find_bookmark_record(
        &records,
        options.handle.as_deref(),
        "report bookmarks set-display-name",
    )?;
    let mut raw = read_json_value(&record.path)?;
    let before_display = raw["displayName"].clone();
    raw["displayName"] = Value::String(display_name.clone());

    let metadata_path = metadata.path.clone();
    let mut metadata_after = metadata_path
        .as_ref()
        .map(|path| read_json_value(path))
        .transpose()?;
    if let Some(metadata_json) = metadata_after.as_mut() {
        set_metadata_display_name(metadata_json, &record.name, &display_name);
    }

    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&record.path, &raw)?;
        if let (Some(path), Some(metadata_json)) = (metadata_path.as_ref(), metadata_after.as_ref())
        {
            write_json_atomic(path, metadata_json)?;
        }
    }

    let mut after_record = bookmark_record_json(record, true);
    after_record["displayName"] = Value::String(display_name.clone());
    after_record["raw"]["displayName"] = Value::String(display_name.clone());
    mutation_response(
        &target_resolved,
        mode,
        "set-display-name",
        json!({
            "bookmark": bookmark_record_json(record, true),
            "after": after_record
        }),
        vec![json!({
            "path": canonical_display(&record.path),
            "pointer": "/displayName",
            "before": before_display,
            "after": display_name
        })],
    )
}

fn delete_bookmark(args: &[String]) -> CliResult<Value> {
    let options = parse_mutation_args("report bookmarks delete", args)?;
    crate::cli_support::preflight_out_dir(args, delete_bookmark)?;
    let (target_resolved, mode) = prepare_bookmark_mutation(&options, "report bookmarks delete")?;
    let (records, metadata, _) = list_report_bookmarks(&target_resolved)?;
    let record = find_bookmark_record(
        &records,
        options.handle.as_deref(),
        "report bookmarks delete",
    )?;
    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&record.handle) {
        return Err(CliError::invalid_args(format!(
            "in-place bookmark deletion requires --confirm {}",
            record.handle
        ))
        .with_hint("Run the same command with --dry-run first, then confirm the exact bookmark handle.")
        .with_suggested_command(format!(
            "powerbi-cli report bookmarks delete --project {} --handle {} --in-place --confirm {} --json",
            command_arg(&target_resolved.project_dir),
            shell_arg(&record.handle),
            shell_arg(&record.handle)
        )));
    }

    let metadata_path = metadata.path.clone();
    let mut metadata_after = metadata_path
        .as_ref()
        .map(|path| read_json_value(path))
        .transpose()?;
    if let Some(metadata_json) = metadata_after.as_mut() {
        remove_bookmark_from_metadata(metadata_json, &record.name);
    }

    if !matches!(mode, MutationMode::DryRun) {
        fs::remove_file(&record.path).map_err(|err| {
            CliError::unexpected(format!(
                "remove bookmark file {}: {err}",
                record.path.display()
            ))
        })?;
        if let (Some(path), Some(metadata_json)) = (metadata_path.as_ref(), metadata_after.as_ref())
        {
            write_json_atomic(path, metadata_json)?;
        }
    }

    mutation_response(
        &target_resolved,
        mode,
        "delete",
        json!({
            "bookmark": bookmark_record_json(record, true)
        }),
        vec![json!({
            "path": canonical_display(&record.path),
            "operation": "delete-file"
        })],
    )
}

fn reorder_bookmarks(args: &[String]) -> CliResult<Value> {
    let options = parse_mutation_args("report bookmarks reorder", args)?;
    if options.order.is_empty() {
        return Err(CliError::invalid_args("report bookmarks reorder requires --order <bookmark-handle,...>")
            .with_hint("The order must include every bookmark exactly once.")
            .with_suggested_command("powerbi-cli report bookmarks reorder --project <project-dir-or.pbip> --order bookmark:A,bookmark:B --dry-run --json"));
    }
    crate::cli_support::preflight_out_dir(args, reorder_bookmarks)?;
    let (target_resolved, mode) = prepare_bookmark_mutation(&options, "report bookmarks reorder")?;
    let (records, metadata, _) = list_report_bookmarks(&target_resolved)?;
    if records.iter().any(|record| record.group.is_some()) {
        return Err(CliError::unsupported_feature(
            "bookmark reorder for grouped bookmark metadata is not implemented",
        )
        .with_hint("Grouped bookmark metadata is preserved read-only until Desktop-backed fixture cases are added.")
        .with_suggested_command(format!(
            "powerbi-cli report bookmarks list --project {} --json",
            command_arg(&target_resolved.project_dir)
        )));
    }

    let name_by_handle = records
        .iter()
        .map(|record| (record.handle.clone(), record.name.clone()))
        .collect::<BTreeMap<_, _>>();
    let all_names = records
        .iter()
        .map(|record| record.name.clone())
        .collect::<BTreeSet<_>>();
    let mut requested_names = Vec::new();
    for item in &options.order {
        let name = if let Some(name) = name_by_handle.get(item) {
            name.clone()
        } else if let Some(stripped) = item.strip_prefix("bookmark:") {
            stripped.to_string()
        } else {
            item.clone()
        };
        requested_names.push(name);
    }
    validate_complete_order(&requested_names, &all_names)?;

    let metadata_path = target_resolved
        .report_dir
        .join("definition")
        .join("bookmarks")
        .join("bookmarks.json");
    let before_metadata = if metadata_path.is_file() {
        read_json_value(&metadata_path)?
    } else {
        json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmarksMetadata/1.0.0/schema.json",
            "items": records.iter().map(|record| json!({"name": record.name})).collect::<Vec<_>>()
        })
    };
    let after_metadata = reordered_metadata(&before_metadata, &requested_names, &records);

    if !matches!(mode, MutationMode::DryRun) {
        if metadata_path.is_file() {
            write_json_atomic(&metadata_path, &after_metadata)?;
        } else {
            write_json_pretty(&metadata_path, &after_metadata)?;
        }
    }

    mutation_response(
        &target_resolved,
        mode,
        "reorder",
        json!({
            "beforeOrder": bookmarks_metadata_json(&metadata)["orderedNames"].clone(),
            "afterOrder": requested_names
        }),
        vec![json!({
            "path": canonical_display(&metadata_path),
            "pointer": "/items",
            "before": before_metadata["items"],
            "after": after_metadata["items"]
        })],
    )
}

fn list_bookmarks(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "report bookmarks list")?;
    let resolved = resolve_project(&project)?;
    let (records, metadata, validation) = list_report_bookmarks(&resolved)?;
    let bookmarks = records
        .iter()
        .map(|record| bookmark_record_json(record, options.include_raw))
        .collect::<Vec<_>>();

    Ok(json!({
        "schema": "powerbi-cli.report.bookmarks.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "bookmarksDir": canonical_display(&resolved.report_dir.join("definition").join("bookmarks")),
        "bookmarksMetadata": bookmarks_metadata_json(&metadata),
        "bookmarkDiagnostics": metadata.diagnostics,
        "counts": bookmark_counts(&records),
        "bookmarks": bookmarks,
        "next": [
            format!("powerbi-cli report bookmarks show --project {} --handle <bookmark-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report pages list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report visuals list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report filters list --project {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": validation.warnings,
        "errors": validation.errors
    }))
}

fn show_bookmark(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "report bookmarks show")?;
    let handle = options.handle.ok_or_else(|| {
        CliError::invalid_args("report bookmarks show requires --handle <bookmark-handle>")
            .with_hint("Use `report bookmarks list` to get stable bookmark handles.")
            .with_suggested_command(
                "powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json",
            )
    })?;
    let resolved = resolve_project(&project)?;
    let (records, metadata, validation) = list_report_bookmarks(&resolved)?;
    let record = records
        .iter()
        .find(|record| record.handle == handle)
        .ok_or_else(|| {
            CliError::invalid_args("bookmark not found")
                .with_hint("Use `report bookmarks list` to get stable bookmark handles.")
                .with_suggested_command(format!(
                    "powerbi-cli report bookmarks list --project {} --json",
                    command_arg(&resolved.project_dir)
                ))
        })?;
    let readback = format!(
        "powerbi-cli report bookmarks list --project {} --json",
        command_arg(&resolved.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );

    Ok(json!({
        "schema": "powerbi-cli.report.bookmarks.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "bookmarksDir": canonical_display(&resolved.report_dir.join("definition").join("bookmarks")),
        "bookmarksMetadata": bookmarks_metadata_json(&metadata),
        "bookmarkDiagnostics": metadata.diagnostics,
        "bookmark": bookmark_record_json(record, options.include_raw),
        "readbackCommand": readback,
        "validateCommand": validate,
        "next": [readback, validate],
        "warnings": validation.warnings,
        "errors": validation.errors
    }))
}

fn parse_list_args(args: &[String]) -> CliResult<ListOptions> {
    let mut options = ListOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report bookmarks list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_show_args(args: &[String]) -> CliResult<ShowOptions> {
    let mut options = ShowOptions {
        include_raw: true,
        ..ShowOptions::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--no-raw" | "--noRaw" => {
                options.include_raw = false;
                i += 1;
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report bookmarks show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report bookmarks show --project <project-dir-or.pbip> --handle <bookmark-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report bookmarks show --project <project-dir-or.pbip> --handle <bookmark-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn bookmark_counts(records: &[ReportBookmarkRecord]) -> Value {
    json!({
        "bookmarks": records.len(),
        "groups": records.iter().filter(|record| record.group.is_some()).count(),
        "targetVisualBookmarks": records.iter().filter(|record| {
            record.options["targetVisualCount"].as_u64().unwrap_or_default() > 0
        }).count(),
        "possibleDataValueBookmarks": records.iter().filter(|record| record.may_contain_data_values).count(),
        "unsupported": records.iter().filter(|record| record.unsupported).count()
    })
}

fn parse_mutation_args(command: &str, args: &[String]) -> CliResult<MutationOptions> {
    let mut options = MutationOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--display-name" | "--displayName" => {
                options.display_name = Some(take_value(args, &mut i, "--display-name")?);
            }
            "--order" => {
                let value = take_value(args, &mut i, "--order")?;
                options.order.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToOwned::to_owned),
                );
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun, command)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace, command)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir, command)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown {command} flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report bookmarks\"` for exact usage.")
                .with_suggested_command("powerbi-cli --json capabilities --for \"report bookmarks\""));
            }
        }
    }
    Ok(options)
}

fn prepare_bookmark_mutation(
    options: &MutationOptions,
    command: &str,
) -> CliResult<(ResolvedProject, MutationMode)> {
    let source_project = required_project(options.project.clone(), command)?;
    let mode = require_mode(options.mode, command)?;
    let source_resolved = resolve_project(&source_project)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    Ok((target_resolved, mode))
}

fn find_bookmark_record<'a>(
    records: &'a [ReportBookmarkRecord],
    handle: Option<&str>,
    command: &str,
) -> CliResult<&'a ReportBookmarkRecord> {
    let handle = handle.ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires --handle <bookmark-handle>"))
            .with_hint("Use `report bookmarks list` to get stable bookmark handles.")
            .with_suggested_command(format!(
                "powerbi-cli {command} --project <project-dir-or.pbip> --handle <bookmark-handle> --dry-run --json"
            ))
    })?;
    records
        .iter()
        .find(|record| record.handle == handle || record.name == handle)
        .ok_or_else(|| {
            CliError::invalid_args("bookmark not found")
                .with_hint("Use `report bookmarks list` to get stable bookmark handles.")
                .with_suggested_command(
                    "powerbi-cli report bookmarks list --project <project-dir-or.pbip> --json",
                )
        })
}

fn set_metadata_display_name(metadata_json: &mut Value, bookmark_name: &str, display_name: &str) {
    if let Some(items) = metadata_json["items"].as_array_mut() {
        for item in items {
            if item["name"].as_str() == Some(bookmark_name) {
                item["displayName"] = Value::String(display_name.to_string());
            }
        }
    }
}

fn remove_bookmark_from_metadata(metadata_json: &mut Value, bookmark_name: &str) {
    let Some(items) = metadata_json["items"].as_array_mut() else {
        return;
    };
    let mut next = Vec::new();
    for mut item in std::mem::take(items) {
        if item["name"].as_str() == Some(bookmark_name) && item.get("children").is_none() {
            continue;
        }
        if let Some(children) = item["children"].as_array_mut() {
            children.retain(|child| child.as_str() != Some(bookmark_name));
        }
        next.push(item);
    }
    *items = next;
}

fn validate_complete_order(
    requested_names: &[String],
    all_names: &BTreeSet<String>,
) -> CliResult<()> {
    let requested = requested_names.iter().cloned().collect::<BTreeSet<_>>();
    if requested.len() != requested_names.len() {
        return Err(CliError::invalid_args(
            "bookmark reorder --order contains duplicate bookmarks",
        )
        .with_hint("The order must include every bookmark exactly once."));
    }
    if &requested != all_names {
        let missing = all_names
            .difference(&requested)
            .cloned()
            .collect::<Vec<_>>();
        let unknown = requested.difference(all_names).cloned().collect::<Vec<_>>();
        return Err(CliError::invalid_args(
            "bookmark reorder --order must include every bookmark exactly once",
        )
        .with_hint(format!(
            "missing: [{}]; unknown: [{}]",
            missing.join(", "),
            unknown.join(", ")
        )));
    }
    Ok(())
}

fn reordered_metadata(
    before_metadata: &Value,
    requested_names: &[String],
    records: &[ReportBookmarkRecord],
) -> Value {
    let existing = before_metadata["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| Some((item["name"].as_str()?.to_string(), item.clone())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let record_display = records
        .iter()
        .map(|record| (record.name.clone(), record.display_name.clone()))
        .collect::<BTreeMap<_, _>>();
    let items = requested_names
        .iter()
        .map(|name| {
            existing.get(name).cloned().unwrap_or_else(|| {
                json!({
                    "name": name,
                    "displayName": record_display.get(name).cloned().unwrap_or_else(|| name.clone())
                })
            })
        })
        .collect::<Vec<_>>();
    let mut after = before_metadata.clone();
    after["items"] = Value::Array(items);
    after
}

fn mutation_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    action: &str,
    target: Value,
    changes: Vec<Value>,
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
    let readback = format!("powerbi-cli report bookmarks list --project {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.report.bookmarks.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": action,
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": target,
        "changes": changes,
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors
        })),
        "readbackCommand": readback,
        "validateCommand": validate,
        "next": [readback, validate]
    }))
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

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for \"report bookmarks\"` for exact usage.",
            )
            .with_suggested_command("powerbi-cli --json capabilities --for \"report bookmarks\"")
    })?;
    *index += 2;
    Ok(value.clone())
}
