use crate::pbir_filters::FilterScope;
use crate::tmdl::{load_table_documents, parse_measure_handle, same_name};
use crate::{CliError, CliResult, ResolvedProject};
use serde_json::{Number, Value, json};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedFilterColumn {
    pub(crate) table: String,
    pub(crate) column: String,
    pub(crate) data_type: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedFilterMeasure {
    pub(crate) table: String,
    pub(crate) measure: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopNDirection {
    Top,
    Bottom,
}

impl TopNDirection {
    fn order_by_direction(self) -> u8 {
        match self {
            Self::Top => 2,
            Self::Bottom => 1,
        }
    }

    pub(crate) fn flag(self) -> &'static str {
        match self {
            Self::Top => "--top",
            Self::Bottom => "--bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelativeDateOperator {
    Last,
    Next,
    This,
}

impl RelativeDateOperator {
    pub(crate) fn parse(value: &str) -> CliResult<Self> {
        match value.to_ascii_lowercase().as_str() {
            "last" => Ok(Self::Last),
            "next" => Ok(Self::Next),
            "this" => Ok(Self::This),
            other => Err(CliError::unsupported_feature(format!(
                "unsupported --relative operator: {other}"
            ))
            .with_hint("Use --relative last, --relative next, or --relative this.")
            .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'DimDate[Date]' --relative last --unit months --span 12 --dry-run --json")),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Last => "last",
            Self::Next => "next",
            Self::This => "this",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelativeDateUnit {
    Days,
    Weeks,
    Months,
    Years,
    CalendarWeeks,
    CalendarMonths,
    CalendarYears,
}

impl RelativeDateUnit {
    pub(crate) fn parse(value: &str) -> CliResult<Self> {
        match value.to_ascii_lowercase().replace('_', "-").as_str() {
            "day" | "days" => Ok(Self::Days),
            "week" | "weeks" => Ok(Self::Weeks),
            "month" | "months" => Ok(Self::Months),
            "year" | "years" => Ok(Self::Years),
            "calendar-week" | "calendar-weeks" => Ok(Self::CalendarWeeks),
            "calendar-month" | "calendar-months" => Ok(Self::CalendarMonths),
            "calendar-year" | "calendar-years" => Ok(Self::CalendarYears),
            other => Err(CliError::unsupported_feature(format!(
                "unsupported relative-date unit: {other}"
            ))
            .with_hint("Use days, weeks, months, years, calendar-weeks, calendar-months, or calendar-years.")
            .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'DimDate[Date]' --relative last --unit months --span 12 --dry-run --json")),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Days => "days",
            Self::Weeks => "weeks",
            Self::Months => "months",
            Self::Years => "years",
            Self::CalendarWeeks => "calendar-weeks",
            Self::CalendarMonths => "calendar-months",
            Self::CalendarYears => "calendar-years",
        }
    }

    fn time_unit(self) -> u8 {
        match self {
            Self::Days => 0,
            Self::Weeks | Self::CalendarWeeks => 1,
            Self::Months | Self::CalendarMonths => 2,
            Self::Years | Self::CalendarYears => 3,
        }
    }

    fn is_calendar(self) -> bool {
        matches!(
            self,
            Self::CalendarWeeks | Self::CalendarMonths | Self::CalendarYears
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) enum FilterSpec {
    Categorical {
        values: Vec<Value>,
    },
    NumericRange {
        min: Option<Value>,
        max: Option<Value>,
    },
    TopN {
        direction: TopNDirection,
        count: u64,
        by: ResolvedFilterMeasure,
    },
    RelativeDate {
        operator: RelativeDateOperator,
        unit: RelativeDateUnit,
        span: u64,
    },
}

impl FilterSpec {
    pub(crate) fn kind_name(&self) -> &'static str {
        match self {
            Self::Categorical { .. } => "categorical",
            Self::NumericRange { .. } => "numeric-range",
            Self::TopN { .. } => "topn",
            Self::RelativeDate { .. } => "relative-date",
        }
    }

    pub(crate) fn safety_message(&self) -> &'static str {
        match self {
            Self::Categorical { .. } => {
                "Categorical filters store selected values in PBIR; use dummy/offline-safe values outside the work environment."
            }
            Self::NumericRange { .. } => {
                "Numeric range filters store threshold literals in PBIR; review them before sharing outside the work environment."
            }
            Self::TopN { .. } => {
                "TopN filters store a row limit and model-measure reference in PBIR; no selected model values are materialized by this command."
            }
            Self::RelativeDate { .. } => {
                "Relative-date filters store only relative period metadata in PBIR; no absolute model values are materialized by this command."
            }
        }
    }

    pub(crate) fn validate_for(
        &self,
        column: &ResolvedFilterColumn,
        scope: FilterScope,
    ) -> CliResult<()> {
        match self {
            Self::Categorical { values } => {
                validate_categorical_values(values, column)?;
            }
            Self::NumericRange { min, max } => {
                if !matches_numeric_type(column.data_type.as_deref()) {
                    return Err(CliError::invalid_args(format!(
                        "numeric range filter target {}[{}] must have a numeric TMDL dataType, found {}",
                        column.table,
                        column.column,
                        column.data_type.as_deref().unwrap_or("unknown")
                    ))
                    .with_hint("Choose an int64, double, decimal, currency, or integer column.")
                    .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json"));
                }
                if min.is_none() && max.is_none() {
                    return Err(CliError::invalid_args(
                        "numeric range filters require --min, --max, or both",
                    )
                    .with_hint("Use --condition-type range --min <number> [--max <number>].")
                    .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'FactSales[Revenue]' --condition-type range --min 100 --max 500 --dry-run --json"));
                }
                if let (Some(min), Some(max)) = (min, max)
                    && compare_json_numbers(min, max).is_some_and(|ordering| ordering.is_gt())
                {
                    return Err(CliError::invalid_args(
                        "numeric range --min must be less than or equal to --max",
                    )
                    .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'FactSales[Revenue]' --condition-type range --min 100 --max 500 --dry-run --json"));
                }
            }
            Self::TopN { count, .. } => {
                if scope != FilterScope::Visual {
                    return Err(CliError::unsupported_feature(
                        "TopN filter authoring is supported only for visual-owned filters",
                    )
                    .with_hint("Microsoft's filter contract permits TopN only at visual level; pass --visual <visual-handle>.")
                    .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --visual <visual-handle> --target 'DimCustomer[CustomerName]' --top 10 --by 'FactSales[Total Revenue]' --dry-run --json"));
                }
                if *count == 0 {
                    return Err(CliError::invalid_args(
                        "--top and --bottom require an integer greater than zero",
                    ));
                }
            }
            Self::RelativeDate { operator, span, .. } => {
                if !matches_date_type(column.data_type.as_deref()) {
                    return Err(CliError::invalid_args(format!(
                        "relative-date filter target {}[{}] must have a date-typed TMDL dataType, found {}",
                        column.table,
                        column.column,
                        column.data_type.as_deref().unwrap_or("unknown")
                    ))
                    .with_hint("Choose a date or dateTime column.")
                    .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json"));
                }
                if *span == 0 {
                    return Err(CliError::invalid_args(
                        "relative-date --span must be greater than zero",
                    ));
                }
                if *operator == RelativeDateOperator::This && *span != 1 {
                    return Err(CliError::invalid_args("--relative this requires --span 1")
                        .with_hint("Use last or next for multi-period relative-date filters."));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn to_pbir(
        &self,
        name: &str,
        display_name: Option<&str>,
        column: &ResolvedFilterColumn,
    ) -> CliResult<Value> {
        let mut filter = match self {
            Self::Categorical { values } => categorical_filter_json(name, column, values)?,
            Self::NumericRange { min, max } => {
                numeric_range_filter_json(name, column, min.as_ref(), max.as_ref())?
            }
            Self::TopN {
                direction,
                count,
                by,
            } => topn_filter_json(name, column, *direction, *count, by),
            Self::RelativeDate {
                operator,
                unit,
                span,
            } => relative_date_filter_json(name, column, *operator, *unit, *span),
        };
        if let Some(display_name) = display_name {
            filter["displayName"] = Value::String(display_name.to_string());
        }
        Ok(filter)
    }
}

pub(crate) fn parse_field_reference(value: &str) -> CliResult<(String, String)> {
    let value = value.trim();
    if let Some((table, rest)) = value.split_once('[')
        && let Some(field) = rest.strip_suffix(']')
        && !table.trim().is_empty()
        && !field.trim().is_empty()
    {
        return Ok((table.trim().to_string(), field.trim().to_string()));
    }
    if let Some((table, field)) = value.split_once('.')
        && !table.trim().is_empty()
        && !field.trim().is_empty()
    {
        return Ok((table.trim().to_string(), field.trim().to_string()));
    }
    Err(CliError::invalid_args(format!(
        "invalid filter target syntax: {value}"
    ))
    .with_hint("Use `Table[Column]` or `Table.Column`.")
    .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'DimCustomer[Segment]' --value Enterprise --dry-run --json"))
}

pub(crate) fn resolve_filter_column(
    resolved: &ResolvedProject,
    requested_table: &str,
    requested_column: &str,
) -> CliResult<ResolvedFilterColumn> {
    let docs = load_table_documents(resolved)?;
    let table = docs
        .iter()
        .find(|doc| same_name(&doc.table, requested_table))
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "table not found for filter target: {requested_table}"
            ))
            .with_hint("Run `inspect --deep` or `model partitions list` to discover canonical table names.")
            .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json")
        })?;
    let column = table
        .columns
        .iter()
        .find(|column| same_name(&column.name, requested_column))
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "column not found for filter target: {}[{}]",
                table.table, requested_column
            ))
            .with_hint("Run `inspect --deep` to discover canonical column names.")
            .with_suggested_command("powerbi-cli inspect --deep <project-dir-or.pbip> --json")
        })?;
    Ok(ResolvedFilterColumn {
        table: table.table.clone(),
        column: column.name.clone(),
        data_type: column.data_type.clone(),
    })
}

pub(crate) fn resolve_filter_measure(
    resolved: &ResolvedProject,
    requested: &str,
) -> CliResult<ResolvedFilterMeasure> {
    let docs = load_table_documents(resolved)?;
    let qualified = if requested.starts_with("measure:") {
        Some(parse_measure_handle(requested)?)
    } else if requested.contains('[') || requested.contains('.') {
        Some(parse_field_reference(requested)?)
    } else {
        None
    };

    if let Some((requested_table, requested_measure)) = qualified {
        let table = docs
            .iter()
            .find(|doc| same_name(&doc.table, &requested_table))
            .ok_or_else(|| {
                CliError::validation_failed(format!(
                    "table not found for TopN --by measure: {requested_table}"
                ))
                .with_suggested_command(
                    "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
                )
            })?;
        let measure = table
            .measures
            .iter()
            .find(|measure| same_name(&measure.name, &requested_measure))
            .ok_or_else(|| {
                CliError::validation_failed(format!(
                    "measure not found for TopN --by: {}[{}]",
                    table.table, requested_measure
                ))
                .with_suggested_command(
                    "powerbi-cli model measures list --project <project-dir-or.pbip> --json",
                )
            })?;
        return Ok(ResolvedFilterMeasure {
            table: table.table.clone(),
            measure: measure.name.clone(),
        });
    }

    let matches = docs
        .iter()
        .flat_map(|table| {
            table
                .measures
                .iter()
                .filter(|measure| same_name(&measure.name, requested))
                .map(|measure| ResolvedFilterMeasure {
                    table: table.table.clone(),
                    measure: measure.name.clone(),
                })
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [measure] => Ok(measure.clone()),
        [] => Err(CliError::validation_failed(format!(
            "measure not found for TopN --by: {requested}"
        ))
        .with_suggested_command("powerbi-cli model measures list --project <project-dir-or.pbip> --json")),
        _ => Err(CliError::invalid_args(format!(
            "TopN --by measure is ambiguous: {requested}"
        ))
        .with_hint("Qualify the measure as Table[Measure] or use its measure:<table>:<measure> handle.")
        .with_suggested_command("powerbi-cli model measures list --project <project-dir-or.pbip> --json")),
    }
}

pub(crate) fn parse_numeric_json(text: &str, flag: &str) -> CliResult<Value> {
    let value = serde_json::from_str::<Value>(text).map_err(|err| {
        CliError::invalid_args(format!("parse {flag}: {err}")).with_hint(format!(
            "Pass a JSON number to {flag}, for example `{flag} 100` or `{flag} 1.25`."
        ))
    })?;
    if value.is_number() {
        Ok(value)
    } else {
        Err(CliError::invalid_args(format!(
            "{flag} must be a JSON number"
        )))
    }
}

pub(crate) fn parse_values_json(text: &str) -> CliResult<Vec<Value>> {
    let value = serde_json::from_str::<Value>(text).map_err(|err| {
        CliError::invalid_args(format!("parse --values-json: {err}"))
            .with_hint("Pass a JSON array, for example `--values-json '[\"Enterprise\",\"SMB\"]'`.")
            .with_suggested_command("powerbi-cli report filters add --project <project-dir-or.pbip> --target 'DimCustomer[Segment]' --values-json '[\"Enterprise\"]' --dry-run --json")
    })?;
    let values = value.as_array().ok_or_else(|| {
        CliError::invalid_args("--values-json must be a JSON array")
            .with_hint("Pass a JSON array, for example `--values-json '[\"Enterprise\",\"SMB\"]'`.")
    })?;
    if values.is_empty() {
        return Err(CliError::invalid_args("--values-json must not be empty")
            .with_hint("Use at least one dummy/offline-safe selected value."));
    }
    Ok(values.clone())
}

pub(crate) fn validate_categorical_values(
    values: &[Value],
    column: &ResolvedFilterColumn,
) -> CliResult<()> {
    if values.is_empty() {
        return Err(CliError::invalid_args(
            "categorical filter authoring requires at least one --value, --value-json, or --values-json item",
        )
        .with_hint(
            "Use dummy/offline-safe values at home; real corporate values should stay at work.",
        ));
    }
    for value in values {
        validate_categorical_value(value, column)?;
    }
    Ok(())
}

fn validate_categorical_value(value: &Value, column: &ResolvedFilterColumn) -> CliResult<()> {
    match value {
        Value::String(_) => {
            if matches_numeric_type(column.data_type.as_deref())
                || matches_bool_type(column.data_type.as_deref())
            {
                return Err(invalid_filter_value_type(column, value));
            }
        }
        Value::Number(_) => {
            if matches_textual_type(column.data_type.as_deref())
                || matches_bool_type(column.data_type.as_deref())
                || matches_date_type(column.data_type.as_deref())
            {
                return Err(invalid_filter_value_type(column, value));
            }
        }
        Value::Bool(_) => {
            if !matches_bool_type(column.data_type.as_deref()) {
                return Err(invalid_filter_value_type(column, value));
            }
        }
        Value::Null | Value::Array(_) | Value::Object(_) => {
            return Err(CliError::invalid_args(format!(
                "report filters add supports only scalar non-null categorical values for {}[{}]",
                column.table, column.column
            ))
            .with_hint("Use --value for text, or --value-json for a JSON string, number, or boolean compatible with the target column."));
        }
    }
    Ok(())
}

fn invalid_filter_value_type(column: &ResolvedFilterColumn, value: &Value) -> CliError {
    CliError::invalid_args(format!(
        "filter value {value} is not compatible with {}[{}] data type {}",
        column.table,
        column.column,
        column.data_type.as_deref().unwrap_or("unknown")
    ))
    .with_hint("Use --value for text columns, numeric --value-json for numeric columns, and true/false --value-json for boolean columns.")
}

pub(crate) fn categorical_value_rows(values: &[Value]) -> CliResult<Vec<Value>> {
    values
        .iter()
        .map(|value| pbi_literal(value).map(|literal| json!([{ "Literal": { "Value": literal } }])))
        .collect()
}

fn categorical_filter_json(
    name: &str,
    column: &ResolvedFilterColumn,
    values: &[Value],
) -> CliResult<Value> {
    let alias = source_alias(&column.table);
    Ok(json!({
        "name": name,
        "type": "Categorical",
        "field": top_level_column(column),
        "filter": {
            "Version": 2,
            "From": [entity_source(&alias, &column.table)],
            "Where": [{
                "Condition": {
                    "In": {
                        "Expressions": [query_column(&alias, &column.column)],
                        "Values": categorical_value_rows(values)?
                    }
                }
            }]
        },
        "howCreated": "User"
    }))
}

fn numeric_range_filter_json(
    name: &str,
    column: &ResolvedFilterColumn,
    min: Option<&Value>,
    max: Option<&Value>,
) -> CliResult<Value> {
    let alias = source_alias(&column.table);
    let lower = min
        .map(|value| comparison(2, query_column(&alias, &column.column), value))
        .transpose()?;
    let upper = max
        .map(|value| comparison(4, query_column(&alias, &column.column), value))
        .transpose()?;
    let condition = match (lower, upper) {
        (Some(left), Some(right)) => json!({ "And": { "Left": left, "Right": right } }),
        (Some(condition), None) | (None, Some(condition)) => condition,
        (None, None) => unreachable!("range bounds validated before PBIR generation"),
    };
    Ok(json!({
        "name": name,
        "type": "Advanced",
        "field": top_level_column(column),
        "filter": {
            "Version": 2,
            "From": [entity_source(&alias, &column.table)],
            "Where": [{ "Condition": condition }]
        },
        "howCreated": "User"
    }))
}

fn topn_filter_json(
    name: &str,
    column: &ResolvedFilterColumn,
    direction: TopNDirection,
    count: u64,
    by: &ResolvedFilterMeasure,
) -> Value {
    let outer_alias = source_alias(&column.table);
    let target_alias = "t";
    let measure_alias = if same_name(&column.table, &by.table) {
        target_alias
    } else {
        "m"
    };
    let mut subquery_from = vec![entity_source(target_alias, &column.table)];
    if measure_alias != target_alias {
        subquery_from.push(entity_source(measure_alias, &by.table));
    }
    json!({
        "name": name,
        "type": "TopN",
        "field": top_level_column(column),
        "filter": {
            "Version": 2,
            "From": [
                {
                    "Name": "topn",
                    "Expression": {
                        "Subquery": {
                            "Query": {
                                "Version": 2,
                                "From": subquery_from,
                                "Select": [{
                                    "Column": {
                                        "Expression": { "SourceRef": { "Source": target_alias } },
                                        "Property": column.column
                                    },
                                    "Name": "field"
                                }],
                                "OrderBy": [{
                                    "Direction": direction.order_by_direction(),
                                    "Expression": {
                                        "Measure": {
                                            "Expression": { "SourceRef": { "Source": measure_alias } },
                                            "Property": by.measure
                                        }
                                    }
                                }],
                                "Top": count
                            }
                        }
                    },
                    "Type": 2
                },
                entity_source(&outer_alias, &column.table)
            ],
            "Where": [{
                "Condition": {
                    "In": {
                        "Expressions": [query_column(&outer_alias, &column.column)],
                        "Table": { "SourceRef": { "Source": "topn" } }
                    }
                }
            }]
        },
        "howCreated": "User"
    })
}

fn relative_date_filter_json(
    name: &str,
    column: &ResolvedFilterColumn,
    operator: RelativeDateOperator,
    unit: RelativeDateUnit,
    span: u64,
) -> Value {
    let alias = source_alias(&column.table);
    let (lower, upper) = relative_date_bounds(operator, unit, span);
    json!({
        "name": name,
        "type": "RelativeDate",
        "field": top_level_column(column),
        "filter": {
            "Version": 2,
            "From": [entity_source(&alias, &column.table)],
            "Where": [{
                "Condition": {
                    "Between": {
                        "Expression": query_column(&alias, &column.column),
                        "LowerBound": lower,
                        "UpperBound": upper
                    }
                }
            }]
        },
        "howCreated": "User"
    })
}

fn relative_date_bounds(
    operator: RelativeDateOperator,
    unit: RelativeDateUnit,
    span: u64,
) -> (Value, Value) {
    if unit.is_calendar() || operator == RelativeDateOperator::This {
        let first_offset = match operator {
            RelativeDateOperator::Last => {
                -i64::try_from(span.saturating_sub(1)).unwrap_or(i64::MAX)
            }
            RelativeDateOperator::Next | RelativeDateOperator::This => 0,
        };
        let last_offset = match operator {
            RelativeDateOperator::Next => i64::try_from(span.saturating_sub(1)).unwrap_or(i64::MAX),
            RelativeDateOperator::Last | RelativeDateOperator::This => 0,
        };
        return (
            start_of_period(unit.time_unit(), first_offset),
            end_of_period(unit.time_unit(), last_offset),
        );
    }

    let span = i64::try_from(span).unwrap_or(i64::MAX);
    match operator {
        RelativeDateOperator::Last => (
            date_span(date_add(date_add(now(), 1, 0), -span, unit.time_unit()), 0),
            date_span(now(), 0),
        ),
        RelativeDateOperator::Next => (
            date_span(now(), 0),
            date_span(date_add(date_add(now(), -1, 0), span, unit.time_unit()), 0),
        ),
        RelativeDateOperator::This => unreachable!("this uses aligned period bounds"),
    }
}

fn start_of_period(time_unit: u8, offset: i64) -> Value {
    let expression = if offset == 0 {
        now()
    } else {
        date_add(now(), offset, time_unit)
    };
    date_span(expression, time_unit)
}

fn end_of_period(time_unit: u8, offset: i64) -> Value {
    let start = start_of_period(time_unit, offset);
    date_span(date_add(date_add(start, 1, time_unit), -1, 0), 0)
}

fn now() -> Value {
    json!({ "Now": {} })
}

fn date_add(expression: Value, amount: i64, time_unit: u8) -> Value {
    json!({
        "DateAdd": {
            "Expression": expression,
            "Amount": amount,
            "TimeUnit": time_unit
        }
    })
}

fn date_span(expression: Value, time_unit: u8) -> Value {
    json!({
        "DateSpan": {
            "Expression": expression,
            "TimeUnit": time_unit
        }
    })
}

fn comparison(kind: u8, left: Value, right: &Value) -> CliResult<Value> {
    Ok(json!({
        "Comparison": {
            "ComparisonKind": kind,
            "Left": left,
            "Right": { "Literal": { "Value": pbi_literal(right)? } }
        }
    }))
}

fn top_level_column(column: &ResolvedFilterColumn) -> Value {
    json!({
        "Column": {
            "Expression": { "SourceRef": { "Entity": column.table } },
            "Property": column.column
        }
    })
}

fn query_column(alias: &str, column: &str) -> Value {
    json!({
        "Column": {
            "Expression": { "SourceRef": { "Source": alias } },
            "Property": column
        }
    })
}

fn entity_source(alias: &str, table: &str) -> Value {
    json!({ "Name": alias, "Entity": table, "Type": 0 })
}

pub(crate) fn pbi_literal(value: &Value) -> CliResult<String> {
    match value {
        Value::String(text) => Ok(format!("'{}'", text.replace('\'', "''"))),
        Value::Number(number) => number_literal(number, value),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => Err(CliError::invalid_args(
            "filter literal encoding supports only scalar non-null values",
        )),
    }
}

fn number_literal(number: &Number, original: &Value) -> CliResult<String> {
    if let Some(value) = number.as_i64() {
        Ok(format!("{value}L"))
    } else if let Some(value) = number.as_u64() {
        Ok(format!("{value}L"))
    } else if number.as_f64().is_some_and(f64::is_finite) {
        Ok(format!("{number}D"))
    } else {
        Err(CliError::invalid_args(format!(
            "filter value {original} cannot be encoded as a Power BI literal"
        )))
    }
}

fn source_alias(table: &str) -> String {
    table
        .chars()
        .find(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase().to_string())
        .unwrap_or_else(|| "t".to_string())
}

pub(crate) fn generated_filter_name(
    scope: FilterScope,
    column: &ResolvedFilterColumn,
    spec: &FilterSpec,
) -> String {
    let scope = match scope {
        FilterScope::Report => "Report",
        FilterScope::Page => "Page",
        FilterScope::Visual => "Visual",
        FilterScope::All => "All",
    };
    let table_id = sanitize_identifier(&column.table);
    let column_id = sanitize_identifier(&column.column);
    let kind = match spec {
        FilterSpec::Categorical { .. } => "Cat",
        FilterSpec::NumericRange { .. } => "Rng",
        FilterSpec::TopN { .. } => "Top",
        FilterSpec::RelativeDate { .. } => "Rel",
    };
    let raw_identity = format!(
        "{scope}\u{0}{}\u{0}{}\u{0}{}",
        column.table,
        column.column,
        spec.kind_name()
    );
    let condition_identity = filter_condition_identity(spec);
    let identity_hash = short_hash_hex(&raw_identity, 8);
    let condition_hash = short_hash_hex(&condition_identity, 8);
    let fixed_len = "PowerBICli".len()
        + scope.len()
        + kind.len()
        + "I".len()
        + identity_hash.len()
        + "C".len()
        + condition_hash.len()
        + "Filter".len();
    let budget = 50usize.saturating_sub(fixed_len).max(2);
    let table_budget = (budget / 2).max(1);
    let column_budget = budget.saturating_sub(table_budget).max(1);
    let table_part = truncate_ascii(&table_id, table_budget);
    let column_part = truncate_ascii(&column_id, column_budget);
    format!(
        "PowerBICli{scope}{table_part}{column_part}{kind}I{identity_hash}C{condition_hash}Filter"
    )
}

fn filter_condition_identity(spec: &FilterSpec) -> String {
    let value = match spec {
        FilterSpec::Categorical { values } => json!({
            "kind": "categorical",
            "values": values
        }),
        FilterSpec::NumericRange { min, max } => json!({
            "kind": "numeric-range",
            "min": min,
            "max": max
        }),
        FilterSpec::TopN {
            direction,
            count,
            by,
        } => json!({
            "kind": "topn",
            "direction": direction.flag(),
            "count": count,
            "byTable": by.table,
            "byMeasure": by.measure
        }),
        FilterSpec::RelativeDate {
            operator,
            unit,
            span,
        } => json!({
            "kind": "relative-date",
            "operator": operator.as_str(),
            "unit": unit.as_str(),
            "span": span
        }),
    };
    serde_json::to_string(&value).unwrap_or_default()
}

pub(crate) fn validate_filter_name(name: &str) -> CliResult<()> {
    if name.trim().is_empty() {
        return Err(invalid_filter_name());
    }
    if name.len() > 50 {
        return Err(CliError::invalid_args(
            "--name must be 50 characters or fewer for Power BI Desktop",
        )
        .with_hint("Omit --name to let powerbi-cli generate a length-safe stable filter name."));
    }
    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Ok(());
    }
    Err(invalid_filter_name())
}

fn invalid_filter_name() -> CliError {
    CliError::invalid_args(
        "--name must be non-empty and contain only ASCII letters, numbers, underscore, dash, or dot",
    )
    .with_hint("Omit --name to let powerbi-cli generate a stable filter name.")
}

fn matches_numeric_type(data_type: Option<&str>) -> bool {
    data_type
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "int64"
                    | "int32"
                    | "integer"
                    | "double"
                    | "single"
                    | "float"
                    | "decimal"
                    | "currency"
            )
        })
        .unwrap_or(false)
}

fn matches_textual_type(data_type: Option<&str>) -> bool {
    data_type
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "string"))
        .unwrap_or(false)
}

fn matches_bool_type(data_type: Option<&str>) -> bool {
    data_type
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "boolean" | "bool"))
        .unwrap_or(false)
}

fn matches_date_type(data_type: Option<&str>) -> bool {
    data_type
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "date" | "datetime" | "datetimeoffset" | "dateonly"
            )
        })
        .unwrap_or(false)
}

fn compare_json_numbers(left: &Value, right: &Value) -> Option<std::cmp::Ordering> {
    let left = left.as_number()?;
    let right = right.as_number()?;
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
        return Some(left.cmp(&right));
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
        return Some(left.cmp(&right));
    }
    left.as_f64()?.partial_cmp(&right.as_f64()?)
}

fn sanitize_identifier(value: &str) -> String {
    let out = value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    if out.is_empty() {
        "Field".to_string()
    } else {
        out
    }
}

fn truncate_ascii(value: &str, max_len: usize) -> String {
    value.chars().take(max_len).collect()
}

fn short_hash_hex(text: &str, length: usize) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}").chars().take(length).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_hash_raw_identity_and_condition_within_desktop_limit() {
        let spec = FilterSpec::Categorical {
            values: vec![Value::from("North")],
        };
        let dashed = generated_filter_name(
            FilterScope::Report,
            &ResolvedFilterColumn {
                table: "Sales-A".to_string(),
                column: "Region".to_string(),
                data_type: Some("string".to_string()),
            },
            &spec,
        );
        let spaced = generated_filter_name(
            FilterScope::Report,
            &ResolvedFilterColumn {
                table: "Sales A".to_string(),
                column: "Region".to_string(),
                data_type: Some("string".to_string()),
            },
            &spec,
        );

        assert_eq!(dashed, "PowerBICliReportSalRegiCatIc3fcad19C7ed6b99dFilter");
        assert_eq!(spaced, "PowerBICliReportSalRegiCatI55dd452fC7ed6b99dFilter");
        assert_ne!(dashed, spaced);
        assert_eq!(dashed.len(), 50);
        assert_eq!(spaced.len(), 50);
    }
}
