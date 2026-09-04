use crate::input_safety::{InputKind, read_utf8, validate_text};
use crate::profile::{load_profile_value, profile_summary, validate_profile_value};
use crate::project_io::write_json_pretty;
use crate::report_build::compile_dashboard_summary;
use crate::schema::{load_schema_value, validate_schema_value};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
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

/// The normalized, agent-facing report intent contract.
///
/// JSON intent files and lightly structured Markdown are deliberately reduced
/// to the same shape before planning. Fields that the starter planner cannot
/// compile yet remain present so an agent can carry the intent forward; the
/// planner emits an explicit warning for those fields instead of silently
/// discarding them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Intent {
    schema: String,
    source: String,
    format: String,
    text: String,
    audience: Option<String>,
    questions: Vec<String>,
    kpis: Vec<IntentKpi>,
    comparisons: Vec<String>,
    periods: Vec<String>,
    drill_paths: Vec<String>,
    alerts: Vec<IntentAlert>,
    filter_dimensions: Vec<String>,
    preferred_archetypes: Vec<String>,
    page_flow: Vec<String>,
    handoff: Option<IntentHandoff>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IntentKpi {
    name: String,
    measure: Option<String>,
    target: Option<Value>,
    #[serde(skip)]
    pointer: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IntentAlert {
    measure: String,
    op: String,
    threshold: Value,
    semantic: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IntentHandoff {
    target: String,
    source_kinds: Vec<String>,
}

#[derive(Debug)]
struct LoadedIntent {
    intent: Intent,
    warnings: Vec<Value>,
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
    let loaded_intent = load_intent(options.intent.as_deref(), options.objective.as_deref())?;
    let model = PlanModel::new(&schema_value, profile_value.as_ref());
    let (mut planned, intent) = build_dashboard_plan(&schema_value, &model, loaded_intent.intent)?;
    planned.warnings.extend(loaded_intent.warnings);
    sort_intent_warnings(&mut planned.warnings);
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
        "intent": intent,
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
    mut intent: Intent,
) -> CliResult<(PlannedDashboard, Intent)> {
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

    resolve_intent_kpis(&mut intent, &measures, &mut decisions)?;
    let primary_measure = intent
        .kpis
        .iter()
        .filter_map(|kpi| kpi.measure.as_deref())
        .find_map(|reference| {
            measures.iter().find(|measure| {
                measure.reference.eq_ignore_ascii_case(reference)
                    || measure.name.eq_ignore_ascii_case(reference)
            })
        })
        .cloned()
        .or_else(|| measures.first().cloned())
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
            "description": format!("Agent-planned dashboard. Objective: {}", one_line(&intent.text)),
            "questions": intent_questions(&intent),
            "audience": intent.audience.as_deref().unwrap_or("agent-authored Power BI users")
        },
        "pages": pages,
        "proof": {
            "required": "desktop-canvas-refresh"
        }
    });
    if !generated_measures.is_empty() {
        spec["model"] = json!({ "measures": generated_measures });
    }

    warnings.extend(unconsumed_intent_warnings(&intent));
    Ok((
        PlannedDashboard {
            spec,
            decisions,
            warnings,
        },
        intent,
    ))
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

const INTENT_SCHEMA: &str = "intent.v1";
const INTENT_OWNER_RULES: &str = "pbi-t6-planner-v2-szr.4";

fn load_intent(intent: Option<&str>, objective: Option<&str>) -> CliResult<LoadedIntent> {
    if let Some(objective) = objective.filter(|value| !value.trim().is_empty()) {
        validate_text(objective, InputKind::Intent)?;
        return Ok(LoadedIntent {
            intent: Intent::from_objective(objective.trim()),
            warnings: Vec::new(),
        });
    }
    let Some(intent) = intent.filter(|value| !value.trim().is_empty()) else {
        return Err(CliError::invalid_args(
            "report plan requires --intent <intent.md|intent.json|text> or --objective <text>",
        )
        .with_hint("Give the planner an intent.v1 document or business objective to optimize for.")
        .with_suggested_command(
            "powerbi-cli report plan --schema <schema.json> --profile <profile.json> --intent <intent.md|intent.json> --out dashboard.json --json",
        ));
    };
    let path = Path::new(intent);
    let raw = if path.is_file() {
        read_utf8(path, InputKind::Intent)?
    } else {
        validate_text(intent, InputKind::Intent)?;
        intent.to_string()
    };
    let format = if looks_like_json(&raw) {
        "json"
    } else {
        "markdown"
    };
    let mut loaded = parse_intent_document(&raw, format)?;
    // Keep the long-standing source value (`intent`) for callers that used
    // the free-form form, while exposing the concrete parser format.
    loaded.intent.source = "intent".to_string();
    loaded.intent.format = format.to_string();
    Ok(loaded)
}

impl Intent {
    fn empty(source: &str, format: &str, text: &str) -> Self {
        Self {
            schema: INTENT_SCHEMA.to_string(),
            source: source.to_string(),
            format: format.to_string(),
            text: text.to_string(),
            audience: None,
            questions: Vec::new(),
            kpis: Vec::new(),
            comparisons: Vec::new(),
            periods: Vec::new(),
            drill_paths: Vec::new(),
            alerts: Vec::new(),
            filter_dimensions: Vec::new(),
            preferred_archetypes: Vec::new(),
            page_flow: Vec::new(),
            handoff: None,
        }
    }

    fn from_objective(text: &str) -> Self {
        let mut intent = Self::empty("objective", "objective", text);
        intent.questions = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| line.trim_start_matches(['-', '*', '+', ' ']).to_string())
            .collect();
        if intent.questions.is_empty() {
            intent.questions.push(one_line(text));
        }
        intent
    }
}

fn looks_like_json(raw: &str) -> bool {
    raw.trim_start().starts_with(['{', '['])
}

fn parse_intent_document(raw: &str, format: &str) -> CliResult<LoadedIntent> {
    if format == "json" {
        let value = serde_json::from_str::<Value>(raw).map_err(|error| {
            intent_error(
                "/",
                format!(
                    "intent JSON is malformed near line {}, column {}: {error}",
                    error.line(),
                    error.column()
                ),
            )
        })?;
        parse_json_intent(raw, value)
    } else {
        Ok(parse_markdown_intent(raw))
    }
}

fn parse_json_intent(raw: &str, value: Value) -> CliResult<LoadedIntent> {
    let object = value
        .as_object()
        .ok_or_else(|| intent_error("/", "intent.v1 must be a JSON object"))?;
    if let Some(schema) = object.get("schema") {
        let schema = schema
            .as_str()
            .ok_or_else(|| intent_error("/schema", "intent schema must be a string"))?;
        if schema != INTENT_SCHEMA && schema != "powerbi-cli.intent.v1" {
            return Err(intent_error(
                "/schema",
                format!("unsupported intent schema `{schema}`; expected {INTENT_SCHEMA}"),
            ));
        }
    }

    let mut warnings = Vec::new();
    let known_fields = [
        "schema",
        "source",
        "format",
        "text",
        "audience",
        "questions",
        "kpis",
        "comparisons",
        "periods",
        "drillPaths",
        "alerts",
        "filterDimensions",
        "preferredArchetypes",
        "pageFlow",
        "handoff",
    ];
    for key in object.keys() {
        if !known_fields.contains(&key.as_str()) {
            warnings.push(intent_warning(
                "intent.unknown_field",
                &format!("/{}", escape_pointer_token(key)),
                format!("unknown intent field `{key}` is preserved nowhere by report plan"),
                INTENT_OWNER_RULES,
            ));
        }
    }

    let audience = optional_json_string(object, "audience", "/audience")?;
    let questions = json_string_array(object, "questions", "/questions")?;
    let comparisons = json_string_array(object, "comparisons", "/comparisons")?;
    let periods = json_string_array(object, "periods", "/periods")?;
    let drill_paths = json_string_array(object, "drillPaths", "/drillPaths")?;
    let filter_dimensions = json_string_array(object, "filterDimensions", "/filterDimensions")?;
    let preferred_archetypes =
        json_string_array(object, "preferredArchetypes", "/preferredArchetypes")?;
    let page_flow = json_string_array(object, "pageFlow", "/pageFlow")?;
    let kpis = json_kpis(object, &mut warnings)?;
    let alerts = json_alerts(object, &mut warnings)?;
    let handoff = json_handoff(object)?;

    if let Some(text) = object.get("text")
        && !text.is_null()
        && !text.is_string()
    {
        return Err(intent_error("/text", "intent text must be a string"));
    }
    Ok(LoadedIntent {
        intent: Intent {
            schema: INTENT_SCHEMA.to_string(),
            source: "intent".to_string(),
            format: "json".to_string(),
            text: raw.to_string(),
            audience,
            questions,
            kpis,
            comparisons,
            periods,
            drill_paths,
            alerts,
            filter_dimensions,
            preferred_archetypes,
            page_flow,
            handoff,
        },
        warnings,
    })
}

fn optional_json_string(
    object: &Map<String, Value>,
    key: &str,
    pointer: &str,
) -> CliResult<Option<String>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            Ok((!value.is_empty()).then(|| value.to_string()))
        }
        Some(_) => Err(intent_error(
            pointer,
            format!("intent field `{key}` must be a string"),
        )),
    }
}

fn json_string_array(
    object: &Map<String, Value>,
    key: &str,
    pointer: &str,
) -> CliResult<Vec<String>> {
    let Some(value) = object.get(key) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        intent_error(
            pointer,
            format!("intent field `{key}` must be an array of strings"),
        )
    })?;
    values
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let value = item.as_str().ok_or_else(|| {
                intent_error(
                    &format!("{pointer}/{index}"),
                    format!("intent field `{key}` entries must be strings"),
                )
            })?;
            let value = value.trim();
            if value.is_empty() {
                return Err(intent_error(
                    &format!("{pointer}/{index}"),
                    format!("intent field `{key}` entries cannot be empty"),
                ));
            }
            Ok(value.to_string())
        })
        .collect()
}

fn json_kpis(object: &Map<String, Value>, warnings: &mut Vec<Value>) -> CliResult<Vec<IntentKpi>> {
    let Some(value) = object.get("kpis") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| intent_error("/kpis", "intent field `kpis` must be an array of objects"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let pointer = format!("/kpis/{index}");
            let item = item
                .as_object()
                .ok_or_else(|| intent_error(&pointer, "each KPI must be an object with a name"))?;
            for key in item.keys() {
                if !["name", "measure", "target"].contains(&key.as_str()) {
                    warnings.push(intent_warning(
                        "intent.unknown_field",
                        &format!("{pointer}/{}", escape_pointer_token(key)),
                        format!("unknown KPI field `{key}` is not consumed by report plan"),
                        INTENT_OWNER_RULES,
                    ));
                }
            }
            let name = required_json_string(item, "name", &format!("{pointer}/name"))?;
            let measure = optional_json_string(item, "measure", &format!("{pointer}/measure"))?;
            let target = match item.get("target") {
                None | Some(Value::Null) => None,
                Some(value) if value.is_object() || value.is_array() => {
                    return Err(intent_error(
                        &format!("{pointer}/target"),
                        "KPI target must be a scalar value",
                    ));
                }
                Some(value) => Some(value.clone()),
            };
            Ok(IntentKpi {
                name,
                measure,
                target,
                pointer: format!("{pointer}/name"),
            })
        })
        .collect()
}

fn required_json_string(
    object: &Map<String, Value>,
    key: &str,
    pointer: &str,
) -> CliResult<String> {
    let value = object
        .get(key)
        .ok_or_else(|| intent_error(pointer, format!("KPI requires a non-empty `{key}` string")))?;
    let value = value
        .as_str()
        .ok_or_else(|| intent_error(pointer, format!("intent field `{key}` must be a string")))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(intent_error(
            pointer,
            format!("intent field `{key}` cannot be empty"),
        ));
    }
    Ok(value.to_string())
}

fn json_alerts(
    object: &Map<String, Value>,
    warnings: &mut Vec<Value>,
) -> CliResult<Vec<IntentAlert>> {
    let Some(value) = object.get("alerts") else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        intent_error(
            "/alerts",
            "intent field `alerts` must be an array of objects",
        )
    })?;
    values
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let pointer = format!("/alerts/{index}");
            let item = item
                .as_object()
                .ok_or_else(|| intent_error(&pointer, "each alert must be an object"))?;
            for key in item.keys() {
                if !["measure", "op", "threshold", "semantic"].contains(&key.as_str()) {
                    warnings.push(intent_warning(
                        "intent.unknown_field",
                        &format!("{pointer}/{}", escape_pointer_token(key)),
                        format!("unknown alert field `{key}` is not consumed by report plan"),
                        INTENT_OWNER_RULES,
                    ));
                }
            }
            let measure = required_json_string(item, "measure", &format!("{pointer}/measure"))?;
            let op = required_json_string(item, "op", &format!("{pointer}/op"))?;
            let threshold = item.get("threshold").ok_or_else(|| {
                intent_error(
                    &format!("{pointer}/threshold"),
                    "alert requires a threshold value",
                )
            })?;
            if threshold.is_object() || threshold.is_array() || threshold.is_null() {
                return Err(intent_error(
                    &format!("{pointer}/threshold"),
                    "alert threshold must be a scalar value",
                ));
            }
            let semantic = item
                .get("semantic")
                .filter(|value| !value.is_null())
                .cloned();
            Ok(IntentAlert {
                measure,
                op,
                threshold: threshold.clone(),
                semantic,
            })
        })
        .collect()
}

fn json_handoff(object: &Map<String, Value>) -> CliResult<Option<IntentHandoff>> {
    let Some(value) = object.get("handoff") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let item = value
        .as_object()
        .ok_or_else(|| intent_error("/handoff", "handoff must be an object"))?;
    for key in item.keys() {
        if !["target", "sourceKinds"].contains(&key.as_str()) {
            return Err(intent_error(
                &format!("/handoff/{}", escape_pointer_token(key)),
                format!("unknown handoff field `{key}`"),
            ));
        }
    }
    let target = required_json_string(item, "target", "/handoff/target")?;
    let source_kinds = json_string_array(item, "sourceKinds", "/handoff/sourceKinds")?;
    Ok(Some(IntentHandoff {
        target,
        source_kinds,
    }))
}

fn parse_markdown_intent(raw: &str) -> LoadedIntent {
    let mut sections: std::collections::BTreeMap<String, Vec<(usize, String)>> =
        std::collections::BTreeMap::new();
    let mut current: Option<String> = None;
    let mut unknown_questions = Vec::new();
    let mut unsectioned = Vec::new();
    let mut warnings = Vec::new();
    for (line_index, line) in raw.lines().enumerate() {
        let line_number = line_index + 1;
        if let Some(heading) = markdown_h2_heading(line) {
            let normalized = normalize_heading(&heading);
            if let Some(canonical) = canonical_section(&normalized) {
                current = Some(canonical.to_string());
                sections.entry(canonical.to_string()).or_default();
            } else {
                current = Some(normalized.clone());
                sections.entry(normalized.clone()).or_default();
                unknown_questions.push(heading.trim().to_string());
                warnings.push(intent_warning(
                    "intent.unknown_heading",
                    &format!("#/line/{line_number}"),
                    format!(
                        "unknown Markdown heading `{}` was promoted to a question",
                        heading.trim()
                    ),
                    INTENT_OWNER_RULES,
                ));
            }
            continue;
        }
        if line.trim_start().starts_with('#') || line.trim().starts_with("```") {
            continue;
        }
        let Some(entry) = markdown_entry(line) else {
            continue;
        };
        if let Some(section) = current.as_ref() {
            sections
                .entry(section.clone())
                .or_default()
                .push((line_number, entry));
        } else {
            unsectioned.push((line_number, entry));
        }
    }

    let audience = sections.get("audience").and_then(|entries| {
        let text = entries
            .iter()
            .map(|(_, entry)| entry.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        (!text.trim().is_empty()).then(|| one_line(&text))
    });
    let mut questions: Vec<String> = sections
        .get("questions")
        .map(|entries| entries.iter().map(|(_, entry)| entry.clone()).collect())
        .unwrap_or_default();
    questions.extend(unknown_questions);
    for (section, entries) in &sections {
        if !is_known_section(section) {
            questions.extend(entries.iter().map(|(_, entry)| entry.clone()));
        }
    }
    if questions.is_empty() {
        questions.extend(unsectioned.iter().map(|(_, entry)| entry.clone()));
    }

    let kpis = sections
        .get("kpis")
        .into_iter()
        .flatten()
        .filter_map(|(line, entry)| parse_markdown_kpi(entry, *line, &mut warnings))
        .collect();
    let alerts = sections
        .get("alerts")
        .into_iter()
        .flatten()
        .filter_map(|(line, entry)| parse_markdown_alert(entry, *line, &mut warnings))
        .collect();
    let handoff = parse_markdown_handoff(sections.get("handoff"), &mut warnings);
    let text = raw.to_string();
    LoadedIntent {
        intent: Intent {
            schema: INTENT_SCHEMA.to_string(),
            source: "intent".to_string(),
            format: "markdown".to_string(),
            text,
            audience,
            questions,
            kpis,
            comparisons: markdown_strings(sections.get("comparisons")),
            periods: markdown_strings(sections.get("periods")),
            drill_paths: markdown_strings(sections.get("drillPaths")),
            alerts,
            filter_dimensions: markdown_strings(sections.get("filterDimensions")),
            preferred_archetypes: markdown_strings(sections.get("preferredArchetypes")),
            page_flow: markdown_strings(sections.get("pageFlow")),
            handoff,
        },
        warnings,
    }
}

fn markdown_h2_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let hashes = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if hashes != 2 {
        return None;
    }
    let heading = trimmed[hashes..].trim().trim_end_matches('#').trim();
    (!heading.is_empty()).then(|| heading.to_string())
}

fn markdown_entry(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("<!--") {
        return None;
    }
    let value = if let Some(value) = trimmed.strip_prefix("- ") {
        value
    } else if let Some(value) = trimmed.strip_prefix("* ") {
        value
    } else if let Some(value) = trimmed.strip_prefix("+ ") {
        value
    } else if trimmed
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count()
        > 0
        && trimmed.contains(". ")
    {
        trimmed
            .split_once(". ")
            .map(|(_, value)| value)
            .unwrap_or(trimmed)
    } else {
        trimmed
    };
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalize_heading(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn canonical_section(value: &str) -> Option<&'static str> {
    match value {
        "audience" => Some("audience"),
        "question" | "questions" | "businessquestion" | "businessquestions" => Some("questions"),
        "kpi" | "kpis" | "keyperformanceindicator" | "keyperformanceindicators" | "metrics" => {
            Some("kpis")
        }
        "comparison" | "comparisons" | "comparisongroup" | "comparisongroups" => {
            Some("comparisons")
        }
        "period" | "periods" | "timeperiod" | "timeperiods" | "timeframes" | "time" => {
            Some("periods")
        }
        "drillpath" | "drillpaths" | "drilldown" | "drilldowns" | "drillthrough"
        | "drillthroughs" | "requireddrillpath" | "requireddrillpaths" => Some("drillPaths"),
        "alert" | "alerts" | "alertrule" | "alertrules" | "conditionalformatting" => Some("alerts"),
        "filterdimension" | "filterdimensions" | "filters" => Some("filterDimensions"),
        "preferredarchetype"
        | "preferredarchetypes"
        | "visualarchetype"
        | "visualarchetypes"
        | "archetypes" => Some("preferredArchetypes"),
        "pageflow" | "pages" | "narrativeflow" | "pageorder" => Some("pageFlow"),
        "handoff" | "handoffrequirement" | "handoffrequirements" => Some("handoff"),
        _ => None,
    }
}

fn is_known_section(section: &str) -> bool {
    matches!(
        section,
        "audience"
            | "questions"
            | "kpis"
            | "comparisons"
            | "periods"
            | "drillPaths"
            | "alerts"
            | "filterDimensions"
            | "preferredArchetypes"
            | "pageFlow"
            | "handoff"
    )
}

fn markdown_strings(entries: Option<&Vec<(usize, String)>>) -> Vec<String> {
    entries
        .into_iter()
        .flatten()
        .map(|(_, entry)| entry.clone())
        .filter(|entry| !entry.trim().is_empty())
        .collect()
}

fn parse_markdown_kpi(entry: &str, line: usize, warnings: &mut Vec<Value>) -> Option<IntentKpi> {
    let (name, attributes) = split_markdown_attributes(entry);
    if name.is_empty() {
        warnings.push(intent_warning(
            "intent.invalid_kpi",
            &format!("#/line/{line}"),
            "KPI entry requires a name",
            INTENT_OWNER_RULES,
        ));
        return None;
    }
    let mut measure = None;
    let mut target = None;
    for (key, value) in attributes {
        match key.as_str() {
            "measure" => measure = (!value.is_empty()).then_some(value),
            "target" => target = Some(parse_scalar_value(&value)),
            _ => warnings.push(intent_warning(
                "intent.unknown_field",
                &format!("#/line/{line}"),
                format!("unknown KPI attribute `{key}` is not consumed by report plan"),
                INTENT_OWNER_RULES,
            )),
        }
    }
    Some(IntentKpi {
        name,
        measure,
        target,
        pointer: format!("#/line/{line}"),
    })
}

fn parse_markdown_alert(
    entry: &str,
    line: usize,
    warnings: &mut Vec<Value>,
) -> Option<IntentAlert> {
    let pointer = format!("#/line/{line}");
    let (name, attributes) = split_markdown_attributes(entry);
    if !attributes.is_empty() {
        let mut measure = None;
        let mut op = None;
        let mut threshold = None;
        let mut semantic = None;
        for (key, value) in attributes {
            match key.as_str() {
                "measure" => measure = Some(value),
                "op" | "operator" => op = Some(value),
                "threshold" | "value" => threshold = Some(parse_scalar_value(&value)),
                "semantic" | "severity" => semantic = Some(Value::String(value)),
                _ => warnings.push(intent_warning(
                    "intent.unknown_field",
                    &pointer,
                    format!("unknown alert attribute `{key}` is not consumed by report plan"),
                    INTENT_OWNER_RULES,
                )),
            }
        }
        if let (Some(measure), Some(op), Some(threshold)) = (measure, op, threshold)
            && !measure.trim().is_empty()
            && !op.trim().is_empty()
        {
            return Some(IntentAlert {
                measure: measure.trim().to_string(),
                op: op.trim().to_string(),
                threshold,
                semantic,
            });
        }
    }

    let expression = name;
    let operators = [">=", "<=", "!=", "=", ">", "<"];
    let Some((operator, index)) = operators
        .iter()
        .filter_map(|operator| expression.find(operator).map(|index| (*operator, index)))
        .min_by_key(|(_, index)| *index)
    else {
        warnings.push(intent_warning(
            "intent.invalid_alert",
            &pointer,
            "alert entry must provide measure, operator, and threshold",
            INTENT_OWNER_RULES,
        ));
        return None;
    };
    let measure = expression[..index].trim();
    let mut remainder = expression[index + operator.len()..].trim();
    let mut semantic = None;
    if let Some((threshold_text, semantic_text)) = remainder.split_once(':') {
        remainder = threshold_text.trim();
        if !semantic_text.trim().is_empty() {
            semantic = Some(Value::String(semantic_text.trim().to_string()));
        }
    }
    if measure.is_empty() || remainder.is_empty() {
        warnings.push(intent_warning(
            "intent.invalid_alert",
            &pointer,
            "alert entry must provide measure, operator, and threshold",
            INTENT_OWNER_RULES,
        ));
        return None;
    }
    Some(IntentAlert {
        measure: measure.to_string(),
        op: operator.to_string(),
        threshold: parse_scalar_value(remainder),
        semantic,
    })
}

fn split_markdown_attributes(value: &str) -> (String, Vec<(String, String)>) {
    let mut name = value.trim().to_string();
    let mut attribute_text = None;
    if let Some(start) = name.rfind(" (")
        && name.ends_with(')')
    {
        attribute_text = Some(name[start + 2..name.len() - 1].to_string());
        name.truncate(start);
    }
    // Accept a compact key/value list without requiring parentheses. This is
    // useful for Markdown handoff and alert sections, for example:
    // `measure: Revenue, op: >, threshold: 100`.
    let plain_attributes = if attribute_text.is_none()
        && name.contains(':')
        && name.split([',', ';']).all(|part| {
            part.split_once(':')
                .map(|(key, _)| is_markdown_attribute_key(key))
                .unwrap_or(false)
        }) {
        let attributes = name.clone();
        name.clear();
        Some(attributes)
    } else {
        None
    };
    let mut parts = name.split('|');
    let base = parts.next().unwrap_or_default().trim().to_string();
    let mut attributes = parts
        .chain(attribute_text.as_deref())
        .chain(plain_attributes.as_deref())
        .flat_map(|part| part.split([',', ';']))
        .filter_map(|part| {
            let (key, value) = part.split_once(':')?;
            Some((normalize_heading(key), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    attributes.retain(|(key, value)| !key.is_empty() && !value.is_empty());
    (base, attributes)
}

fn is_markdown_attribute_key(value: &str) -> bool {
    matches!(
        normalize_heading(value).as_str(),
        "measure"
            | "op"
            | "operator"
            | "threshold"
            | "value"
            | "semantic"
            | "severity"
            | "target"
            | "sourcekinds"
            | "sources"
    )
}

fn parse_scalar_value(value: &str) -> Value {
    serde_json::from_str::<Value>(value)
        .ok()
        .filter(|value| !value.is_object() && !value.is_array())
        .unwrap_or_else(|| Value::String(value.trim().to_string()))
}

fn parse_markdown_handoff(
    entries: Option<&Vec<(usize, String)>>,
    warnings: &mut Vec<Value>,
) -> Option<IntentHandoff> {
    let entries = entries?;
    let mut target = None;
    let mut source_kinds = Vec::new();
    for (line, entry) in entries {
        let (_, attributes) = split_markdown_attributes(entry);
        if attributes.is_empty() {
            if target.is_none() {
                target = Some(entry.trim().to_string());
            }
            continue;
        }
        for (key, value) in attributes {
            match key.as_str() {
                "target" => target = Some(value),
                "sourcekinds" | "sources" => source_kinds.extend(
                    value
                        .split([',', ';'])
                        .map(str::trim)
                        .filter(|kind| !kind.is_empty())
                        .map(ToOwned::to_owned),
                ),
                _ => warnings.push(intent_warning(
                    "intent.unknown_field",
                    &format!("#/line/{line}"),
                    format!("unknown handoff attribute `{key}` is not consumed by report plan"),
                    INTENT_OWNER_RULES,
                )),
            }
        }
    }
    let Some(target) = target.filter(|target| !target.trim().is_empty()) else {
        warnings.push(intent_warning(
            "intent.invalid_handoff",
            "#/handoff",
            "handoff section requires a target",
            INTENT_OWNER_RULES,
        ));
        return None;
    };
    Some(IntentHandoff {
        target,
        source_kinds,
    })
}

fn resolve_intent_kpis(
    intent: &mut Intent,
    measures: &[MeasureChoice],
    decisions: &mut Vec<Value>,
) -> CliResult<()> {
    let candidates = measures
        .iter()
        .flat_map(|measure| [measure.name.clone(), measure.reference.clone()])
        .collect::<Vec<_>>();
    for kpi in &mut intent.kpis {
        let requested = kpi.measure.as_deref().unwrap_or(&kpi.name);
        let choice = measures.iter().find(|measure| {
            measure.name.eq_ignore_ascii_case(requested)
                || measure.reference.eq_ignore_ascii_case(requested)
                || measure.name.eq_ignore_ascii_case(&kpi.name)
                || measure.reference.eq_ignore_ascii_case(&kpi.name)
        });
        let Some(choice) = choice else {
            let candidate_text = if candidates.is_empty() {
                "<none>".to_string()
            } else {
                candidates.join(", ")
            };
            return Err(CliError::new(
                "spec.missing_input",
                EXIT_VALIDATION_FAILED,
                format!(
                    "KPI `{}` does not resolve to a model measure; candidates: {candidate_text}",
                    kpi.name
                ),
            )
            .with_pointer(if kpi.pointer.is_empty() {
                "/kpis".to_string()
            } else {
                kpi.pointer.clone()
            })
            .with_hint("Rename the KPI to an existing measure or set its `measure` field to an exact model measure name.")
            .with_suggested_command(
                "powerbi-cli report spec fields --schema <schema.json> --profile <profile.json> --json",
            ));
        };
        kpi.measure = Some(choice.reference.clone());
        decisions.push(json!({
            "kind": "kpi-measure",
            "kpi": kpi.name,
            "selected": choice.reference,
            "reason": "exact case-insensitive intent KPI/name match"
        }));
    }
    Ok(())
}

fn unconsumed_intent_warnings(intent: &Intent) -> Vec<Value> {
    let mut warnings = Vec::new();
    let fields = [
        (
            "comparisons",
            !intent.comparisons.is_empty(),
            "pbi-t6-planner-v2-szr.4",
        ),
        (
            "periods",
            !intent.periods.is_empty(),
            "pbi-t6-planner-v2-szr.4",
        ),
        (
            "drillPaths",
            !intent.drill_paths.is_empty(),
            "pbi-t6-planner-v2-szr.4",
        ),
        (
            "alerts",
            !intent.alerts.is_empty(),
            "pbi-t6-planner-v2-szr.4",
        ),
        (
            "filterDimensions",
            !intent.filter_dimensions.is_empty(),
            "pbi-t3-compiler-completeness-1qi.2",
        ),
        (
            "preferredArchetypes",
            !intent.preferred_archetypes.is_empty(),
            "pbi-t6-planner-v2-szr.4",
        ),
        (
            "pageFlow",
            !intent.page_flow.is_empty(),
            "pbi-t6-planner-v2-szr.9",
        ),
        (
            "handoff",
            intent.handoff.is_some(),
            "pbi-t13-handoff-generalization-dqs.4",
        ),
    ];
    for (field, present, owner) in fields {
        if present {
            warnings.push(intent_warning(
                "intent.unconsumed_field",
                &format!("/{field}"),
                format!("intent field `{field}` is preserved but not compiled by report plan yet; owning bead {owner}"),
                owner,
            ));
        }
    }
    for (index, kpi) in intent.kpis.iter().enumerate() {
        if kpi.target.is_some() {
            warnings.push(intent_warning(
                "intent.unconsumed_field",
                &format!("/kpis/{index}/target"),
                format!("intent field `kpis[].target` is preserved but not compiled by report plan yet; owning bead {INTENT_OWNER_RULES}"),
                INTENT_OWNER_RULES,
            ));
        }
    }
    warnings
}

fn intent_warning(code: &str, pointer: &str, message: impl Into<String>, owner: &str) -> Value {
    json!({
        "code": code,
        "message": message.into(),
        "pointer": pointer,
        "owningBead": owner
    })
}

fn intent_error(pointer: &str, message: impl Into<String>) -> CliError {
    CliError::new("spec.invalid_intent", EXIT_VALIDATION_FAILED, message)
        .with_pointer(pointer)
        .with_hint("Use intent.v1 JSON or Markdown with H2 sections for audience, questions, KPIs, comparisons, periods, drill paths, and alerts.")
        .with_suggested_command(
            "powerbi-cli report plan --schema <schema.json> --intent <intent.md|intent.json> --json",
        )
}

fn sort_intent_warnings(warnings: &mut [Value]) {
    warnings.sort_by_key(|warning| {
        (
            warning["pointer"].as_str().unwrap_or_default().to_string(),
            warning["code"].as_str().unwrap_or_default().to_string(),
            warning["message"].as_str().unwrap_or_default().to_string(),
        )
    });
}

fn escape_pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
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

fn intent_questions(intent: &Intent) -> Vec<Value> {
    let mut questions = intent
        .questions
        .iter()
        .map(|question| Value::String(question.clone()))
        .take(6)
        .collect::<Vec<_>>();
    if questions.is_empty() {
        questions.push(Value::String(one_line(&intent.text)));
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
            "powerbi-cli report plan --schema {} --profile <profile.json> --intent <intent.md|intent.json> --out <dashboard.json> --json",
            command_arg(schema)
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_intent_normalizes_documented_fields_and_preserves_unconsumed_fields() {
        let raw = r#"{
            "schema": "intent.v1",
            "audience": "Finance leaders",
            "questions": ["How is revenue trending?"],
            "kpis": [{"name": "Revenue", "target": 100}],
            "comparisons": ["Region"],
            "periods": ["Last 12 months"],
            "drillPaths": ["Region -> Customer"],
            "alerts": [{"measure": "Revenue", "op": ">", "threshold": 100, "semantic": "warning"}],
            "filterDimensions": ["Region"],
            "preferredArchetypes": ["kpi-overview"],
            "pageFlow": ["Overview", "Detail"],
            "handoff": {"target": "work", "sourceKinds": ["csv"]}
        }"#;
        let loaded = parse_intent_document(raw, "json").expect("intent JSON");
        assert_eq!(loaded.intent.schema, INTENT_SCHEMA);
        assert_eq!(loaded.intent.audience.as_deref(), Some("Finance leaders"));
        assert_eq!(loaded.intent.questions, vec!["How is revenue trending?"]);
        assert_eq!(loaded.intent.kpis[0].name, "Revenue");
        assert_eq!(loaded.intent.alerts[0].op, ">");
        assert_eq!(
            loaded.intent.handoff.as_ref().expect("handoff").target,
            "work"
        );
        assert!(
            loaded.warnings.is_empty(),
            "parser should not warn for known fields"
        );
    }

    #[test]
    fn markdown_intent_maps_h2_sections_and_promotes_unknown_heading_to_question() {
        let raw = "## Audience\nFinance leaders\n\n## Questions\n- How is revenue trending?\n\n## KPIs\n- Revenue (target: 100)\n\n## Mystery\n- Which customers need attention?\n\n## Alerts\n- measure: Revenue, op: >, threshold: 100, semantic: warning\n";
        let loaded = parse_intent_document(raw, "markdown").expect("intent Markdown");
        assert_eq!(loaded.intent.audience.as_deref(), Some("Finance leaders"));
        assert_eq!(loaded.intent.kpis[0].target, Some(Value::from(100)));
        assert_eq!(loaded.intent.alerts[0].measure, "Revenue");
        assert!(
            loaded
                .intent
                .questions
                .iter()
                .any(|question| question == "Mystery")
        );
        assert!(
            loaded
                .intent
                .questions
                .iter()
                .any(|question| question == "Which customers need attention?")
        );
        assert!(
            loaded
                .warnings
                .iter()
                .any(|warning| warning["code"] == "intent.unknown_heading")
        );
    }

    #[test]
    fn markdown_alert_expression_parses_operator_threshold_and_semantic() {
        let loaded = parse_intent_document("## Alerts\n- Revenue < 100: warning\n", "markdown")
            .expect("intent Markdown");
        let alert = &loaded.intent.alerts[0];
        assert_eq!(alert.measure, "Revenue");
        assert_eq!(alert.op, "<");
        assert_eq!(alert.threshold, Value::from(100));
        assert_eq!(alert.semantic, Some(Value::from("warning")));
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn malformed_json_reports_the_failing_intent_pointer() {
        let error = parse_intent_document(
            r#"{"schema":"intent.v1","questions":"not-an-array"}"#,
            "json",
        )
        .expect_err("malformed questions must fail");
        assert_eq!(error.code, "spec.invalid_intent");
        assert_eq!(error.pointer(), Some("/questions"));
    }

    #[test]
    fn unresolved_kpi_returns_spec_missing_input_with_measure_candidates() {
        let mut intent = Intent::empty("intent", "json", "{}");
        intent.kpis.push(IntentKpi {
            name: "Missing KPI".to_string(),
            measure: None,
            target: None,
            pointer: "/kpis/0/name".to_string(),
        });
        let measures = vec![MeasureChoice {
            name: "Revenue".to_string(),
            reference: "Fact[Revenue]".to_string(),
            generated: false,
        }];
        let mut decisions = Vec::new();
        let error = resolve_intent_kpis(&mut intent, &measures, &mut decisions)
            .expect_err("unknown KPI must not silently use primary measure");
        assert_eq!(error.code, "spec.missing_input");
        assert_eq!(error.pointer(), Some("/kpis/0/name"));
        assert!(error.message.contains("Fact[Revenue]"));
    }

    #[test]
    fn intent_parser_serialization_is_deterministic() {
        let raw = "## Questions\n- What changed?\n## Unknown\n- Why?\n";
        let first = parse_intent_document(raw, "markdown").expect("first parse");
        let second = parse_intent_document(raw, "markdown").expect("second parse");
        let first_bytes = serde_json::to_vec(&first.intent).expect("serialize first");
        let second_bytes = serde_json::to_vec(&second.intent).expect("serialize second");
        assert_eq!(first_bytes, second_bytes);
        assert_eq!(first.warnings, second.warnings);
    }
}
