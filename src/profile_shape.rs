//! Deterministic, evidence-backed model-shape classification.
//!
//! The classifier intentionally consumes the already-loaded schema and profile
//! values.  It never opens another file and never emits row literals: profile
//! statistics are reduced to role, relationship, coverage, and cardinality
//! evidence that a planner can carry forward.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

const NUMERIC_TYPES: &[&str] = &[
    "int",
    "integer",
    "whole",
    "whole_number",
    "int8",
    "int16",
    "int32",
    "int64",
    "double",
    "float",
    "number",
    "decimal",
    "fixed_decimal",
    "currency",
];

#[derive(Debug, Clone)]
pub(crate) struct Shape {
    pub(crate) kind: String,
    pub(crate) facts: Vec<ShapeRole>,
    pub(crate) dimensions: Vec<ShapeRole>,
    pub(crate) date_tables: Vec<ShapeDateTable>,
    pub(crate) key_candidates: Vec<ShapeKeyCandidate>,
    pub(crate) high_cardinality: Vec<ShapeHighCardinality>,
    pub(crate) warnings: Vec<Value>,
    pub(crate) hypotheses: Vec<String>,
    relationships: Vec<Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapeRole {
    pub(crate) table: String,
    pub(crate) evidence: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapeDateTable {
    pub(crate) table: String,
    pub(crate) column: String,
    pub(crate) proposed: bool,
    pub(crate) evidence: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapeKeyCandidate {
    pub(crate) table: String,
    pub(crate) column: String,
    pub(crate) uniqueness: Option<f64>,
    pub(crate) evidence: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapeHighCardinality {
    pub(crate) table: String,
    pub(crate) column: String,
    pub(crate) distinct: u64,
    pub(crate) evidence: Vec<String>,
}

impl Shape {
    pub(crate) fn into_value(self) -> Value {
        self.to_value()
    }

    pub(crate) fn to_value(&self) -> Value {
        json!({
            "kind": self.kind,
            "facts": self.facts.iter().map(ShapeRole::to_value).collect::<Vec<_>>(),
            "dimensions": self.dimensions.iter().map(ShapeRole::to_value).collect::<Vec<_>>(),
            "dateTables": self.date_tables.iter().map(ShapeDateTable::to_value).collect::<Vec<_>>(),
            "keyCandidates": self.key_candidates.iter().map(ShapeKeyCandidate::to_value).collect::<Vec<_>>(),
            "highCardinality": self.high_cardinality.iter().map(ShapeHighCardinality::to_value).collect::<Vec<_>>(),
            "warnings": &self.warnings,
            "hypotheses": &self.hypotheses,
            "relationships": &self.relationships
        })
    }
}

impl ShapeRole {
    fn to_value(&self) -> Value {
        json!({"table": &self.table, "evidence": &self.evidence})
    }
}

impl ShapeDateTable {
    fn to_value(&self) -> Value {
        json!({
            "table": &self.table,
            "column": &self.column,
            "proposed": self.proposed,
            "evidence": &self.evidence
        })
    }
}

impl ShapeKeyCandidate {
    fn to_value(&self) -> Value {
        json!({
            "table": &self.table,
            "column": &self.column,
            "uniqueness": self.uniqueness,
            "evidence": &self.evidence
        })
    }
}

impl ShapeHighCardinality {
    fn to_value(&self) -> Value {
        json!({
            "table": &self.table,
            "column": &self.column,
            "distinct": self.distinct,
            "evidence": &self.evidence
        })
    }
}

#[derive(Debug, Clone)]
struct ColumnSignal {
    name: String,
    data_type: String,
    is_key: bool,
    date_like: bool,
    numeric: bool,
    categorical: bool,
    distinct: Option<u64>,
    time_coverage: Option<TimeCoverage>,
}

#[derive(Debug, Clone)]
struct TimeCoverage {
    start: Option<String>,
    end: Option<String>,
    count: Option<u64>,
}

#[derive(Debug, Clone)]
struct TableSignal {
    name: String,
    row_count: u64,
    columns: Vec<ColumnSignal>,
    measure_count: usize,
    explicit_role: Option<String>,
    name_fact: bool,
    name_dimension: bool,
}

impl TableSignal {
    fn key_count(&self) -> usize {
        self.columns.iter().filter(|column| column.is_key).count()
    }

    fn date_columns(&self) -> Vec<&ColumnSignal> {
        self.columns
            .iter()
            .filter(|column| column.date_like)
            .collect()
    }

    fn numeric_measure_count(&self) -> usize {
        self.columns
            .iter()
            .filter(|column| column.numeric && !column.date_like && !column.is_key)
            .count()
    }

    fn numeric_share(&self) -> (usize, usize) {
        let total = self.columns.len();
        let numeric = self
            .columns
            .iter()
            .filter(|column| column.numeric && !column.date_like)
            .count();
        (numeric, total)
    }

    fn categorical_count(&self) -> usize {
        self.columns
            .iter()
            .filter(|column| column.categorical)
            .count()
    }

    fn explicit_fact(&self) -> bool {
        self.explicit_role.as_deref() == Some("fact") || self.name_fact
    }

    fn explicit_dimension(&self) -> bool {
        self.explicit_role.as_deref() == Some("dimension") || self.name_dimension
    }
}

#[derive(Debug, Clone)]
struct RelationshipSignal {
    from_table: String,
    from_column: String,
    to_table: String,
    to_column: String,
    from_cardinality: String,
    to_cardinality: String,
}

impl RelationshipSignal {
    fn evidence(&self) -> String {
        format!(
            "relationship {}[{}] -> {}[{}] has {}-to-{} cardinality",
            self.from_table,
            self.from_column,
            self.to_table,
            self.to_column,
            self.from_cardinality,
            self.to_cardinality
        )
    }

    fn key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.from_table.to_ascii_lowercase(),
            self.from_column.to_ascii_lowercase(),
            self.to_table.to_ascii_lowercase(),
            self.to_column.to_ascii_lowercase()
        )
    }
}

/// Classify a schema using the optional profile v1/v2 statistics.
///
/// `schema` is already normalized by the schema loader in command paths.  A
/// missing profile is valid: the classifier falls back to schema metadata and
/// explains that fallback in each role's evidence.  No file is opened here.
pub(crate) fn classify(schema: &Value, profile: Option<&Value>) -> Shape {
    let tables = collect_tables(schema, profile);
    let table_names = tables
        .iter()
        .map(|table| table.name.clone())
        .collect::<Vec<_>>();
    let relationships = collect_relationships(schema, profile, &table_names);

    let fact_flags = tables
        .iter()
        .map(|table| is_fact_candidate(table, &relationships, &tables))
        .collect::<BTreeMap<_, _>>();
    let dimension_flags = tables
        .iter()
        .map(|table| is_dimension_candidate(table, &relationships, &tables))
        .collect::<BTreeMap<_, _>>();
    let fact_names = fact_flags
        .iter()
        .filter(|(_, candidate)| candidate.is_candidate)
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    let dimension_names = dimension_flags
        .iter()
        .filter(|(_, candidate)| candidate.is_candidate)
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();

    let kind = classify_kind(
        tables.len(),
        &fact_names,
        &dimension_names,
        &relationships,
        &fact_flags,
    );
    let hypotheses = competing_hypotheses(&kind, tables.len(), &fact_names, &dimension_names);

    let mut facts = tables
        .iter()
        .filter(|table| fact_names.contains(&table.name))
        .map(|table| ShapeRole {
            table: table.name.clone(),
            evidence: role_evidence(
                table,
                "fact",
                fact_flags
                    .get(&table.name)
                    .expect("fact candidate has evidence"),
                &tables,
                &relationships,
                &fact_names,
                &dimension_names,
            ),
        })
        .collect::<Vec<_>>();
    let mut dimensions = tables
        .iter()
        .filter(|table| dimension_names.contains(&table.name))
        .map(|table| ShapeRole {
            table: table.name.clone(),
            evidence: role_evidence(
                table,
                "dimension",
                dimension_flags
                    .get(&table.name)
                    .expect("dimension candidate has evidence"),
                &tables,
                &relationships,
                &fact_names,
                &dimension_names,
            ),
        })
        .collect::<Vec<_>>();
    facts.sort_by_key(|left| canonical_name(&left.table));
    dimensions.sort_by_key(|left| canonical_name(&left.table));

    let date_tables = classify_date_tables(&tables, &fact_names, &dimension_names, &relationships);
    let key_candidates = key_candidates(&tables, &relationships);
    let high_cardinality = high_cardinality(&tables);
    let mut warnings = Vec::new();
    if kind == "ambiguous" {
        let joined = hypotheses.join(", ");
        warnings.push(json!({
            "code": "shape.ambiguous",
            "message": format!("model shape is ambiguous; competing hypotheses: {joined}"),
            "hypotheses": hypotheses
        }));
    }
    if tables.len() > 1 && relationships.is_empty() {
        warnings.push(json!({
            "code": "shape.no_relationships",
            "message": "multiple tables have no declared relationships; star, snowflake, or multi-fact shape cannot be proven from metadata"
        }));
    }
    if date_tables.iter().any(|date_table| date_table.proposed) {
        for date_table in date_tables.iter().filter(|date_table| date_table.proposed) {
            warnings.push(json!({
                "code": "shape.date_table_proposal",
                "message": format!(
                    "date-like column {}[{}] has no date dimension; propose a dedicated date table",
                    date_table.table, date_table.column
                ),
                "table": &date_table.table,
                "column": &date_table.column,
                "evidence": &date_table.evidence
            }));
        }
    }
    if !high_cardinality.is_empty() {
        warnings.push(json!({
            "code": "shape.high_cardinality_noise",
            "message": format!(
                "{} categorical column(s) have high distinct counts and may be noisy slicer candidates",
                high_cardinality.len()
            ),
            "columns": high_cardinality
                .iter()
                .map(|column| format!("{}[{}]", column.table, column.column))
                .collect::<Vec<_>>()
        }));
    }
    if tables.iter().all(|table| table.columns.is_empty()) {
        warnings.push(json!({
            "code": "shape.no_columns",
            "message": "tables contain no column metadata; role and shape signals are unavailable"
        }));
    }
    warnings.sort_by_key(|warning| {
        (
            warning["code"].as_str().unwrap_or_default().to_string(),
            warning["message"].as_str().unwrap_or_default().to_string(),
        )
    });

    let relationship_values = relationships
        .iter()
        .map(|relationship| {
            json!({
                "fromTable": &relationship.from_table,
                "fromColumn": &relationship.from_column,
                "toTable": &relationship.to_table,
                "toColumn": &relationship.to_column,
                "fromCardinality": &relationship.from_cardinality,
                "toCardinality": &relationship.to_cardinality,
                "evidence": relationship.evidence()
            })
        })
        .collect();

    Shape {
        kind,
        facts,
        dimensions,
        date_tables,
        key_candidates,
        high_cardinality,
        warnings,
        hypotheses,
        relationships: relationship_values,
    }
}

/// Classify a profile that carries the schema relationship metadata emitted by
/// `profile infer`.  This is the profile-only surface used by `profile
/// summarize`; it deliberately does not read the original schema path.
pub(crate) fn classify_profile(profile: &Value) -> Shape {
    let schema = json!({
        "tables": profile.get("tables").cloned().unwrap_or_else(|| Value::Array(Vec::new())),
        "relationships": profile
            .get("relationships")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()))
    });
    classify(&schema, Some(profile))
}

#[derive(Debug, Clone)]
struct RoleCandidate {
    is_candidate: bool,
    strong: bool,
    reasons: Vec<String>,
}

fn is_fact_candidate(
    table: &TableSignal,
    relationships: &[RelationshipSignal],
    tables: &[TableSignal],
) -> (String, RoleCandidate) {
    let outgoing = relationships
        .iter()
        .filter(|relationship| relationship.from_table.eq_ignore_ascii_case(&table.name))
        .count();
    let (numeric, total) = table.numeric_share();
    let mut reasons = Vec::new();
    let strong = table.explicit_fact();
    let mut candidate = strong;
    if table.explicit_role.as_deref() == Some("fact") {
        reasons.push("profile role is explicitly fact".to_string());
    }
    if table.name_fact {
        reasons.push("table name uses the Fact prefix".to_string());
    }
    if table.measure_count > 0 {
        reasons.push(format!(
            "schema declares {} measure(s), a fact-table signal",
            table.measure_count
        ));
        candidate = true;
    }
    if numeric > 0 {
        reasons.push(format!(
            "numeric column share is {numeric}/{total} ({})",
            percentage(numeric, total)
        ));
    }
    if table.numeric_measure_count() >= 2 && table.row_count > 0 {
        reasons.push(format!(
            "{} non-key numeric columns with {} profiled row(s) indicate additive measures",
            table.numeric_measure_count(),
            table.row_count
        ));
        candidate = true;
    } else if table.numeric_measure_count() > 0 && outgoing > 0 && table.row_count > 0 {
        reasons.push(format!(
            "{} non-key numeric column(s) and {} outgoing relationship(s) indicate a transactional table",
            table.numeric_measure_count(), outgoing
        ));
        candidate = true;
    }
    if outgoing > 0 {
        reasons.push(format!("relationship fan-out is {outgoing}"));
    }
    if candidate && !strong && tables.len() > 1 && outgoing == 0 && table.measure_count == 0 {
        // Numeric-only guesses with no relationship or declared measure are
        // intentionally weak; retaining the reason lets the final verdict be
        // ambiguous instead of silently selecting a fact.
        candidate = false;
    }
    if table.explicit_dimension() && !table.explicit_fact() {
        candidate = false;
    }
    (
        table.name.clone(),
        RoleCandidate {
            is_candidate: candidate,
            strong,
            reasons,
        },
    )
}

fn is_dimension_candidate(
    table: &TableSignal,
    relationships: &[RelationshipSignal],
    tables: &[TableSignal],
) -> (String, RoleCandidate) {
    let incoming = relationships
        .iter()
        .filter(|relationship| relationship.to_table.eq_ignore_ascii_case(&table.name))
        .count();
    let outgoing = relationships
        .iter()
        .filter(|relationship| relationship.from_table.eq_ignore_ascii_case(&table.name))
        .count();
    let mut reasons = Vec::new();
    let strong = table.explicit_dimension();
    let mut candidate = strong;
    if table.explicit_role.as_deref() == Some("dimension") {
        reasons.push("profile role is explicitly dimension".to_string());
    }
    if table.name_dimension {
        reasons.push("table name uses the Dim/calendar prefix".to_string());
    }
    if table.key_count() > 0 {
        reasons.push(format!(
            "{} declared key column(s) provide a dimension-key signal",
            table.key_count()
        ));
        candidate = true;
    }
    if table.categorical_count() > 0 {
        reasons.push(format!(
            "{} categorical column(s) provide descriptive-attribute signals",
            table.categorical_count()
        ));
    }
    if incoming > 0 {
        reasons.push(format!("relationship fan-in is {incoming}"));
        candidate = true;
    }
    if outgoing > 0 {
        reasons.push(format!("relationship fan-out is {outgoing}"));
    }
    // A table with a fact name or explicit fact role must not also become a
    // dimension merely because it has foreign-key-shaped numeric columns.
    if table.explicit_fact() {
        candidate = false;
    }
    if candidate && !strong && tables.len() > 1 && incoming == 0 && table.key_count() == 0 {
        candidate = false;
    }
    (
        table.name.clone(),
        RoleCandidate {
            is_candidate: candidate,
            strong,
            reasons,
        },
    )
}

fn role_evidence(
    table: &TableSignal,
    role: &str,
    candidate: &RoleCandidate,
    tables: &[TableSignal],
    relationships: &[RelationshipSignal],
    fact_names: &BTreeSet<String>,
    dimension_names: &BTreeSet<String>,
) -> Vec<String> {
    let mut evidence = candidate.reasons.clone();
    if !candidate.strong {
        evidence.push(
            "role is inferred from metadata/profile signals rather than an explicit declaration"
                .to_string(),
        );
    }
    if table.row_count > 0 {
        let comparison = tables
            .iter()
            .filter(|other| !other.name.eq_ignore_ascii_case(&table.name))
            .map(|other| other.row_count)
            .filter(|count| *count > 0)
            .max();
        if let Some(other_rows) = comparison {
            let ratio = table.row_count as f64 / other_rows as f64;
            evidence.push(format!(
                "row count ratio is {}x: table has {} rows vs next-largest profiled table ({other_rows})",
                ratio_string(ratio), table.row_count
            ));
        } else {
            evidence.push(format!("row count is {}", table.row_count));
        }
    } else {
        evidence.push(
            "row count is unavailable or zero; row-ratio evidence is inconclusive".to_string(),
        );
    }
    let (numeric, total) = table.numeric_share();
    evidence.push(format!(
        "numeric column share is {numeric}/{total} ({})",
        percentage(numeric, total)
    ));
    for relationship in relationships.iter().filter(|relationship| {
        relationship.from_table.eq_ignore_ascii_case(&table.name)
            || relationship.to_table.eq_ignore_ascii_case(&table.name)
    }) {
        let connected_role = if relationship.from_table.eq_ignore_ascii_case(&table.name) {
            relationship.to_table.as_str()
        } else {
            relationship.from_table.as_str()
        };
        let relation_evidence = relationship.evidence();
        let role_context =
            if fact_names.contains(connected_role) || dimension_names.contains(connected_role) {
                format!("; connected table {connected_role} is a classified model role")
            } else {
                String::new()
            };
        evidence.push(format!("{relation_evidence}{role_context}"));
    }
    if role == "fact" && table.date_columns().is_empty() {
        evidence.push("no date-like column signal was found on this fact candidate".to_string());
    }
    evidence.sort();
    evidence.dedup();
    evidence
}

fn classify_kind(
    table_count: usize,
    facts: &BTreeSet<String>,
    dimensions: &BTreeSet<String>,
    relationships: &[RelationshipSignal],
    fact_flags: &BTreeMap<String, RoleCandidate>,
) -> String {
    if table_count <= 1 {
        return "flat".to_string();
    }
    let strong_fact_count = facts
        .iter()
        .filter(|name| {
            fact_flags
                .get(*name)
                .is_some_and(|candidate| candidate.strong)
        })
        .count();
    if facts.len() >= 2 && strong_fact_count >= 2 && !relationships.is_empty() {
        return "multi-fact".to_string();
    }
    if facts.len() != 1 || dimensions.is_empty() || relationships.is_empty() {
        return "ambiguous".to_string();
    }
    let fact = facts.iter().next().expect("one fact");
    let connected = connected_role_graph(fact, facts, dimensions, relationships);
    if !dimensions
        .iter()
        .all(|dimension| connected.contains(dimension))
    {
        return "ambiguous".to_string();
    }
    let dimension_to_dimension = relationships.iter().any(|relationship| {
        dimensions.contains(&relationship.from_table) && dimensions.contains(&relationship.to_table)
    });
    if dimension_to_dimension {
        "snowflake".to_string()
    } else {
        "star".to_string()
    }
}

fn connected_role_graph(
    fact: &str,
    facts: &BTreeSet<String>,
    dimensions: &BTreeSet<String>,
    relationships: &[RelationshipSignal],
) -> BTreeSet<String> {
    let mut visited = BTreeSet::from([fact.to_string()]);
    let mut changed = true;
    while changed {
        changed = false;
        for relationship in relationships {
            let from_allowed = facts.contains(&relationship.from_table)
                || dimensions.contains(&relationship.from_table);
            let to_allowed = facts.contains(&relationship.to_table)
                || dimensions.contains(&relationship.to_table);
            if !from_allowed || !to_allowed {
                continue;
            }
            if visited.contains(&relationship.from_table)
                && visited.insert(relationship.to_table.clone())
            {
                changed = true;
            }
            if visited.contains(&relationship.to_table)
                && visited.insert(relationship.from_table.clone())
            {
                changed = true;
            }
        }
    }
    visited
}

fn competing_hypotheses(
    kind: &str,
    table_count: usize,
    facts: &BTreeSet<String>,
    dimensions: &BTreeSet<String>,
) -> Vec<String> {
    if kind != "ambiguous" || table_count <= 1 {
        return Vec::new();
    }
    let mut hypotheses = BTreeSet::new();
    if facts.len() >= 2 {
        hypotheses.insert("multi-fact".to_string());
    }
    if !facts.is_empty() && !dimensions.is_empty() {
        hypotheses.insert("snowflake".to_string());
        hypotheses.insert("star".to_string());
    }
    if hypotheses.is_empty() {
        hypotheses.insert("star".to_string());
        hypotheses.insert("snowflake".to_string());
        hypotheses.insert("multi-fact".to_string());
    }
    hypotheses.into_iter().collect()
}

fn collect_tables(schema: &Value, profile: Option<&Value>) -> Vec<TableSignal> {
    let schema_tables = value_objects(schema.get("tables"));
    let profile_tables = profile
        .and_then(|value| value.get("tables"))
        .map(|value| value_objects(Some(value)))
        .unwrap_or_default();
    let profile_by_name = profile_tables
        .iter()
        .filter_map(|table| Some((string_field(table, "name")?.to_ascii_lowercase(), *table)))
        .collect::<BTreeMap<_, _>>();

    let mut names = BTreeSet::new();
    for table in &schema_tables {
        if let Some(name) = string_field(table, "name") {
            names.insert(name.to_ascii_lowercase());
        }
    }
    if names.is_empty() {
        for table in &profile_tables {
            if let Some(name) = string_field(table, "name") {
                names.insert(name.to_ascii_lowercase());
            }
        }
    }

    let mut tables = Vec::new();
    if !schema_tables.is_empty() {
        for schema_table in schema_tables {
            let Some(name) = string_field(schema_table, "name") else {
                continue;
            };
            let profile_table = profile_by_name.get(&name.to_ascii_lowercase()).copied();
            tables.push(table_signal(schema_table, profile_table));
        }
    } else {
        for profile_table in profile_tables {
            if string_field(profile_table, "name").is_none() {
                continue;
            }
            tables.push(table_signal(profile_table, Some(profile_table)));
        }
    }
    tables.sort_by_key(|left| canonical_name(&left.name));
    tables
}

fn table_signal(
    schema_table: &Map<String, Value>,
    profile_table: Option<&Map<String, Value>>,
) -> TableSignal {
    let name = string_field(schema_table, "name")
        .or_else(|| profile_table.and_then(|table| string_field(table, "name")))
        .unwrap_or_default();
    let row_count = profile_table
        .and_then(|table| table.get("rowCount"))
        .and_then(Value::as_u64)
        .or_else(|| {
            schema_table
                .get("rows")
                .and_then(Value::as_array)
                .map(|rows| rows.len() as u64)
        })
        .unwrap_or(0);
    let profile_columns = profile_table
        .and_then(|table| table.get("columns"))
        .map(|value| value_objects(Some(value)))
        .unwrap_or_default();
    let profile_by_name = profile_columns
        .iter()
        .filter_map(|column| Some((string_field(column, "name")?.to_ascii_lowercase(), *column)))
        .collect::<BTreeMap<_, _>>();
    let schema_columns = value_objects(schema_table.get("columns"));
    let mut columns = Vec::new();
    if !schema_columns.is_empty() {
        for schema_column in schema_columns {
            let Some(column_name) = string_field(schema_column, "name") else {
                continue;
            };
            columns.push(column_signal(
                schema_column,
                profile_by_name
                    .get(&column_name.to_ascii_lowercase())
                    .copied(),
                schema_table.get("rows").and_then(Value::as_array),
            ));
        }
    } else {
        for profile_column in profile_columns {
            columns.push(column_signal(profile_column, Some(profile_column), None));
        }
    }
    columns.sort_by_key(|left| canonical_name(&left.name));
    let explicit_role = profile_table
        .and_then(|table| string_field(table, "role"))
        .or_else(|| string_field(schema_table, "role"))
        .map(|role| role.to_ascii_lowercase())
        .filter(|role| matches!(role.as_str(), "fact" | "dimension"));
    let lower_name = name.to_ascii_lowercase();
    let name_fact = lower_name.starts_with("fact") || lower_name.contains("_fact");
    let name_dimension = lower_name.starts_with("dim")
        || lower_name.starts_with("calendar")
        || lower_name.contains("date_dimension")
        || lower_name.contains("date dimension");
    let measure_count = schema_table
        .get("measures")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    TableSignal {
        name,
        row_count,
        columns,
        measure_count,
        explicit_role,
        name_fact,
        name_dimension,
    }
}

fn column_signal(
    schema_column: &Map<String, Value>,
    profile_column: Option<&Map<String, Value>>,
    rows: Option<&Vec<Value>>,
) -> ColumnSignal {
    let name = string_field(schema_column, "name")
        .or_else(|| profile_column.and_then(|column| string_field(column, "name")))
        .unwrap_or_default();
    let data_type = profile_column
        .and_then(|column| string_field(column, "dataType"))
        .or_else(|| string_field(schema_column, "dataType"))
        .unwrap_or_else(|| "string".to_string());
    let schema_is_key = schema_column
        .get("isKey")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let is_key = profile_column
        .and_then(|column| column.get("isKey"))
        .and_then(Value::as_bool)
        .unwrap_or(schema_is_key);
    let inferred_date = date_like(&name, &data_type);
    let date_like = profile_column
        .and_then(|column| column.get("roles"))
        .and_then(|roles| roles.get("dateLike"))
        .and_then(Value::as_bool)
        .unwrap_or(inferred_date);
    let inferred_numeric = numeric_type(&data_type);
    let numeric = profile_column
        .and_then(|column| column.get("roles"))
        .and_then(|roles| roles.get("numeric"))
        .and_then(Value::as_bool)
        .unwrap_or(inferred_numeric);
    let categorical = profile_column
        .and_then(|column| column.get("roles"))
        .and_then(|roles| roles.get("categorical"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| categorical_type(&name, &data_type, is_key));
    let distinct = profile_column
        .and_then(|column| column.get("distinctCount"))
        .and_then(Value::as_u64)
        .or_else(|| rows.map(|values| distinct_from_rows(values, &name)));
    let time_coverage =
        profile_column.and_then(|column| parse_time_coverage(column.get("timeCoverage")));
    ColumnSignal {
        name,
        data_type,
        is_key,
        date_like,
        numeric,
        categorical,
        distinct,
        time_coverage,
    }
}

fn classify_date_tables(
    tables: &[TableSignal],
    facts: &BTreeSet<String>,
    dimensions: &BTreeSet<String>,
    relationships: &[RelationshipSignal],
) -> Vec<ShapeDateTable> {
    let has_date_relationship = |table: &TableSignal| {
        relationships.iter().any(|relationship| {
            (relationship.from_table.eq_ignore_ascii_case(&table.name)
                && facts.contains(&relationship.to_table))
                || (relationship.to_table.eq_ignore_ascii_case(&table.name)
                    && facts.contains(&relationship.from_table))
        })
    };
    let has_actual_date_dimension = tables.iter().any(|table| {
        dimensions.contains(&table.name)
            && !table.date_columns().is_empty()
            && (table.name.to_ascii_lowercase().contains("date")
                || table.name.to_ascii_lowercase().contains("calendar")
                || has_date_relationship(table))
    });
    let mut result = Vec::new();
    for table in tables {
        let date_columns = table.date_columns();
        if date_columns.is_empty() {
            continue;
        }
        let column = preferred_date_column(&date_columns)
            .map(|column| column.name.clone())
            .unwrap_or_default();
        let mut evidence = Vec::new();
        let selected = date_columns
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&column))
            .copied();
        if let Some(selected) = selected {
            evidence.push(format!(
                "column {} has date-like signal from data type {}",
                selected.name, selected.data_type
            ));
            evidence.extend(time_coverage_evidence(selected));
        }
        let actual = has_actual_date_dimension
            && dimensions.contains(&table.name)
            && (table.name.to_ascii_lowercase().contains("date")
                || table.name.to_ascii_lowercase().contains("calendar")
                || has_date_relationship(table));
        // Once a related date dimension is known, fact-side date keys are
        // relationship evidence rather than a missing calendar proposal.
        if has_actual_date_dimension && !actual {
            continue;
        }
        if actual {
            if has_date_relationship(table) {
                evidence.push(
                    "dimension is connected to a fact table and can serve as the date table"
                        .to_string(),
                );
            } else {
                evidence.push(
                    "table name identifies a date/calendar dimension; relationship evidence is unavailable"
                        .to_string(),
                );
            }
        } else {
            evidence.push("no related date dimension was found".to_string());
            evidence.push(
                "proposal: add a dedicated date table and relate this date-like column".to_string(),
            );
        }
        evidence.sort();
        evidence.dedup();
        result.push(ShapeDateTable {
            table: table.name.clone(),
            column,
            proposed: !actual,
            evidence,
        });
    }
    result.sort_by(|left, right| {
        canonical_name(&left.table)
            .cmp(&canonical_name(&right.table))
            .then_with(|| canonical_name(&left.column).cmp(&canonical_name(&right.column)))
    });
    result
}

fn preferred_date_column<'a>(columns: &'a [&'a ColumnSignal]) -> Option<&'a ColumnSignal> {
    columns.iter().copied().min_by_key(|column| {
        (
            if column.data_type.to_ascii_lowercase().contains("date")
                || column.data_type.to_ascii_lowercase().contains("time")
            {
                0
            } else {
                1
            },
            if column.is_key { 1 } else { 0 },
            canonical_name(&column.name),
        )
    })
}

fn time_coverage_evidence(column: &ColumnSignal) -> Vec<String> {
    let Some(coverage) = column.time_coverage.as_ref() else {
        return vec!["date coverage is unavailable in the supplied profile".to_string()];
    };
    let range = match (coverage.start.as_deref(), coverage.end.as_deref()) {
        (Some(start), Some(end)) => format!("{start} to {end}"),
        (Some(start), None) => format!("starting {start}"),
        (None, Some(end)) => format!("ending {end}"),
        (None, None) => "range unavailable".to_string(),
    };
    let count = coverage
        .count
        .map(|count| format!(" across {count} observations"))
        .unwrap_or_default();
    vec![format!("date coverage {range}{count}")]
}

fn key_candidates(
    tables: &[TableSignal],
    relationships: &[RelationshipSignal],
) -> Vec<ShapeKeyCandidate> {
    let mut result = Vec::new();
    for table in tables {
        let related_columns = relationships
            .iter()
            .filter(|relationship| {
                relationship.to_table.eq_ignore_ascii_case(&table.name)
                    || relationship.from_table.eq_ignore_ascii_case(&table.name)
            })
            .map(|relationship| {
                if relationship.to_table.eq_ignore_ascii_case(&table.name) {
                    relationship.to_column.as_str()
                } else {
                    relationship.from_column.as_str()
                }
            })
            .collect::<BTreeSet<_>>();
        for column in &table.columns {
            let name_signal = column.name.to_ascii_lowercase().ends_with("id")
                || column.name.to_ascii_lowercase().ends_with("key")
                || column.name.to_ascii_lowercase().contains("_id")
                || column.name.to_ascii_lowercase().contains("_key");
            if !column.is_key && !related_columns.contains(column.name.as_str()) && !name_signal {
                continue;
            }
            let uniqueness = match (column.distinct, table.row_count) {
                (Some(distinct), rows) if rows > 0 => {
                    Some((distinct as f64 / rows as f64).min(1.0))
                }
                _ => None,
            };
            let mut evidence = Vec::new();
            if column.is_key {
                evidence.push("schema/profile marks the column as a key".to_string());
            }
            if related_columns.contains(column.name.as_str()) {
                evidence.push(
                    "relationship endpoint identifies the column as a key candidate".to_string(),
                );
            }
            if name_signal {
                evidence.push("column name uses an id/key suffix signal".to_string());
            }
            match uniqueness {
                Some(value) => evidence.push(format!(
                    "distinct count {} / row count {} = uniqueness {}",
                    column.distinct.unwrap_or_default(),
                    table.row_count,
                    ratio_string(value)
                )),
                None => evidence.push(
                    "uniqueness is unavailable because no positive row count was profiled"
                        .to_string(),
                ),
            }
            evidence.sort();
            evidence.dedup();
            result.push(ShapeKeyCandidate {
                table: table.name.clone(),
                column: column.name.clone(),
                uniqueness,
                evidence,
            });
        }
    }
    result.sort_by(|left, right| {
        canonical_name(&left.table)
            .cmp(&canonical_name(&right.table))
            .then_with(|| canonical_name(&left.column).cmp(&canonical_name(&right.column)))
    });
    result
}

fn high_cardinality(tables: &[TableSignal]) -> Vec<ShapeHighCardinality> {
    let mut result = Vec::new();
    for table in tables {
        for column in &table.columns {
            let Some(distinct) = column.distinct else {
                continue;
            };
            if column.is_key || column.date_like || column.numeric || !column.categorical {
                continue;
            }
            let ratio = if table.row_count > 0 {
                distinct as f64 / table.row_count as f64
            } else {
                0.0
            };
            if distinct < 50 && !(table.row_count >= 10 && ratio >= 0.8) {
                continue;
            }
            let mut evidence = vec![format!("categorical column has {distinct} distinct values")];
            if table.row_count > 0 {
                evidence.push(format!(
                    "distinct-to-row ratio is {} ({distinct}/{})",
                    ratio_string(ratio),
                    table.row_count
                ));
            }
            evidence.push("high-cardinality text may be noise for slicers or grouping".to_string());
            result.push(ShapeHighCardinality {
                table: table.name.clone(),
                column: column.name.clone(),
                distinct,
                evidence,
            });
        }
    }
    result.sort_by(|left, right| {
        canonical_name(&left.table)
            .cmp(&canonical_name(&right.table))
            .then_with(|| canonical_name(&left.column).cmp(&canonical_name(&right.column)))
    });
    result
}

fn collect_relationships(
    schema: &Value,
    profile: Option<&Value>,
    table_names: &[String],
) -> Vec<RelationshipSignal> {
    let source = schema
        .get("relationships")
        .and_then(Value::as_array)
        .filter(|relationships| !relationships.is_empty())
        .cloned()
        .or_else(|| {
            profile
                .and_then(|value| value.get("relationships"))
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default();
    let mut relationships = Vec::new();
    let mut seen = BTreeSet::new();
    for relationship in source.iter().filter_map(Value::as_object) {
        let Some(raw_from_table) = string_field(relationship, "fromTable") else {
            continue;
        };
        let Some(raw_from_column) = string_field(relationship, "fromColumn") else {
            continue;
        };
        let Some(raw_to_table) = string_field(relationship, "toTable") else {
            continue;
        };
        let Some(raw_to_column) = string_field(relationship, "toColumn") else {
            continue;
        };
        let from_table = canonical_table_name(table_names, &raw_from_table);
        let to_table = canonical_table_name(table_names, &raw_to_table);
        let signal = RelationshipSignal {
            from_cardinality: relationship_cardinality(relationship, true, &from_table, &to_table),
            to_cardinality: relationship_cardinality(relationship, false, &from_table, &to_table),
            from_table,
            from_column: raw_from_column,
            to_table,
            to_column: raw_to_column,
        };
        if seen.insert(signal.key()) {
            relationships.push(signal);
        }
    }
    relationships.sort_by_key(RelationshipSignal::key);
    relationships
}

fn relationship_cardinality(
    relationship: &Map<String, Value>,
    from: bool,
    from_table: &str,
    to_table: &str,
) -> String {
    let direct = if from {
        "fromCardinality"
    } else {
        "toCardinality"
    };
    if let Some(value) = relationship.get(direct).and_then(Value::as_str)
        && let Some(normalized) = normalize_cardinality(value)
    {
        return normalized.to_string();
    }
    if let Some(value) = relationship.get("cardinality").and_then(Value::as_str) {
        let lower = value
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if lower.contains("manytoone") {
            return if from { "many" } else { "one" }.to_string();
        }
        if lower.contains("onetomany") {
            return if from { "one" } else { "many" }.to_string();
        }
        if lower.contains("onetoone") {
            return "one".to_string();
        }
    }
    // Relationship direction in normalized Power BI manifests is generally
    // fact -> dimension. This fallback is explicitly called inferred in the
    // evidence string because it is not a Desktop cardinality observation.
    let from_fact = from_table.to_ascii_lowercase().starts_with("fact");
    let to_fact = to_table.to_ascii_lowercase().starts_with("fact");
    if from_fact && !to_fact {
        return if from { "many" } else { "one" }.to_string();
    }
    if to_fact && !from_fact {
        return if from { "one" } else { "many" }.to_string();
    }
    "unknown".to_string()
}

fn normalize_cardinality(value: &str) -> Option<&'static str> {
    let compact = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    match compact.as_str() {
        "one" | "1" | "single" => Some("one"),
        "many" | "n" | "*" => Some("many"),
        _ => None,
    }
}

fn parse_time_coverage(value: Option<&Value>) -> Option<TimeCoverage> {
    let object = value?.as_object()?;
    let start = object
        .get("start")
        .or_else(|| object.get("min"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let end = object
        .get("end")
        .or_else(|| object.get("max"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let count = object.get("count").and_then(Value::as_u64);
    Some(TimeCoverage { start, end, count })
}

fn value_objects(value: Option<&Value>) -> Vec<&Map<String, Value>> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .collect()
}

fn string_field(object: &Map<String, Value>, name: &str) -> Option<String> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn canonical_table_name(table_names: &[String], requested: &str) -> String {
    table_names
        .iter()
        .find(|name| name.eq_ignore_ascii_case(requested))
        .cloned()
        .unwrap_or_else(|| requested.to_string())
}

fn canonical_name(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn numeric_type(data_type: &str) -> bool {
    NUMERIC_TYPES.contains(&data_type.to_ascii_lowercase().as_str())
}

fn date_like(name: &str, data_type: &str) -> bool {
    let lower_name = name.to_ascii_lowercase();
    let lower_type = data_type.to_ascii_lowercase();
    lower_type.contains("date")
        || lower_type.contains("time")
        || lower_name.contains("date")
        || lower_name.contains("datum")
        || lower_name.contains("year")
        || lower_name.contains("jahr")
        || lower_name.contains("month")
        || lower_name.contains("monat")
}

fn categorical_type(name: &str, data_type: &str, is_key: bool) -> bool {
    if is_key {
        return false;
    }
    let lower_name = name.to_ascii_lowercase();
    let lower_type = data_type.to_ascii_lowercase();
    matches!(
        lower_type.as_str(),
        "string" | "text" | "varchar" | "boolean" | "bool" | "logical"
    ) || [
        "branch", "category", "segment", "status", "type", "group", "name", "region",
    ]
    .iter()
    .any(|needle| lower_name.contains(needle))
}

fn distinct_from_rows(rows: &[Value], column_name: &str) -> u64 {
    let mut values = BTreeSet::new();
    for row in rows.iter().filter_map(Value::as_object) {
        if let Some(value) = row.get(column_name).or_else(|| {
            row.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(column_name))
                .map(|(_, value)| value)
        }) && !value.is_null()
        {
            values.insert(value.to_string());
        }
    }
    values.len() as u64
}

fn percentage(numerator: usize, denominator: usize) -> String {
    if denominator == 0 {
        "0%".to_string()
    } else {
        format!(
            "{}%",
            ((numerator as f64 / denominator as f64) * 100.0).round() as u64
        )
    }
}

fn ratio_string(value: f64) -> String {
    if !value.is_finite() {
        return "unknown".to_string();
    }
    format!("{value:.2}")
}

#[cfg(test)]
mod tests {
    use super::classify;
    use serde_json::json;

    fn table(name: &str, columns: serde_json::Value, rows: u64) -> serde_json::Value {
        json!({"name": name, "columns": columns, "rows": (0..rows).map(|_| json!({})).collect::<Vec<_>>()})
    }

    #[test]
    fn flat_shape_is_deterministic_and_has_json_fields() {
        let schema = json!({
            "tables": [table("Orders", json!([
                {"name":"OrderId","dataType":"int64","isKey":true},
                {"name":"Amount","dataType":"decimal"},
                {"name":"OrderDate","dataType":"date"}
            ]), 4)]
        });
        let first = classify(&schema, None).to_value();
        let second = classify(&schema, None).to_value();
        assert_eq!(first, second);
        assert_eq!(first["kind"], "flat");
        for field in [
            "facts",
            "dimensions",
            "dateTables",
            "keyCandidates",
            "highCardinality",
            "warnings",
        ] {
            assert!(first.get(field).is_some(), "missing shape field {field}");
        }
        assert!(
            first["dateTables"][0]["proposed"]
                .as_bool()
                .unwrap_or(false)
        );
    }

    #[test]
    fn relationship_cardinality_is_carried_into_shape_evidence() {
        let schema = json!({
            "tables": [
                {"name":"FactSales","columns":[{"name":"CustomerKey","dataType":"int64"},{"name":"Revenue","dataType":"decimal"}]},
                {"name":"DimCustomer","columns":[{"name":"CustomerKey","dataType":"int64","isKey":true},{"name":"Name","dataType":"string"}]}
            ],
            "relationships": [{"fromTable":"FactSales","fromColumn":"CustomerKey","toTable":"DimCustomer","toColumn":"CustomerKey","cardinality":"manyToOne"}]
        });
        let shape = classify(&schema, None).to_value();
        assert_eq!(shape["kind"], "star");
        let evidence = shape["facts"][0]["evidence"].to_string();
        assert!(evidence.contains("many-to-one"));
        assert_eq!(shape["relationships"][0]["fromCardinality"], "many");
        assert_eq!(shape["relationships"][0]["toCardinality"], "one");
    }

    #[test]
    fn ambiguous_shape_names_competing_hypotheses() {
        let schema = json!({
            "tables": [
                {"name":"Events","columns":[{"name":"Value","dataType":"decimal"}]},
                {"name":"Lookup","columns":[{"name":"Code","dataType":"string"}]}
            ]
        });
        let shape = classify(&schema, None).to_value();
        assert_eq!(shape["kind"], "ambiguous");
        assert!(
            shape["hypotheses"]
                .as_array()
                .is_some_and(|items| items.len() >= 2)
        );
        assert_eq!(shape["warnings"][0]["code"], "shape.ambiguous");
    }

    #[test]
    fn high_cardinality_profile_signal_is_reported_without_values() {
        let schema = json!({
            "tables": [{
                "name": "FactEvents",
                "columns": [
                    {"name":"EventId","dataType":"int64"},
                    {"name":"Label","dataType":"string"}
                ]
            }]
        });
        let profile = json!({
            "schema": "powerbi-cli.dataProfile.v2",
            "dataValues": false,
            "tables": [{
                "name": "FactEvents",
                "role": "fact",
                "rowCount": 100,
                "columns": [
                    {"name":"EventId","dataType":"int64","isKey":false,"distinctCount":100,"roles":{"numeric":true,"dateLike":false,"categorical":false}},
                    {"name":"Label","dataType":"string","isKey":false,"distinctCount":100,"roles":{"numeric":false,"dateLike":false,"categorical":true}}
                ]
            }]
        });
        let shape = classify(&schema, Some(&profile)).to_value();
        assert_eq!(shape["kind"], "flat");
        assert_eq!(shape["highCardinality"][0]["column"], "Label");
        assert_eq!(shape["highCardinality"][0]["distinct"], 100);
        assert!(!shape.to_string().contains("label-value"));
    }
}
