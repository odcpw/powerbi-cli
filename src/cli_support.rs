use crate::project_io::copy_project_dir;
use crate::{CliError, CliResult, ResolvedProject, resolve_project};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MutationMode {
    DryRun,
    InPlace,
    OutDir,
}

pub(crate) fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    take_value_with_usage(args, index, flag, "powerbi-cli --json capabilities")
}

pub(crate) fn take_value_with_usage(
    args: &[String],
    index: &mut usize,
    flag: &str,
    usage_command: &str,
) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint(format!("Run `{usage_command}` for exact usage."))
            .with_suggested_command(usage_command)
    })?;
    *index += 2;
    Ok(value.clone())
}

pub(crate) fn take_report_value(
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> CliResult<String> {
    take_value_with_usage(
        args,
        index,
        flag,
        "powerbi-cli --json capabilities --for report",
    )
}

pub(crate) fn take_report_interaction_value(
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> CliResult<String> {
    take_value_with_usage(
        args,
        index,
        flag,
        "powerbi-cli --json capabilities --for \"report interactions\"",
    )
}

pub(crate) fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    required_project_with_suggestion(
        project,
        command,
        format!("powerbi-cli {command} --project <project-dir-or.pbip> --json"),
    )
}

pub(crate) fn required_project_with_suggestion(
    project: Option<PathBuf>,
    command: &str,
    suggested_command: impl Into<String>,
) -> CliResult<PathBuf> {
    let suggested_command = suggested_command.into();
    project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass the PBIP project directory or the .pbip file explicitly with `--project`.")
        .with_suggested_command(suggested_command)
    })
}

pub(crate) fn require_mode(mode: Option<MutationMode>, command: &str) -> CliResult<MutationMode> {
    require_mode_with_contract(
        mode,
        command,
        "Start with `--dry-run`; use `--out-dir` or confirmed `--in-place` only after review.",
        format!("powerbi-cli {command} --project <project-dir-or.pbip> --dry-run --json"),
    )
}

pub(crate) fn require_mode_with_contract(
    mode: Option<MutationMode>,
    command: &str,
    hint: impl Into<String>,
    suggested_command: impl Into<String>,
) -> CliResult<MutationMode> {
    require_mode_with_allowed_modes(
        mode,
        command,
        "--dry-run, --in-place, or --out-dir <dir>",
        hint,
        suggested_command,
    )
}

pub(crate) fn require_mode_with_allowed_modes(
    mode: Option<MutationMode>,
    command: &str,
    allowed_modes: &str,
    hint: impl Into<String>,
    suggested_command: impl Into<String>,
) -> CliResult<MutationMode> {
    let hint = hint.into();
    let suggested_command = suggested_command.into();
    mode.ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires {allowed_modes}"))
            .with_hint(hint)
            .with_suggested_command(suggested_command)
    })
}

pub(crate) fn set_mode(
    current: &mut Option<MutationMode>,
    next: MutationMode,
    command: &str,
) -> CliResult<()> {
    set_mode_with_contract(
        current,
        next,
        "Start with `--dry-run`; rerun with `--out-dir` or confirmed `--in-place` after review.",
        format!("powerbi-cli {command} --project <project-dir-or.pbip> --dry-run --json"),
    )
}

pub(crate) fn set_mode_with_contract(
    current: &mut Option<MutationMode>,
    next: MutationMode,
    hint: impl Into<String>,
    suggested_command: impl Into<String>,
) -> CliResult<()> {
    set_mode_with_allowed_modes(
        current,
        next,
        "--dry-run, --in-place, or --out-dir <dir>",
        hint,
        suggested_command,
    )
}

pub(crate) fn set_mode_with_allowed_modes(
    current: &mut Option<MutationMode>,
    next: MutationMode,
    allowed_modes: &str,
    hint: impl Into<String>,
    suggested_command: impl Into<String>,
) -> CliResult<()> {
    if current.is_some() {
        let hint = hint.into();
        let suggested_command = suggested_command.into();
        return Err(CliError::invalid_args(format!(
            "choose exactly one output mode: {allowed_modes}"
        ))
        .with_hint(hint)
        .with_suggested_command(suggested_command));
    }
    *current = Some(next);
    Ok(())
}

pub(crate) fn require_report_page_mode(
    mode: Option<MutationMode>,
    command: &str,
) -> CliResult<MutationMode> {
    require_mode_with_contract(
        mode,
        command,
        "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.",
        format!("powerbi-cli {command} --project <project-dir-or.pbip> --dry-run --json"),
    )
}

pub(crate) fn set_report_page_mode(
    current: &mut Option<MutationMode>,
    next: MutationMode,
) -> CliResult<()> {
    set_mode_with_contract(
        current,
        next,
        "Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.",
        "powerbi-cli report pages add --project <project-dir-or.pbip> --display-name <name> --dry-run --json",
    )
}

pub(crate) fn require_report_interaction_mode(
    mode: Option<MutationMode>,
    command: &str,
) -> CliResult<MutationMode> {
    require_mode_with_contract(
        mode,
        command,
        "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.",
        format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json"
        ),
    )
}

pub(crate) fn set_report_interaction_mode(
    current: &mut Option<MutationMode>,
    next: MutationMode,
    command: &str,
) -> CliResult<()> {
    set_mode_with_contract(
        current,
        next,
        "Start with `--dry-run`; rerun with `--out-dir` or `--in-place` after review.",
        format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json"
        ),
    )
}

pub(crate) fn set_report_visual_mode(
    current: &mut Option<MutationMode>,
    next: MutationMode,
) -> CliResult<()> {
    set_mode_with_contract(
        current,
        next,
        "Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.",
        "powerbi-cli report visuals set-position --project <project-dir-or.pbip> --handle <visual-handle> --x 40 --y 40 --dry-run --json",
    )
}

pub(crate) fn mode_name(mode: MutationMode) -> &'static str {
    match mode {
        MutationMode::DryRun => "dry-run",
        MutationMode::InPlace => "in-place",
        MutationMode::OutDir => "out-dir",
    }
}

pub(crate) fn preflight_out_dir<F>(args: &[String], command: F) -> CliResult<()>
where
    F: FnOnce(&[String]) -> CliResult<Value>,
{
    let Some(dry_run_args) = dry_run_args_for_out_dir(args)? else {
        return Ok(());
    };
    let response = command(&dry_run_args)?;
    if response.get("ok").and_then(Value::as_bool) == Some(false) {
        let details = response
            .pointer("/validation/errors")
            .and_then(Value::as_array)
            .map(|errors| {
                errors
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|details| !details.is_empty())
            .unwrap_or_else(|| "dry-run plan reported ok=false".to_string());
        return Err(CliError::validation_failed(format!(
            "--out-dir preflight failed before project copy: {details}"
        )));
    }
    Ok(())
}

fn dry_run_args_for_out_dir(args: &[String]) -> CliResult<Option<Vec<String>>> {
    let mut dry_run_args = Vec::with_capacity(args.len());
    let mut replaced = false;
    let mut index = 0;
    while index < args.len() {
        if matches!(args[index].as_str(), "--out-dir" | "--out") {
            if args.get(index + 1).is_none() {
                return Err(CliError::invalid_args(format!(
                    "{} requires a directory",
                    args[index]
                )));
            }
            if replaced {
                return Err(CliError::invalid_args(
                    "--out-dir may be specified only once",
                ));
            }
            dry_run_args.push("--dry-run".to_string());
            replaced = true;
            index += 2;
        } else {
            dry_run_args.push(args[index].clone());
            index += 1;
        }
    }
    Ok(replaced.then_some(dry_run_args))
}

pub(crate) fn target_project(
    source_resolved: &ResolvedProject,
    mode: MutationMode,
    out_dir: Option<&Path>,
) -> CliResult<ResolvedProject> {
    match (mode, out_dir) {
        (MutationMode::DryRun | MutationMode::InPlace, _) => Ok(source_resolved.clone()),
        (MutationMode::OutDir, Some(out_dir)) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)
        }
        (MutationMode::OutDir, None) => {
            Err(CliError::invalid_args("--out-dir requires a directory"))
        }
    }
}

pub(crate) fn shell_arg(value: &str) -> String {
    let is_safe = !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-' | '/' | '\\' | '=')
        });

    if is_safe {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

#[cfg(test)]
mod tests {
    use super::shell_arg;

    #[test]
    fn shell_arg_keeps_allowlisted_ascii_handle_unquoted() {
        let handle = r"visual:page-1/section\item=value";

        assert_eq!(shell_arg(handle), handle);
    }

    #[test]
    fn shell_arg_quotes_values_outside_the_allowlist() {
        let cases = [
            ("x|Start-Sleep", "'x|Start-Sleep'"),
            ("$profile", "'$profile'"),
            ("`Start-Sleep", "'`Start-Sleep'"),
            ("<input>", "'<input>'"),
            ("{value}", "'{value}'"),
            ("value#comment", "'value#comment'"),
            ("Überblick", "'Überblick'"),
            ("two words", "'two words'"),
            ("O'Brien", "'O''Brien'"),
            ("", "''"),
        ];

        for (value, expected) in cases {
            assert_eq!(
                shell_arg(value),
                expected,
                "unexpected quoting for {value:?}"
            );
        }
    }
}
