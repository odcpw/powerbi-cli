use crate::cli_support::{
    MutationMode, mode_name, require_mode_with_contract, required_project, set_mode_with_contract,
    shell_arg, take_report_value as take_value, target_project,
};
use crate::input_safety::{InputKind, read_utf8, read_utf8_stream};
use crate::pbir::{PageRecord, PageSelector, find_page, load_report_snapshot};
use crate::pbir_visual_factory::SLICER_MIN_HEIGHT;
use crate::project_io::write_json_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, ResolvedProject, canonical_display,
    command_arg, resolve_project, validate_project,
};
use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const VISUAL_CONTAINER_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json";
const REQUIRE_MODE_HINT: &str =
    "Start with `--dry-run`; use `--out-dir` or `--in-place` only after review.";
const SET_MODE_HINT: &str =
    "Start with `--dry-run`; rerun with `--in-place` or `--out-dir` after review.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScaffoldKind {
    Card,
    Slicer,
    Textbox,
}

impl ScaffoldKind {
    fn command(self) -> &'static str {
        match self {
            Self::Card => "report visuals add-card",
            Self::Slicer => "report visuals add-slicer",
            Self::Textbox => "report visuals add-textbox",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Card => "add-card",
            Self::Slicer => "add-slicer",
            Self::Textbox => "add-textbox",
        }
    }

    fn visual_type(self) -> &'static str {
        match self {
            Self::Card => "card",
            Self::Slicer => "slicer",
            Self::Textbox => "textbox",
        }
    }

    fn how_created(self) -> &'static str {
        match self {
            Self::Textbox => "InsertVisualButton",
            Self::Card | Self::Slicer => "DraggedToFieldWell",
        }
    }

    fn dry_run_command(self) -> &'static str {
        match self {
            Self::Card => {
                "powerbi-cli report visuals add-card --project <project-dir-or.pbip> --page <page-name-or-handle> --measure <Table.Measure> --title <text> --x <n> --y <n> --width <n> --height <n> --dry-run --json"
            }
            Self::Slicer => {
                "powerbi-cli report visuals add-slicer --project <project-dir-or.pbip> --page <page-name-or-handle> --field <Table.Column> --title <text> --x <n> --y <n> --width <n> --height <n> --dry-run --json"
            }
            Self::Textbox => {
                "powerbi-cli report visuals add-textbox --project <project-dir-or.pbip> --page <page-name-or-handle> --title <text> --text <paragraph> --x <n> --y <n> --width <n> --height <n> --dry-run --json"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlicerModeOpt {
    Basic,
    Dropdown,
}

impl SlicerModeOpt {
    fn as_str(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Dropdown => "Dropdown",
        }
    }
}

#[derive(Debug, Default)]
struct ScaffoldOptions {
    project: Option<PathBuf>,
    page: Option<String>,
    name: Option<String>,
    title: Option<String>,
    field: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    value_font_size: Option<f64>,
    category_font_size: Option<f64>,
    word_wrap: bool,
    slicer_mode: Option<SlicerModeOpt>,
    single_select: bool,
    paragraphs_file: Option<String>,
    text: Option<String>,
    mode: Option<MutationMode>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn add_card(args: &[String]) -> CliResult<Value> {
    scaffold_visual(ScaffoldKind::Card, args)
}

pub(crate) fn add_slicer(args: &[String]) -> CliResult<Value> {
    scaffold_visual(ScaffoldKind::Slicer, args)
}

pub(crate) fn add_textbox(args: &[String]) -> CliResult<Value> {
    scaffold_visual(ScaffoldKind::Textbox, args)
}

fn scaffold_visual(kind: ScaffoldKind, args: &[String]) -> CliResult<Value> {
    let options = parse_scaffold_args(kind, args)?;
    let source_project = required_project(options.project.clone(), kind.command())?;
    let page_arg = options.page.as_deref().ok_or_else(|| {
        CliError::invalid_args(format!(
            "{} requires --page <page-name-or-handle>",
            kind.command()
        ))
        .with_hint("Use `report pages list` to get stable page handles.")
        .with_suggested_command(kind.dry_run_command())
    })?;
    let title = options.title.as_deref().ok_or_else(|| {
        CliError::invalid_args(format!("{} requires --title", kind.command()))
            .with_hint("Give every created visual a readable title.")
            .with_suggested_command(kind.dry_run_command())
    })?;
    validate_nonempty_text(title, "--title")?;
    let x = require_geometry(options.x, "--x", kind)?;
    let y = require_geometry(options.y, "--y", kind)?;
    let width = require_positive_geometry(options.width, "--width", kind)?;
    let height = require_positive_geometry(options.height, "--height", kind)?;
    if x < 0.0 || y < 0.0 {
        return Err(
            CliError::invalid_args("visual position x/y must be nonnegative")
                .with_hint("Pass nonnegative --x and --y values.")
                .with_suggested_command(kind.dry_run_command()),
        );
    }
    let mode = require_mode_with_contract(
        options.mode,
        kind.command(),
        REQUIRE_MODE_HINT,
        kind.dry_run_command(),
    )?;
    let source_resolved = resolve_project(&source_project)?;
    match kind {
        ScaffoldKind::Card => crate::cli_support::preflight_out_dir(args, add_card)?,
        ScaffoldKind::Slicer => crate::cli_support::preflight_out_dir(args, add_slicer)?,
        ScaffoldKind::Textbox => crate::cli_support::preflight_out_dir(args, add_textbox)?,
    }
    let target_resolved = target_project(&source_resolved, mode, options.out_dir.as_deref())?;
    let snapshot = load_report_snapshot(&target_resolved)?;
    let page = find_page(
        &snapshot.pages,
        &selector_from_page(page_arg),
        kind.command(),
    )?
    .clone();
    let next_stack = next_stack_index(&page);
    let position = json!({
        "height": height,
        "tabOrder": next_stack,
        "width": width,
        "x": x,
        "y": y,
        "z": next_stack
    });
    validate_position_bounds(&position, page.width.as_f64(), page.height.as_f64(), kind)?;
    if kind == ScaffoldKind::Slicer && height < SLICER_MIN_HEIGHT {
        return Err(CliError::invalid_args(format!(
            "slicer height {height} is below the Power BI minimum of {SLICER_MIN_HEIGHT}"
        ))
        .with_hint(format!(
            "Increase --height to at least {SLICER_MIN_HEIGHT}."
        ))
        .with_suggested_command(kind.dry_run_command()));
    }
    let visual_name = match options.name.as_deref() {
        Some(name) => validate_new_visual_name(name, &page, kind)?,
        None => generated_visual_name(title, &page),
    };
    let name_generated = options.name.is_none();
    let visual_json = match kind {
        ScaffoldKind::Card => {
            let (table, measure) = parse_table_field(
                options.field.as_deref().ok_or_else(|| {
                    CliError::invalid_args(
                        "report visuals add-card requires --measure <Table.Measure>",
                    )
                    .with_hint("Pass a model measure as `<Table>.<Measure>`.")
                    .with_suggested_command(kind.dry_run_command())
                })?,
                "--measure",
            )?;
            card_visual_json(&CardSpec {
                name: &visual_name,
                title,
                table: &table,
                measure: &measure,
                position: &position,
                value_font_size: options.value_font_size,
                category_font_size: options.category_font_size,
                word_wrap: options.word_wrap,
            })
        }
        ScaffoldKind::Slicer => {
            let (table, column) = parse_table_field(
                options.field.as_deref().ok_or_else(|| {
                    CliError::invalid_args(
                        "report visuals add-slicer requires --field <Table.Column>",
                    )
                    .with_hint("Pass a model column as `<Table>.<Column>`.")
                    .with_suggested_command(kind.dry_run_command())
                })?,
                "--field",
            )?;
            let slicer_mode = options.slicer_mode.unwrap_or(SlicerModeOpt::Dropdown);
            slicer_visual_json(
                &visual_name,
                title,
                &table,
                &column,
                &position,
                slicer_mode,
                options.single_select,
            )
        }
        ScaffoldKind::Textbox => {
            let paragraphs = load_paragraphs(&options)?;
            textbox_visual_json(&visual_name, title, &position, &paragraphs)
        }
    };
    let visual_dir = page_visuals_dir(&page)?.join(&visual_name);
    let visual_path = visual_dir.join("visual.json");
    ensure_child_path(&visual_dir, &page_visuals_dir(&page)?)?;
    if visual_dir.exists() {
        return Err(CliError::invalid_args(format!(
            "target visual directory already exists: {}",
            visual_dir.display()
        ))
        .with_hint("Choose a unique --name or omit it so powerbi-cli can generate one.")
        .with_suggested_command(kind.dry_run_command()));
    }
    let dry_run = matches!(mode, MutationMode::DryRun);
    if !dry_run {
        fs::create_dir_all(&visual_dir).map_err(|err| {
            CliError::unexpected(format!("create visual dir {}: {err}", visual_dir.display()))
        })?;
        write_visual_json(&visual_path, &visual_json)?;
    }
    mutation_response(MutationPayload {
        kind,
        target_resolved: &target_resolved,
        mode,
        page: &page,
        visual_name: &visual_name,
        title,
        name_generated,
        visual_path: &visual_path,
        position,
        visual_json,
        slicer_mode: options.slicer_mode.unwrap_or(SlicerModeOpt::Dropdown),
        single_select: options.single_select,
    })
}

fn parse_scaffold_args(kind: ScaffoldKind, args: &[String]) -> CliResult<ScaffoldOptions> {
    let mut options = ScaffoldOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            "--page" => options.page = Some(take_value(args, &mut i, "--page")?),
            "--name" => options.name = Some(take_value(args, &mut i, "--name")?),
            "--title" => options.title = Some(take_value(args, &mut i, "--title")?),
            "--measure" | "--field" => {
                let flag = args[i].as_str();
                if !matches!(kind, ScaffoldKind::Card | ScaffoldKind::Slicer) {
                    return unknown_flag(kind, flag);
                }
                if kind == ScaffoldKind::Slicer && flag == "--measure" {
                    return unknown_flag(kind, flag);
                }
                if options.field.is_some() {
                    return Err(CliError::invalid_args(format!(
                        "{} accepts only one --measure/--field value",
                        kind.command()
                    ))
                    .with_hint("Pass a single `<Table>.<Name>` binding.")
                    .with_suggested_command(kind.dry_run_command()));
                }
                options.field = Some(take_value(args, &mut i, flag)?);
            }
            "--x" => options.x = Some(take_f64(args, &mut i, "--x")?),
            "--y" => options.y = Some(take_f64(args, &mut i, "--y")?),
            "--width" => options.width = Some(take_f64(args, &mut i, "--width")?),
            "--height" => options.height = Some(take_f64(args, &mut i, "--height")?),
            "--value-font-size" => {
                if kind != ScaffoldKind::Card {
                    return unknown_flag(kind, "--value-font-size");
                }
                options.value_font_size =
                    Some(take_positive_f64(args, &mut i, "--value-font-size")?);
            }
            "--category-font-size" => {
                if kind != ScaffoldKind::Card {
                    return unknown_flag(kind, "--category-font-size");
                }
                options.category_font_size =
                    Some(take_positive_f64(args, &mut i, "--category-font-size")?);
            }
            "--word-wrap" => {
                if kind != ScaffoldKind::Card {
                    return unknown_flag(kind, "--word-wrap");
                }
                options.word_wrap = true;
                i += 1;
            }
            "--mode" => {
                if kind != ScaffoldKind::Slicer {
                    return unknown_flag(kind, "--mode");
                }
                options.slicer_mode =
                    Some(parse_slicer_mode(&take_value(args, &mut i, "--mode")?)?);
            }
            "--single-select" => {
                if kind != ScaffoldKind::Slicer {
                    return unknown_flag(kind, "--single-select");
                }
                options.single_select = true;
                i += 1;
            }
            "--paragraphs-file" => {
                if kind != ScaffoldKind::Textbox {
                    return unknown_flag(kind, "--paragraphs-file");
                }
                options.paragraphs_file = Some(take_value(args, &mut i, "--paragraphs-file")?);
            }
            "--text" => {
                if kind != ScaffoldKind::Textbox {
                    return unknown_flag(kind, "--text");
                }
                options.text = Some(take_value(args, &mut i, "--text")?);
            }
            "--dry-run" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::DryRun,
                    SET_MODE_HINT,
                    kind.dry_run_command(),
                )?;
                i += 1;
            }
            "--in-place" => {
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::InPlace,
                    SET_MODE_HINT,
                    kind.dry_run_command(),
                )?;
                i += 1;
            }
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode_with_contract(
                    &mut options.mode,
                    MutationMode::OutDir,
                    SET_MODE_HINT,
                    kind.dry_run_command(),
                )?;
                options.out_dir = Some(out_dir);
            }
            other => return unknown_flag(kind, other),
        }
    }
    Ok(options)
}

fn unknown_flag(kind: ScaffoldKind, flag: &str) -> CliResult<ScaffoldOptions> {
    Err(
        CliError::invalid_args(format!("unknown {} flag: {flag}", kind.command()))
            .with_hint(format!(
                "Run `powerbi-cli --json capabilities --for \"{}\"` for exact flags.",
                kind.command()
            ))
            .with_suggested_command(format!(
                "powerbi-cli --json capabilities --for \"{}\"",
                kind.command()
            )),
    )
}

struct CardSpec<'a> {
    name: &'a str,
    title: &'a str,
    table: &'a str,
    measure: &'a str,
    position: &'a Value,
    value_font_size: Option<f64>,
    category_font_size: Option<f64>,
    word_wrap: bool,
}

fn card_visual_json(spec: &CardSpec<'_>) -> Value {
    let mut visual = Map::new();
    visual.insert("drillFilterOtherVisuals".to_string(), Value::Bool(true));
    if let Some(objects) = card_objects(
        spec.value_font_size,
        spec.category_font_size,
        spec.word_wrap,
    ) {
        visual.insert("objects".to_string(), objects);
    }
    visual.insert(
        "query".to_string(),
        json!({
            "queryState": {
                "Values": {
                    "projections": [{
                        "field": {
                            "Measure": {
                                "Expression": { "SourceRef": { "Entity": spec.table } },
                                "Property": spec.measure
                            }
                        },
                        "nativeQueryRef": spec.measure,
                        "queryRef": format!("{}.{}", spec.table, spec.measure)
                    }]
                }
            }
        }),
    );
    visual.insert(
        "visualContainerObjects".to_string(),
        title_container_objects(spec.title),
    );
    visual.insert("visualType".to_string(), Value::String("card".to_string()));
    scaffold_container(
        spec.name,
        spec.title,
        spec.position,
        ScaffoldKind::Card,
        Value::Object(visual),
    )
}

fn slicer_visual_json(
    name: &str,
    title: &str,
    table: &str,
    column: &str,
    position: &Value,
    mode: SlicerModeOpt,
    single_select: bool,
) -> Value {
    let mut objects = Map::new();
    objects.insert(
        "data".to_string(),
        json!([{
            "properties": {
                "mode": literal_text_expression(mode.as_str())
            }
        }]),
    );
    if single_select {
        objects.insert(
            "selection".to_string(),
            json!([{
                "properties": {
                    "singleSelect": literal_bool_expression(true)
                }
            }]),
        );
    }
    let visual = json!({
        "drillFilterOtherVisuals": true,
        "objects": Value::Object(objects),
        "query": {
            "queryState": {
                "Values": {
                    "projections": [{
                        "field": {
                            "Column": {
                                "Expression": { "SourceRef": { "Entity": table } },
                                "Property": column
                            }
                        },
                        "nativeQueryRef": column,
                        "queryRef": format!("{table}.{column}"),
                        "active": true
                    }]
                }
            }
        },
        "visualContainerObjects": title_container_objects(title),
        "visualType": "slicer"
    });
    scaffold_container(name, title, position, ScaffoldKind::Slicer, visual)
}

fn textbox_visual_json(name: &str, title: &str, position: &Value, paragraphs: &[String]) -> Value {
    let paragraph_values = paragraphs
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                json!({
                    "textRuns": [{
                        "value": line,
                        "textStyle": {
                            "fontWeight": "bold",
                            "fontSize": "12pt"
                        }
                    }]
                })
            } else {
                json!({
                    "textRuns": [{
                        "value": line,
                        "textStyle": {
                            "fontSize": "10pt"
                        }
                    }]
                })
            }
        })
        .collect::<Vec<_>>();
    let visual = json!({
        "drillFilterOtherVisuals": true,
        "objects": {
            "general": [{
                "properties": {
                    "paragraphs": paragraph_values
                }
            }]
        },
        "visualType": "textbox"
    });
    scaffold_container(name, title, position, ScaffoldKind::Textbox, visual)
}

fn scaffold_container(
    name: &str,
    title: &str,
    position: &Value,
    kind: ScaffoldKind,
    visual: Value,
) -> Value {
    let mut annotations = vec![json!({
        "name": "powerbi-cli.placeholderTitle",
        "value": title
    })];
    if kind != ScaffoldKind::Textbox {
        annotations.push(json!({
            "name": "powerbi-cli.bindingStatus",
            "value": "bound"
        }));
    }
    json!({
        "$schema": VISUAL_CONTAINER_SCHEMA,
        "annotations": annotations,
        "howCreated": kind.how_created(),
        "name": name,
        "position": position,
        "visual": visual
    })
}

fn card_objects(
    value_font_size: Option<f64>,
    category_font_size: Option<f64>,
    word_wrap: bool,
) -> Option<Value> {
    if value_font_size.is_none() && category_font_size.is_none() && !word_wrap {
        return None;
    }
    let mut objects = Map::new();
    if let Some(size) = value_font_size {
        objects.insert(
            "labels".to_string(),
            json!([{
                "properties": {
                    "fontSize": literal_double_expression(size)
                }
            }]),
        );
    }
    if category_font_size.is_some() || word_wrap {
        let mut properties = Map::new();
        properties.insert("show".to_string(), literal_bool_expression(true));
        if let Some(size) = category_font_size {
            properties.insert("fontSize".to_string(), literal_double_expression(size));
        }
        if word_wrap {
            properties.insert("wordWrap".to_string(), literal_bool_expression(true));
        }
        objects.insert(
            "categoryLabels".to_string(),
            json!([{ "properties": Value::Object(properties) }]),
        );
    }
    Some(Value::Object(objects))
}

fn title_container_objects(title: &str) -> Value {
    json!({
        "title": [{
            "properties": {
                "show": literal_bool_expression(true),
                "text": literal_text_expression(title)
            }
        }]
    })
}

struct MutationPayload<'a> {
    kind: ScaffoldKind,
    target_resolved: &'a ResolvedProject,
    mode: MutationMode,
    page: &'a PageRecord,
    visual_name: &'a str,
    title: &'a str,
    name_generated: bool,
    visual_path: &'a Path,
    position: Value,
    visual_json: Value,
    slicer_mode: SlicerModeOpt,
    single_select: bool,
}

fn mutation_response(payload: MutationPayload<'_>) -> CliResult<Value> {
    let MutationPayload {
        kind,
        target_resolved,
        mode,
        page,
        visual_name,
        title,
        name_generated,
        visual_path,
        position,
        visual_json,
        slicer_mode,
        single_select,
    } = payload;
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
    let target_handle = format!("visual:{}:{visual_name}", page.name);
    let readback = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        shell_arg(&target_handle)
    );
    let slicer_readback = (kind == ScaffoldKind::Slicer).then(|| {
        format!(
            "powerbi-cli report slicers show --project {} --handle {} --json",
            project_arg,
            shell_arg(&format!("slicer:{}:{visual_name}", page.name))
        )
    });
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
    let mut next = vec![
        readback.clone(),
        wireframe.clone(),
        inspect.clone(),
        validate.clone(),
    ];
    if let Some(command) = &slicer_readback {
        next.insert(1, command.clone());
    }
    let mut target = json!({
        "handle": target_handle,
        "name": visual_name,
        "title": title,
        "visualType": kind.visual_type(),
        "page": {
            "handle": page.handle,
            "name": page.name,
            "displayName": page.display_name,
            "ordinal": page.ordinal
        },
        "path": canonical_display(visual_path),
        "position": position,
        "nameGenerated": name_generated
    });
    if kind == ScaffoldKind::Slicer {
        target["mode"] = Value::String(slicer_mode.as_str().to_string());
        target["singleSelect"] = Value::Bool(single_select);
    }
    Ok(json!({
        "schema": "powerbi-cli.report.visuals.scaffoldMutation.v1",
        "ok": validation_ok,
        "exitCode": exit_code,
        "action": kind.action(),
        "dryRun": dry_run,
        "mode": mode_name(mode),
        "projectDir": canonical_display(&target_resolved.project_dir),
        "pbip": canonical_display(&target_resolved.pbip_path),
        "reportDir": canonical_display(&target_resolved.report_dir),
        "target": target,
        "changes": [{
            "kind": "pbir.visual",
            "action": kind.action(),
            "path": canonical_display(visual_path),
            "before": Value::Null,
            "after": visual_json
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
        "slicerReadbackCommand": slicer_readback,
        "wireframeCommand": wireframe,
        "inspectCommand": inspect,
        "validateCommand": validate,
        "next": next
    }))
}

fn load_paragraphs(options: &ScaffoldOptions) -> CliResult<Vec<String>> {
    match (options.paragraphs_file.as_deref(), options.text.as_deref()) {
        (Some(_), Some(_)) => Err(CliError::invalid_args(
            "report visuals add-textbox accepts exactly one of --paragraphs-file or --text",
        )
        .with_hint("Pass a UTF-8 paragraphs file or a single --text paragraph, not both.")
        .with_suggested_command(ScaffoldKind::Textbox.dry_run_command())),
        (None, None) => Err(CliError::invalid_args(
            "report visuals add-textbox requires --paragraphs-file <path|-> or --text <paragraph>",
        )
        .with_hint(
            "Use a UTF-8 file with one paragraph per line, or --text for a single paragraph.",
        )
        .with_suggested_command(ScaffoldKind::Textbox.dry_run_command())),
        (None, Some(text)) => {
            validate_nonempty_text(text, "--text")?;
            Ok(vec![text.to_string()])
        }
        (Some(path), None) => {
            let raw = read_paragraphs_source(path)?;
            let paragraphs = raw
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if paragraphs.is_empty() {
                return Err(CliError::invalid_args(
                    "--paragraphs-file must contain at least one non-blank paragraph",
                )
                .with_hint("Write one paragraph per line and skip blank lines.")
                .with_suggested_command(ScaffoldKind::Textbox.dry_run_command()));
            }
            for paragraph in &paragraphs {
                if paragraph.chars().any(char::is_control) {
                    return Err(CliError::invalid_args(
                        "--paragraphs-file lines must not contain control characters",
                    )
                    .with_suggested_command(ScaffoldKind::Textbox.dry_run_command()));
                }
            }
            Ok(paragraphs)
        }
    }
}

fn read_paragraphs_source(path: &str) -> CliResult<String> {
    let text = if path == "-" {
        read_utf8_stream(
            &mut io::stdin(),
            InputKind::SourceText,
            "paragraphs from stdin",
        )?
    } else {
        read_utf8(Path::new(path), InputKind::SourceText)?
    };
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

fn parse_table_field(value: &str, flag: &str) -> CliResult<(String, String)> {
    let trimmed = value.trim();
    let Some((table, name)) = trimmed.split_once('.') else {
        return Err(CliError::invalid_args(format!(
            "{flag} must be <Table>.<Name>"
        ))
        .with_hint("Validate the `<Table>.<Name>` syntax shape only; project model lookup is not required.")
        .with_suggested_command(
            "powerbi-cli report visuals add-card --project <project-dir-or.pbip> --page <page-name-or-handle> --measure FactSales.Total Revenue --title <text> --x 40 --y 40 --width 200 --height 120 --dry-run --json",
        ));
    };
    let table = table.trim();
    let name = name.trim();
    if table.is_empty() || name.is_empty() {
        return Err(CliError::invalid_args(format!(
            "{flag} must be <Table>.<Name>"
        ))
        .with_hint("Both the table and field name are required.")
        .with_suggested_command(
            "powerbi-cli report visuals add-card --project <project-dir-or.pbip> --page <page-name-or-handle> --measure FactSales.Total Revenue --title <text> --x 40 --y 40 --width 200 --height 120 --dry-run --json",
        ));
    }
    Ok((table.to_string(), name.to_string()))
}

fn parse_slicer_mode(value: &str) -> CliResult<SlicerModeOpt> {
    match value.trim().to_ascii_lowercase().as_str() {
        "basic" => Ok(SlicerModeOpt::Basic),
        "dropdown" => Ok(SlicerModeOpt::Dropdown),
        other => Err(CliError::unsupported_feature(format!(
            "unsupported slicer mode: {other}"
        ))
        .with_hint("add-slicer supports Basic and Dropdown. Use `report visuals add --visual-type slicer --mode between` for a range slider.")
        .with_suggested_command(ScaffoldKind::Slicer.dry_run_command())),
    }
}

fn next_stack_index(page: &PageRecord) -> u64 {
    page.visuals
        .iter()
        .map(|visual| json_u64(&visual.position["z"]).max(json_u64(&visual.position["tabOrder"])))
        .max()
        .unwrap_or(0)
        + 1
}

fn json_u64(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| value.as_f64().and_then(|n| (n >= 0.0).then_some(n as u64)))
        .unwrap_or(0)
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

fn validate_new_visual_name(
    name: &str,
    page: &PageRecord,
    kind: ScaffoldKind,
) -> CliResult<String> {
    validate_visual_name(name, kind)?;
    if page.visuals.iter().any(|visual| visual.name == name) {
        return Err(CliError::invalid_args(format!(
            "visual already exists on page {}: {name}",
            page.handle
        ))
        .with_hint("Choose a unique internal --name or omit it so powerbi-cli can generate one.")
        .with_suggested_command(kind.dry_run_command()));
    }
    Ok(name.to_string())
}

fn validate_visual_name(name: &str, kind: ScaffoldKind) -> CliResult<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch == ':' || ch.is_control())
    {
        return Err(
            CliError::invalid_args(format!("unsafe visual name: {name}"))
                .with_hint("Use a simple internal visual name without path separators.")
                .with_suggested_command(kind.dry_run_command()),
        );
    }
    Ok(())
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
    kind: ScaffoldKind,
) -> CliResult<()> {
    let x = position["x"].as_f64().unwrap_or_default();
    let y = position["y"].as_f64().unwrap_or_default();
    let width = position["width"].as_f64().unwrap_or_default();
    let height = position["height"].as_f64().unwrap_or_default();
    if let (Some(page_width), Some(page_height)) = (page_width, page_height)
        && (x + width > page_width || y + height > page_height)
    {
        return Err(
            CliError::invalid_args("visual position would extend outside page bounds")
                .with_hint("Keep the visual inside the page.")
                .with_suggested_command(kind.dry_run_command()),
        );
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
    if path_abs.starts_with(&parent_abs) {
        return Ok(());
    }
    Err(CliError::validation_failed(format!(
        "refusing to write visual outside page visuals directory: {}",
        path.display()
    )))
}

fn write_visual_json(path: &Path, value: &Value) -> CliResult<()> {
    if path.exists() {
        return write_json_atomic(path, value);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    }
    let mut text = serde_json::to_string_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    fs::write(path, text)
        .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))
}

fn require_geometry(value: Option<f64>, flag: &str, kind: ScaffoldKind) -> CliResult<f64> {
    value.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{} requires --x --y --width --height",
            kind.command()
        ))
        .with_hint(format!("{flag} is required."))
        .with_suggested_command(kind.dry_run_command())
    })
}

fn require_positive_geometry(value: Option<f64>, flag: &str, kind: ScaffoldKind) -> CliResult<f64> {
    let value = require_geometry(value, flag, kind)?;
    if value > 0.0 {
        return Ok(value);
    }
    Err(
        CliError::invalid_args(format!("{flag} must be a positive finite number"))
            .with_suggested_command(kind.dry_run_command()),
    )
}

fn validate_nonempty_text(value: &str, flag: &str) -> CliResult<()> {
    if !value.trim().is_empty() && !value.chars().any(char::is_control) {
        return Ok(());
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be nonempty text"
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

fn take_positive_f64(args: &[String], index: &mut usize, flag: &str) -> CliResult<f64> {
    let value = take_f64(args, index, flag)?;
    if value > 0.0 {
        return Ok(value);
    }
    Err(CliError::invalid_args(format!(
        "{flag} must be a positive finite number"
    )))
}

fn literal_text_expression(text: &str) -> Value {
    json!({ "expr": { "Literal": { "Value": encode_text_literal(text) } } })
}

fn literal_bool_expression(value: bool) -> Value {
    json!({ "expr": { "Literal": { "Value": value.to_string() } } })
}

fn literal_double_expression(value: f64) -> Value {
    json!({ "expr": { "Literal": { "Value": encode_double_literal(value) } } })
}

fn encode_text_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn encode_double_literal(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}D", value as i64)
    } else {
        format!("{value}D")
    }
}
