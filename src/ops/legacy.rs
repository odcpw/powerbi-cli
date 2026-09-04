//! Typed operation payloads for the mutation families that predate the
//! operation IR.
//!
//! The command modules remain the compatibility oracle for these operations:
//! a legacy kernel reconstructs the canonical flags from a typed, flattened
//! payload and invokes that command against the transaction working copy. The
//! command therefore keeps its existing validation and PBIR/TMDL writer while
//! the operation plan gets a stable JSON shape. The adapter is deliberately
//! narrow and only ever supplies a local `--in-place` project path; it cannot
//! reach the source tree or a remote service.

use super::{MutationPayload, Op, OpOutcome, Transaction};
use crate::cli_support::MutationMode;
use crate::{CliError, CliResult, model, report, source_template};
use serde_json::Value;
use std::collections::BTreeMap;

/// Parse an argv mutation into the flattened operation payload. The public
/// command parsers still own semantic validation; this helper only canonicalizes
/// the transport fields used by the operation layer.
pub(crate) fn parse_args(args: &[String]) -> CliResult<(MutationPayload, MutationMode)> {
    let mut fields = BTreeMap::new();
    let mut mode = None;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        match argument.as_str() {
            "--dry-run" => {
                set_mode(&mut mode, MutationMode::DryRun)?;
                index += 1;
            }
            "--in-place" => {
                set_mode(&mut mode, MutationMode::InPlace)?;
                index += 1;
            }
            "--out-dir" => {
                set_mode(&mut mode, MutationMode::OutDir)?;
                index = index.saturating_add(2);
                if index > args.len() {
                    return Err(CliError::invalid_args("--out-dir requires a value"));
                }
            }
            "--project" | "-p" => {
                index = index.saturating_add(2);
                if index > args.len() {
                    return Err(CliError::invalid_args("--project requires a value"));
                }
            }
            "--json" | "--format" => {
                // Global output flags are not operation fields. `--format`
                // consumes its value when present.
                index += if argument == "--format" { 2 } else { 1 };
            }
            value if value.starts_with("--") => {
                let (raw_key, inline) = value
                    .strip_prefix("--")
                    .and_then(|value| value.split_once('='))
                    .map_or_else(
                        || (value.strip_prefix("--").unwrap_or_default(), None),
                        |(key, value)| (key, Some(value.to_string())),
                    );
                let key = kebab_to_camel(raw_key);
                let (value, consumed) = match inline {
                    Some(value) => (Value::String(value), 1),
                    None if args
                        .get(index + 1)
                        .is_some_and(|candidate| !candidate.starts_with('-')) =>
                    {
                        (Value::String(args[index + 1].clone()), 2)
                    }
                    None => (Value::Bool(true), 1),
                };
                insert_field(&mut fields, key, value);
                index += consumed;
            }
            positional => {
                positionals.push(Value::String(positional.to_string()));
                index += 1;
            }
        }
    }
    if !positionals.is_empty() {
        fields.insert("_".to_string(), Value::Array(positionals));
    }
    let mode = mode.ok_or_else(|| {
        CliError::invalid_args(
            "mutation operation requires --dry-run, --in-place, or --out-dir <directory>",
        )
    })?;
    Ok((MutationPayload { fields }, mode))
}

fn set_mode(mode: &mut Option<MutationMode>, next: MutationMode) -> CliResult<()> {
    if mode.replace(next).is_some() {
        return Err(CliError::invalid_args(
            "choose exactly one of --dry-run, --in-place, or --out-dir",
        ));
    }
    Ok(())
}

fn insert_field(fields: &mut BTreeMap<String, Value>, key: String, value: Value) {
    match fields.get_mut(&key) {
        Some(Value::Array(values)) => values.push(value),
        Some(previous) => {
            let previous_value = std::mem::replace(previous, Value::Null);
            *previous = Value::Array(vec![previous_value, value]);
        }
        None => {
            fields.insert(key, value);
        }
    }
}

fn kebab_to_camel(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut uppercase = false;
    for character in value.chars() {
        if character == '-' {
            uppercase = true;
        } else if uppercase {
            result.extend(character.to_uppercase());
            uppercase = false;
        } else {
            result.push(character);
        }
    }
    result
}

fn camel_to_kebab(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_uppercase() {
            result.push('-');
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

/// Apply a payload through the existing command parser/writer on a staged
/// transaction tree. `command` is the canonical command path without the
/// global `--json` flag (for example `report pages add`).
pub(crate) fn apply_legacy(
    operation: &Op,
    payload: &MutationPayload,
    transaction: &mut Transaction,
    command: &[&str],
) -> CliResult<OpOutcome> {
    let mut args = command
        .iter()
        .map(|part| (*part).to_string())
        .collect::<Vec<_>>();
    append_fields(&mut args, &payload.fields);
    args.extend([
        "--project".to_string(),
        transaction.work_dir().to_string_lossy().into_owned(),
        "--in-place".to_string(),
    ]);

    let value = if command.first() == Some(&"source-template") {
        source_template::source_template_command(&args[1..])?
    } else if command.first() == Some(&"model") {
        model::model_command(&args[1..])?
    } else {
        report::report_command(&args[1..])?
    };
    let changed = value
        .get("projectModified")
        .and_then(Value::as_bool)
        .or_else(|| value.get("changed").and_then(Value::as_bool))
        .unwrap_or(true);
    let changes = value
        .get("changes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|change| rewrite_work_paths(change, transaction))
        .collect();
    let readback = value
        .get("readbackCommand")
        .and_then(Value::as_str)
        .map(|command| rewrite_work_path(command, transaction))
        .into_iter()
        .collect();
    let warnings = value
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|warning| rewrite_work_paths(warning, transaction))
        .collect();
    // Derive handles through the operation itself so the transaction receipt
    // uses exactly the same escaping/canonicalization as plan validation.
    // This matters for names containing `%` or `:` and for page names that
    // require the stable-handle slugging rules.
    let created_handles = operation
        .declared_handle_owned()
        .into_iter()
        .collect::<Vec<_>>();
    Ok(OpOutcome {
        changed,
        changes,
        readback,
        warnings,
        created_handles,
    })
}

fn rewrite_work_paths(value: Value, transaction: &Transaction) -> Value {
    match value {
        Value::String(text) => Value::String(rewrite_work_path(&text, transaction)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| rewrite_work_paths(value, transaction))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, rewrite_work_paths(value, transaction)))
                .collect(),
        ),
        scalar => scalar,
    }
}

fn rewrite_work_path(value: &str, transaction: &Transaction) -> String {
    let work = transaction.work_dir().to_string_lossy();
    let source = transaction.source.project_dir.to_string_lossy();
    value.replace(work.as_ref(), source.as_ref())
}

fn append_fields(args: &mut Vec<String>, fields: &BTreeMap<String, Value>) {
    for (key, value) in fields {
        if key == "_" {
            if let Value::Array(values) = value {
                args.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            continue;
        }
        // Transport flags are owned by the transaction boundary. A plan
        // payload must never be able to redirect a legacy command to a
        // caller-selected project or alter its output mode.
        if matches!(
            key.as_str(),
            "project" | "outDir" | "dryRun" | "inPlace" | "json" | "format"
        ) {
            continue;
        }
        // BookmarkMetadata uses `action` solely to select the command path;
        // it is not a flag accepted by any bookmark mutation parser.
        if key == "action" {
            continue;
        }
        let flag = format!("--{}", camel_to_kebab(key));
        match value {
            Value::Bool(true) => args.push(flag),
            Value::Bool(false) | Value::Null => {}
            Value::Array(values) => {
                for value in values {
                    append_one(args, &flag, value);
                }
            }
            value => append_one(args, &flag, value),
        }
    }
}

fn append_one(args: &mut Vec<String>, flag: &str, value: &Value) {
    args.push(flag.to_string());
    args.push(match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{OpPlan, PageKernel, ProjectIndex, kernel_for};
    use crate::{ResolvedProject, resolve_project, scaffold_schema_value};
    use serde_json::json;
    use std::path::Path;

    fn scaffold(root: &Path) -> ResolvedProject {
        let schema = serde_json::from_str(include_str!("../../examples/sales.schema.json"))
            .expect("sales schema");
        scaffold_schema_value(schema, Path::new("examples/sales.schema.json"), root, false)
            .expect("scaffold project");
        resolve_project(root).expect("resolve project")
    }

    #[test]
    fn parser_canonicalizes_repeated_flags_and_excludes_project_transport() {
        let args = [
            "--project",
            "/tmp/source",
            "--field",
            "DimCustomer.Segment",
            "--field",
            "DimCustomer.Region",
            "--dry-run",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let (payload, mode) = parse_args(&args).expect("payload");
        assert_eq!(mode, MutationMode::DryRun);
        assert!(!payload.fields.contains_key("project"));
        assert_eq!(
            payload.fields["field"],
            json!(["DimCustomer.Segment", "DimCustomer.Region"])
        );
        let operation = Op::AddCalculatedColumn(payload);
        let encoded = serde_json::to_value(operation).expect("operation json");
        assert_eq!(encoded["op"], "addCalculatedColumn");
        assert_eq!(
            encoded["field"],
            json!(["DimCustomer.Segment", "DimCustomer.Region"])
        );
    }

    #[test]
    fn every_remaining_operation_tag_round_trips_through_ops_v1() {
        let payload = MutationPayload {
            fields: BTreeMap::from([("handle".to_string(), json!("visual:Overview:Revenue"))]),
        };
        let operations = vec![
            Op::AddCalculatedColumn(payload.clone()),
            Op::AddStaticTable(payload.clone()),
            Op::SetSortBy(payload.clone()),
            Op::SourceTemplateApply(payload.clone()),
            Op::AddPage(payload.clone()),
            Op::UpdatePage(payload.clone()),
            Op::ReorderPages(payload.clone()),
            Op::SetActivePage(payload.clone()),
            Op::DeleteEmptyPage(payload.clone()),
            Op::ClonePage(payload),
            Op::SetBindings(MutationPayload {
                fields: BTreeMap::new(),
            }),
            Op::SetDisplayName(MutationPayload {
                fields: BTreeMap::new(),
            }),
            Op::SetTopNGuard(MutationPayload {
                fields: BTreeMap::new(),
            }),
            Op::SetDrilldownHierarchy(MutationPayload {
                fields: BTreeMap::new(),
            }),
        ];
        for operation in operations {
            let value = serde_json::to_value(&operation).expect("serialize operation");
            let decoded: Op = serde_json::from_value(value).expect("deserialize operation");
            assert_eq!(decoded, operation);
            assert!(
                kernel_for(&decoded).is_some(),
                "{} is not registered",
                decoded.tag()
            );
        }
    }

    #[test]
    fn generated_mutation_handles_are_stable_and_plan_visible() {
        let calculated_column = Op::AddCalculatedColumn(MutationPayload {
            fields: BTreeMap::from([
                ("table".to_string(), json!("Fact:Sales")),
                ("name".to_string(), json!("Gross%Margin")),
            ]),
        });
        assert_eq!(
            calculated_column.declared_handle_owned().as_deref(),
            Some("column:Fact%3ASales:Gross%25Margin")
        );
        let static_table = Op::AddStaticTable(MutationPayload {
            fields: BTreeMap::from([("table".to_string(), json!("DimMetric"))]),
        });
        assert_eq!(
            static_table.declared_handle_owned().as_deref(),
            Some("table:DimMetric")
        );
        let plan = OpPlan::new(vec![calculated_column, static_table]);
        let validated = plan
            .validate(&ProjectIndex::empty())
            .expect("handles validate");
        assert_eq!(validated.ops.len(), 2);
        assert_eq!(
            kernel_for(&Op::AddStaticTable(MutationPayload::default())).is_some(),
            true
        );
    }

    #[test]
    fn page_kernel_applies_existing_writer_to_transaction_copy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold(&temp.path().join("project"));
        let args = [
            "--display-name",
            "Operations",
            "--name",
            "ReportSectionOperations",
            "--dry-run",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let (payload, _) = parse_args(&args).expect("parse page add");
        let operation = Op::AddPage(payload);
        let index = ProjectIndex::from_project(&project).expect("project index");
        let plan = OpPlan::new(vec![operation]);
        let validated = plan.validate(&index).expect("valid page plan");
        let mut transaction = Transaction::begin(project).expect("transaction");
        let mut kernel = PageKernel;
        let receipt = transaction
            .apply_all(&validated, &mut kernel)
            .expect("apply");
        assert!(receipt.outcomes[0].changed);
        assert!(!receipt.outcomes[0].changes.is_empty());
        assert!(receipt.outcomes[0]
            .readback
            .iter()
            .all(|command| !command.contains(transaction.work_dir().to_string_lossy().as_ref())));
        assert!(!receipt.changes.is_empty());
    }
}
