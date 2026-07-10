use crate::cli_support::{
    MutationMode, require_mode_with_allowed_modes, set_mode_with_allowed_modes,
};
use crate::pbir_visual_factory::resolve_slicer_mode;
use crate::profile::{load_profile_value, profile_summary, validate_profile_value};
use crate::report_spec_fields::fields_command;
use crate::schema::{load_schema_value, merge_schema_and_spec, validate_schema_value};
use crate::visual_catalog::{canonical_visual_type, normalize_role};
use crate::{
    CliError, CliResult, EXIT_SUCCESS, EXIT_VALIDATION_FAILED, canonical_display, command_arg,
    scaffold_schema_value, validate_project,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct BuildOptions {
    schema: Option<PathBuf>,
    profile: Option<PathBuf>,
    spec: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    force: bool,
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
        CliError::invalid_args("report build requires --schema <schema.json>")
            .with_suggested_command(
                "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json",
            )
    })?;
    let schema_value = load_schema_value(&schema_path)?;
    let spec_value = load_optional_value(options.spec.as_deref(), "dashboard spec")?;
    let profile_value = load_optional_profile(options.profile.as_deref())?;
    let compiled = compile_dashboard(&schema_value, spec_value.as_ref())?;
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
        }));
    }

    let out_dir = options.out_dir.ok_or_else(|| {
        CliError::invalid_args("report build --out-dir mode requires --out-dir <project-dir>")
            .with_suggested_command(
                "powerbi-cli report build --schema <schema.json> --spec <dashboard.json> --out-dir <project-dir> --json",
            )
    })?;
    let scaffold = scaffold_schema_value(
        compiled.schema.clone(),
        &schema_path,
        &out_dir,
        options.force,
    )?;
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
    }))
}

pub(crate) fn spec_command(args: &[String]) -> CliResult<Value> {
    match args {
        [action, rest @ ..] if action == "validate" => spec_validate(rest),
        [action, rest @ ..] if action == "fields" => fields_command(rest),
        [] => Err(CliError::invalid_args("report spec requires a subcommand: validate or fields")
            .with_suggested_command(
                "powerbi-cli report spec validate --schema <schema.json> --spec <dashboard.json> --json",
            )
            .with_suggested_command(
                "powerbi-cli report spec fields --schema <schema.json> --json",
            )),
        _ => Err(CliError::invalid_args("unknown report spec command")
            .with_suggested_command("powerbi-cli --json capabilities --for \"report spec\"")),
    }
}

pub(crate) fn compile_dashboard_summary(schema: &Value, spec: &Value) -> CliResult<Value> {
    let compiled = compile_dashboard(schema, Some(spec))?;
    Ok(compiled_summary(&compiled))
}

fn spec_validate(args: &[String]) -> CliResult<Value> {
    let options = parse_spec_validate_args(args)?;
    let spec_path = options.spec.ok_or_else(|| {
        CliError::invalid_args("report spec validate requires --spec <dashboard.json>")
            .with_suggested_command(
                "powerbi-cli report spec validate --schema <schema.json> --spec <dashboard.json> --json",
            )
    })?;
    let spec_value = load_json_value(&spec_path, "dashboard spec")?;
    let profile_value = load_optional_profile(options.profile.as_deref())?;
    let (ok, validation_level, errors, warnings, compiled, schema_path) = if let Some(schema_path) =
        options.schema.as_deref()
    {
        let schema_value = load_schema_value(schema_path)?;
        match compile_dashboard(&schema_value, Some(&spec_value)) {
            Ok(compiled) => {
                let schema_validation = validate_schema_value(&compiled.schema);
                (
                    schema_validation.errors.is_empty(),
                    "compiled",
                    schema_validation.errors,
                    Vec::new(),
                    Some(compiled),
                    Some(schema_path.to_path_buf()),
                )
            }
            Err(err) => (
                false,
                "compiled",
                vec![err.message],
                Vec::new(),
                None,
                Some(schema_path.to_path_buf()),
            ),
        }
    } else {
        let errors = validate_spec_shape(&spec_value);
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
        "profileSummary": profile_value.as_ref().map(profile_summary),
        "compiled": compiled.as_ref().map(compiled_summary),
        "warnings": warnings,
        "errors": errors,
        "next": next_for_spec_validate(&spec_path, schema_path.as_deref(), ok, validation_level)
    }))
}

#[derive(Debug)]
struct CompiledDashboard {
    schema: Value,
    operations: Vec<Value>,
    warnings: Vec<Value>,
}

fn compile_dashboard(schema: &Value, spec: Option<&Value>) -> CliResult<CompiledDashboard> {
    let Some(spec) = spec else {
        let (schema, notes) = merge_schema_and_spec(schema.clone(), None)?;
        return Ok(CompiledDashboard {
            schema,
            operations: vec![
                json!({"kind": "legacySchema", "summary": "used pages embedded in schema manifest"}),
            ],
            warnings: notes
                .into_iter()
                .map(|message| json!({"code": "report_build.legacy_schema", "message": message}))
                .collect(),
        });
    };
    if spec.get("report").is_none() && spec.get("pages").is_some() {
        let (schema, notes) = merge_schema_and_spec(schema.clone(), Some(spec))?;
        return Ok(CompiledDashboard {
            schema,
            operations: vec![
                json!({"kind": "legacySpecMerge", "summary": "merged top-level dashboard fields into schema manifest"}),
            ],
            warnings: notes
                .into_iter()
                .map(|message| json!({"code": "report_build.legacy_spec", "message": message}))
                .collect(),
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
    let pages = compile_pages(spec_object, &model)?;
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
        warnings: Vec::new(),
    })
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

fn compile_pages(spec: &Map<String, Value>, model: &ModelIndex) -> CliResult<Vec<Value>> {
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
        if page.get("filters").is_some() {
            return Err(CliError::unsupported_feature(
                "report build page filters from dashboard spec are planned; add filters after build with report filters add"
            )
            .with_suggested_command(
                "powerbi-cli report filters add --project <project-dir> --target <Table[Column]> --value <value> --dry-run --json",
            ));
        }
        let visuals = compile_visuals(page_index, page, model)?;
        out.insert("visuals".to_string(), Value::Array(visuals));
        pages.push(Value::Object(out));
    }
    Ok(pages)
}

fn compile_visuals(
    page_index: usize,
    page: &Map<String, Value>,
    model: &ModelIndex,
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
        if let Some(mode) = resolve_slicer_mode(&visual_type, requested_mode)? {
            out.insert("mode".to_string(), Value::String(mode.as_str().to_string()));
        }
        if let Some(title) = visual.get("title").and_then(Value::as_str) {
            out.insert("title".to_string(), Value::String(title.to_string()));
        }
        apply_layout(visual_index, visual, &mut out);
        let bindings = compile_bindings(page_index, visual_index, &visual_type, visual, model)?;
        validate_binding_contract(page_index, visual_index, &visual_type, &bindings)?;
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
        let role = normalize_role(
            visual_type,
            &required_string(binding, "role", "visual binding")?,
        )?;
        let mut out = Map::new();
        out.insert("role".to_string(), Value::String(role));
        if let Some(field) = binding.get("field").and_then(Value::as_str) {
            let field = model.resolve_field(field)?;
            out.insert("table".to_string(), Value::String(field.table));
            match field.kind {
                FieldKind::Column => out.insert("column".to_string(), Value::String(field.name)),
                FieldKind::Measure => out.insert("measure".to_string(), Value::String(field.name)),
            };
        } else {
            let table = required_string(binding, "table", "visual binding")?;
            let column = binding.get("column").and_then(Value::as_str);
            let measure = binding.get("measure").and_then(Value::as_str);
            match (column, measure) {
                (Some(column), None) => {
                    model.resolve_structured_field(&table, column, FieldKind::Column)?;
                    out.insert("table".to_string(), Value::String(table));
                    out.insert("column".to_string(), Value::String(column.to_string()));
                }
                (None, Some(measure)) => {
                    model.resolve_structured_field(&table, measure, FieldKind::Measure)?;
                    out.insert("table".to_string(), Value::String(table));
                    out.insert("measure".to_string(), Value::String(measure.to_string()));
                }
                (Some(_), Some(_)) => {
                    return Err(CliError::invalid_args(
                        "visual binding must set either column or measure, not both",
                    ));
                }
                (None, None) => {
                    return Err(CliError::invalid_args(
                        "visual binding requires field or table plus column/measure",
                    ));
                }
            }
            for key in ["displayName", "formatString"] {
                if let Some(value) = binding.get(key) {
                    out.insert(key.to_string(), value.clone());
                }
            }
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
            if values > 1 {
                return Err(CliError::invalid_args(format!(
                    "{} card accepts at most one Values binding, got {values}",
                    visual_path()
                )));
            }
        }
        VisualBindingFamily::ValuesList => {}
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
            for role in ["Category", "Size", "Legend"] {
                if count(role) > 1 {
                    return Err(CliError::invalid_args(format!(
                        "{} scatterChart accepts at most one {role} binding",
                        visual_path()
                    )));
                }
            }
            if has_measure("Category") || has_measure("Legend") {
                return Err(CliError::invalid_args(format!(
                    "{} scatterChart Category and Legend bindings must be columns, not measures",
                    visual_path()
                )));
            }
        }
    }
    Ok(())
}

fn apply_layout(visual_index: usize, visual: &Map<String, Value>, out: &mut Map<String, Value>) {
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
    }
}

#[derive(Debug)]
struct ModelIndex {
    columns: BTreeMap<String, BTreeSet<String>>,
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
                    .filter_map(|column| column.get("name").and_then(Value::as_str))
                    .map(|name| name.to_ascii_lowercase())
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
            .is_some_and(|items| items.contains(&name_key));
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
                .is_some_and(|items| items.contains(&name_key)),
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
    let changes = vec![json!({
        "kind": "pbip.project",
        "action": "create",
        "path": project_dir.clone(),
        "before": Value::Null,
        "after": {
            "projectDir": project_dir.clone(),
            "counts": compiled["counts"].clone()
        }
    })];
    let validation = response
        .out_dir
        .and_then(|path| crate::resolve_project(path).ok())
        .and_then(|project| validate_project(&project).ok());
    json!({
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
        "changes": changes,
        "profileSummary": response.profile.map(profile_summary),
        "executedPrimitives": if response.changed { vec![json!({"command": "scaffold", "reason": "report build compiled schema/spec into scaffold-compatible manifest"})] } else { Vec::new() },
        "operations": response.compiled.operations,
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
        "next": next_for_build(response.out_dir, response.dry_run, response.schema_path, response.spec_path)
    })
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
        "tables": validation.tables
    })
}

fn next_for_build(
    out_dir: Option<&Path>,
    dry_run: bool,
    schema_path: &Path,
    spec_path: Option<&Path>,
) -> Vec<String> {
    if dry_run {
        return vec![format!(
            "powerbi-cli report build --schema {}{} --out-dir <project-dir> --json",
            command_arg(schema_path),
            spec_path
                .map(|path| format!(" --spec {}", command_arg(path)))
                .unwrap_or_default()
        )];
    }
    out_dir
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
        .unwrap_or_default()
}

fn next_for_spec_validate(
    spec_path: &Path,
    schema_path: Option<&Path>,
    ok: bool,
    validation_level: &str,
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
    commands
}

fn load_optional_value(path: Option<&Path>, label: &str) -> CliResult<Option<Value>> {
    path.map(|path| load_json_value(path, label)).transpose()
}

fn load_json_value(path: &Path, label: &str) -> CliResult<Value> {
    let text = fs::read_to_string(path).map_err(|err| {
        CliError::file_not_found(format!("read {label} {}: {err}", path.display()))
    })?;
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

fn page_name(value: &str) -> String {
    if value.starts_with("ReportSection") {
        value.to_string()
    } else {
        format!("ReportSection{}", slug(value))
    }
}

fn visual_name(value: &str) -> String {
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
