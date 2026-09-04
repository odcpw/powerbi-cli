//! Guarded authoring for semantic-model named M expressions.
//!
//! Discovery and line ranges are supplied by `model_advanced`; this module
//! only validates mutation inputs and applies newline-aware block edits. That
//! keeps one parser responsible for both readback and authoring.

use crate::cli_support::{
    MutationMode, mode_name, preflight_out_dir, require_mode_with_contract, required_project,
    set_mode_with_contract, take_value, target_project,
};
use crate::input_safety::{InputKind, read_utf8, read_utf8_stream, validate_text};
use crate::model_advanced::{AdvancedFamily, AdvancedRecord, load_family_records};
use crate::project_io::write_text_atomic_validated;
use crate::safety_scan::contains_credential_like_text_str;
use crate::tmdl::{
    expression_handle, load_tmdl_lines, parse_expression_handle, render_tmdl_lines,
    unsupported_named_expression_line,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Default)]
struct Options {
    project: Option<PathBuf>,
    handle: Option<String>,
    name: Option<String>,
    expression: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    confirm: Option<String>,
}

#[derive(Debug)]
struct Plan {
    path: PathBuf,
    name: String,
    handle: String,
    before: Option<String>,
    after: Option<String>,
    new_text: String,
}

pub(crate) fn expressions_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model expressions requires a subcommand: list, show, add, update, delete",
        )
        .with_hint("Use list/show for readback or add/update/delete for guarded named-expression authoring.")
        .with_suggested_command(
            "powerbi-cli model expressions list --project <project-dir-or.pbip> --json",
        ));
    };
    match action.as_str() {
        // Keep the established readback implementation and parser in
        // model_advanced; only mutations are owned here.
        "list" | "ls" | "show" | "get" => {
            crate::model_advanced::advanced_model_command("expressions", args)
        }
        "add" | "create" => mutate(Action::Add, rest),
        "update" => mutate(Action::Update, rest),
        "delete" | "remove" => mutate(Action::Delete, rest),
        other => Err(CliError::invalid_args(format!(
            "unknown model expressions command: {other}"
        ))
        .with_hint("Run `powerbi-cli --json capabilities --for \"model expressions\"` for exact usage.")
        .with_suggested_command(
            "powerbi-cli model expressions list --project <project-dir-or.pbip> --json",
        )),
    }
}

fn mutate(action: Action, args: &[String]) -> CliResult<Value> {
    let options = parse_options(action, args)?;
    let command = format!("model expressions {}", action.as_str());
    let source_project = required_project(options.project.clone(), &command)?;
    let mode = require_mode_with_contract(
        options.mode,
        &command,
        "Start with `--dry-run`; rerun with `--out-dir` or guarded `--in-place` after review.",
        format!("powerbi-cli {command} --project <project-dir-or.pbip> --dry-run --json"),
    )?;
    if mode == MutationMode::OutDir {
        preflight_out_dir(args, |dry_args| mutate(action, dry_args))?;
    }
    let source_resolved = resolve_project(&source_project)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let records = load_family_records(
        &target_resolved.semantic_model_dir,
        AdvancedFamily::Expressions,
    )?;
    let plan = build_plan(&target_resolved, &records, action, &options)?;
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
    let project_arg = command_arg(&target_resolved.project_dir);
    let readback = match action {
        Action::Delete => {
            format!("powerbi-cli model expressions list --project {project_arg} --json")
        }
        Action::Add | Action::Update => format!(
            "powerbi-cli model expressions show --project {project_arg} --handle {} --json",
            shell_arg(&plan.handle)
        ),
    };
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    Ok(json!({
        "schema": "powerbi-cli.model.expressions.mutation.v1",
        "ok": validation_ok,
        "exitCode": if validation_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "action": action.as_str(),
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectModified": project_modified,
        "rollback": (!dry_run && !validation_ok).then(|| json!({"performed": true, "projectModified": false, "reason": "post-mutation validation failed; the original expressions file was restored"})),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "semanticModelDir": canonical_display(&target_resolved.semantic_model_dir),
        "target": {"handle": plan.handle, "name": plan.name, "path": canonical_display(&plan.path)},
        "changes": [{"kind": "tmdl.namedExpression", "action": action.as_str(), "path": canonical_display(&plan.path), "before": plan.before, "after": plan.after}],
        "validation": validation.map(validation_json),
        "readbackCommand": readback,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, inspect, validate]
    }))
}

fn build_plan(
    resolved: &crate::ResolvedProject,
    records: &[AdvancedRecord],
    action: Action,
    options: &Options,
) -> CliResult<Plan> {
    match action {
        Action::Add => {
            let name = options.name.as_deref().expect("validated name");
            if records
                .iter()
                .any(|record| record.name.eq_ignore_ascii_case(name))
            {
                return Err(CliError::invalid_args(format!(
                    "named expression already exists: {name}"
                ))
                .with_hint("Choose a new expression name or update the existing handle.")
                .with_suggested_command(format!(
                    "powerbi-cli model expressions list --project {} --json",
                    command_arg(&resolved.project_dir)
                )));
            }
            let path = resolved
                .semantic_model_dir
                .join("definition")
                .join("expressions.tmdl");
            let (mut lines, newline, _) = if path.is_file() {
                load_tmdl_lines(&path)?
            } else {
                (Vec::new(), "\n".to_string(), true)
            };
            if lines.len() == 1 && lines[0].is_empty() {
                lines.clear();
            }
            if lines.iter().any(|line| !line.trim().is_empty())
                && !lines.last().is_some_and(|line| line.trim().is_empty())
            {
                lines.push(String::new());
            }
            let block = expression_block_lines(
                name,
                options.expression.as_deref().expect("validated expression"),
            );
            lines.extend(block.clone());
            let new_text = render_tmdl_lines(&lines, &newline, true);
            Ok(Plan {
                path,
                name: name.to_string(),
                handle: expression_handle(name),
                before: None,
                after: Some(render_tmdl_lines(&block, &newline, true)),
                new_text,
            })
        }
        Action::Update | Action::Delete => {
            let record = select_record(records, options)?;
            ensure_supported_block(record, resolved)?;
            if action == Action::Delete
                && options.mode == Some(MutationMode::InPlace)
                && options.confirm.as_deref() != Some(record.handle.as_str())
            {
                return Err(CliError::invalid_args(format!(
                    "in-place delete requires --confirm {}",
                    record.handle
                ))
                .with_hint("Run delete with --dry-run first, then rerun with the exact expression handle.")
                .with_suggested_command(format!(
                    "powerbi-cli model expressions delete --project {} --handle {} --dry-run --json",
                    command_arg(&resolved.project_dir),
                    shell_arg(&record.handle)
                )));
            }
            let (mut lines, newline, had_final_newline) = load_tmdl_lines(&record.path)?;
            let replacement = if action == Action::Update {
                expression_block_lines(
                    &record.name,
                    options.expression.as_deref().expect("validated expression"),
                )
            } else {
                Vec::new()
            };
            lines.splice(record.start_line..record.end_line, replacement.clone());
            let new_text = render_tmdl_lines(&lines, &newline, had_final_newline);
            let after =
                (action == Action::Update).then(|| render_tmdl_lines(&replacement, &newline, true));
            Ok(Plan {
                path: record.path.clone(),
                name: record.name.clone(),
                handle: record.handle.clone(),
                before: Some(record.block.clone()),
                after,
                new_text,
            })
        }
    }
}

fn select_record<'a>(
    records: &'a [AdvancedRecord],
    options: &Options,
) -> CliResult<&'a AdvancedRecord> {
    if let Some(handle) = options.handle.as_deref() {
        return records
            .iter()
            .find(|record| record.handle == handle)
            .ok_or_else(|| expression_not_found(handle));
    }
    let name = options.name.as_deref().expect("validated selector");
    let matches = records
        .iter()
        .filter(|record| record.name.eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(record),
        [] => Err(expression_not_found(name)),
        _ => Err(
            CliError::validation_failed(format!("expression selector is ambiguous: {name}"))
                .with_hint("Use the exact handle returned by `model expressions list`.")
                .with_suggested_command(
                    "powerbi-cli model expressions list --project <project-dir-or.pbip> --json",
                ),
        ),
    }
}

fn expression_not_found(selector: &str) -> CliError {
    CliError::validation_failed(format!("named expression not found: {selector}"))
        .with_hint("Run `model expressions list` to get stable handles.")
        .with_suggested_command(
            "powerbi-cli model expressions list --project <project-dir-or.pbip> --json",
        )
}

fn ensure_supported_block(
    record: &AdvancedRecord,
    resolved: &crate::ResolvedProject,
) -> CliResult<()> {
    if let Some(line) = unsupported_named_expression_line(&record.block) {
        return Err(unsupported_metadata(record, resolved, &line));
    }
    Ok(())
}

fn unsupported_metadata(
    record: &AdvancedRecord,
    resolved: &crate::ResolvedProject,
    line: &str,
) -> CliError {
    CliError::unsupported_feature(format!(
        "named expression update would drop unsupported TMDL line: {line}"
    ))
    .with_hint("This expression contains Desktop-authored metadata this writer does not model; inspect it and edit in Desktop or preserve the original block.")
    .with_suggested_command(format!(
        "powerbi-cli model expressions show --project {} --handle {} --include-raw --json",
        command_arg(&resolved.project_dir),
        shell_arg(&record.handle)
    ))
}

fn expression_block_lines(name: &str, expression: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let name = crate::tmdl::tmdl_object_name(name);
    let expression = expression
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n']);
    if expression.contains('\n') || expression.contains('\r') {
        lines.push(format!("expression {name} ="));
        for line in expression.replace("\r\n", "\n").replace('\r', "\n").lines() {
            lines.push(format!("    {}", line.trim_end()));
        }
    } else {
        lines.push(format!("expression {name} = {}", expression.trim()));
    }
    lines.push(String::new());
    lines
}

fn parse_options(action: Action, args: &[String]) -> CliResult<Options> {
    let mut options = Options::default();
    let mut expression_source = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?))
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--expression" => {
                if expression_source {
                    return Err(CliError::invalid_args(
                        "--expression and --expression-file are mutually exclusive",
                    ));
                }
                options.expression = Some(take_value(args, &mut i, "--expression")?);
                expression_source = true;
            }
            "--expression-file" => {
                if expression_source {
                    return Err(CliError::invalid_args(
                        "--expression and --expression-file are mutually exclusive",
                    ));
                }
                let path = take_value(args, &mut i, "--expression-file")?;
                options.expression = Some(read_expression_file(&path)?);
                expression_source = true;
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--dry-run" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "Start with `--dry-run`; rerun with `--out-dir` or guarded `--in-place` after review.",
                    format!(
                        "powerbi-cli model expressions {} --project <project-dir-or.pbip> --dry-run --json",
                        action.as_str()
                    ),
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "Start with `--dry-run`; rerun with `--out-dir` or guarded `--in-place` after review.",
                    format!(
                        "powerbi-cli model expressions {} --project <project-dir-or.pbip> --dry-run --json",
                        action.as_str()
                    ),
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                options.out_dir = Some(PathBuf::from(take_value(args, &mut i, "--out-dir")?));
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "Start with `--dry-run`; rerun with `--out-dir` or guarded `--in-place` after review.",
                    format!(
                        "powerbi-cli model expressions {} --project <project-dir-or.pbip> --dry-run --json",
                        action.as_str()
                    ),
                )?;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model expressions {} flag: {other}",
                    action.as_str()
                ))
                .with_hint("Run capabilities for exact model expressions flags.")
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"model expressions\"",
                ));
            }
        }
    }
    match action {
        Action::Add => {
            if options.name.is_none() {
                return Err(CliError::invalid_args(
                    "model expressions add requires --name",
                ));
            }
            if options.handle.is_some() {
                return Err(CliError::invalid_args(
                    "model expressions add does not accept --handle",
                ));
            }
            if options.confirm.is_some() {
                return Err(CliError::invalid_args(
                    "--confirm is only valid for model expressions delete",
                ));
            }
            validate_expression_name(options.name.as_deref().expect("validated name"))?;
            require_expression(&options)?;
        }
        Action::Update => {
            require_selector(&options)?;
            require_expression(&options)?;
            if options.confirm.is_some() {
                return Err(CliError::invalid_args(
                    "--confirm is only valid for model expressions delete",
                ));
            }
        }
        Action::Delete => {
            require_selector(&options)?;
            if options.expression.is_some() {
                return Err(CliError::invalid_args(
                    "model expressions delete does not accept --expression",
                ));
            }
            if options.confirm.is_some() && options.mode != Some(MutationMode::InPlace) {
                return Err(CliError::invalid_args(
                    "--confirm is only valid with --in-place model expressions delete",
                ));
            }
        }
    }
    if options.handle.is_some() && options.name.is_some() {
        return Err(CliError::invalid_args(
            "model expressions accepts one selector: --handle or --name",
        ));
    }
    if let Some(handle) = options.handle.as_deref() {
        parse_expression_handle(handle)?;
    }
    if let Some(expression) = options.expression.as_deref() {
        validate_text(expression, InputKind::SourceText)?;
        if expression.trim().is_empty() {
            return Err(CliError::invalid_args("named expression must not be empty"));
        }
        if contains_credential_like_text_str(expression) {
            return Err(
                CliError::invalid_args("named expression contains credential-like text")
                    .with_hint("Remove credentials and keep the semantic model offline-safe."),
            );
        }
    }
    Ok(options)
}

fn require_selector(options: &Options) -> CliResult<()> {
    if options.handle.is_none() && options.name.is_none() {
        return Err(CliError::invalid_args(
            "model expressions update/delete requires --handle or --name",
        )
        .with_hint("Use a stable handle from `model expressions list`.")
        .with_suggested_command(
            "powerbi-cli model expressions list --project <project-dir-or.pbip> --json",
        ));
    }
    if let Some(name) = options.name.as_deref() {
        validate_expression_name(name)?;
    }
    Ok(())
}

fn require_expression(options: &Options) -> CliResult<()> {
    if options.expression.is_none() {
        return Err(CliError::invalid_args(
            "model expressions add/update requires --expression or --expression-file",
        )
        .with_hint("Provide bounded UTF-8 M text inline or with --expression-file <path|->.")
        .with_suggested_command(
            "powerbi-cli model expressions add --project <project-dir-or.pbip> --name SharedQuery --expression \"#table(type table [Value = Int64.Type], {{1}})\" --dry-run --json",
        ));
    }
    Ok(())
}

fn validate_expression_name(name: &str) -> CliResult<()> {
    if name.trim().is_empty()
        || name.trim() != name
        || name.chars().count() > 100
        || name.chars().any(char::is_control)
    {
        return Err(CliError::invalid_args(
            "expression name must be non-empty, at most 100 characters, and free of surrounding whitespace/control characters",
        ));
    }
    Ok(())
}

fn read_expression_file(path: &str) -> CliResult<String> {
    let text = if path == "-" {
        read_utf8_stream(&mut io::stdin(), InputKind::SourceText, "named expression")?
    } else {
        read_utf8(Path::new(path), InputKind::SourceText)?
    };
    let expression = text
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if expression.trim().is_empty() {
        return Err(CliError::invalid_args("named expression file is empty")
            .with_hint("Provide an M expression.")
            .with_suggested_command(
                "powerbi-cli model expressions add --project <project-dir-or.pbip> --name SharedQuery --expression-file <m.txt> --dry-run --json",
            ));
    }
    Ok(expression)
}

fn validation_json(report: crate::ValidationReport) -> Value {
    json!({
        "ok": report.errors.is_empty(),
        "warnings": report.warnings,
        "errors": report.errors,
        "counts": {
            "tables": report.tables,
            "measures": report.measures,
            "relationships": report.relationships,
            "pages": report.pages,
            "visuals": report.visuals
        }
    })
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
