use crate::cli_support::{
    MutationMode, mode_name, require_report_interaction_mode as require_mode, required_project,
    set_report_interaction_mode as set_mode, shell_arg,
    take_report_interaction_value as take_value, target_project,
};
use crate::feature_catalog::unsupported_feature_error;
use crate::pbir::{PageRecord, PageSelector, VisualRecord, find_page, load_report_snapshot};
use crate::pbir_interactions::{
    interaction_matches_handle, interaction_record_from_raw, interaction_record_json,
    interaction_semantics, list_report_interactions,
};
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct MutateOptions {
    project: Option<PathBuf>,
    handle: Option<String>,
    page: Option<String>,
    source: Option<String>,
    target: Option<String>,
    interaction_type: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

struct ResolvedInteractionMutation {
    page: PageRecord,
    page_path: PathBuf,
    source: VisualRecord,
    target: VisualRecord,
    interaction_type: String,
}

struct InteractionPageMutation {
    page_json: Value,
    before: Value,
    after: Value,
    ordinal: usize,
    existed: bool,
    changed: bool,
}

pub(crate) fn set_interaction(args: &[String]) -> CliResult<Value> {
    let mut options = parse_mutate_args(args, "report interactions set")?;
    if options.interaction_type.is_none() {
        return Err(CliError::invalid_args(
            "report interactions set requires --type DataFilter|HighlightFilter|NoFilter",
        )
        .with_hint("Use `disable` as a shortcut for setting NoFilter.")
        .with_suggested_command(
            "powerbi-cli report interactions set --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --type NoFilter --dry-run --json",
        ));
    }
    if options.interaction_type.as_deref() == Some("Default") {
        return Err(unsupported_feature_error(
            "report.interaction-default-reset",
        ));
    }
    crate::cli_support::preflight_out_dir(args, set_interaction)?;
    mutate_interaction("set", &mut options)
}

pub(crate) fn disable_interaction(args: &[String]) -> CliResult<Value> {
    let mut options = parse_mutate_args(args, "report interactions disable")?;
    if let Some(value) = options.interaction_type.as_deref()
        && value != "NoFilter"
    {
        if value == "Default" {
            return Err(unsupported_feature_error(
                "report.interaction-default-reset",
            ));
        }
        return Err(CliError::invalid_args(
            "report interactions disable only supports --type NoFilter",
        )
        .with_hint("Use `report interactions set` for Default, DataFilter, or HighlightFilter.")
        .with_suggested_command(
            "powerbi-cli report interactions set --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --type DataFilter --dry-run --json",
        ));
    }
    options.interaction_type = Some("NoFilter".to_string());
    crate::cli_support::preflight_out_dir(args, disable_interaction)?;
    mutate_interaction("disable", &mut options)
}

fn mutate_interaction(action: &'static str, options: &mut MutateOptions) -> CliResult<Value> {
    require_mutation_selector(options, action)?;
    let mode = require_mode(options.mode, &format!("report interactions {action}"))?;
    let source_project = required_project(
        options.project.clone(),
        &format!("report interactions {action}"),
    )?;
    let source_resolved = resolve_project(&source_project)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let resolved = resolve_mutation_target(&target_resolved, options, action)?;
    let mutation = upsert_page_interaction(
        &resolved.page_path,
        &resolved.source.name,
        &resolved.target.name,
        &resolved.interaction_type,
    )?;
    if !matches!(mode, MutationMode::DryRun) && mutation.changed {
        write_json_atomic(&resolved.page_path, &mutation.page_json)?;
    }
    let after_record = interaction_record_from_raw(
        &resolved.page,
        &resolved.page_path,
        mutation.ordinal,
        &mutation.after,
    );
    interaction_mutation_response(
        &target_resolved,
        mode,
        action,
        &resolved,
        &mutation,
        interaction_record_json(&after_record, true),
    )
}

fn parse_mutate_args(args: &[String], command: &str) -> CliResult<MutateOptions> {
    let mut options = MutateOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--handle" => options.handle = Some(take_value(args, &mut i, "--handle")?),
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--source" => options.source = Some(take_value(args, &mut i, "--source")?),
            "--target" => options.target = Some(take_value(args, &mut i, "--target")?),
            "--type" | "--interaction-type" | "--interactionType" => {
                options.interaction_type = Some(parse_interaction_type(&take_value(
                    args, &mut i, "--type",
                )?)?);
            }
            "--dry-run" => {
                set_mode(&mut options.mode, MutationMode::DryRun, command)?;
                i += 1;
            }
            "--in-place" => {
                set_mode(&mut options.mode, MutationMode::InPlace, command)?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(&mut options.mode, MutationMode::OutDir, command)?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown {command} flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report interactions\"` for exact usage.",
                )
                .with_suggested_command("powerbi-cli --json capabilities --for \"report interactions\""));
            }
        }
    }
    Ok(options)
}

fn parse_interaction_type(value: &str) -> CliResult<String> {
    let normalized = match value.to_ascii_lowercase().as_str() {
        "default" => "Default",
        "datafilter" | "data-filter" | "filter" => "DataFilter",
        "highlightfilter" | "highlight-filter" | "highlight" => "HighlightFilter",
        "nofilter" | "no-filter" | "none" | "disabled" => "NoFilter",
        other => {
            return Err(CliError::invalid_args(format!(
                "invalid report interaction type: {other}"
            ))
            .with_hint("Use Default, DataFilter, HighlightFilter, or NoFilter.")
            .with_suggested_command(
                "powerbi-cli report interactions list --project <project-dir-or.pbip> --type NoFilter --json",
            ));
        }
    };
    Ok(normalized.to_string())
}

fn resolve_mutation_target(
    resolved: &ResolvedProject,
    options: &MutateOptions,
    action: &str,
) -> CliResult<ResolvedInteractionMutation> {
    let snapshot = load_report_snapshot(resolved)?;
    let interaction_type = options
        .interaction_type
        .clone()
        .ok_or_else(|| CliError::unexpected("interaction type was not resolved"))?;
    let (page_selector, source_selector, target_selector) = if let Some(handle) =
        options.handle.as_deref()
    {
        let (records, _) = list_report_interactions(resolved)?;
        let matches = records
            .iter()
            .filter(|record| interaction_matches_handle(record, handle))
            .collect::<Vec<_>>();
        let record = match matches.as_slice() {
            [record] => *record,
            [] => {
                return Err(CliError::invalid_args("interaction not found")
                    .with_hint("Use `report interactions list` to get stable interaction handles.")
                    .with_suggested_command(
                        "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
                    ));
            }
            _ => {
                return Err(CliError::invalid_args(
                    "interaction handle matched multiple interactions",
                )
                .with_hint("Run `report interactions list` and use the exact handle.")
                .with_suggested_command(
                    "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
                ));
            }
        };
        let source = record.source_visual.as_ref().ok_or_else(|| {
                CliError::invalid_args("cannot mutate interaction with missing source visual")
                    .with_hint("Repair the stale interaction manually or use live page/source/target selectors.")
                    .with_suggested_command(
                        "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
                    )
            })?;
        let target = record.target_visual.as_ref().ok_or_else(|| {
                CliError::invalid_args("cannot mutate interaction with missing target visual")
                    .with_hint("Repair the stale interaction manually or use live page/source/target selectors.")
                    .with_suggested_command(
                        "powerbi-cli report interactions list --project <project-dir-or.pbip> --json",
                    )
            })?;
        (
            selector_from_page_value(&record.page_handle),
            source.handle.clone(),
            target.handle.clone(),
        )
    } else {
        let page = options
            .page
            .as_deref()
            .ok_or_else(|| missing_mutation_selector(action))?;
        let source = options
            .source
            .as_deref()
            .ok_or_else(|| missing_mutation_selector(action))?;
        let target = options
            .target
            .as_deref()
            .ok_or_else(|| missing_mutation_selector(action))?;
        (
            selector_from_page_value(page),
            source.to_string(),
            target.to_string(),
        )
    };
    let page = find_page(
        &snapshot.pages,
        &page_selector,
        &format!("report interactions {action}"),
    )?
    .clone();
    let source = resolve_visual_on_page(&page, &source_selector, "--source", action)?;
    let target = resolve_visual_on_page(&page, &target_selector, "--target", action)?;
    if source.name == target.name {
        return Err(CliError::invalid_args(
            "report interactions require distinct source and target visuals",
        )
        .with_hint("Choose two different visual handles from `report visuals list`.")
        .with_suggested_command(
            "powerbi-cli report visuals list --project <project-dir-or.pbip> --page <page-handle> --json",
        ));
    }
    let page_path = page_path(&page)?;
    ensure_page_json_path(resolved, &page, &page_path)?;
    Ok(ResolvedInteractionMutation {
        page,
        page_path,
        source,
        target,
        interaction_type,
    })
}

fn upsert_page_interaction(
    page_path: &Path,
    source_name: &str,
    target_name: &str,
    interaction_type: &str,
) -> CliResult<InteractionPageMutation> {
    let mut page_json = read_json_value(page_path)?;
    let page_object = page_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} is not a JSON object", page_path.display()))
    })?;
    let interactions = page_object
        .entry("visualInteractions".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let interactions = interactions.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "{} visualInteractions is not an array",
            page_path.display()
        ))
    })?;
    let matches = interactions
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item["source"].as_str() == Some(source_name)
                && item["target"].as_str() == Some(target_name)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(CliError::validation_failed(format!(
            "{} contains duplicate visualInteractions for source {source_name} and target {target_name}",
            page_path.display()
        ))
        .with_hint("Open the page JSON or Power BI Desktop to remove duplicate interaction rows first.")
        .with_suggested_command(
            "powerbi-cli report interactions list --project <project-dir-or.pbip> --include-raw --json",
        ));
    }
    let new_row = json!({
        "source": source_name,
        "target": target_name,
        "type": interaction_type
    });
    let (ordinal, before, existed) = if let Some(index) = matches.first().copied() {
        let before = interactions[index].clone();
        let object = interactions[index].as_object_mut().ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} visualInteractions[{index}] is not a JSON object",
                page_path.display()
            ))
        })?;
        object.insert("source".to_string(), Value::String(source_name.to_string()));
        object.insert("target".to_string(), Value::String(target_name.to_string()));
        object.insert(
            "type".to_string(),
            Value::String(interaction_type.to_string()),
        );
        (index, before, true)
    } else {
        interactions.push(new_row);
        (interactions.len() - 1, Value::Null, false)
    };
    let after = interactions[ordinal].clone();
    Ok(InteractionPageMutation {
        page_json,
        changed: before != after,
        before,
        after,
        ordinal,
        existed,
    })
}

fn interaction_mutation_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    action: &str,
    resolved: &ResolvedInteractionMutation,
    mutation: &InteractionPageMutation,
    target: Value,
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
        "powerbi-cli report interactions show --project {} --page {} --source {} --target {} --json",
        project_arg,
        shell_arg(&resolved.page.handle),
        shell_arg(&resolved.source.handle),
        shell_arg(&resolved.target.handle)
    );
    let page_readback = format!(
        "powerbi-cli report pages show --project {} --handle {} --json",
        project_arg,
        shell_arg(&resolved.page.handle)
    );
    let source_visual = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&resolved.source.handle)
    );
    let target_visual = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&resolved.target.handle)
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
        "schema": "powerbi-cli.report.interactions.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": action,
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": target,
        "interactionPlan": {
            "before": mutation.before,
            "after": mutation.after,
            "existed": mutation.existed,
            "changed": mutation.changed,
            "semantics": interaction_semantics()
        },
        "changes": [{
            "kind": "pbir.page.visualInteractions",
            "action": if mutation.existed { "update" } else { "insert" },
            "path": canonical_display(&resolved.page_path),
            "jsonPointer": format!("/visualInteractions/{}", mutation.ordinal),
            "before": mutation.before,
            "after": mutation.after
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
        "pageReadbackCommand": page_readback,
        "sourceVisualReadbackCommand": source_visual,
        "targetVisualReadbackCommand": target_visual,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, page_readback, source_visual, target_visual, wireframe, inspect, validate]
    }))
}

fn require_mutation_selector(options: &MutateOptions, action: &str) -> CliResult<()> {
    if options.handle.is_some()
        && (options.page.is_some() || options.source.is_some() || options.target.is_some())
    {
        return Err(CliError::invalid_args(
            "choose --handle or --page plus --source and --target, not both",
        )
        .with_hint("Use a handle for an existing explicit interaction, or endpoint selectors to create one.")
        .with_suggested_command(format!(
            "powerbi-cli report interactions {action} --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json"
        )));
    }
    if options.handle.is_some()
        || (options.page.is_some() && options.source.is_some() && options.target.is_some())
    {
        return Ok(());
    }
    Err(missing_mutation_selector(action))
}

fn missing_mutation_selector(action: &str) -> CliError {
    CliError::invalid_args(format!(
        "report interactions {action} requires --handle or --page plus --source and --target"
    ))
    .with_hint("Use `report interactions list` and `report visuals list` to get stable handles.")
    .with_suggested_command(format!(
        "powerbi-cli report interactions {action} --project <project-dir-or.pbip> --page <page-handle> --source <visual-handle> --target <visual-handle> --dry-run --json"
    ))
}

fn resolve_visual_on_page(
    page: &PageRecord,
    selector: &str,
    flag: &str,
    action: &str,
) -> CliResult<VisualRecord> {
    let matches = page
        .visuals
        .iter()
        .filter(|visual| visual_selector_matches(visual, selector))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [visual] => Ok((*visual).clone()),
        [] => Err(CliError::invalid_args(format!(
            "{flag} visual not found on page {}: {selector}",
            page.handle
        ))
        .with_hint("Use visual handles from `report visuals list` for the same page.")
        .with_suggested_command(format!(
            "powerbi-cli report interactions {action} --project <project-dir-or.pbip> --page {} --source <visual-handle> --target <visual-handle> --dry-run --json",
            shell_arg(&page.handle)
        ))),
        _ => Err(CliError::invalid_args(format!(
            "{flag} visual selector is ambiguous on page {}: {selector}",
            page.handle
        ))
        .with_hint("Use the exact visual handle instead of a title or display value.")
        .with_suggested_command("powerbi-cli report visuals list --project <project-dir-or.pbip> --json")),
    }
}

fn visual_selector_matches(visual: &VisualRecord, selector: &str) -> bool {
    visual.handle == selector
        || visual.name == selector
        || visual.title == selector
        || visual.name.eq_ignore_ascii_case(selector)
        || visual.title.eq_ignore_ascii_case(selector)
}

fn selector_from_page_value(value: &str) -> PageSelector {
    if value.starts_with("page:") {
        PageSelector {
            handle: Some(value.to_string()),
            name: None,
        }
    } else {
        PageSelector {
            handle: None,
            name: Some(value.to_string()),
        }
    }
}

fn page_path(page: &PageRecord) -> CliResult<PathBuf> {
    page.path.clone().ok_or_else(|| {
        CliError::validation_failed(format!(
            "page has no path in inspect output: {}",
            page.handle
        ))
        .with_hint("Run `validate --strict` before mutating this report.")
        .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json")
    })
}

fn ensure_page_json_path(
    resolved: &ResolvedProject,
    page: &PageRecord,
    page_path: &Path,
) -> CliResult<()> {
    if page_path.file_name().and_then(|value| value.to_str()) != Some("page.json") {
        return Err(CliError::validation_failed(format!(
            "refusing to write page interaction because path is not page.json: {}",
            page_path.display()
        )));
    }
    let expected_page_dir = resolved
        .report_dir
        .join("definition")
        .join("pages")
        .join(&page.name);
    let page_dir = page_path.parent().ok_or_else(|| {
        CliError::validation_failed(format!("page path has no parent: {}", page_path.display()))
    })?;
    let page_dir_abs = std::fs::canonicalize(page_dir)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", page_dir.display())))?;
    let expected_abs = std::fs::canonicalize(&expected_page_dir).map_err(|err| {
        CliError::unexpected(format!("resolve {}: {err}", expected_page_dir.display()))
    })?;
    if page_dir_abs == expected_abs {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to write interaction outside page directory: {}",
        page_path.display()
    ))
    .with_hint("Run `validate --strict` before mutating this report.")
    .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json"))
}
