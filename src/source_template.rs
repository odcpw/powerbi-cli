use crate::project_io::{copy_project_dir, write_text_atomic_validated};
use crate::source_templates::{
    ExcelSourceTemplateInput, OdbcSourceTemplateInput, PostgresSourceTemplateInput,
    SourceTemplateRecord, SqlSourceTemplateInput, excel_source_template, find_template,
    load_source_template_store, odbc_source_template, postgres_source_template,
    save_source_template_store, source_template_findings, source_template_json,
    source_templates_path, sql_source_template, template_has_errors, upsert_template,
};
use crate::tmdl::{
    ColumnRecord, PartitionSelector, find_partition, load_table_documents,
    partition_selector_parts, replace_partition_source_plan, same_name,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(crate) fn source_template_command(args: &[String]) -> CliResult<Value> {
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "source-template requires a subcommand: list, show, add, apply",
        )
        .with_hint("Run `powerbi-cli source-template list --project <project-dir-or.pbip> --json`.")
        .with_suggested_command(
            "powerbi-cli source-template list --project <project-dir-or.pbip> --json",
        ));
    };

    match normalize_action(action).as_str() {
        "list" => list_source_templates(rest),
        "show" => show_source_template(rest),
        "add" => add_source_template(rest),
        "apply" | "materialize" => apply_source_template(rest),
        _ => Err(CliError::invalid_args(format!(
            "unknown source-template command: {action}"
        ))
        .with_hint(
            "Run `powerbi-cli --json capabilities --for source-template` for supported source-template commands.",
        )
        .with_suggested_command("powerbi-cli --json capabilities --for source-template")),
    }
}

#[derive(Debug, Default)]
struct ListOptions {
    project: Option<PathBuf>,
    table: Option<String>,
    kind: Option<String>,
}

#[derive(Debug, Default)]
struct ShowOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    name: Option<String>,
}

#[derive(Debug)]
enum MutationMode {
    DryRun,
    InPlace,
    OutDir(PathBuf),
}

#[derive(Debug, Default)]
struct AddOptions {
    project: Option<PathBuf>,
    selector: PartitionSelector,
    template_name: Option<String>,
    kind: Option<String>,
    server: Option<String>,
    dsn: Option<String>,
    database: Option<String>,
    sql_schema: Option<String>,
    object: Option<String>,
    file: Option<String>,
    item: Option<String>,
    item_kind: Option<String>,
    description: Option<String>,
    mode: Option<MutationMode>,
}

#[derive(Debug, Default)]
struct ApplyOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    name: Option<String>,
    server: Option<String>,
    dsn: Option<String>,
    database: Option<String>,
    sql_schema: Option<String>,
    object: Option<String>,
    file: Option<String>,
    item: Option<String>,
    item_kind: Option<String>,
    replace_existing: bool,
    confirm: Option<String>,
    mode: Option<MutationMode>,
}

fn list_source_templates(args: &[String]) -> CliResult<Value> {
    let options = parse_list_args(args)?;
    let project = required_project(options.project, "source-template list")?;
    let resolved = resolve_project(&project)?;
    let store = load_source_template_store(&resolved)?;
    let path = source_templates_path(&resolved.project_dir);
    let mut templates = store
        .templates
        .iter()
        .filter(|template| {
            options
                .table
                .as_ref()
                .is_none_or(|table| same_name(table, &template.table))
        })
        .filter(|template| {
            options
                .kind
                .as_ref()
                .is_none_or(|kind| same_name(kind, &template.kind))
        })
        .map(|template| source_template_json(template, &path))
        .collect::<Vec<_>>();
    templates.sort_by(|left, right| {
        left["handle"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["handle"].as_str().unwrap_or_default())
    });

    Ok(json!({
        "schema": "powerbi-cli.source-template.list.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "templateStore": canonical_display(&path),
        "filter": {
            "table": options.table,
            "kind": options.kind
        },
        "counts": {
            "templates": templates.len()
        },
        "templates": templates,
        "next": [
            format!("powerbi-cli source-template show --project {} --handle <source-template-handle> --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli handoff rebind-plan {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli handoff check {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn show_source_template(args: &[String]) -> CliResult<Value> {
    let options = parse_show_args(args)?;
    let project = required_project(options.project.clone(), "source-template show")?;
    let resolved = resolve_project(&project)?;
    let store = load_source_template_store(&resolved)?;
    let record = find_template_by_show_options(&store.templates, &options)?;
    let path = source_templates_path(&resolved.project_dir);

    Ok(json!({
        "schema": "powerbi-cli.source-template.show.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "templateStore": canonical_display(&path),
        "sourceTemplate": source_template_json(record, &path),
        "next": [
            format!("powerbi-cli handoff rebind-plan {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli handoff check {} --json", command_arg(&resolved.project_dir))
        ]
    }))
}

fn add_source_template(args: &[String]) -> CliResult<Value> {
    let options = parse_add_args(args)?;
    let source_project = required_project(options.project.clone(), "source-template add")?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = options.mode.as_ref().ok_or_else(|| {
        CliError::invalid_args("source-template add requires --dry-run, --in-place, or --out-dir <dir>")
            .with_hint("Start with `--dry-run`; source templates are sidecar metadata, not executable partition sources.")
            .with_suggested_command("powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json")
    })?;
    let kind = normalize_kind(options.kind.as_deref())?;
    if kind == "odbc" {
        validate_bare_odbc_dsn(options.dsn.as_deref().unwrap_or("<dsn>"))?;
    }

    let target_resolved = match mode {
        MutationMode::DryRun | MutationMode::InPlace => source_resolved,
        MutationMode::OutDir(out_dir) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)?
        }
    };

    let docs = load_table_documents(&target_resolved)?;
    let partition = find_partition(&docs, &options.selector)?;
    let record = match kind.as_str() {
        "sql" => {
            reject_flag_for_kind(&options.dsn, "--dsn", "sql", "--server")?;
            reject_flag_for_kind(&options.file, "--file", "sql", "--server")?;
            reject_flag_for_kind(&options.item, "--item", "sql", "--object")?;
            reject_flag_for_kind(&options.item_kind, "--item-kind", "sql", "--schema")?;
            let server = options.server.as_deref().unwrap_or("<server>");
            let database = options.database.as_deref().unwrap_or("<database>");
            let schema = options.sql_schema.as_deref().unwrap_or("dbo");
            let object = options.object.as_deref().unwrap_or(&partition.table);
            validate_template_parameters(&[
                ("server", server),
                ("database", database),
                ("schema", schema),
                ("object", object),
            ])?;
            sql_source_template(SqlSourceTemplateInput {
                table: partition.table.clone(),
                partition: partition.name.clone(),
                name: options.template_name.clone(),
                server: server.to_string(),
                database: database.to_string(),
                schema: schema.to_string(),
                object: object.to_string(),
                description: options.description.clone(),
            })
        }
        "postgres" => {
            reject_flag_for_kind(&options.dsn, "--dsn", "postgres", "--server")?;
            reject_flag_for_kind(&options.file, "--file", "postgres", "--server")?;
            reject_flag_for_kind(&options.item, "--item", "postgres", "--object")?;
            reject_flag_for_kind(&options.item_kind, "--item-kind", "postgres", "--schema")?;
            let server = options.server.as_deref().unwrap_or("<server>");
            let database = options.database.as_deref().unwrap_or("<database>");
            let schema = options.sql_schema.as_deref().unwrap_or("public");
            let object = options.object.as_deref().unwrap_or("<object>");
            validate_template_parameters(&[
                ("server", server),
                ("database", database),
                ("schema", schema),
                ("object", object),
            ])?;
            postgres_source_template(PostgresSourceTemplateInput {
                table: partition.table.clone(),
                partition: partition.name.clone(),
                name: options.template_name.clone(),
                server: server.to_string(),
                database: database.to_string(),
                schema: schema.to_string(),
                object: object.to_string(),
                description: options.description.clone(),
            })
        }
        "odbc" => {
            reject_flag_for_kind(&options.server, "--server", "odbc", "--dsn")?;
            reject_flag_for_kind(&options.file, "--file", "odbc", "--dsn")?;
            reject_flag_for_kind(&options.item, "--item", "odbc", "--object")?;
            reject_flag_for_kind(&options.item_kind, "--item-kind", "odbc", "--schema")?;
            let dsn = options.dsn.as_deref().unwrap_or("<dsn>");
            let database = options.database.as_deref().unwrap_or("<database>");
            let schema = options.sql_schema.as_deref().unwrap_or("<schema>");
            let object = options.object.as_deref().unwrap_or("<object>");
            validate_template_parameters(&[
                ("dsn", dsn),
                ("database", database),
                ("schema", schema),
                ("object", object),
            ])?;
            odbc_source_template(OdbcSourceTemplateInput {
                table: partition.table.clone(),
                partition: partition.name.clone(),
                name: options.template_name.clone(),
                dsn: dsn.to_string(),
                database: database.to_string(),
                schema: schema.to_string(),
                object: object.to_string(),
                description: options.description.clone(),
            })
        }
        "excel" => {
            reject_flag_for_kind(&options.server, "--server", "excel", "--file")?;
            reject_flag_for_kind(&options.dsn, "--dsn", "excel", "--file")?;
            reject_flag_for_kind(&options.database, "--database", "excel", "--file")?;
            reject_flag_for_kind(&options.sql_schema, "--schema", "excel", "--item-kind")?;
            reject_flag_for_kind(&options.object, "--object", "excel", "--item")?;
            let file = options.file.as_deref().unwrap_or("<file>");
            let item = options.item.as_deref().unwrap_or(&partition.table);
            let item_kind = normalize_excel_item_kind(options.item_kind.as_deref())?;
            if !(file.contains('<') && file.contains('>')) {
                validate_excel_file(file)?;
            }
            validate_template_parameters(&[
                ("file", file),
                ("item", item),
                ("itemKind", &item_kind),
            ])?;
            excel_source_template(ExcelSourceTemplateInput {
                table: partition.table.clone(),
                partition: partition.name.clone(),
                name: options.template_name.clone(),
                file: file.to_string(),
                item: item.to_string(),
                item_kind,
                description: options.description.clone(),
            })
        }
        _ => return Err(unsupported_kind_error(&kind)),
    };
    if template_has_errors(&record) {
        return Err(CliError::invalid_args(
            "source-template add refuses to store credential-like template text",
        )
        .with_hint("Use placeholders such as `<server>` and configure credentials only inside Power BI Desktop at work.")
        .with_suggested_command("powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind <sql|postgres|odbc|excel> --dry-run --json"));
    }

    let mut store = load_source_template_store(&target_resolved)?;
    let before = find_template(&store, &record.partition_handle).cloned();
    upsert_template(&mut store, record.clone());
    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        save_source_template_store(&target_resolved, &store)?;
    }
    let validation = if dry_run {
        None
    } else {
        Some(validate_project(&target_resolved)?)
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
    let path = source_templates_path(&target_resolved.project_dir);
    let readback = format!(
        "powerbi-cli source-template show --project {} --handle {} --json",
        project_arg,
        shell_arg(&record.handle)
    );
    let rebind = format!("powerbi-cli handoff rebind-plan {} --json", project_arg);
    let handoff = format!("powerbi-cli handoff check {} --json", project_arg);
    let validate = format!("powerbi-cli validate --strict {} --json", project_arg);

    Ok(json!({
        "schema": "powerbi-cli.source-template.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "add",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "templateStore": canonical_display(&path),
        "target": {
            "handle": record.handle,
            "partitionHandle": record.partition_handle,
            "table": record.table,
            "partition": record.partition,
            "path": canonical_display(&path)
        },
        "changes": [{
            "kind": "source-template",
            "action": if before.is_some() { "replace" } else { "add" },
            "path": canonical_display(&path),
            "before": before.as_ref().map(|record| source_template_json(record, &path)),
            "after": source_template_json(&record, &path)
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
                "visuals": report.visuals
            }
        })),
        "readbackCommand": readback,
        "rebindPlanCommand": rebind,
        "handoffCheckCommand": handoff,
        "validateCommand": validate,
        "next": [readback, rebind, handoff, validate]
    }))
}

fn apply_source_template(args: &[String]) -> CliResult<Value> {
    let options = parse_apply_args(args)?;
    let source_project = required_project(options.project.clone(), "source-template apply")?;
    let source_resolved = resolve_project(&source_project)?;
    let mode = options.mode.as_ref().ok_or_else(|| {
        CliError::invalid_args(
            "source-template apply requires --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with `--dry-run`; use `--out-dir` for a work-machine staging install.")
        .with_suggested_command("powerbi-cli source-template apply --project <project-dir-or.pbip> --handle <source-template-handle> --server <server> --database <database> --dry-run --json")
    })?;

    let source_store = load_source_template_store(&source_resolved)?;
    let selector = ShowOptions {
        project: None,
        handle: options.handle.clone(),
        name: options.name.clone(),
    };
    let record = find_template_by_show_options(&source_store.templates, &selector)?.clone();
    if template_has_errors(&record) {
        return Err(CliError::invalid_args(
            "source-template apply refuses an unsafe or credential-bearing template",
        )
        .with_hint("Remove credentials from the source-template store; credentials belong only in Power BI Desktop.")
        .with_suggested_command(format!(
            "powerbi-cli source-template show --project {} --handle {} --json",
            command_arg(&source_resolved.project_dir),
            shell_arg(&record.handle)
        )));
    }
    let (mut m_source, parameters) = materialize_template(&record, &options)?;

    let source_docs = load_table_documents(&source_resolved)?;
    let partition_selector = PartitionSelector {
        handle: Some(record.partition_handle.clone()),
        table: None,
        name: None,
    };
    let source_partition = find_partition(&source_docs, &partition_selector)?;
    if record.kind == "excel"
        && let Some(table) = source_docs
            .iter()
            .find(|table| same_name(&table.table, &record.table))
    {
        m_source = add_excel_column_types(&m_source, &table.columns);
    }
    let replacing_dummy =
        source_partition.source_kind == "dummyMTable" && source_partition.safety.status == "safe";
    let replacement_mode = if replacing_dummy {
        "generated-dummy"
    } else {
        let handle = source_partition.handle();
        if !options.replace_existing {
            return Err(CliError::invalid_args(format!(
                "source-template apply only replaces a safe generated dummy partition by default; {} is {} ({})",
                handle, source_partition.source_kind, source_partition.safety.status
            ))
            .with_hint("To intentionally retarget a known credential-free SQL, PostgreSQL, ODBC, or external-file partition, pass --replace-existing and --confirm with the exact partition handle. Unknown, web, and credential-bearing sources remain refused.")
            .with_suggested_command(format!(
                "powerbi-cli source-template apply --project {} --handle {} --replace-existing --confirm {} --dry-run --json",
                command_arg(&source_resolved.project_dir),
                shell_arg(&record.handle),
                shell_arg(&handle)
            )));
        }
        if options.confirm.as_deref() != Some(handle.as_str()) {
            return Err(CliError::invalid_args(format!(
                "source-template apply --replace-existing requires --confirm {handle}"
            ))
            .with_hint("Use the exact partition handle returned by `model partitions show`; this confirmation prevents accidental connection replacement.")
            .with_suggested_command(format!(
                "powerbi-cli source-template apply --project {} --handle {} --replace-existing --confirm {} --dry-run --json",
                command_arg(&source_resolved.project_dir),
                shell_arg(&record.handle),
                shell_arg(&handle)
            )));
        }
        if !matches!(
            source_partition.source_kind.as_str(),
            "sqlDatabase" | "postgresqlDatabase" | "odbcDataSource" | "externalFile"
        ) {
            return Err(CliError::invalid_args(format!(
                "source-template apply refuses confirmed replacement of source kind {}",
                source_partition.source_kind
            ))
            .with_hint("Only recognized credential-free SQL, PostgreSQL, ODBC, and external-file sources can be retargeted. Rebuild unknown or web sources from a reviewed source package."));
        }
        if source_partition
            .safety
            .findings
            .iter()
            .any(|finding| finding.code == "partition.credential_like_text")
        {
            return Err(CliError::invalid_args(
                "source-template apply refuses to replace a credential-bearing existing partition",
            )
            .with_hint("Remove credentials from the source text first; Power BI Desktop must own authentication."));
        }
        "confirmed-existing"
    };

    let target_resolved = match mode {
        MutationMode::DryRun | MutationMode::InPlace => source_resolved,
        MutationMode::OutDir(out_dir) => {
            copy_project_dir(&source_resolved.project_dir, out_dir)?;
            resolve_project(out_dir)?
        }
    };
    let docs = load_table_documents(&target_resolved)?;
    let plan = replace_partition_source_plan(&docs, &partition_selector, &m_source)?;
    let dry_run = matches!(mode, MutationMode::DryRun);
    let (validation, project_modified) = if dry_run {
        (None, false)
    } else {
        let (validation, project_modified) = write_text_atomic_validated(
            &plan.path,
            &plan.new_text,
            || validate_project(&target_resolved),
            |report| report.errors.is_empty(),
        )?;
        (Some(validation), project_modified)
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
        "powerbi-cli model partitions show --project {} --handle {} --json",
        project_arg,
        shell_arg(&plan.handle)
    );
    let validate = format!("powerbi-cli validate --strict {} --json", project_arg);
    let requires_desktop_authentication = record.kind != "excel";
    let instructions = if record.kind == "excel" {
        vec![
            "Ensure the configured Excel workbook exists at the materialized path.",
            "Open the PBIP in Power BI Desktop and refresh the semantic model.",
            "If the project moves, reapply the Excel source template with the workbook's new absolute path.",
        ]
    } else {
        vec![
            "Open the PBIP in Power BI Desktop on the work machine.",
            "When prompted, choose the appropriate database authentication method and enter credentials in Power BI Desktop.",
            "Refresh the semantic model. Credentials are not stored in the PBIP project.",
        ]
    };

    Ok(json!({
        "schema": "powerbi-cli.source-template.apply.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "apply",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "replacementMode": replacement_mode,
        "projectModified": project_modified,
        "credentialsEmbedded": false,
        "requiresDesktopAuthentication": requires_desktop_authentication,
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "target": {
            "templateHandle": record.handle,
            "partitionHandle": plan.handle,
            "table": plan.table,
            "partition": plan.name,
            "path": canonical_display(&plan.path)
        },
        "connection": {
            "kind": record.kind,
            "parameters": parameters
        },
        "changes": [{
            "kind": "tmdl.partition.source",
            "action": if replacing_dummy { "replace-dummy-with-source" } else { "replace-confirmed-existing-source" },
            "path": canonical_display(&plan.path),
            "beforeSourceKind": source_partition.source_kind,
            "afterSource": m_source
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
                "visuals": report.visuals
            }
        })),
        "rollback": (!dry_run && !validation_ok).then(|| json!({
            "performed": true,
            "projectModified": false,
            "reason": "post-mutation validation failed; the original TMDL file was restored"
        })),
        "readbackCommand": readback,
        "validateCommand": validate,
        "instructions": instructions,
        "next": [readback, validate]
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
            "--table" => options.table = Some(take_value(args, &mut i, "--table")?),
            "--kind" => {
                options.kind = Some(normalize_kind_arg(&take_value(args, &mut i, "--kind")?))
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown source-template list flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli source-template list --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli source-template list --project <project-dir-or.pbip> --json",
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
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown source-template show flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli source-template show --project <project-dir-or.pbip> --handle <source-template-handle> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli source-template show --project <project-dir-or.pbip> --handle <source-template-handle> --json",
                ));
            }
        }
    }
    if options.handle.is_none() && options.name.is_none() {
        return Err(
            CliError::invalid_args("source-template show requires --handle or --name")
                .with_hint("Use `source-template list` to get stable source-template handles.")
                .with_suggested_command(
                    "powerbi-cli source-template list --project <project-dir-or.pbip> --json",
                ),
        );
    }
    Ok(options)
}

fn parse_add_args(args: &[String]) -> CliResult<AddOptions> {
    let mut options = AddOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => {
                options.selector.handle = Some(take_value(args, &mut i, "--handle")?);
            }
            "--table" => {
                options.selector.table = Some(take_value(args, &mut i, "--table")?);
            }
            "--partition" | "--partition-name" => {
                let value = take_value(args, &mut i, "--partition")?;
                if value.starts_with("partition:") {
                    options.selector.handle = Some(value);
                } else {
                    options.selector.name = Some(value);
                }
            }
            "--name" => options.template_name = Some(take_value(args, &mut i, "--name")?),
            "--kind" => options.kind = Some(take_value(args, &mut i, "--kind")?),
            "--server" | "--server-placeholder" => {
                options.server = Some(take_value(args, &mut i, "--server")?);
            }
            "--dsn" | "--dsn-placeholder" => {
                options.dsn = Some(take_value(args, &mut i, "--dsn")?);
            }
            "--database" | "--database-placeholder" => {
                options.database = Some(take_value(args, &mut i, "--database")?);
            }
            "--schema" | "--sql-schema" => {
                options.sql_schema = Some(take_value(args, &mut i, "--schema")?);
            }
            "--object" => options.object = Some(take_value(args, &mut i, "--object")?),
            "--file" | "--path" => options.file = Some(take_value(args, &mut i, "--file")?),
            "--item" | "--sheet" => options.item = Some(take_value(args, &mut i, "--item")?),
            "--item-kind" => options.item_kind = Some(take_value(args, &mut i, "--item-kind")?),
            "--description" => {
                options.description = Some(take_value(args, &mut i, "--description")?)
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir(out_dir))?;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown source-template add flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for source-template` for exact flags.",
                )
                .with_suggested_command("powerbi-cli --json capabilities --for source-template"));
            }
        }
    }
    if options.selector.handle.is_none() {
        if options.selector.table.is_none() {
            return Err(CliError::invalid_args(
                "source-template add requires --handle or --table",
            )
            .with_hint("For scaffolded projects, partition names usually match table names.")
            .with_suggested_command(
                "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json",
            ));
        }
        if options.selector.name.is_none() {
            options.selector.name = options.selector.table.clone();
        }
    }
    if options.kind.is_none() {
        return Err(CliError::invalid_args("source-template add requires --kind")
            .with_hint("Supported kinds are `sql`, `postgres`, `odbc`, and `excel`; CSV and generic M templates are planned.")
            .with_suggested_command(
                "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json",
            ));
    }
    let _ = partition_selector_parts(&options.selector)?;
    Ok(options)
}

fn parse_apply_args(args: &[String]) -> CliResult<ApplyOptions> {
    let mut options = ApplyOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--server" => options.server = Some(take_value(args, &mut i, "--server")?),
            "--dsn" => options.dsn = Some(take_value(args, &mut i, "--dsn")?),
            "--database" => options.database = Some(take_value(args, &mut i, "--database")?),
            "--schema" | "--sql-schema" => {
                options.sql_schema = Some(take_value(args, &mut i, "--schema")?)
            }
            "--object" => options.object = Some(take_value(args, &mut i, "--object")?),
            "--file" | "--path" => options.file = Some(take_value(args, &mut i, "--file")?),
            "--item" | "--sheet" => options.item = Some(take_value(args, &mut i, "--item")?),
            "--item-kind" => options.item_kind = Some(take_value(args, &mut i, "--item-kind")?),
            "--replace-existing" => {
                options.replace_existing = true;
                i += 1;
            }
            "--confirm" => options.confirm = Some(take_value(args, &mut i, "--confirm")?),
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir(out_dir))?;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown source-template apply flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for source-template` for exact flags.",
                )
                .with_suggested_command("powerbi-cli --json capabilities --for source-template"));
            }
        }
    }
    if options.handle.is_none() && options.name.is_none() {
        return Err(
            CliError::invalid_args("source-template apply requires --handle or --name")
                .with_hint("Use `source-template list` to get stable source-template handles.")
                .with_suggested_command(
                    "powerbi-cli source-template list --project <project-dir-or.pbip> --json",
                ),
        );
    }
    Ok(options)
}

fn materialize_template(
    record: &SourceTemplateRecord,
    options: &ApplyOptions,
) -> CliResult<(String, BTreeMap<String, String>)> {
    let kind = normalize_kind(Some(&record.kind))?;
    let mut parameters = BTreeMap::new();
    let source = match kind.as_str() {
        "sql" | "postgres" => {
            reject_apply_flag_for_kind(&options.dsn, "--dsn", &kind, "--server")?;
            reject_apply_flag_for_kind(&options.file, "--file", &kind, "--server")?;
            reject_apply_flag_for_kind(&options.item, "--item", &kind, "--object")?;
            reject_apply_flag_for_kind(&options.item_kind, "--item-kind", &kind, "--schema")?;
            let server = concrete_parameter(record, "server", options.server.as_deref())?;
            let database = concrete_parameter(record, "database", options.database.as_deref())?;
            let schema = concrete_parameter(record, "schema", options.sql_schema.as_deref())?;
            let object = concrete_parameter(record, "object", options.object.as_deref())?;
            validate_template_parameters(&[
                ("server", &server),
                ("database", &database),
                ("schema", &schema),
                ("object", &object),
            ])?;
            parameters.insert("server".to_string(), server.clone());
            parameters.insert("database".to_string(), database.clone());
            parameters.insert("schema".to_string(), schema.clone());
            parameters.insert("object".to_string(), object.clone());
            if kind == "sql" {
                sql_source_template(SqlSourceTemplateInput {
                    table: record.table.clone(),
                    partition: record.partition.clone(),
                    name: record.name.clone(),
                    server,
                    database,
                    schema,
                    object,
                    description: record.description.clone(),
                })
                .m_template
            } else {
                postgres_source_template(PostgresSourceTemplateInput {
                    table: record.table.clone(),
                    partition: record.partition.clone(),
                    name: record.name.clone(),
                    server,
                    database,
                    schema,
                    object,
                    description: record.description.clone(),
                })
                .m_template
            }
        }
        "odbc" => {
            reject_apply_flag_for_kind(&options.server, "--server", "odbc", "--dsn")?;
            reject_apply_flag_for_kind(&options.file, "--file", "odbc", "--dsn")?;
            reject_apply_flag_for_kind(&options.item, "--item", "odbc", "--object")?;
            reject_apply_flag_for_kind(&options.item_kind, "--item-kind", "odbc", "--schema")?;
            let dsn = concrete_parameter(record, "dsn", options.dsn.as_deref())?;
            let database = concrete_parameter(record, "database", options.database.as_deref())?;
            let schema = concrete_parameter(record, "schema", options.sql_schema.as_deref())?;
            let object = concrete_parameter(record, "object", options.object.as_deref())?;
            validate_template_parameters(&[
                ("dsn", &dsn),
                ("database", &database),
                ("schema", &schema),
                ("object", &object),
            ])?;
            validate_bare_odbc_dsn(&dsn)?;
            parameters.insert("dsn".to_string(), dsn.clone());
            parameters.insert("database".to_string(), database.clone());
            parameters.insert("schema".to_string(), schema.clone());
            parameters.insert("object".to_string(), object.clone());
            odbc_source_template(OdbcSourceTemplateInput {
                table: record.table.clone(),
                partition: record.partition.clone(),
                name: record.name.clone(),
                dsn,
                database,
                schema,
                object,
                description: record.description.clone(),
            })
            .m_template
        }
        "excel" => {
            reject_apply_flag_for_kind(&options.server, "--server", "excel", "--file")?;
            reject_apply_flag_for_kind(&options.dsn, "--dsn", "excel", "--file")?;
            reject_apply_flag_for_kind(&options.database, "--database", "excel", "--file")?;
            reject_apply_flag_for_kind(&options.sql_schema, "--schema", "excel", "--item-kind")?;
            reject_apply_flag_for_kind(&options.object, "--object", "excel", "--item")?;
            let file = concrete_parameter(record, "file", options.file.as_deref())?;
            let item = concrete_parameter(record, "item", options.item.as_deref())?;
            let item_kind = normalize_excel_item_kind(
                options
                    .item_kind
                    .as_deref()
                    .or_else(|| record.parameters.get("itemKind").map(String::as_str)),
            )?;
            validate_template_parameters(&[
                ("file", &file),
                ("item", &item),
                ("itemKind", &item_kind),
            ])?;
            validate_excel_file(&file)?;
            parameters.insert("file".to_string(), file.clone());
            parameters.insert("item".to_string(), item.clone());
            parameters.insert("itemKind".to_string(), item_kind.clone());
            excel_source_template(ExcelSourceTemplateInput {
                table: record.table.clone(),
                partition: record.partition.clone(),
                name: record.name.clone(),
                file,
                item,
                item_kind,
                description: record.description.clone(),
            })
            .m_template
        }
        _ => return Err(unsupported_kind_error(&kind)),
    };
    Ok((source, parameters))
}

fn concrete_parameter(
    record: &SourceTemplateRecord,
    name: &str,
    override_value: Option<&str>,
) -> CliResult<String> {
    let value = override_value
        .map(ToOwned::to_owned)
        .or_else(|| record.parameters.get(name).cloned())
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "source template {} is missing parameter {name}",
                record.handle
            ))
        })?;
    if value.contains('<') && value.contains('>') {
        return Err(CliError::invalid_args(format!(
            "source-template apply requires a concrete --{name} value; the template still contains {value}"
        ))
        .with_hint("Pass only non-secret source identifiers. Power BI Desktop will request credentials separately.")
        .with_suggested_command(format!(
            "powerbi-cli source-template apply --project <project-dir-or.pbip> --handle {} --server <server> --database <database> --dry-run --json",
            shell_arg(&record.handle)
        )));
    }
    Ok(value)
}

fn add_excel_column_types(source: &str, columns: &[ColumnRecord]) -> String {
    let transformations = columns
        .iter()
        .filter(|column| !column.is_calculated())
        .filter_map(|column| {
            let data_type = excel_m_type(column.data_type.as_deref()?)?;
            let source_column = column.source_column.as_deref().unwrap_or(&column.name);
            Some(format!(
                "{{\"{}\", {data_type}}}",
                source_column.replace('"', "\"\"")
            ))
        })
        .collect::<Vec<_>>();
    if transformations.is_empty() {
        return source.to_string();
    }

    let marker = "\nin\n    PromotedHeaders";
    let replacement = format!(
        ",\n    TypedColumns = Table.TransformColumnTypes(PromotedHeaders, {{{}}}, \"en-US\")\nin\n    TypedColumns",
        transformations.join(", ")
    );
    source.replacen(marker, &replacement, 1)
}

fn excel_m_type(data_type: &str) -> Option<&'static str> {
    match data_type.trim().to_ascii_lowercase().as_str() {
        "int64" => Some("Int64.Type"),
        "double" => Some("type number"),
        "decimal" => Some("Decimal.Type"),
        "currency" => Some("Currency.Type"),
        "datetime" => Some("type datetime"),
        "datetimezone" => Some("type datetimezone"),
        "date" => Some("type date"),
        "time" => Some("type time"),
        "boolean" => Some("type logical"),
        "string" => Some("type text"),
        "binary" => Some("type binary"),
        _ => None,
    }
}

fn reject_apply_flag_for_kind(
    value: &Option<String>,
    flag: &str,
    kind: &str,
    replacement: &str,
) -> CliResult<()> {
    if value.is_none() {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} is not valid when applying source-template kind {kind}"
    ))
    .with_hint(format!(
        "Use {replacement} with source-template kind {kind}."
    )))
}

fn find_template_by_show_options<'a>(
    templates: &'a [SourceTemplateRecord],
    options: &ShowOptions,
) -> CliResult<&'a SourceTemplateRecord> {
    let matches = templates
        .iter()
        .filter(|template| {
            options
                .handle
                .as_ref()
                .is_some_and(|handle| handle == &template.handle)
                || options.name.as_ref().is_some_and(|name| {
                    template
                        .name
                        .as_deref()
                        .is_some_and(|template_name| same_name(name, template_name))
                        || same_name(name, &template.handle)
                })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [record] => Ok(record),
        [] => Err(CliError::validation_failed("source template not found")
            .with_hint("Run `source-template list` to get valid handles.")
            .with_suggested_command(
                "powerbi-cli source-template list --project <project-dir-or.pbip> --json",
            )),
        _ => Err(
            CliError::validation_failed("source template selector is ambiguous").with_hint(
                "Use the exact source-template handle returned by `source-template list`.",
            ),
        ),
    }
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

fn validate_template_value(label: &str, value: &str) -> CliResult<()> {
    if value.trim().is_empty() {
        return Err(CliError::invalid_args(format!(
            "source-template {label} must not be empty"
        ))
        .with_hint("Use a placeholder such as `<server>` if the real value is only known at work.")
        .with_suggested_command(
            "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json",
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(CliError::invalid_args(format!(
            "source-template {label} must be a single line"
        ))
        .with_hint("Use placeholders, not multiline M, for typed SQL templates.")
        .with_suggested_command(
            "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json",
        ));
    }
    let probe = SourceTemplateRecord {
        handle: "source-template:probe:probe".to_string(),
        name: None,
        partition_handle: "partition:probe:probe".to_string(),
        table: "probe".to_string(),
        partition: "probe".to_string(),
        kind: "sql".to_string(),
        parameters: [(label.to_string(), value.to_string())]
            .into_iter()
            .collect(),
        m_template: String::new(),
        description: None,
        requirements: Vec::new(),
    };
    if source_template_findings(&probe)
        .iter()
        .any(|finding| finding.code == "sourceTemplate.credential_like_text")
    {
        return Err(CliError::invalid_args(format!(
            "source-template {label} contains credential-like text"
        ))
        .with_hint("Do not store passwords, tokens, secrets, or credential strings in a home/offline project.")
        .with_suggested_command(
            "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --server <server> --database <database> --dry-run --json",
        ));
    }
    Ok(())
}

fn validate_template_parameters(parameters: &[(&str, &str)]) -> CliResult<()> {
    for (label, value) in parameters {
        validate_template_value(label, value)?;
    }
    Ok(())
}

fn validate_bare_odbc_dsn(dsn: &str) -> CliResult<()> {
    if !dsn.contains(';') && !dsn.contains('=') {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "source-template --dsn must be a bare ODBC DSN name without ';' or '=' attributes",
    )
    .with_hint(
        "Configure credentials in the ODBC manager or Power BI Desktop credential UI on the work machine; do not embed connection attributes in --dsn.",
    )
    .with_suggested_command(
        "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind odbc --dsn <dsn-name> --dry-run --json",
    ))
}

fn validate_excel_file(file: &str) -> CliResult<()> {
    let normalized = file.to_ascii_lowercase();
    if [".xlsx", ".xlsm", ".xlsb", ".xls"]
        .iter()
        .any(|extension| normalized.ends_with(extension))
    {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "source-template --file must name an Excel workbook (.xlsx, .xlsm, .xlsb, or .xls)",
    )
    .with_hint("Use a workbook path only. CSV sources remain a separate planned template kind.")
    .with_suggested_command(
        "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind excel --file <workbook.xlsx> --sheet <sheet> --dry-run --json",
    ))
}

fn normalize_excel_item_kind(value: Option<&str>) -> CliResult<String> {
    match value.unwrap_or("Sheet").trim().to_ascii_lowercase().as_str() {
        "sheet" | "worksheet" => Ok("Sheet".to_string()),
        "table" => Ok("Table".to_string()),
        other => Err(CliError::invalid_args(format!(
            "source-template --item-kind must be Sheet or Table; got {other}"
        ))
        .with_hint("Use `--sheet <name>` for a worksheet, or `--item <name> --item-kind Table` for an Excel table.")
        .with_suggested_command(
            "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind excel --file <workbook.xlsx> --sheet <sheet> --dry-run --json",
        )),
    }
}

fn reject_flag_for_kind(
    value: &Option<String>,
    flag: &str,
    kind: &str,
    replacement: &str,
) -> CliResult<()> {
    if value.is_some() {
        return Err(CliError::invalid_args(format!(
            "{flag} is not valid with source-template kind {kind}"
        ))
        .with_hint(format!("Use {replacement} with `--kind {kind}`."))
        .with_suggested_command(format!(
            "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind {kind} {replacement} <placeholder> --dry-run --json"
        )));
    }
    Ok(())
}

fn unsupported_kind_error(kind: &str) -> CliError {
    CliError::invalid_args(format!(
        "source-template add supports kinds sql, postgres, odbc, and excel; got {kind}"
    ))
    .with_hint(
        "Use `--kind sql`, `--kind postgres`, `--kind odbc`, or `--kind excel`; CSV and generic M templates are planned.",
    )
    .with_suggested_command(
        "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind <sql|postgres|odbc|excel> --dry-run --json",
    )
}

fn normalize_action(value: &str) -> String {
    match value {
        "ls" => "list",
        "get" => "show",
        "create" => "add",
        other => other,
    }
    .to_string()
}

fn normalize_kind(value: Option<&str>) -> CliResult<String> {
    let Some(value) = value else {
        return Err(CliError::invalid_args("source-template kind is required")
            .with_hint("Use `--kind sql` for the first supported source-template kind.")
            .with_suggested_command(
                "powerbi-cli source-template add --project <project-dir-or.pbip> --table <table> --kind sql --dry-run --json",
            ));
    };
    Ok(normalize_kind_arg(value))
}

fn normalize_kind_arg(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "sql" | "sql-server" | "sqlserver" => "sql".to_string(),
        "postgres" | "postgresql" => "postgres".to_string(),
        "excel" | "xlsx" | "xls" => "excel".to_string(),
        "generic-m" | "genericm" | "m" => "generic-m".to_string(),
        other => other.to_string(),
    }
}

fn set_mode(current: &mut Option<MutationMode>, next: MutationMode) -> CliResult<()> {
    if current.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one output mode: --dry-run, --in-place, or --out-dir <dir>",
        )
        .with_hint("Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.")
        .with_suggested_command("powerbi-cli --json capabilities --for source-template"));
    }
    *current = Some(next);
    Ok(())
}

fn mode_name(mode: &MutationMode) -> &'static str {
    match mode {
        MutationMode::DryRun => "dry-run",
        MutationMode::InPlace => "in-place",
        MutationMode::OutDir(_) => "out-dir",
    }
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint(
                "Run `powerbi-cli --json capabilities --for source-template` for exact usage.",
            )
            .with_suggested_command("powerbi-cli --json capabilities --for source-template")
    })?;
    *index += 2;
    Ok(value.clone())
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
