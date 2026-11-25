//! Lower v2 model declarations into the same typed operations used by replay.
use super::{FieldKind, FieldRef, ModelIndex};
use crate::ops::{AddRelationship, MutationPayload, Op};
use crate::relationship_tmdl::{
    default_relationship_name, normalize_cross_filtering_behavior, relationship_handle,
};
use crate::{CliError, CliResult};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

type Endpoint = (String, String, String, String);
type Column = (String, String);

pub(super) fn compile(
    root: &Map<String, Value>,
    schema: &Value,
    index: &ModelIndex,
) -> CliResult<Vec<(Op, String)>> {
    let mut ops = Vec::new();
    if root.get("schema").and_then(Value::as_str) != Some("powerbi-cli.dashboard.v2") {
        return Ok(ops);
    }
    let Some(model) = root.get("model") else {
        return Ok(ops);
    };
    let model = model
        .as_object()
        .ok_or_else(|| invalid("/model", "model must be an object"))?;
    // A manifest relationship is already authored by the scaffold. A second
    // declaration cannot safely override its cardinality, activity or direction.
    let mut endpoints: BTreeSet<Endpoint> = schema["relationships"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some((
                r["fromTable"].as_str()?.to_ascii_lowercase(),
                r["fromColumn"].as_str()?.to_ascii_lowercase(),
                r["toTable"].as_str()?.to_ascii_lowercase(),
                r["toColumn"].as_str()?.to_ascii_lowercase(),
            ))
        })
        .collect();
    let mut sorts = BTreeMap::new();
    for key in ["relationships", "sortBy"] {
        let Some(values) = model.get(key) else {
            continue;
        };
        let values = values
            .as_array()
            .ok_or_else(|| invalid(&format!("/model/{key}"), "expected an array"))?;
        for (i, value) in values.iter().enumerate() {
            let pointer = format!("/model/{key}/{i}");
            let fields = value
                .as_object()
                .ok_or_else(|| invalid(&pointer, "expected an object"))?;
            let op = if key == "relationships" {
                relationship(fields, index, &pointer, &mut endpoints)?
            } else {
                sort_by(fields, index, &pointer, &mut sorts)?
            };
            ops.push((op, pointer));
        }
    }
    Ok(ops)
}

fn invalid(pointer: &str, message: &str) -> CliError {
    CliError::invalid_args(message).with_pointer(pointer.to_owned())
}

fn column(
    fields: &Map<String, Value>,
    key: &str,
    index: &ModelIndex,
    pointer: &str,
) -> CliResult<FieldRef> {
    let pointer = format!("{pointer}/{key}");
    let reference = fields.get(key).and_then(Value::as_str).ok_or_else(|| {
        invalid(
            &pointer,
            "expected a qualified column reference, e.g. Calendar[Month]",
        )
    })?;
    let field = index
        .resolve_field(reference)
        .map_err(|e| e.with_pointer(pointer.clone()))?;
    if field.kind != FieldKind::Column {
        return Err(invalid(
            &pointer,
            "expected a model column rather than a measure",
        ));
    }
    Ok(field)
}

fn relationship(
    fields: &Map<String, Value>,
    index: &ModelIndex,
    pointer: &str,
    seen: &mut BTreeSet<Endpoint>,
) -> CliResult<Op> {
    let from = column(fields, "from", index, pointer)?;
    let to = column(fields, "to", index, pointer)?;
    let field_error = |key: &str, message: &str| invalid(&format!("{pointer}/{key}"), message);
    if from.table.eq_ignore_ascii_case(&to.table) {
        return Err(field_error(
            "to",
            "a model relationship must connect two different tables",
        ));
    }
    let from_type =
        crate::scaffold::normalize_data_type(index.column_data_type(&from.table, &from.name))?.tmdl;
    let to_type =
        crate::scaffold::normalize_data_type(index.column_data_type(&to.table, &to.name))?.tmdl;
    if from_type != to_type {
        return Err(field_error(
            "to",
            "relationship endpoints must have matching model data types",
        ));
    }
    let card = fields
        .get("cardinality")
        .map(|v| {
            v.as_str()
                .ok_or_else(|| field_error("cardinality", "expected a cardinality string"))
        })
        .transpose()?
        .unwrap_or("manyToOne");
    let (a, b) = match card {
        "manyToOne" => ("many", "one"),
        "oneToMany" => ("one", "many"),
        "oneToOne" => ("one", "one"),
        "manyToMany" => ("many", "many"),
        _ => {
            return Err(field_error(
                "cardinality",
                "expected manyToOne, oneToMany, oneToOne, or manyToMany",
            ));
        }
    };
    let cross = fields
        .get("crossFilteringBehavior")
        .map(|v| {
            let s = v.as_str().ok_or_else(|| {
                field_error(
                    "crossFilteringBehavior",
                    "expected a cross-filtering behavior string",
                )
            })?;
            normalize_cross_filtering_behavior(s)
                .map_err(|e| e.with_pointer(format!("{pointer}/crossFilteringBehavior")))
        })
        .transpose()?;
    let is_active = fields
        .get("isActive")
        .map(|v| {
            v.as_bool()
                .ok_or_else(|| field_error("isActive", "expected a boolean"))
        })
        .transpose()?;
    if !seen.insert((
        from.table.to_ascii_lowercase(),
        from.name.to_ascii_lowercase(),
        to.table.to_ascii_lowercase(),
        to.name.to_ascii_lowercase(),
    )) {
        return Err(field_error(
            "from",
            "relationship endpoints are already declared in the schema or spec",
        ));
    }
    let handle = relationship_handle(&default_relationship_name(
        &from.table,
        &from.name,
        &to.table,
        &to.name,
    ));
    Ok(Op::AddRelationship(AddRelationship {
        handle,
        from_table: from.table,
        from_column: from.name,
        to_table: to.table,
        to_column: to.name,
        from_cardinality: Some(a.into()),
        to_cardinality: Some(b.into()),
        cross_filtering_behavior: cross,
        is_active,
    }))
}

fn sort_by(
    fields: &Map<String, Value>,
    index: &ModelIndex,
    pointer: &str,
    sorts: &mut BTreeMap<Column, Column>,
) -> CliResult<Op> {
    let col = column(fields, "column", index, pointer)?;
    let by = column(fields, "by", index, pointer)?;
    let key = (
        col.table.to_ascii_lowercase(),
        col.name.to_ascii_lowercase(),
    );
    let target = (by.table.to_ascii_lowercase(), by.name.to_ascii_lowercase());
    if key.0 != target.0 || key == target {
        return Err(invalid(
            &format!("{pointer}/by"),
            "sort-by must be a different column of the same table",
        ));
    }
    if sorts.contains_key(&key) {
        return Err(invalid(
            &format!("{pointer}/column"),
            "duplicate sort-by declaration",
        ));
    }
    let mut cursor = &target;
    while let Some(next) = sorts.get(cursor) {
        if next == &key {
            return Err(invalid(
                &format!("{pointer}/by"),
                "sort-by declarations must not create a cycle",
            ));
        }
        cursor = next;
    }
    sorts.insert(key, target);
    Ok(Op::SetSortBy(MutationPayload {
        fields: BTreeMap::from([
            ("table".into(), json!(col.table)),
            ("column".into(), json!(col.name)),
            ("by".into(), json!(by.name)),
        ]),
    }))
}
