use crate::contract::{
    CONTRACT_VERSION, capabilities, help_json, help_text, robot_docs_json, robot_docs_markdown,
    robot_triage, suggested_command_path,
};
use crate::desktop::desktop_command;
use crate::feature_catalog::features_command;
use crate::fixture::fixture_command;
use crate::handoff::handoff_command;
use crate::lint::lint_command;
use crate::microsoft::integrations_command;
use crate::model::model_command;
use crate::package::package_command;
use crate::profile::profile_command;
use crate::report::report_command;
use crate::schema::schema_command;
use crate::source_template::source_template_command;
use crate::workflow::workflow_command;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, diff::diff_command, doctor_json,
    inspect_command, scaffold_command, validate_command,
};
use serde_json::{Value, json};

#[derive(Debug, Default)]
struct GlobalFlags {
    json: bool,
}

#[derive(Debug)]
struct CliOutput {
    body: OutputBody,
    exit_code: i32,
}

#[derive(Debug)]
enum OutputBody {
    Json(Value),
    Text(String),
}

pub(crate) fn main_entry() {
    match run() {
        Ok(output) => {
            match output.body {
                OutputBody::Json(value) => {
                    println!(
                        "{}",
                        serde_json::to_string(&value).expect("serialize output")
                    );
                }
                OutputBody::Text(text) => {
                    print!("{text}");
                    if !text.ends_with('\n') {
                        println!();
                    }
                }
            }
            std::process::exit(output.exit_code);
        }
        Err(err) => {
            eprintln!(
                "{}",
                serde_json::to_string(&error_json(&err)).expect("serialize error")
            );
            std::process::exit(err.exit_code);
        }
    }
}

fn run() -> CliResult<CliOutput> {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let (flags, args) = parse_global_flags(&raw_args)?;
    if args.is_empty() || is_help_request(&args) {
        let body = if flags.json {
            OutputBody::Json(help_json())
        } else {
            OutputBody::Text(help_text())
        };
        return Ok(output(body, EXIT_SUCCESS));
    }

    if args[0] == "help" && args.len() == 2 {
        let filter_arg = vec!["--for".to_string(), args[1].clone()];
        return value_output(capabilities(&filter_arg)?, flags.json);
    }

    match args[0].as_str() {
        "version" => {
            require_no_args(&args[1..], "version")?;
            value_output(
                json!({
                    "tool": "powerbi-cli",
                    "binary": "powerbi-cli",
                    "version": env!("CARGO_PKG_VERSION"),
                    "contractVersion": CONTRACT_VERSION
                }),
                flags.json,
            )
        }
        "capabilities" => value_output(capabilities(&args[1..])?, flags.json),
        "features" | "feature" => value_output(features_command(&args[1..])?, flags.json),
        "robot-docs" => robot_docs_output(&args[1..], flags.json),
        "--robot-triage" | "robot-triage" => {
            require_no_args(&args[1..], "robot-triage")?;
            value_output(robot_triage(), true)
        }
        "doctor" => {
            require_no_args(&args[1..], "doctor")?;
            value_output(doctor_json(), flags.json)
        }
        "desktop" => value_output(desktop_command(&args[1..])?, flags.json),
        "diff" => value_output(diff_command(&args[1..])?, flags.json),
        "fixture" | "fixtures" => value_output(fixture_command(&args[1..])?, flags.json),
        "handoff" => value_output(handoff_command(&args[1..])?, flags.json),
        "handoff-check" => {
            let mut check_args = vec!["check".to_string()];
            check_args.extend_from_slice(&args[1..]);
            value_output(handoff_command(&check_args)?, flags.json)
        }
        "handoff-rebind-plan" => {
            let mut rebind_args = vec!["rebind-plan".to_string()];
            rebind_args.extend_from_slice(&args[1..]);
            value_output(handoff_command(&rebind_args)?, flags.json)
        }
        "scaffold" => value_output(scaffold_command(&args[1..])?, flags.json),
        "schema" => value_output(schema_command(&args[1..])?, flags.json),
        "profile" => value_output(profile_command(&args[1..])?, flags.json),
        "inspect" => value_output(inspect_command(&args[1..])?, flags.json),
        "lint" => value_output(lint_command(&args[1..])?, flags.json),
        "integrations" => value_output(integrations_command(&args[1..])?, flags.json),
        "model" => value_output(model_command(&args[1..])?, flags.json),
        "package" | "packages" => value_output(package_command(&args[1..])?, flags.json),
        "report" => value_output(report_command(&args[1..])?, flags.json),
        "source-template" | "source-templates" | "sourceTemplate" | "sourceTemplates" => {
            value_output(source_template_command(&args[1..])?, flags.json)
        }
        "validate" => value_output(validate_command(&args[1..])?, flags.json),
        "workflow" => value_output(workflow_command(&args[1..])?, flags.json),
        _ => Err(unknown_command_error(&args)),
    }
}

fn output(body: OutputBody, exit_code: i32) -> CliOutput {
    CliOutput { body, exit_code }
}

fn value_output(mut value: Value, force_json: bool) -> CliResult<CliOutput> {
    let exit_code = value
        .get("exitCode")
        .and_then(Value::as_i64)
        .map(|value| value as i32)
        .or_else(|| inferred_exit_code(&value))
        .unwrap_or(EXIT_SUCCESS);
    if let Some(object) = value.as_object_mut()
        && object.contains_key("ok")
        && !object.contains_key("exitCode")
    {
        object.insert("exitCode".to_string(), Value::from(exit_code));
    }
    let body = if force_json {
        OutputBody::Json(value)
    } else {
        OutputBody::Text(serde_json::to_string_pretty(&value).expect("pretty JSON"))
    };
    Ok(output(body, exit_code))
}

fn inferred_exit_code(value: &Value) -> Option<i32> {
    match value.get("ok").and_then(Value::as_bool) {
        Some(false) => Some(EXIT_VALIDATION_FAILED),
        _ => None,
    }
}

fn robot_docs_output(args: &[String], force_json: bool) -> CliResult<CliOutput> {
    match args {
        [guide] if guide == "guide" => {
            if force_json {
                value_output(robot_docs_json(), true)
            } else {
                Ok(output(
                    OutputBody::Text(robot_docs_markdown()),
                    EXIT_SUCCESS,
                ))
            }
        }
        [] => Err(
            CliError::invalid_args("robot-docs requires a subcommand: guide")
                .with_hint(
                    "Run `powerbi-cli robot-docs guide` or `powerbi-cli --json robot-docs guide`.",
                )
                .with_suggested_command("powerbi-cli robot-docs guide"),
        ),
        _ => Err(CliError::invalid_args("unknown robot-docs subcommand")
            .with_hint("Run `powerbi-cli robot-docs guide`.")
            .with_suggested_command("powerbi-cli robot-docs guide")),
    }
}

fn parse_global_flags(raw_args: &[String]) -> CliResult<(GlobalFlags, Vec<String>)> {
    let mut flags = GlobalFlags::default();
    let mut args = Vec::new();
    let mut i = 0;
    while i < raw_args.len() {
        match raw_args[i].as_str() {
            "--json" => {
                flags.json = true;
                i += 1;
            }
            "--format" | "-f" => {
                let value = raw_args.get(i + 1).ok_or_else(|| {
                    CliError::invalid_args("--format requires a value")
                        .with_hint("Use `--format json` or the shorter `--json`.")
                        .with_suggested_command("powerbi-cli --json capabilities")
                })?;
                if value != "json" {
                    return Err(CliError::invalid_args(format!(
                        "invalid format: {value}; only json is supported"
                    ))
                    .with_hint("Use `--format json` or the shorter `--json`.")
                    .with_suggested_command("powerbi-cli --format json capabilities"));
                }
                flags.json = true;
                i += 2;
            }
            "--format=json" | "-f=json" => {
                flags.json = true;
                i += 1;
            }
            flag if looks_like_json_flag(flag) => {
                return Err(CliError::invalid_args(format!("unknown global flag: {flag}"))
                    .with_hint(
                        "Did you mean `--json`? The flag is accepted before or after the command.",
                    )
                    .with_suggested_command(correct_json_flag_command(raw_args)));
            }
            other => {
                args.push(other.to_string());
                i += 1;
            }
        }
    }
    Ok((flags, args))
}

fn is_help_request(args: &[String]) -> bool {
    matches!(args, [one] if one == "--help" || one == "-h" || one == "help")
}

fn looks_like_json_flag(flag: &str) -> bool {
    flag.starts_with("--j") && edit_distance(flag, "--json") <= 2
}

fn correct_json_flag_command(raw_args: &[String]) -> String {
    let mut corrected = Vec::new();
    let mut replaced = false;
    for arg in raw_args {
        if !replaced && looks_like_json_flag(arg) {
            corrected.push("--json".to_string());
            replaced = true;
        } else {
            corrected.push(arg.clone());
        }
    }
    format!("powerbi-cli {}", corrected.join(" "))
}

fn unknown_command_error(args: &[String]) -> CliError {
    let command = args.first().map(String::as_str).unwrap_or_default();
    if let Some(candidate) = suggested_command_path(args) {
        return CliError::invalid_args(format!("unknown command: {}", args.join(" ")))
            .with_hint(format!(
                "Did you mean `powerbi-cli {candidate}`? Inspect that exact command contract before running it."
            ))
            .with_suggested_command(format!(
                "powerbi-cli --json capabilities --for \"{candidate}\""
            ));
    }
    let known = [
        "capabilities",
        "features",
        "robot-docs",
        "robot-triage",
        "doctor",
        "diff",
        "desktop",
        "fixture",
        "scaffold",
        "schema",
        "profile",
        "inspect",
        "handoff",
        "lint",
        "model",
        "report",
        "source-template",
        "validate",
        "version",
    ];
    if let Some(candidate) = known
        .iter()
        .filter(|candidate| edit_distance(command, candidate) <= 2)
        .min_by_key(|candidate| edit_distance(command, candidate))
    {
        CliError::invalid_args(format!("unknown command: {command}"))
            .with_hint(format!(
                "Did you mean `{candidate}`? Run `powerbi-cli --json capabilities --for {candidate}` for the exact contract."
            ))
            .with_suggested_command(format!(
                "powerbi-cli --json capabilities --for {candidate}"
            ))
    } else {
        CliError::invalid_args(format!("unknown command: {command}"))
            .with_hint(
                "Run `powerbi-cli --json capabilities` to inspect the supported command contract.",
            )
            .with_suggested_command("powerbi-cli --json capabilities")
    }
}

fn require_no_args(args: &[String], command: &str) -> CliResult<()> {
    if args.is_empty() {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{command} does not accept arguments: {}",
        args.join(" ")
    ))
    .with_hint(format!("Run `{command}` without trailing arguments."))
    .with_suggested_command(format!("powerbi-cli {command} --json")))
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b_chars = b.chars().collect::<Vec<_>>();
    let mut prev = (0..=b_chars.len()).collect::<Vec<_>>();
    for (i, ca) in a.chars().enumerate() {
        let mut curr = vec![i + 1];
        for (j, cb) in b_chars.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            curr.push((prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost));
        }
        prev = curr;
    }
    prev[b_chars.len()]
}

fn error_json(err: &CliError) -> Value {
    let mut error = serde_json::Map::new();
    error.insert("code".to_string(), Value::String(err.code.to_string()));
    error.insert("exitCode".to_string(), Value::from(err.exit_code));
    error.insert("message".to_string(), Value::String(err.message.clone()));
    if let Some(hint) = &err.hint {
        error.insert("hint".to_string(), Value::String(hint.clone()));
    }
    if !err.suggested_commands.is_empty() {
        error.insert(
            "suggestedCommands".to_string(),
            Value::Array(
                err.suggested_commands
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    json!({ "error": Value::Object(error) })
}
