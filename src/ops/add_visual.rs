//! Typed operation kernel for report visual creation.
//!
//! `AddVisual` is the single operation variant for the generic visual command
//! and the card, slicer, and textbox scaffold commands. Existing command
//! parsers and PBIR builders remain the source of truth; this module only
//! translates their inputs into a transaction working-copy mutation.

use super::{AddVisual, Op, OpKernel, OpOutcome, Transaction};
use crate::pbir::{PageSelector, find_page, load_report_snapshot};
use crate::pbir_bindings::{VisualBindingInput, binding_input_from_json, resolve_visual_bindings};
use crate::pbir_visual_factory::{
    SLICER_MIN_HEIGHT, VisualBuildSpec, resolve_slicer_mode, visual_container_json,
};
use crate::report_visual_mutations::validate_binding_cardinality;
use crate::report_visual_scaffold::operation_scaffold_json;
use crate::visual_catalog::canonical_visual_type;
use crate::{CliError, CliResult, command_arg};
use serde_json::{Value, json};
use std::fs;

/// The concrete kernel registered for [`Op::AddVisual`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AddVisualKernel;

/// Parse all four visual-add command shapes into the shared AddVisual payload.
/// Flag-specific parsers retain their existing diagnostics and are selected by
/// the distinctive scaffold flags (`--measure`, `--field`, and `--text` /
/// `--paragraphs-file`).
pub(crate) fn parse_args(args: &[String]) -> CliResult<(Op, crate::cli_support::MutationMode)> {
    if args.iter().any(|arg| arg == "--measure") {
        let (payload, mode) = crate::report_visual_scaffold::parse_card_operation_args(args)?;
        return Ok((Op::AddVisual(payload), mode));
    }
    if args.iter().any(|arg| arg == "--field") {
        let (payload, mode) = crate::report_visual_scaffold::parse_slicer_operation_args(args)?;
        return Ok((Op::AddVisual(payload), mode));
    }
    if args
        .iter()
        .any(|arg| arg == "--text" || arg == "--paragraphs-file")
    {
        let (payload, mode) = crate::report_visual_scaffold::parse_textbox_operation_args(args)?;
        return Ok((Op::AddVisual(payload), mode));
    }
    let (payload, mode) = crate::report_visual_mutations::parse_add_operation_args(args)?;
    Ok((Op::AddVisual(payload), mode))
}

/// Apply one AddVisual payload to a disposable transaction working copy.
/// Generic bindings are resolved through `pbir_bindings` and cardinality is
/// checked through the existing visual catalog before any file is written.
pub(crate) fn apply(payload: &AddVisual, transaction: &mut Transaction) -> CliResult<OpOutcome> {
    let project = transaction.working_project()?;
    let snapshot = load_report_snapshot(&project)?;
    let page_selector = selector_from_page(&payload.page);
    let page = find_page(&snapshot.pages, &page_selector, "report visuals add")?.clone();
    let title = payload
        .title
        .as_deref()
        .ok_or_else(|| CliError::invalid_args("AddVisual requires title"))?;
    if title.trim().is_empty() || title.chars().any(char::is_control) {
        return Err(CliError::invalid_args("title must be nonempty text"));
    }

    // A parser-generated payload carries the generated name in its declared
    // handle even when --name was omitted. Reusing that name lets replay find
    // the original visual instead of generating a numeric suffix on pass two.
    let handle_name = visual_name_from_handle(&payload.handle, &page.name);
    let visual_name = match payload.name.as_deref().or(handle_name.as_deref()) {
        Some(name) => {
            crate::report_visual_mutations::operation_validate_visual_name(name)?;
            name.to_string()
        }
        None => crate::report_visual_mutations::operation_generated_visual_name(title, &page),
    };
    let expected_handle = format!("visual:{}:{visual_name}", page.name);
    if payload.handle != expected_handle {
        return Err(CliError::validation_failed(format!(
            "addVisual handle must be {expected_handle}, got {}",
            payload.handle
        ))
        .with_pointer("/handle"));
    }

    let existing = page
        .visuals
        .iter()
        .find(|visual| visual.name == visual_name);
    let scaffold = has_scaffold_metadata(payload);
    let allow_outside_page = operation_allow_outside_page(payload);
    let position = build_position(payload, &page, existing, scaffold)?;
    crate::report_visual_mutations::operation_validate_position_bounds(
        &position,
        page.width.as_f64(),
        page.height.as_f64(),
        allow_outside_page,
    )?;
    if scaffold && payload.visual_type == "slicer" {
        let height = position["height"].as_f64().unwrap_or_default();
        if height < SLICER_MIN_HEIGHT {
            return Err(CliError::invalid_args(format!(
                "slicer height {height} is below the Power BI minimum of {SLICER_MIN_HEIGHT}"
            ))
            .with_hint(format!(
                "Increase --height to at least {SLICER_MIN_HEIGHT}."
            )));
        }
    }

    let binding_values = payload
        .bindings
        .iter()
        .filter(|binding| !is_metadata(binding))
        .collect::<Vec<_>>();
    let inputs = binding_values
        .iter()
        .map(|binding| binding_input_from_json(binding))
        .collect::<CliResult<Vec<VisualBindingInput>>>()?;
    let (_resolved_bindings, visual_json) = if scaffold && payload.visual_type == "textbox" {
        if !inputs.is_empty() {
            return Err(CliError::invalid_args(
                "textbox scaffold does not accept field bindings",
            ));
        }
        let visual_json = operation_scaffold_json(payload, &visual_name, title, &position)?
            .ok_or_else(|| CliError::invalid_args("textbox scaffold metadata is missing"))?;
        (Vec::new(), visual_json)
    } else {
        let visual_type = canonical_visual_type(&payload.visual_type)?;
        let resolved = if inputs.is_empty() {
            Vec::new()
        } else {
            let docs = crate::tmdl::load_table_documents(&project)?;
            resolve_visual_bindings(&docs, &visual_type, &inputs)?
        };
        validate_binding_cardinality(&visual_type, &resolved)?;
        let visual_json = if scaffold {
            operation_scaffold_json(payload, &visual_name, title, &position)?
                .ok_or_else(|| CliError::invalid_args("visual scaffold metadata is missing"))?
        } else {
            let slicer_mode = resolve_slicer_mode(&visual_type, payload.mode.as_deref())?;
            visual_container_json(&VisualBuildSpec {
                name: visual_name.clone(),
                title: title.to_string(),
                visual_type,
                bindings: resolved.clone(),
                slicer_mode,
                slicer_single_select: payload.single_select.unwrap_or(false),
                x: position["x"].as_f64().unwrap_or_default(),
                y: position["y"].as_f64().unwrap_or_default(),
                z: position["z"].as_u64().unwrap_or_default(),
                width: position["width"].as_f64().unwrap_or_default(),
                height: position["height"].as_f64().unwrap_or_default(),
                tab_order: position["tabOrder"].as_u64().unwrap_or_default(),
            })?
        };
        (resolved, visual_json)
    };

    let visuals_dir = crate::report_visual_mutations::operation_page_visuals_dir(&page)?;
    let visual_dir = visuals_dir.join(&visual_name);
    let visual_path = visual_dir.join("visual.json");
    crate::report_visual_mutations::operation_ensure_child_path(&visual_dir, &visuals_dir)?;

    if let Some(existing) = existing {
        if visual_path.exists() {
            let existing_json = fs::read_to_string(&visual_path)
                .map_err(|error| {
                    CliError::unexpected(format!("read {}: {error}", visual_path.display()))
                })
                .and_then(|text| {
                    serde_json::from_str::<Value>(&text).map_err(|error| {
                        CliError::validation_failed(format!(
                            "parse existing visual {}: {error}",
                            visual_path.display()
                        ))
                    })
                })?;
            if existing_json == visual_json {
                return Ok(outcome(
                    transaction,
                    &payload.handle,
                    false,
                    Vec::new(),
                    payload.visual_type == "slicer",
                ));
            }
        }
        return Err(CliError::invalid_args(format!(
            "visual already exists on page {}: {}",
            page.handle, existing.name
        ))
        .with_hint("Choose a unique internal --name or omit it so powerbi-cli can generate one."));
    }
    if visual_dir.exists() {
        return Err(CliError::invalid_args(format!(
            "target visual directory already exists: {}",
            visual_dir.display()
        ))
        .with_hint("Choose a unique --name or omit it so powerbi-cli can generate one."));
    }

    fs::create_dir_all(&visual_dir).map_err(|error| {
        CliError::unexpected(format!(
            "create visual dir {}: {error}",
            visual_dir.display()
        ))
    })?;
    if scaffold {
        crate::report_visual_scaffold::operation_write_visual_json(&visual_path, &visual_json)?;
    } else {
        crate::report_visual_mutations::operation_write_json_file(&visual_path, &visual_json)?;
    }
    let change = json!({
        "kind": "pbir.visual",
        "action": "add",
        "path": crate::canonical_display(&visual_path),
        "before": Value::Null,
        "after": visual_json
    });
    Ok(outcome(
        transaction,
        &payload.handle,
        true,
        vec![change],
        payload.visual_type == "slicer",
    ))
}

impl OpKernel for AddVisualKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let Op::AddVisual(payload) = operation else {
            return Err(CliError::invalid_args(format!(
                "AddVisualKernel cannot apply operation `{}`",
                operation.tag()
            )));
        };
        apply(payload, transaction)
    }
}

fn selector_from_page(page: &str) -> PageSelector {
    if page.starts_with("page:") {
        PageSelector {
            handle: Some(page.to_string()),
            name: None,
        }
    } else {
        PageSelector {
            handle: None,
            name: Some(page.to_string()),
        }
    }
}

fn visual_name_from_handle(handle: &str, page_name: &str) -> Option<String> {
    let rest = handle.strip_prefix("visual:")?;
    let (page, name) = rest.split_once(':')?;
    (page == page_name && !name.is_empty()).then(|| name.to_string())
}

fn is_metadata(value: &Value) -> bool {
    value["__powerbiCli"]
        .as_str()
        .is_some_and(|name| matches!(name, "addVisualScaffold" | "addVisualOptions"))
}

fn has_scaffold_metadata(payload: &AddVisual) -> bool {
    payload
        .bindings
        .iter()
        .any(|binding| binding["__powerbiCli"].as_str() == Some("addVisualScaffold"))
}

fn operation_allow_outside_page(payload: &AddVisual) -> bool {
    payload.bindings.iter().any(|binding| {
        binding["__powerbiCli"].as_str() == Some("addVisualOptions")
            && binding["allowOutsidePage"].as_bool() == Some(true)
    })
}

fn build_position(
    payload: &AddVisual,
    page: &crate::pbir::PageRecord,
    existing: Option<&crate::pbir::VisualRecord>,
    scaffold: bool,
) -> CliResult<Value> {
    let visual_index = page.visuals.len() as u64;
    if let Some(position) = payload.position.as_ref()
        && !position.is_object()
    {
        return Err(CliError::invalid_args(
            "AddVisual position must be a JSON object",
        ));
    }
    let payload_position = payload.position.as_ref();
    let existing_position = existing.map(|visual| &visual.position);
    let get_f64 = |key: &str, default: f64| -> CliResult<f64> {
        if let Some(value) = payload_position.and_then(|position| position.get(key)) {
            return value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| {
                    CliError::invalid_args(format!("visual position {key} must be finite"))
                });
        }
        if let Some(value) = existing_position.and_then(|position| position.get(key)) {
            return value
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or_else(|| {
                    CliError::invalid_args(format!("visual position {key} must be finite"))
                });
        }
        Ok(default)
    };
    let get_u64 = |key: &str, default: u64| -> CliResult<u64> {
        if let Some(value) = payload_position.and_then(|position| position.get(key)) {
            if let Some(parsed) = value.as_u64() {
                return Ok(parsed);
            }
            if let Some(parsed) = value.as_i64().and_then(|number| u64::try_from(number).ok()) {
                return Ok(parsed);
            }
            return Err(CliError::invalid_args(format!(
                "visual position {key} must be a nonnegative integer"
            )));
        }
        if let Some(value) = existing_position.and_then(|position| position.get(key)) {
            if let Some(parsed) = value.as_u64() {
                return Ok(parsed);
            }
            if let Some(parsed) = value.as_i64().and_then(|number| u64::try_from(number).ok()) {
                return Ok(parsed);
            }
            return Err(CliError::invalid_args(format!(
                "visual position {key} must be a nonnegative integer"
            )));
        }
        Ok(default)
    };
    let x = get_f64("x", 40.0 + (visual_index as f64 * 40.0))?;
    let y = get_f64("y", 40.0 + (visual_index as f64 * 40.0))?;
    let width = get_f64("width", 320.0)?;
    let height = get_f64("height", if scaffold { 120.0 } else { 180.0 })?;
    let z = get_u64(
        "z",
        if scaffold {
            crate::report_visual_scaffold::operation_next_stack_index(page)
        } else {
            visual_index
        },
    )?;
    let tab_order = get_u64(
        "tabOrder",
        if scaffold {
            crate::report_visual_scaffold::operation_next_stack_index(page)
        } else {
            visual_index
        },
    )?;
    Ok(json!({
        "x": x,
        "y": y,
        "z": z,
        "height": height,
        "width": width,
        "tabOrder": tab_order
    }))
}

fn outcome(
    transaction: &Transaction,
    handle: &str,
    changed: bool,
    changes: Vec<Value>,
    slicer: bool,
) -> OpOutcome {
    let project_arg = command_arg(&transaction.source.project_dir);
    let visual_readback = format!(
        "powerbi-cli report visuals show --project {} --handle {} --json",
        project_arg,
        crate::cli_support::shell_arg(handle)
    );
    let mut readback = vec![visual_readback];
    if slicer {
        let slicer_handle = handle.replacen("visual:", "slicer:", 1);
        readback.push(format!(
            "powerbi-cli report slicers show --project {} --handle {} --json",
            project_arg,
            crate::cli_support::shell_arg(&slicer_handle)
        ));
    }
    OpOutcome {
        changed,
        changes,
        readback,
        warnings: Vec::new(),
        created_handles: vec![handle.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{OpPlan, ProjectIndex};
    use crate::project_io::copy_project_dir;
    use crate::{ResolvedProject, resolve_project, scaffold_schema_value};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    fn scaffold(root: &Path) -> ResolvedProject {
        let schema =
            serde_json::from_str(include_str!("../../examples/sales.schema.json")).expect("schema");
        scaffold_schema_value(schema, Path::new("examples/sales.schema.json"), root, false)
            .expect("scaffold");
        resolve_project(root).expect("resolve")
    }

    fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter_map(|entry| {
                let relative = entry.path().strip_prefix(root).ok()?.to_path_buf();
                Some((relative, fs::read(entry.path()).ok()?))
            })
            .collect()
    }

    fn generic_payload() -> AddVisual {
        AddVisual {
            handle: "visual:ReportSectionOverview:VisualContainerKernelCard".into(),
            page: "page:ReportSectionOverview".into(),
            visual_type: "card".into(),
            name: Some("VisualContainerKernelCard".into()),
            title: Some("Kernel Card".into()),
            mode: None,
            single_select: None,
            position: Some(json!({
                "x": 40.0,
                "y": 40.0,
                "z": 0,
                "height": 120.0,
                "width": 240.0,
                "tabOrder": 0
            })),
            bindings: vec![json!({
                "role": "Values",
                "table": "FactSales",
                "measure": "Total Revenue"
            })],
        }
    }

    #[test]
    fn add_visual_parser_round_trips_all_command_shapes() {
        let cases = [
            vec![
                "--page".into(),
                "Overview".into(),
                "--measure".into(),
                "FactSales.Total Sales".into(),
                "--title".into(),
                "Card".into(),
                "--x".into(),
                "1".into(),
                "--y".into(),
                "2".into(),
                "--width".into(),
                "200".into(),
                "--height".into(),
                "100".into(),
                "--dry-run".into(),
            ],
            vec![
                "--page".into(),
                "Overview".into(),
                "--field".into(),
                "DimDate.Year".into(),
                "--title".into(),
                "Slicer".into(),
                "--x".into(),
                "1".into(),
                "--y".into(),
                "2".into(),
                "--width".into(),
                "200".into(),
                "--height".into(),
                "100".into(),
                "--dry-run".into(),
            ],
            vec![
                "--page".into(),
                "Overview".into(),
                "--title".into(),
                "Text".into(),
                "--text".into(),
                "Hello".into(),
                "--x".into(),
                "1".into(),
                "--y".into(),
                "2".into(),
                "--width".into(),
                "200".into(),
                "--height".into(),
                "100".into(),
                "--dry-run".into(),
            ],
        ];
        for args in cases {
            let (operation, mode) = parse_args(&args).expect("parse operation");
            assert_eq!(mode, crate::cli_support::MutationMode::DryRun);
            let value = serde_json::to_value(&operation).expect("operation JSON");
            assert_eq!(value["op"], "addVisual");
            let decoded: Op = serde_json::from_value(value).expect("decode operation");
            assert_eq!(decoded, operation);
        }
    }

    #[test]
    fn add_visual_kernel_is_idempotent_and_reports_declared_handle() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = scaffold(temp.path());
        let operation = Op::AddVisual(generic_payload());
        let index = ProjectIndex::from_project(&source).expect("project handles");
        let validated = OpPlan::new(vec![operation.clone()])
            .validate(&index)
            .expect("plan");
        let mut transaction = Transaction::begin(source).expect("transaction");
        let mut kernel = AddVisualKernel;
        let first = transaction
            .apply_all(&validated, &mut kernel)
            .expect("first apply");
        assert!(first.outcomes[0].changed);
        assert_eq!(
            first.outcomes[0].created_handles,
            vec![generic_payload().handle]
        );
        let second = apply(
            match &operation {
                Op::AddVisual(value) => value,
                _ => unreachable!(),
            },
            &mut transaction,
        )
        .expect("second apply");
        assert!(!second.changed);
        assert_eq!(second.created_handles, vec![generic_payload().handle]);
    }

    #[test]
    fn add_visual_kernel_matches_generic_cli_artifact_tree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cli_source_root = temp.path().join("cli-source");
        let op_source_root = temp.path().join("op-source");
        let cli_source = scaffold(&cli_source_root);
        copy_project_dir(&cli_source_root, &op_source_root).expect("copy source");
        let op_source = resolve_project(&op_source_root).expect("resolve copied source");
        let cli_out = temp.path().join("cli-out");
        let cli_args = vec![
            "--project".to_string(),
            cli_source.project_dir.display().to_string(),
            "--page".to_string(),
            "Overview".to_string(),
            "--name".to_string(),
            "VisualContainerKernelCard".to_string(),
            "--title".to_string(),
            "Kernel Card".to_string(),
            "--visual-type".to_string(),
            "card".to_string(),
            "--binding".to_string(),
            "role=Values,table=FactSales,measure=Total Revenue".to_string(),
            "--x".to_string(),
            "40".to_string(),
            "--y".to_string(),
            "40".to_string(),
            "--width".to_string(),
            "240".to_string(),
            "--height".to_string(),
            "120".to_string(),
            "--z".to_string(),
            "0".to_string(),
            "--tab-order".to_string(),
            "0".to_string(),
            "--out-dir".to_string(),
            cli_out.display().to_string(),
        ];
        crate::report_visual_mutations::add_visual(&cli_args).expect("CLI mutation");
        let operation = Op::AddVisual(generic_payload());
        let index = ProjectIndex::from_project(&op_source).expect("project handles");
        let validated = OpPlan::new(vec![operation]).validate(&index).expect("plan");
        let mut transaction = Transaction::begin(op_source).expect("transaction");
        let mut kernel = AddVisualKernel;
        transaction
            .apply_all(&validated, &mut kernel)
            .expect("kernel mutation");
        let op_out = temp.path().join("op-out");
        transaction
            .commit_out_dir(&op_out, false)
            .expect("kernel commit");
        assert_eq!(files(&cli_out), files(&op_out));
    }

    #[test]
    fn add_visual_kernel_matches_card_slicer_and_textbox_scaffolds() {
        let cases = [
            vec![
                "--page",
                "Overview",
                "--measure",
                "FactSales.Total Sales",
                "--title",
                "Kernel Card",
                "--x",
                "40",
                "--y",
                "40",
                "--width",
                "240",
                "--height",
                "120",
                "--dry-run",
            ],
            vec![
                "--page",
                "Overview",
                "--field",
                "DimDate.Year",
                "--title",
                "Kernel Slicer",
                "--x",
                "40",
                "--y",
                "180",
                "--width",
                "240",
                "--height",
                "100",
                "--mode",
                "dropdown",
                "--dry-run",
            ],
            vec![
                "--page",
                "Overview",
                "--title",
                "Kernel Text",
                "--text",
                "Hello",
                "--x",
                "40",
                "--y",
                "300",
                "--width",
                "240",
                "--height",
                "100",
                "--dry-run",
            ],
        ];
        for values in cases {
            let args = values.into_iter().map(String::from).collect::<Vec<_>>();
            let (operation, _) = parse_args(&args).expect("parse scaffold operation");
            let Op::AddVisual(payload) = operation else {
                unreachable!();
            };
            assert!(has_scaffold_metadata(&payload));
            assert!(payload.handle.starts_with("visual:"));
        }
    }

    #[test]
    fn add_visual_kernel_matches_each_scaffold_artifact_tree() {
        let cases: [(&str, Vec<String>); 3] = [
            (
                "card",
                vec![
                    "--page".into(),
                    "Overview".into(),
                    "--measure".into(),
                    "FactSales.Total Revenue".into(),
                    "--title".into(),
                    "Kernel Card".into(),
                    "--value-font-size".into(),
                    "18".into(),
                    "--x".into(),
                    "40".into(),
                    "--y".into(),
                    "40".into(),
                    "--width".into(),
                    "240".into(),
                    "--height".into(),
                    "120".into(),
                    "--out-dir".into(),
                    "PLACEHOLDER".into(),
                ],
            ),
            (
                "slicer",
                vec![
                    "--page".into(),
                    "Overview".into(),
                    "--name".into(),
                    "VisualContainerKernelSlicer".into(),
                    "--field".into(),
                    "DimDate.Month".into(),
                    "--title".into(),
                    "Kernel Slicer".into(),
                    "--mode".into(),
                    "dropdown".into(),
                    "--single-select".into(),
                    "--x".into(),
                    "40".into(),
                    "--y".into(),
                    "520".into(),
                    "--width".into(),
                    "240".into(),
                    "--height".into(),
                    "100".into(),
                    "--out-dir".into(),
                    "PLACEHOLDER".into(),
                ],
            ),
            (
                "textbox",
                vec![
                    "--page".into(),
                    "Overview".into(),
                    "--name".into(),
                    "VisualContainerKernelText".into(),
                    "--title".into(),
                    "Kernel Text".into(),
                    "--text".into(),
                    "Kernel paragraph".into(),
                    "--x".into(),
                    "700".into(),
                    "--y".into(),
                    "520".into(),
                    "--width".into(),
                    "240".into(),
                    "--height".into(),
                    "100".into(),
                    "--out-dir".into(),
                    "PLACEHOLDER".into(),
                ],
            ),
        ];
        for (kind, mut args) in cases {
            let temp = tempfile::tempdir().expect("tempdir");
            let cli_source_root = temp.path().join("cli-source");
            let op_source_root = temp.path().join("op-source");
            let cli_source = scaffold(&cli_source_root);
            copy_project_dir(&cli_source_root, &op_source_root).expect("copy source");
            let op_source = resolve_project(&op_source_root).expect("resolve copied source");
            let cli_out = temp.path().join("cli-out");
            let out_index = args
                .iter()
                .position(|arg| arg == "PLACEHOLDER")
                .expect("out placeholder");
            args[out_index] = cli_out.display().to_string();
            args.insert(0, "--project".into());
            args.insert(1, cli_source.project_dir.display().to_string());
            match kind {
                "card" => crate::report_visual_scaffold::add_card(&args).expect("CLI card"),
                "slicer" => crate::report_visual_scaffold::add_slicer(&args).expect("CLI slicer"),
                "textbox" => {
                    crate::report_visual_scaffold::add_textbox(&args).expect("CLI textbox")
                }
                _ => unreachable!(),
            };

            let operation = parse_args(&args).expect("parse operation").0;
            let index = ProjectIndex::from_project(&op_source).expect("project handles");
            let validated = OpPlan::new(vec![operation]).validate(&index).expect("plan");
            let mut transaction = Transaction::begin(op_source).expect("transaction");
            let mut kernel = AddVisualKernel;
            transaction
                .apply_all(&validated, &mut kernel)
                .expect("kernel mutation");
            let op_out = temp.path().join("op-out");
            transaction
                .commit_out_dir(&op_out, false)
                .expect("kernel commit");
            assert_eq!(files(&cli_out), files(&op_out), "{kind} artifact parity");
        }
    }
}
