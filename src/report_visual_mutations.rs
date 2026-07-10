use crate::cli_support::{
    MutationMode, mode_name, require_mode_with_contract, required_project, set_mode_with_contract,
    shell_arg, take_report_value as take_value, target_project,
};
use crate::pbir::{PageRecord, PageSelector, find_page, load_report_snapshot};
use crate::pbir_bindings::{
    VisualBindingInput, VisualBindingKind, VisualBindingResolved, binding_summary,
    parse_binding_spec, parse_bindings_json_file, parse_bindings_json_text,
    resolve_visual_bindings,
};
use crate::pbir_visual_factory::{
    SlicerMode, VisualBuildSpec, resolve_slicer_mode, visual_container_json,
};
use crate::project_io::write_json_atomic;
use crate::tmdl::load_table_documents;
use crate::visual_catalog::{VisualBindingFamily, binding_family, canonical_visual_type};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_WIDTH: f64 = 320.0;
const DEFAULT_HEIGHT: f64 = 180.0;
const ADD_DRY_RUN_COMMAND: &str = "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --dry-run --json";
const REQUIRE_MODE_HINT: &str =
    "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.";
const SET_MODE_HINT: &str =
    "Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.";

#[derive(Debug, Default)]
struct AddVisualOptions {
    project: Option<PathBuf>,
    page: Option<String>,
    name: Option<String>,
    title: Option<String>,
    visual_type: Option<String>,
    slicer_mode: Option<String>,
    bindings: Vec<VisualBindingInput>,
    bindings_file: Option<PathBuf>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    z: Option<u64>,
    tab_order: Option<u64>,
    allow_outside_page: bool,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

struct VisualMutationOutput {
    action: &'static str,
    target: Value,
    name_generated: bool,
    binding_plan: Value,
    changes: Vec<Value>,
    visual_handle: String,
}

pub(crate) fn add_visual(args: &[String]) -> CliResult<Value> {
    let options = parse_add_args(args)?;
    let source_project = required_project(options.project.clone(), "report visuals add")?;
    let page_selector = options.page.as_ref().ok_or_else(|| {
        CliError::invalid_args("report visuals add requires --page <page-name-or-handle>")
            .with_hint("Use `report pages list` to get stable page handles.")
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --dry-run --json",
            )
    })?;
    let page_selector = selector_from_page(page_selector);
    let title = options.title.as_deref().ok_or_else(|| {
        CliError::invalid_args("report visuals add requires --title")
            .with_hint("Agents should give every created visual a readable title.")
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --dry-run --json",
            )
    })?;
    validate_nonempty_text(title, "--title")?;
    let visual_type = options
        .visual_type
        .as_deref()
        .map(canonical_visual_type)
        .transpose()?
        .unwrap_or_else(|| "card".to_string());
    let slicer_mode = resolve_slicer_mode(&visual_type, options.slicer_mode.as_deref())?;
    let width = options.width.unwrap_or(DEFAULT_WIDTH);
    let height = options.height.unwrap_or(DEFAULT_HEIGHT);
    validate_positive_number(width, "--width")?;
    validate_positive_number(height, "--height")?;
    let mode = require_mode_with_contract(
        options.mode,
        "report visuals add",
        REQUIRE_MODE_HINT,
        ADD_DRY_RUN_COMMAND,
    )?;
    let source_resolved = resolve_project(&source_project)?;
    crate::cli_support::preflight_out_dir(args, add_visual)?;
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let page = find_page(&snapshot.pages, &page_selector, "report visuals add")?.clone();
    let visual_index = page.visuals.len();
    let x = options.x.unwrap_or(40.0 + (visual_index as f64 * 40.0));
    let y = options.y.unwrap_or(40.0 + (visual_index as f64 * 40.0));
    let z = options.z.unwrap_or(visual_index as u64);
    let tab_order = options.tab_order.unwrap_or(visual_index as u64);
    let position = json!({
        "x": x,
        "y": y,
        "z": z,
        "height": height,
        "width": width,
        "tabOrder": tab_order
    });
    validate_position_bounds(
        &position,
        page.width.as_f64(),
        page.height.as_f64(),
        options.allow_outside_page,
    )?;
    let visual_name = match options.name.as_deref() {
        Some(name) => validate_new_visual_name(name, &page)?,
        None => generated_visual_name(title, &page),
    };
    let name_generated = options.name.is_none();
    let bindings = if options.bindings.is_empty() {
        Vec::new()
    } else {
        let docs = load_table_documents(&target_resolved)?;
        resolve_visual_bindings(&docs, &visual_type, &options.bindings)?
    };
    validate_binding_cardinality(&visual_type, &bindings)?;
    let build_spec = VisualBuildSpec {
        name: visual_name.clone(),
        title: title.to_string(),
        visual_type: visual_type.clone(),
        bindings: bindings.clone(),
        slicer_mode,
        x,
        y,
        z,
        width,
        height,
        tab_order,
    };
    let visual_json = visual_container_json(&build_spec);
    let visual_dir = page_visuals_dir(&page)?.join(&visual_name);
    let visual_path = visual_dir.join("visual.json");
    ensure_child_path(&visual_dir, &page_visuals_dir(&page)?)?;
    let target = visual_target_summary(&page, &build_spec, &visual_path, position.clone());
    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        fs::create_dir_all(&visual_dir).map_err(|err| {
            CliError::unexpected(format!("create visual dir {}: {err}", visual_dir.display()))
        })?;
        write_json_file(&visual_path, &visual_json)?;
    }
    let binding_plan_after = bindings.iter().map(binding_summary).collect::<Vec<_>>();

    mutation_response(
        &target_resolved,
        mode,
        VisualMutationOutput {
            action: "add",
            target,
            name_generated,
            binding_plan: json!({
                "clear": false,
                "bindingsFile": options.bindings_file.as_ref().map(|path| canonical_display(path)),
                "before": [],
                "after": binding_plan_after
            }),
            changes: vec![json!({
                "kind": "pbir.visual",
                "action": "add",
                "path": canonical_display(&visual_path),
                "before": Value::Null,
                "after": visual_json
            })],
            visual_handle: format!("visual:{}:{}", page.name, visual_name),
        },
    )
}

fn parse_add_args(args: &[String]) -> CliResult<AddVisualOptions> {
    let mut options = AddVisualOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--title" => options.title = Some(take_value(args, &mut i, "--title")?),
            "--visual-type" | "--visualType" | "--type" | "--chart" | "--chart-type" => {
                options.visual_type = Some(take_value(args, &mut i, "--visual-type")?);
            }
            "--mode" => {
                options.slicer_mode = Some(take_value(args, &mut i, "--mode")?);
            }
            "--binding" | "--bind" => {
                let value = take_value(args, &mut i, "--binding")?;
                options.bindings.push(parse_binding_spec(&value)?);
            }
            "--bindings-json" => {
                let value = take_value(args, &mut i, "--bindings-json")?;
                options.bindings.extend(parse_bindings_json_text(&value)?);
            }
            "--bindings-file" => {
                let path = PathBuf::from(take_value(args, &mut i, "--bindings-file")?);
                options.bindings.extend(parse_bindings_json_file(&path)?);
                options.bindings_file = Some(path);
            }
            "--x" => options.x = Some(take_f64(args, &mut i, "--x")?),
            "--y" => options.y = Some(take_f64(args, &mut i, "--y")?),
            "--width" => options.width = Some(take_f64(args, &mut i, "--width")?),
            "--height" => options.height = Some(take_f64(args, &mut i, "--height")?),
            "--z" => options.z = Some(take_u64(args, &mut i, "--z")?),
            "--tab-order" | "--tabOrder" => {
                options.tab_order = Some(take_u64(args, &mut i, "--tab-order")?);
            }
            "--allow-outside-page" => {
                options.allow_outside_page = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::DryRun,
                    SET_MODE_HINT,
                    ADD_DRY_RUN_COMMAND,
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::InPlace,
                    SET_MODE_HINT,
                    ADD_DRY_RUN_COMMAND,
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::OutDir,
                    SET_MODE_HINT,
                    ADD_DRY_RUN_COMMAND,
                )?;
                options.out_dir = Some(out_dir);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals add flag: {other}"
                ))
                .with_hint("Run `powerbi-cli --json capabilities --for \"report visuals add\"` for exact flags.")
                .with_suggested_command("powerbi-cli --json capabilities --for \"report visuals add\""));
            }
        }
    }
    Ok(options)
}

fn mutation_response(
    target_resolved: &ResolvedProject,
    mode: MutationMode,
    output: VisualMutationOutput,
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
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&output.visual_handle)
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
        "schema": "powerbi-cli.report.visuals.mutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": output.action,
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": output.target.clone(),
        "visualPlan": {
            "before": Value::Null,
            "after": output.target.clone(),
            "nameGenerated": output.name_generated,
            "visualType": output.target["visualType"]
        },
        "bindingPlan": output.binding_plan,
        "changes": output.changes,
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
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": [readback, wireframe, inspect, validate]
    }))
}

fn visual_target_summary(
    page: &PageRecord,
    spec: &VisualBuildSpec,
    visual_path: &Path,
    position: Value,
) -> Value {
    json!({
        "handle": format!("visual:{}:{}", page.name, spec.name),
        "name": spec.name,
        "title": spec.title,
        "visualType": spec.visual_type,
        "mode": spec.slicer_mode.map(SlicerMode::as_str),
        "page": {
            "handle": page.handle,
            "name": page.name,
            "displayName": page.display_name,
            "ordinal": page.ordinal
        },
        "path": canonical_display(visual_path),
        "position": position,
        "bindingCount": spec.bindings.len(),
        "bindings": spec.bindings.iter().map(binding_summary).collect::<Vec<_>>()
    })
}

pub(crate) fn validate_binding_cardinality(
    visual_type: &str,
    bindings: &[VisualBindingResolved],
) -> CliResult<()> {
    let family = binding_family(visual_type)?;
    if bindings.is_empty()
        && !matches!(
            family,
            VisualBindingFamily::CategoryShare
                | VisualBindingFamily::RowsColumnsValues
                | VisualBindingFamily::SlicerField
        )
    {
        return Ok(());
    }
    match family {
        VisualBindingFamily::SingleValue => {
            if bindings.len() == 1 && bindings[0].role == "Values" {
                return Ok(());
            }
            Err(CliError::invalid_args(
                "single-value visuals accept exactly one Values binding",
            )
            .with_hint("Use one measure or column in the Values role.")
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
            ))
        }
        VisualBindingFamily::ValuesList => {
            if bindings.iter().all(|binding| binding.role == "Values") {
                return Ok(());
            }
            Err(
                CliError::invalid_args("values-list visuals accept Values bindings only")
                    .with_hint("Use one or more column or measure bindings in the Values role."),
            )
        }
        VisualBindingFamily::CategoryY => {
            let category_count = bindings
                .iter()
                .filter(|binding| binding.role == "Category")
                .count();
            let series_count = bindings
                .iter()
                .filter(|binding| binding.role == "Series")
                .count();
            let has_y = bindings.iter().any(|binding| binding.role == "Y");
            let category_is_column = bindings.iter().all(|binding| {
                binding.role != "Category" || matches!(binding.kind, VisualBindingKind::Column)
            });
            let series_is_column = bindings.iter().all(|binding| {
                binding.role != "Series" || matches!(binding.kind, VisualBindingKind::Column)
            });
            let only_supported_roles = bindings
                .iter()
                .all(|binding| matches!(binding.role.as_str(), "Category" | "Y" | "Series"));
            if category_count >= 1
                && category_is_column
                && has_y
                && series_count <= 1
                && series_is_column
                && only_supported_roles
            {
                return Ok(());
            }
            Err(CliError::invalid_args(
                "chart visuals require one or more Category column bindings, at least one Y binding, and at most one Series column binding",
            )
            .with_hint("Use Category for axis columns, Y for one or more values, and optional Series for a legend column. Multiple Category bindings become a drill hierarchy.")
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type lineChart --title <title> --binding \"role=Category,table=<table>,column=<year-column>\" --binding \"role=Category,table=<table>,column=<month-column>\" --binding \"role=Y,table=<table>,measure=<measure>\" --dry-run --json",
            ))
        }
        VisualBindingFamily::CategoryShare => {
            let category_count = bindings
                .iter()
                .filter(|binding| binding.role == "Category")
                .count();
            let y_count = bindings
                .iter()
                .filter(|binding| binding.role == "Y")
                .count();
            let category_is_column = bindings.iter().all(|binding| {
                binding.role != "Category" || matches!(binding.kind, VisualBindingKind::Column)
            });
            if category_count == 1 && y_count >= 1 && category_is_column {
                return Ok(());
            }
            Err(CliError::invalid_args(format!(
                "{visual_type} requires exactly one Category column binding and at least one Y binding; got {category_count} Category and {y_count} Y bindings"
            ))
            .with_hint(
                "Use one Category column and one or more Y measures or columns; pie and donut visuals do not accept Series.",
            )
            .with_suggested_command(format!(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {visual_type} --title <title> --binding \"role=Category,table=<table>,column=<column>\" --binding \"role=Y,table=<table>,measure=<measure>\" --dry-run --json"
            )))
        }
        VisualBindingFamily::RowsColumnsValues => {
            let rows_count = bindings
                .iter()
                .filter(|binding| binding.role == "Rows")
                .count();
            let columns_count = bindings
                .iter()
                .filter(|binding| binding.role == "Columns")
                .count();
            let values_count = bindings
                .iter()
                .filter(|binding| binding.role == "Values")
                .count();
            let hierarchy_fields_are_columns = bindings.iter().all(|binding| {
                !matches!(binding.role.as_str(), "Rows" | "Columns")
                    || matches!(binding.kind, VisualBindingKind::Column)
            });
            if rows_count >= 1 && values_count >= 1 && hierarchy_fields_are_columns {
                return Ok(());
            }
            Err(CliError::invalid_args(format!(
                "matrix (pivotTable) requires at least one Rows column binding and at least one Values binding; Columns are optional columns; got {rows_count} Rows, {columns_count} Columns, and {values_count} Values bindings"
            ))
            .with_hint(
                "Use Rows and optional Columns for hierarchy columns, and Values for one or more measures or columns.",
            )
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type matrix --title <title> --binding \"role=Rows,table=<table>,column=<row-column>\" --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
            ))
        }
        VisualBindingFamily::SlicerField => {
            let values_count = bindings
                .iter()
                .filter(|binding| binding.role == "Values")
                .count();
            let value_is_column = bindings.iter().all(|binding| {
                binding.role != "Values" || matches!(binding.kind, VisualBindingKind::Column)
            });
            if values_count == 1 && value_is_column {
                return Ok(());
            }
            Err(CliError::invalid_args(format!(
                "slicer requires exactly one Values column binding; got {values_count} Values bindings{}",
                if value_is_column { "" } else { ", including a measure" }
            ))
            .with_hint("Bind one model column to Values; slicer measures are unsupported.")
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type slicer --mode basic --title <title> --binding \"role=Values,table=<table>,column=<column>\" --dry-run --json",
            ))
        }
        VisualBindingFamily::ScatterBubble => {
            let count_role = |role: &str| {
                bindings
                    .iter()
                    .filter(|binding| binding.role == role)
                    .count()
            };
            let x_count = count_role("X");
            let y_count = count_role("Y");
            let category_count = count_role("Category");
            let size_count = count_role("Size");
            let legend_count = count_role("Legend");
            let only_supported_roles = bindings.iter().all(|binding| {
                matches!(
                    binding.role.as_str(),
                    "Category" | "X" | "Y" | "Size" | "Legend" | "Tooltips"
                )
            });
            let category_is_column = bindings.iter().all(|binding| {
                binding.role != "Category" || matches!(binding.kind, VisualBindingKind::Column)
            });
            let legend_is_column = bindings.iter().all(|binding| {
                binding.role != "Legend" || matches!(binding.kind, VisualBindingKind::Column)
            });
            if x_count == 1
                && y_count == 1
                && category_count <= 1
                && size_count <= 1
                && legend_count <= 1
                && category_is_column
                && legend_is_column
                && only_supported_roles
            {
                return Ok(());
            }
            Err(CliError::invalid_args(
                "scatter/bubble visuals require exactly one X binding and exactly one Y binding; Category, Size, Legend, and Tooltips are optional",
            )
            .with_hint("Use X and Y for numeric axes, optional Size for bubble size, optional Category for bubble identity, optional Legend for color grouping, and Tooltips for extra fields.")
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type scatterChart --title <title> --binding \"role=Category,table=<table>,column=<detail-column>\" --binding \"role=X,table=<table>,measure=<x-measure>\" --binding \"role=Y,table=<table>,measure=<y-measure>\" --binding \"role=Size,table=<table>,measure=<size-measure>\" --dry-run --json",
            ))
        }
    }
}

fn validate_new_visual_name(name: &str, page: &PageRecord) -> CliResult<String> {
    validate_visual_name(name)?;
    if page.visuals.iter().any(|visual| visual.name == name) {
        return Err(CliError::invalid_args(format!(
            "visual already exists on page {}: {name}",
            page.handle
        ))
        .with_hint("Choose a unique internal --name or omit it so powerbi-cli can generate one.")
        .with_suggested_command(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --dry-run --json",
        ));
    }
    Ok(name.to_string())
}

fn validate_visual_name(name: &str) -> CliResult<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch == ':' || ch.is_control())
    {
        return Err(CliError::invalid_args(format!("unsafe visual name: {name}"))
            .with_hint("Use a simple internal visual name without path separators.")
            .with_suggested_command(
                "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --dry-run --json",
            ));
    }
    Ok(())
}

fn generated_visual_name(title: &str, page: &PageRecord) -> String {
    let stem = pascal_identifier(title).unwrap_or_else(|| "Visual".to_string());
    let base = format!("VisualContainer{stem}");
    if !page.visuals.iter().any(|visual| visual.name == base) {
        return base;
    }
    for index in 2..1000 {
        let candidate = format!("{base}{index}");
        if !page.visuals.iter().any(|visual| visual.name == candidate) {
            return candidate;
        }
    }
    format!("VisualContainer{}", page.visuals.len() + 1)
}

fn pascal_identifier(value: &str) -> Option<String> {
    let mut output = String::new();
    for part in value.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if part.is_empty() {
            continue;
        }
        let mut chars = part.chars();
        let first = chars.next()?.to_ascii_uppercase();
        output.push(first);
        output.extend(chars.map(|ch| ch.to_ascii_lowercase()));
    }
    (!output.is_empty()).then_some(output)
}

fn page_visuals_dir(page: &PageRecord) -> CliResult<PathBuf> {
    let page_json = page.path.as_ref().ok_or_else(|| {
        CliError::validation_failed(format!(
            "page has no path in inspect output: {}",
            page.handle
        ))
    })?;
    let page_dir = page_json.parent().ok_or_else(|| {
        CliError::validation_failed(format!("page path has no parent: {}", page_json.display()))
    })?;
    Ok(page_dir.join("visuals"))
}

fn validate_position_bounds(
    position: &Value,
    page_width: Option<f64>,
    page_height: Option<f64>,
    allow_outside_page: bool,
) -> CliResult<()> {
    let x = position["x"].as_f64().unwrap_or_default();
    let y = position["y"].as_f64().unwrap_or_default();
    let width = position["width"].as_f64().unwrap_or_default();
    let height = position["height"].as_f64().unwrap_or_default();
    if x < 0.0 || y < 0.0 {
        return Err(CliError::invalid_args(
            "visual position x/y must be nonnegative",
        )
        .with_hint("Use --allow-outside-page only for page overflow, not negative coordinates.")
        .with_suggested_command(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --x 0 --y 0 --dry-run --json",
        ));
    }
    if width <= 0.0 || height <= 0.0 {
        return Err(CliError::invalid_args(
            "visual position width/height must be positive",
        )
        .with_hint("Pass positive --width and --height values.")
        .with_suggested_command(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --width 320 --height 180 --dry-run --json",
        ));
    }
    if !allow_outside_page
        && let (Some(page_width), Some(page_height)) = (page_width, page_height)
        && (x + width > page_width || y + height > page_height)
    {
        return Err(CliError::invalid_args(
            "visual position would extend outside page bounds",
        )
        .with_hint("Keep the visual inside the page or pass --allow-outside-page deliberately.")
        .with_suggested_command(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type card --title <title> --allow-outside-page --dry-run --json",
        ));
    }
    Ok(())
}

fn selector_from_page(page: &str) -> PageSelector {
    if page.starts_with("page:") {
        PageSelector {
            handle: Some(page.to_string()),
            name: None,
        }
    } else {
        PageSelector {
            handle: None,
            name: Some(page.to_string()),
        }
    }
}

fn ensure_child_path(path: &Path, parent: &Path) -> CliResult<()> {
    let parent_abs = if parent.exists() {
        fs::canonicalize(parent)
            .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", parent.display())))?
    } else {
        parent.to_path_buf()
    };
    let path_abs = if path.exists() {
        fs::canonicalize(path)
            .map_err(|err| CliError::unexpected(format!("resolve {}: {err}", path.display())))?
    } else {
        parent_abs.join(path.file_name().unwrap_or(path.as_os_str()))
    };
    if path_abs.starts_with(parent_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to write visual outside page visuals directory: {}",
        path.display()
    )))
}

fn write_json_file(path: &Path, value: &Value) -> CliResult<()> {
    if path.exists() {
        return write_json_atomic(path, value);
    }
    let parent = path
        .parent()
        .ok_or_else(|| CliError::unexpected(format!("path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    let text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    fs::write(path, text)
        .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))?;
    Ok(())
}

fn validate_nonempty_text(value: &str, flag: &str) -> CliResult<()> {
    if !value.trim().is_empty() && !value.chars().any(char::is_control) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be nonempty text"
    )))
}

fn validate_positive_number(value: f64, flag: &str) -> CliResult<()> {
    if value.is_finite() && value > 0.0 {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be a positive finite number"
    )))
}

fn take_f64(args: &[String], index: &mut usize, flag: &str) -> CliResult<f64> {
    let value = take_value(args, index, flag)?;
    let parsed = value
        .parse::<f64>()
        .map_err(|_| CliError::invalid_args(format!("{flag} must be a number")))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(CliError::invalid_args(format!(
            "{flag} must be a finite number"
        )))
    }
}

fn take_u64(args: &[String], index: &mut usize, flag: &str) -> CliResult<u64> {
    let value = take_value(args, index, flag)?;
    value
        .parse::<u64>()
        .map_err(|_| CliError::invalid_args(format!("{flag} must be a nonnegative integer")))
}
