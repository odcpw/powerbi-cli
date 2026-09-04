use crate::cli_support::{required_project, shell_arg, take_report_value as take_value};
use crate::pbir::{VisualSelector, find_visual, load_report_snapshot, visual_detail};
use crate::pbir_bindings::{VisualBindingInput, binding_summary, resolve_visual_bindings};
use crate::report_visual_mutations::validate_binding_cardinality;
use crate::tmdl::load_table_documents;
use crate::visual_catalog::{normalize_role, visual_type_role_rule};
use crate::{CliError, CliResult, EXIT_SUCCESS, canonical_display, command_arg, resolve_project};
use serde_json::{Value, json};
use std::path::PathBuf;

const DRY_RUN_COMMAND: &str = "powerbi-cli report visuals repair-bindings --project <project-dir-or.pbip> --handle <visual-handle> --dry-run --json";

#[derive(Debug, Default)]
struct RepairOptions {
    project: Option<PathBuf>,
    selector: VisualSelector,
    dry_run: bool,
}

#[derive(Debug)]
struct RepairCandidate {
    input: VisualBindingInput,
    before_kind: String,
}

pub(crate) fn repair_bindings(args: &[String]) -> CliResult<Value> {
    let options = parse_args(args)?;
    if !options.dry_run {
        return Err(CliError::invalid_args(
            "report visuals repair-bindings requires --dry-run",
        )
        .with_hint(
            "This command only proposes a typed set-bindings op. Review it, then run the returned preview or apply command.",
        )
        .with_suggested_command(DRY_RUN_COMMAND));
    }
    let project = required_project(options.project, "report visuals repair-bindings")?;
    require_selector(&options.selector)?;
    let resolved = resolve_project(&project)?;
    let snapshot = load_report_snapshot(&resolved)?;
    let visual = find_visual(
        &snapshot.pages,
        &options.selector,
        "report visuals repair-bindings",
    )?;
    let rule = visual_type_role_rule(&visual.visual_type)?;
    if visual.bindings.is_empty() {
        return Err(cannot_repair(
            &visual.handle,
            "the visual has no bindings, so required fields cannot be inferred",
        ));
    }

    let mut repairs = Vec::new();
    let mut candidates = visual
        .bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| {
            candidate_from_binding(
                &visual.visual_type,
                &visual.handle,
                index,
                binding,
                &mut repairs,
            )
        })
        .collect::<CliResult<Vec<_>>>()?;
    let category_is_bound = candidates
        .iter()
        .any(|candidate| candidate.input.role == "Category");
    for (index, candidate) in candidates.iter_mut().enumerate() {
        if visual.visual_type == "scatterChart"
            && category_is_bound
            && matches!(candidate.input.role.as_str(), "X" | "Y" | "Size")
            && candidate.before_kind == "column"
        {
            repairs.push(json!({
                "ruleId": "scatter.category-aggregated-value-axes",
                "bindingIndex": index,
                "role": candidate.input.role,
                "action": "wrap-sum-aggregation",
                "beforeKind": "column",
                "afterKind": "aggregatedColumn",
                "aggregationFunction": "Sum"
            }));
        } else if visual.visual_type == "hundredPercentStackedColumnChart"
            && candidate.input.role == "Y"
            && candidate.before_kind == "column"
        {
            repairs.push(json!({
                "ruleId": "binding.explicit-sum-column",
                "bindingIndex": index,
                "role": "Y",
                "action": "wrap-sum-aggregation",
                "beforeKind": "column",
                "afterKind": "aggregatedColumn",
                "aggregationFunction": "Sum"
            }));
        }
    }

    let docs = load_table_documents(&resolved)?;
    let inputs = candidates
        .iter()
        .map(|candidate| candidate.input.clone())
        .collect::<Vec<_>>();
    let resolved_bindings = resolve_visual_bindings(&docs, &visual.visual_type, &inputs)
        .and_then(|bindings| {
            validate_binding_cardinality(&visual.visual_type, &bindings)?;
            Ok(bindings)
        })
        .map_err(|error| cannot_repair(&visual.handle, &error.message))?;

    let project_arg = command_arg(&resolved.project_dir);
    let handle_arg = shell_arg(&visual.handle);
    let readback_command = format!(
        "powerbi-cli report visuals show --project {project_arg} --handle {handle_arg} --json"
    );
    let catalog_command = format!(
        "powerbi-cli report visuals catalog --visual-type {} --json",
        visual.visual_type
    );
    let validate_command = format!(
        "powerbi-cli validate --strict {} --json",
        command_arg(&resolved.project_dir)
    );

    if repairs.is_empty() {
        return Ok(json!({
            "schema": "powerbi-cli.report.visuals.bindingRepair.v1",
            "ok": true,
            "exitCode": EXIT_SUCCESS,
            "action": "repair-bindings",
            "dryRun": true,
            "changed": false,
            "projectDir": canonical_display(&resolved.project_dir),
            "pbip": canonical_display(&resolved.pbip_path),
            "reportDir": canonical_display(&resolved.report_dir),
            "target": visual_detail(visual),
            "rule": rule,
            "repairs": [],
            "repairPlan": {
                "strategy": "none",
                "before": visual.bindings,
                "after": resolved_bindings.iter().map(binding_summary).collect::<Vec<_>>(),
                "op": Value::Null
            },
            "previewCommand": Value::Null,
            "applyCommand": Value::Null,
            "readbackCommand": readback_command,
            "validateCommand": validate_command,
            "next": [readback_command, catalog_command, validate_command]
        }));
    }

    let op_bindings = candidates.iter().map(op_binding_json).collect::<Vec<_>>();
    let op = json!({
        "schema": "powerbi-cli.op.report.visuals.set-bindings.v1",
        "kind": "report.visuals.setBindings",
        "target": visual.handle,
        "bindings": op_bindings
    });
    let bindings_text = serde_json::to_string(&op["bindings"])
        .map_err(|error| CliError::unexpected(format!("serialize repair bindings: {error}")))?;
    let bindings_arg = shell_arg(&bindings_text);
    let preview_command = format!(
        "powerbi-cli report visuals set-bindings --project {project_arg} --handle {handle_arg} --bindings-json {bindings_arg} --dry-run --json"
    );
    let apply_command = format!(
        "powerbi-cli report visuals set-bindings --project {project_arg} --handle {handle_arg} --bindings-json {bindings_arg} --in-place --json"
    );

    Ok(json!({
        "schema": "powerbi-cli.report.visuals.bindingRepair.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "action": "repair-bindings",
        "dryRun": true,
        "changed": true,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "target": visual_detail(visual),
        "rule": rule,
        "repairs": repairs,
        "repairPlan": {
            "strategy": "replace-bindings-with-fixture-backed-role-map",
            "before": visual.bindings,
            "after": resolved_bindings.iter().map(binding_summary).collect::<Vec<_>>(),
            "op": op
        },
        "previewCommand": preview_command,
        "applyCommand": apply_command,
        "readbackCommand": readback_command,
        "validateCommand": validate_command,
        "next": [preview_command, apply_command, readback_command, catalog_command, validate_command]
    }))
}

fn candidate_from_binding(
    visual_type: &str,
    handle: &str,
    index: usize,
    binding: &Value,
    repairs: &mut Vec<Value>,
) -> CliResult<RepairCandidate> {
    let before_role = binding["role"]
        .as_str()
        .ok_or_else(|| cannot_repair(handle, &format!("binding {index} has no string role")))?;
    let canonical_role =
        if visual_type == "scatterChart" && before_role.eq_ignore_ascii_case("Details") {
            "Category".to_string()
        } else {
            normalize_role(visual_type, before_role)
                .map_err(|error| cannot_repair(handle, &error.message))?
        };
    if canonical_role != before_role {
        let rule_id =
            if visual_type == "scatterChart" && before_role.eq_ignore_ascii_case("Details") {
                "scatter.details-role-refused"
            } else if visual_type == "scatterChart" && canonical_role == "Series" {
                "scatter.series-pbir-role"
            } else {
                "binding.canonical-role-name"
            };
        repairs.push(json!({
            "ruleId": rule_id,
            "bindingIndex": index,
            "action": "rename-role",
            "beforeRole": before_role,
            "afterRole": canonical_role
        }));
    }

    let table = required_binding_string(binding, "table", handle, index)?;
    let before_kind = required_binding_string(binding, "kind", handle, index)?;
    let (column, measure) = match before_kind.as_str() {
        "column" | "aggregatedColumn" => (
            Some(required_binding_string(binding, "column", handle, index)?),
            None,
        ),
        "measure" => (
            None,
            Some(required_binding_string(binding, "measure", handle, index)?),
        ),
        other => {
            return Err(cannot_repair(
                handle,
                &format!("binding {index} has unsupported field kind {other}"),
            ));
        }
    };
    Ok(RepairCandidate {
        input: VisualBindingInput {
            role: canonical_role,
            table,
            column,
            measure,
            display_name: optional_binding_string(binding, "displayName"),
            format_string: optional_binding_string(binding, "format"),
            sort_direction: optional_binding_string(binding, "sortDirection"),
        },
        before_kind,
    })
}

fn required_binding_string(
    binding: &Value,
    field: &str,
    handle: &str,
    index: usize,
) -> CliResult<String> {
    binding[field]
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| cannot_repair(handle, &format!("binding {index} has no string {field}")))
}

fn optional_binding_string(binding: &Value, field: &str) -> Option<String> {
    binding[field].as_str().map(ToOwned::to_owned)
}

fn op_binding_json(candidate: &RepairCandidate) -> Value {
    let mut binding = json!({
        "role": candidate.input.role,
        "table": candidate.input.table
    });
    if let Some(column) = &candidate.input.column {
        binding["column"] = Value::String(column.clone());
    }
    if let Some(measure) = &candidate.input.measure {
        binding["measure"] = Value::String(measure.clone());
    }
    if let Some(display_name) = &candidate.input.display_name {
        binding["displayName"] = Value::String(display_name.clone());
    }
    if let Some(format) = &candidate.input.format_string {
        binding["formatString"] = Value::String(format.clone());
    }
    if let Some(sort_direction) = &candidate.input.sort_direction {
        binding["sortDirection"] = Value::String(sort_direction.clone());
    }
    binding
}

fn parse_args(args: &[String]) -> CliResult<RepairOptions> {
    let mut options = RepairOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--project" | "-p" => {
                options.project = Some(PathBuf::from(take_value(args, &mut index, "--project")?));
            }
            "--handle" => {
                options.selector.handle = Some(take_value(args, &mut index, "--handle")?);
            }
            "--page" => {
                options.selector.page = Some(take_value(args, &mut index, "--page")?);
            }
            "--visual" => {
                let value = take_value(args, &mut index, "--visual")?;
                if value.starts_with("visual:") {
                    options.selector.handle = Some(value);
                } else {
                    options.selector.visual = Some(value);
                }
            }
            "--dry-run" => {
                if options.dry_run {
                    return Err(CliError::invalid_args(
                        "report visuals repair-bindings accepts --dry-run once",
                    )
                    .with_hint("Remove the duplicate flag.")
                    .with_suggested_command(DRY_RUN_COMMAND));
                }
                options.dry_run = true;
                index += 1;
            }
            "--in-place" | "--out-dir" | "--out" => {
                return Err(CliError::invalid_args(
                    "report visuals repair-bindings is a dry-run-only planner and never writes",
                )
                .with_hint(
                    "Run with --dry-run, review repairPlan.op, then use the returned set-bindings command.",
                )
                .with_suggested_command(DRY_RUN_COMMAND));
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown report visuals repair-bindings flag: {other}"
                ))
                .with_hint("Run the exact dry-run command with a project and stable visual handle.")
                .with_suggested_command(DRY_RUN_COMMAND));
            }
        }
    }
    Ok(options)
}

fn require_selector(selector: &VisualSelector) -> CliResult<()> {
    if selector.handle.is_some() || (selector.page.is_some() && selector.visual.is_some()) {
        return Ok(());
    }
    Err(CliError::invalid_args(
        "report visuals repair-bindings requires --handle or --page plus --visual",
    )
    .with_hint("Use `report visuals list` to get a stable visual handle.")
    .with_suggested_command(DRY_RUN_COMMAND))
}

fn cannot_repair(handle: &str, reason: &str) -> CliError {
    CliError::unsupported_feature(format!(
        "cannot propose a deterministic binding repair for {handle}: {reason}"
    ))
    .with_hint(
        "The repair planner changes only proven role aliases and proven Sum aggregation wrappers; it never invents, drops, or substitutes model fields.",
    )
    .with_suggested_command("powerbi-cli report visuals catalog --visual-type <visual-type> --json")
    .with_suggested_command(format!(
        "powerbi-cli report visuals set-bindings --project <project-dir-or.pbip> --handle {} --bindings-json <reviewed-bindings-json> --dry-run --json",
        shell_arg(handle)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_requires_explicit_dry_run() {
        let error = repair_bindings(&[]).expect_err("missing dry run must fail");
        assert_eq!(error.code, "invalid_args");
        assert!(error.message.contains("requires --dry-run"));
        assert_eq!(error.suggested_commands, vec![DRY_RUN_COMMAND]);
    }
}
