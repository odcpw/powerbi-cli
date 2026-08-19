use crate::contract::command_catalog;
use crate::{CliError, CliResult};
use serde_json::{Value, json};

pub(crate) enum HelpDocument {
    Text(String),
    Json(Value),
}

pub(crate) fn help_path(args: &[String]) -> Option<Vec<String>> {
    let help_prefix = args.first().is_some_and(|arg| arg == "help");
    let help_flag = args.iter().any(|arg| arg == "--help" || arg == "-h");
    if !help_prefix && !help_flag {
        return None;
    }
    Some(
        args.iter()
            .filter(|arg| *arg != "help" && *arg != "--help" && *arg != "-h")
            .filter(|arg| !arg.starts_with('-'))
            .cloned()
            .collect(),
    )
}

pub(crate) fn render_help(path: &[String], json: bool) -> CliResult<HelpDocument> {
    let prefix = path.join(" ");
    let catalog = command_catalog();
    if let Some(entry) = catalog.iter().find(|command| command["path"] == prefix) {
        return Ok(if json {
            HelpDocument::Json(json!({ "help": entry }))
        } else {
            HelpDocument::Text(render_leaf_text(entry))
        });
    }

    let family = family_commands(&catalog, &prefix);
    if !family.is_empty() {
        return Ok(if json {
            HelpDocument::Json(json!({
                "help": {
                    "path": prefix,
                    "commands": family,
                    "capabilitiesCommand": capabilities_command(&prefix)
                }
            }))
        } else {
            HelpDocument::Text(render_family_text(&prefix, &family))
        });
    }

    let hint = if prefix.contains(' ') {
        format!(
            "Run `{cmd}` for the exact contract.",
            cmd = capabilities_command(&prefix)
        )
    } else {
        format!("Run `powerbi-cli --json capabilities --for {prefix}` for the exact contract.")
    };
    Err(CliError::invalid_args(format!("unknown command: {prefix}"))
        .with_hint(hint)
        .with_suggested_command(capabilities_command(&prefix)))
}

pub(crate) fn enrich_did_you_mean(mut err: CliError, args: &[String]) -> CliError {
    if err.code != "invalid_args" {
        return err;
    }
    if err
        .hint
        .as_deref()
        .is_some_and(|hint| hint.starts_with("Did you mean"))
    {
        return err;
    }

    let suggestion = unknown_flag_token(&err.message)
        .and_then(|flag| suggest_flag(args, flag))
        .or_else(|| {
            looks_like_unknown_command(&err.message)
                .then(|| suggest_subcommand(args))
                .flatten()
        });

    if let Some(suggestion) = suggestion {
        let phrase = format!("Did you mean `{suggestion}`?");
        err.hint = Some(match err.hint.take() {
            Some(existing) => format!("{phrase} {existing}"),
            None => phrase,
        });
    }
    err
}

pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
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

fn render_leaf_text(entry: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(usage) = entry["usage"].as_str() {
        lines.push(usage.to_string());
        lines.push(String::new());
    }
    if let Some(summary) = entry["summary"].as_str() {
        lines.push(summary.to_string());
        lines.push(String::new());
    }
    let flags = string_list(&entry["flags"]);
    if !flags.is_empty() {
        lines.push("Flags:".to_string());
        for flag in flags {
            lines.push(format!("  {flag}"));
        }
        lines.push(String::new());
    }
    if let Some(example) = entry["examples"]
        .as_array()
        .and_then(|examples| examples.first())
        .and_then(Value::as_str)
    {
        lines.push("Example:".to_string());
        lines.push(format!("  {example}"));
        lines.push(String::new());
    }
    let limitations = string_list(&entry["limitations"]);
    if !limitations.is_empty() {
        lines.push("Limitations:".to_string());
        for limitation in limitations {
            lines.push(format!("  {limitation}"));
        }
        lines.push(String::new());
    }
    let aliases = string_list(&entry["aliases"]);
    if !aliases.is_empty() {
        lines.push("Aliases:".to_string());
        for alias in aliases {
            lines.push(format!("  {alias}"));
        }
        lines.push(String::new());
    }
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.push(String::new());
    lines.join("\n")
}

fn render_family_text(prefix: &str, commands: &[&Value]) -> String {
    let mut lines = commands
        .iter()
        .filter_map(|command| {
            let path = command["path"].as_str()?;
            let summary = command["summary"].as_str().unwrap_or_default();
            Some(format!("  {path} — {summary}"))
        })
        .collect::<Vec<_>>();
    lines.push(String::new());
    lines.push(capabilities_command(prefix));
    lines.push(String::new());
    lines.join("\n")
}

fn family_commands<'a>(catalog: &'a [Value], prefix: &str) -> Vec<&'a Value> {
    catalog
        .iter()
        .filter(|command| {
            command["path"]
                .as_str()
                .is_some_and(|path| path_has_prefix(path, prefix))
        })
        .collect()
}

fn path_has_prefix(path: &str, prefix: &str) -> bool {
    path == prefix || (path.starts_with(prefix) && path[prefix.len()..].starts_with(' '))
}

fn capabilities_command(filter: &str) -> String {
    if filter.contains(' ') {
        format!("powerbi-cli --json capabilities --for \"{filter}\"")
    } else {
        format!("powerbi-cli --json capabilities --for {filter}")
    }
}

fn string_list(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn unknown_flag_token(message: &str) -> Option<&str> {
    let token = message.split(" flag: ").nth(1)?.trim();
    token.starts_with('-').then_some(token)
}

fn looks_like_unknown_command(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("unknown") && lower.contains("command")
}

fn suggest_flag(args: &[String], flag: &str) -> Option<String> {
    let tokens = positional_tokens(args);
    let entry = catalog_entry_for_tokens(&tokens)?;
    let candidates = catalog_flag_names(&entry);
    nearest_name(flag, candidates.iter().map(String::as_str))
}

fn suggest_subcommand(args: &[String]) -> Option<String> {
    let tokens = positional_tokens(args);
    let (last, prefix) = tokens.split_last()?;
    let candidates = if prefix.is_empty() {
        catalog_root_tokens()
    } else {
        catalog_next_tokens(&prefix.join(" "))
    };
    nearest_name(last, candidates.iter().map(String::as_str))
}

fn positional_tokens(args: &[String]) -> Vec<&str> {
    args.iter()
        .map(String::as_str)
        .filter(|arg| !arg.starts_with('-'))
        .collect()
}

fn catalog_entry_for_tokens(tokens: &[&str]) -> Option<Value> {
    let catalog = command_catalog();
    let exact = tokens.join(" ");
    if let Some(entry) = catalog.iter().find(|command| command["path"] == exact) {
        return Some(entry.clone());
    }
    catalog
        .into_iter()
        .filter(|command| {
            command["path"].as_str().is_some_and(|path| {
                let path_tokens = path.split_whitespace().collect::<Vec<_>>();
                tokens.starts_with(&path_tokens)
            })
        })
        .max_by_key(|command| {
            command["path"]
                .as_str()
                .map(|path| path.split_whitespace().count())
                .unwrap_or(0)
        })
}

fn catalog_flag_names(entry: &Value) -> Vec<String> {
    string_list(&entry["flags"])
        .into_iter()
        .filter_map(|flag| {
            let name = flag.split_whitespace().next()?;
            name.starts_with('-').then(|| name.to_string())
        })
        .collect()
}

fn catalog_root_tokens() -> Vec<String> {
    unique_tokens(
        command_catalog()
            .iter()
            .flat_map(command_names)
            .filter_map(|name| name.split_whitespace().next().map(ToOwned::to_owned)),
    )
}

fn catalog_next_tokens(prefix: &str) -> Vec<String> {
    unique_tokens(
        command_catalog()
            .iter()
            .flat_map(command_names)
            .filter_map(|name| {
                path_has_prefix(&name, prefix)
                    .then(|| name[prefix.len()..].trim_start())
                    .and_then(|rest| rest.split_whitespace().next())
                    .filter(|token| !token.is_empty())
                    .map(ToOwned::to_owned)
            }),
    )
}

fn command_names(command: &Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(path) = command["path"].as_str() {
        names.push(path.to_string());
    }
    names.extend(
        string_list(&command["aliases"])
            .into_iter()
            .map(str::to_string),
    );
    names
}

fn unique_tokens(tokens: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for token in tokens {
        if !seen.iter().any(|existing| existing == &token) {
            seen.push(token);
        }
    }
    seen
}

fn nearest_name<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        if !is_near_miss(input, candidate) {
            continue;
        }
        let distance = edit_distance(input, candidate);
        let replace = match best {
            None => true,
            Some((best_distance, best_name)) => {
                distance < best_distance || (distance == best_distance && candidate < best_name)
            }
        };
        if replace {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, name)| name.to_string())
}

fn is_near_miss(input: &str, candidate: &str) -> bool {
    if input == candidate || candidate.is_empty() {
        return false;
    }
    // Nearest name must sit within distance 2 and be ≥1 char shorter than input+2.
    candidate.len() < input.len() + 2 && {
        let distance = edit_distance(input, candidate);
        (1..=2).contains(&distance)
    }
}
