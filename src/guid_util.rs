use crate::{CliError, CliResult};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

const MIN_COUNT: u64 = 1;
const MAX_COUNT: u64 = 100;

pub(crate) fn guid_command(args: &[String]) -> CliResult<Value> {
    let count = parse_guid_args(args)?;
    let mut guids = Vec::with_capacity(count as usize);
    let mut seen = BTreeSet::new();
    while (guids.len() as u64) < count {
        let guid = Uuid::new_v4().to_string();
        if seen.insert(guid.clone()) {
            guids.push(guid);
        }
    }
    Ok(json!({
        "guids": guids,
        "count": count
    }))
}

fn parse_guid_args(args: &[String]) -> CliResult<u64> {
    let mut count = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--count" => {
                if count.is_some() {
                    return Err(CliError::invalid_args("--count may be specified only once")
                        .with_hint("Run `powerbi-cli guid [--count <1..100>] --json`.")
                        .with_suggested_command("powerbi-cli guid --json"));
                }
                let value = args.get(index + 1).ok_or_else(|| {
                    CliError::invalid_args("--count requires a value")
                        .with_hint("Use --count <1..100>.")
                        .with_suggested_command("powerbi-cli guid --count 1 --json")
                })?;
                let parsed = value.parse::<u64>().map_err(|_| {
                    CliError::invalid_args("--count must be an integer from 1 to 100")
                        .with_hint("Use --count <1..100>.")
                        .with_suggested_command("powerbi-cli guid --count 1 --json")
                })?;
                if !(MIN_COUNT..=MAX_COUNT).contains(&parsed) {
                    return Err(CliError::invalid_args(format!(
                        "--count must be from {MIN_COUNT} to {MAX_COUNT}; got {parsed}"
                    ))
                    .with_hint("Use --count <1..100>.")
                    .with_suggested_command("powerbi-cli guid --count 1 --json"));
                }
                count = Some(parsed);
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown guid flag: {other}"))
                        .with_hint("Run `powerbi-cli guid [--count <1..100>] --json`.")
                        .with_suggested_command("powerbi-cli guid --json"),
                );
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "guid does not accept positional arguments: {other}"
                ))
                .with_hint("Run `powerbi-cli guid [--count <1..100>] --json`.")
                .with_suggested_command("powerbi-cli guid --json"));
            }
        }
    }
    Ok(count.unwrap_or(1))
}
