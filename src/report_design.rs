use crate::pbir::load_report_snapshot;
use crate::tmdl::{ColumnRecord, TableDocument, load_table_documents};
use crate::{
    CliError, CliResult, canonical_display, command_arg, resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Default)]
struct DesignPlanOptions {
    project: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct ColumnChoice {
    table: String,
    column: String,
    data_type: Option<String>,
}

#[derive(Debug, Clone)]
struct MeasureChoice {
    table: String,
    measure: String,
    format_string: Option<String>,
}

pub(crate) fn design_plan_command(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    let project = required_project(options.project, "report design-plan")?;
    let resolved = resolve_project(&project)?;
    let validation = validate_project(&resolved)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let docs = load_table_documents(&resolved)?;
    let profile = model_profile(&docs);
    let date_columns = candidate_columns(&docs, is_date_column);
    let numeric_columns = candidate_columns(&docs, is_numeric_column);
    let category_columns = candidate_columns(&docs, is_category_column);
    let measures = candidate_measures(&docs);
    let opportunities = visual_opportunities(
        &resolved.project_dir,
        &date_columns,
        &category_columns,
        &numeric_columns,
        &measures,
    );
    let workflow = recommended_workflow(&resolved.project_dir, &opportunities);

    Ok(json!({
        "schema": "powerbi-cli.report.designPlan.v1",
        "ok": validation.errors.is_empty(),
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "profile": profile,
        "candidates": {
            "dateColumns": date_columns.iter().map(column_choice_json).collect::<Vec<_>>(),
            "categoryColumns": category_columns.iter().map(column_choice_json).collect::<Vec<_>>(),
            "numericColumns": numeric_columns.iter().map(column_choice_json).collect::<Vec<_>>(),
            "measures": measures.iter().map(measure_choice_json).collect::<Vec<_>>()
        },
        "reportState": {
            "pages": snapshot.pages.len(),
            "visuals": snapshot.pages.iter().map(|page| page.visuals.len()).sum::<usize>(),
            "boundVisuals": snapshot.pages.iter()
                .flat_map(|page| page.visuals.iter())
                .filter(|visual| !visual.bindings.is_empty())
                .count()
        },
        "opportunities": opportunities,
        "recommendedWorkflow": workflow,
        "warnings": validation.warnings,
        "errors": validation.errors,
        "next": workflow
    }))
}

fn model_profile(docs: &[TableDocument]) -> Value {
    json!({
        "tables": docs.iter().map(|doc| json!({
            "name": doc.table,
            "path": canonical_display(&doc.path),
            "columns": doc.columns.len(),
            "calculatedColumns": doc.columns.iter().filter(|column| column.is_calculated()).count(),
            "measures": doc.measures.len(),
            "partitions": doc.partitions.len(),
            "dateColumns": doc.columns.iter().filter(|column| is_date_column(column)).count(),
            "categoryColumns": doc.columns.iter().filter(|column| is_category_column(column)).count(),
            "numericColumns": doc.columns.iter().filter(|column| is_numeric_column(column)).count()
        })).collect::<Vec<_>>(),
        "counts": {
            "tables": docs.len(),
            "columns": docs.iter().map(|doc| doc.columns.len()).sum::<usize>(),
            "calculatedColumns": docs.iter().flat_map(|doc| doc.columns.iter()).filter(|column| column.is_calculated()).count(),
            "measures": docs.iter().map(|doc| doc.measures.len()).sum::<usize>(),
            "partitions": docs.iter().map(|doc| doc.partitions.len()).sum::<usize>()
        }
    })
}

fn visual_opportunities(
    project_dir: &std::path::Path,
    date_columns: &[ColumnChoice],
    category_columns: &[ColumnChoice],
    numeric_columns: &[ColumnChoice],
    measures: &[MeasureChoice],
) -> Vec<Value> {
    let mut items = Vec::new();
    if let (Some(date), Some(value)) =
        (date_columns.first(), first_value(measures, numeric_columns))
    {
        items.push(json!({
            "kind": "time-series",
            "visualType": "lineChart",
            "confidence": "high",
            "reason": "The model has a date-like column and at least one numeric value.",
            "fields": {
                "category": column_choice_json(date),
                "value": value_json(&value)
            },
            "command": format!(
                "powerbi-cli report visuals add --project {} --page <page-handle> --visual-type lineChart --title 'Trend' --binding {} --binding {} --dry-run --json",
                command_arg(project_dir),
                binding_arg("Category", date),
                value_binding_arg("Y", &value)
            )
        }));
    }
    if let (Some(category), Some(value)) = (
        category_columns.first(),
        first_value(measures, numeric_columns),
    ) {
        items.push(json!({
            "kind": "ranking",
            "visualType": "clusteredBarChart",
            "confidence": "high",
            "reason": "The model has categorical fields and a numeric value for comparison.",
            "fields": {
                "category": column_choice_json(category),
                "value": value_json(&value)
            },
            "command": format!(
                "powerbi-cli report visuals add --project {} --page <page-handle> --visual-type clusteredBarChart --title 'Top categories' --binding {} --binding {} --dry-run --json",
                command_arg(project_dir),
                binding_arg("Category", category),
                value_binding_arg("Y", &value)
            )
        }));
    }
    if let (Some(x), Some(y), Some(category)) = (
        first_value(measures, numeric_columns),
        second_value(measures, numeric_columns),
        category_columns.first(),
    ) {
        items.push(json!({
            "kind": "scatter-bubble",
            "visualType": "scatterChart",
            "confidence": "medium",
            "reason": "The model has at least two numeric values and a category field.",
            "fields": {
                "category": column_choice_json(category),
                "x": value_json(&x),
                "y": value_json(&y)
            },
            "command": format!(
                "powerbi-cli report visuals add --project {} --page <page-handle> --visual-type scatterChart --title 'Relationship map' --binding {} --binding {} --binding {} --dry-run --json",
                command_arg(project_dir),
                binding_arg("Category", category),
                value_binding_arg("X", &x),
                value_binding_arg("Y", &y)
            )
        }));
    }
    if category_columns.len() >= 2 {
        let first = &category_columns[0];
        let second = &category_columns[1];
        items.push(json!({
            "kind": "drilldown-hierarchy",
            "visualType": "lineChart|barChart|columnChart",
            "confidence": "medium",
            "reason": "Multiple categorical/date-like columns can become a hierarchy axis on an existing chart.",
            "fields": [column_choice_json(first), column_choice_json(second)],
            "command": format!(
                "powerbi-cli report drilldown set-hierarchy --project {} --handle <visual-handle> --field '{}' --field '{}' --dry-run --json",
                command_arg(project_dir),
                field_ref(first),
                field_ref(second)
            )
        }));
    }
    items.push(json!({
        "kind": "layout",
        "visualType": "all",
        "confidence": "high",
        "reason": "Existing visuals can be normalized into deterministic canvas slots.",
        "command": format!(
            "powerbi-cli report layout auto --project {} --page <page-handle> --preset overview --dry-run --json",
            command_arg(project_dir)
        )
    }));
    items.push(json!({
        "kind": "style",
        "visualType": "all",
        "confidence": "high",
        "reason": "Report-level theme presets and extracted theme bundles are portable across offline PBIP projects.",
        "command": format!(
            "powerbi-cli report themes apply-preset --project {} --preset risk-dashboard --dry-run --json",
            command_arg(project_dir)
        )
    }));
    items
}

fn recommended_workflow(project_dir: &std::path::Path, opportunities: &[Value]) -> Vec<String> {
    let mut commands = vec![
        format!(
            "powerbi-cli report pages list --project {} --json",
            command_arg(project_dir)
        ),
        "powerbi-cli report visuals catalog --json".to_string(),
    ];
    commands.extend(
        opportunities
            .iter()
            .filter_map(|item| item["command"].as_str().map(ToOwned::to_owned)),
    );
    commands.push(format!(
        "powerbi-cli report wireframe export {} --json",
        command_arg(project_dir)
    ));
    commands.push(format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(project_dir)
    ));
    commands
}

#[derive(Debug, Clone)]
enum ValueChoice {
    Measure(MeasureChoice),
    Column(ColumnChoice),
}

fn first_value(
    measures: &[MeasureChoice],
    numeric_columns: &[ColumnChoice],
) -> Option<ValueChoice> {
    measures
        .first()
        .cloned()
        .map(ValueChoice::Measure)
        .or_else(|| numeric_columns.first().cloned().map(ValueChoice::Column))
}

fn second_value(
    measures: &[MeasureChoice],
    numeric_columns: &[ColumnChoice],
) -> Option<ValueChoice> {
    measures
        .get(1)
        .cloned()
        .map(ValueChoice::Measure)
        .or_else(|| {
            numeric_columns
                .iter()
                .find(|column| {
                    measures.first().is_none_or(|measure| {
                        measure.table != column.table || measure.measure != column.column
                    })
                })
                .cloned()
                .map(ValueChoice::Column)
        })
}

fn value_json(value: &ValueChoice) -> Value {
    match value {
        ValueChoice::Measure(measure) => measure_choice_json(measure),
        ValueChoice::Column(column) => column_choice_json(column),
    }
}

fn value_binding_arg(role: &str, value: &ValueChoice) -> String {
    match value {
        ValueChoice::Measure(measure) => format!(
            "\"role={role},table={},measure={}\"",
            measure.table, measure.measure
        ),
        ValueChoice::Column(column) => binding_arg(role, column),
    }
}

fn binding_arg(role: &str, column: &ColumnChoice) -> String {
    format!(
        "\"role={role},table={},column={}\"",
        column.table, column.column
    )
}

fn field_ref(column: &ColumnChoice) -> String {
    format!("{}[{}]", column.table, column.column)
}

fn candidate_columns<F>(docs: &[TableDocument], mut predicate: F) -> Vec<ColumnChoice>
where
    F: FnMut(&ColumnRecord) -> bool,
{
    let mut columns = docs
        .iter()
        .flat_map(|doc| {
            doc.columns
                .iter()
                .filter(|column| !column.is_hidden && predicate(column))
                .map(|column| ColumnChoice {
                    table: doc.table.clone(),
                    column: column.name.clone(),
                    data_type: column.data_type.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    columns.sort_by_key(|column| {
        (
            column_rank(&column.column),
            column.table.clone(),
            column.column.clone(),
        )
    });
    columns
}

fn candidate_measures(docs: &[TableDocument]) -> Vec<MeasureChoice> {
    let mut measures = docs
        .iter()
        .flat_map(|doc| {
            doc.measures.iter().map(|measure| MeasureChoice {
                table: doc.table.clone(),
                measure: measure.name.clone(),
                format_string: measure.format_string.clone(),
            })
        })
        .collect::<Vec<_>>();
    measures.sort_by_key(|measure| {
        (
            measure_rank(measure),
            measure.table.clone(),
            measure.measure.clone(),
        )
    });
    measures
}

fn column_choice_json(column: &ColumnChoice) -> Value {
    json!({
        "table": column.table,
        "column": column.column,
        "dataType": column.data_type,
        "field": field_ref(column)
    })
}

fn measure_choice_json(measure: &MeasureChoice) -> Value {
    json!({
        "table": measure.table,
        "measure": measure.measure,
        "formatString": measure.format_string,
        "field": format!("{}[{}]", measure.table, measure.measure)
    })
}

fn is_date_column(column: &ColumnRecord) -> bool {
    let name = column.name.to_ascii_lowercase();
    column.data_type.as_deref().is_some_and(|kind| {
        let kind = kind.to_ascii_lowercase();
        kind.contains("date") || kind.contains("time")
    }) || name.contains("date")
        || name.contains("datum")
        || name.contains("jahr")
        || name.contains("year")
        || name.contains("month")
        || name.contains("monat")
}

fn is_numeric_column(column: &ColumnRecord) -> bool {
    column.data_type.as_deref().is_some_and(|kind| {
        matches!(
            kind.to_ascii_lowercase().as_str(),
            "int64" | "double" | "decimal" | "currency" | "number"
        )
    })
}

fn is_category_column(column: &ColumnRecord) -> bool {
    if is_date_column(column) {
        return true;
    }
    if column.is_key {
        return false;
    }
    let name = column.name.to_ascii_lowercase();
    column
        .data_type
        .as_deref()
        .is_none_or(|kind| matches!(kind.to_ascii_lowercase().as_str(), "string" | "boolean"))
        || name.contains("branche")
        || name.contains("branch")
        || name.contains("company")
        || name.contains("firma")
        || name.contains("kunde")
        || name.contains("segment")
        || name.contains("group")
        || name.contains("gruppe")
}

fn column_rank(name: &str) -> usize {
    let name = name.to_ascii_lowercase();
    if name.contains("date")
        || name.contains("datum")
        || name.contains("jahr")
        || name.contains("year")
    {
        0
    } else if name.contains("branch") || name.contains("branche") || name.contains("segment") {
        1
    } else if name.contains("company") || name.contains("firma") || name.contains("kunde") {
        2
    } else {
        10
    }
}

fn measure_rank(measure: &MeasureChoice) -> usize {
    let name = measure.measure.to_ascii_lowercase();
    if name.contains("rate") || name.contains("quote") {
        0
    } else if name.contains("cost") || name.contains("kosten") || name.contains("revenue") {
        1
    } else {
        10
    }
}

fn parse_args(args: &[String]) -> CliResult<DesignPlanOptions> {
    let mut options = DesignPlanOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut i, "--project")?));
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!(
                    "unknown report design-plan flag: {other}"
                ))
                .with_hint(
                    "Run `powerbi-cli report design-plan --project <project-dir-or.pbip> --json`.",
                )
                .with_suggested_command(
                    "powerbi-cli report design-plan --project <project-dir-or.pbip> --json",
                ));
            }
            other => {
                if options.project.is_some() {
                    return Err(CliError::invalid_args(
                        "report design-plan accepts at most one positional project",
                    )
                    .with_suggested_command(
                        "powerbi-cli report design-plan --project <project-dir-or.pbip> --json",
                    ));
                }
                options.project = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(options)
}

fn required_project(project: Option<PathBuf>, command: &str) -> CliResult<PathBuf> {
    project.ok_or_else(|| {
        CliError::invalid_args(format!(
            "{command} requires --project <project-dir-or.pbip>"
        ))
        .with_hint("Pass a PBIP project directory or .pbip file.")
        .with_suggested_command(format!(
            "powerbi-cli {command} --project <project-dir-or.pbip> --json"
        ))
    })
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args.get(*index + 1).ok_or_else(|| {
        CliError::invalid_args(format!("{flag} requires a value")).with_suggested_command(
            "powerbi-cli report design-plan --project <project-dir-or.pbip> --json",
        )
    })?;
    *index += 2;
    Ok(value.clone())
}
