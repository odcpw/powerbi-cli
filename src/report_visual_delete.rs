use crate::cli_support::{
    MutationMode, mode_name, require_mode_with_contract, required_project_with_suggestion,
    set_mode_with_contract, shell_arg, take_report_value as take_value, target_project,
};
use crate::pbir::{VisualSelector, find_visual, load_report_snapshot, visual_detail};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const DELETE_DRY_RUN_COMMAND: &str = "powerbi-cli report visuals delete --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json";
const REQUIRE_MODE_HINT: &str =
    "Start with `--dry-run`; use `--out-dir` or confirmed `--in-place` only after review.";
const SET_MODE_HINT: &str =
    "Start with `--dry-run`; rerun with `--out-dir` or confirmed `--in-place` after review.";

#[derive(Debug, Default)]
struct DeleteVisualOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    confirm: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn delete_visual(args: &[String]) -> CliResult<Value> {
    let options = parse_delete_args(args)?;
    let source_project = required_project_with_suggestion(
        options.project.clone(),
        "report visuals delete",
        DELETE_DRY_RUN_COMMAND,
    )?;
    require_visual_selector(&options.selector, "report visuals delete")?;
    let mode = require_mode_with_contract(
        options.mode,
        "report visuals delete",
        REQUIRE_MODE_HINT,
        DELETE_DRY_RUN_COMMAND,
    )?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, delete_visual)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let visual = find_visual(&snapshot.pages, &options.selector, "report visuals delete")?.clone();

    if mode == MutationMode::InPlace && options.confirm.as_deref() != Some(&visual.handle) {
        return Err(CliError::invalid_args(format!(
            "in-place visual deletion requires --confirm {}",
            visual.handle
        ))
        .with_hint("Run the same command with --dry-run first, then confirm the exact visual handle.")
        .with_suggested_command(format!(
            "powerbi-cli report visuals delete --project {} --handle {} --in-place --confirm {} --json",
            command_arg(&target_resolved.project_dir),
            shell_arg(&visual.handle),
            shell_arg(&visual.handle)
        )));
    }

    let visual_path = visual.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual has no path in inspect output: {}",
            visual.handle
        ))
        .with_hint("Run `validate --strict` before mutating this report.")
        .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })?;
    let visual_dir = validated_visual_delete_dir(&target_resolved, visual_path, &visual.page_name)?;
    ensure_visual_dir_contains_only_visual_json(&visual_dir, visual_path)?;

    let target = visual_detail(&visual);
    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        remove_visual_container(visual_path, &visual_dir)?;
    }

    mutation_response(
        &target_resolved,
        mode,
        target,
        canonical_display(visual_path),
        &visual.page_handle,
    )
}

fn remove_visual_container(visual_path: &Path, visual_dir: &Path) -> CliResult<()> {
    let original_visual = fs::read(visual_path).map_err(|err| {
        CliError::unexpected(format!(
            "read visual file before deletion {}: {err}",
            visual_path.display()
        ))
    })?;
    let original_permissions = prepare_visual_dir_for_removal(visual_dir)?;

    if let Err(err) = fs::remove_file(visual_path) {
        let restore_note = restore_visual_dir_permissions(visual_dir, original_permissions)
            .err()
            .map(|restore_err| {
                format!("; restoring directory permissions also failed: {restore_err}")
            })
            .unwrap_or_default();
        return Err(CliError::unexpected(format!(
            "remove visual file {}: {err}{restore_note}",
            visual_path.display()
        )));
    }

    if let Err(err) = fs::remove_dir(visual_dir) {
        let file_restore = fs::write(visual_path, &original_visual);
        let permission_restore = restore_visual_dir_permissions(visual_dir, original_permissions);
        let rollback_note = match (file_restore, permission_restore) {
            (Ok(()), Ok(())) => "the visual file was restored".to_string(),
            (file_result, permission_result) => {
                let file_note = file_result
                    .err()
                    .map(|restore_err| format!("visual restore failed: {restore_err}"))
                    .unwrap_or_else(|| "visual restored".to_string());
                let permission_note = permission_result
                    .err()
                    .map(|restore_err| format!("permission restore failed: {restore_err}"))
                    .unwrap_or_else(|| "permissions restored".to_string());
                format!("rollback incomplete ({file_note}; {permission_note})")
            }
        };
        return Err(CliError::unexpected(format!(
            "remove visual dir {}: {err}; {rollback_note}",
            visual_dir.display()
        )));
    }

    Ok(())
}

// Windows exposes only the readonly flag here; the Unix world-writable concern behind
// this Clippy lint does not apply to the cfg-gated branch below.
#[cfg_attr(windows, allow(clippy::permissions_set_readonly_false))]
fn prepare_visual_dir_for_removal(visual_dir: &Path) -> CliResult<fs::Permissions> {
    let original_permissions = fs::metadata(visual_dir)
        .map_err(|err| {
            CliError::unexpected(format!(
                "read visual dir permissions {}: {err}",
                visual_dir.display()
            ))
        })?
        .permissions();

    #[cfg(windows)]
    if original_permissions.readonly() {
        let mut writable_permissions = original_permissions.clone();
        writable_permissions.set_readonly(false);
        fs::set_permissions(visual_dir, writable_permissions).map_err(|err| {
            CliError::unexpected(format!(
                "make visual dir removable {}: {err}",
                visual_dir.display()
            ))
        })?;
    }

    Ok(original_permissions)
}

fn restore_visual_dir_permissions(
    visual_dir: &Path,
    original_permissions: fs::Permissions,
) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        fs::set_permissions(visual_dir, original_permissions)
    }

    #[cfg(not(windows))]
    {
        let _ = (visual_dir, original_permissions);
        Ok(())
    }
}

fn parse_delete_args(args: &[String]) -> CliResult<DeleteVisualOptions> {
    let mut options = DeleteVisualOptions::default();
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
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--dry-run" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::DryRun,
                    SET_MODE_HINT,
                    DELETE_DRY_RUN_COMMAND,
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::InPlace,
                    SET_MODE_HINT,
                    DELETE_DRY_RUN_COMMAND,
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::OutDir,
                    SET_MODE_HINT,
                    DELETE_DRY_RUN_COMMAND,
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals delete flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals delete\"` for exact flags.")
                .with_suggested_command("powerbi-cli --json capabilities --for \"report visuals delete\""));
            }
        }
    }
    Ok(options)
}

fn mutation_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    target: Value,
    visual_path: String,
    page_handle: &str,
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
        "powerbi-cli report visuals list --project {} --page {} --json",
        project_arg,
        shell_arg(page_handle)
    );
    let wireframe = format!(
        "powerbi-cli report wireframe export {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let inspect = format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&target_resolved.project_dir)
    );
    let validate = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&target_resolved.project_dir)
    );

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.deleteMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "delete",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": target,
        "deletePlan": {
            "before": target,
            "after": Value::Null
        },
        "changes": [{
            "kind": "pbir.visual",
            "action": "delete",
            "path": visual_path,
            "before": target,
            "after": Value::Null
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
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, wireframe, inspect, validate]
    }))
}

fn validated_visual_delete_dir(
    resolved: &ResolvedProject,
    visual_path: &Path,
    page_name: &str,
) -> CliResult<PathBuf> {
    if visual_path.file_name().and_then(|value| value.to_str()) != Some("visual.json") {
        return Err(CliError::validation_failed(format!(
            "refusing to delete visual because path is not visual.json: {}",
            visual_path.display()
        )));
    }
    let visual_dir = visual_path.parent().ok_or_else(|| {
        CliError::validation_failed(format!(
            "visual path has no parent: {}",
            visual_path.display()
        ))
    })?;
    let page_visuals_dir = resolved
        .report_dir
        .join("definition")
        .join("pages")
        .join(page_name)
        .join("visuals");
    let visual_dir_abs = fs::canonicalize(visual_dir)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", visual_dir.display())))?;
    let page_visuals_abs = fs::canonicalize(&page_visuals_dir).map_err(|err| {
        CliError::unexpected(format!("resolve {}: {err}", page_visuals_dir.display()))
    })?;
    let visual_path_abs = fs::canonicalize(visual_path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", visual_path.display())))?;
    if visual_dir_abs.parent() != Some(page_visuals_abs.as_path())
        || visual_path_abs.parent() != Some(visual_dir_abs.as_path())
    {
        return Err(CliError::validation_failed(format!(
            "refusing to delete visual outside page visuals directory: {}",
            visual_path.display()
        ))
        .with_hint("Run `validate --strict` before deleting visuals.")
        .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json"));
    }
    Ok(visual_dir.to_path_buf())
}

fn ensure_visual_dir_contains_only_visual_json(
    visual_dir: &Path,
    visual_path: &Path,
) -> CliResult<()> {
    let visual_path_abs = fs::canonicalize(visual_path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", visual_path.display())))?;
    let entries = fs::read_dir(visual_dir)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", visual_dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            CliError::unexpected(format!("read {} entry: {err}", visual_dir.display()))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| {
            CliError::unexpected(format!("read file type {}: {err}", path.display()))
        })?;
        let path_abs = fs::canonicalize(&path)
            .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?;
        if !file_type.is_file() || path_abs != visual_path_abs {
            return Err(CliError::invalid_args(format!(
                "report visuals delete refuses visual directories with unknown files: {}",
                visual_dir.display()
            ))
            .with_hint("This first-slice delete only removes a container containing exactly visual.json.")
            .with_suggested_command(
                "powerbi-cli report visuals delete --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
            ));
        }
    }
    Ok(())
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
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json"
    )))
}
