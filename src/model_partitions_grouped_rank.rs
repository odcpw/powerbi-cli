//! Guarded refresh-time grouped-rank M generation for safe dummy partitions.

use crate::cli_support::{
    MutationMode, mode_name, preflight_out_dir, require_mode_with_contract, required_project,
    set_mode_with_contract, shell_arg, take_value, target_project,
};
use crate::project_io::write_text_atomic_validated;
use crate::safety_scan::contains_credential_like_text_str;
use crate::tmdl::{
    PartitionSelector, find_column, find_table, load_table_documents,
    replace_partition_source_plan, same_name,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

const COMMAND: &str = "model partitions add-grouped-rank";
const MAX_PREDICATE_BYTES: usize = 4_096;
const GENERATED_PREFIX: &str = "PowerBICliGroupedRank";

#[derive(Debug, Default)]
struct Options {
    project: Option<PathBuf>,
    table: Option<String>,
    group_by: Vec<String>,
    order_by: Option<String>,
    descending: bool,
    desc_seen: bool,
    rank_column: Option<String>,
    eligible_when: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct ResolvedColumn {
    model: String,
    source: String,
}

pub(crate) fn add_grouped_rank_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    let project = required_project(options.project.clone(), COMMAND)?;
    let table = required_option(options.table.as_deref(), "--table <table>")?;
    if options.group_by.is_empty() {
        return Err(arguments_error(format!(
            "{COMMAND} requires at least one --group-by <column>"
        )));
    }
    let order_by = required_option(options.order_by.as_deref(), "--order-by <column>")?;
    let rank_column = required_option(options.rank_column.as_deref(), "--rank-column <column>")?;
    let eligible_when = required_option(
        options.eligible_when.as_deref(),
        "--eligible-when <M-predicate>",
    )?;
    validate_predicate(eligible_when)?;
    let mode = require_mode_with_contract(
        options.mode,
        COMMAND,
        "Start with --dry-run and review the complete generated M source before applying it.",
        example_command(),
    )?;

    let source_resolved = resolve_project(&project)?;
    preflight_out_dir(args, add_grouped_rank_command)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let docs = load_table_documents(&target_resolved)?;
    let table_doc = find_table(&docs, table)?;
    let [partition] = table_doc.partitions.as_slice() else {
        return Err(arguments_error(format!(
            "{COMMAND} requires table {} to have exactly one partition; found {}",
            table_doc.table,
            table_doc.partitions.len()
        )));
    };
    if partition.source_kind != "dummyMTable" || partition.safety.status != "safe" {
        return Err(arguments_error(format!(
            "{COMMAND} only modifies a safe generated dummy partition; {} is {} ({})",
            partition.handle(),
            partition.source_kind,
            partition.safety.status
        ))
        .with_hint("Rebuild a reviewed offline dummy partition before generating refresh-time analytics; live, unknown, and unsafe M is never patched."));
    }
    let source = partition.source.as_deref().ok_or_else(|| {
        arguments_error(format!(
            "{COMMAND} requires a generated partition source: {}",
            partition.handle()
        ))
    })?;
    if source
        .to_ascii_lowercase()
        .contains(&GENERATED_PREFIX.to_ascii_lowercase())
    {
        return Err(arguments_error(format!(
            "{} already contains a generated grouped-rank chain",
            partition.handle()
        ))
        .with_hint("Restore the original generated dummy partition before changing grouped-rank parameters."));
    }

    let mut group_columns = Vec::new();
    let mut group_names = BTreeSet::new();
    for requested in &options.group_by {
        let column = source_column(&docs, &table_doc.table, requested, "group-by")?;
        let canonical = column.model.to_ascii_lowercase();
        if !group_names.insert(canonical) {
            return Err(arguments_error(format!(
                "duplicate --group-by column: {}",
                column.model
            )));
        }
        group_columns.push(column);
    }
    let order_column = source_column(&docs, &table_doc.table, order_by, "order-by")?;
    if group_columns
        .iter()
        .any(|column| same_name(&column.model, &order_column.model))
    {
        return Err(arguments_error(
            "--order-by must name a different column than every --group-by",
        ));
    }
    let rank = source_column(&docs, &table_doc.table, rank_column, "rank")?;
    if group_columns
        .iter()
        .any(|column| same_name(&column.model, &rank.model))
        || same_name(&order_column.model, &rank.model)
    {
        return Err(arguments_error(
            "--rank-column must differ from group-by and order-by columns",
        ));
    }
    let rank_record = find_column(&docs, &table_doc.table, &rank.model)?;
    if !rank_record
        .data_type
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("int64"))
    {
        return Err(arguments_error(format!(
            "rank column {} must have TMDL dataType int64 for the generated 1-based/zero rank",
            rank_record.handle()
        )));
    }

    let generated_m = render_grouped_rank_m(
        source,
        &group_columns,
        &order_column,
        options.descending,
        &rank,
        eligible_when,
    )?;
    let selector = PartitionSelector {
        handle: Some(partition.handle()),
        table: None,
        name: None,
    };
    let plan = replace_partition_source_plan(&docs, &selector, &generated_m)?;
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
    let readback = format!(
        "powerbi-cli model partitions show --project {project_arg} --handle {} --json",
        shell_arg(&plan.handle)
    );
    let lint = format!("powerbi-cli lint {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let handoff = format!("powerbi-cli handoff check {project_arg} --json");

    Ok(json!({
        "schema": "powerbi-cli.model.partitions.addGroupedRank.v1",
        "ok": validation_ok,
        "exitCode": if validation_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "action": "add-grouped-rank",
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
            "table": table_doc.table,
            "partition": partition.name,
            "path": canonical_display(&plan.path)
        },
        "rank": {
            "groupBy": group_columns.iter().map(column_json).collect::<Vec<_>>(),
            "orderBy": column_json(&order_column),
            "direction": if options.descending { "descending" } else { "ascending" },
            "rankColumn": column_json(&rank),
            "eligibleWhen": eligible_when,
            "ineligibleRank": 0,
            "eligibleRankStart": 1,
            "finalType": "Int64.Type"
        },
        "changes": [{
            "kind": "tmdl.partition.groupedRankM",
            "action": "replace-source",
            "path": canonical_display(&plan.path),
            "before": plan.before_block,
            "after": plan.after_block
        }],
        "validation": validation.map(|report| json!({
            "ok": report.errors.is_empty(),
            "warnings": report.warnings,
            "errors": report.errors
        })),
        "readbackCommand": readback,
        "inspectCommand": format!("powerbi-cli inspect --deep {project_arg} --json"),
        "lintCommand": lint,
        "validateCommand": validate,
        "handoffCheckCommand": handoff,
        "next": [readback, lint, validate, handoff]
    }))
}

fn parse_args(args: &[String]) -> CliResult<Options> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut index, "--project")?));
            }
            "--table" => options.table = Some(take_value(args, &mut index, "--table")?),
            "--group-by" => options
                .group_by
                .push(take_value(args, &mut index, "--group-by")?),
            "--order-by" => {
                if options.order_by.is_some() {
                    return Err(arguments_error("--order-by may be specified only once"));
                }
                options.order_by = Some(take_value(args, &mut index, "--order-by")?);
            }
            "--desc" => {
                if options.desc_seen {
                    return Err(arguments_error("--desc may be specified only once"));
                }
                options.desc_seen = true;
                options.descending = true;
                index += 1;
            }
            "--rank-column" => {
                if options.rank_column.is_some() {
                    return Err(arguments_error("--rank-column may be specified only once"));
                }
                options.rank_column = Some(take_value(args, &mut index, "--rank-column")?);
            }
            "--eligible-when" => {
                if options.eligible_when.is_some() {
                    return Err(arguments_error(
                        "--eligible-when may be specified only once",
                    ));
                }
                options.eligible_when = Some(take_value(args, &mut index, "--eligible-when")?);
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                index += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                index += 1;
            }
            "--out-dir" | "--out" => {
                options.out_dir = Some(PathBuf::from(take_value(args, &mut index, "--out-dir")?));
                set_mode(&mut options.mode, MutationMode::OutDir)?;
            }
            other => {
                return Err(arguments_error(format!("unknown {COMMAND} flag: {other}")));
            }
        }
    }
    Ok(options)
}

fn set_mode(mode: &mut Option<MutationMode>, next: MutationMode) -> CliResult<()> {
    set_mode_with_contract(
        mode,
        next,
        "Start with --dry-run and review the generated M before choosing an output mode.",
        example_command(),
    )
}

fn required_option<'a>(value: Option<&'a str>, flag: &str) -> CliResult<&'a str> {
    value.ok_or_else(|| arguments_error(format!("{COMMAND} requires {flag}")))
}

fn arguments_error(message: impl Into<String>) -> CliError {
    CliError::invalid_args(message)
        .with_hint("Use a safe generated table with existing source columns and inspect the dry-run M before applying it.")
        .with_suggested_command(example_command())
}

fn example_command() -> &'static str {
    "powerbi-cli model partitions add-grouped-rank --project <project-dir-or.pbip> --table <table> --group-by <column> --order-by <column> --desc --rank-column <int64-column> --eligible-when <M-predicate> --dry-run --json"
}

fn source_column(
    docs: &[crate::tmdl::TableDocument],
    table: &str,
    requested: &str,
    role: &str,
) -> CliResult<ResolvedColumn> {
    let column = find_column(docs, table, requested)?;
    if column.is_calculated() {
        return Err(arguments_error(format!(
            "{role} column {} is calculated and unavailable in partition M",
            column.handle()
        )));
    }
    Ok(ResolvedColumn {
        model: column.name.clone(),
        source: column
            .source_column
            .clone()
            .unwrap_or_else(|| column.name.clone()),
    })
}

fn column_json(column: &ResolvedColumn) -> Value {
    json!({ "model": column.model, "source": column.source })
}

fn validate_predicate(predicate: &str) -> CliResult<()> {
    let trimmed = predicate.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_PREDICATE_BYTES || trimmed != predicate {
        return Err(arguments_error(format!(
            "--eligible-when must be a trimmed non-empty single-line M predicate of at most {MAX_PREDICATE_BYTES} bytes"
        )));
    }
    if predicate.chars().any(|character| character.is_control())
        || predicate.contains("//")
        || predicate.contains("/*")
        || predicate.contains("*/")
    {
        return Err(arguments_error(
            "--eligible-when must not contain control characters or M comments",
        ));
    }
    if contains_credential_like_text_str(predicate) {
        return Err(arguments_error(
            "--eligible-when contains credential-like text",
        ));
    }
    let lower = predicate.to_ascii_lowercase();
    if [
        ".database(",
        "web.contents(",
        "file.contents(",
        "folder.files(",
        "csv.document(",
        "excel.workbook(",
        "expression.evaluate(",
        "#shared",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return Err(arguments_error(
            "--eligible-when must be a row predicate and cannot access connectors, files, shared globals, or dynamic evaluation",
        ));
    }
    validate_balanced_fragment(predicate)
}

fn validate_balanced_fragment(value: &str) -> CliResult<()> {
    let chars = value.chars().collect::<Vec<_>>();
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if quoted {
            if character == '"' && chars.get(index + 1) == Some(&'"') {
                index += 2;
                continue;
            }
            if character == '"' {
                quoted = false;
            }
            index += 1;
            continue;
        }
        if character == '"' {
            quoted = true;
        } else if matches!(character, '(' | '[' | '{') {
            stack.push(character);
        } else if matches!(character, ')' | ']' | '}') {
            let expected = match character {
                ')' => '(',
                ']' => '[',
                '}' => '{',
                _ => unreachable!(),
            };
            if stack.pop() != Some(expected) {
                return Err(arguments_error(
                    "--eligible-when has unbalanced M delimiters",
                ));
            }
        } else if character == ',' && stack.is_empty() {
            return Err(arguments_error(
                "--eligible-when cannot contain a top-level comma",
            ));
        }
        index += 1;
    }
    if quoted || !stack.is_empty() {
        return Err(arguments_error(
            "--eligible-when has an unterminated string or unbalanced M delimiters",
        ));
    }
    Ok(())
}

fn render_grouped_rank_m(
    source: &str,
    group_by: &[ResolvedColumn],
    order_by: &ResolvedColumn,
    descending: bool,
    rank: &ResolvedColumn,
    eligible_when: &str,
) -> CliResult<String> {
    let (body, result) = split_let_source(source)?;
    let group_list = group_by
        .iter()
        .map(|column| m_string(&column.source))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sort_fields = group_by
        .iter()
        .map(|column| format!("{{{}, Order.Ascending}}", m_string(&column.source)))
        .collect::<Vec<_>>();
    sort_fields.push(format!(
        "{{{}, Order.{}}}",
        m_string(&order_by.source),
        if descending {
            "Descending"
        } else {
            "Ascending"
        }
    ));
    let rank_name = m_string(&rank.source);
    Ok(format!(
        "let{body},\n    {GENERATED_PREFIX}Input = Table.RemoveColumns({result}, {{{rank_name}}}),\n    {GENERATED_PREFIX}Sorted = Table.Sort({GENERATED_PREFIX}Input, {{{}}}),\n    {GENERATED_PREFIX}Grouped = Table.Group(\n        {GENERATED_PREFIX}Sorted,\n        {{{group_list}}},\n        {{\n            {{\n                \"__PowerBICliGroupedRankRows\",\n                ({GENERATED_PREFIX}Rows as table) as table =>\n                    let\n                        {GENERATED_PREFIX}Source = Table.Buffer({GENERATED_PREFIX}Rows),\n                        {GENERATED_PREFIX}Eligible = Table.SelectRows({GENERATED_PREFIX}Source, each {eligible_when}),\n                        {GENERATED_PREFIX}Ineligible = Table.SelectRows({GENERATED_PREFIX}Source, each not ({eligible_when})),\n                        {GENERATED_PREFIX}Indexed = Table.AddIndexColumn({GENERATED_PREFIX}Eligible, {rank_name}, 1, 1, Int64.Type),\n                        {GENERATED_PREFIX}Zeroed = Table.AddColumn({GENERATED_PREFIX}Ineligible, {rank_name}, each 0, Int64.Type)\n                    in\n                        Table.Combine({{{GENERATED_PREFIX}Indexed, {GENERATED_PREFIX}Zeroed}}),\n                type table\n            }}\n        }},\n        GroupKind.Local\n    ),\n    {GENERATED_PREFIX}Combined = Table.Combine(Table.Column({GENERATED_PREFIX}Grouped, \"__PowerBICliGroupedRankRows\")),\n    {GENERATED_PREFIX}Typed = Table.TransformColumnTypes({GENERATED_PREFIX}Combined, {{{{{rank_name}, Int64.Type}}}})\nin\n    {GENERATED_PREFIX}Typed",
        sort_fields.join(", ")
    ))
}

fn split_let_source(source: &str) -> CliResult<(String, String)> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim();
    if !trimmed
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("let"))
    {
        return Err(arguments_error(
            "generated partition source must begin with a let expression",
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let in_position = lower.rfind("\nin\n").ok_or_else(|| {
        arguments_error("generated partition source must end with a standalone in expression")
    })?;
    let body = trimmed[3..in_position].trim_end();
    let result = trimmed[in_position + 4..].trim();
    if body.is_empty() || result.is_empty() {
        return Err(arguments_error(
            "generated partition let expression has an empty body or result",
        ));
    }
    Ok((body.to_string(), result.to_string()))
}

fn m_string(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_rank_renderer_emits_buffer_split_index_zero_and_final_retype() {
        let source = "let\n    Source = #table(type table [Group = text, Score = number, Eligible = logical, Rank = number], {})\nin\n    Source";
        let rendered = render_grouped_rank_m(
            source,
            &[ResolvedColumn {
                model: "Group".into(),
                source: "Group".into(),
            }],
            &ResolvedColumn {
                model: "Score".into(),
                source: "Score".into(),
            },
            true,
            &ResolvedColumn {
                model: "Rank".into(),
                source: "Rank".into(),
            },
            "[Eligible] = true",
        )
        .expect("render grouped rank");
        assert_eq!(
            rendered,
            include_str!("../testdata/golden/model-partitions/grouped-rank-descending.m")
                .trim_end()
        );
        assert!(rendered.contains("Table.Sort"));
        assert!(rendered.contains("Order.Descending"));
        assert!(rendered.contains("Table.Buffer"));
        assert!(rendered.contains("Table.AddIndexColumn"));
        assert!(rendered.contains("each 0, Int64.Type"));
        assert!(rendered.contains("Table.TransformColumnTypes"));
        assert!(rendered.ends_with("PowerBICliGroupedRankTyped"));
    }

    #[test]
    fn eligibility_predicate_refuses_injection_and_external_access() {
        for predicate in [
            "[Eligible], Injected = 1",
            "Web.Contents(\"https://example.invalid\") <> null",
            "[Value] > 0 // hide suffix",
            "[Value] > (0",
            "Password = \"test-only-placeholder\"",
        ] {
            assert!(
                validate_predicate(predicate).is_err(),
                "accepted {predicate}"
            );
        }
        validate_predicate("[Eligible] = true and Number.Abs([Score]) > 0")
            .expect("safe row predicate");
    }
}
