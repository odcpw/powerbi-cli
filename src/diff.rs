use crate::relationship_tmdl::{RelationshipRecord, load_relationship_document};
use crate::tmdl::{ColumnRecord, MeasureRecord, load_table_documents};
use crate::{
    CliError, CliResult, ResolvedProject, ValidationReport, canonical_display, command_arg,
    resolve_project, validate_project,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) fn diff_command(args: &[String]) -> CliResult<Value> {
    let options = parse_diff_args(args)?;
    let before = project_snapshot(&options.before)?;
    let after = project_snapshot(&options.after)?;
    let diff = match options.scope.as_str() {
        "model.measures" => diff_measure_maps(&before.measures, &after.measures),
        "model.calculatedColumns" => {
            diff_calculated_column_maps(&before.calculated_columns, &after.calculated_columns)
        }
        "model.relationships" => {
            diff_relationship_maps(&before.relationships, &after.relationships)
        }
        _ => unreachable!("validated diff scope"),
    };
    let same = diff.changes.is_empty();

    let mut next = Vec::new();
    if let Some(first_readback) = first_readback_command(&diff.changes, &after.resolved) {
        next.push(first_readback);
    }
    next.push(format!(
        "powerbi-cli inspect --deep {} --json",
        command_arg(&after.resolved.project_dir)
    ));
    next.push(format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&after.resolved.project_dir)
    ));

    Ok(json!({
        "schema": "powerbi-cli.diff.v1",
        "ok": true,
        "exitCode": 0,
        "mode": "semantic",
        "scope": options.scope,
        "same": same,
        "identical": same,
        "summary": {
            "added": diff.added,
            "removed": diff.removed,
            "modified": diff.modified,
            "unchanged": diff.unchanged,
            "changes": diff.changes.len()
        },
        "before": project_side_json(&before.resolved, &before.validation),
        "after": project_side_json(&after.resolved, &after.validation),
        "changes": diff.changes,
        "next": next
    }))
}

#[derive(Debug)]
struct DiffOptions {
    before: PathBuf,
    after: PathBuf,
    scope: String,
}

#[derive(Debug)]
struct ProjectSnapshot {
    resolved: ResolvedProject,
    validation: ValidationReport,
    measures: BTreeMap<String, MeasureSummary>,
    calculated_columns: BTreeMap<String, CalculatedColumnSummary>,
    relationships: BTreeMap<String, RelationshipSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MeasureSummary {
    handle: String,
    table: String,
    name: String,
    expression: String,
    format_string: Option<String>,
    display_folder: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CalculatedColumnSummary {
    handle: String,
    table: String,
    name: String,
    expression: String,
    data_type: Option<String>,
    format_string: Option<String>,
    summarize_by: Option<String>,
    display_folder: Option<String>,
    description: Option<String>,
    is_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationshipSummary {
    handle: String,
    name: String,
    from_table: String,
    from_column: String,
    to_table: String,
    to_column: String,
    cross_filtering_behavior: String,
    is_active: bool,
}

#[derive(Debug)]
struct SemanticDiff {
    added: usize,
    removed: usize,
    modified: usize,
    unchanged: usize,
    changes: Vec<Value>,
}

fn parse_diff_args(args: &[String]) -> CliResult<DiffOptions> {
    let mut paths = Vec::new();
    let mut scope = "model.measures".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--scope" => {
                scope = args
                    .get(i + 1)
                    .ok_or_else(|| {
                        CliError::invalid_args("--scope requires a value")
                            .with_hint("The first diff scope is `model.measures`.")
                            .with_suggested_command(
                                "powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --scope model.measures --json",
                            )
                    })?
                    .clone();
                i += 2;
            }
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!("unknown diff flag: {other}"))
                    .with_hint(
                        "Run `powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --json`.",
                    )
                    .with_suggested_command(
                        "powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --json",
                    ));
            }
            other => {
                paths.push(PathBuf::from(other));
                i += 1;
            }
        }
    }
    scope = match scope.as_str() {
        "model.measures" => "model.measures".to_string(),
        "model.calculatedColumns" | "model.calculated-columns" => {
            "model.calculatedColumns".to_string()
        }
        "model.relationships" => "model.relationships".to_string(),
        _ => {
            return Err(CliError::unsupported_feature(format!(
                "unsupported diff scope: {scope}"
            ))
            .with_hint("Supported semantic diff scopes are `model.measures`, `model.calculatedColumns`, and `model.relationships`.")
            .with_suggested_command(
                "powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --scope model.relationships --json",
            ));
        }
    };
    match paths.as_slice() {
        [before, after] => Ok(DiffOptions {
            before: before.clone(),
            after: after.clone(),
            scope,
        }),
        [] | [_] => Err(CliError::invalid_args("diff requires before and after project paths")
            .with_hint(
                "Run `powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --json`.",
            )
            .with_suggested_command(
                "powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --json",
            )),
        _ => Err(CliError::invalid_args("diff accepts exactly two project paths")
            .with_hint("Pass one before project and one after project.")
            .with_suggested_command(
                "powerbi-cli diff <before-project-or.pbip> <after-project-or.pbip> --json",
            )),
    }
}

fn project_snapshot(path: &Path) -> CliResult<ProjectSnapshot> {
    let resolved = resolve_project(path)?;
    let validation = validate_project(&resolved)?;
    let docs = load_table_documents(&resolved)?;
    let measures = docs
        .iter()
        .flat_map(|table| table.measures.iter())
        .map(|measure| (measure.handle(), measure_summary(measure)))
        .collect::<BTreeMap<_, _>>();
    let calculated_columns = docs
        .iter()
        .flat_map(|table| table.columns.iter())
        .filter(|column| column.is_calculated())
        .map(|column| (column.handle(), calculated_column_summary(column)))
        .collect::<BTreeMap<_, _>>();
    let relationships = load_relationship_document(&resolved)?
        .relationships
        .iter()
        .map(|relationship| (relationship.handle(), relationship_summary(relationship)))
        .collect::<BTreeMap<_, _>>();
    Ok(ProjectSnapshot {
        resolved,
        validation,
        measures,
        calculated_columns,
        relationships,
    })
}

fn project_side_json(resolved: &ResolvedProject, validation: &ValidationReport) -> Value {
    json!({
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "valid": validation.errors.is_empty(),
        "counts": {
            "tables": validation.tables,
            "measures": validation.measures,
            "relationships": validation.relationships,
            "pages": validation.pages,
            "visuals": validation.visuals,
            "boundVisuals": validation.bound_visuals
        },
        "warnings": validation.warnings,
        "errors": validation.errors
    })
}

fn diff_measure_maps(
    before: &BTreeMap<String, MeasureSummary>,
    after: &BTreeMap<String, MeasureSummary>,
) -> SemanticDiff {
    let mut added = 0;
    let mut removed = 0;
    let mut modified = 0;
    let mut unchanged = 0;
    let mut changes = Vec::new();
    let handles = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for handle in handles {
        match (before.get(&handle), after.get(&handle)) {
            (Some(left), Some(right)) if left == right => unchanged += 1,
            (Some(left), Some(right)) => {
                modified += 1;
                changes.push(modified_measure_change(left, right));
            }
            (Some(left), None) => {
                removed += 1;
                changes.push(json!({
                    "kind": "model.measure",
                    "op": "removed",
                    "handle": left.handle,
                    "table": left.table,
                    "name": left.name,
                    "fieldsChanged": [],
                    "before": measure_json(left),
                    "after": Value::Null
                }));
            }
            (None, Some(right)) => {
                added += 1;
                changes.push(json!({
                    "kind": "model.measure",
                    "op": "added",
                    "handle": right.handle,
                    "table": right.table,
                    "name": right.name,
                    "fieldsChanged": ["expression", "properties.formatString", "properties.displayFolder", "properties.description"],
                    "before": Value::Null,
                    "after": measure_json(right)
                }));
            }
            (None, None) => {}
        }
    }

    SemanticDiff {
        added,
        removed,
        modified,
        unchanged,
        changes,
    }
}

fn diff_calculated_column_maps(
    before: &BTreeMap<String, CalculatedColumnSummary>,
    after: &BTreeMap<String, CalculatedColumnSummary>,
) -> SemanticDiff {
    let mut added = 0;
    let mut removed = 0;
    let mut modified = 0;
    let mut unchanged = 0;
    let mut changes = Vec::new();
    let handles = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for handle in handles {
        match (before.get(&handle), after.get(&handle)) {
            (Some(left), Some(right)) if left == right => unchanged += 1,
            (Some(left), Some(right)) => {
                modified += 1;
                changes.push(modified_calculated_column_change(left, right));
            }
            (Some(left), None) => {
                removed += 1;
                changes.push(json!({
                    "kind": "model.calculatedColumn",
                    "op": "removed",
                    "handle": left.handle,
                    "table": left.table,
                    "name": left.name,
                    "fieldsChanged": [],
                    "before": calculated_column_json(left),
                    "after": Value::Null
                }));
            }
            (None, Some(right)) => {
                added += 1;
                changes.push(json!({
                    "kind": "model.calculatedColumn",
                    "op": "added",
                    "handle": right.handle,
                    "table": right.table,
                    "name": right.name,
                    "fieldsChanged": ["expression", "properties.dataType", "properties.formatString", "properties.summarizeBy", "properties.displayFolder", "properties.description", "properties.isHidden"],
                    "before": Value::Null,
                    "after": calculated_column_json(right)
                }));
            }
            (None, None) => {}
        }
    }

    SemanticDiff {
        added,
        removed,
        modified,
        unchanged,
        changes,
    }
}

fn diff_relationship_maps(
    before: &BTreeMap<String, RelationshipSummary>,
    after: &BTreeMap<String, RelationshipSummary>,
) -> SemanticDiff {
    let mut added = 0;
    let mut removed = 0;
    let mut modified = 0;
    let mut unchanged = 0;
    let mut changes = Vec::new();
    let handles = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for handle in handles {
        match (before.get(&handle), after.get(&handle)) {
            (Some(left), Some(right)) if left == right => unchanged += 1,
            (Some(left), Some(right)) => {
                modified += 1;
                changes.push(modified_relationship_change(left, right));
            }
            (Some(left), None) => {
                removed += 1;
                changes.push(json!({
                    "kind": "model.relationship",
                    "op": "removed",
                    "handle": left.handle,
                    "name": left.name,
                    "fieldsChanged": [],
                    "before": relationship_json(left),
                    "after": Value::Null
                }));
            }
            (None, Some(right)) => {
                added += 1;
                changes.push(json!({
                    "kind": "model.relationship",
                    "op": "added",
                    "handle": right.handle,
                    "name": right.name,
                    "fieldsChanged": ["fromTable", "fromColumn", "toTable", "toColumn", "properties.crossFilteringBehavior", "properties.isActive"],
                    "before": Value::Null,
                    "after": relationship_json(right)
                }));
            }
            (None, None) => {}
        }
    }

    SemanticDiff {
        added,
        removed,
        modified,
        unchanged,
        changes,
    }
}

fn modified_measure_change(before: &MeasureSummary, after: &MeasureSummary) -> Value {
    let mut fields = Vec::new();
    if before.expression != after.expression {
        fields.push("expression");
    }
    if before.format_string != after.format_string {
        fields.push("properties.formatString");
    }
    if before.display_folder != after.display_folder {
        fields.push("properties.displayFolder");
    }
    if before.description != after.description {
        fields.push("properties.description");
    }
    json!({
        "kind": "model.measure",
        "op": "modified",
        "handle": after.handle,
        "table": after.table,
        "name": after.name,
        "fieldsChanged": fields,
        "before": measure_json(before),
        "after": measure_json(after)
    })
}

fn modified_calculated_column_change(
    before: &CalculatedColumnSummary,
    after: &CalculatedColumnSummary,
) -> Value {
    let mut fields = Vec::new();
    if before.expression != after.expression {
        fields.push("expression");
    }
    if before.data_type != after.data_type {
        fields.push("properties.dataType");
    }
    if before.format_string != after.format_string {
        fields.push("properties.formatString");
    }
    if before.summarize_by != after.summarize_by {
        fields.push("properties.summarizeBy");
    }
    if before.display_folder != after.display_folder {
        fields.push("properties.displayFolder");
    }
    if before.description != after.description {
        fields.push("properties.description");
    }
    if before.is_hidden != after.is_hidden {
        fields.push("properties.isHidden");
    }
    json!({
        "kind": "model.calculatedColumn",
        "op": "modified",
        "handle": after.handle,
        "table": after.table,
        "name": after.name,
        "fieldsChanged": fields,
        "before": calculated_column_json(before),
        "after": calculated_column_json(after)
    })
}

fn modified_relationship_change(
    before: &RelationshipSummary,
    after: &RelationshipSummary,
) -> Value {
    let mut fields = Vec::new();
    if before.from_table != after.from_table {
        fields.push("fromTable");
    }
    if before.from_column != after.from_column {
        fields.push("fromColumn");
    }
    if before.to_table != after.to_table {
        fields.push("toTable");
    }
    if before.to_column != after.to_column {
        fields.push("toColumn");
    }
    if before.cross_filtering_behavior != after.cross_filtering_behavior {
        fields.push("properties.crossFilteringBehavior");
    }
    if before.is_active != after.is_active {
        fields.push("properties.isActive");
    }
    json!({
        "kind": "model.relationship",
        "op": "modified",
        "handle": after.handle,
        "name": after.name,
        "fieldsChanged": fields,
        "before": relationship_json(before),
        "after": relationship_json(after)
    })
}

fn first_readback_command(changes: &[Value], after: &ResolvedProject) -> Option<String> {
    changes.iter().find_map(|change| {
        let op = change["op"].as_str()?;
        let handle = change["handle"].as_str()?;
        match op {
            "added" | "modified" => Some(format!(
                "powerbi-cli model {} show --project {} --handle {} --json",
                readback_family(change["kind"].as_str().unwrap_or_default()),
                command_arg(&after.project_dir),
                shell_arg(handle)
            )),
            "removed" => {
                if change["kind"].as_str() == Some("model.relationship") {
                    return Some(format!(
                        "powerbi-cli model relationships list --project {} --json",
                        command_arg(&after.project_dir)
                    ));
                }
                let table = change["table"].as_str()?;
                Some(format!(
                    "powerbi-cli model {} list --project {} --table {} --json",
                    readback_family(change["kind"].as_str().unwrap_or_default()),
                    command_arg(&after.project_dir),
                    shell_arg(table)
                ))
            }
            _ => None,
        }
    })
}

fn readback_family(kind: &str) -> &'static str {
    match kind {
        "model.calculatedColumn" => "calculated-columns",
        "model.relationship" => "relationships",
        _ => "measures",
    }
}

fn measure_summary(measure: &MeasureRecord) -> MeasureSummary {
    MeasureSummary {
        handle: measure.handle(),
        table: measure.table.clone(),
        name: measure.name.clone(),
        expression: measure.expression.clone(),
        format_string: measure.format_string.clone(),
        display_folder: measure.display_folder.clone(),
        description: measure.description.clone(),
    }
}

fn calculated_column_summary(column: &ColumnRecord) -> CalculatedColumnSummary {
    CalculatedColumnSummary {
        handle: column.handle(),
        table: column.table.clone(),
        name: column.name.clone(),
        expression: column.expression.clone().unwrap_or_default(),
        data_type: column.data_type.clone(),
        format_string: column.format_string.clone(),
        summarize_by: column.summarize_by.clone(),
        display_folder: column.display_folder.clone(),
        description: column.description.clone(),
        is_hidden: column.is_hidden,
    }
}

fn relationship_summary(relationship: &RelationshipRecord) -> RelationshipSummary {
    RelationshipSummary {
        handle: relationship.handle(),
        name: relationship.name.clone(),
        from_table: relationship.from_table.clone(),
        from_column: relationship.from_column.clone(),
        to_table: relationship.to_table.clone(),
        to_column: relationship.to_column.clone(),
        cross_filtering_behavior: relationship.cross_filtering_behavior.clone(),
        is_active: relationship.is_active,
    }
}

fn measure_json(measure: &MeasureSummary) -> Value {
    json!({
        "handle": measure.handle,
        "table": measure.table,
        "name": measure.name,
        "expression": measure.expression,
        "properties": {
            "formatString": measure.format_string,
            "displayFolder": measure.display_folder,
            "description": measure.description
        }
    })
}

fn calculated_column_json(column: &CalculatedColumnSummary) -> Value {
    json!({
        "handle": column.handle,
        "table": column.table,
        "name": column.name,
        "expression": column.expression,
        "properties": {
            "dataType": column.data_type,
            "formatString": column.format_string,
            "summarizeBy": column.summarize_by,
            "displayFolder": column.display_folder,
            "description": column.description,
            "isHidden": column.is_hidden
        }
    })
}

fn relationship_json(relationship: &RelationshipSummary) -> Value {
    json!({
        "handle": relationship.handle,
        "name": relationship.name,
        "fromTable": relationship.from_table,
        "fromColumn": relationship.from_column,
        "toTable": relationship.to_table,
        "toColumn": relationship.to_column,
        "properties": {
            "crossFilteringBehavior": relationship.cross_filtering_behavior,
            "isActive": relationship.is_active
        }
    })
}

fn shell_arg(value: &str) -> String {
    crate::cli_support::shell_arg(value)
}
