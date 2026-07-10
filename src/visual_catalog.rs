use crate::feature_catalog::unsupported_feature_error_with_message;
use crate::{CliError, CliResult};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualBindingFamily {
    SingleValue,
    ValuesList,
    CategoryY,
    CategoryShare,
    RowsColumnsValues,
    SlicerField,
    ScatterBubble,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct VisualTypeSpec {
    pub(crate) visual_type: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) family: VisualBindingFamily,
    pub(crate) summary: &'static str,
}

const VISUAL_TYPES: &[VisualTypeSpec] = &[
    VisualTypeSpec {
        visual_type: "card",
        aliases: &["card", "kpi"],
        family: VisualBindingFamily::SingleValue,
        summary: "Single KPI value card; accepts exactly one Values measure binding.",
    },
    VisualTypeSpec {
        visual_type: "tableEx",
        aliases: &["table", "tableex"],
        family: VisualBindingFamily::ValuesList,
        summary: "Table visual; accepts one or more Values bindings.",
    },
    VisualTypeSpec {
        visual_type: "lineChart",
        aliases: &["line", "linechart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Line chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, and an optional Series column.",
    },
    VisualTypeSpec {
        visual_type: "areaChart",
        aliases: &["area", "areachart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Area chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, and an optional Series column.",
    },
    VisualTypeSpec {
        visual_type: "stackedAreaChart",
        aliases: &["stackedarea", "stackedareachart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Stacked area chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, and an optional Series column.",
    },
    VisualTypeSpec {
        visual_type: "clusteredBarChart",
        aliases: &["clusteredbar", "clusteredbarchart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Clustered bar chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, and an optional Series column.",
    },
    VisualTypeSpec {
        visual_type: "clusteredColumnChart",
        aliases: &["clusteredcolumn", "clusteredcolumnchart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Clustered column chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, and an optional Series column.",
    },
    VisualTypeSpec {
        visual_type: "barChart",
        aliases: &["bar", "barchart", "stackedbar", "stackedbarchart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Stacked bar chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, and an optional Series column.",
    },
    VisualTypeSpec {
        visual_type: "columnChart",
        aliases: &[
            "column",
            "columnchart",
            "stackedcolumn",
            "stackedcolumnchart",
        ],
        family: VisualBindingFamily::CategoryY,
        summary: "Stacked column chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, and an optional Series column.",
    },
    VisualTypeSpec {
        visual_type: "scatterChart",
        aliases: &["scatter", "scatterchart", "bubble", "bubblechart"],
        family: VisualBindingFamily::ScatterBubble,
        summary: "Scatter/bubble chart; accepts required X and Y measures plus optional Category, Size measure, Legend, and Tooltips bindings.",
    },
    VisualTypeSpec {
        visual_type: "pieChart",
        aliases: &["pie", "piechart"],
        family: VisualBindingFamily::CategoryShare,
        summary: "Pie chart; accepts exactly one Category column and one or more Y measure bindings, with no Series role.",
    },
    VisualTypeSpec {
        visual_type: "donutChart",
        aliases: &["donut", "donutchart"],
        family: VisualBindingFamily::CategoryShare,
        summary: "Donut chart; accepts exactly one Category column and one or more Y measure bindings, with no Series role.",
    },
    VisualTypeSpec {
        visual_type: "pivotTable",
        aliases: &["matrix", "pivottable"],
        family: VisualBindingFamily::RowsColumnsValues,
        summary: "Matrix visual (PBIR pivotTable); accepts one or more Rows columns, optional Columns columns, and one or more Values measure bindings.",
    },
    VisualTypeSpec {
        visual_type: "slicer",
        aliases: &["slicer"],
        family: VisualBindingFamily::SlicerField,
        summary: "Slicer visual; accepts exactly one Values column. Generated mode is Basic by default or Dropdown when requested.",
    },
];

const TEMPLATE_ONLY_TYPES: &[(&str, &str)] = &[];

const PLANNED_TYPES: &[(&str, &str)] = &[(
    "map",
    "Planned after Desktop-authored PBIR fixtures prove location, latitude/longitude, legend, and size role shapes.",
)];

#[derive(Debug, Default)]
struct CatalogOptions {
    visual_type: Option<String>,
}

pub(crate) fn visual_catalog_command(args: &[String]) -> CliResult<Value> {
    let options = parse_catalog_args(args)?;
    let specs = match options.visual_type.as_deref() {
        Some(value) => vec![lookup_visual_type(value)?],
        None => VISUAL_TYPES.to_vec(),
    };
    Ok(json!({
        "schema": "powerbi-cli.report.visuals.catalog.v1",
        "generatedVisualTypeCount": specs.len(),
        "supportedVisualTypes": specs.iter().map(|spec| spec.visual_type).collect::<Vec<_>>(),
        "visualTypes": specs.iter().map(visual_type_json).collect::<Vec<_>>(),
        "templateOnlyVisualTypes": TEMPLATE_ONLY_TYPES.iter().map(|(visual_type, note)| json!({
            "visualType": visual_type,
            "authoring": "clone-only",
            "note": note
        })).collect::<Vec<_>>(),
        "plannedVisualTypes": PLANNED_TYPES.iter().map(|(visual_type, note)| json!({
            "visualType": visual_type,
            "status": "planned",
            "note": note
        })).collect::<Vec<_>>(),
        "rules": [
            "Generated visuals use a deliberately small PBIR visual.json pattern.",
            "Value-axis roles require measures until a Desktop-authored aggregation binding proves raw-column semantics.",
            "The same model field cannot be projected more than once per visual until Desktop-authored duplicate queryRef numbering is available.",
            "Use `report visuals clone` for Desktop-authored visuals outside this catalog.",
            "Do not infer support for planned or template-only visual types from this catalog."
        ],
        "next": [
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type lineChart --title <title> --binding \"role=Category,table=<table>,column=<column>\" --binding \"role=Y,table=<table>,measure=<measure>\" --dry-run --json",
            "powerbi-cli report visuals clone --project <project-dir-or.pbip> --handle <template-visual-handle> --dry-run --json",
            "powerbi-cli --json capabilities --for \"report visuals add\""
        ]
    }))
}

pub(crate) fn canonical_visual_type(value: &str) -> CliResult<String> {
    lookup_visual_type(value).map(|spec| spec.visual_type.to_string())
}

pub(crate) fn supported_visual_type_names() -> Vec<&'static str> {
    VISUAL_TYPES.iter().map(|spec| spec.visual_type).collect()
}

pub(crate) fn visual_type_contracts() -> Vec<Value> {
    VISUAL_TYPES.iter().map(visual_type_json).collect()
}

pub(crate) fn normalize_role(visual_type: &str, role: &str) -> CliResult<String> {
    let spec = lookup_visual_type(visual_type)?;
    let normalized = role.trim();
    let lower_role = normalized.to_ascii_lowercase();
    let canonical = match spec.family {
        VisualBindingFamily::SingleValue | VisualBindingFamily::ValuesList => {
            match lower_role.as_str() {
                "values" | "value" | "columns" | "field" => Some("Values"),
                _ => None,
            }
        }
        VisualBindingFamily::CategoryY => match lower_role.as_str() {
            "category" | "categories" | "axis" | "x" => Some("Category"),
            "y" | "values" | "value" => Some("Y"),
            "series" | "legend" | "color" | "colour" => Some("Series"),
            _ => None,
        },
        VisualBindingFamily::CategoryShare => match lower_role.as_str() {
            "category" | "categories" | "legend" => Some("Category"),
            "y" | "values" | "value" => Some("Y"),
            _ => None,
        },
        VisualBindingFamily::RowsColumnsValues => match lower_role.as_str() {
            "rows" | "row" => Some("Rows"),
            "columns" | "column" => Some("Columns"),
            "values" | "value" => Some("Values"),
            _ => None,
        },
        VisualBindingFamily::SlicerField => match lower_role.as_str() {
            "values" | "value" | "field" => Some("Values"),
            _ => None,
        },
        VisualBindingFamily::ScatterBubble => match lower_role.as_str() {
            "category" | "categories" | "details" | "detail" | "values" | "value" => {
                Some("Category")
            }
            "x" | "xaxis" | "x-axis" | "x_axis" => Some("X"),
            "y" | "yaxis" | "y-axis" | "y_axis" => Some("Y"),
            "size" | "bubble" | "bubblesize" | "bubble-size" | "bubble_size" => Some("Size"),
            "legend" | "series" | "color" | "colour" => Some("Legend"),
            "tooltip" | "tooltips" => Some("Tooltips"),
            _ => None,
        },
    };
    canonical.map(ToOwned::to_owned).ok_or_else(|| {
        CliError::unsupported_feature(format!(
            "unsupported role {role} for visual type {}",
            spec.visual_type
        ))
        .with_hint(format!(
            "Supported roles for {} are: {}.",
            spec.visual_type,
            role_names(spec.family).join(", ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli report visuals catalog --visual-type {} --json",
            spec.visual_type
        ))
    })
}

pub(crate) fn binding_family(visual_type: &str) -> CliResult<VisualBindingFamily> {
    lookup_visual_type(visual_type).map(|spec| spec.family)
}

pub(crate) fn column_binding_is_proven(visual_type: &str, role: &str) -> CliResult<bool> {
    let family = binding_family(visual_type)?;
    Ok(!matches!(
        (family, role),
        (VisualBindingFamily::SingleValue, "Values")
            | (VisualBindingFamily::CategoryY, "Y")
            | (VisualBindingFamily::CategoryShare, "Y")
            | (VisualBindingFamily::RowsColumnsValues, "Values")
            | (VisualBindingFamily::ScatterBubble, "X" | "Y" | "Size")
    ))
}

pub(crate) fn catalog_hint() -> String {
    format!(
        "Generated visual creation supports: {}. Run `powerbi-cli report visuals catalog --json` for roles and aliases.",
        supported_visual_type_names().join(", ")
    )
}

fn parse_catalog_args(args: &[String]) -> CliResult<CatalogOptions> {
    let mut options = CatalogOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--visual-type" | "--visualType" | "--type" => {
                options.visual_type = Some(take_value(args, &mut i, "--visual-type")?);
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals catalog flag: {other}"
                ))
                .with_hint("Run `powerbi-cli report visuals catalog --json`.")
                .with_suggested_command("powerbi-cli report visuals catalog --json"));
            }
        }
    }
    Ok(options)
}

fn lookup_visual_type(value: &str) -> CliResult<VisualTypeSpec> {
    let normalized = normalize_key(value);
    VISUAL_TYPES
        .iter()
        .copied()
        .find(|spec| {
            normalize_key(spec.visual_type) == normalized
                || spec
                    .aliases
                    .iter()
                    .any(|alias| normalize_key(alias) == normalized)
        })
        .ok_or_else(|| unsupported_visual_type_error(value, &normalized))
}

fn unsupported_visual_type_error(value: &str, normalized: &str) -> CliError {
    if TEMPLATE_ONLY_TYPES
        .iter()
        .any(|(visual_type, _)| normalize_key(visual_type) == normalized)
        || PLANNED_TYPES
            .iter()
            .any(|(visual_type, _)| normalize_key(visual_type) == normalized)
    {
        unsupported_feature_error_with_message(
            "report.visuals.planned-types",
            format!("unsupported visual type for generated report visuals: {value}"),
        )
        .with_hint(format!(
            "{} Use `report visuals clone` for Desktop-authored template visuals, or add a Desktop-authored golden fixture before generated support.",
            catalog_hint()
        ))
        .with_suggested_command("powerbi-cli report visuals catalog --json")
    } else {
        CliError::invalid_args(format!(
            "unknown visual type for generated report visuals: {value}"
        ))
        .with_hint(catalog_hint())
        .with_suggested_command("powerbi-cli report visuals catalog --json")
    }
}

fn visual_type_json(spec: &VisualTypeSpec) -> Value {
    json!({
        "visualType": spec.visual_type,
        "aliases": spec.aliases,
        "generatedBy": "report visuals add",
        "bindingFamily": binding_family_name(spec.family),
        "proofLevel": "desktop-golden-pending",
        "bindingProofLevel": binding_proof_level(spec.family),
        "proofNote": "The binding family retains its recorded proof, but the current title-bearing generated visual bytes await Desktop open/refresh/save re-verification.",
        "summary": spec.summary,
        "roles": role_specs_json(spec.family),
        "examples": example_commands(spec),
        "limitations": [
            "Generated PBIR is a minimal visual container plus queryState.",
            "Raw columns are refused in value-axis roles until Desktop-authored aggregation-binding fixtures exist.",
            "Repeated use of one model field is refused until Desktop-authored duplicate queryRef numbering is available.",
            "Use formatting bundles, themes, or cloned Desktop-authored templates for style beyond generated defaults."
        ]
    })
}

fn role_specs_json(family: VisualBindingFamily) -> Value {
    match family {
        VisualBindingFamily::SingleValue => json!([
            {
                "role": "Values",
                "required": false,
                "min": 0,
                "max": 1,
                "fieldKinds": ["measure"],
                "aliases": ["values", "value", "field"],
                "summary": "Optional for placeholders; exactly one measure binding when bound."
            }
        ]),
        VisualBindingFamily::ValuesList => json!([
            {
                "role": "Values",
                "required": false,
                "min": 0,
                "max": null,
                "fieldKinds": ["column", "measure"],
                "aliases": ["values", "value", "columns", "field"],
                "summary": "Optional for placeholders; one or more bindings when bound."
            }
        ]),
        VisualBindingFamily::CategoryY => json!([
            {
                "role": "Category",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["column"],
                "aliases": ["category", "categories", "axis", "x"],
                "summary": "Axis/category columns. Multiple projections become a hierarchy axis for Desktop drill up/down."
            },
            {
                "role": "Y",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["measure"],
                "aliases": ["y", "values", "value", "series"],
                "summary": "One or more measure bindings; raw columns require an unproven aggregation shape and are refused."
            },
            {
                "role": "Series",
                "required": false,
                "min": 0,
                "max": 1,
                "fieldKinds": ["column"],
                "aliases": ["series", "legend", "color", "colour"],
                "summary": "Optional legend/series grouping column."
            }
        ]),
        VisualBindingFamily::CategoryShare => json!([
            {
                "role": "Category",
                "required": true,
                "min": 1,
                "max": 1,
                "fieldKinds": ["column"],
                "aliases": ["category", "categories", "legend"],
                "summary": "Exactly one category column. The generated projection is active."
            },
            {
                "role": "Y",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["measure"],
                "aliases": ["y", "values", "value"],
                "summary": "One or more measure bindings; the first Y measure drives the default descending sort."
            }
        ]),
        VisualBindingFamily::RowsColumnsValues => json!([
            {
                "role": "Rows",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["column"],
                "aliases": ["rows", "row"],
                "summary": "One or more row hierarchy columns in drill order."
            },
            {
                "role": "Columns",
                "required": false,
                "min": 0,
                "max": null,
                "fieldKinds": ["column"],
                "aliases": ["columns", "column"],
                "summary": "Optional column hierarchy columns in drill order."
            },
            {
                "role": "Values",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["measure"],
                "aliases": ["values", "value"],
                "summary": "One or more matrix measures; raw value columns are refused pending aggregation-binding proof."
            }
        ]),
        VisualBindingFamily::SlicerField => json!([
            {
                "role": "Values",
                "required": true,
                "min": 1,
                "max": 1,
                "fieldKinds": ["column"],
                "aliases": ["values", "value", "field"],
                "summary": "Exactly one slicer field column; measures are refused."
            }
        ]),
        VisualBindingFamily::ScatterBubble => json!([
            {
                "role": "X",
                "required": true,
                "min": 1,
                "max": 1,
                "fieldKinds": ["measure"],
                "aliases": ["x", "xAxis"],
                "summary": "Continuous X-axis measure."
            },
            {
                "role": "Y",
                "required": true,
                "min": 1,
                "max": 1,
                "fieldKinds": ["measure"],
                "aliases": ["y", "yAxis"],
                "summary": "Continuous Y-axis measure."
            },
            {
                "role": "Category",
                "required": false,
                "min": 0,
                "max": 1,
                "fieldKinds": ["column"],
                "aliases": ["category", "details", "values"],
                "summary": "Optional bubble identity/detail column."
            },
            {
                "role": "Size",
                "required": false,
                "min": 0,
                "max": 1,
                "fieldKinds": ["measure"],
                "aliases": ["size", "bubbleSize"],
                "summary": "Optional bubble-size measure."
            },
            {
                "role": "Legend",
                "required": false,
                "min": 0,
                "max": 1,
                "fieldKinds": ["column"],
                "aliases": ["legend", "series", "color", "colour"],
                "summary": "Optional color grouping column."
            },
            {
                "role": "Tooltips",
                "required": false,
                "min": 0,
                "max": null,
                "fieldKinds": ["column", "measure"],
                "aliases": ["tooltip", "tooltips"],
                "summary": "Optional fields shown in tooltips."
            }
        ]),
    }
}

fn example_commands(spec: &VisualTypeSpec) -> Vec<String> {
    match spec.family {
        VisualBindingFamily::SingleValue => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
            spec.visual_type
        )],
        VisualBindingFamily::ValuesList => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Values,table=<table>,column=<column>\" --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json",
            spec.visual_type
        )],
        VisualBindingFamily::CategoryY => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Category,table=<table>,column=<column>\" --binding \"role=Y,table=<table>,measure=<measure>\" --dry-run --json",
            spec.visual_type
        )],
        VisualBindingFamily::CategoryShare => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Category,table=<table>,column=<column>\" --binding \"role=Y,table=<table>,measure=<measure>\" --dry-run --json",
            spec.visual_type
        )],
        VisualBindingFamily::RowsColumnsValues => vec![
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type matrix --title <title> --binding \"role=Rows,table=<table>,column=<row-column>\" --binding \"role=Columns,table=<table>,column=<column-column>\" --binding \"role=Values,table=<table>,measure=<measure>\" --dry-run --json".to_string(),
        ],
        VisualBindingFamily::SlicerField => vec![
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type slicer --mode basic --title <title> --binding \"role=Values,table=<table>,column=<column>\" --dry-run --json".to_string(),
        ],
        VisualBindingFamily::ScatterBubble => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Category,table=<table>,column=<detail-column>\" --binding \"role=X,table=<table>,measure=<x-measure>\" --binding \"role=Y,table=<table>,measure=<y-measure>\" --binding \"role=Size,table=<table>,measure=<size-measure>\" --dry-run --json",
            spec.visual_type
        )],
    }
}

fn role_names(family: VisualBindingFamily) -> Vec<&'static str> {
    match family {
        VisualBindingFamily::SingleValue | VisualBindingFamily::ValuesList => vec!["Values"],
        VisualBindingFamily::CategoryY => vec!["Category", "Y", "Series"],
        VisualBindingFamily::CategoryShare => vec!["Category", "Y"],
        VisualBindingFamily::RowsColumnsValues => vec!["Rows", "Columns", "Values"],
        VisualBindingFamily::SlicerField => vec!["Values"],
        VisualBindingFamily::ScatterBubble => {
            vec!["Category", "X", "Y", "Size", "Legend", "Tooltips"]
        }
    }
}

fn binding_family_name(family: VisualBindingFamily) -> &'static str {
    match family {
        VisualBindingFamily::SingleValue => "singleValue",
        VisualBindingFamily::ValuesList => "valuesList",
        VisualBindingFamily::CategoryY => "categoryY",
        VisualBindingFamily::CategoryShare => "categoryShare",
        VisualBindingFamily::RowsColumnsValues => "rowsColumnsValues",
        VisualBindingFamily::SlicerField => "slicerField",
        VisualBindingFamily::ScatterBubble => "scatterBubble",
    }
}

fn binding_proof_level(family: VisualBindingFamily) -> &'static str {
    match family {
        VisualBindingFamily::CategoryShare
        | VisualBindingFamily::RowsColumnsValues
        | VisualBindingFamily::SlicerField => "manual-desktop-canvas-refresh",
        _ => "unit-smoke",
    }
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value"))
            .with_hint("Run `powerbi-cli report visuals catalog --json`.")
            .with_suggested_command("powerbi-cli report visuals catalog --json")
    })?;
    *index += 2;
    Ok(value.clone())
}
