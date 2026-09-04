use crate::feature_catalog::unsupported_feature_error_with_message;
use crate::{CliError, CliResult};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualBindingFamily {
    SingleValue,
    ValuesList,
    CategoryY,
    CategorySeriesYAggregatable,
    ComboCategoryY,
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
        summary: "Line chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, an optional Series column, and Tooltips.",
    },
    VisualTypeSpec {
        visual_type: "areaChart",
        aliases: &["area", "areachart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Area chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, an optional Series column, and Tooltips.",
    },
    VisualTypeSpec {
        visual_type: "stackedAreaChart",
        aliases: &["stackedarea", "stackedareachart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Stacked area chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, an optional Series column, and Tooltips.",
    },
    VisualTypeSpec {
        visual_type: "clusteredBarChart",
        aliases: &["clusteredbar", "clusteredbarchart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Clustered bar chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, an optional Series column, and Tooltips.",
    },
    VisualTypeSpec {
        visual_type: "clusteredColumnChart",
        aliases: &["clusteredcolumn", "clusteredcolumnchart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Clustered column chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, an optional Series column, and Tooltips.",
    },
    VisualTypeSpec {
        visual_type: "barChart",
        aliases: &["bar", "barchart", "stackedbar", "stackedbarchart"],
        family: VisualBindingFamily::CategoryY,
        summary: "Stacked bar chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, an optional Series column, and Tooltips.",
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
        summary: "Stacked column chart; accepts one or more Category columns for hierarchy axes, one or more Y measure bindings, an optional Series column, and Tooltips.",
    },
    VisualTypeSpec {
        visual_type: "hundredPercentStackedColumnChart",
        aliases: &[
            "hundredpercentstackedcolumn",
            "hundredpercentstackedcolumnchart",
            "100percentstackedcolumn",
            "100percentstackedcolumnchart",
        ],
        family: VisualBindingFamily::CategorySeriesYAggregatable,
        summary: "100% stacked column chart; accepts Category columns, an optional Series column, and Y measures or summed columns.",
    },
    VisualTypeSpec {
        visual_type: "lineClusteredColumnComboChart",
        aliases: &[
            "combo",
            "combochart",
            "lineclusteredcolumn",
            "lineclusteredcolumnchart",
            "lineandclusteredcolumn",
        ],
        family: VisualBindingFamily::ComboCategoryY,
        summary: "Line and clustered column combo chart; accepts one or more Category columns, one or more column-axis Y measures, one or more line-axis Y2 measures, and optional Tooltips.",
    },
    VisualTypeSpec {
        visual_type: "scatterChart",
        aliases: &["scatter", "scatterchart", "bubble", "bubblechart"],
        family: VisualBindingFamily::ScatterBubble,
        summary: "Scatter/bubble chart; accepts required X and Y measures or summed columns plus optional Category, Size measure or summed column, Series, and Tooltips bindings.",
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
        summary: "Slicer visual; accepts exactly one Values column. Generated mode is Basic by default, Dropdown for a compact selector, or Between for a numeric/date range slider.",
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
        "schema": "powerbi-cli.report.visuals.catalog.v2",
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
        "rules": specs.iter().map(visual_type_rules_json).collect::<Vec<_>>(),
        "catalogNotes": [
            "Generated visuals use a deliberately small PBIR visual.json pattern.",
            "Scatter X/Y/Size and 100% stacked-column Y accept columns by emitting the Desktop-proven explicit Sum aggregation shape; other value-axis columns require measures.",
            "The same model field cannot be projected more than once per visual until Desktop-authored duplicate queryRef numbering is available.",
            "Use `report visuals clone` for Desktop-authored visuals outside this catalog.",
            "Do not infer support for planned or template-only visual types from this catalog."
        ],
        "next": [
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type lineChart --title <title> --binding \"role=Category,table=<table>,column=<column>\" --binding \"role=Y,table=<table>,measure=<measure>\" --dry-run --json",
            "powerbi-cli report visuals repair-bindings --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json",
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

pub(crate) fn schema_golden_visual_type_names() -> Vec<&'static str> {
    VISUAL_TYPES
        .iter()
        .map(|spec| spec.visual_type)
        .filter(|visual_type| pilot_2026_08_schema_golden(visual_type))
        .collect()
}

pub(crate) fn visual_type_contracts() -> Vec<Value> {
    VISUAL_TYPES.iter().map(visual_type_json).collect()
}

pub(crate) fn visual_type_role_rules() -> Vec<Value> {
    VISUAL_TYPES.iter().map(visual_type_rules_json).collect()
}

pub(crate) fn visual_type_role_rule(visual_type: &str) -> CliResult<Value> {
    lookup_visual_type(visual_type).map(|spec| visual_type_rules_json(&spec))
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
            "tooltip" | "tooltips" => Some("Tooltips"),
            _ => None,
        },
        VisualBindingFamily::CategorySeriesYAggregatable => match lower_role.as_str() {
            "category" | "categories" | "axis" | "x" => Some("Category"),
            "y" | "values" | "value" => Some("Y"),
            "series" | "legend" | "color" | "colour" => Some("Series"),
            _ => None,
        },
        VisualBindingFamily::ComboCategoryY => match lower_role.as_str() {
            "category" | "categories" | "axis" | "x" => Some("Category"),
            "y" | "column" | "columns" | "columny" | "column-y" | "column_y" | "columnvalues"
            | "column-values" | "column_values" => Some("Y"),
            "y2" | "line" | "lines" | "liney" | "line-y" | "line_y" | "linevalues"
            | "line-values" | "line_values" => Some("Y2"),
            "tooltip" | "tooltips" => Some("Tooltips"),
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
        VisualBindingFamily::ScatterBubble
            if matches!(lower_role.as_str(), "details" | "detail") =>
        {
            return Err(CliError::unsupported_feature(
                "scatterChart does not support the Details role",
            )
            .with_hint(
                "Power BI Desktop uses the Category PBIR role for scatter detail identity; use role=Category instead of Details.",
            )
            .with_suggested_command(
                "powerbi-cli report visuals catalog --visual-type scatterChart --json",
            ));
        }
        VisualBindingFamily::ScatterBubble => match lower_role.as_str() {
            "category" | "categories" | "values" | "value" => Some("Category"),
            "x" | "xaxis" | "x-axis" | "x_axis" => Some("X"),
            "y" | "yaxis" | "y-axis" | "y_axis" => Some("Y"),
            "size" | "bubble" | "bubblesize" | "bubble-size" | "bubble_size" => Some("Size"),
            "legend" | "series" | "color" | "colour" => Some("Series"),
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
            | (VisualBindingFamily::ComboCategoryY, "Y" | "Y2")
            | (VisualBindingFamily::CategoryShare, "Y")
            | (VisualBindingFamily::RowsColumnsValues, "Values")
    ) || column_binding_is_aggregated(visual_type, role)?)
}

pub(crate) fn column_binding_is_aggregated(visual_type: &str, role: &str) -> CliResult<bool> {
    let family = binding_family(visual_type)?;
    Ok(matches!(
        (family, role),
        (VisualBindingFamily::CategorySeriesYAggregatable, "Y")
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
    let (proof_level, proof_note) = if pilot_2026_08_schema_golden(spec.visual_type) {
        (
            "schema-golden",
            "Exact visual.json output replicates a Power BI Desktop-rendered fixture from the 2026-08 production pilot.",
        )
    } else if spec.family == VisualBindingFamily::ComboCategoryY {
        (
            "manual-desktop-canvas-refresh",
            "Power BI Desktop Store 2.156.951.0 refreshed, rendered, sorted, and saved the generated combo fixture on 2026-07-23.",
        )
    } else {
        (
            "desktop-golden-pending",
            "The binding family retains its recorded proof, but the current title-bearing generated visual bytes await Desktop open/refresh/save re-verification.",
        )
    };
    let binding_proof_level = if pilot_2026_08_schema_golden(spec.visual_type) {
        "schema-golden"
    } else {
        binding_proof_level(spec.family)
    };
    let mut contract = json!({
        "visualType": spec.visual_type,
        "aliases": spec.aliases,
        "generatedBy": "report visuals add",
        "bindingFamily": binding_family_name(spec.family),
        "proofLevel": proof_level,
        "bindingProofLevel": binding_proof_level,
        "proofNote": proof_note,
        "summary": spec.summary,
        "roles": role_specs_json(spec.family),
        "examples": example_commands(spec),
        "limitations": [
            "Generated PBIR is a minimal visual container plus queryState.",
            "Columns in proven aggregatable roles are emitted as an explicit Sum Aggregation; other raw value-axis columns remain refused.",
            "Repeated use of one model field is refused until Desktop-authored duplicate queryRef numbering is available.",
            "Use formatting bundles, themes, or cloned Desktop-authored templates for style beyond generated defaults."
        ]
    });
    if spec.family == VisualBindingFamily::SlicerField {
        contract["modes"] = json!(["Basic", "Dropdown", "Between"]);
    }
    contract
}

fn visual_type_rules_json(spec: &VisualTypeSpec) -> Value {
    let roles = role_specs_json(spec.family);
    let role_items = roles.as_array().expect("role specs are always arrays");
    let required = role_items
        .iter()
        .filter(|role| role["required"].as_bool() == Some(true))
        .filter_map(|role| role["role"].as_str())
        .collect::<Vec<_>>();
    let optional = role_items
        .iter()
        .filter(|role| role["required"].as_bool() == Some(false))
        .filter_map(|role| role["role"].as_str())
        .collect::<Vec<_>>();
    let measure_only = role_items
        .iter()
        .filter(|role| role["fieldKinds"] == json!(["measure"]))
        .filter_map(|role| role["role"].as_str())
        .collect::<Vec<_>>();
    let max_projections = role_items
        .iter()
        .filter_map(|role| Some((role["role"].as_str()?.to_string(), role["max"].clone())))
        .collect::<serde_json::Map<_, _>>();
    let (proof_level, fixture_kind, evidence) = role_rule_provenance(spec);

    json!({
        "visualType": spec.visual_type,
        "bindingFamily": binding_family_name(spec.family),
        "required": required,
        "optional": optional,
        "measureOnly": measure_only,
        "maxProjections": max_projections,
        "mutuallyExclusive": mutually_exclusive_roles(spec.family),
        "runtimeParity": runtime_parity_rules(spec),
        "proofLevel": proof_level,
        "fixtureKind": fixture_kind,
        "evidence": evidence,
        "refusalCode": "unsupported_feature"
    })
}

fn mutually_exclusive_roles(_family: VisualBindingFamily) -> Vec<Vec<&'static str>> {
    // None of the sixteen currently generated families has two *supported* roles
    // that are mutually exclusive. Unsupported aliases such as scatter Details
    // are represented as runtime-parity refusals instead of pretending they are
    // members of the supported role set.
    Vec::new()
}

fn runtime_parity_rules(spec: &VisualTypeSpec) -> Vec<Value> {
    let mut rules = vec![json!({
        "id": "binding.no-duplicate-field",
        "requirement": "The same model field may be projected at most once per visual.",
        "onViolation": "refuse",
        "repair": "manual",
        "evidence": "src/pbir_bindings.rs::reject_duplicate_fields"
    })];
    match spec.family {
        VisualBindingFamily::SingleValue
        | VisualBindingFamily::CategoryY
        | VisualBindingFamily::ComboCategoryY
        | VisualBindingFamily::CategoryShare
        | VisualBindingFamily::RowsColumnsValues => rules.push(json!({
            "id": "binding.measure-only-value-role",
            "roles": role_specs_json(spec.family).as_array().expect("roles").iter()
                .filter(|role| role["fieldKinds"] == json!(["measure"]))
                .filter_map(|role| role["role"].as_str()).collect::<Vec<_>>(),
            "requirement": "Bare Column expressions are refused in measure-only roles.",
            "onViolation": "refuse",
            "repair": "select-existing-measure",
            "evidence": "src/pbir_bindings.rs::resolve_binding"
        })),
        _ => {}
    }
    match spec.family {
        VisualBindingFamily::CategorySeriesYAggregatable => rules.push(json!({
            "id": "binding.explicit-sum-column",
            "roles": ["Y"],
            "requirement": "Column inputs are emitted as PBIR Aggregation(Function=0), never as bare Column expressions.",
            "onViolation": "repairable",
            "repair": "wrap-sum-aggregation",
            "evidence": "2026-08 repository-generated pilot fixture"
        })),
        VisualBindingFamily::ScatterBubble => {
            rules.push(json!({
                "id": "scatter.details-role-refused",
                "roles": ["Details"],
                "requirement": "Desktop runtime detail identity uses queryState.Category; queryState.Details is refused.",
                "onViolation": "repairable",
                "repair": "rename-role-to-Category",
                "evidence": "docs/pilot-lessons.md lesson 3 and src/pbir_bindings.rs::visual_query_state_errors"
            }));
            rules.push(json!({
                "id": "scatter.category-aggregated-value-axes",
                "when": "Category has at least one projection",
                "roles": ["X", "Y", "Size"],
                "requirement": "Each value-axis field is a Measure or PBIR Aggregation(Function=0); a bare Column is refused.",
                "onViolation": "repairable",
                "repair": "wrap-sum-aggregation",
                "evidence": "2026-08 repository-generated pilot fixture and PBIR_ROLE_KIND_MISMATCH validation"
            }));
            rules.push(json!({
                "id": "scatter.series-pbir-role",
                "roles": ["Series"],
                "requirement": "Legend/color input aliases are serialized as the Desktop runtime PBIR role Series.",
                "onViolation": "repairable",
                "repair": "rename-role-to-Series",
                "evidence": "docs/pbir-desktop-oracle.md scatter runtime finding"
            }));
        }
        VisualBindingFamily::SlicerField => rules.push(json!({
            "id": "slicer.no-persisted-selection",
            "requirement": "Generated slicers omit objects.general.filter and other selected-value state.",
            "onViolation": "refuse",
            "repair": "report slicers clear",
            "evidence": "docs/reference/desktop-authored-visuals/slicer.visual.json"
        })),
        _ => {}
    }
    rules
}

fn role_rule_provenance(spec: &VisualTypeSpec) -> (&'static str, &'static str, Vec<&'static str>) {
    match spec.visual_type {
        "pieChart" => (
            "desktop-golden-pending",
            "desktop-authored-reference",
            vec!["docs/reference/desktop-authored-visuals/pieChart.visual.json"],
        ),
        "donutChart" => (
            "desktop-golden-pending",
            "desktop-authored-reference",
            vec!["docs/reference/desktop-authored-visuals/donutChart.visual.json"],
        ),
        "pivotTable" => (
            "desktop-golden-pending",
            "desktop-authored-reference",
            vec!["docs/reference/desktop-authored-visuals/matrix.visual.json"],
        ),
        "slicer" => (
            "desktop-golden-pending",
            "desktop-authored-reference",
            vec!["docs/reference/desktop-authored-visuals/slicer.visual.json"],
        ),
        "lineClusteredColumnComboChart" => (
            "manual-desktop-canvas-refresh",
            "repository-generated-desktop-saved",
            vec![
                "docs/reference/desktop-authored-visuals/lineClusteredColumnComboChart.visual.json",
                "testdata/desktop-proof/combo-pareto.2026-07-23.refresh-session.json",
            ],
        ),
        visual_type if pilot_2026_08_schema_golden(visual_type) => (
            "schema-golden",
            "repository-generated-pilot",
            vec![
                "docs/pilot-lessons.md",
                "src/report_visual_mutations.rs::validate_binding_cardinality",
            ],
        ),
        _ => (
            "unit-smoke",
            "repository-generated",
            vec![
                "src/report_visual_mutations.rs::validate_binding_cardinality",
                "src/pbir_visual_factory.rs::visual_container_json",
            ],
        ),
    }
}

fn role_specs_json(family: VisualBindingFamily) -> Value {
    match family {
        VisualBindingFamily::SingleValue => json!([
            {
                "role": "Values",
                "required": true,
                "min": 1,
                "max": 1,
                "fieldKinds": ["measure"],
                "aliases": ["values", "value", "field"],
                "summary": "Exactly one measure binding; consumed PBIR data visuals cannot be emitted as unbound placeholders."
            }
        ]),
        VisualBindingFamily::ValuesList => json!([
            {
                "role": "Values",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["column", "measure"],
                "aliases": ["values", "value", "columns", "field"],
                "summary": "One or more column or measure bindings."
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
        VisualBindingFamily::CategorySeriesYAggregatable => json!([
            {
                "role": "Category",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["column"],
                "aliases": ["category", "categories", "axis", "x"],
                "summary": "One or more category columns."
            },
            {
                "role": "Y",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["measure", "aggregatedColumn"],
                "aliases": ["y", "values", "value"],
                "summary": "One or more measures or columns emitted as explicit Sum aggregations."
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
        VisualBindingFamily::ComboCategoryY => json!([
            {
                "role": "Category",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["column"],
                "aliases": ["category", "categories", "axis", "x"],
                "summary": "Shared category axis. Multiple projections become a hierarchy axis for Desktop drill up/down."
            },
            {
                "role": "Y",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["measure"],
                "aliases": ["y", "columnY", "column-values"],
                "summary": "One or more clustered-column measures."
            },
            {
                "role": "Y2",
                "required": true,
                "min": 1,
                "max": null,
                "fieldKinds": ["measure"],
                "aliases": ["y2", "lineY", "line-values"],
                "summary": "One or more line measures on the secondary value axis."
            },
            {
                "role": "Tooltips",
                "required": false,
                "min": 0,
                "max": null,
                "fieldKinds": ["column", "measure"],
                "aliases": ["tooltip", "tooltips"],
                "summary": "Optional projected fields available to tooltips and explicit visual sorting."
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
                "fieldKinds": ["measure", "aggregatedColumn"],
                "aliases": ["x", "xAxis"],
                "summary": "Continuous X-axis measure or explicitly summed column."
            },
            {
                "role": "Y",
                "required": true,
                "min": 1,
                "max": 1,
                "fieldKinds": ["measure", "aggregatedColumn"],
                "aliases": ["y", "yAxis"],
                "summary": "Continuous Y-axis measure or explicitly summed column."
            },
            {
                "role": "Category",
                "required": false,
                "min": 0,
                "max": 1,
                "fieldKinds": ["column"],
                "aliases": ["category", "values"],
                "summary": "Optional bubble identity/detail column."
            },
            {
                "role": "Size",
                "required": false,
                "min": 0,
                "max": 1,
                "fieldKinds": ["measure", "aggregatedColumn"],
                "aliases": ["size", "bubbleSize"],
                "summary": "Optional bubble-size measure or explicitly summed column."
            },
            {
                "role": "Series",
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
        VisualBindingFamily::CategorySeriesYAggregatable => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Category,table=<table>,column=<category-column>\" --binding \"role=Series,table=<table>,column=<series-column>\" --binding \"role=Y,table=<table>,column=<numeric-column>\" --dry-run --json",
            spec.visual_type
        )],
        VisualBindingFamily::ComboCategoryY => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Category,table=<table>,column=<column>\" --binding \"role=Y,table=<table>,measure=<column-measure>\" --binding \"role=Y2,table=<table>,measure=<line-measure>\" --dry-run --json",
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
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type slicer --mode between --title <title> --binding \"role=Values,table=<table>,column=<numeric-or-date-column>\" --dry-run --json".to_string(),
        ],
        VisualBindingFamily::ScatterBubble => vec![format!(
            "powerbi-cli report visuals add --project <project-dir-or.pbip> --page <page-handle> --visual-type {} --title <title> --binding \"role=Category,table=<table>,column=<detail-column>\" --binding \"role=X,table=<table>,column=<x-column>\" --binding \"role=Y,table=<table>,measure=<y-measure>\" --binding \"role=Size,table=<table>,column=<size-column>\" --dry-run --json",
            spec.visual_type
        )],
    }
}

fn role_names(family: VisualBindingFamily) -> Vec<&'static str> {
    match family {
        VisualBindingFamily::SingleValue | VisualBindingFamily::ValuesList => vec!["Values"],
        VisualBindingFamily::CategoryY => vec!["Category", "Y", "Series", "Tooltips"],
        VisualBindingFamily::CategorySeriesYAggregatable => vec!["Category", "Y", "Series"],
        VisualBindingFamily::ComboCategoryY => vec!["Category", "Y", "Y2", "Tooltips"],
        VisualBindingFamily::CategoryShare => vec!["Category", "Y"],
        VisualBindingFamily::RowsColumnsValues => vec!["Rows", "Columns", "Values"],
        VisualBindingFamily::SlicerField => vec!["Values"],
        VisualBindingFamily::ScatterBubble => {
            vec!["Category", "X", "Y", "Size", "Series", "Tooltips"]
        }
    }
}

pub(crate) fn supported_roles(visual_type: &str) -> CliResult<Vec<&'static str>> {
    binding_family(visual_type).map(role_names)
}

fn binding_family_name(family: VisualBindingFamily) -> &'static str {
    match family {
        VisualBindingFamily::SingleValue => "singleValue",
        VisualBindingFamily::ValuesList => "valuesList",
        VisualBindingFamily::CategoryY => "categoryY",
        VisualBindingFamily::CategorySeriesYAggregatable => "categorySeriesYAggregatable",
        VisualBindingFamily::ComboCategoryY => "comboCategoryY",
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
        | VisualBindingFamily::SlicerField
        | VisualBindingFamily::ComboCategoryY => "manual-desktop-canvas-refresh",
        _ => "unit-smoke",
    }
}

fn pilot_2026_08_schema_golden(visual_type: &str) -> bool {
    matches!(
        visual_type,
        "card" | "tableEx" | "lineChart" | "scatterChart" | "hundredPercentStackedColumnChart"
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_generated_visual_type_has_one_complete_role_rule_row() {
        let rules = visual_type_role_rules();
        assert_eq!(rules.len(), VISUAL_TYPES.len());
        let visual_types = rules
            .iter()
            .map(|rule| rule["visualType"].as_str().expect("visualType"))
            .collect::<BTreeSet<_>>();
        assert_eq!(visual_types.len(), VISUAL_TYPES.len());
        for rule in rules {
            assert!(rule["required"].is_array(), "{rule}");
            assert!(rule["optional"].is_array(), "{rule}");
            assert!(rule["measureOnly"].is_array(), "{rule}");
            assert!(rule["maxProjections"].is_object(), "{rule}");
            assert!(rule["mutuallyExclusive"].is_array(), "{rule}");
            assert!(rule["runtimeParity"].is_array(), "{rule}");
            assert!(rule["proofLevel"].is_string(), "{rule}");
            assert!(rule["fixtureKind"].is_string(), "{rule}");
            assert!(
                rule["evidence"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
            );
            assert_eq!(rule["refusalCode"], "unsupported_feature");
        }
    }

    #[test]
    fn only_independent_desktop_reference_rows_claim_that_fixture_kind() {
        let reference_types = visual_type_role_rules()
            .into_iter()
            .filter(|rule| rule["fixtureKind"] == "desktop-authored-reference")
            .map(|rule| rule["visualType"].as_str().expect("visualType").to_string())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            reference_types,
            ["donutChart", "pieChart", "pivotTable", "slicer"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect()
        );
    }

    #[test]
    fn scatter_rule_encodes_runtime_parity_refusals_and_repairs() {
        let rule = visual_type_role_rule("scatterChart").expect("scatter rule");
        assert_eq!(rule["required"], json!(["X", "Y"]));
        assert_eq!(
            rule["optional"],
            json!(["Category", "Size", "Series", "Tooltips"])
        );
        assert_eq!(rule["maxProjections"]["X"], 1);
        assert_eq!(rule["maxProjections"]["Y"], 1);
        assert_eq!(rule["maxProjections"]["Size"], 1);
        let ids = rule["runtimeParity"]
            .as_array()
            .expect("runtimeParity")
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("scatter.details-role-refused"));
        assert!(ids.contains("scatter.category-aggregated-value-axes"));
        assert!(ids.contains("scatter.series-pbir-role"));
    }
}
