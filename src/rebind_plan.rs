use crate::project_io::write_text_atomic;
use crate::source_templates::{
    SOURCE_TEMPLATES_SCHEMA, SourceTemplateStore, find_template, load_source_template_store,
    source_template_findings_json, source_template_json, source_templates_path,
};
use crate::tmdl::{PartitionRecord, load_table_documents, same_name};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project,
};
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct RebindOptions {
    project: Option<PathBuf>,
    templates: Option<String>,
    table: Option<String>,
    partition: Option<String>,
    allow_unmapped: bool,
    out: Option<PathBuf>,
    force: bool,
}

pub(crate) fn rebind_plan(args: &[String]) -> CliResult<Value> {
    let options = parse_rebind_args(args)?;
    let project = options.project.ok_or_else(|| {
        CliError::invalid_args("handoff rebind-plan requires <project-dir-or.pbip> or --project")
            .with_hint(
                "Pass the PBIP project directory or the .pbip file to plan work-machine rebinding.",
            )
            .with_suggested_command("powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json")
    })?;
    let resolved = resolve_project(&project)?;
    let docs = load_table_documents(&resolved)?;
    let (store, template_path) =
        load_rebind_template_store(&resolved, options.templates.as_deref())?;
    let partitions = docs
        .iter()
        .flat_map(|doc| doc.partitions.iter())
        .filter(|partition| {
            options
                .table
                .as_ref()
                .is_none_or(|table| same_name(table, &partition.table))
        })
        .filter(|partition| {
            options
                .partition
                .as_ref()
                .is_none_or(|selector| selector_matches_partition(selector, partition))
        })
        .collect::<Vec<_>>();
    let mut findings = Vec::new();
    for template in &store.templates {
        findings.extend(source_template_findings_json(template, &template_path));
    }

    let mut plans = Vec::new();
    let mut mapped = 0usize;
    let mut dummy = 0usize;
    let mut unmapped = 0usize;
    for partition in &partitions {
        if partition.source_kind == "dummyMTable" {
            dummy += 1;
            if partition.safety.status == "review" {
                findings.push(json!({
                    "code": "rebindPlan.partition_requires_review",
                    "severity": "warning",
                    "message": format!("rebind plan partition requires safety review before handoff: {}", partition.handle()),
                    "handle": partition.handle(),
                    "path": canonical_display(&partition.path)
                }));
            } else if partition.safety.status != "safe" {
                findings.push(json!({
                    "code": "rebindPlan.partition_unsafe",
                    "severity": "error",
                    "message": format!("rebind plan partition is not safe for handoff: {}", partition.handle()),
                    "handle": partition.handle(),
                    "path": canonical_display(&partition.path)
                }));
            }
        } else {
            findings.push(json!({
                "code": "rebindPlan.partition_not_dummy",
                "severity": "error",
                "message": format!("rebind plan expects a dummy #table partition; {} uses {}", partition.handle(), partition.source_kind),
                "handle": partition.handle(),
                "path": canonical_display(&partition.path)
            }));
        }
        let template = find_template(&store, &partition.handle());
        if template.is_some() {
            mapped += 1;
        } else {
            unmapped += 1;
            findings.push(json!({
                "code": "rebindPlan.missing_source_template",
                "severity": if options.allow_unmapped { "warning" } else { "error" },
                "message": format!("no source template is configured for {}", partition.handle()),
                "handle": partition.handle(),
                "path": Value::Null,
                "suggestedCommand": format!("powerbi-cli source-template add --project {} --handle {} --kind sql --dry-run --json", command_arg(&resolved.project_dir), shell_arg(&partition.handle()))
            }));
        }
        let template_json = template.map(|record| source_template_json(record, &template_path));
        let m_template = template_json
            .as_ref()
            .and_then(|value| value["mTemplate"].as_str())
            .map(ToOwned::to_owned);
        plans.push(json!({
            "handle": format!("rebind:{}:{}", partition.table, partition.name),
            "partitionHandle": partition.handle(),
            "table": partition.table,
            "partition": partition.name,
            "currentSourceKind": partition.source_kind,
            "sourceRange": source_range_json(partition),
            "template": template_json,
            "mTemplate": m_template,
            "manualSteps": [
                "Open the PBIP at work in Power BI Desktop.",
                "Replace the dummy #table partition source with the rendered corporate source template.",
                "Configure credentials in Power BI Desktop inside the corporate environment.",
                "Refresh, validate relationships and measures, then save according to workplace process."
            ]
        }));
    }
    plans.sort_by(|left, right| {
        left["partitionHandle"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["partitionHandle"].as_str().unwrap_or_default())
    });

    let error_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count();
    let ok = error_count == 0;
    let review_count = findings
        .iter()
        .filter(|finding| finding["severity"] == "warning")
        .count();
    let status = if error_count > 0 {
        "unsafe"
    } else if review_count > 0
        || partitions
            .iter()
            .any(|partition| partition.safety.status == "review")
    {
        "review"
    } else {
        "safe"
    };
    let unsafe_template_detected = findings.iter().any(|finding| {
        finding["severity"] == "error"
            && finding["code"]
                .as_str()
                .is_some_and(|code| code.starts_with("sourceTemplate."))
    });
    let partition_credential_detected = partitions.iter().any(|partition| {
        partition
            .safety
            .findings
            .iter()
            .any(|finding| finding.code == "partition.credential_like_text")
    });
    let materialization_blocked = unsafe_template_detected || partition_credential_detected;
    let project_arg = command_arg(&resolved.project_dir);
    let handoff = format!("powerbi-cli handoff check {} --json", project_arg);
    let validate = format!("powerbi-cli validate --strict {} --json", project_arg);
    let partition_list = format!(
        "powerbi-cli model partitions list --project {} --json",
        project_arg
    );
    let markdown = rebind_plan_markdown(&resolved, &plans, &findings);
    let runbook_written = if let Some(out) = options.out.as_ref()
        && !materialization_blocked
    {
        write_rebind_runbook(out, &markdown, options.force)?;
        true
    } else {
        false
    };
    let runbook_path = runbook_written
        .then(|| options.out.as_ref().map(|path| canonical_display(path)))
        .flatten();

    Ok(json!({
        "schema": "powerbi-cli.handoff.rebind-plan.v1",
        "ok": ok,
        "complete": ok && unmapped == 0,
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "safeForOfflineHandoff": status == "safe" && partitions.iter().all(|partition| partition.source_kind == "dummyMTable" && partition.safety.status == "safe"),
        "status": status,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "templateStore": canonical_display(&template_path),
        "counts": {
            "partitions": partitions.len(),
            "dummyPartitions": dummy,
            "templates": store.templates.len(),
            "mappedPartitions": mapped,
            "unmappedPartitions": unmapped,
            "findings": findings.len(),
            "errors": error_count,
            "review": review_count
        },
        "plans": plans,
        "templates": store.templates.iter().map(|template| source_template_json(template, &template_path)).collect::<Vec<_>>(),
        "findings": findings,
        "instructionsMarkdown": markdown,
        "runbookPath": runbook_path,
        "runbookRequestedPath": options.out.as_ref().map(|path| canonical_display(path)),
        "runbookWritten": runbook_written,
        "materializationBlocked": materialization_blocked,
        "materializationBlockReasons": {
            "unsafeTemplate": unsafe_template_detected,
            "partitionCredential": partition_credential_detected
        },
        "handoffCheckCommand": handoff,
        "validateCommand": validate,
        "next": [handoff, validate, partition_list]
    }))
}

fn parse_rebind_args(args: &[String]) -> CliResult<RebindOptions> {
    let mut options = RebindOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                if options.project.is_some() {
                    return Err(one_project_error("handoff rebind-plan"));
                }
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--templates" => options.templates = Some(take_value(args, &mut i, "--templates")?),
            "--table" => options.table = Some(take_value(args, &mut i, "--table")?),
            "--partition" | "--handle" => {
                options.partition = Some(take_value(args, &mut i, "--partition")?);
            }
            "--allow-unmapped" => {
                options.allow_unmapped = true;
                i += 1;
            }
            "--out" | "--out-file" => {
                if options.out.is_some() {
                    return Err(CliError::invalid_args(
                        "handoff rebind-plan accepts exactly one --out path",
                    )
                    .with_hint("Choose one Markdown runbook output path.")
                    .with_suggested_command(
                        "powerbi-cli handoff rebind-plan <project-dir-or.pbip> --out <file.md> --json",
                    ));
                }
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?));
            }
            "--force" => {
                options.force = true;
                i += 1;
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown handoff rebind-plan flag: {other}"
                ))
                .with_hint("Run `powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json`.")
                .with_suggested_command(
                    "powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json",
                ));
            }
            other => {
                if options.project.is_some() {
                    return Err(one_project_error("handoff rebind-plan"));
                }
                options.project = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    if options.force && options.out.is_none() {
        return Err(CliError::invalid_args(
            "handoff rebind-plan --force requires --out <file.md>",
        )
        .with_hint("Use --force only to replace an existing Markdown runbook.")
        .with_suggested_command(
            "powerbi-cli handoff rebind-plan <project-dir-or.pbip> --out <file.md> --force --json",
        ));
    }
    Ok(options)
}

fn load_rebind_template_store(
    resolved: &crate::ResolvedProject,
    templates: Option<&str>,
) -> CliResult<(SourceTemplateStore, PathBuf)> {
    if let Some(path) = templates {
        let text = if path == "-" {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .map_err(|err| CliError::unexpected(format!("read templates from stdin: {err}")))?;
            text
        } else {
            fs::read_to_string(path)
                .map_err(|err| CliError::file_not_found(format!("read templates {path}: {err}")))?
        };
        let mut store: SourceTemplateStore = serde_json::from_str(&text)
            .map_err(|err| CliError::validation_failed(format!("parse templates {path}: {err}")))?;
        if store.schema.trim().is_empty() {
            store.schema = SOURCE_TEMPLATES_SCHEMA.to_string();
        }
        if store.schema != SOURCE_TEMPLATES_SCHEMA {
            return Err(CliError::validation_failed(format!(
                "unsupported source template schema in {path}: {}",
                store.schema
            )));
        }
        return Ok((store, PathBuf::from(path)));
    }
    Ok((
        load_source_template_store(resolved)?,
        source_templates_path(&resolved.project_dir),
    ))
}

fn selector_matches_partition(selector: &str, partition: &PartitionRecord) -> bool {
    if selector.starts_with("partition:") {
        selector == partition.handle()
    } else {
        same_name(selector, &partition.name)
    }
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

fn rebind_plan_markdown(
    resolved: &crate::ResolvedProject,
    plans: &[Value],
    findings: &[Value],
) -> String {
    let mut out = String::new();
    out.push_str("# Power BI Rebind Plan\n\n");
    out.push_str(&format!(
        "Project: `{}`\n\n",
        canonical_display(&resolved.project_dir)
    ));
    out.push_str(&format!(
        "Power BI project: `{}`\n\n",
        canonical_display(&resolved.pbip_path)
    ));
    if findings
        .iter()
        .any(|finding| finding["severity"] == "error")
    {
        out.push_str("Status: incomplete. Resolve error findings before relying on this plan.\n\n");
    } else {
        out.push_str("Status: ready for work-machine review.\n\n");
    }

    out.push_str("## Prerequisites\n\n");
    out.push_str("- Power BI Desktop is installed on the work machine.\n");
    if plans.iter().any(|plan| {
        plan["template"]["kind"]
            .as_str()
            .is_some_and(|kind| kind == "postgres")
    }) {
        out.push_str(
            "- PostgreSQL templates require the Npgsql driver for the Power BI PostgreSQL connector to be installed on the work machine.\n",
        );
    }
    if plans.iter().any(|plan| {
        plan["template"]["kind"]
            .as_str()
            .is_some_and(|kind| kind == "odbc")
    }) {
        out.push_str(
            "- Every ODBC DSN named by a template must already exist on the work machine.\n",
        );
    }
    out.push_str("- The work machine can reach the required corporate data sources.\n\n");

    out.push_str("## Rebind procedure\n\n");
    out.push_str("1. Open the `.pbip` project in Power BI Desktop at work.\n");
    out.push_str(
        "2. Open Transform data / Power Query and locate each table or partition listed below.\n",
    );
    out.push_str("3. Replace its dummy `#table(...)` query with the corresponding M snippet.\n");
    out.push_str(
        "4. Enter source credentials only in Power BI Desktop when prompted, then apply the query changes.\n\n",
    );

    if !findings.is_empty() {
        out.push_str("## Plan findings\n\n");
        for finding in findings {
            out.push_str(&format!(
                "- **{}** `{}`: {}\n",
                finding["severity"].as_str().unwrap_or("finding"),
                finding["code"].as_str().unwrap_or("rebindPlan.finding"),
                finding["message"]
                    .as_str()
                    .unwrap_or("Review this finding.")
            ));
        }
        out.push('\n');
    }

    out.push_str("## Partition replacements\n\n");
    for plan in plans {
        out.push_str(&format!(
            "### `{}`\n\n",
            plan["partitionHandle"].as_str().unwrap_or("<partition>")
        ));
        if let Some(kind) = plan["template"]["kind"].as_str() {
            out.push_str(&format!("Template kind: `{kind}`\n\n"));
        }
        if let Some(requirements) = plan["template"]["requirements"].as_array()
            && !requirements.is_empty()
        {
            out.push_str("Template requirements:\n\n");
            for requirement in requirements {
                if let Some(requirement) = requirement.as_str() {
                    out.push_str(&format!("- {requirement}\n"));
                }
            }
            out.push('\n');
        }
        if let Some(template) = plan["mTemplate"].as_str() {
            out.push_str("Replace the dummy partition source with:\n\n```m\n");
            out.push_str(template);
            out.push_str("\n```\n\n");
        } else {
            out.push_str(
                "No source template configured yet. Add one with `source-template add`.\n\n",
            );
        }
    }
    out.push_str("## Post-rebind verification\n\n");
    out.push_str("- [ ] Refresh completes successfully for every rebound table.\n");
    out.push_str("- [ ] Every report page canvas renders with the expected visuals and data.\n");
    out.push_str("- [ ] No Power BI Desktop issue, warning, or error banners remain.\n");
    out.push_str(
        "- [ ] Optional, if `powerbi-cli` is available at work: re-run fixture normalization and verification.\n",
    );
    out.push_str(
        "  - `powerbi-cli fixture normalize <project-dir-or.pbip> --out <work-machine-summary.json> --json`\n",
    );
    out.push_str(
        "  - `powerbi-cli fixture verify <project-dir-or.pbip> --expected <approved-summary.json> --json`\n\n",
    );
    out.push_str(
        "Credentials must live only in Power BI Desktop on the work machine. Never put them in TMDL, source-template sidecar metadata, or this runbook.\n",
    );
    out
}

fn write_rebind_runbook(path: &std::path::Path, markdown: &str, force: bool) -> CliResult<()> {
    if path.exists() && !force {
        return Err(CliError::invalid_args(format!(
            "rebind runbook output already exists: {}",
            path.display()
        ))
        .with_hint("Pass --force after reviewing the existing file, or choose a new --out path.")
        .with_suggested_command(
            "powerbi-cli handoff rebind-plan <project-dir-or.pbip> --out <file.md> --force --json",
        ));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    }
    if path.exists() {
        write_text_atomic(path, markdown)
    } else {
        fs::write(path, markdown)
            .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))
    }
}

fn one_project_error(command: &str) -> CliError {
    CliError::invalid_args(format!("{command} accepts exactly one project"))
        .with_hint("Pass either a positional project path or --project, not both.")
        .with_suggested_command(format!(
            "powerbi-cli {command} <project-dir-or.pbip> --json"
        ))
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json`.")
            .with_suggested_command("powerbi-cli handoff rebind-plan <project-dir-or.pbip> --json")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
