use crate::safety_scan::redact_credential_values;
use crate::tmdl::{
    PartitionRecord, PartitionSelector, find_partition, load_table_documents,
    partition_source_kind_is_external, same_name,
};
use crate::{CliError, CliResult, canonical_display, command_arg, resolve_project};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(crate) fn partitions_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "model partitions requires a subcommand: list, show",
        )
        .with_hint("Run `powerbi-cli model partitions list --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli model partitions list --project <project-dir-or.pbip> --json",
        ));
    };

    match action.as_str() {
        "list" => list_partitions(rest),
        "show" => show_partition(rest),
        "set-sql-template" | "set-m" | "set-dummy" => Err(CliError::invalid_args(format!(
            "model partitions {action} is deferred; source templates are sidecar metadata in this slice"
        ))
        .with_hint(
            "Use `source-template add` to prepare credential-free work-machine rebind metadata, then run `handoff rebind-plan`.",
        )
        .with_suggested_command(
            "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json",
        )
        .with_suggested_command("powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json")),
        _ => Err(CliError::invalid_args(format!(
            "unknown model partitions command: {action}"
        ))
        .with_hint(
            "Run `powerbi-cli --json capabilities --for partitions` for supported partition commands.",
        )
        .with_suggested_command("powerbi-cli --json capabilities --for partitions")),
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
    selector: PartitionSelector,
    include_source: bool,
}

fn list_partitions(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "model partitions list")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let mut partitions = Vec::new();
    for doc in &docs {
        if options
            .table
            .as_ref()
            .is_none_or(|table| same_name(table, &doc.table))
        {
            partitions.extend(doc.partitions.iter().map(partition_summary_json));
        }
    }
    partitions.sort_by(|left, right| {
        left["handle"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["handle"].as_str().unwrap_or_default())
    });
    let safe_partitions = partitions
        .iter()
        .filter(|partition| partition["offlineSafety"]["status"] == "safe")
        .count();

    Ok(json!({
        "schema": "powerbi-cli.model.partitions.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "filter": {
            "table": options.table
        },
        "counts": {
            "tables": docs.len(),
            "partitions": partitions.len(),
            "safePartitions": safe_partitions,
            "unsafePartitions": partitions.len().saturating_sub(safe_partitions)
        },
        "partitions": partitions,
        "next": [
            format!("powerbi-cli model partitions show --project {} --handle <partition-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli handoff check {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn show_partition(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project, "model partitions show")?;
    require_selector(&options.selector, "model partitions show")?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let record = find_partition(&docs, &options.selector)?;
    if options.include_source && record.safety.status != "safe" {
        return Err(CliError::invalid_args(format!(
            "--include-source is refused for a partition with {} offline safety: {}",
            record.safety.status,
            record.handle()
        ))
        .with_hint("Review the redacted sourcePreview and safety findings; remove credentials or PII-suspect literals before requesting a raw source dump.")
        .with_suggested_command(format!(
            "powerbi-cli model partitions show --project {} --handle {} --json",
            command_arg(&resolved.project_dir),
            record.handle()
        )));
    }

    Ok(json!({
        "schema": "powerbi-cli.model.partitions.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "partition": partition_detail_json(record, options.include_source),
        "block": options.include_source.then(|| record.block.clone()),
        "next": [
            format!("powerbi-cli handoff check {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate --strict {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

pub(crate) fn partition_summary_json(record: &PartitionRecord) -> Value {
    json!({
        "handle": record.handle(),
        "table": record.table,
        "name": record.name,
        "expressionKind": record.expression_kind,
        "mode": record.mode,
        "sourceKind": record.source_kind,
        "sourcePreview": record.source.as_deref().map(source_preview),
        "offlineSafety": safety_json(record),
        "path": canonical_display(&record.path),
        "lineRange": {
            "start": record.start_line + 1,
            "end": record.end_line
        },
        "sourceRange": source_range_json(record)
    })
}

pub(crate) fn partition_detail_json(record: &PartitionRecord, include_source: bool) -> Value {
    let mut value = partition_summary_json(record);
    value["source"] = record
        .source
        .as_deref()
        .map(|source| {
            if include_source {
                source.to_string()
            } else {
                source_preview(source)
            }
        })
        .map(Value::String)
        .unwrap_or(Value::Null);
    value["sourceIncluded"] = Value::Bool(include_source);
    value
}

fn safety_json(record: &PartitionRecord) -> Value {
    let safety = &record.safety;
    json!({
        "status": safety.status,
        "safeForHome": safety.status == "safe",
        "generatedDummyTable": record.source_kind == "dummyMTable",
        "externalConnector": partition_source_kind_is_external(&record.source_kind),
        "credentialLikeText": safety.findings.iter().any(|finding| finding.code == "partition.credential_like_text"),
        "findings": safety.findings.iter().map(|finding| json!({
            "code": finding.code,
            "severity": finding.severity,
            "message": finding.message
        })).collect::<Vec<_>>()
    })
}

fn source_range_json(record: &PartitionRecord) -> Value {
    match (record.source_start_line, record.source_end_line) {
        (Some(start), Some(end)) => json!({
            "start": start + 1,
            "end": end
        }),
        _ => Value::Null,
    }
}

fn source_preview(source: &str) -> String {
    let redacted = redact_credential_values(source);
    let compact = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 180 {
        let mut value = compact.chars().take(177).collect::<String>();
        value.push_str("...");
        value
    } else {
        compact
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
            "--table" => options.table = Some(take_value(args, &mut i, "--table")?),
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model partitions list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli model partitions list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli model partitions list --project <project-dir-or.pbip> --json",
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
            "--table" => options.selector.table = Some(take_value(args, &mut i, "--table")?),
            "--name" | "--partition" => {
                options.selector.name = Some(take_value(args, &mut i, "--name")?);
            }
            "--include-source" => {
                options.include_source = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown model partitions show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli model partitions show --project <project-dir-or.pbip> --handle <partition-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli model partitions show --project <project-dir-or.pbip> --handle <partition-handle> --json",
                ));
            }
        }
    }
    Ok(options)
}

fn require_selector(selector: &PartitionSelector, command: &str) -> CliResult<()> {
    if selector.handle.is_some() || (selector.table.is_some() && selector.name.is_some()) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{command} requires --handle or --table plus --name"
    ))
    .with_hint("Use `model partitions list` to get stable partition handles.")
    .with_suggested_command(format!(
        "powerbi-cli {command} --project <project-dir-or.pbip> --handle <partition-handle> --json"
    )))
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
            .with_hint("Run `powerbi-cli --json capabilities --for partitions` for exact usage.")
            .with_suggested_command("powerbi-cli --json capabilities --for partitions")
    })?;
    *index += 2;
    Ok(value.clone())
}
