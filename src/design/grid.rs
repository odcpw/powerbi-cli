//! Deterministic twelve-column page grid and named template resolver.

use crate::{CliError, CliResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const TEMPLATE_DATA: &str = include_str!("templates.json");
const REFERENCE_WIDTH: f64 = 1280.0;
const REFERENCE_HEIGHT: f64 = 720.0;

/// The two page-size presets used by the design system.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageSize {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl PageSize {
    pub(crate) const STANDARD: Self = Self {
        width: REFERENCE_WIDTH,
        height: REFERENCE_HEIGHT,
    };

    pub(crate) const WIDE: Self = Self {
        width: 1920.0,
        height: 1080.0,
    };

    pub(crate) fn preset(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "1280x720" | "standard" | "default" | "hd" => Some(Self::STANDARD),
            "1920x1080" | "wide" | "full-hd" | "fullhd" => Some(Self::WIDE),
            _ => None,
        }
    }

    fn validate(self) -> CliResult<Self> {
        if !self.width.is_finite() || !self.height.is_finite() {
            return Err(CliError::invalid_args(
                "layout page width and height must be finite numbers",
            )
            .with_pointer("/pageSize"));
        }
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err(CliError::invalid_args(
                "layout page width and height must be positive numbers",
            )
            .with_pointer("/pageSize"));
        }
        Ok(self)
    }
}

/// Grid dimensions are expressed at the 1280x720 reference size and scale
/// proportionally for the wide preset.  `columns` stays twelve by design,
/// while accepting a smaller value internally makes diagnostics useful for
/// malformed template data and future experimental catalogs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Grid {
    pub(crate) columns: u32,
    pub(crate) gutter: f64,
    pub(crate) margin: f64,
    pub(crate) row_unit: f64,
}

impl Default for Grid {
    fn default() -> Self {
        Self {
            columns: 12,
            gutter: 16.0,
            margin: 24.0,
            row_unit: 8.0,
        }
    }
}

impl Grid {
    pub(crate) fn validate(self) -> CliResult<Self> {
        if self.columns != 12 {
            return Err(CliError::invalid_args(format!(
                "layout grid requires exactly 12 columns; got {}",
                self.columns
            ))
            .with_pointer("/grid/columns")
            .with_hint(
                "Use the twelve-column design grid; gutter, margin, and rowUnit are configurable.",
            ));
        }
        for (name, value) in [
            ("gutter", self.gutter),
            ("margin", self.margin),
            ("rowUnit", self.row_unit),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(CliError::invalid_args(format!(
                    "layout grid {name} must be a finite nonnegative number"
                ))
                .with_pointer(format!("/grid/{name}")));
            }
        }
        if self.row_unit <= 0.0 {
            return Err(
                CliError::invalid_args("layout grid rowUnit must be greater than zero")
                    .with_pointer("/grid/rowUnit"),
            );
        }
        Ok(self)
    }
}

/// A named side reserved for a slicer rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RailSide {
    #[serde(alias = "rail-left")]
    Left,
    #[serde(alias = "rail-right")]
    Right,
}

impl RailSide {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeadingBand {
    pub(crate) row: u32,
    pub(crate) row_span: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Slot {
    pub(crate) name: String,
    pub(crate) col: u32,
    pub(crate) row: u32,
    pub(crate) col_span: u32,
    pub(crate) row_span: u32,
    pub(crate) preferred_families: Vec<String>,
    #[serde(default)]
    pub(crate) min_family: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Template {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) rail: Option<RailSide>,
    #[serde(default)]
    pub(crate) heading_band: Option<HeadingBand>,
    #[serde(default)]
    pub(crate) budget: Option<usize>,
    pub(crate) slots: Vec<Slot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TemplateCatalog {
    pub(crate) schema: String,
    pub(crate) grid: Grid,
    pub(crate) templates: Vec<Template>,
}

/// Coordinates resolved from one slot.  Values are rounded to hundredths so
/// output is byte-stable across platforms and easy for agents to compare.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SlotPosition {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Debug, Clone, Copy)]
struct MinimumSize {
    width: f64,
    height: f64,
}

/// Load the embedded, offline template catalog.
pub(crate) fn catalog() -> CliResult<TemplateCatalog> {
    let catalog: TemplateCatalog = serde_json::from_str(TEMPLATE_DATA).map_err(|error| {
        CliError::unexpected(format!(
            "embedded design template catalog is invalid: {error}"
        ))
    })?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}

#[allow(dead_code)]
pub(crate) fn catalog_json() -> CliResult<Value> {
    serde_json::to_value(catalog()?).map_err(|error| {
        CliError::unexpected(format!("serialize design template catalog: {error}"))
    })
}

pub(crate) fn template(name: &str) -> CliResult<Template> {
    let catalog = catalog()?;
    let normalized = name.trim().to_ascii_lowercase();
    if let Some(index) = catalog
        .templates
        .iter()
        .position(|candidate| candidate.name.to_ascii_lowercase() == normalized)
    {
        return Ok(catalog.templates[index].clone());
    }
    Err({
        let names = catalog
            .templates
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        CliError::invalid_args(format!("unknown layout template: {name}"))
                .with_pointer("/template")
                .with_hint(format!("Choose one of the named templates: {names}."))
                .with_suggested_command(
                    "powerbi-cli report layout auto --project <project-dir-or.pbip> --template overview --dry-run --json",
                )
    })
}

/// Resolve a template using the default twelve-column grid.
pub(crate) fn resolve(
    template: &Template,
    page_size: PageSize,
    rail: Option<RailSide>,
) -> CliResult<BTreeMap<String, SlotPosition>> {
    resolve_with_grid(template, page_size, Grid::default(), rail)
}

/// Resolve a template with an explicit grid override.  The optional rail is
/// an override for the template metadata; passing `None` uses the template's
/// declared rail side.
pub(crate) fn resolve_with_grid(
    template: &Template,
    page_size: PageSize,
    grid: Grid,
    rail: Option<RailSide>,
) -> CliResult<BTreeMap<String, SlotPosition>> {
    let page_size = page_size.validate()?;
    let grid = grid.validate()?;
    validate_template(template)?;

    let scale_x = page_size.width / REFERENCE_WIDTH;
    let scale_y = page_size.height / REFERENCE_HEIGHT;
    let margin_x = grid.margin * scale_x;
    let margin_y = grid.margin * scale_y;
    let gutter_x = grid.gutter * scale_x;
    let row_unit = grid.row_unit * scale_y;
    let columns = grid.columns as f64;
    let column_width = (page_size.width - margin_x * 2.0 - gutter_x * (columns - 1.0)) / columns;
    if !column_width.is_finite() || column_width <= 0.0 {
        return Err(
            CliError::invalid_args("layout grid leaves no usable page width").with_pointer("/grid"),
        );
    }

    // A template's rail is data-driven, but callers may mirror a rail-bearing
    // template for a right-to-left locale or an alternate page convention.
    // Mirroring every slot across the twelve-column axis keeps content and the
    // rail aligned without introducing a second layout algorithm.
    let mirror = matches!(
        (template.rail, rail),
        (Some(RailSide::Left), Some(RailSide::Right))
            | (Some(RailSide::Right), Some(RailSide::Left))
    );
    let mut resolved = BTreeMap::new();
    for (slot_index, slot) in template.slots.iter().enumerate() {
        let col = if mirror {
            grid.columns
                .saturating_sub(slot.col.saturating_add(slot.col_span))
        } else {
            slot.col
        };
        let x = margin_x + col as f64 * (column_width + gutter_x);
        let y = margin_y + slot.row as f64 * row_unit;
        let width =
            slot.col_span as f64 * column_width + slot.col_span.saturating_sub(1) as f64 * gutter_x;
        let height = slot.row_span as f64 * row_unit;
        let position = SlotPosition {
            x: round(x),
            y: round(y),
            width: round(width),
            height: round(height),
        };
        validate_position(slot_index, slot, position, page_size, margin_x, margin_y)?;
        if resolved.insert(slot.name.clone(), position).is_some() {
            return Err(CliError::invalid_args(format!(
                "layout template {} repeats slot name {}",
                template.name, slot.name
            ))
            .with_pointer(format!("/template/slots/{slot_index}/name")));
        }
    }
    Ok(resolved)
}

pub(crate) fn content_slots(template: &Template) -> impl Iterator<Item = &Slot> {
    template.slots.iter().filter(|slot| {
        !matches!(slot.name.as_str(), "heading" | "rail")
            && !template
                .heading_band
                .as_ref()
                .is_some_and(|band| slot.row == band.row && slot.row_span == band.row_span)
    })
}

/// Return the vertical and horizontal guide coordinates for a resolved
/// template.  Keeping guide math beside [`resolve_with_grid`] ensures a
/// wireframe cannot drift from the coordinates used by auto-layout.
pub(crate) fn guide_lines(
    template: &Template,
    page_size: PageSize,
    grid: Grid,
) -> CliResult<(Vec<f64>, Vec<f64>)> {
    let page_size = page_size.validate()?;
    let grid = grid.validate()?;
    validate_template(template)?;
    let scale_x = page_size.width / REFERENCE_WIDTH;
    let scale_y = page_size.height / REFERENCE_HEIGHT;
    let margin_x = grid.margin * scale_x;
    let margin_y = grid.margin * scale_y;
    let gutter_x = grid.gutter * scale_x;
    let row_unit = grid.row_unit * scale_y;
    let columns = grid.columns as f64;
    let column_width = (page_size.width - margin_x * 2.0 - gutter_x * (columns - 1.0)) / columns;
    if !column_width.is_finite() || column_width <= 0.0 {
        return Err(
            CliError::invalid_args("layout grid leaves no usable page width").with_pointer("/grid"),
        );
    }

    let mut vertical = Vec::with_capacity(grid.columns as usize + 1);
    for column in 0..=grid.columns {
        let x = if column == grid.columns {
            page_size.width - margin_x
        } else {
            margin_x + column as f64 * (column_width + gutter_x)
        };
        vertical.push(round(x));
    }
    let max_row = template
        .slots
        .iter()
        .map(|slot| slot.row.saturating_add(slot.row_span))
        .max()
        .unwrap_or(0);
    let mut horizontal = Vec::with_capacity(max_row as usize + 1);
    for row in 0..=max_row {
        horizontal.push(round(margin_y + row as f64 * row_unit));
    }
    Ok((vertical, horizontal))
}

fn validate_catalog(catalog: &TemplateCatalog) -> CliResult<()> {
    if catalog.schema != "powerbi-cli.design.templates.v1" {
        return Err(CliError::unexpected(format!(
            "embedded design template catalog has unsupported schema {}",
            catalog.schema
        )));
    }
    catalog.grid.validate()?;
    if catalog.templates.len() != 11 {
        return Err(CliError::unexpected(format!(
            "embedded design template catalog must contain eleven templates; got {}",
            catalog.templates.len()
        )));
    }
    for template in &catalog.templates {
        validate_template(template)?;
        resolve_with_grid(template, PageSize::STANDARD, catalog.grid, None)?;
        resolve_with_grid(template, PageSize::WIDE, catalog.grid, None)?;
    }
    Ok(())
}

fn validate_template(template: &Template) -> CliResult<()> {
    if template.name.trim().is_empty() {
        return Err(
            CliError::invalid_args("layout template name must not be empty")
                .with_pointer("/template/name"),
        );
    }
    if template.slots.is_empty() {
        return Err(CliError::invalid_args(format!(
            "layout template {} must define at least one slot",
            template.name
        ))
        .with_pointer("/template/slots"));
    }
    let mut names = BTreeMap::new();
    for (index, slot) in template.slots.iter().enumerate() {
        let pointer = format!("/template/slots/{index}");
        if slot.name.trim().is_empty() {
            return Err(CliError::invalid_args("layout slot name must not be empty")
                .with_pointer(format!("{pointer}/name")));
        }
        if names.insert(slot.name.clone(), index).is_some() {
            return Err(CliError::invalid_args(format!(
                "layout template {} repeats slot name {}",
                template.name, slot.name
            ))
            .with_pointer(format!("{pointer}/name")));
        }
        if slot.col_span == 0 || slot.row_span == 0 {
            return Err(CliError::invalid_args(format!(
                "layout slot {} must have positive colSpan and rowSpan",
                slot.name
            ))
            .with_pointer(pointer));
        }
        if slot.col.saturating_add(slot.col_span) > 12 {
            return Err(CliError::invalid_args(format!(
                "layout slot {} extends beyond the twelve-column grid",
                slot.name
            ))
            .with_pointer(format!("{pointer}/colSpan")));
        }
        if slot.preferred_families.is_empty() {
            return Err(CliError::invalid_args(format!(
                "layout slot {} must name at least one preferred visual family",
                slot.name
            ))
            .with_pointer(format!("{pointer}/preferredFamilies")));
        }
        if let Some(min_family) = slot.min_family.as_deref()
            && minimum_size(min_family).is_none()
        {
            return Err(CliError::invalid_args(format!(
                "layout slot {} has unknown minFamily {}",
                slot.name, min_family
            ))
            .with_pointer(format!("{pointer}/minFamily")));
        }
    }
    for (left_index, left) in template.slots.iter().enumerate() {
        for (right_index, right) in template.slots.iter().enumerate().skip(left_index + 1) {
            if rectangles_overlap(left, right) {
                return Err(CliError::invalid_args(format!(
                    "layout template {} overlaps slots {} and {}",
                    template.name, left.name, right.name
                ))
                .with_pointer(format!("/template/slots/{right_index}")));
            }
        }
    }
    if let Some(band) = &template.heading_band
        && band.row_span == 0
    {
        return Err(CliError::invalid_args(format!(
            "layout template {} heading band must have positive rowSpan",
            template.name
        ))
        .with_pointer("/template/headingBand/rowSpan"));
    }
    Ok(())
}

fn rectangles_overlap(left: &Slot, right: &Slot) -> bool {
    left.col < right.col.saturating_add(right.col_span)
        && right.col < left.col.saturating_add(left.col_span)
        && left.row < right.row.saturating_add(right.row_span)
        && right.row < left.row.saturating_add(left.row_span)
}

fn validate_position(
    slot_index: usize,
    slot: &Slot,
    position: SlotPosition,
    page_size: PageSize,
    margin_x: f64,
    margin_y: f64,
) -> CliResult<()> {
    let epsilon = 0.01;
    if position.x < margin_x - epsilon
        || position.y < margin_y - epsilon
        || position.x + position.width > page_size.width - margin_x + epsilon
        || position.y + position.height > page_size.height - margin_y + epsilon
    {
        return Err(CliError::invalid_args(format!(
            "layout slot {} is outside the page content bounds",
            slot.name
        ))
        .with_pointer(format!("/template/slots/{slot_index}")));
    }
    if let Some(min_family) = slot.min_family.as_deref()
        && let Some(minimum) = minimum_size(min_family)
        && (position.width + epsilon < minimum.width || position.height + epsilon < minimum.height)
    {
        return Err(CliError::invalid_args(format!(
            "layout slot {} is too small for minFamily {} ({}x{} required; got {}x{})",
            slot.name,
            min_family,
            trim_number(minimum.width),
            trim_number(minimum.height),
            trim_number(position.width),
            trim_number(position.height)
        ))
        .with_pointer(format!("/template/slots/{slot_index}/minFamily")));
    }
    Ok(())
}

fn minimum_size(family: &str) -> Option<MinimumSize> {
    let family = family.trim().to_ascii_lowercase();
    let size = match family.as_str() {
        "card" | "kpi" => MinimumSize {
            width: 160.0,
            height: 72.0,
        },
        "chart"
        | "linechart"
        | "areachart"
        | "barchart"
        | "columnchart"
        | "combochart"
        | "clusteredbarchart"
        | "clusteredcolumnchart" => MinimumSize {
            width: 240.0,
            height: 160.0,
        },
        "scatter" | "scatterchart" => MinimumSize {
            width: 320.0,
            height: 220.0,
        },
        "table" | "tableex" => MinimumSize {
            width: 320.0,
            height: 180.0,
        },
        "matrix" | "pivottable" => MinimumSize {
            width: 360.0,
            height: 220.0,
        },
        // Power BI's regular slicer floor is 76 px high; a Between/range
        // slicer needs 104 px for both handles and the draggable band.  Keep
        // the width floor modest because the Desktop validator gates the
        // height, not a fixed column width.
        "slicer" => MinimumSize {
            width: 76.0,
            height: 76.0,
        },
        "slicerbetween" | "slicer-between" => MinimumSize {
            width: 76.0,
            height: 104.0,
        },
        "textbox" | "text" => MinimumSize {
            width: 120.0,
            height: 40.0,
        },
        _ => return None,
    };
    Some(size)
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn trim_number(value: f64) -> String {
    let rounded = round(value);
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_eleven_templates_with_valid_both_size_resolutions() {
        let catalog = catalog().expect("embedded catalog");
        assert_eq!(catalog.templates.len(), 11);
        for template in &catalog.templates {
            let standard = resolve(template, PageSize::STANDARD, None).expect("standard layout");
            let wide = resolve(template, PageSize::WIDE, None).expect("wide layout");
            assert_eq!(standard.len(), template.slots.len());
            assert_eq!(wide.len(), template.slots.len());
        }
    }

    #[test]
    fn resolver_is_deterministic_and_scales_coordinates_proportionally() {
        let template = template("overview").expect("overview template");
        let first = resolve(&template, PageSize::WIDE, None).expect("first resolution");
        let second = resolve(&template, PageSize::WIDE, None).expect("second resolution");
        assert_eq!(first, second);
        let standard = resolve(&template, PageSize::STANDARD, None).expect("standard resolution");
        let heading = standard["heading"];
        let wide_heading = first["heading"];
        assert_eq!(heading.x * 1.5, wide_heading.x);
        assert_eq!(heading.y * 1.5, wide_heading.y);
        assert_eq!(heading.width * 1.5, wide_heading.width);
        assert_eq!(heading.height * 1.5, wide_heading.height);
    }

    #[test]
    fn resolver_mirrors_a_declared_rail_when_the_side_is_overridden() {
        let template = template("overview").expect("overview template");
        let left = resolve(&template, PageSize::STANDARD, None).expect("left rail");
        let right =
            resolve(&template, PageSize::STANDARD, Some(RailSide::Right)).expect("right rail");
        assert_eq!(left["rail"].width, right["rail"].width);
        assert_eq!(
            left["rail"].x + right["rail"].x + left["rail"].width,
            1280.0
        );
        assert!(right["rail"].x > left["rail"].x);
        assert!(right["heading"].x < left["heading"].x);
    }

    #[test]
    fn resolver_rejects_overlapping_slots_with_pointer() {
        let template = Template {
            name: "broken".to_string(),
            rail: None,
            heading_band: None,
            budget: None,
            slots: vec![
                Slot {
                    name: "a".to_string(),
                    col: 0,
                    row: 0,
                    col_span: 2,
                    row_span: 2,
                    preferred_families: vec!["card".to_string()],
                    min_family: Some("card".to_string()),
                },
                Slot {
                    name: "b".to_string(),
                    col: 1,
                    row: 1,
                    col_span: 2,
                    row_span: 2,
                    preferred_families: vec!["card".to_string()],
                    min_family: Some("card".to_string()),
                },
            ],
        };
        let error = resolve(&template, PageSize::STANDARD, None).expect_err("overlap");
        assert_eq!(error.pointer(), Some("/template/slots/1"));
        assert!(error.message.contains("overlaps"));
    }

    #[test]
    fn resolver_rejects_slot_that_is_smaller_than_its_family_minimum() {
        let template = Template {
            name: "broken".to_string(),
            rail: None,
            heading_band: None,
            budget: None,
            slots: vec![Slot {
                name: "chart".to_string(),
                col: 0,
                row: 0,
                col_span: 1,
                row_span: 1,
                preferred_families: vec!["lineChart".to_string()],
                min_family: Some("chart".to_string()),
            }],
        };
        let error = resolve(&template, PageSize::STANDARD, None).expect_err("minimum");
        assert_eq!(error.pointer(), Some("/template/slots/0/minFamily"));
        assert!(error.message.contains("too small"));
    }
}
