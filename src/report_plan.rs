use crate::profile::{load_profile_value, profile_summary, validate_profile_value};
use crate::project_io::write_json_pretty;
use crate::report_build::compile_dashboard_summary;
use crate::schema::{load_schema_value, validate_schema_value};
use crate::{CliError, CliResult, EXIT_SUCCESS, canonical_display, command_arg};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct PlanOptions {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
    intent: Option<String>,
    objective: Option<String>,
    out: Option<PathBuf>,
    force: bool,
}

#[derive(Debug, Clone)]
struct FieldChoice {
    table: String,
    name: String,
    data_type: Option<String>,
    reference: String,
}

#[derive(Debug, Clone)]
struct MeasureChoice {
    name: String,
    reference: String,
    generated: bool,
}

#[derive(Clone, Copy)]
struct VisualLayout {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

struct PlanModel<'a> {
    schema: &'a Value,
    profile: Option<&'a Value>,
    fact_tables: Vec<String>,
    existing_measures: Vec<MeasureChoice>,
    numeric_columns: Vec<FieldChoice>,
    date_columns: Vec<FieldChoice>,
    category_columns: Vec<FieldChoice>,
}

pub(crate) fn plan_command(args: &[String]) -> CliResult<Value> {
    let options = parse_plan_args(args)?;
    let schema_path = options.schema.ok_or_else(|| {
        CliError::invalid_args("report plan requires --schema <schema.json>")
            .with_suggested_command(
                "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --intent <intent.md|text> --out <dashboard.json> --json",
            )
    })?;
    let schema_value = load_schema_value(&schema_path)?;
    let schema_validation = validate_schema_value(&schema_value);
    if !schema_validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "schema is not valid: {}",
            schema_validation.errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli schema validate {} --json",
            command_arg(&schema_path)
        )));
    }

    let profile_value = load_optional_profile(options.profile.as_deref())?;
    let intent_text = load_intent_text(options.intent.as_deref(), options.objective.as_deref())?;
    let model = PlanModel::new(&schema_value, profile_value.as_ref());
    let planned = build_dashboard_plan(&schema_value, &model, &intent_text)?;
    let compiled = compile_dashboard_summary(&schema_value, &planned.spec)?;

    if let Some(out) = options.out.as_ref() {
        if out.exists() && !options.force {
            return Err(CliError::invalid_args(format!(
                "report plan output already exists: {}",
                out.display()
            ))
            .with_hint("Pass --force after reviewing the existing file, or choose a new --out path.")
            .with_suggested_command(format!(
                "powerbi-cli report plan --schema {} --profile <profile.json> --intent <intent.md|text> --out <dashboard.json> --force --json",
                command_arg(&schema_path)
            )));
        }
        write_json_pretty(out, &planned.spec)?;
    }

    Ok(json!({
        "schema": "powerbi-cli.report.plan.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "schemaPath": canonical_display(&schema_path),
        "profilePath": options.profile.as_ref().map(|path| canonical_display(path)),
        "specPath": options.out.as_ref().map(|path| canonical_display(path)),
        "changed": options.out.is_some(),
        "intent": {
            "text": intent_text,
            "source": if options.intent.is_some() { "intent" } else { "objective" }
        },
        "profileSummary": profile_value.as_ref().map(profile_summary),
        "spec": planned.spec,
        "compiled": compiled,
        "decisions": planned.decisions,
        "warnings": planned.warnings,
        "next": next_for_plan(options.out.as_deref(), &schema_path, options.profile.as_deref())
    }))
}

struct PlannedDashboard {
    spec: Value,
    decisions: Vec<Value>,
    warnings: Vec<Value>,
}

fn build_dashboard_plan(
    schema: &Value,
    model: &PlanModel<'_>,
    intent_text: &str,
) -> CliResult<PlannedDashboard> {
    let mut decisions = Vec::new();
    let mut warnings = Vec::new();
    let mut generated_measures = Vec::new();
    let mut measures = model.existing_measures.clone();
    if measures.is_empty() {
        for column in model.numeric_columns.iter().take(3) {
            let measure_name = format!("Total {}", column.name);
            let reference = field_reference(&column.table, &measure_name);
            generated_measures.push(json!({
                "table": column.table,
                "name": measure_name,
                "expression": format!("SUM('{}'[{}])", escape_dax_table(&column.table), escape_dax_column(&column.name)),
                "formatString": format_string_for_type(column.data_type.as_deref()),
                "description": "Generated by report plan from a numeric column"
            }));
            measures.push(MeasureChoice {
                name: measure_name,
                reference,
                generated: true,
            });
        }
        if generated_measures.is_empty() {
            return Err(CliError::validation_failed(
                "report plan could not find an existing measure or numeric column to summarize",
            )
            .with_hint("Add at least one measure to the schema, or include a numeric fact column.")
            .with_suggested_command(
                "powerbi-cli report spec fields --schema <schema.json> --profile <profile.json> --json",
            ));
        }
        warnings.push(json!({
            "code": "report_plan.generated_measures",
            "message": "schema had no measures; generated SUM measures for numeric columns"
        }));
    }

    let primary_measure = measures
        .first()
        .cloned()
        .ok_or_else(|| CliError::validation_failed("report plan requires at least one measure"))?;
    let secondary_measure = measures.get(1).cloned();
    let tertiary_measure = measures.get(2).cloned();
    let primary_category = model.category_columns.first().cloned();
    let secondary_category = model.category_columns.get(1).cloned();
    let date_column = model.date_columns.first().cloned();
    let display_name = schema
        .get("displayName")
        .and_then(Value::as_str)
        .or_else(|| schema.get("name").and_then(Value::as_str))
        .unwrap_or("Power BI Dashboard");
    let report_name = schema
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("PowerBIDashboard");

    decisions.push(json!({
        "kind": "fact-table",
        "selected": model.fact_tables.first(),
        "reason": "first profile fact table, falling back to first schema table"
    }));
    decisions.push(json!({
        "kind": "primary-measure",
        "selected": primary_measure.reference,
        "generated": primary_measure.generated
    }));
    if let Some(category) = primary_category.as_ref() {
        decisions.push(json!({
            "kind": "primary-category",
            "selected": category.reference
        }));
    } else {
        warnings.push(json!({
            "code": "report_plan.no_category",
            "message": "no category column found; category visuals were omitted"
        }));
    }
    if let Some(date) = date_column.as_ref() {
        decisions.push(json!({
            "kind": "date-axis",
            "selected": date.reference
        }));
    } else {
        warnings.push(json!({
            "code": "report_plan.no_date",
            "message": "no date-like column found; trend visual was omitted"
        }));
    }

    let mut overview_visuals = Vec::new();
    overview_visuals.push(card_visual(
        "primary_kpi",
        &primary_measure.name,
        visual_layout(32, 32, 220, 112),
        &primary_measure.reference,
    ));
    if let Some(measure) = secondary_measure.as_ref() {
        overview_visuals.push(card_visual(
            "secondary_kpi",
            &measure.name,
            visual_layout(276, 32, 220, 112),
            &measure.reference,
        ));
    }
    if let Some(date) = date_column.as_ref() {
        overview_visuals.push(line_visual(
            "trend",
            &format!("{} over time", primary_measure.name),
            visual_layout(32, 184, 600, 300),
            &date.reference,
            &primary_measure.reference,
        ));
    }
    if let Some(category) = primary_category.as_ref() {
        overview_visuals.push(column_visual(
            "category_bar",
            &format!("{} by {}", primary_measure.name, category.name),
            visual_layout(664, 184, 560, 300),
            &category.reference,
            &primary_measure.reference,
        ));
    }
    overview_visuals.push(table_visual(
        "detail_table",
        "Detail",
        visual_layout(32, 516, 1192, 156),
        table_fields(
            date_column.as_ref(),
            primary_category.as_ref(),
            secondary_category.as_ref(),
            &primary_measure,
            secondary_measure.as_ref(),
        ),
    ));

    let mut pages = vec![json!({
        "id": "overview",
        "displayName": "Overview",
        "size": {"width": 1280, "height": 720},
        "visuals": overview_visuals
    })];

    if let (Some(category), Some(secondary)) =
        (primary_category.as_ref(), secondary_measure.as_ref())
    {
        let mut analysis_visuals = Vec::new();
        analysis_visuals.push(scatter_visual(
            "portfolio_scatter",
            &format!("{} vs {}", primary_measure.name, secondary.name),
            visual_layout(32, 64, 620, 420),
            &category.reference,
            &primary_measure.reference,
            &secondary.reference,
            tertiary_measure
                .as_ref()
                .map(|measure| measure.reference.as_str()),
        ));
        analysis_visuals.push(table_visual(
            "portfolio_detail",
            "Portfolio Detail",
            visual_layout(688, 64, 536, 420),
            table_fields(
                None,
                primary_category.as_ref(),
                secondary_category.as_ref(),
                &primary_measure,
                secondary_measure.as_ref(),
            ),
        ));
        pages.push(json!({
            "id": "analysis",
            "displayName": "Analysis",
            "size": {"width": 1280, "height": 720},
            "visuals": analysis_visuals
        }));
    }

    let mut spec = json!({
        "schema": "powerbi-cli.dashboard.v1",
        "report": {
            "name": report_name,
            "displayName": display_name,
            "description": format!("Agent-planned dashboard. Objective: {}", one_line(intent_text)),
            "questions": intent_questions(intent_text),
            "audience": "agent-authored Power BI users"
        },
        "pages": pages,
        "proof": {
            "required": "desktop-canvas-refresh"
        }
    });
    if !generated_measures.is_empty() {
        spec["model"] = json!({ "measures": generated_measures });
    }

    Ok(PlannedDashboard {
        spec,
        decisions,
        warnings,
    })
}

impl<'a> PlanModel<'a> {
    fn new(schema: &'a Value, profile: Option<&'a Value>) -> Self {
        let mut fact_tables = profile_fact_tables(profile);
        if fact_tables.is_empty() {
            fact_tables = schema_tables(schema)
                .into_iter()
                .take(1)
                .map(|(name, _)| name)
                .collect();
        }
        let mut model = Self {
            schema,
            profile,
            fact_tables,
            existing_measures: Vec::new(),
            numeric_columns: Vec::new(),
            date_columns: Vec::new(),
            category_columns: Vec::new(),
        };
        model.existing_measures = model.schema_measures();
        model.numeric_columns =
            model.profile_or_schema_columns("numericColumns", ColumnRole::Numeric);
        model.date_columns = model.profile_or_schema_columns("dateColumns", ColumnRole::Date);
        model.category_columns =
            model.profile_or_schema_columns("categoryColumns", ColumnRole::Category);
        model
    }

    fn schema_measures(&self) -> Vec<MeasureChoice> {
        schema_tables(self.schema)
            .into_iter()
            .flat_map(|(table_name, table)| {
                table
                    .get("measures")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(move |measure| {
                        let name = measure.get("name").and_then(Value::as_str)?;
                        Some(MeasureChoice {
                            name: name.to_string(),
                            reference: field_reference(&table_name, name),
                            generated: false,
                        })
                    })
            })
            .collect()
    }

    fn profile_or_schema_columns(&self, candidate_key: &str, role: ColumnRole) -> Vec<FieldChoice> {
        let mut fields = Vec::new();
        if let Some(profile) = self.profile
            && let Some(items) = profile
                .get("candidates")
                .and_then(|candidates| candidates.get(candidate_key))
                .and_then(Value::as_array)
        {
            for item in items {
                if let Some(field) = field_from_profile_candidate(item)
                    && self.has_column(&field.table, &field.name)
                {
                    fields.push(field);
                }
            }
        }
        if fields.is_empty() {
            fields = self.schema_columns(role);
        }
        prioritize_fact_columns(fields, &self.fact_tables)
    }

    fn schema_columns(&self, role: ColumnRole) -> Vec<FieldChoice> {
        let mut fields = Vec::new();
        for (table_name, table) in schema_tables(self.schema) {
            for column in table
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = column.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let data_type = column
                    .get("dataType")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                if column.get("isKey").and_then(Value::as_bool) == Some(true)
                    && role != ColumnRole::Date
                {
                    continue;
                }
                if role.matches(name, data_type.as_deref()) {
                    fields.push(FieldChoice {
                        table: table_name.clone(),
                        name: name.to_string(),
                        data_type,
                        reference: field_reference(&table_name, name),
                    });
                }
            }
        }
        fields
    }

    fn has_column(&self, table: &str, column: &str) -> bool {
        schema_tables(self.schema)
            .into_iter()
            .any(|(table_name, table_value)| {
                table_name.eq_ignore_ascii_case(table)
                    && table_value
                        .get("columns")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .any(|candidate| {
                            candidate
                                .get("name")
                                .and_then(Value::as_str)
                                .is_some_and(|name| name.eq_ignore_ascii_case(column))
                        })
            })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColumnRole {
    Numeric,
    Date,
    Category,
}

impl ColumnRole {
    fn matches(self, name: &str, data_type: Option<&str>) -> bool {
        let lower_name = name.to_ascii_lowercase();
        let lower_type = data_type.unwrap_or_default().to_ascii_lowercase();
        match self {
            Self::Numeric => matches!(
                lower_type.as_str(),
                "int"
                    | "integer"
                    | "whole"
                    | "whole_number"
                    | "int64"
                    | "double"
                    | "float"
                    | "number"
                    | "decimal"
                    | "fixed_decimal"
                    | "currency"
            ),
            Self::Date => {
                matches!(lower_type.as_str(), "date" | "datetime" | "date_time")
                    || lower_name.contains("date")
                    || lower_name.contains("year")
                    || lower_name.contains("month")
            }
            Self::Category => {
                matches!(lower_type.as_str(), "text" | "string")
                    || lower_name.contains("name")
                    || lower_name.contains("category")
                    || lower_name.contains("segment")
                    || lower_name.contains("region")
            }
        }
    }
}

fn parse_plan_args(args: &[String]) -> CliResult<PlanOptions> {
    let mut options = PlanOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                options.schema = Some(PathBuf::from(take_value(args, &mut i, "--schema")?))
            }
            "--profile" => {
                options.profile = Some(PathBuf::from(take_value(args, &mut i, "--profile")?))
            }
            "--intent" | "--intent-file" => {
                options.intent = Some(take_value(args, &mut i, "--intent")?)
            }
            "--objective" | "--goal" => {
                options.objective = Some(take_value(args, &mut i, "--objective")?)
            }
            "--out" | "--out-file" => {
                options.out = Some(PathBuf::from(take_value(args, &mut i, "--out")?))
            }
            "--force" => {
                options.force = true;
                i += 1;
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!("unknown report plan flag: {other}"))
                    .with_suggested_command(
                        "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --intent <intent.md|text> --out <dashboard.json> --json",
                    ));
            }
            other => {
                if options.intent.is_some() {
                    return Err(CliError::invalid_args(
                        "report plan accepts at most one positional intent",
                    )
                    .with_suggested_command(
                        "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --intent <intent.md|text> --out <dashboard.json> --json",
                    ));
                }
                options.intent = Some(other.to_string());
                i += 1;
            }
        }
    }
    Ok(options)
}

fn load_optional_profile(path: Option<&Path>) -> CliResult<Option<Value>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let profile = load_profile_value(path)?;
    let errors = validate_profile_value(&profile);
    if !errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "profile is not valid: {}",
            errors.join("; ")
        )));
    }
    Ok(Some(profile))
}

fn load_intent_text(intent: Option<&str>, objective: Option<&str>) -> CliResult<String> {
    if let Some(objective) = objective.filter(|value| !value.trim().is_empty()) {
        return Ok(objective.trim().to_string());
    }
    let Some(intent) = intent.filter(|value| !value.trim().is_empty()) else {
        return Err(CliError::invalid_args(
            "report plan requires --intent <intent.md|text> or --objective <text>",
        )
        .with_hint("Give the planner the business question or dashboard objective to optimize for.")
        .with_suggested_command(
            "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --objective \"Executive overview with trends and segment breakdown\" --out dashboard.json --json",
        ));
    };
    let path = Path::new(intent);
    if path.is_file() {
        return fs::read_to_string(path).map_err(|err| {
            CliError::file_not_found(format!("read intent {}: {err}", path.display()))
        });
    }
    Ok(intent.to_string())
}

fn schema_tables(schema: &Value) -> Vec<(String, &Map<String, Value>)> {
    schema
        .get("tables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|table| {
            let object = table.as_object()?;
            let name = object.get("name").and_then(Value::as_str)?;
            Some((name.to_string(), object))
        })
        .collect()
}

fn profile_fact_tables(profile: Option<&Value>) -> Vec<String> {
    profile
        .and_then(|profile| {
            profile
                .get("candidates")
                .and_then(|candidates| candidates.get("factTables"))
        })
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn field_from_profile_candidate(value: &Value) -> Option<FieldChoice> {
    let table = value.get("table").and_then(Value::as_str)?.to_string();
    let name = value.get("column").and_then(Value::as_str)?.to_string();
    let data_type = value
        .get("dataType")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let reference = value
        .get("field")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| field_reference(&table, &name));
    Some(FieldChoice {
        table,
        name,
        data_type,
        reference,
    })
}

fn prioritize_fact_columns(
    mut fields: Vec<FieldChoice>,
    fact_tables: &[String],
) -> Vec<FieldChoice> {
    let fact_table_set = fact_tables
        .iter()
        .map(|table| table.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    fields.sort_by_key(|field| {
        (
            !fact_table_set.contains(&field.table.to_ascii_lowercase()),
            field.table.clone(),
            field.name.clone(),
        )
    });
    dedupe_fields(fields)
}

fn dedupe_fields(fields: Vec<FieldChoice>) -> Vec<FieldChoice> {
    let mut seen = BTreeSet::new();
    fields
        .into_iter()
        .filter(|field| seen.insert(field.reference.to_ascii_lowercase()))
        .collect()
}

fn visual_layout(x: i64, y: i64, width: i64, height: i64) -> VisualLayout {
    VisualLayout {
        x,
        y,
        width,
        height,
    }
}

fn card_visual(id: &str, title: &str, layout: VisualLayout, measure: &str) -> Value {
    visual(id, "card", title, layout, vec![binding("Values", measure)])
}

fn line_visual(
    id: &str,
    title: &str,
    layout: VisualLayout,
    category: &str,
    measure: &str,
) -> Value {
    visual(
        id,
        "lineChart",
        title,
        layout,
        vec![binding("Category", category), binding("Y", measure)],
    )
}

fn column_visual(
    id: &str,
    title: &str,
    layout: VisualLayout,
    category: &str,
    measure: &str,
) -> Value {
    visual(
        id,
        "columnChart",
        title,
        layout,
        vec![binding("Category", category), binding("Y", measure)],
    )
}

fn scatter_visual(
    id: &str,
    title: &str,
    layout: VisualLayout,
    category: &str,
    x_measure: &str,
    y_measure: &str,
    size_measure: Option<&str>,
) -> Value {
    let mut bindings = vec![
        binding("Category", category),
        binding("X", x_measure),
        binding("Y", y_measure),
    ];
    if let Some(size_measure) = size_measure {
        bindings.push(binding("Size", size_measure));
    }
    visual(id, "scatterChart", title, layout, bindings)
}

fn table_visual(id: &str, title: &str, layout: VisualLayout, fields: Vec<String>) -> Value {
    visual(
        id,
        "tableEx",
        title,
        layout,
        fields
            .into_iter()
            .map(|field| binding("Values", &field))
            .collect(),
    )
}

fn visual(
    id: &str,
    visual_type: &str,
    title: &str,
    layout: VisualLayout,
    bindings: Vec<Value>,
) -> Value {
    json!({
        "id": id,
        "type": visual_type,
        "title": title,
        "layout": {"x": layout.x, "y": layout.y, "width": layout.width, "height": layout.height},
        "bindings": bindings
    })
}

fn binding(role: &str, field: &str) -> Value {
    json!({"role": role, "field": field})
}

fn table_fields(
    date: Option<&FieldChoice>,
    primary_category: Option<&FieldChoice>,
    secondary_category: Option<&FieldChoice>,
    primary_measure: &MeasureChoice,
    secondary_measure: Option<&MeasureChoice>,
) -> Vec<String> {
    let mut fields = Vec::new();
    for field in [
        date.map(|field| field.reference.clone()),
        primary_category.map(|field| field.reference.clone()),
        secondary_category.map(|field| field.reference.clone()),
        Some(primary_measure.reference.clone()),
        secondary_measure.map(|measure| measure.reference.clone()),
    ]
    .into_iter()
    .flatten()
    {
        if !fields.contains(&field) {
            fields.push(field);
        }
    }
    fields
}

fn intent_questions(intent: &str) -> Vec<Value> {
    let mut questions = intent
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(6)
        .map(|line| Value::String(line.trim_start_matches(['-', '*', ' ']).to_string()))
        .collect::<Vec<_>>();
    if questions.is_empty() {
        questions.push(Value::String(one_line(intent)));
    }
    questions
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn field_reference(table: &str, field: &str) -> String {
    format!("{table}[{field}]")
}

fn format_string_for_type(data_type: Option<&str>) -> &'static str {
    match data_type.unwrap_or_default().to_ascii_lowercase().as_str() {
        "currency" | "fixed_decimal" => "$#,##0",
        "decimal" | "double" | "float" | "number" => "#,##0.0",
        _ => "#,##0",
    }
}

fn escape_dax_table(value: &str) -> String {
    value.replace('\'', "''")
}

fn escape_dax_column(value: &str) -> String {
    value.replace(']', "]]")
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
    *index += 2;
    Ok(value.clone())
}

fn next_for_plan(out: Option<&Path>, schema: &Path, profile: Option<&Path>) -> Vec<String> {
    if let Some(out) = out {
        let profile_arg = profile
            .map(|path| format!(" --profile {}", command_arg(path)))
            .unwrap_or_default();
        vec![
            format!(
                "powerbi-cli report spec validate --schema {}{} --spec {} --json",
                command_arg(schema),
                profile_arg,
                command_arg(out)
            ),
            format!(
                "powerbi-cli report build --schema {}{} --spec {} --out-dir <project-dir> --json",
                command_arg(schema),
                profile_arg,
                command_arg(out)
            ),
        ]
    } else {
        vec![format!(
            "powerbi-cli report plan --schema {} --profile <profile.json> --objective <text> --out <dashboard.json> --json",
            command_arg(schema)
        )]
    }
}
