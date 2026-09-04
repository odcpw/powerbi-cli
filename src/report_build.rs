use crate::cli_support::{
    MutationMode, require_mode_with_allowed_modes, set_mode_with_allowed_modes, shell_arg,
};
use crate::input_safety::{InputKind, read_utf8};
use crate::json_composition::normalize_spec_file;
use crate::ops::{Op, OpKernel, OpOutcome, OpPlan, ProjectIndex, Transaction, kernel_for};
use crate::pbir_filters::{FilterArrayOrigin, FilterScope, named_filter_handle};
use crate::pbir_visual_factory::{
    BETWEEN_SLICER_MIN_HEIGHT, SLICER_MIN_HEIGHT, SlicerMode, resolve_slicer_mode,
    slicer_between_data_type_is_supported,
};
use crate::profile::{load_profile_value, profile_summary, validate_profile_value};
use crate::report_filter_shapes::{
    FilterSpec, RelativeDateOperator, RelativeDateUnit, ResolvedFilterColumn,
    ResolvedFilterMeasure, TopNDirection, generated_filter_name, parse_field_reference,
};
use crate::report_proof::{ProofPlan, compile_proof_plan};
use crate::report_spec_explain::explain_command;
use crate::report_spec_fields::fields_command;
use crate::report_spec_normalize::normalize_command;
use crate::report_spec_schema::{reject_uncompiled_v2_sections, validate_known_fields};
use crate::report_spec_upgrade::upgrade_command;
use crate::schema::{load_schema_value, merge_schema_and_spec, validate_schema_value};
use crate::scorecard::{dry_run_scorecard, project_scorecard};
use crate::tmdl::{measure_handle, table_handle};
use crate::visual_catalog::{canonical_visual_type, normalize_role};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    resolve_project, scaffold_schema_value, validate_project,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct BuildOptions {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
    spec: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    force: bool,
    trace: bool,
    mode: Option<MutationMode>,
}

#[derive(Debug, Default)]
struct SpecValidateOptions {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
    spec: Option<PathBuf>,
}

struct BuildResponse<'a> {
    dry_run: bool,
    changed: bool,
    schema_path: &'a Path,
    profile_path: Option<&'a Path>,
    spec_path: Option<&'a Path>,
    out_dir: Option<&'a Path>,
    compiled: &'a CompiledDashboard,
    profile: Option<&'a Value>,
    scaffold: Option<Value>,
    proof_plan: Option<&'a ProofPlan>,
    trace: bool,
}

pub(crate) fn build_command(args: &[String]) -> CliResult<Value> {
    let options = parse_build_args(args)?;
    let mode = require_mode_with_allowed_modes(
        options.mode,
        "report build",
        "--dry-run or --out-dir <dir>",
        "Choose exactly one build mode: preview with --dry-run or write a new project with --out-dir <dir>.",
        "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --dry-run --json",
    )?;
    let schema_path = options.schema.ok_or_else(|| {
        spec_missing_input_with_command(
            "/schema",
            "schema",
            "report build needs a schema manifest to resolve model fields and emit a PBIP project",
            json!({"--schema": "<schema.json>"}),
            "powerbi-cli schema validate <schema.json> --json",
        )
    })?;
    let schema_value = load_schema_value(&schema_path)?;
    let spec_value = load_optional_value(options.spec.as_deref(), "dashboard spec")?;
    if let Some(spec) = spec_value.as_ref() {
        validate_known_fields(spec)?;
    }
    let profile_value = load_optional_profile(options.profile.as_deref())?;
    let mut compiled = compile_dashboard(&schema_value, spec_value.as_ref())?;
    let dry_run_proof_plan = if mode == MutationMode::DryRun {
        compile_proof_plan(spec_value.as_ref(), None)?
    } else {
        None
    };
    let schema_validation = validate_schema_value(&compiled.schema);
    if !schema_validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "compiled dashboard schema is invalid: {}",
            schema_validation.errors.join("; ")
        ))
        .with_suggested_command(format!(
            "powerbi-cli report spec validate --schema {} --spec {} --json",
            command_arg(&schema_path),
            options
                .spec
                .as_deref()
                .map(command_arg)
                .unwrap_or_else(|| "<dashboard.json>".to_string())
        )));
    }

    if mode == MutationMode::DryRun {
        return Ok(build_response(BuildResponse {
            dry_run: true,
            changed: false,
            schema_path: &schema_path,
            profile_path: options.profile.as_deref(),
            spec_path: options.spec.as_deref(),
            out_dir: None,
            compiled: &compiled,
            profile: profile_value.as_ref(),
            scaffold: None,
            proof_plan: dry_run_proof_plan.as_ref(),
            trace: options.trace,
        }));
    }

    let out_dir = options.out_dir.ok_or_else(|| {
        CliError::invalid_args("report build --out-dir mode requires --out-dir <project-dir>")
            .with_suggested_command(
                "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json",
            )
    })?;
    let proof_plan = compile_proof_plan(spec_value.as_ref(), Some(&out_dir))?;
    let scaffold = if compiled.typed_operations.is_empty() {
        scaffold_schema_value(
            compiled.schema.clone(),
            &schema_path,
            &out_dir,
            options.force,
        )?
    } else {
        build_with_operations(&mut compiled, &schema_path, &out_dir, options.force)?
    };
    Ok(build_response(BuildResponse {
        dry_run: false,
        changed: true,
        schema_path: &schema_path,
        profile_path: options.profile.as_deref(),
        spec_path: options.spec.as_deref(),
        out_dir: Some(&out_dir),
        compiled: &compiled,
        profile: profile_value.as_ref(),
        scaffold: Some(scaffold),
        proof_plan: proof_plan.as_ref(),
        trace: options.trace,
    }))
}

pub(crate) fn spec_command(args: &[String]) -> CliResult<Value> {
    match args {
        [action, rest @ ..] if action == "validate" => spec_validate(rest),
        [action, rest @ ..] if action == "normalize" => normalize_command(rest),
        [action, rest @ ..] if action == "fields" => fields_command(rest),
        [action, rest @ ..] if action == "schema" => crate::report_spec_schema::schema_command(rest),
        [action, rest @ ..] if action == "explain" => explain_command(rest),
        [action, rest @ ..] if action == "upgrade" => upgrade_command(rest),
        [] => Err(CliError::invalid_args(
            "report spec requires a subcommand: validate, normalize, fields, schema, explain, or upgrade",
        )
            .with_suggested_command(
                "powerbi-cli report spec validate --schema <schema.json> --spec <dashboard.json> --json",
            )
            .with_suggested_command(
                "powerbi-cli report spec fields --schema <schema.json> --json",
            )
            .with_suggested_command(
                "powerbi-cli report spec upgrade --spec <v1.json> --out <v2.json> --json",
            )),
        _ => Err(CliError::invalid_args("unknown report spec command")
            .with_suggested_command("powerbi-cli --json capabilities --for \"report spec\"")),
    }
}

pub(crate) fn compile_dashboard_summary(schema: &Value, spec: &Value) -> CliResult<Value> {
    let compiled = compile_dashboard(schema, Some(spec))?;
    Ok(compiled_summary(&compiled))
}

/// Compile a sanitized dashboard spec for the read-only `report spec explain`
/// surface. The explain command removes recognized-but-uncompiled sections
/// before calling this helper, while the normal build path retains its strict
/// refusal behavior.
pub(crate) fn compiled_schema_for_explain(schema: &Value, spec: &Value) -> CliResult<Value> {
    Ok(compile_dashboard(schema, Some(spec))?.schema)
}

fn spec_validate(args: &[String]) -> CliResult<Value> {
    let options = parse_spec_validate_args(args)?;
    let spec_path = options.spec.ok_or_else(|| {
        spec_missing_input_with_command(
            "/spec",
            "spec",
            "report spec validate needs a dashboard spec document",
            json!({"--spec": "<dashboard.json>"}),
            "powerbi-cli report spec fields --json",
        )
    })?;
    let normalized_spec = normalize_spec_file(&spec_path)?;
    let spec_value = normalized_spec.value;
    let known_fields = validate_known_fields(&spec_value);
    let proof_plan_result = if known_fields.is_ok() {
        compile_proof_plan(Some(&spec_value), None)
    } else {
        Ok(None)
    };
    let profile_value = load_optional_profile(options.profile.as_deref())?;
    let (ok, validation_level, errors, warnings, compiled, schema_path) = if let Some(schema_path) =
        options.schema.as_deref()
    {
        let schema_value = load_schema_value(schema_path)?;
        match known_fields.and_then(|_| compile_dashboard(&schema_value, Some(&spec_value))) {
            Ok(compiled) => {
                let schema_validation = validate_schema_value(&compiled.schema);
                (
                    schema_validation.errors.is_empty(),
                    "compiled",
                    schema_validation
                        .errors
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                    Vec::new(),
                    Some(compiled),
                    Some(schema_path.to_path_buf()),
                )
            }
            Err(err) => (
                false,
                "compiled",
                vec![spec_error_json(&err)],
                Vec::new(),
                None,
                Some(schema_path.to_path_buf()),
            ),
        }
    } else {
        let errors = match (known_fields, &proof_plan_result) {
            (Ok(_), Ok(_)) => validate_spec_shape(&spec_value)
                .into_iter()
                .map(Value::String)
                .collect(),
            (Ok(_), Err(error)) | (Err(_), Err(error)) => vec![spec_error_json(error)],
            (Err(error), Ok(_)) => vec![spec_error_json(&error)],
        };
        let warnings = if errors.is_empty() {
            vec![
            "schema was not provided; shape-only validation cannot prove field references, measures, visual roles, or build compatibility".to_string()
            ]
        } else {
            Vec::new()
        };
        (
            errors.is_empty(),
            "shape-only",
            errors,
            warnings,
            None,
            None,
        )
    };
    Ok(json!({
        "schema": "powerbi-cli.report.spec.validate.v1",
        "ok": if validation_level == "shape-only" && errors.is_empty() { Value::Null } else { Value::Bool(ok) },
        "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "validationLevel": validation_level,
        "specPath": canonical_display(&spec_path),
        "schemaPath": schema_path.as_ref().map(|path| canonical_display(path)),
        "profilePath": options.profile.as_ref().map(|path| canonical_display(path)),
        "normalizedFrom": normalized_spec.normalized_from,
        "profileSummary": profile_value.as_ref().map(profile_summary),
        "compiled": compiled.as_ref().map(compiled_summary),
        "defaultsApplied": compiled
            .as_ref()
            .map(|compiled| Value::Array(compiled.defaults_applied.clone()))
            .unwrap_or_else(|| {
                if validation_level == "shape-only" && errors.is_empty() {
                    json!([{
                        "pointer": "/schema",
                        "field": "schema",
                        "value": "shape-only",
                        "reason": "schema input was omitted; shape-only validation is the documented fallback"
                    }])
                } else {
                    Value::Array(Vec::new())
                }
            }),
        "proofPlan": proof_plan_result
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map(|plan| plan.value.clone()),
        "warnings": warnings,
        "errors": errors,
        "next": next_for_spec_validate(
            &spec_path,
            schema_path.as_deref(),
            ok,
            validation_level,
            proof_plan_result
                .as_ref()
                .ok()
                .and_then(Option::as_ref),
        )
    }))
}

fn spec_error_json(error: &CliError) -> Value {
    let mut value = json!({
        "code": error.code,
        "message": error.message,
    });
    if let Some(pointer) = error.pointer() {
        value["pointer"] = Value::String(pointer.to_string());
    }
    if let Some(did_you_mean) = error.did_you_mean() {
        value["didYouMean"] = Value::String(did_you_mean.to_string());
    }
    if let Some(field) = error.field() {
        value["field"] = Value::String(field.to_string());
    }
    if let Some(reason) = error.reason() {
        value["reason"] = Value::String(reason.to_string());
    }
    if let Some(candidates_command) = error.candidates_command() {
        value["candidatesCommand"] = Value::String(candidates_command.to_string());
    }
    if let Some(example) = error.example() {
        value["example"] = example.clone();
    }
    if let Some(hint) = &error.hint {
        value["hint"] = Value::String(hint.clone());
    }
    if !error.suggested_commands.is_empty() {
        value["suggestedCommands"] = Value::Array(
            error
                .suggested_commands
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        );
    }
    value
}

const SPEC_FIELDS_COMMAND: &str = "powerbi-cli report spec fields --schema <schema.json> --json";

/// Build the one structured diagnostic used when a dashboard spec cannot be
/// compiled without inventing user intent. Keeping this constructor in the
/// compiler lets the planner and report-spec validator expose the same
/// machine-readable contract on stderr and stdout respectively.
pub(crate) fn spec_missing_input(
    pointer: impl Into<String>,
    field: impl Into<String>,
    reason: impl Into<String>,
    example: Value,
) -> CliError {
    spec_missing_input_with_command(pointer, field, reason, example, SPEC_FIELDS_COMMAND)
}

pub(crate) fn spec_missing_input_with_command(
    pointer: impl Into<String>,
    field: impl Into<String>,
    reason: impl Into<String>,
    example: Value,
    candidates_command: impl Into<String>,
) -> CliError {
    let pointer = pointer.into();
    let field = field.into();
    let reason = reason.into();
    let candidates_command = candidates_command.into();
    CliError::new(
        "spec.missing_input",
        EXIT_VALIDATION_FAILED,
        format!("required dashboard-spec input `{field}` is missing: {reason}"),
    )
    .with_pointer(pointer)
    .with_field(field)
    .with_reason(reason)
    .with_candidates_command(candidates_command.clone())
    .with_example(example)
    .with_hint("Supply the required value; no dashboard-spec default is applied when it would change user intent.")
    .with_suggested_command(candidates_command)
}

fn validate_required_spec_inputs(schema: &Value, spec: &Value) -> CliResult<()> {
    let Some(root) = spec.as_object() else {
        return Ok(());
    };
    let is_v2 = root.get("schema").and_then(Value::as_str) == Some("powerbi-cli.dashboard.v2");
    let model = ModelIndex::from_schema(schema);

    if let Some(model_object) = root.get("model").and_then(Value::as_object)
        && let Some(patterns) = model_object
            .get("measurePatterns")
            .and_then(Value::as_array)
    {
        for (index, pattern) in patterns.iter().enumerate() {
            let Some(pattern_object) = pattern.as_object() else {
                continue;
            };
            let needs_date = pattern_object
                .get("pattern")
                .and_then(Value::as_str)
                .is_some_and(measure_pattern_needs_date);
            if needs_date && !has_nonempty_string(pattern_object.get("date")) {
                return Err(spec_missing_input(
                    format!("/model/measurePatterns/{index}/date"),
                    "model.measurePatterns[].date",
                    "this measure pattern references a period-aware calculation and needs a date column",
                    json!({"date": "DimDate[Date]"}),
                ));
            }
        }
    }

    let semantic_tokens = root
        .get("style")
        .and_then(Value::as_object)
        .and_then(|style| style.get("tokens"))
        .and_then(Value::as_object)
        .and_then(|tokens| tokens.get("semantic"))
        .and_then(Value::as_object);
    let mut referenced_tokens = BTreeSet::new();
    for page in root
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(page) = page.as_object() else {
            continue;
        };
        for visual in page
            .get("visuals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(conditional) = visual.get("conditionalFormatting") {
                collect_semantic_tokens(conditional, false, &mut referenced_tokens);
            }
        }
    }
    for token in referenced_tokens {
        if !semantic_tokens.is_some_and(|tokens| {
            tokens.get(&token).is_some_and(|value| {
                !value.is_null() && value.as_str().is_none_or(|value| !value.trim().is_empty())
            })
        }) {
            return Err(spec_missing_input(
                format!("/style/tokens/semantic/{}", escape_pointer_token(&token)),
                "style.tokens.semantic",
                format!(
                    "conditional formatting uses semantic color `{token}`, but no matching style token is defined"
                ),
                json!({"style": {"tokens": {"semantic": {token.clone(): "#2E7D32"}}}}),
            ));
        }
    }

    if let Some(layout) = root.get("layout").and_then(Value::as_object)
        && let Some(rail) = layout.get("rail").and_then(Value::as_object)
    {
        validate_slicer_fields(rail.get("slicers"), "/layout/rail/slicers", &model)?;
    }

    for (page_index, page) in root
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(page) = page.as_object() else {
            continue;
        };
        if let Some(drillthrough) = page.get("drillthrough").and_then(Value::as_object)
            && !has_nonempty_string(drillthrough.get("target"))
        {
            return Err(spec_missing_input(
                format!("/pages/{page_index}/drillthrough/target"),
                "pages[].drillthrough.target",
                "a drillthrough page must name the column that receives the bound filter",
                json!({"target": "DimCustomer[CustomerName]"}),
            ));
        }
        validate_slicer_fields(
            page.get("slicers"),
            &format!("/pages/{page_index}/slicers"),
            &model,
        )?;

        let template = page.get("template").and_then(Value::as_str);
        for (visual_index, visual) in page
            .get("visuals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(visual) = visual.as_object() else {
                continue;
            };
            let visual_pointer = format!("/pages/{page_index}/visuals/{visual_index}");
            if let (Some(template), Some(slot)) = (
                template.filter(|template| !template.trim().is_empty()),
                visual.get("slot").and_then(Value::as_str),
            ) && !template_slot_allowed(template, slot)
            {
                return Err(spec_missing_input(
                    format!("{visual_pointer}/slot"),
                    "visuals[].slot",
                    format!("slot `{slot}` is not defined by page template `{template}`"),
                    json!({"slot": "primary"}),
                ));
            }

            if let Some(topn) = visual.get("topnGuard").and_then(Value::as_object) {
                let measure_count = visual_measure_binding_count(visual, &model);
                if measure_count > 1 && !has_nonempty_string(topn.get("orderBy")) {
                    return Err(spec_missing_input(
                        format!("{visual_pointer}/topnGuard/orderBy"),
                        "visuals[].topnGuard.orderBy",
                        "a TopN guard with multiple measures needs an explicit measure ordering",
                        json!({"orderBy": "FactSales[Total Revenue]"}),
                    ));
                }
            }

            let has_uncompiled_section = [
                "sort",
                "drilldown",
                "topnGuard",
                "filters",
                "format",
                "conditionalFormatting",
                "slot",
                "subtitle",
            ]
            .iter()
            .any(|field| visual.contains_key(*field));
            if has_uncompiled_section {
                continue;
            }

            let requested_type = visual
                .get("type")
                .or_else(|| visual.get("visualType"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty());
            let Some(requested_type) = requested_type else {
                return Err(spec_missing_input(
                    format!("{visual_pointer}/type"),
                    "visuals[].type",
                    "the compiler cannot choose a visual family without changing the requested dashboard intent",
                    json!({"type": "card"}),
                ));
            };
            let bindings = visual
                .get("bindings")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if bindings == 0 {
                let has_text = visual_text(visual).is_some();
                if !has_text || !requested_type.eq_ignore_ascii_case("textbox") {
                    return Err(spec_missing_input(
                        format!("{visual_pointer}/bindings"),
                        "visuals[].bindings",
                        "the visual has no field bindings; provide the required role bindings instead of defaulting to an empty card",
                        json!({"bindings": [{"role": "Values", "field": "FactSales[Total Revenue]"}]}),
                    ));
                }
            }

            if is_v2 && requested_type.eq_ignore_ascii_case("slicer") {
                validate_visual_slicer_binding(
                    visual.get("bindings"),
                    &format!("{visual_pointer}/bindings"),
                    &model,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_slicer_fields(
    slicers: Option<&Value>,
    pointer: &str,
    model: &ModelIndex,
) -> CliResult<()> {
    let Some(slicers) = slicers.and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, slicer) in slicers.iter().enumerate() {
        let Some(slicer) = slicer.as_object() else {
            continue;
        };
        let field_pointer = format!("{pointer}/{index}/field");
        let Some(field) = slicer.get("field").and_then(Value::as_str) else {
            return Err(spec_missing_input(
                field_pointer,
                "slicers[].field",
                "a slicer needs a model column to populate its filter values",
                json!({"field": "DimCustomer[Segment]"}),
            ));
        };
        if !matches!(
            model.resolve_field(field),
            Ok(FieldRef {
                kind: FieldKind::Column,
                ..
            })
        ) {
            return Err(spec_missing_input(
                field_pointer,
                "slicers[].field",
                format!("`{field}` is not a resolvable model column"),
                json!({"field": "DimCustomer[Segment]"}),
            ));
        }
    }
    Ok(())
}

fn validate_visual_slicer_binding(
    bindings: Option<&Value>,
    pointer: &str,
    model: &ModelIndex,
) -> CliResult<()> {
    let Some(bindings) = bindings.and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, binding) in bindings.iter().enumerate() {
        let Some(binding) = binding.as_object() else {
            continue;
        };
        let role = binding.get("role").and_then(Value::as_str);
        if role.is_some_and(|role| role.eq_ignore_ascii_case("values")) {
            let field_value = binding
                .get("field")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    let table = binding.get("table").and_then(Value::as_str)?;
                    let column = binding.get("column").and_then(Value::as_str)?;
                    Some(format!("{table}[{column}]"))
                });
            let field_pointer = format!("{pointer}/{index}/field");
            let Some(field) = field_value else {
                return Err(spec_missing_input(
                    field_pointer,
                    "visuals[].bindings[].field",
                    "a slicer Values role must identify a model column, not a missing measure or inferred aggregation",
                    json!({"field": "DimCustomer[Segment]"}),
                ));
            };
            if !matches!(
                model.resolve_field(&field),
                Ok(FieldRef {
                    kind: FieldKind::Column,
                    ..
                })
            ) {
                return Err(spec_missing_input(
                    field_pointer,
                    "visuals[].bindings[].field",
                    format!("`{field}` is not a model column; slicers cannot bind measures"),
                    json!({"field": "DimCustomer[Segment]"}),
                ));
            }
        }
    }
    Ok(())
}

fn visual_measure_binding_count(visual: &Map<String, Value>, model: &ModelIndex) -> usize {
    visual
        .get("bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|binding| {
            binding.contains_key("measure")
                || binding
                    .get("field")
                    .and_then(Value::as_str)
                    .and_then(|field| model.resolve_field(field).ok())
                    .is_some_and(|field| field.kind == FieldKind::Measure)
        })
        .count()
}

fn visual_text(visual: &Map<String, Value>) -> Option<&str> {
    visual
        .get("text")
        .or_else(|| visual.get("title"))
        .or_else(|| visual.get("subtitle"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(str::trim)
}

fn measure_pattern_needs_date(pattern: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    [
        "date", "time", "period", "yoy", "ytd", "mom", "qoq", "prior", "trend",
    ]
    .iter()
    .any(|needle| pattern.contains(needle))
}

fn collect_semantic_tokens(value: &Value, semantic_context: bool, tokens: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let key_context = semantic_context
                    || key.to_ascii_lowercase().contains("semantic")
                    || key.to_ascii_lowercase().contains("token");
                collect_semantic_tokens(value, key_context, tokens);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_semantic_tokens(value, semantic_context, tokens);
            }
        }
        Value::String(value) => {
            let value = value.trim();
            let explicit_semantic =
                value.starts_with("semantic.") || value.starts_with("semantic:");
            if !semantic_context && !explicit_semantic {
                return;
            }
            let token = value
                .strip_prefix("semantic.")
                .or_else(|| value.strip_prefix("semantic:"))
                .unwrap_or(value)
                .trim();
            if !token.is_empty()
                && !token.starts_with('#')
                && !token.starts_with("rgb")
                && !token.starts_with("rgba")
                && token.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
                })
            {
                tokens.insert(token.to_string());
            }
        }
        _ => {}
    }
}

fn template_slot_allowed(template: &str, slot: &str) -> bool {
    let templates = [
        "kpi-strip-trend-breakdown",
        "overview",
        "time-series",
        "ranking",
        "distribution",
        "comparison",
        "detail-table",
        "drillthrough-detail",
        "exception-list",
        "matrix-focus",
        "scatter-focus",
    ];
    if !templates
        .iter()
        .any(|name| name.eq_ignore_ascii_case(template.trim()))
    {
        // Unknown template names are still owned by the layout compiler. Let
        // the normal unsupported-feature boundary report that section rather
        // than guessing a slot catalogue for an unrecognized template.
        return true;
    }
    [
        "heading",
        "kpi.1",
        "kpi.2",
        "kpi.3",
        "kpi.4",
        "primary",
        "secondary",
        "tertiary",
        "detail",
        "rail",
    ]
    .iter()
    .any(|name| name.eq_ignore_ascii_case(slot.trim()))
}

fn has_nonempty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn escape_pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[derive(Debug)]
struct CompiledDashboard {
    schema: Value,
    operations: Vec<Value>,
    typed_operations: Vec<Op>,
    operation_pointers: Vec<String>,
    operation_outcomes: Vec<OpOutcome>,
    warnings: Vec<Value>,
    defaults_applied: Vec<Value>,
}

fn compile_dashboard(schema: &Value, spec: Option<&Value>) -> CliResult<CompiledDashboard> {
    let Some(spec) = spec else {
        let (schema, notes) = merge_schema_and_spec(schema.clone(), None)?;
        return Ok(CompiledDashboard {
            schema,
            operations: vec![
                json!({"kind": "legacySchema", "summary": "used pages embedded in schema manifest"}),
            ],
            typed_operations: Vec::new(),
            operation_pointers: Vec::new(),
            operation_outcomes: Vec::new(),
            warnings: notes
                .into_iter()
                .map(|message| json!({"code": "report_build.legacy_schema", "message": message}))
                .collect(),
            defaults_applied: vec![json!({
                "pointer": "/spec",
                "field": "spec",
                "value": "schema.pages",
                "reason": "the optional dashboard spec was omitted; pages embedded in the schema are used"
            })],
        });
    };
    validate_known_fields(spec)?;
    validate_required_spec_inputs(schema, spec)?;
    // `proof` is metadata compiled by the side-effect-free proof planner below.
    // Keep the existing v2 refusal boundary for every other recognized section
    // without changing the shared walker while adjacent spec work lands.
    let sections_to_check = if spec.get("proof").is_some() {
        let mut stripped = spec.clone();
        stripped
            .as_object_mut()
            .expect("validated dashboard spec object")
            .remove("proof");
        stripped
    } else {
        spec.clone()
    };
    reject_uncompiled_v2_sections(&sections_to_check)?;
    let _proof_plan = compile_proof_plan(Some(spec), None)?;
    if spec.get("report").is_none() && spec.get("pages").is_some() {
        let (schema, notes) = merge_schema_and_spec(schema.clone(), Some(spec))?;
        return Ok(CompiledDashboard {
            schema,
            operations: vec![
                json!({"kind": "legacySpecMerge", "summary": "merged top-level dashboard fields into schema manifest"}),
            ],
            typed_operations: Vec::new(),
            operation_pointers: Vec::new(),
            operation_outcomes: Vec::new(),
            warnings: notes
                .into_iter()
                .map(|message| json!({"code": "report_build.legacy_spec", "message": message}))
                .collect(),
            defaults_applied: Vec::new(),
        });
    }

    let mut merged = schema.clone();
    let spec_object = spec
        .as_object()
        .ok_or_else(|| CliError::invalid_args("dashboard spec root must be an object"))?;
    let report = spec_object
        .get("report")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::invalid_args("dashboard spec requires report object"))?;
    {
        let merged_object = merged
            .as_object_mut()
            .ok_or_else(|| CliError::invalid_args("schema root must be an object"))?;
        copy_report_field(report, merged_object, "name");
        copy_report_field(report, merged_object, "displayName");
        copy_report_field(report, merged_object, "description");
        copy_report_field(report, merged_object, "locale");
        apply_model_extensions(merged_object, spec_object)?;
    }
    let model = ModelIndex::from_schema(&merged);
    let mut defaults_applied = Vec::new();
    let pages = compile_pages(spec_object, &model, &mut defaults_applied)?;
    if !pages.is_empty() {
        merged
            .as_object_mut()
            .ok_or_else(|| CliError::invalid_args("schema root must be an object"))?
            .insert("pages".to_string(), Value::Array(pages));
    }
    let mut operations = vec![json!({
        "kind": "compileDashboardSpec",
        "summary": "compiled powerbi-cli.dashboard.v1 report/pages/visuals into scaffold-compatible manifest"
    })];
    let (typed_operations, operation_pointers) = compile_filter_operations(spec_object, &model)?;
    operations.extend(
        typed_operations
            .iter()
            .map(|operation| serde_json::to_value(operation).expect("typed operation serializes")),
    );
    if spec_object.get("style").is_some() {
        return Err(CliError::unsupported_feature(
            "report build style application from dashboard spec is not implemented yet"
        )
        .with_suggested_command(
            "powerbi-cli report themes apply-preset --project <project-dir> --preset <preset> --dry-run --json",
        ));
    }
    if spec_object.get("proof").is_some() {
        operations.push(json!({
            "kind": "proofRequirements",
            "summary": "proof block recorded by report build output; proof commands are returned but not executed automatically"
        }));
    }
    Ok(CompiledDashboard {
        schema: merged,
        operations,
        typed_operations,
        operation_pointers,
        operation_outcomes: Vec::new(),
        warnings: Vec::new(),
        defaults_applied,
    })
}

/// Apply compiler-generated operations to a scaffolded working copy before it
/// is published. Keeping this path on the same transaction/kernels used by
/// operation replay makes spec builds and command mutations artifact-identical
/// while still discarding a failed plan without touching the requested output.
fn build_with_operations(
    compiled: &mut CompiledDashboard,
    schema_path: &Path,
    out_dir: &Path,
    force: bool,
) -> CliResult<Value> {
    let staging = tempfile::tempdir().map_err(|error| {
        CliError::unexpected(format!("create report build staging directory: {error}"))
    })?;
    let mut scaffold =
        scaffold_schema_value(compiled.schema.clone(), schema_path, staging.path(), false)?;
    let staged_project = resolve_project(staging.path())?;
    let plan = OpPlan::new(compiled.typed_operations.clone());
    let project_index = ProjectIndex::from_project(&staged_project)?;
    let validated = plan
        .validate(&project_index)
        .map_err(|error| map_plan_error(error, &compiled.operation_pointers))?;
    let mut transaction = Transaction::begin(staged_project)?;
    // Kernel summaries carry paths rooted at the transaction's source
    // project (the scaffold's nested `*.Report` directory), while the
    // scaffold metadata itself is rooted at the outer staging directory.
    // Capture both roots before committing so every path in the response is
    // rewritten to the requested output directory.
    let source_project_root = canonical_display(&transaction.source.project_dir);
    let working_project_root = canonical_display(transaction.work_dir());
    let mut kernel = CompilerOperationKernel;
    let receipt = transaction
        .apply_all(&validated, &mut kernel)
        .map_err(|failure| map_plan_failure(failure, &compiled.operation_pointers))?;
    compiled.operation_outcomes = receipt.outcomes;
    transaction.commit_out_dir(out_dir, force)?;

    let from = canonical_display(staging.path());
    let to = canonical_display(out_dir);
    relocate_json_paths(&mut scaffold, &from, &to);
    relocate_json_paths(&mut scaffold, &source_project_root, &to);
    relocate_operation_outcomes(&mut compiled.operation_outcomes, &from, &to);
    relocate_operation_outcomes(&mut compiled.operation_outcomes, &source_project_root, &to);
    relocate_json_paths(&mut scaffold, &working_project_root, &to);
    relocate_operation_outcomes(&mut compiled.operation_outcomes, &working_project_root, &to);
    Ok(scaffold)
}

#[derive(Debug, Default, Clone, Copy)]
struct CompilerOperationKernel;

impl OpKernel for CompilerOperationKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let mut kernel = kernel_for(operation).ok_or_else(|| {
            CliError::unsupported_feature(format!(
                "report build cannot apply operation `{}` because its kernel is not registered",
                operation.tag()
            ))
        })?;
        kernel.apply(operation, transaction)
    }
}

fn map_plan_error(error: crate::ops::PlanError, pointers: &[String]) -> CliError {
    let mut mapped = error.as_cli_error();
    if let Some(index) = operation_index_from_pointer(&error.pointer)
        && let Some(pointer) = pointers.get(index)
    {
        mapped = mapped.with_pointer(pointer.clone());
    }
    mapped
}

fn map_plan_failure(failure: crate::ops::PlanFailure, pointers: &[String]) -> CliError {
    let mut mapped = *failure.error;
    if let Some(pointer) = pointers.get(failure.failed_index) {
        mapped = mapped.with_pointer(pointer.clone());
    }
    mapped
}

fn operation_index_from_pointer(pointer: &str) -> Option<usize> {
    pointer
        .strip_prefix("/ops/")
        .and_then(|value| value.split('/').next())
        .and_then(|value| value.parse().ok())
}

fn relocate_operation_outcomes(outcomes: &mut [OpOutcome], from: &str, to: &str) {
    for outcome in outcomes {
        for command in &mut outcome.readback {
            *command = command.replace(from, to);
        }
        for change in &mut outcome.changes {
            relocate_json_paths(change, from, to);
        }
    }
}

fn relocate_json_paths(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(text) => {
            if text.contains(from) {
                *text = text.replace(from, to);
            }
        }
        Value::Array(items) => {
            for item in items {
                relocate_json_paths(item, from, to);
            }
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                relocate_json_paths(item, from, to);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn apply_model_extensions(
    schema: &mut Map<String, Value>,
    spec: &Map<String, Value>,
) -> CliResult<()> {
    let Some(model) = spec.get("model").and_then(Value::as_object) else {
        return Ok(());
    };
    if let Some(measures) = model.get("measures").and_then(Value::as_array) {
        for measure in measures {
            add_measure_to_schema(schema, measure)?;
        }
    }
    if model.get("relationships").is_some() {
        return Err(CliError::unsupported_feature(
            "report build model.relationships in dashboard spec are planned; put relationships in --schema for this slice"
        )
        .with_suggested_command("powerbi-cli schema validate <schema.json> --json"));
    }
    Ok(())
}

fn add_measure_to_schema(schema: &mut Map<String, Value>, measure: &Value) -> CliResult<()> {
    let measure = measure
        .as_object()
        .ok_or_else(|| CliError::invalid_args("dashboard spec model.measures[] must be objects"))?;
    let table_name = required_string(measure, "table", "model measure")?;
    let tables = schema
        .get_mut("tables")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| CliError::invalid_args("schema must contain tables array"))?;
    let table = tables
        .iter_mut()
        .filter_map(Value::as_object_mut)
        .find(|table| {
            table
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(&table_name))
        })
        .ok_or_else(|| {
            CliError::invalid_args(format!(
                "model measure references missing table {table_name}"
            ))
        })?;
    let measures = table
        .entry("measures".to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            CliError::invalid_args(format!(
                "schema table {table_name} measures must be an array"
            ))
        })?;
    let name = required_string(measure, "name", "model measure")?;
    if measures.iter().any(|existing| {
        existing
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|existing| existing.eq_ignore_ascii_case(&name))
    }) {
        return Ok(());
    }
    let mut out = Map::new();
    for key in [
        "name",
        "expression",
        "description",
        "formatString",
        "displayFolder",
    ] {
        if let Some(value) = measure.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    measures.push(Value::Object(out));
    Ok(())
}

fn compile_pages(
    spec: &Map<String, Value>,
    model: &ModelIndex,
    defaults_applied: &mut Vec<Value>,
) -> CliResult<Vec<Value>> {
    let mut pages = Vec::new();
    for (page_index, page) in spec
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let page = page.as_object().ok_or_else(|| {
            CliError::invalid_args(format!("pages[{page_index}] must be an object"))
        })?;
        let mut out = Map::new();
        if let Some(id) = page
            .get("id")
            .or_else(|| page.get("name"))
            .and_then(Value::as_str)
        {
            out.insert("name".to_string(), Value::String(page_name(id)));
        }
        if let Some(display_name) = page.get("displayName").and_then(Value::as_str) {
            out.insert(
                "displayName".to_string(),
                Value::String(display_name.to_string()),
            );
        }
        if let Some(size) = page.get("size").and_then(Value::as_object) {
            if let Some(width) = size.get("width") {
                out.insert("width".to_string(), width.clone());
            }
            if let Some(height) = size.get("height") {
                out.insert("height".to_string(), height.clone());
            }
        }
        let visuals = compile_visuals(page_index, page, model, defaults_applied)?;
        out.insert("visuals".to_string(), Value::Array(visuals));
        let interactions = compile_interactions(page_index, page)?;
        if !interactions.is_empty() {
            out.insert("interactions".to_string(), Value::Array(interactions));
        }
        pages.push(Value::Object(out));
    }
    Ok(pages)
}

fn compile_filter_operations(
    spec: &Map<String, Value>,
    model: &ModelIndex,
) -> CliResult<(Vec<Op>, Vec<String>)> {
    let mut operations = Vec::new();
    let mut pointers = Vec::new();

    if let Some(filters) = spec.get("filters") {
        let filters = filters.as_array().ok_or_else(|| {
            CliError::invalid_args("dashboard spec filters must be an array")
                .with_pointer("/filters")
        })?;
        for (filter_index, filter) in filters.iter().enumerate() {
            let pointer = format!("/filters/{filter_index}");
            operations.push(compile_filter_operation(
                filter,
                FilterScope::Report,
                "report:main".to_string(),
                None,
                None,
                model,
                &pointer,
            )?);
            pointers.push(pointer);
        }
    }

    for (page_index, page) in spec
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let page = page.as_object().ok_or_else(|| {
            CliError::invalid_args(format!("pages[{page_index}] must be an object"))
        })?;
        let page_name = compiled_page_name(page, page_index)?;
        let page_handle = format!("page:{page_name}");
        if let Some(filters) = page.get("filters") {
            let filters = filters.as_array().ok_or_else(|| {
                CliError::invalid_args(format!("pages[{page_index}].filters must be an array"))
                    .with_pointer(format!("/pages/{page_index}/filters"))
            })?;
            for (filter_index, filter) in filters.iter().enumerate() {
                let pointer = format!("/pages/{page_index}/filters/{filter_index}");
                operations.push(compile_filter_operation(
                    filter,
                    FilterScope::Page,
                    page_handle.clone(),
                    Some(&page_name),
                    None,
                    model,
                    &pointer,
                )?);
                pointers.push(pointer);
            }
        }

        for (visual_index, visual) in page
            .get("visuals")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let visual = visual.as_object().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].visuals[{visual_index}] must be an object"
                ))
            })?;
            let Some(filters) = visual.get("filters") else {
                continue;
            };
            let visual_name = compiled_visual_name(visual, visual_index)?;
            let visual_handle = format!("visual:{page_name}:{visual_name}");
            let filters = filters.as_array().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].visuals[{visual_index}].filters must be an array"
                ))
                .with_pointer(format!(
                    "/pages/{page_index}/visuals/{visual_index}/filters"
                ))
            })?;
            for (filter_index, filter) in filters.iter().enumerate() {
                let pointer =
                    format!("/pages/{page_index}/visuals/{visual_index}/filters/{filter_index}");
                operations.push(compile_filter_operation(
                    filter,
                    FilterScope::Visual,
                    visual_handle.clone(),
                    Some(&page_name),
                    Some(&visual_name),
                    model,
                    &pointer,
                )?);
                pointers.push(pointer);
            }
        }
    }

    Ok((operations, pointers))
}

fn compile_filter_operation(
    value: &Value,
    expected_scope: FilterScope,
    owner: String,
    page_name: Option<&str>,
    visual_name: Option<&str>,
    model: &ModelIndex,
    pointer: &str,
) -> CliResult<Op> {
    let filter = value.as_object().ok_or_else(|| {
        CliError::invalid_args("dashboard spec filter must be an object")
            .with_pointer(pointer.to_string())
    })?;
    let scope = match filter.get("scope") {
        Some(value) => {
            let value = value.as_str().ok_or_else(|| {
                CliError::invalid_args("dashboard spec filter scope must be a string")
                    .with_pointer(format!("{pointer}/scope"))
            })?;
            parse_filter_scope(value)
                .map_err(|error| error.with_pointer(format!("{pointer}/scope")))?
        }
        None => expected_scope,
    };
    if scope != expected_scope {
        return Err(CliError::invalid_args(format!(
            "dashboard spec filter scope `{}` does not match its {} owner",
            scope.as_str(),
            expected_scope.as_str()
        ))
        .with_pointer(format!("{pointer}/scope")));
    }

    let target = filter
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::invalid_args("dashboard spec filter requires target Table[Column]")
                .with_pointer(format!("{pointer}/target"))
        })?;
    let (table, column) = parse_field_reference(target)
        .map_err(|error| error.with_pointer(format!("{pointer}/target")))?;
    let resolved_column = model
        .resolve_filter_column(&table, &column)
        .map_err(|error| error.with_pointer(format!("{pointer}/target")))?;
    let kind = filter.get("kind").and_then(Value::as_str).ok_or_else(|| {
        CliError::invalid_args("dashboard spec filter requires kind")
            .with_pointer(format!("{pointer}/kind"))
    })?;
    let spec = compile_filter_spec(filter, kind, model, pointer)?;
    spec.validate_for(&resolved_column, scope)
        .map_err(|error| error.with_pointer(pointer.to_string()))?;

    let name = generated_filter_name(scope, &resolved_column, &spec);
    let handle = named_filter_handle(
        scope,
        page_name,
        visual_name,
        &name,
        FilterArrayOrigin::FilterConfig,
    );
    let display_name = match filter.get("displayName") {
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| {
                    CliError::invalid_args("dashboard spec filter displayName must be a string")
                        .with_pointer(format!("{pointer}/displayName"))
                })?
                .to_string(),
        ),
        None => None,
    };
    let (filter_type, condition, values, relative) = filter_operation_parts(&spec);
    Ok(Op::AddFilter(crate::ops::AddFilter {
        handle,
        scope: scope.as_str().to_string(),
        owner,
        filter_type,
        target: json!({"table": resolved_column.table, "column": resolved_column.column}),
        name: Some(name),
        display_name,
        condition,
        values,
        relative,
    }))
}

fn compile_filter_spec(
    filter: &Map<String, Value>,
    kind: &str,
    model: &ModelIndex,
    pointer: &str,
) -> CliResult<FilterSpec> {
    let normalized = kind.to_ascii_lowercase().replace(['_', '-'], "");
    match normalized.as_str() {
        "categorical" | "in" => {
            let values = filter
                .get("values")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    CliError::invalid_args(
                        "categorical dashboard spec filters require a values array",
                    )
                    .with_pointer(format!("{pointer}/values"))
                })?;
            Ok(FilterSpec::Categorical { values })
        }
        "range" | "numericrange" | "advanced" => {
            let min = filter.get("min").cloned().filter(|value| !value.is_null());
            let max = filter.get("max").cloned().filter(|value| !value.is_null());
            for (field, value) in [("min", min.as_ref()), ("max", max.as_ref())] {
                if let Some(value) = value
                    && !value.is_number()
                {
                    return Err(CliError::invalid_args(format!(
                        "numeric range dashboard spec filter {field} must be a number"
                    ))
                    .with_pointer(format!("{pointer}/{field}")));
                }
            }
            Ok(FilterSpec::NumericRange { min, max })
        }
        "relativedate" | "relativetime" => {
            let relative = filter
                .get("relative")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    CliError::invalid_args(
                        "relative-date dashboard spec filters require a relative object",
                    )
                    .with_pointer(format!("{pointer}/relative"))
                })?;
            let direction = relative
                .get("direction")
                .or_else(|| relative.get("operator"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CliError::invalid_args("relative filter requires direction")
                        .with_pointer(format!("{pointer}/relative/direction"))
                })?;
            let operator = RelativeDateOperator::parse(direction)
                .map_err(|error| error.with_pointer(format!("{pointer}/relative/direction")))?;
            let unit = relative
                .get("unit")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    CliError::invalid_args("relative filter requires unit")
                        .with_pointer(format!("{pointer}/relative/unit"))
                })?;
            let unit = RelativeDateUnit::parse(unit)
                .map_err(|error| error.with_pointer(format!("{pointer}/relative/unit")))?;
            let span = relative
                .get("span")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    CliError::invalid_args("relative filter requires integer span")
                        .with_pointer(format!("{pointer}/relative/span"))
                })?;
            for field in ["calendar", "includeToday"] {
                if relative.contains_key(field) {
                    return Err(CliError::unsupported_feature(format!(
                        "relative filter field `{field}` is not compiled yet"
                    ))
                    .with_pointer(format!("{pointer}/relative/{field}"))
                    .with_hint(
                        "Use relative direction, unit, and span only; calendar-boundary options require a later filter compiler slice.",
                    ));
                }
            }
            Ok(FilterSpec::RelativeDate {
                operator,
                unit,
                span,
            })
        }
        "topn" | "top" => {
            let top = filter.get("top").and_then(Value::as_u64);
            let bottom = filter.get("bottom").and_then(Value::as_u64);
            let direction_value = filter.get("direction").and_then(Value::as_str);
            let count_value = filter.get("count").and_then(Value::as_u64);
            let (direction, count) = match (top, bottom, direction_value, count_value) {
                (Some(_), Some(_), _, _) => {
                    return Err(CliError::invalid_args(
                        "TopN dashboard spec filters may set top or bottom, not both",
                    )
                    .with_pointer(pointer.to_string()));
                }
                (Some(count), None, _, _) => (TopNDirection::Top, count),
                (None, Some(count), _, _) => (TopNDirection::Bottom, count),
                (None, None, Some(direction), Some(count)) => {
                    let direction = match direction.to_ascii_lowercase().as_str() {
                        "top" | "--top" => TopNDirection::Top,
                        "bottom" | "--bottom" => TopNDirection::Bottom,
                        _ => {
                            return Err(CliError::invalid_args(
                                "TopN direction must be top or bottom",
                            )
                            .with_pointer(format!("{pointer}/direction")));
                        }
                    };
                    (direction, count)
                }
                _ => {
                    return Err(CliError::invalid_args(
                        "TopN dashboard spec filters require top or bottom count",
                    )
                    .with_pointer(format!("{pointer}/top")));
                }
            };
            let by = filter
                .get("by")
                .and_then(|value| {
                    value.as_str().map(ToOwned::to_owned).or_else(|| {
                        Some(format!(
                            "{}[{}]",
                            value.get("table")?.as_str()?,
                            value.get("measure")?.as_str()?
                        ))
                    })
                })
                .ok_or_else(|| {
                    CliError::invalid_args("TopN dashboard spec filters require by measure")
                        .with_pointer(format!("{pointer}/by"))
                })?;
            let by = model
                .resolve_filter_measure(&by)
                .map_err(|error| error.with_pointer(format!("{pointer}/by")))?;
            Ok(FilterSpec::TopN {
                direction,
                count,
                by,
            })
        }
        _ => Err(CliError::unsupported_feature(format!(
            "unsupported dashboard spec filter kind: {kind}"
        ))
        .with_pointer(format!("{pointer}/kind"))),
    }
}

fn filter_operation_parts(spec: &FilterSpec) -> (String, Option<Value>, Vec<Value>, Option<Value>) {
    match spec {
        FilterSpec::Categorical { values } => (
            "Categorical".to_string(),
            Some(json!({"values": values})),
            values.clone(),
            None,
        ),
        FilterSpec::NumericRange { min, max } => (
            "Advanced".to_string(),
            Some(json!({"min": min, "max": max})),
            Vec::new(),
            None,
        ),
        FilterSpec::TopN {
            direction,
            count,
            by,
        } => (
            "TopN".to_string(),
            Some(json!({
                "direction": direction.flag(),
                "count": count,
                "by": {"table": by.table, "measure": by.measure}
            })),
            Vec::new(),
            None,
        ),
        FilterSpec::RelativeDate {
            operator,
            unit,
            span,
        } => (
            "RelativeDate".to_string(),
            None,
            Vec::new(),
            Some(json!({
                "operator": operator.as_str(),
                "unit": unit.as_str(),
                "span": span
            })),
        ),
    }
}

fn parse_filter_scope(value: &str) -> CliResult<FilterScope> {
    match value.trim().to_ascii_lowercase().as_str() {
        "report" => Ok(FilterScope::Report),
        "page" => Ok(FilterScope::Page),
        "visual" => Ok(FilterScope::Visual),
        other => Err(CliError::invalid_args(format!(
            "unsupported dashboard spec filter scope: {other}"
        ))),
    }
}

fn compiled_page_name(page: &Map<String, Value>, page_index: usize) -> CliResult<String> {
    if let Some(value) = page
        .get("id")
        .or_else(|| page.get("name"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(page_name(value));
    }
    Ok(crate::scaffold::object_name(
        "ReportSection",
        page.get("displayName")
            .and_then(Value::as_str)
            .unwrap_or("Page"),
        page_index,
    ))
}

fn compiled_visual_name(visual: &Map<String, Value>, visual_index: usize) -> CliResult<String> {
    if let Some(value) = visual
        .get("id")
        .or_else(|| visual.get("name"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(visual_name(value));
    }
    Ok(crate::scaffold::object_name(
        "VisualContainer",
        visual
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("visual"),
        visual_index,
    ))
}

fn compile_interactions(page_index: usize, page: &Map<String, Value>) -> CliResult<Vec<Value>> {
    let Some(raw_interactions) = page.get("interactions") else {
        return Ok(Vec::new());
    };
    let interactions = raw_interactions.as_array().ok_or_else(|| {
        CliError::invalid_args(format!("pages[{page_index}].interactions must be an array"))
    })?;
    let mut visual_names = BTreeMap::new();
    for visual in page
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = visual
            .get("id")
            .or_else(|| visual.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let name = visual_name(id);
        visual_names.insert(id.to_ascii_lowercase(), name.clone());
        visual_names.insert(name.to_ascii_lowercase(), name);
    }

    let mut compiled = Vec::new();
    let mut pairs = BTreeSet::new();
    for (interaction_index, interaction) in interactions.iter().enumerate() {
        let interaction = interaction.as_object().ok_or_else(|| {
            CliError::invalid_args(format!(
                "pages[{page_index}].interactions[{interaction_index}] must be an object"
            ))
        })?;
        let source_ref = required_string(interaction, "source", "page interaction")?;
        let target_ref = required_string(interaction, "target", "page interaction")?;
        let source = visual_names
            .get(&source_ref.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].interactions[{interaction_index}] source visual {source_ref} does not exist on the page"
                ))
            })?;
        let target = visual_names
            .get(&target_ref.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].interactions[{interaction_index}] target visual {target_ref} does not exist on the page"
                ))
            })?;
        if source == target {
            return Err(CliError::invalid_args(format!(
                "pages[{page_index}].interactions[{interaction_index}] source and target must be different visuals"
            )));
        }
        let interaction_type =
            normalize_interaction_type(&required_string(interaction, "type", "page interaction")?)?;
        if !pairs.insert((source.clone(), target.clone())) {
            return Err(CliError::invalid_args(format!(
                "pages[{page_index}] contains duplicate interactions for source {source_ref} and target {target_ref}"
            )));
        }
        compiled.push(json!({
            "source": source,
            "target": target,
            "type": interaction_type
        }));
    }
    Ok(compiled)
}

fn normalize_interaction_type(value: &str) -> CliResult<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "datafilter" | "data-filter" | "filter" => Ok("DataFilter"),
        "highlightfilter" | "highlight-filter" | "highlight" => Ok("HighlightFilter"),
        "nofilter" | "no-filter" | "none" | "disabled" => Ok("NoFilter"),
        _ => Err(CliError::invalid_args(format!(
            "unsupported dashboard interaction type: {value}"
        ))
        .with_hint("Use DataFilter, HighlightFilter, or NoFilter.")),
    }
}

fn compile_visuals(
    page_index: usize,
    page: &Map<String, Value>,
    model: &ModelIndex,
    defaults_applied: &mut Vec<Value>,
) -> CliResult<Vec<Value>> {
    let mut visuals = Vec::new();
    for (visual_index, visual) in page
        .get("visuals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let visual = visual.as_object().ok_or_else(|| {
            CliError::invalid_args(format!(
                "pages[{page_index}].visuals[{visual_index}] must be an object"
            ))
        })?;
        let requested_type = visual
            .get("type")
            .or_else(|| visual.get("visualType"))
            .and_then(Value::as_str)
            .unwrap_or("card");
        let visual_type = canonical_visual_type(requested_type)?;
        let mut out = Map::new();
        if let Some(id) = visual
            .get("id")
            .or_else(|| visual.get("name"))
            .and_then(Value::as_str)
        {
            out.insert("name".to_string(), Value::String(visual_name(id)));
        }
        out.insert("visualType".to_string(), Value::String(visual_type.clone()));
        let requested_mode = match visual.get("mode") {
            Some(value) => Some(value.as_str().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].visuals[{visual_index}].mode must be a string"
                ))
            })?),
            None => None,
        };
        let slicer_mode = resolve_slicer_mode(&visual_type, requested_mode)?;
        if let Some(mode) = slicer_mode {
            out.insert("mode".to_string(), Value::String(mode.as_str().to_string()));
        }
        if let Some(single_select) = visual.get("singleSelect") {
            let single_select = single_select.as_bool().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].visuals[{visual_index}].singleSelect must be a boolean"
                ))
            })?;
            if visual_type != "slicer" {
                return Err(CliError::invalid_args(format!(
                    "pages[{page_index}].visuals[{visual_index}].singleSelect is supported only for slicer visuals"
                )));
            }
            out.insert("singleSelect".to_string(), Value::Bool(single_select));
        }
        if let Some(title) = visual.get("title").and_then(Value::as_str) {
            out.insert("title".to_string(), Value::String(title.to_string()));
        }
        apply_layout(page_index, visual_index, visual, &mut out, defaults_applied);
        validate_minimum_visual_size(page_index, visual_index, &visual_type, slicer_mode, &out)?;
        let bindings = compile_bindings(page_index, visual_index, &visual_type, visual, model)?;
        validate_binding_contract(page_index, visual_index, &visual_type, &bindings)?;
        if slicer_mode == Some(SlicerMode::Between) {
            validate_between_binding(page_index, visual_index, &bindings, model)?;
        }
        out.insert("bindings".to_string(), Value::Array(bindings));
        if visual.get("drilldown").is_some() {
            return Err(CliError::unsupported_feature(
                "report build drilldown from dashboard spec is planned for a later slice; build first, then run report drilldown set-hierarchy"
            )
            .with_suggested_command(
                "powerbi-cli report drilldown set-hierarchy --project <project-dir> --handle <visual-handle> --field <Table[Column]> --field <Table[Column]> --dry-run --json",
            ));
        }
        visuals.push(Value::Object(out));
    }
    Ok(visuals)
}

fn validate_minimum_visual_size(
    page_index: usize,
    visual_index: usize,
    visual_type: &str,
    slicer_mode: Option<SlicerMode>,
    visual: &Map<String, Value>,
) -> CliResult<()> {
    if visual_type == "slicer" {
        let minimum = if slicer_mode == Some(SlicerMode::Between) {
            BETWEEN_SLICER_MIN_HEIGHT
        } else {
            SLICER_MIN_HEIGHT
        };
        let qualifier = if slicer_mode == Some(SlicerMode::Between) {
            "Between slicer"
        } else {
            "slicer"
        };
        let height = visual
            .get("height")
            .and_then(Value::as_f64)
            .ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].visuals[{visual_index}] {qualifier} height must be a number of at least {minimum} for Power BI compatibility"
                ))
            })?;
        if height < minimum {
            return Err(CliError::invalid_args(format!(
                "pages[{page_index}].visuals[{visual_index}] {qualifier} height must be at least {minimum} for Power BI compatibility"
            )));
        }
    }
    Ok(())
}

fn validate_between_binding(
    page_index: usize,
    visual_index: usize,
    bindings: &[Value],
    model: &ModelIndex,
) -> CliResult<()> {
    let binding = bindings.first().and_then(Value::as_object).ok_or_else(|| {
        CliError::invalid_args(format!(
            "pages[{page_index}].visuals[{visual_index}] Between slicer requires one Values column binding"
        ))
    })?;
    let table = binding
        .get("table")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let column = binding
        .get("column")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let data_type = model.column_data_type(table, column).unwrap_or("unknown");
    if slicer_between_data_type_is_supported(data_type) {
        return Ok(());
    }
    Err(CliError::unsupported_feature(format!(
        "pages[{page_index}].visuals[{visual_index}] Between slicer requires a numeric or date column; {table}[{column}] is {data_type}"
    ))
    .with_hint("Use Basic/Dropdown for text categories, or bind Between to an int64, double, decimal, or dateTime column."))
}

fn compile_bindings(
    page_index: usize,
    visual_index: usize,
    visual_type: &str,
    visual: &Map<String, Value>,
    model: &ModelIndex,
) -> CliResult<Vec<Value>> {
    let mut bindings = Vec::new();
    for (binding_index, binding) in visual
        .get("bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let binding = binding.as_object().ok_or_else(|| {
            CliError::invalid_args(format!(
                "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}] must be an object"
            ))
        })?;
        let binding_pointer =
            format!("/pages/{page_index}/visuals/{visual_index}/bindings/{binding_index}");
        let role_value = binding
            .get("role")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                spec_missing_input(
                    format!("{binding_pointer}/role"),
                    "visuals[].bindings[].role",
                    "every visual binding must declare the field-well role it populates",
                    json!({"role": "Values"}),
                )
            })?;
        let role = normalize_role(visual_type, role_value)?;
        let mut out = Map::new();
        out.insert("role".to_string(), Value::String(role));
        if let Some(field) = binding
            .get("field")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            let field = model.resolve_field(field).map_err(|error| {
                if error
                    .message
                    .contains("dashboard spec field reference does not exist in schema")
                {
                    spec_missing_input(
                        format!("{binding_pointer}/field"),
                        "visuals[].bindings[].field",
                        error.message,
                        json!({"field": "FactSales[Total Revenue]"}),
                    )
                } else {
                    error
                }
            })?;
            out.insert("table".to_string(), Value::String(field.table));
            match field.kind {
                FieldKind::Column => out.insert("column".to_string(), Value::String(field.name)),
                FieldKind::Measure => out.insert("measure".to_string(), Value::String(field.name)),
            };
        } else {
            let table = binding
                .get("table")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    spec_missing_input(
                        format!("{binding_pointer}/table"),
                        "visuals[].bindings[].table",
                        "a structured binding needs a table so its column or measure can be resolved",
                        json!({"table": "FactSales"}),
                    )
                })?;
            let column = binding
                .get("column")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty());
            let measure = binding
                .get("measure")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty());
            match (column, measure) {
                (Some(column), None) => {
                    model
                        .resolve_structured_field(table, column, FieldKind::Column)
                        .map_err(|error| {
                            spec_missing_input(
                                format!("{binding_pointer}/column"),
                                "visuals[].bindings[].column",
                                error.message,
                                json!({"column": "Total Revenue"}),
                            )
                        })?;
                    out.insert("table".to_string(), Value::String(table.to_string()));
                    out.insert("column".to_string(), Value::String(column.to_string()));
                }
                (None, Some(measure)) => {
                    model
                        .resolve_structured_field(table, measure, FieldKind::Measure)
                        .map_err(|error| {
                            spec_missing_input(
                                format!("{binding_pointer}/measure"),
                                "visuals[].bindings[].measure",
                                error.message,
                                json!({"measure": "Total Revenue"}),
                            )
                        })?;
                    out.insert("table".to_string(), Value::String(table.to_string()));
                    out.insert("measure".to_string(), Value::String(measure.to_string()));
                }
                (Some(_), Some(_)) => {
                    return Err(CliError::invalid_args(
                        "visual binding must set either column or measure, not both",
                    ));
                }
                (None, None) => {
                    return Err(spec_missing_input(
                        format!("{binding_pointer}/field"),
                        "visuals[].bindings[].field",
                        "a binding must identify a field, or provide table plus exactly one column or measure",
                        json!({"field": "FactSales[Total Revenue]"}),
                    ));
                }
            }
        }
        for key in ["displayName", "formatString"] {
            if let Some(value) = binding.get(key) {
                out.insert(key.to_string(), value.clone());
            }
        }
        if let Some(value) = binding.get("sortDirection") {
            let direction = value.as_str().ok_or_else(|| {
                CliError::invalid_args(format!(
                    "pages[{page_index}].visuals[{visual_index}].bindings[{binding_index}].sortDirection must be a string"
                ))
            })?;
            if !matches!(
                direction.trim().to_ascii_lowercase().as_str(),
                "descending" | "desc"
            ) {
                return Err(CliError::unsupported_feature(format!(
                    "unsupported visual sort direction: {direction}"
                ))
                .with_hint("The first typed slice supports descending measure sort only."));
            }
            out.insert(
                "sortDirection".to_string(),
                Value::String("Descending".to_string()),
            );
        }
        bindings.push(Value::Object(out));
    }
    Ok(bindings)
}

fn validate_binding_contract(
    page_index: usize,
    visual_index: usize,
    visual_type: &str,
    bindings: &[Value],
) -> CliResult<()> {
    use crate::visual_catalog::VisualBindingFamily;

    let family = crate::visual_catalog::binding_family(visual_type)?;
    let count = |role: &str| {
        bindings
            .iter()
            .filter(|binding| binding.get("role").and_then(Value::as_str) == Some(role))
            .count()
    };
    let has_measure = |role: &str| {
        bindings.iter().any(|binding| {
            binding.get("role").and_then(Value::as_str) == Some(role)
                && binding.get("measure").is_some()
        })
    };
    let visual_path = || format!("pages[{page_index}].visuals[{visual_index}]");
    match family {
        VisualBindingFamily::SingleValue => {
            let values = count("Values");
            if values != 1 || !has_measure("Values") {
                return Err(CliError::invalid_args(format!(
                    "{} card requires exactly one Values measure binding, got {values}",
                    visual_path()
                )));
            }
        }
        VisualBindingFamily::ValuesList => {
            let values = count("Values");
            if values < 1 {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} requires at least one Values binding",
                    visual_path()
                ))
                .with_suggested_command(format!(
                    "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
                )));
            }
        }
        VisualBindingFamily::CategoryY => {
            let categories = count("Category");
            let y = count("Y");
            let series = count("Series");
            if categories < 1 || y < 1 || series > 1 {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} requires at least one Category, at least one Y, and at most one Series binding",
                    visual_path()
                ))
                .with_suggested_command(format!(
                    "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
                )));
            }
            if has_measure("Category") || has_measure("Series") {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} Category and Series bindings must be columns, not measures",
                    visual_path()
                )));
            }
            if bindings.iter().any(|binding| {
                binding.get("role").and_then(Value::as_str) == Some("Y")
                    && binding.get("measure").is_none()
            }) {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} Y bindings must be measures, not columns",
                    visual_path()
                )));
            }
        }
        VisualBindingFamily::CategorySeriesYAggregatable => {
            let categories = count("Category");
            let y = count("Y");
            let series = count("Series");
            if categories < 1 || y < 1 || series > 1 {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} requires at least one Category, at least one Y, and at most one Series binding",
                    visual_path()
                ))
                .with_suggested_command(format!(
                    "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
                )));
            }
            if has_measure("Category") || has_measure("Series") {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} Category and Series bindings must be columns, not measures",
                    visual_path()
                )));
            }
        }
        VisualBindingFamily::ComboCategoryY => {
            let categories = count("Category");
            let y = count("Y");
            let y2 = count("Y2");
            if categories < 1 || y < 1 || y2 < 1 {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} requires at least one Category column, at least one column-axis Y measure, and at least one line-axis Y2 measure",
                    visual_path()
                ))
                .with_suggested_command(format!(
                    "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
                )));
            }
            if has_measure("Category") {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} Category bindings must be columns, not measures",
                    visual_path()
                )));
            }
            if bindings.iter().any(|binding| {
                matches!(
                    binding.get("role").and_then(Value::as_str),
                    Some("Y" | "Y2")
                ) && binding.get("measure").is_none()
            }) {
                return Err(CliError::unsupported_feature(format!(
                    "{} {visual_type} Y and Y2 bindings require measures; bare columns are not fixture-proven",
                    visual_path()
                ))
                .with_hint("Define measures for both value axes; the compiler refuses to invent an aggregation shape for combo charts.")
                .with_suggested_command(format!(
                    "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
                )));
            }
        }
        VisualBindingFamily::CategoryShare => {
            let categories = count("Category");
            let y = count("Y");
            if categories != 1 || y < 1 {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} requires exactly one Category column binding and at least one Y binding; got {categories} Category and {y} Y bindings",
                    visual_path()
                ))
                .with_suggested_command(format!(
                    "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
                )));
            }
            if has_measure("Category") {
                return Err(CliError::invalid_args(format!(
                    "{} {visual_type} Category binding must be a column, not a measure",
                    visual_path()
                )));
            }
            if bindings.iter().any(|binding| {
                binding.get("role").and_then(Value::as_str) == Some("Y")
                    && binding.get("measure").is_none()
            }) {
                return Err(CliError::unsupported_feature(format!(
                    "{} {visual_type} Y bindings require measures; bare columns are not proven by the Desktop-authored reference",
                    visual_path()
                ))
                .with_hint("Define a measure for the value well; the reference fixture proves measure projections only.")
                .with_suggested_command(format!(
                    "powerbi-cli report visuals catalog --visual-type {visual_type} --json"
                )));
            }
        }
        VisualBindingFamily::RowsColumnsValues => {
            let rows = count("Rows");
            let columns = count("Columns");
            let values = count("Values");
            if rows < 1 || values < 1 {
                return Err(CliError::invalid_args(format!(
                    "{} matrix (pivotTable) requires at least one Rows column binding and at least one Values binding; Columns are optional; got {rows} Rows, {columns} Columns, and {values} Values bindings",
                    visual_path()
                ))
                .with_suggested_command(
                    "powerbi-cli report visuals catalog --visual-type matrix --json",
                ));
            }
            if has_measure("Rows") || has_measure("Columns") {
                return Err(CliError::invalid_args(format!(
                    "{} matrix (pivotTable) Rows and Columns bindings must be columns, not measures",
                    visual_path()
                )));
            }
            if bindings.iter().any(|binding| {
                binding.get("role").and_then(Value::as_str) == Some("Values")
                    && binding.get("measure").is_none()
            }) {
                return Err(CliError::unsupported_feature(format!(
                    "{} matrix (pivotTable) Values bindings require measures; bare columns are not proven by the Desktop-authored reference",
                    visual_path()
                ))
                .with_hint("Define a measure for matrix Values; the reference fixture does not prove an aggregation wrapper for raw columns.")
                .with_suggested_command(
                    "powerbi-cli report visuals catalog --visual-type matrix --json",
                ));
            }
        }
        VisualBindingFamily::SlicerField => {
            let values = count("Values");
            if values != 1 || has_measure("Values") {
                return Err(CliError::invalid_args(format!(
                    "{} slicer requires exactly one Values column binding; got {values} Values bindings{}",
                    visual_path(),
                    if has_measure("Values") {
                        ", including a measure"
                    } else {
                        ""
                    }
                ))
                .with_suggested_command(
                    "powerbi-cli report visuals catalog --visual-type slicer --json",
                ));
            }
        }
        VisualBindingFamily::ScatterBubble => {
            let x = count("X");
            let y = count("Y");
            if x != 1 || y != 1 {
                return Err(CliError::invalid_args(format!(
                    "{} scatterChart requires exactly one X and exactly one Y binding",
                    visual_path()
                ))
                .with_suggested_command(
                    "powerbi-cli report visuals catalog --visual-type scatterChart --json",
                ));
            }
            for role in ["Category", "Size", "Series"] {
                if count(role) > 1 {
                    return Err(CliError::invalid_args(format!(
                        "{} scatterChart accepts at most one {role} binding",
                        visual_path()
                    )));
                }
            }
            if has_measure("Category") || has_measure("Series") {
                return Err(CliError::invalid_args(format!(
                    "{} scatterChart Category and Series bindings must be columns, not measures",
                    visual_path()
                )));
            }
        }
    }
    let sorted = bindings
        .iter()
        .filter(|binding| binding.get("sortDirection").is_some())
        .collect::<Vec<_>>();
    if sorted.len() > 1 {
        return Err(CliError::unsupported_feature(format!(
            "{} supports exactly one explicit sort binding",
            visual_path()
        )));
    }
    if let Some(binding) = sorted.first() {
        let role = binding
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if binding.get("measure").is_none() {
            return Err(CliError::unsupported_feature(format!(
                "{} explicit visual sort is currently proven only for measures",
                visual_path()
            )));
        }
        if !matches!(role, "Y" | "Y2" | "Values" | "Tooltips") {
            return Err(CliError::unsupported_feature(format!(
                "{} explicit visual sort is not supported on role {role}",
                visual_path()
            )));
        }
    }
    Ok(())
}

fn apply_layout(
    page_index: usize,
    visual_index: usize,
    visual: &Map<String, Value>,
    out: &mut Map<String, Value>,
    defaults_applied: &mut Vec<Value>,
) {
    let layout = visual.get("layout").and_then(Value::as_object);
    for key in ["x", "y", "width", "height"] {
        if let Some(value) = visual
            .get(key)
            .or_else(|| layout.and_then(|layout| layout.get(key)))
        {
            out.insert(key.to_string(), value.clone());
        }
    }
    if !out.contains_key("x") {
        let x = 32.0 + ((visual_index % 2) as f64 * 608.0);
        let y = 32.0 + ((visual_index / 2) as f64 * 216.0);
        out.insert("x".to_string(), Value::from(x));
        out.insert("y".to_string(), Value::from(y));
        out.insert("width".to_string(), Value::from(560.0));
        out.insert("height".to_string(), Value::from(184.0));
        let pointer = format!("/pages/{page_index}/visuals/{visual_index}");
        defaults_applied.push(json!({
            "pointer": format!("{pointer}/x"),
            "field": "visuals[].layout.x",
            "value": x,
            "reason": "no explicit visual layout was supplied; the deterministic two-column grid applies"
        }));
        defaults_applied.push(json!({
            "pointer": format!("{pointer}/y"),
            "field": "visuals[].layout.y",
            "value": y,
            "reason": "no explicit visual layout was supplied; the deterministic two-column grid applies"
        }));
        defaults_applied.push(json!({
            "pointer": format!("{pointer}/width"),
            "field": "visuals[].layout.width",
            "value": 560.0,
            "reason": "no explicit visual layout was supplied; the deterministic two-column grid applies"
        }));
        defaults_applied.push(json!({
            "pointer": format!("{pointer}/height"),
            "field": "visuals[].layout.height",
            "value": 184.0,
            "reason": "no explicit visual layout was supplied; the deterministic two-column grid applies"
        }));
    }
}

#[derive(Debug)]
struct ModelIndex {
    columns: BTreeMap<String, BTreeMap<String, String>>,
    measures: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug)]
struct FieldRef {
    table: String,
    name: String,
    kind: FieldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    Column,
    Measure,
}

impl ModelIndex {
    fn from_schema(schema: &Value) -> Self {
        let mut columns = BTreeMap::new();
        let mut measures = BTreeMap::new();
        for table in schema["tables"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
        {
            let table_name = table
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let table_key = table_name.to_ascii_lowercase();
            columns.insert(
                table_key.clone(),
                table
                    .get("columns")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|column| {
                        let name = column.get("name").and_then(Value::as_str)?;
                        let data_type = column
                            .get("dataType")
                            .and_then(Value::as_str)
                            .unwrap_or("string");
                        Some((name.to_ascii_lowercase(), data_type.to_string()))
                    })
                    .collect(),
            );
            measures.insert(
                table_key,
                table
                    .get("measures")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|measure| measure.get("name").and_then(Value::as_str))
                    .map(|name| name.to_ascii_lowercase())
                    .collect(),
            );
        }
        Self { columns, measures }
    }

    fn resolve_field(&self, value: &str) -> CliResult<FieldRef> {
        let (table, name) = parse_field(value)?;
        let table_key = table.to_ascii_lowercase();
        let name_key = name.to_ascii_lowercase();
        let is_measure = self
            .measures
            .get(&table_key)
            .is_some_and(|items| items.contains(&name_key));
        let is_column = self
            .columns
            .get(&table_key)
            .is_some_and(|items| items.contains_key(&name_key));
        if is_measure && is_column {
            return Err(CliError::invalid_args(format!(
                "dashboard spec field reference is ambiguous because both a column and measure exist: {value}"
            ))
            .with_hint(
                "Use a structured binding with table+column or table+measure to disambiguate.",
            ));
        }
        if is_measure {
            return Ok(FieldRef {
                table,
                name,
                kind: FieldKind::Measure,
            });
        }
        if is_column {
            return Ok(FieldRef {
                table,
                name,
                kind: FieldKind::Column,
            });
        }
        Err(CliError::invalid_args(format!(
            "dashboard spec field reference does not exist in schema: {value}"
        ))
        .with_suggested_command("powerbi-cli schema validate <schema.json> --json"))
    }

    fn resolve_structured_field(&self, table: &str, name: &str, kind: FieldKind) -> CliResult<()> {
        let table_key = table.to_ascii_lowercase();
        let name_key = name.to_ascii_lowercase();
        let found = match kind {
            FieldKind::Column => self
                .columns
                .get(&table_key)
                .is_some_and(|items| items.contains_key(&name_key)),
            FieldKind::Measure => self
                .measures
                .get(&table_key)
                .is_some_and(|items| items.contains(&name_key)),
        };
        if found {
            Ok(())
        } else {
            Err(CliError::invalid_args(format!(
                "dashboard spec structured binding references missing {kind:?}: {table}[{name}]"
            ))
            .with_suggested_command("powerbi-cli schema validate <schema.json> --json"))
        }
    }

    fn resolve_filter_column(&self, table: &str, column: &str) -> CliResult<ResolvedFilterColumn> {
        let table_key = table.to_ascii_lowercase();
        let column_key = column.to_ascii_lowercase();
        if !self
            .columns
            .get(&table_key)
            .is_some_and(|columns| columns.contains_key(&column_key))
        {
            return Err(CliError::invalid_args(format!(
                "dashboard spec filter target does not exist in schema: {table}[{column}]"
            )));
        }
        Ok(ResolvedFilterColumn {
            table: table.to_string(),
            column: column.to_string(),
            data_type: self.column_data_type(table, column).map(ToOwned::to_owned),
        })
    }

    fn resolve_filter_measure(&self, value: &str) -> CliResult<ResolvedFilterMeasure> {
        let (table, measure) = parse_field_reference(value)?;
        let table_key = table.to_ascii_lowercase();
        let measure_key = measure.to_ascii_lowercase();
        if !self
            .measures
            .get(&table_key)
            .is_some_and(|measures| measures.contains(&measure_key))
        {
            return Err(CliError::invalid_args(format!(
                "dashboard spec TopN measure does not exist in schema: {table}[{measure}]"
            )));
        }
        Ok(ResolvedFilterMeasure { table, measure })
    }

    fn column_data_type(&self, table: &str, column: &str) -> Option<&str> {
        self.columns
            .get(&table.to_ascii_lowercase())?
            .get(&column.to_ascii_lowercase())
            .map(String::as_str)
    }
}

fn parse_field(value: &str) -> CliResult<(String, String)> {
    let (table, rest) = value.split_once('[').ok_or_else(|| {
        CliError::invalid_args(format!(
            "field reference must use Table[Field] syntax: {value}"
        ))
    })?;
    let field = rest.strip_suffix(']').ok_or_else(|| {
        CliError::invalid_args(format!(
            "field reference must use Table[Field] syntax: {value}"
        ))
    })?;
    Ok((table.to_string(), field.to_string()))
}

fn validate_spec_shape(spec: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(object) = spec.as_object() else {
        return vec!["dashboard spec root must be an object".to_string()];
    };
    if object.get("report").is_none() && object.get("pages").is_none() {
        errors.push("dashboard spec requires report/pages or legacy top-level pages".to_string());
    }
    errors
}

fn build_response(response: BuildResponse<'_>) -> Value {
    let project_dir = response.out_dir.map(canonical_display);
    let compiled = compiled_summary(response.compiled);
    let execution = legacy_execution_trace(response.compiled, response.out_dir, response.dry_run);
    // A typed compiler build has one outcome per operation. Use those concrete
    // kernel changes/readbacks in its response; legacy/schema-only builds keep
    // the aggregate execution adapter used by the public build contract.
    let typed_execution = !response.compiled.operation_outcomes.is_empty();
    let mut changes = if typed_execution {
        vec![json!({
            "kind": "pbip.project",
            "action": "create",
            "path": project_dir.clone(),
            "before": Value::Null,
            "after": {
                "projectDir": project_dir.clone(),
                "counts": compiled["counts"].clone()
            }
        })]
    } else {
        execution.changes.clone()
    };
    if typed_execution {
        for outcome in &response.compiled.operation_outcomes {
            changes.extend(outcome.changes.iter().cloned());
        }
    }
    let mut executed_primitives = if response.changed {
        vec![
            json!({"command": "scaffold", "reason": "report build compiled schema/spec into scaffold-compatible manifest"}),
        ]
    } else {
        Vec::new()
    };
    executed_primitives.extend(response.compiled.operation_outcomes.iter().map(|outcome| {
        json!({
            "operationCount": 1,
            "changed": outcome.changed,
            "createdHandles": outcome.created_handles,
        })
    }));
    let readback = if typed_execution {
        let mut grouped = BTreeMap::<String, Vec<String>>::new();
        for outcome in &response.compiled.operation_outcomes {
            for (handle, command) in outcome.created_handles.iter().zip(&outcome.readback) {
                grouped
                    .entry(handle.clone())
                    .or_default()
                    .push(command.clone());
            }
        }
        json!(grouped)
    } else {
        execution.readback.clone()
    };
    let scope = if typed_execution {
        json!({
            "kind": "report-build",
            "mode": if response.dry_run { "dry-run" } else { "out-dir" },
            "projectDir": project_dir,
            "operationCount": response.compiled.operation_outcomes.len(),
            "handles": response.compiled.operation_outcomes.iter().flat_map(|outcome| outcome.created_handles.iter().cloned()).collect::<Vec<_>>()
        })
    } else {
        execution.scope.clone()
    };
    let validation = response
        .out_dir
        .and_then(|path| crate::resolve_project(path).ok())
        .and_then(|project| validate_project(&project).ok());
    let scorecard = response
        .out_dir
        .and_then(|path| crate::resolve_project(path).ok())
        .map(|project| project_scorecard(&project, "unit-smoke"))
        .unwrap_or_else(|| {
            dry_run_scorecard(
                "unit-smoke",
                next_for_build(
                    response.out_dir,
                    response.dry_run,
                    response.schema_path,
                    response.spec_path,
                    response.proof_plan,
                ),
            )
        });
    let mut output = json!({
        "schema": "powerbi-cli.report.build.v1",
        "ok": validation.as_ref().is_none_or(|validation| validation.errors.is_empty()),
        "changed": response.changed,
        "dryRun": response.dry_run,
        "projectDir": project_dir,
        "inputs": {
            "schema": canonical_display(response.schema_path),
            "profile": response.profile_path.map(canonical_display),
            "spec": response.spec_path.map(canonical_display)
        },
        "compiled": compiled,
        "defaultsApplied": response.compiled.defaults_applied,
        "changes": changes,
        "readback": readback,
        "scope": scope,
        "profileSummary": response.profile.map(profile_summary),
        "executedPrimitives": executed_primitives,
        "operations": response.compiled.operations,
        "operationOutcomes": response.compiled.operation_outcomes,
        "warnings": response.compiled.warnings,
        "validation": validation.as_ref().map(|validation| json!({
            "ok": validation.errors.is_empty(),
            "errors": validation.errors,
            "warnings": validation.warnings
        })),
        "scaffold": response.scaffold,
        "inspectCommand": response.out_dir.map(|path| format!("powerbi-cli inspect --deep {} --json", command_arg(path))),
        "validateCommand": response.out_dir.map(|path| format!("powerbi-cli validate --strict {} --json", command_arg(path))),
        "handoffCheckCommand": response.out_dir.map(|path| format!("powerbi-cli handoff check {} --json", command_arg(path))),
        "fixtureNormalizeCommand": response.out_dir.map(|path| format!("powerbi-cli fixture normalize {} --out testdata/golden/<name>.summary.json --json", command_arg(path))),
        "desktopOpenCheckCommand": response.out_dir.map(|path| format!("powerbi-cli desktop open-check {} --json", command_arg(path))),
        "proof": {
            "claimedDesktopCompatibility": false,
            "requiredForCompatibility": "desktop-canvas-refresh",
            "note": "report build writes local PBIP/PBIR/TMDL metadata; Desktop canvas/refresh proof is a separate oracle step"
        },
        "proofPlan": response.proof_plan.map(|plan| plan.value.clone()),
        "next": next_for_build(
            response.out_dir,
            response.dry_run,
            response.schema_path,
            response.spec_path,
            response.proof_plan,
        ),
        "scorecard": scorecard
    });
    if response.trace {
        output["trace"] = Value::Array(execution.trace);
    }
    output
}

fn compiled_summary(compiled: &CompiledDashboard) -> Value {
    let validation = validate_schema_value(&compiled.schema);
    json!({
        "counts": {
            "tables": validation.counts.tables,
            "columns": validation.counts.columns,
            "measures": validation.counts.measures,
            "relationships": validation.counts.relationships,
            "pages": validation.counts.pages,
            "visuals": validation.counts.visuals,
            "bindings": validation.counts.bindings,
            "rows": validation.counts.rows
        },
        "ops": compiled.operations.len(),
        "tables": validation.tables,
        "defaultsApplied": compiled.defaults_applied
    })
}

/// The report compiler still emits a compact legacy operation summary.  Keep
/// that implementation detail behind one adapter that has the same aggregate
/// fields as an OpPlan transaction receipt: flattened changes, stable-handle
/// readbacks, and an opt-in deterministic timing trace.  The adapter materializes
/// `OpOutcome`s here so switching the compiler to a real OpPlan later changes
/// only this boundary, not the public build response.
struct BuildExecutionTrace {
    changes: Vec<Value>,
    readback: Value,
    trace: Vec<Value>,
    scope: Value,
}

fn legacy_execution_trace(
    compiled: &CompiledDashboard,
    out_dir: Option<&Path>,
    dry_run: bool,
) -> BuildExecutionTrace {
    let project_dir = out_dir.map(canonical_display);
    let project_arg = out_dir
        .map(command_arg)
        .unwrap_or_else(|| "<project-dir>".to_string());
    let generated = generated_handles(&compiled.schema);
    let mut outcomes = Vec::with_capacity(compiled.operations.len());
    let mut trace = Vec::with_capacity(compiled.operations.len());
    for (index, operation) in compiled.operations.iter().enumerate() {
        let kind = operation["kind"].as_str().unwrap_or("operation");
        let change = if index == 0 {
            json!({
                "kind": "pbip.project",
                "action": "create",
                "op": kind,
                "path": project_dir.clone(),
                "before": Value::Null,
                "after": {
                    "projectDir": project_dir.clone(),
                    "counts": compiled_summary(compiled)["counts"].clone()
                }
            })
        } else {
            json!({
                "kind": "operation",
                "action": if dry_run { "plan" } else { "apply" },
                "op": kind,
                "path": project_dir.clone(),
                "before": Value::Null,
                "after": {
                    "index": index,
                    "summary": operation["summary"].clone()
                }
            })
        };
        let created_handles = if index == 0 {
            let mut handles = Vec::with_capacity(generated.len() + 1);
            handles.push("report:main".to_string());
            handles.extend(generated.iter().cloned());
            handles
        } else {
            Vec::new()
        };
        // Legacy compilation has one aggregate operation for the generated
        // project. Keep its readback commands in the same order as
        // `created_handles`, allowing the response adapter below to expose
        // per-handle command arrays while still consuming the OpOutcome shape.
        let readback = created_handles
            .iter()
            .filter_map(|handle| readback_command_for_handle(handle, &project_arg))
            .collect();
        outcomes.push(OpOutcome {
            changed: !dry_run,
            changes: vec![change],
            readback,
            warnings: Vec::new(),
            created_handles,
        });
        // Wall-clock durations would make an otherwise deterministic JSON
        // response impossible to snapshot.  The legacy adapter reports the
        // deterministic planning bucket (zero milliseconds); a future
        // transaction adapter can supply rounded buckets from its trace.
        trace.push(json!({"op": kind, "ms": 0}));
    }

    let mut readback = BTreeMap::<String, Vec<String>>::new();
    for outcome in &outcomes {
        for (handle, command) in outcome.created_handles.iter().zip(&outcome.readback) {
            readback
                .entry(handle.clone())
                .or_default()
                .push(command.clone());
        }
    }
    let changes = outcomes
        .into_iter()
        .flat_map(|outcome| outcome.changes)
        .collect();
    let handles = readback.keys().cloned().collect::<Vec<_>>();
    BuildExecutionTrace {
        changes,
        readback: json!(readback),
        trace,
        scope: json!({
            "kind": "report-build",
            "mode": if dry_run { "dry-run" } else { "out-dir" },
            "projectDir": project_dir,
            "operationCount": compiled.operations.len(),
            "handles": handles
        }),
    }
}

fn generated_handles(schema: &Value) -> Vec<String> {
    let mut handles = Vec::new();
    for table in schema["tables"].as_array().into_iter().flatten() {
        let Some(table_name) = table["name"].as_str() else {
            continue;
        };
        handles.push(table_handle(table_name));
        for measure in table["measures"].as_array().into_iter().flatten() {
            if let Some(name) = measure["name"].as_str() {
                handles.push(measure_handle(table_name, name));
            }
        }
    }
    for page in schema["pages"].as_array().into_iter().flatten() {
        let Some(page_name_value) = page["name"]
            .as_str()
            .or_else(|| page["displayName"].as_str())
        else {
            continue;
        };
        let page_handle = format!("page:{}", page_name(page_name_value));
        handles.push(page_handle.clone());
        for (visual_index, visual) in page["visuals"].as_array().into_iter().flatten().enumerate() {
            let name = visual["name"]
                .as_str()
                .or_else(|| visual["id"].as_str())
                .or_else(|| visual["title"].as_str())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("visual-{visual_index}"));
            handles.push(format!(
                "visual:{}:{}",
                page_name(page_name_value),
                visual_name(&name)
            ));
        }
    }
    handles.sort();
    handles.dedup();
    handles
}

fn readback_command_for_handle(handle: &str, project_arg: &str) -> Option<String> {
    let handle_arg = shell_arg(handle);
    if handle == "report:main" {
        return Some(format!("powerbi-cli inspect --deep {project_arg} --json"));
    }
    if handle.starts_with("page:") {
        return Some(format!(
            "powerbi-cli report pages show --project {project_arg} --handle {handle_arg} --json"
        ));
    }
    if handle.starts_with("visual:") {
        return Some(format!(
            "powerbi-cli report visuals show --project {project_arg} --handle {handle_arg} --json"
        ));
    }
    if handle.starts_with("measure:") {
        return Some(format!(
            "powerbi-cli model measures show --project {project_arg} --handle {handle_arg} --json"
        ));
    }
    if handle.starts_with("table:") {
        return Some(format!(
            "powerbi-cli model tables show --project {project_arg} --handle {handle_arg} --json"
        ));
    }
    None
}

fn next_for_build(
    out_dir: Option<&Path>,
    dry_run: bool,
    schema_path: &Path,
    spec_path: Option<&Path>,
    proof_plan: Option<&ProofPlan>,
) -> Vec<String> {
    if dry_run {
        let mut commands = vec![format!(
            "powerbi-cli report build --schema {}{} --out-dir <project-dir> --json",
            command_arg(schema_path),
            spec_path
                .map(|path| format!(" --spec {}", command_arg(path)))
                .unwrap_or_default()
        )];
        if let Some(plan) = proof_plan {
            commands.extend(plan.next.iter().cloned());
        }
        return commands;
    }
    let mut commands = out_dir
        .map(|path| {
            vec![
                format!("powerbi-cli inspect --deep {} --json", command_arg(path)),
                format!("powerbi-cli validate --strict {} --json", command_arg(path)),
                format!("powerbi-cli handoff check {} --json", command_arg(path)),
                format!(
                    "powerbi-cli fixture normalize {} --out testdata/golden/<name>.summary.json --json",
                    command_arg(path)
                ),
                format!("powerbi-cli desktop open-check {} --json", command_arg(path)),
            ]
        })
        .unwrap_or_default();
    if let Some(plan) = proof_plan {
        commands.extend(plan.next.iter().cloned());
    }
    commands
}

fn next_for_spec_validate(
    spec_path: &Path,
    schema_path: Option<&Path>,
    ok: bool,
    validation_level: &str,
    proof_plan: Option<&ProofPlan>,
) -> Vec<String> {
    if !ok {
        return Vec::new();
    }
    let mut commands = Vec::new();
    if let Some(schema_path) = schema_path {
        commands.push(format!(
            "powerbi-cli report build --schema {} --spec {} --dry-run --json",
            command_arg(schema_path),
            command_arg(spec_path)
        ));
    } else if validation_level == "shape-only" {
        commands.push(format!(
            "powerbi-cli report spec validate --schema <schema.json> --spec {} --json",
            command_arg(spec_path)
        ));
    }
    if let Some(plan) = proof_plan {
        commands.extend(plan.next.iter().cloned());
    }
    commands
}

fn load_optional_value(path: Option<&Path>, label: &str) -> CliResult<Option<Value>> {
    path.map(|path| {
        if label == "dashboard spec" {
            Ok(normalize_spec_file(path)?.value)
        } else {
            load_json_value(path, label)
        }
    })
    .transpose()
}

fn load_json_value(path: &Path, label: &str) -> CliResult<Value> {
    let kind = if label == "dashboard spec" {
        InputKind::DashboardSpec
    } else {
        InputKind::JsonArtifact
    };
    let text = read_utf8(path, kind)?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::invalid_args(format!("parse {label} {}: {err}", path.display())))
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

fn parse_build_args(args: &[String]) -> CliResult<BuildOptions> {
    let mut options = BuildOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                options.schema = Some(PathBuf::from(take_value(args, &mut i, "--schema")?))
            }
            "--profile" => {
                options.profile = Some(PathBuf::from(take_value(args, &mut i, "--profile")?))
            }
            "--spec" => options.spec = Some(PathBuf::from(take_value(args, &mut i, "--spec")?)),
            "--out-dir" | "--out" => {
                let out_dir = PathBuf::from(take_value(args, &mut i, "--out-dir")?);
                set_mode_with_allowed_modes(
                    &mut options.mode,
                    MutationMode::OutDir,
                    "--dry-run or --out-dir <dir>",
                    "Choose exactly one build mode: --dry-run or --out-dir <dir>.",
                    "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --dry-run --json",
                )?;
                options.out_dir = Some(out_dir);
            }
            "--force" => {
                options.force = true;
                i += 1;
            }
            "--trace" => {
                if options.trace {
                    return Err(CliError::invalid_args("--trace may be specified only once"));
                }
                options.trace = true;
                i += 1;
            }
            "--dry-run" => {
                set_mode_with_allowed_modes(
                    &mut options.mode,
                    MutationMode::DryRun,
                    "--dry-run or --out-dir <dir>",
                    "Choose exactly one build mode: --dry-run or --out-dir <dir>.",
                    "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --dry-run --json",
                )?;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!("unknown report build flag: {other}"))
                    .with_suggested_command(
                        "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json",
                    ));
            }
        }
    }
    Ok(options)
}

fn parse_spec_validate_args(args: &[String]) -> CliResult<SpecValidateOptions> {
    let mut options = SpecValidateOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                options.schema = Some(PathBuf::from(take_value(args, &mut i, "--schema")?))
            }
            "--profile" => {
                options.profile = Some(PathBuf::from(take_value(args, &mut i, "--profile")?))
            }
            "--spec" => options.spec = Some(PathBuf::from(take_value(args, &mut i, "--spec")?)),
            other if other.starts_with('-') => {
                return Err(CliError::invalid_args(format!("unknown report spec validate flag: {other}"))
                    .with_suggested_command(
                        "powerbi-cli report spec validate --schema <schema.json> --spec <dashboard.json> --json",
                    ));
            }
            other => {
                if options.spec.is_some() {
                    return Err(CliError::invalid_args(
                        "report spec validate accepts exactly one spec path",
                    )
                    .with_suggested_command(
                        "powerbi-cli report spec validate --schema <schema.json> --spec <dashboard.json> --json",
                    ));
                }
                options.spec = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }
    Ok(options)
}

fn take_value(args: &[String], index: &mut usize, flag: &str) -> CliResult<String> {
    let value = args
        .get(*index + 1)
        .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
    *index += 2;
    Ok(value.clone())
}

fn copy_report_field(report: &Map<String, Value>, schema: &mut Map<String, Value>, key: &str) {
    if let Some(value) = report.get(key) {
        schema.insert(key.to_string(), value.clone());
    }
}

fn required_string(object: &Map<String, Value>, field: &str, owner: &str) -> CliResult<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::invalid_args(format!("{owner} requires {field}")))
}

pub(crate) fn page_name(value: &str) -> String {
    if value.starts_with("ReportSection") {
        value.to_string()
    } else {
        format!("ReportSection{}", slug(value))
    }
}

pub(crate) fn visual_name(value: &str) -> String {
    if value.starts_with("VisualContainer") {
        value.to_string()
    } else {
        format!("VisualContainer{}", slug(value))
    }
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut upper_next = true;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper_next {
                out.push(ch.to_ascii_uppercase());
            } else {
                out.push(ch);
            }
            upper_next = false;
        } else {
            upper_next = true;
        }
    }
    if out.is_empty() {
        "Generated".to_string()
    } else {
        out
    }
}
