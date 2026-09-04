//! Deterministic, offline report wireframe rendering.
//!
//! JSON wireframe export is kept as the compatibility baseline.  SVG and HTML
//! previews use the same deep-inspection geometry as JSON and the design grid
//! resolver for slot/guide coordinates; they never mutate the PBIP project.

use crate::cli_support::{MutationMode, take_report_value};
use crate::design::grid::{
    Grid, PageSize, SlotPosition, Template, guide_lines, resolve_with_grid, template,
};
use crate::input_safety::artifact_destination;
use crate::inspect::deep_inspect;
use crate::project_io::write_text_atomic;
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, Finding, ResolvedProject,
    canonical_display, command_arg, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const DEFAULT_TEMPLATE: &str = "overview";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireframeFormat {
    Json,
    Svg,
    Html,
}

impl WireframeFormat {
    fn parse(value: &str) -> CliResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "svg" => Ok(Self::Svg),
            "html" | "htm" => Ok(Self::Html),
            other => Err(CliError::invalid_args(format!(
                "unsupported wireframe format: {other}"
            ))
            .with_pointer("/format")
            .with_hint("Choose json, svg, or html.")
            .with_suggested_command(
                "powerbi-cli report wireframe export <project-dir-or.pbip> --format svg --dry-run --json",
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Svg => "svg",
            Self::Html => "html",
        }
    }
}

#[derive(Debug, Default)]
struct WireframeOptions {
    path: Option<PathBuf>,
    format: Option<WireframeFormat>,
    template: Option<String>,
    page_size: Option<PageSize>,
    grid: Grid,
    mode: Option<MutationMode>,
    out: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct PageArtifact {
    ordinal: usize,
    name: String,
    display_name: String,
    handle: String,
    width: f64,
    height: f64,
    file_name: String,
    svg: String,
    slots: Vec<Value>,
    visuals: Vec<Value>,
    markers: Vec<LintMarker>,
}

#[derive(Debug, Clone)]
struct LintMarker {
    code: String,
    severity: String,
    message: String,
    page: Option<usize>,
    visual: Option<usize>,
}

/// Dispatch the existing `report wireframe export` path.
pub(crate) fn wireframe_export(args: &[String]) -> CliResult<Value> {
    let options = parse_wireframe_args(args)?;
    let path = options.path.clone().ok_or_else(|| {
        CliError::invalid_args("report wireframe export requires a path")
            .with_hint(wireframe_usage())
            .with_suggested_command(wireframe_suggested_command())
    })?;
    let format = options.format.unwrap_or(WireframeFormat::Json);

    if format == WireframeFormat::Json {
        if options.mode.is_some() || options.out.is_some() {
            return Err(CliError::invalid_args(
                "JSON wireframe export is read-only and does not accept --out or --dry-run",
            )
            .with_hint("Use --format svg or --format html for external artifacts.")
            .with_suggested_command(
                "powerbi-cli report wireframe export <project-dir-or.pbip> --format svg --dry-run --json",
            ));
        }
        let resolved = resolve_project(&path)?;
        let validation = validate_project(&resolved)?;
        let deep = deep_inspect(&resolved, &validation)?;
        return json_wireframe(&resolved, &validation, &deep);
    }

    let mode = options.mode.ok_or_else(|| {
        CliError::invalid_args(format!(
            "wireframe {} export requires --dry-run or --out <path>",
            format.as_str()
        ))
        .with_hint("Start with --dry-run; use --out only after reviewing the artifact.")
        .with_suggested_command(format!(
            "powerbi-cli report wireframe export <project-dir-or.pbip> --format {} --dry-run --json",
            format.as_str()
        ))
    })?;
    if mode == MutationMode::InPlace {
        return Err(CliError::invalid_args(
            "wireframe export cannot use --in-place; choose --dry-run or --out <path>",
        )
        .with_hint("Wireframe artifacts are external previews and never modify the PBIP project.")
        .with_suggested_command(format!(
            "powerbi-cli report wireframe export <project-dir-or.pbip> --format {} --dry-run --json",
            format.as_str()
        )));
    }
    if mode == MutationMode::OutDir && options.out.is_none() {
        return Err(CliError::invalid_args(
            "wireframe export --out mode requires an output path",
        ));
    }

    let resolved = resolve_project(&path)?;
    let validation = validate_project(&resolved)?;
    let deep = deep_inspect(&resolved, &validation)?;
    render_artifacts(&resolved, &validation, &deep, &options, format, mode)
}

fn json_wireframe(
    resolved: &ResolvedProject,
    validation: &crate::ValidationReport,
    deep: &Value,
) -> CliResult<Value> {
    let report = deep["report"].clone();
    let handles = deep["handles"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| matches!(item["kind"].as_str(), Some("project" | "page" | "visual")))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(json!({
        "schema": "powerbi-cli.report.wireframe.v1",
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "valid": validation.errors.is_empty(),
        "counts": {
            "pages": validation.pages,
            "visuals": validation.visuals,
            "boundVisuals": validation.bound_visuals
        },
        "handles": handles,
        "pages": report["pages"].clone(),
        "next": [
            format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate {} --json", command_arg(&resolved.project_dir))
        ],
        "warnings": validation.warnings,
        "errors": validation.errors
    }))
}

fn parse_wireframe_args(args: &[String]) -> CliResult<WireframeOptions> {
    let mut options = WireframeOptions {
        grid: Grid::default(),
        ..WireframeOptions::default()
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" | "-f" => {
                let value = take_report_value(args, &mut index, "--format")?;
                options.format = Some(WireframeFormat::parse(&value)?);
            }
            value if value.starts_with("--format=") => {
                options.format = Some(WireframeFormat::parse(&value[9..])?);
                index += 1;
            }
            "--project" | "-p" => {
                set_path(
                    &mut options.path,
                    PathBuf::from(take_report_value(args, &mut index, "--project")?),
                )?;
            }
            "--template" => {
                options.template = Some(take_report_value(args, &mut index, "--template")?);
            }
            "--page-size" | "--size" => {
                options.page_size = Some(parse_page_size(&take_report_value(
                    args,
                    &mut index,
                    "--page-size",
                )?)?);
            }
            "--grid" => {
                let value = take_report_value(args, &mut index, "--grid")?;
                parse_grid_override(&mut options.grid, &value)?;
            }
            "--margin" => {
                options.grid.margin = parse_number(
                    &take_report_value(args, &mut index, "--margin")?,
                    "/grid/margin",
                )?;
            }
            "--gap" | "--gutter" => {
                options.grid.gutter = parse_number(
                    &take_report_value(args, &mut index, "--gutter")?,
                    "/grid/gutter",
                )?;
            }
            "--row-unit" | "--rowUnit" => {
                options.grid.row_unit = parse_number(
                    &take_report_value(args, &mut index, "--row-unit")?,
                    "/grid/rowUnit",
                )?;
            }
            "--dry-run" => {
                set_wireframe_mode(&mut options.mode, MutationMode::DryRun)?;
                index += 1;
            }
            "--out" | "--out-dir" => {
                let flag = args[index].clone();
                let out = PathBuf::from(take_report_value(args, &mut index, &flag)?);
                set_wireframe_mode(&mut options.mode, MutationMode::OutDir)?;
                options.out = Some(out);
            }
            "--in-place" => {
                set_wireframe_mode(&mut options.mode, MutationMode::InPlace)?;
                index += 1;
            }
            "--json" => index += 1,
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown report wireframe export flag: {other}"
                ))
                .with_hint(wireframe_usage())
                .with_suggested_command(wireframe_suggested_command()));
            }
            other => {
                set_path(&mut options.path, PathBuf::from(other))?;
                index += 1;
            }
        }
    }
    if options.path.is_none() {
        return Err(
            CliError::invalid_args("report wireframe export requires a path")
                .with_hint(wireframe_usage())
                .with_suggested_command(wireframe_suggested_command()),
        );
    }
    Ok(options)
}

fn set_path(path: &mut Option<PathBuf>, next: PathBuf) -> CliResult<()> {
    if path.is_some() {
        return Err(CliError::invalid_args(
            "report wireframe export accepts exactly one project path",
        )
        .with_hint(wireframe_usage())
        .with_suggested_command(wireframe_suggested_command()));
    }
    *path = Some(next);
    Ok(())
}

fn set_wireframe_mode(mode: &mut Option<MutationMode>, next: MutationMode) -> CliResult<()> {
    if mode.is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one output mode: --dry-run or --out <path>",
        )
        .with_hint("Wireframe exports never modify the project; use --dry-run or --out.")
        .with_suggested_command(wireframe_suggested_command()));
    }
    *mode = Some(next);
    Ok(())
}

fn parse_page_size(raw: &str) -> CliResult<PageSize> {
    if let Some(page_size) = PageSize::preset(raw) {
        return Ok(page_size);
    }
    let (width, height) = raw
        .split_once('x')
        .or_else(|| raw.split_once('X'))
        .ok_or_else(|| {
            CliError::invalid_args(format!("unknown page-size preset: {raw}"))
                .with_pointer("/pageSize")
                .with_hint("Use 1280x720, 1920x1080, standard, or wide.")
        })?;
    let width = width.parse::<f64>().map_err(|_| {
        CliError::invalid_args("page-size width must be a number").with_pointer("/pageSize/width")
    })?;
    let height = height.parse::<f64>().map_err(|_| {
        CliError::invalid_args("page-size height must be a number").with_pointer("/pageSize/height")
    })?;
    Ok(PageSize { width, height })
}

fn parse_grid_override(grid: &mut Grid, raw: &str) -> CliResult<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CliError::invalid_args("--grid requires a value")
            .with_pointer("/grid")
            .with_hint("Use --grid columns=12,gutter=16,margin=24,rowUnit=8."));
    }
    if trimmed.starts_with('{') {
        *grid = serde_json::from_str(trimmed).map_err(|error| {
            CliError::invalid_args(format!("--grid must be a grid object: {error}"))
                .with_pointer("/grid")
        })?;
        return Ok(());
    }
    for entry in trimmed.split(',') {
        let (key, value) = entry.split_once('=').ok_or_else(|| {
            CliError::invalid_args(format!("--grid entry must be key=value: {entry}"))
                .with_pointer("/grid")
                .with_hint("Use --grid columns=12,gutter=16,margin=24,rowUnit=8.")
        })?;
        match key.trim().to_ascii_lowercase().as_str() {
            "columns" => {
                grid.columns = value.trim().parse::<u32>().map_err(|_| {
                    CliError::invalid_args("--grid columns must be a positive integer")
                        .with_pointer("/grid/columns")
                })?;
            }
            "gutter" => grid.gutter = parse_number(value.trim(), "/grid/gutter")?,
            "margin" => grid.margin = parse_number(value.trim(), "/grid/margin")?,
            "rowunit" | "row-unit" => {
                grid.row_unit = parse_number(value.trim(), "/grid/rowUnit")?;
            }
            other => {
                return Err(
                    CliError::invalid_args(format!("unknown --grid setting: {other}"))
                        .with_pointer(format!("/grid/{other}"))
                        .with_hint("Supported settings are columns, gutter, margin, and rowUnit."),
                );
            }
        }
    }
    Ok(())
}

fn parse_number(raw: &str, pointer: &str) -> CliResult<f64> {
    raw.parse::<f64>().map_err(|_| {
        CliError::invalid_args(format!("grid value must be a number: {raw}")).with_pointer(pointer)
    })
}

fn render_artifacts(
    resolved: &ResolvedProject,
    validation: &crate::ValidationReport,
    deep: &Value,
    options: &WireframeOptions,
    format: WireframeFormat,
    mode: MutationMode,
) -> CliResult<Value> {
    let template_name = options.template.as_deref().unwrap_or(DEFAULT_TEMPLATE);
    let template = template(template_name)?;
    let report_pages = deep["report"]["pages"].as_array().ok_or_else(|| {
        CliError::validation_failed("deep inspection did not return report pages")
    })?;
    let markers = lint_markers(validation);
    let mut pages = Vec::with_capacity(report_pages.len());
    for (ordinal, page) in report_pages.iter().enumerate() {
        pages.push(render_page(ordinal, page, &template, options, &markers)?);
    }

    let html = (format == WireframeFormat::Html).then(|| render_html(&pages));
    let dry_run = mode == MutationMode::DryRun;
    let mut artifacts = Vec::new();
    if dry_run {
        match format {
            WireframeFormat::Svg => {
                for page in &pages {
                    artifacts.push(json!({
                        "kind": "svg",
                        "page": page.handle,
                        "fileName": page.file_name,
                        "bytes": page.svg.len(),
                        "content": page.svg
                    }));
                }
            }
            WireframeFormat::Html => {
                let content = html.as_deref().unwrap_or_default();
                artifacts.push(json!({
                    "kind": "html",
                    "fileName": "wireframe.html",
                    "bytes": content.len(),
                    "content": content
                }));
            }
            WireframeFormat::Json => unreachable!("JSON handled before renderer"),
        }
    } else {
        let out = options.out.as_deref().ok_or_else(|| {
            CliError::invalid_args("wireframe export --out mode requires an output path")
        })?;
        let destination = artifact_destination(&resolved.project_dir, out)?;
        match format {
            WireframeFormat::Svg => write_svg_artifacts(
                &resolved.project_dir,
                out,
                &destination,
                &pages,
                &mut artifacts,
            )?,
            WireframeFormat::Html => {
                let content = html.as_deref().unwrap_or_default();
                if destination.exists() && destination.is_dir() {
                    return Err(CliError::invalid_args(format!(
                        "HTML wireframe output must be a file, not a directory: {}",
                        out.display()
                    ))
                    .with_pointer("/out"));
                }
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        CliError::unexpected(format!(
                            "create wireframe output parent {}: {error}",
                            parent.display()
                        ))
                    })?;
                }
                write_text_atomic(&destination, content)?;
                artifacts.push(json!({
                    "kind": "html",
                    "fileName": "wireframe.html",
                    "path": canonical_display(&destination),
                    "bytes": content.len()
                }));
            }
            WireframeFormat::Json => unreachable!("JSON handled before renderer"),
        }
    }

    let page_values = pages
        .iter()
        .map(|page| {
            json!({
                "ordinal": page.ordinal,
                "handle": page.handle,
                "name": page.name,
                "displayName": page.display_name,
                "width": number_json(page.width),
                "height": number_json(page.height),
                "slotCount": page.slots.len(),
                "visualCount": page.visuals.len(),
                "slots": page.slots,
                "visuals": page.visuals,
                "lintMarkers": page.markers.iter().map(marker_json).collect::<Vec<_>>(),
                "artifact": page.file_name
            })
        })
        .collect::<Vec<_>>();
    let handles = deep["handles"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| matches!(item["kind"].as_str(), Some("project" | "page" | "visual")))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let errors = validation
        .errors
        .iter()
        .map(finding_json)
        .collect::<Vec<_>>();
    let warnings = validation
        .warnings
        .iter()
        .map(finding_json)
        .collect::<Vec<_>>();
    let valid = validation.errors.is_empty();
    let mut output = json!({
        "schema": "powerbi-cli.report.wireframe.v2",
        "ok": valid,
        "exitCode": if valid { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "format": format.as_str(),
        "dryRun": dry_run,
        "mode": if dry_run { "dry-run" } else { "out" },
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "template": template.name,
        "grid": template_grid_json(&options.grid),
        "geometrySource": "inspect.deep.report.pages[].visuals[].position",
        "gridSource": "design.grid.resolve_with_grid",
        "valid": valid,
        "counts": {
            "pages": validation.pages,
            "visuals": validation.visuals,
            "boundVisuals": validation.bound_visuals,
            "slots": pages.iter().map(|page| page.slots.len()).sum::<usize>(),
            "lintMarkers": pages.iter().map(|page| page.markers.len()).sum::<usize>()
        },
        "handles": handles,
        "pages": page_values,
        "artifacts": artifacts,
        "warnings": warnings,
        "errors": errors,
        "next": [
            format!("powerbi-cli inspect --deep {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli validate {} --json", command_arg(&resolved.project_dir)),
            format!("powerbi-cli report wireframe export {} --format json --json", command_arg(&resolved.project_dir))
        ]
    });
    if let Some(out) = options.out.as_ref()
        && !dry_run
    {
        output["out"] = Value::String(canonical_display(out));
    }
    Ok(output)
}

fn render_page(
    ordinal: usize,
    page: &Value,
    template: &Template,
    options: &WireframeOptions,
    markers: &[LintMarker],
) -> CliResult<PageArtifact> {
    let source_width = value_number(&page["width"], PageSize::STANDARD.width);
    let source_height = value_number(&page["height"], PageSize::STANDARD.height);
    let page_size = options.page_size.unwrap_or(PageSize {
        width: source_width,
        height: source_height,
    });
    let (positions, vertical_guides, horizontal_guides) =
        resolve_preview_geometry(template, page_size, options.grid)?;
    let visuals = page["visuals"].as_array().cloned().unwrap_or_default();
    let page_markers = markers
        .iter()
        .filter(|marker| marker.page.is_none() || marker.page == Some(ordinal))
        .cloned()
        .collect::<Vec<_>>();
    let width = page_size.width;
    let height = page_size.height;
    let scale_x = if source_width > 0.0 {
        width / source_width
    } else {
        1.0
    };
    let scale_y = if source_height > 0.0 {
        height / source_height
    } else {
        1.0
    };
    let svg = render_svg(
        page,
        template,
        &positions,
        (&vertical_guides, &horizontal_guides),
        &visuals,
        &page_markers,
        scale_x,
        scale_y,
        width,
        height,
    );
    let slots = template
        .slots
        .iter()
        .map(|slot| {
            let position = positions
                .get(&slot.name)
                .copied()
                .map(slot_position_json)
                .unwrap_or(Value::Null);
            json!({
                "name": slot.name,
                "col": slot.col,
                "row": slot.row,
                "colSpan": slot.col_span,
                "rowSpan": slot.row_span,
                "preferredFamilies": slot.preferred_families,
                "minFamily": slot.min_family,
                "position": position
            })
        })
        .collect::<Vec<_>>();
    let visual_values = visuals
        .iter()
        .enumerate()
        .map(|(index, visual)| visual_summary(index, visual, scale_x, scale_y))
        .collect::<Vec<_>>();
    let name = value_string(&page["name"], &format!("Page{ordinal}"));
    let display_name = value_string(&page["displayName"], &name);
    let handle = value_string(&page["handle"], &format!("page:{name}"));
    Ok(PageArtifact {
        ordinal,
        name: name.clone(),
        display_name,
        handle,
        width,
        height,
        file_name: format!("{}.svg", safe_file_stem(&name, ordinal)),
        svg,
        slots,
        visuals: visual_values,
        markers: page_markers,
    })
}

fn resolve_preview_geometry(
    template: &Template,
    page_size: PageSize,
    grid: Grid,
) -> CliResult<(
    std::collections::BTreeMap<String, SlotPosition>,
    Vec<f64>,
    Vec<f64>,
)> {
    match (
        resolve_with_grid(template, page_size, grid, None),
        page_size == PageSize::STANDARD,
    ) {
        (Ok(positions), _) => {
            let (vertical, horizontal) = guide_lines(template, page_size, grid)?;
            Ok((positions, vertical, horizontal))
        }
        (Err(error), true) => Err(error),
        (Err(error), false) if error.message.contains("is too small for minFamily") => {
            // Some Desktop-authored fixtures use a compact canvas (for
            // example 800x500) that is below the design-system minimum text
            // floor.  Resolve once at the reference canvas, then scale that
            // engine result for a faithful preview instead of inventing a
            // second slot algorithm.
            let reference_positions = resolve_with_grid(template, PageSize::STANDARD, grid, None)?;
            let (reference_vertical, reference_horizontal) =
                guide_lines(template, PageSize::STANDARD, grid)?;
            let scale_x = page_size.width / PageSize::STANDARD.width;
            let scale_y = page_size.height / PageSize::STANDARD.height;
            let positions = reference_positions
                .into_iter()
                .map(|(name, position)| {
                    (
                        name,
                        SlotPosition {
                            x: position.x * scale_x,
                            y: position.y * scale_y,
                            width: position.width * scale_x,
                            height: position.height * scale_y,
                        },
                    )
                })
                .collect();
            let vertical = reference_vertical
                .into_iter()
                .map(|value| value * scale_x)
                .collect();
            let horizontal = reference_horizontal
                .into_iter()
                .map(|value| value * scale_y)
                .collect();
            Ok((positions, vertical, horizontal))
        }
        (Err(error), false) => Err(error),
    }
}

fn render_svg(
    page: &Value,
    template: &Template,
    positions: &std::collections::BTreeMap<String, SlotPosition>,
    guides: (&[f64], &[f64]),
    visuals: &[Value],
    markers: &[LintMarker],
    scale_x: f64,
    scale_y: f64,
    width: f64,
    height: f64,
) -> String {
    let page_name = value_string(&page["name"], "Page");
    let page_title = value_string(&page["displayName"], &page_name);
    let id = safe_id(&format!("wireframe-{page_name}"));
    let mut svg = String::new();
    svg.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        svg,
        "<svg xmlns=\"{SVG_NAMESPACE}\" role=\"img\" aria-labelledby=\"{id}-title\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        fmt_num(width),
        fmt_num(height),
        fmt_num(width),
        fmt_num(height)
    );
    let _ = writeln!(
        svg,
        "  <title id=\"{id}-title\">{}</title>",
        xml_escape(&page_title)
    );
    svg.push_str("  <style>\n");
    svg.push_str(
        "    .canvas{fill:#ffffff;stroke:#cbd5e1;stroke-width:1}.grid{stroke:#dbe4ef;stroke-width:1;stroke-dasharray:2 4}.slot{fill:#f8fafc;stroke:#94a3b8;stroke-width:1;stroke-dasharray:6 3}.slot-label{fill:#64748b;font:10px sans-serif}.visual{fill:#e0f2fe;fill-opacity:.72;stroke:#0369a1;stroke-width:2}.visual-label{fill:#0c4a6e;font:bold 12px sans-serif}.visual-binding{fill:#164e63;font:10px sans-serif}.lint-marker{fill:#dc2626;stroke:#ffffff;stroke-width:1}.lint-label{fill:#991b1b;font:bold 9px sans-serif}.lint-marker.warning{fill:#d97706}.lint-label.warning{fill:#92400e}\n",
    );
    svg.push_str("  </style>\n");
    let _ = writeln!(
        svg,
        "  <rect class=\"canvas\" x=\"0.00\" y=\"0.00\" width=\"{}\" height=\"{}\"/>",
        fmt_num(width),
        fmt_num(height)
    );
    svg.push_str("  <g class=\"grid\" data-template=\"");
    svg.push_str(&xml_escape(&template.name));
    svg.push_str("\">\n");
    for x in guides.0 {
        let _ = writeln!(
            svg,
            "    <line x1=\"{}\" y1=\"0.00\" x2=\"{}\" y2=\"{}\"/>",
            fmt_num(*x),
            fmt_num(*x),
            fmt_num(height)
        );
    }
    for y in guides.1 {
        let _ = writeln!(
            svg,
            "    <line x1=\"0.00\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>",
            fmt_num(*y),
            fmt_num(width),
            fmt_num(*y)
        );
    }
    svg.push_str("  </g>\n");
    svg.push_str("  <g class=\"slots\">\n");
    for slot in &template.slots {
        let Some(position) = positions.get(&slot.name) else {
            continue;
        };
        let _ = writeln!(
            svg,
            "    <g class=\"slot\" data-slot=\"{}\" data-col-span=\"{}\" data-row-span=\"{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/><text class=\"slot-label\" x=\"{}\" y=\"{}\">{} ({}x{})</text></g>",
            xml_escape(&slot.name),
            slot.col_span,
            slot.row_span,
            fmt_num(position.x),
            fmt_num(position.y),
            fmt_num(position.width),
            fmt_num(position.height),
            fmt_num(position.x + 4.0),
            fmt_num(position.y + 13.0),
            xml_escape(&slot.name),
            slot.col_span,
            slot.row_span
        );
    }
    svg.push_str("  </g>\n");
    svg.push_str("  <g class=\"visuals\">\n");
    for (index, visual) in visuals.iter().enumerate() {
        let position = scaled_position(&visual["position"], scale_x, scale_y);
        let handle = value_string(&visual["handle"], &format!("visual-{index}"));
        let visual_type = value_string(&visual["visualType"], "visual");
        let title = value_string(&visual["title"], &handle);
        let binding = binding_summary(visual);
        let _ = writeln!(
            svg,
            "    <g class=\"visual\" data-handle=\"{}\" data-visual-type=\"{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/><text class=\"visual-label\" x=\"{}\" y=\"{}\">{} — {}</text><text class=\"visual-binding\" x=\"{}\" y=\"{}\">{}</text></g>",
            xml_escape(&handle),
            xml_escape(&visual_type),
            fmt_num(position.x),
            fmt_num(position.y),
            fmt_num(position.width),
            fmt_num(position.height),
            fmt_num(position.x + 6.0),
            fmt_num(position.y + 17.0),
            xml_escape(&visual_type),
            xml_escape(&title),
            fmt_num(position.x + 6.0),
            fmt_num(position.y + 31.0),
            xml_escape(&binding)
        );
    }
    svg.push_str("  </g>\n");
    svg.push_str("  <g class=\"lint-markers\">\n");
    let mut marker_offset = 0usize;
    for marker in markers {
        let (x, y) = if let Some(index) = marker.visual {
            visuals
                .get(index)
                .map(|visual| {
                    let position = scaled_position(&visual["position"], scale_x, scale_y);
                    (
                        (position.x + position.width - 10.0).max(8.0),
                        (position.y + 10.0).max(10.0),
                    )
                })
                .unwrap_or((12.0, 16.0 + marker_offset as f64 * 14.0))
        } else {
            (12.0, 16.0 + marker_offset as f64 * 14.0)
        };
        let class = if marker.severity == "warning" {
            "lint-marker warning"
        } else {
            "lint-marker"
        };
        let label_class = if marker.severity == "warning" {
            "lint-label warning"
        } else {
            "lint-label"
        };
        let _ = writeln!(
            svg,
            "    <g data-code=\"{}\" data-severity=\"{}\"><circle class=\"{}\" cx=\"{}\" cy=\"{}\" r=\"6.00\"><title>{}: {}</title></circle><text class=\"{}\" x=\"{}\" y=\"{}\">{}</text></g>",
            xml_escape(&marker.code),
            xml_escape(&marker.severity),
            class,
            fmt_num(x),
            fmt_num(y),
            xml_escape(&marker.code),
            xml_escape(&marker.message),
            label_class,
            fmt_num(x + 9.0),
            fmt_num(y + 3.0),
            xml_escape(&marker.code)
        );
        marker_offset += 1;
    }
    svg.push_str("  </g>\n</svg>\n");
    svg
}

fn render_html(pages: &[PageArtifact]) -> String {
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>Power BI report wireframe</title>\n<style>\nbody{margin:0;background:#f1f5f9;color:#0f172a;font:14px sans-serif}header{position:sticky;top:0;z-index:2;padding:12px 20px;background:#0f172a;color:#f8fafc}nav{display:flex;flex-wrap:wrap;gap:10px;margin-top:8px}nav a{color:#bae6fd;text-decoration:none}main{padding:20px}section{margin:0 auto 28px;max-width:1920px;background:#fff;padding:12px;box-shadow:0 1px 3px #94a3b833}h2{margin:4px 0 12px;font-size:18px}svg{display:block;max-width:100%;height:auto}\n</style>\n</head>\n<body>\n<header><div>Power BI report wireframe</div><nav>\n");
    for page in pages {
        let id = safe_id(&format!("page-{}-{}", page.name, page.ordinal));
        let _ = writeln!(
            html,
            "<a href=\"#{id}\">{}</a>",
            xml_escape(&page.display_name)
        );
    }
    html.push_str("</nav></header>\n<main>\n");
    for page in pages {
        let id = safe_id(&format!("page-{}-{}", page.name, page.ordinal));
        let _ = writeln!(
            html,
            "<section id=\"{id}\"><h2>{}</h2>{}</section>",
            xml_escape(&page.display_name),
            page.svg
        );
    }
    html.push_str("</main>\n</body>\n</html>\n");
    html
}

fn write_svg_artifacts(
    project_root: &Path,
    requested: &Path,
    destination: &Path,
    pages: &[PageArtifact],
    artifacts: &mut Vec<Value>,
) -> CliResult<()> {
    let single_file = pages.len() == 1
        && requested
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
        && !(destination.exists() && destination.is_dir());
    if single_file {
        let page = &pages[0];
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CliError::unexpected(format!(
                    "create wireframe output parent {}: {error}",
                    parent.display()
                ))
            })?;
        }
        write_text_atomic(destination, &page.svg)?;
        artifacts.push(json!({
            "kind": "svg",
            "page": page.handle,
            "fileName": page.file_name,
            "path": canonical_display(destination),
            "bytes": page.svg.len()
        }));
        return Ok(());
    }
    if destination.exists() && !destination.is_dir() {
        return Err(CliError::invalid_args(format!(
            "multi-page SVG output must be a directory: {}",
            requested.display()
        ))
        .with_pointer("/out")
        .with_hint("Use an output directory, or use a .svg file for a single-page report."));
    }
    fs::create_dir_all(destination).map_err(|error| {
        CliError::unexpected(format!(
            "create wireframe SVG output directory {}: {error}",
            destination.display()
        ))
    })?;
    for page in pages {
        let requested_file = destination.join(&page.file_name);
        let target = artifact_destination(project_root, &requested_file)?;
        write_text_atomic(&target, &page.svg)?;
        artifacts.push(json!({
            "kind": "svg",
            "page": page.handle,
            "fileName": page.file_name,
            "path": canonical_display(&target),
            "bytes": page.svg.len()
        }));
    }
    Ok(())
}

fn lint_markers(validation: &crate::ValidationReport) -> Vec<LintMarker> {
    let mut markers = validation
        .errors
        .iter()
        .map(|finding| marker_from_finding(finding))
        .chain(
            validation
                .warnings
                .iter()
                .map(|finding| marker_from_finding(finding)),
        )
        .collect::<Vec<_>>();
    markers.sort_by(|left, right| {
        left.page
            .cmp(&right.page)
            .then_with(|| left.visual.cmp(&right.visual))
            .then_with(|| left.severity.cmp(&right.severity))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    markers
}

fn marker_from_finding(finding: &Finding) -> LintMarker {
    let (page, visual) = finding_location(&finding.pointer);
    LintMarker {
        code: finding.code.clone(),
        severity: finding.severity.clone(),
        message: finding.message.clone(),
        page,
        visual,
    }
}

fn finding_location(pointer: &str) -> (Option<usize>, Option<usize>) {
    let tokens = pointer.split('/').collect::<Vec<_>>();
    let page = tokens
        .windows(2)
        .find(|window| window[0] == "pages")
        .and_then(|window| window[1].parse::<usize>().ok());
    let visual = tokens
        .windows(2)
        .find(|window| window[0] == "visuals")
        .and_then(|window| window[1].parse::<usize>().ok());
    (page, visual)
}

fn marker_json(marker: &LintMarker) -> Value {
    json!({
        "code": marker.code,
        "severity": marker.severity,
        "message": marker.message,
        "page": marker.page,
        "visual": marker.visual
    })
}

fn finding_json(finding: &Finding) -> Value {
    json!({
        "code": finding.code,
        "message": finding.message,
        "path": finding.path,
        "pointer": finding.pointer,
        "severity": finding.severity
    })
}

fn visual_summary(index: usize, visual: &Value, scale_x: f64, scale_y: f64) -> Value {
    let handle = value_string(&visual["handle"], &format!("visual-{index}"));
    let visual_type = value_string(&visual["visualType"], "visual");
    let title = value_string(&visual["title"], &handle);
    json!({
        "index": index,
        "handle": handle,
        "visualType": visual_type,
        "title": title,
        "bindings": visual["bindings"].clone(),
        "bindingsSummary": binding_summary(visual),
        "position": scaled_position_json(&visual["position"], scale_x, scale_y)
    })
}

fn binding_summary(visual: &Value) -> String {
    visual["bindings"]
        .as_array()
        .map(|bindings| {
            bindings
                .iter()
                .map(|binding| {
                    let role = value_string(&binding["role"], "binding");
                    let table = value_string(&binding["table"], "");
                    let field = binding["field"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                        .or_else(|| binding["measure"].as_str())
                        .or_else(|| binding["column"].as_str())
                        .unwrap_or("field");
                    let target = if table.is_empty() {
                        field.to_string()
                    } else {
                        format!("{table}.{field}")
                    };
                    format!("{role}: {target}")
                })
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_else(|| "No bindings".to_string())
}

fn scaled_position_json(value: &Value, scale_x: f64, scale_y: f64) -> Value {
    let position = scaled_position(value, scale_x, scale_y);
    json!({
        "x": number_json(position.x),
        "y": number_json(position.y),
        "width": number_json(position.width),
        "height": number_json(position.height),
        "z": value["z"].clone(),
        "tabOrder": value["tabOrder"].clone()
    })
}

fn scaled_position(value: &Value, scale_x: f64, scale_y: f64) -> SlotPosition {
    SlotPosition {
        x: value_number(&value["x"], 0.0) * scale_x,
        y: value_number(&value["y"], 0.0) * scale_y,
        width: value_number(&value["width"], 0.0) * scale_x,
        height: value_number(&value["height"], 0.0) * scale_y,
    }
}

fn slot_position_json(position: SlotPosition) -> Value {
    json!({
        "x": number_json(position.x),
        "y": number_json(position.y),
        "width": number_json(position.width),
        "height": number_json(position.height)
    })
}

fn template_grid_json(grid: &Grid) -> Value {
    json!({
        "columns": grid.columns,
        "gutter": number_json(grid.gutter),
        "margin": number_json(grid.margin),
        "rowUnit": number_json(grid.row_unit)
    })
}

fn value_string(value: &Value, fallback: &str) -> String {
    value.as_str().unwrap_or(fallback).to_string()
}

fn value_number(value: &Value, fallback: f64) -> f64 {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .unwrap_or(fallback)
}

fn number_json(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or_else(|| Value::from(0))
}

fn fmt_num(value: f64) -> String {
    let mut rounded = if value.is_finite() {
        (value * 100.0).round() / 100.0
    } else {
        0.0
    };
    if rounded == 0.0 {
        rounded = 0.0;
    }
    format!("{rounded:.2}")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn safe_file_stem(value: &str, ordinal: usize) -> String {
    let mut stem = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    while stem.starts_with('.') || stem.starts_with('_') {
        stem.remove(0);
    }
    if stem.is_empty() {
        stem = format!("page-{ordinal}");
    }
    stem
}

fn safe_id(value: &str) -> String {
    let mut id = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if id.is_empty()
        || id
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
    {
        id.insert(0, 'p');
    }
    id
}

fn wireframe_usage() -> &'static str {
    "Use report wireframe export <project-dir-or.pbip> [--format json|svg|html] [--template <name>] [--out <path> | --dry-run] --json."
}

fn wireframe_suggested_command() -> &'static str {
    "powerbi-cli report wireframe export <project-dir-or.pbip> --format svg --dry-run --json"
}

#[cfg(test)]
mod tests {
    use super::{WireframeFormat, finding_location, fmt_num, safe_file_stem, xml_escape};

    #[test]
    fn fixed_decimal_format_is_platform_stable() {
        assert_eq!(fmt_num(24.0), "24.00");
        assert_eq!(fmt_num(24.125), "24.13");
        assert_eq!(fmt_num(-0.0), "0.00");
    }

    #[test]
    fn wireframe_format_accepts_only_json_svg_and_html() {
        assert_eq!(WireframeFormat::parse("svg").unwrap().as_str(), "svg");
        assert_eq!(WireframeFormat::parse("HTML").unwrap().as_str(), "html");
        assert!(WireframeFormat::parse("png").is_err());
    }

    #[test]
    fn finding_locations_follow_report_page_and_visual_pointers() {
        assert_eq!(
            finding_location("/pages/2/visuals/4/position"),
            (Some(2), Some(4))
        );
        assert_eq!(finding_location("/pages/1/filters"), (Some(1), None));
        assert_eq!(finding_location(""), (None, None));
    }

    #[test]
    fn xml_and_file_name_escaping_is_deterministic() {
        assert_eq!(xml_escape("A & <B>"), "A &amp; &lt;B&gt;");
        assert_eq!(safe_file_stem("../Sales Overview", 0), "Sales_Overview");
    }
}
