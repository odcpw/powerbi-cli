use crate::cli_support::{
    MutationMode, mode_name, require_mode, required_project, set_mode, shell_arg, take_value,
    target_project,
};
use crate::pbir::{VisualSelector, find_page, find_visual, load_report_snapshot};
use crate::pbir_filters::{
    FilterArrayOrigin, FilterScope, filter_fingerprint, filter_target, named_filter_handle,
};
use crate::project_io::write_json_atomic;
use crate::report_filter_shapes::{
    FilterSpec, RelativeDateOperator, RelativeDateUnit, ResolvedFilterColumn, TopNDirection,
    generated_filter_name, parse_field_reference, parse_numeric_json, parse_values_json,
    resolve_filter_column, resolve_filter_measure, validate_filter_name,
};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, read_json_value, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct AddFilterOptions {
    project: Option<PathBuf>,
    scope: Option<FilterScope>,
    page: Option<String>,
    visual: Option<String>,
    target: Option<String>,
    table: Option<String>,
    column: Option<String>,
    name: Option<String>,
    display_name: Option<String>,
    values: Vec<Value>,
    condition_type: Option<String>,
    min: Option<Value>,
    max: Option<Value>,
    top: Option<u64>,
    bottom: Option<u64>,
    by: Option<String>,
    relative: Option<RelativeDateOperator>,
    unit: Option<RelativeDateUnit>,
    span: Option<u64>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
    include_raw: bool,
}

#[derive(Debug, Clone)]
struct ResolvedFilterOwner {
    scope: FilterScope,
    path: PathBuf,
    stable_id: String,
    owner: Value,
    readback_command: String,
    owner_readback_command: String,
}

struct FilterAddPlan {
    file_json: Value,
    filter: Value,
    json_pointer: String,
    before_count: usize,
    after_count: usize,
    handle: String,
}

pub(crate) fn add_filter(args: &[String]) -> CliResult<Value> {
    let options = parse_add_args(args)?;
    let source_project = required_project(options.project.clone(), "report filters add")?;
    let mode = require_mode(options.mode, "report filters add")?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, add_filter)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let owner = resolve_filter_owner(&target_resolved, &options)?;
    ensure_filter_owner_path(&target_resolved, &owner.path)?;
    let column = resolve_requested_column(&target_resolved, &options)?;
    let spec = resolve_filter_spec(&target_resolved, &options)?;
    spec.validate_for(&column, owner.scope)?;
    let name = options
        .name
        .clone()
        .unwrap_or_else(|| generated_filter_name(owner.scope, &column, &spec));
    validate_filter_name(&name)?;
    let filter = spec.to_pbir(&name, options.display_name.as_deref(), &column)?;
    let plan = add_filter_to_file(&owner, filter)?;

    if !matches!(mode, MutationMode::DryRun) {
        write_json_atomic(&owner.path, &plan.file_json)?;
    }

    add_filter_response(
        &target_resolved,
        mode,
        &owner,
        &column,
        &spec,
        &plan,
        options.include_raw,
    )
}

fn parse_add_args(args: &[String]) -> CliResult<AddFilterOptions> {
    let mut options = AddFilterOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--scope" => {
                options.scope = Some(parse_add_scope(&take_value(args, &mut i, "--scope")?)?)
            }
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--visual" => options.visual = Some(take_value(args, &mut i, "--visual")?),
            "--target" => options.target = Some(take_value(args, &mut i, "--target")?),
            "--table" => options.table = Some(take_value(args, &mut i, "--table")?),
            "--column" => options.column = Some(take_value(args, &mut i, "--column")?),
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--display-name" | "--displayName" => {
                options.display_name = Some(take_value(args, &mut i, "--display-name")?);
            }
            "--condition-type" | "--conditionType" => {
                set_once(
                    &mut options.condition_type,
                    take_value(args, &mut i, "--condition-type")?,
                    "--condition-type",
                )?;
            }
            "--value" => {
                options
                    .values
                    .push(Value::String(take_value(args, &mut i, "--value")?));
            }
            "--value-json" | "--valueJson" => {
                let text = take_value(args, &mut i, "--value-json")?;
                let value = serde_json::from_str(&text).map_err(|err| {
                    CliError::invalid_args(format!("parse --value-json: {err}"))
                        .with_hint("Pass one JSON literal, for example `--value-json 2026` or `--value-json '\"North\"'`.")
                        .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --scope report --table <table> --column <column> --value <text> --dry-run --json")
                })?;
                options.values.push(value);
            }
            "--values-json" | "--valuesJson" => {
                let text = take_value(args, &mut i, "--values-json")?;
                let values = parse_values_json(&text)?;
                options.values.extend(values);
            }
            "--min" => {
                let value = parse_numeric_json(&take_value(args, &mut i, "--min")?, "--min")?;
                set_once(&mut options.min, value, "--min")?;
            }
            "--max" => {
                let value = parse_numeric_json(&take_value(args, &mut i, "--max")?, "--max")?;
                set_once(&mut options.max, value, "--max")?;
            }
            "--top" => {
                let value = take_positive_u64(args, &mut i, "--top")?;
                set_once(&mut options.top, value, "--top")?;
            }
            "--bottom" => {
                let value = take_positive_u64(args, &mut i, "--bottom")?;
                set_once(&mut options.bottom, value, "--bottom")?;
            }
            "--by" => {
                set_once(&mut options.by, take_value(args, &mut i, "--by")?, "--by")?;
            }
            "--relative" => {
                let value = RelativeDateOperator::parse(&take_value(args, &mut i, "--relative")?)?;
                set_once(&mut options.relative, value, "--relative")?;
            }
            "--unit" => {
                let value = RelativeDateUnit::parse(&take_value(args, &mut i, "--unit")?)?;
                set_once(&mut options.unit, value, "--unit")?;
            }
            "--span" => {
                let value = take_positive_u64(args, &mut i, "--span")?;
                set_once(&mut options.span, value, "--span")?;
            }
            "--include-raw" | "--includeRaw" => {
                options.include_raw = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "report filters add",
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode(
                    &mut options.mode,
                    MutationMode::InPlace,
                    "report filters add",
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "report filters add",
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report filters add flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli --json capabilities --for \"report filters add\"` for exact flags.",
                )
                .with_suggested_command(
                    "powerbi-cli --json capabilities --for \"report filters add\"",
                ));
            }
        }
    }
    Ok(options)
}

fn parse_add_scope(value: &str) -> CliResult<FilterScope> {
    match value {
        "report" => Ok(FilterScope::Report),
        "page" => Ok(FilterScope::Page),
        "visual" => Ok(FilterScope::Visual),
        "all" => Err(CliError::invalid_args(
            "report filters add cannot use --scope all",
        )
        .with_hint("Choose exactly one owner: report, page, or visual.")
        .with_suggested_command(
            "powerbi-cli report filters add --project <project-dir-or.pbip> --scope report --table <table> --column <column> --value <text> --dry-run --json",
        )),
        other => Err(CliError::invalid_args(format!(
            "invalid report filters add scope: {other}"
        ))
        .with_hint("Use --scope report, --scope page, or --scope visual.")
        .with_suggested_command(
            "powerbi-cli report filters add --project <project-dir-or.pbip> --scope report --table <table> --column <column> --value <text> --dry-run --json",
        )),
    }
}

fn resolve_filter_owner(
    resolved: &ResolvedProject,
    options: &AddFilterOptions,
) -> CliResult<ResolvedFilterOwner> {
    let scope = inferred_scope(options)?;
    let project = command_arg(&resolved.project_dir);
    match scope {
        FilterScope::Report => {
            if options.page.is_some() || options.visual.is_some() {
                return Err(CliError::invalid_args(
                    "report filters add --scope report cannot be combined with --page or --visual",
                )
                .with_hint("Report filters live in report.json and have no page or visual owner.")
                .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --scope report --table <table> --column <column> --value <text> --dry-run --json"));
            }
            let path = resolved.report_dir.join("definition").join("report.json");
            Ok(ResolvedFilterOwner {
                scope,
                path: path.clone(),
                stable_id: "report:main".to_string(),
                owner: json!({
                    "kind": "report",
                    "handle": "report:main",
                    "name": "report",
                    "displayName": "Report",
                    "path": canonical_display(&path)
                }),
                readback_command: format!(
                    "powerbi-cli report filters list --project {project} --scope report --json"
                ),
                owner_readback_command: format!(
                    "powerbi-cli report wireframe export {project} --json"
                ),
            })
        }
        FilterScope::Page => {
            let page = options.page.as_ref().ok_or_else(|| {
                CliError::invalid_args("report filters add --scope page requires --page")
                    .with_hint("Use `report pages list` to get stable page handles.")
                    .with_suggested_command(
                        "powerbi-cli report pages list --project <project-dir-or.pbip> --json",
                    )
            })?;
            if options.visual.is_some() {
                return Err(CliError::invalid_args(
                    "report filters add --scope page cannot be combined with --visual",
                )
                .with_hint("Use --scope visual for visual-owned filters.")
                .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --scope visual --visual <visual-handle> --table <table> --column <column> --value <text> --dry-run --json"));
            }
            let snapshot = load_report_snapshot(resolved)?;
            let page_record = find_page(
                &snapshot.pages,
                &crate::pbir::PageSelector {
                    handle: page.starts_with("page:").then(|| page.clone()),
                    name: (!page.starts_with("page:")).then(|| page.clone()),
                },
                "report filters add",
            )?;
            let path = page_record.path.clone().ok_or_else(|| {
                CliError::validation_failed(format!(
                    "page {} has no page.json path",
                    page_record.handle
                ))
            })?;
            Ok(ResolvedFilterOwner {
                scope,
                path: path.clone(),
                stable_id: page_record.handle.clone(),
                owner: json!({
                    "kind": "page",
                    "handle": page_record.handle,
                    "name": page_record.name,
                    "displayName": page_record.display_name,
                    "ordinal": page_record.ordinal,
                    "path": canonical_display(&path)
                }),
                readback_command: format!(
                    "powerbi-cli report filters list --project {project} --scope page --page {} --json",
                    shell_arg(&page_record.handle)
                ),
                owner_readback_command: format!(
                    "powerbi-cli report pages show --project {project} --handle {} --json",
                    shell_arg(&page_record.handle)
                ),
            })
        }
        FilterScope::Visual => {
            let visual = options.visual.as_ref().ok_or_else(|| {
                CliError::invalid_args("report filters add --scope visual requires --visual")
                    .with_hint("Use `report visuals list` to get stable visual handles.")
                    .with_suggested_command(
                        "powerbi-cli report visuals list --project <project-dir-or.pbip> --json",
                    )
            })?;
            let snapshot = load_report_snapshot(resolved)?;
            let visual_record = find_visual(
                &snapshot.pages,
                &VisualSelector {
                    handle: visual.starts_with("visual:").then(|| visual.clone()),
                    page: (!visual.starts_with("visual:"))
                        .then(|| options.page.clone())
                        .flatten(),
                    visual: (!visual.starts_with("visual:")).then(|| visual.clone()),
                },
                "report filters add",
            )?;
            let path = visual_record.path.clone().ok_or_else(|| {
                CliError::validation_failed(format!(
                    "visual {} has no visual.json path",
                    visual_record.handle
                ))
            })?;
            Ok(ResolvedFilterOwner {
                scope,
                path: path.clone(),
                stable_id: visual_record.handle.clone(),
                owner: json!({
                    "kind": "visual",
                    "handle": visual_record.handle,
                    "name": visual_record.name,
                    "title": visual_record.title,
                    "visualType": visual_record.visual_type,
                    "path": canonical_display(&path),
                    "page": {
                        "handle": visual_record.page_handle,
                        "name": visual_record.page_name,
                        "displayName": visual_record.page_display_name,
                        "ordinal": visual_record.page_ordinal
                    }
                }),
                readback_command: format!(
                    "powerbi-cli report filters list --project {project} --scope visual --visual {} --json",
                    shell_arg(&visual_record.handle)
                ),
                owner_readback_command: format!(
                    "powerbi-cli report visuals show --project {project} --handle {} --json",
                    shell_arg(&visual_record.handle)
                ),
            })
        }
        FilterScope::All => unreachable!("add scope parser never returns all"),
    }
}

fn inferred_scope(options: &AddFilterOptions) -> CliResult<FilterScope> {
    if let Some(scope) = options.scope {
        return Ok(scope);
    }
    if options.visual.is_some() {
        return Ok(FilterScope::Visual);
    }
    if options.page.is_some() {
        return Ok(FilterScope::Page);
    }
    Ok(FilterScope::Report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedFilterKind {
    Categorical,
    NumericRange,
    TopN,
    RelativeDate,
}

fn resolve_requested_column(
    resolved: &ResolvedProject,
    options: &AddFilterOptions,
) -> CliResult<ResolvedFilterColumn> {
    let (table, column) = requested_target(options)?;
    resolve_filter_column(resolved, &table, &column)
}

fn requested_target(options: &AddFilterOptions) -> CliResult<(String, String)> {
    if let Some(target) = options.target.as_deref() {
        if options.table.is_some() || options.column.is_some() {
            return Err(CliError::invalid_args(
                "report filters add accepts either --target or --table plus --column, not both",
            )
            .with_hint("Use --target 'Table[Column]' for compact input, or use --table and --column separately.")
            .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'DimCustomer[Segment]' --value Enterprise --dry-run --json"));
        }
        return parse_field_reference(target);
    }
    let table = options.table.clone().ok_or_else(|| {
        CliError::invalid_args("report filters add requires --target or --table plus --column")
            .with_hint("Use TMDL table and column names from `inspect --deep`.")
            .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'DimCustomer[Segment]' --value Enterprise --dry-run --json")
    })?;
    let column = options.column.clone().ok_or_else(|| {
        CliError::invalid_args("report filters add requires --column when --table is used")
            .with_hint("Use TMDL column names from `inspect --deep`.")
            .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --table DimCustomer --column Segment --value Enterprise --dry-run --json")
    })?;
    Ok((table, column))
}

fn resolve_filter_spec(
    resolved: &ResolvedProject,
    options: &AddFilterOptions,
) -> CliResult<FilterSpec> {
    let explicit_kind = options
        .condition_type
        .as_deref()
        .map(parse_condition_type)
        .transpose()?;
    let categorical_signal = !options.values.is_empty();
    let range_signal = options.min.is_some() || options.max.is_some();
    let topn_signal = options.top.is_some() || options.bottom.is_some() || options.by.is_some();
    let relative_signal =
        options.relative.is_some() || options.unit.is_some() || options.span.is_some();
    let inferred = [
        (categorical_signal, RequestedFilterKind::Categorical),
        (range_signal, RequestedFilterKind::NumericRange),
        (topn_signal, RequestedFilterKind::TopN),
        (relative_signal, RequestedFilterKind::RelativeDate),
    ]
    .into_iter()
    .filter_map(|(present, kind)| present.then_some(kind))
    .collect::<Vec<_>>();
    let kind = if let Some(kind) = explicit_kind {
        if inferred.iter().any(|inferred| *inferred != kind) {
            return Err(CliError::invalid_args(
                "filter kind flags cannot be mixed in one report filters add command",
            )
            .with_hint(
                "Choose categorical values, range bounds, TopN flags, or relative-date flags.",
            ));
        }
        kind
    } else {
        match inferred.as_slice() {
            [] => RequestedFilterKind::Categorical,
            [kind] => *kind,
            _ => {
                return Err(CliError::invalid_args(
                    "filter kind flags cannot be mixed in one report filters add command",
                )
                .with_hint(
                    "Choose categorical values, range bounds, TopN flags, or relative-date flags.",
                ));
            }
        }
    };

    match kind {
        RequestedFilterKind::Categorical => Ok(FilterSpec::Categorical {
            values: options.values.clone(),
        }),
        RequestedFilterKind::NumericRange => Ok(FilterSpec::NumericRange {
            min: options.min.clone(),
            max: options.max.clone(),
        }),
        RequestedFilterKind::TopN => {
            let (direction, count) = match (options.top, options.bottom) {
                (Some(count), None) => (TopNDirection::Top, count),
                (None, Some(count)) => (TopNDirection::Bottom, count),
                (Some(_), Some(_)) => {
                    return Err(CliError::invalid_args(
                        "choose exactly one of --top or --bottom",
                    ));
                }
                (None, None) => {
                    return Err(CliError::invalid_args(
                        "TopN filters require --top <N> or --bottom <N>",
                    ));
                }
            };
            let requested_measure = options.by.as_deref().ok_or_else(|| {
                CliError::invalid_args("TopN filters require --by <measure>")
                    .with_hint(
                        "Use a globally unique measure name, Table[Measure], or a measure handle.",
                    )
                    .with_suggested_command(
                        "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
                    )
            })?;
            Ok(FilterSpec::TopN {
                direction,
                count,
                by: resolve_filter_measure(resolved, requested_measure)?,
            })
        }
        RequestedFilterKind::RelativeDate => Ok(FilterSpec::RelativeDate {
            operator: options.relative.ok_or_else(|| {
                CliError::invalid_args("relative-date filters require --relative last|next|this")
            })?,
            unit: options.unit.ok_or_else(|| {
                CliError::invalid_args("relative-date filters require --unit <unit>")
            })?,
            span: options.span.ok_or_else(|| {
                CliError::invalid_args("relative-date filters require --span <N>")
            })?,
        }),
    }
}

fn parse_condition_type(value: &str) -> CliResult<RequestedFilterKind> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "categorical" => Ok(RequestedFilterKind::Categorical),
        "range" | "numeric-range" => Ok(RequestedFilterKind::NumericRange),
        "topn" | "top-n" => Ok(RequestedFilterKind::TopN),
        "relative-date" | "relativedate" => Ok(RequestedFilterKind::RelativeDate),
        other => Err(CliError::unsupported_feature(format!(
            "report filters add cannot emit condition type {other}"
        ))
        .with_hint("Supported condition types are categorical, range, topn, and relative-date. Other types are refused instead of emitting guessed PBIR.")),
    }
}

fn take_positive_u64(args: &[String], index: &mut usize, flag: &str) -> CliResult<u64> {
    let raw = take_value(args, index, flag)?;
    let value = raw
        .parse::<u64>()
        .map_err(|_| CliError::invalid_args(format!("{flag} must be a positive whole number")))?;
    if value == 0 || value > i64::MAX as u64 {
        return Err(CliError::invalid_args(format!(
            "{flag} must be between 1 and {}",
            i64::MAX
        )));
    }
    Ok(value)
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> CliResult<()> {
    if slot.is_some() {
        return Err(CliError::invalid_args(format!(
            "{flag} may be passed only once"
        )));
    }
    *slot = Some(value);
    Ok(())
}

fn add_filter_to_file(owner: &ResolvedFilterOwner, filter: Value) -> CliResult<FilterAddPlan> {
    let mut file_json = read_json_value(&owner.path)?;
    let root = file_json.as_object_mut().ok_or_else(|| {
        CliError::validation_failed(format!("{} is not a JSON object", owner.path.display()))
    })?;
    let filter_config = root
        .entry("filterConfig")
        .or_insert_with(|| json!({ "filters": [] }));
    if !filter_config.is_object() {
        return Err(CliError::validation_failed(format!(
            "{} filterConfig is not an object",
            owner.path.display()
        )));
    }
    let filter_config_object = filter_config.as_object_mut().expect("checked object");
    let filters = filter_config_object
        .entry("filters")
        .or_insert_with(|| Value::Array(Vec::new()));
    let filters = filters.as_array_mut().ok_or_else(|| {
        CliError::validation_failed(format!(
            "{} /filterConfig/filters is not an array",
            owner.path.display()
        ))
    })?;
    if let Some(name) = filter["name"].as_str()
        && filters.iter().any(|existing| {
            existing["name"]
                .as_str()
                .is_some_and(|existing_name| existing_name.eq_ignore_ascii_case(name))
        })
    {
        return Err(CliError::invalid_args(format!(
            "filter name already exists for this owner: {name}"
        ))
        .with_hint("Pass a unique --name or delete/update the existing filter explicitly.")
        .with_suggested_command(
            "powerbi-cli report filters list --project <project-dir-or.pbip> --json",
        ));
    }
    let before_count = filters.len();
    let handle = filter_handle(
        owner,
        filter["name"]
            .as_str()
            .expect("filter names are validated before PBIR generation"),
    );
    filters.push(filter.clone());
    let json_pointer = format!("/filterConfig/filters/{before_count}");
    Ok(FilterAddPlan {
        file_json,
        filter,
        json_pointer,
        before_count,
        after_count: before_count + 1,
        handle,
    })
}

fn add_filter_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    owner: &ResolvedFilterOwner,
    column: &ResolvedFilterColumn,
    spec: &FilterSpec,
    plan: &FilterAddPlan,
    include_raw: bool,
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
    let filter_show = (!dry_run).then(|| {
        format!(
            "powerbi-cli report filters show --project {project_arg} --handle {} --json",
            shell_arg(&plan.handle)
        )
    });
    let raw_review = dry_run.then(|| {
        format!(
            "powerbi-cli report filters add --project {project_arg} {} --target {} {} --dry-run --include-raw --json",
            owner_selector_args(owner),
            shell_arg(&format!("{}[{}]", column.table, column.column)),
            filter_spec_args(spec)
        )
    });
    let wireframe = format!("powerbi-cli report wireframe export {project_arg} --json");
    let inspect = format!("powerbi-cli inspect --deep {project_arg} --json");
    let validate = format!("powerbi-cli validate --strict {project_arg} --json");
    let filter_summary = filter_summary(owner, plan, spec, include_raw);
    let mut next = vec![
        owner.readback_command.clone(),
        owner.owner_readback_command.clone(),
        wireframe.clone(),
        inspect.clone(),
        validate.clone(),
    ];
    if let Some(command) = &filter_show {
        next.insert(1, command.clone());
    }

    Ok(json!({
        "schema": "powerbi-cli.report.filters.addMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": "add",
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": filter_summary,
        "owner": owner.owner,
        "filterPlan": {
            "filterKind": spec.kind_name(),
            "beforeCount": plan.before_count,
            "afterCount": plan.after_count,
            "jsonPointer": plan.json_pointer,
            "rawAfterIncluded": include_raw,
            "after": if include_raw { plan.filter.clone() } else { filter_summary.clone() }
        },
        "changes": [{
            "kind": "pbir.filter",
            "action": "add",
            "path": canonical_display(&owner.path),
            "jsonPointer": plan.json_pointer,
            "parentJsonPointer": "/filterConfig/filters",
            "ordinal": plan.before_count,
            "before": Value::Null,
            "after": if include_raw { plan.filter.clone() } else { filter_summary.clone() }
        }],
        "safety": {
            "dataValueRisk": "possible",
            "mayContainDataValues": true,
            "message": spec.safety_message()
        },
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
        "readbackCommand": owner.readback_command,
        "filterReadbackCommand": filter_show,
        "ownerReadbackCommand": owner.owner_readback_command,
        "rawReviewCommand": raw_review,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": next
    }))
}

fn filter_summary(
    owner: &ResolvedFilterOwner,
    plan: &FilterAddPlan,
    spec: &FilterSpec,
    include_raw: bool,
) -> Value {
    let mut value = json!({
        "handle": plan.handle,
        "handleIdentity": "name",
        "handleAmbiguous": false,
        "scope": owner.scope.as_str(),
        "ordinal": plan.before_count,
        "arrayOrigin": "filterConfig",
        "name": plan.filter["name"],
        "displayName": plan.filter["displayName"],
        "filterType": plan.filter["type"],
        "filterKind": spec.kind_name(),
        "unsupported": false,
        "target": filter_target(&plan.filter),
        "conditionSummary": condition_summary(&plan.filter),
        "isActive": true,
        "path": canonical_display(&owner.path),
        "jsonPointer": plan.json_pointer,
        "fingerprint": filter_fingerprint(&plan.filter),
        "owner": owner.owner,
        "safety": {
            "dataValueRisk": "possible",
            "mayContainDataValues": true,
            "literalCountInFilterDefinition": count_literals(&plan.filter["filter"]),
            "rawIncluded": include_raw,
            "findings": [{
                "code": "filter.possible_persisted_values",
                "severity": "warning",
                "message": "Power BI filter metadata can persist selected values; use dummy/offline-safe values outside the work environment."
            }]
        }
    });
    if include_raw {
        value["raw"] = plan.filter.clone();
    }
    value
}

fn condition_summary(filter: &Value) -> String {
    let target = filter_target(filter);
    let filter_type = filter["type"].as_str().unwrap_or("Unknown");
    match (target["table"].as_str(), target["column"].as_str()) {
        (Some(table), Some(column)) => {
            format!("{filter_type} filter on {table}[{column}] with persisted filter definition")
        }
        _ => format!("{filter_type} filter on unknown column with persisted filter definition"),
    }
}

fn filter_spec_args(spec: &FilterSpec) -> String {
    match spec {
        FilterSpec::Categorical { values } => format!(
            "--values-json {}",
            shell_arg(&serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string()))
        ),
        FilterSpec::NumericRange { min, max } => {
            let mut args = vec!["--condition-type range".to_string()];
            if let Some(min) = min {
                args.push(format!("--min {min}"));
            }
            if let Some(max) = max {
                args.push(format!("--max {max}"));
            }
            args.join(" ")
        }
        FilterSpec::TopN {
            direction,
            count,
            by,
        } => format!(
            "{} {count} --by {}",
            direction.flag(),
            shell_arg(&format!("{}[{}]", by.table, by.measure))
        ),
        FilterSpec::RelativeDate {
            operator,
            unit,
            span,
        } => format!(
            "--relative {} --unit {} --span {span}",
            operator.as_str(),
            unit.as_str()
        ),
    }
}

fn filter_handle(owner: &ResolvedFilterOwner, name: &str) -> String {
    let page_name = match owner.scope {
        FilterScope::Page => owner.owner["name"].as_str(),
        FilterScope::Visual => owner.owner["page"]["name"].as_str(),
        _ => None,
    };
    let visual_name = (owner.scope == FilterScope::Visual)
        .then(|| owner.owner["name"].as_str())
        .flatten();
    named_filter_handle(
        owner.scope,
        page_name,
        visual_name,
        name,
        FilterArrayOrigin::FilterConfig,
    )
}

fn count_literals(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Bool(_) | Value::Number(_) | Value::String(_) => 1,
        Value::Array(items) => items.iter().map(count_literals).sum(),
        Value::Object(object) => object.values().map(count_literals).sum(),
    }
}

fn owner_selector_args(owner: &ResolvedFilterOwner) -> String {
    match owner.scope {
        FilterScope::Report => "--scope report".to_string(),
        FilterScope::Page => format!("--page {}", shell_arg(&owner.stable_id)),
        FilterScope::Visual => format!("--visual {}", shell_arg(&owner.stable_id)),
        FilterScope::All => unreachable!("add scope cannot be all"),
    }
}

fn ensure_filter_owner_path(resolved: &ResolvedProject, path: &Path) -> CliResult<()> {
    let file_name = path.file_name().and_then(|value| value.to_str());
    if !matches!(file_name, Some("report.json" | "page.json" | "visual.json")) {
        return Err(CliError::validation_failed(format!(
            "refusing to add filter to unsupported file path: {}",
            path.display()
        )));
    }
    let report_abs = fs::canonicalize(&resolved.report_dir).map_err(|err| {
        CliError::unexpected(format!("resolve {}: {err}", resolved.report_dir.display()))
    })?;
    let path_abs = fs::canonicalize(path)
        .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?;
    if path_abs.starts_with(report_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to add filter outside report directory: {}",
        path.display()
    ))
    .with_hint("Run `validate --strict` before mutating this report.")
    .with_suggested_command("powerbi-cli validate --strict <project-dir-or.pbip> --json"))
}
