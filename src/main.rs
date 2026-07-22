#![recursion_limit = "512"]

mod bridge;
mod calculated_columns;
mod child_process;
mod cli;
mod cli_support;
mod contract;
mod dax_execute;
mod desktop;
mod desktop_session;
mod desktop_target;
mod diff;
mod doctor;
mod feature_catalog;
mod fixture;
mod handoff;
mod inspect;
mod lint;
mod live_model;
mod mcp;
mod measures;
mod microsoft;
mod model;
mod model_advanced;
mod model_dax;
mod model_live;
mod package;
mod partitions;
mod pbir;
mod pbir_bindings;
mod pbir_bookmarks;
mod pbir_filters;
mod pbir_interactions;
mod pbir_slicers;
mod pbir_themes;
mod pbir_visual_factory;
mod profile;
mod project_io;
mod rebind_plan;
mod relationship_tmdl;
mod relationships;
mod report;
mod report_bookmarks;
mod report_build;
mod report_conditional_formatting;
mod report_design;
mod report_drilldown;
mod report_drillthrough;
mod report_filter_add;
mod report_filter_clear;
mod report_filter_mutations;
mod report_filter_shapes;
mod report_filter_update;
mod report_filters;
mod report_hygiene;
mod report_interaction_mutations;
mod report_interactions;
mod report_layout;
mod report_objects;
mod report_page_mutations;
mod report_pages;
mod report_plan;
mod report_slicer_clear;
mod report_slicers;
mod report_spec_fields;
mod report_style;
mod report_themes;
mod report_visual_clone;
mod report_visual_delete;
mod report_visual_formatting;
mod report_visual_formatting_bundle;
mod report_visual_formatting_color;
mod report_visual_formatting_text;
mod report_visual_mutations;
mod report_visuals;
mod safety_scan;
mod schema;
mod skill_package;
mod source_template;
mod source_templates;
mod static_tables;
mod tmdl;
mod visual_catalog;
mod workflow;

pub(crate) use doctor::doctor_json;

use crate::pbir_bindings::{VisualBindingKind, VisualBindingResolved};
use crate::pbir_visual_factory::{VisualBuildSpec, resolve_slicer_mode, visual_container_json};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

pub(crate) const EXIT_SUCCESS: i32 = 0;
pub(crate) const EXIT_INVALID_ARGS: i32 = 2;
pub(crate) const EXIT_FILE_NOT_FOUND: i32 = 3;
pub(crate) const EXIT_VALIDATION_FAILED: i32 = 10;
pub(crate) const EXIT_PROOF_INCOMPLETE: i32 = 20;
pub(crate) const EXIT_ORACLE_UNAVAILABLE: i32 = 30;
pub(crate) const EXIT_ORACLE_FAILED: i32 = 40;
pub(crate) const EXIT_UNEXPECTED: i32 = 70;

pub(crate) const PBIP_SCHEMA: &str =
    "https://developer.microsoft.com/json-schemas/fabric/pbip/pbipProperties/1.0.0/schema.json";
pub(crate) const REPORT_DEFINITION_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json";
pub(crate) const SEMANTIC_MODEL_DEFINITION_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/semanticModel/definitionProperties/1.0.0/schema.json";
const REPORT_VERSION_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json";
const REPORT_DEFINITION_VERSION: &str = "2.0.0";
const REPORT_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/2.0.0/schema.json";
const PAGES_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/pagesMetadata/1.0.0/schema.json";
const PAGE_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.0.0/schema.json";

#[derive(Debug)]
pub(crate) struct CliError {
    pub(crate) code: &'static str,
    pub(crate) exit_code: i32,
    pub(crate) message: String,
    pub(crate) hint: Option<String>,
    pub(crate) suggested_commands: Vec<String>,
}

impl CliError {
    pub(crate) fn invalid_args(message: impl Into<String>) -> Self {
        Self::new("invalid_args", EXIT_INVALID_ARGS, message)
    }

    pub(crate) fn file_not_found(message: impl Into<String>) -> Self {
        Self::new("file_not_found", EXIT_FILE_NOT_FOUND, message)
    }

    pub(crate) fn validation_failed(message: impl Into<String>) -> Self {
        Self::new("validation_failed", EXIT_VALIDATION_FAILED, message)
    }

    pub(crate) fn unsupported_feature(message: impl Into<String>) -> Self {
        Self::new("unsupported_feature", EXIT_INVALID_ARGS, message)
    }

    pub(crate) fn unexpected(message: impl Into<String>) -> Self {
        Self::new("unexpected", EXIT_UNEXPECTED, message)
    }

    fn new(code: &'static str, exit_code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            exit_code,
            message: message.into(),
            hint: None,
            suggested_commands: Vec::new(),
        }
    }

    pub(crate) fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub(crate) fn with_suggested_command(mut self, command: impl Into<String>) -> Self {
        self.suggested_commands.push(command.into());
        self
    }
}

pub(crate) type CliResult<T> = Result<T, CliError>;

pub(crate) fn walkdir_entry(
    root: &Path,
    entry: Result<walkdir::DirEntry, walkdir::Error>,
    operation: &str,
) -> CliResult<walkdir::DirEntry> {
    entry.map_err(|err| {
        let failing_path = err.path().unwrap_or(root);
        CliError::unexpected(format!(
            "{operation} failed at {}: {err}",
            failing_path.display()
        ))
    })
}

pub(crate) fn read_dir_entry(
    directory: &Path,
    entry: std::io::Result<fs::DirEntry>,
    operation: &str,
) -> CliResult<fs::DirEntry> {
    entry.map_err(|err| {
        CliError::unexpected(format!(
            "{operation} failed while reading {}: {err}",
            directory.display()
        ))
    })
}

#[cfg(test)]
mod filesystem_error_tests {
    use super::*;

    #[test]
    fn walkdir_entry_accepts_accessible_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let raw_entry = WalkDir::new(temp.path())
            .into_iter()
            .next()
            .expect("root entry");
        let entry = walkdir_entry(temp.path(), raw_entry, "test walk").expect("walk entry");
        assert_eq!(entry.path(), temp.path());
    }

    #[test]
    fn walkdir_entry_reports_the_failing_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("missing");
        let raw_entry = WalkDir::new(&missing)
            .into_iter()
            .next()
            .expect("missing root error");
        let error =
            walkdir_entry(&missing, raw_entry, "test walk").expect_err("missing root must fail");
        assert!(error.message.contains("test walk failed at"));
        assert!(error.message.contains(&missing.display().to_string()));
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DashboardSpec {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    tables: Vec<TableSpec>,
    #[serde(default)]
    relationships: Vec<RelationshipSpec>,
    #[serde(default)]
    pages: Vec<PageSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TableSpec {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    columns: Vec<ColumnSpec>,
    #[serde(default)]
    measures: Vec<MeasureSpec>,
    #[serde(default)]
    rows: Vec<Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ColumnSpec {
    name: String,
    #[serde(default)]
    data_type: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    format_string: Option<String>,
    #[serde(default)]
    source_column: Option<String>,
    #[serde(default)]
    is_hidden: bool,
    #[serde(default)]
    is_key: bool,
    #[serde(default)]
    summarize_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeasureSpec {
    name: String,
    expression: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    format_string: Option<String>,
    #[serde(default)]
    display_folder: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelationshipSpec {
    #[serde(default)]
    name: Option<String>,
    from_table: String,
    from_column: String,
    to_table: String,
    to_column: String,
    #[serde(default)]
    cross_filtering_behavior: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageSpec {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    visuals: Vec<VisualSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisualSpec {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    visual_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    bindings: Vec<VisualBindingSpec>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisualBindingSpec {
    role: String,
    table: String,
    #[serde(default)]
    column: Option<String>,
    #[serde(default)]
    measure: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    format_string: Option<String>,
}

#[derive(Debug)]
struct ScaffoldOptions {
    schema: PathBuf,
    out_dir: PathBuf,
    force: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProject {
    pub(crate) project_dir: PathBuf,
    pub(crate) pbip_path: PathBuf,
    pub(crate) report_dir: PathBuf,
    pub(crate) semantic_model_dir: PathBuf,
}

#[derive(Debug, Default)]
pub(crate) struct ValidationReport {
    pub(crate) errors: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) json_files_checked: usize,
    pub(crate) pages: usize,
    pub(crate) visuals: usize,
    pub(crate) bound_visuals: usize,
    pub(crate) tables: usize,
    pub(crate) measures: usize,
    pub(crate) relationships: usize,
}

fn main() {
    cli::main_entry();
}

pub(crate) fn scaffold_command(args: &[String]) -> CliResult<Value> {
    let options = parse_scaffold_args(args)?;
    let schema_value = schema::load_schema_value(&options.schema)?;
    scaffold_schema_value(
        schema_value,
        &options.schema,
        &options.out_dir,
        options.force,
    )
}

pub(crate) fn scaffold_schema_value(
    schema_value: Value,
    schema_path: &Path,
    out_dir: &Path,
    force: bool,
) -> CliResult<Value> {
    let spec: DashboardSpec = serde_json::from_value(schema_value).map_err(|err| {
        CliError::invalid_args(format!("parse schema {}: {err}", schema_path.display()))
    })?;
    validate_spec(&spec)?;

    let output_has_entries = out_dir.exists() && directory_has_entries(out_dir)?;
    if output_has_entries {
        if !force {
            return Err(CliError::invalid_args(format!(
                "output directory is not empty: {}; pass --force to overwrite generated files",
                out_dir.display()
            )));
        }
        remove_previous_scaffold_artifacts(out_dir)?;
    }

    fs::create_dir_all(out_dir).map_err(|err| {
        CliError::unexpected(format!(
            "create output directory {}: {err}",
            out_dir.display()
        ))
    })?;

    write_project(&spec, out_dir)?;
    let resolved = resolve_project(out_dir)?;
    let validation = validate_project(&resolved)?;
    if !validation.errors.is_empty() {
        return Err(CliError::validation_failed(format!(
            "generated project failed validation: {}",
            validation.errors.join("; ")
        )));
    }

    Ok(json!({
        "ok": true,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "schema": canonical_display(schema_path),
        "offlineSafe": true,
        "counts": {
            "tables": validation.tables,
            "measures": validation.measures,
            "relationships": validation.relationships,
            "pages": validation.pages,
            "visuals": validation.visuals,
            "boundVisuals": validation.bound_visuals
        },
        "next": [
            format!("powerbi-cli --json inspect {}", command_arg(&resolved.project_dir)),
            format!("powerbi-cli --json validate {}", command_arg(&resolved.project_dir))
        ],
        "instructions": [
            format!("Open {} in Power BI Desktop at work, then rebind partitions from dummy #table M to corporate data sources.", command_arg(&resolved.pbip_path))
        ],
        "warnings": validation.warnings
    }))
}

fn parse_scaffold_args(args: &[String]) -> CliResult<ScaffoldOptions> {
    let mut schema: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut force = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--schema" => {
                schema =
                    Some(PathBuf::from(args.get(i + 1).ok_or_else(|| {
                        CliError::invalid_args("--schema requires a path")
                    })?));
                i += 2;
            }
            "--out-dir" | "--out" => {
                out_dir =
                    Some(PathBuf::from(args.get(i + 1).ok_or_else(|| {
                        CliError::invalid_args("--out-dir requires a path")
                    })?));
                i += 2;
            }
            "--force" => {
                force = true;
                i += 1;
            }
            other => {
                return Err(CliError::invalid_args(format!(
                    "unknown scaffold flag: {other}"
                )));
            }
        }
    }
    Ok(ScaffoldOptions {
        schema: schema.ok_or_else(|| CliError::invalid_args("scaffold requires --schema"))?,
        out_dir: out_dir.ok_or_else(|| CliError::invalid_args("scaffold requires --out-dir"))?,
        force,
    })
}

fn validate_spec(spec: &DashboardSpec) -> CliResult<()> {
    if spec.name.trim().is_empty() {
        return Err(CliError::invalid_args("schema name must not be empty"));
    }
    if spec.tables.is_empty() {
        return Err(CliError::invalid_args(
            "schema must contain at least one table",
        ));
    }
    let mut table_names = BTreeSet::new();
    for table in &spec.tables {
        if table.name.trim().is_empty() {
            return Err(CliError::invalid_args("table name must not be empty"));
        }
        if !table_names.insert(table.name.to_ascii_lowercase()) {
            return Err(CliError::invalid_args(format!(
                "duplicate table name: {}",
                table.name
            )));
        }
        if table.columns.is_empty() {
            return Err(CliError::invalid_args(format!(
                "table {} must contain at least one column",
                table.name
            )));
        }
        let mut columns = BTreeSet::new();
        for column in &table.columns {
            if column.name.trim().is_empty() {
                return Err(CliError::invalid_args(format!(
                    "table {} contains an empty column name",
                    table.name
                )));
            }
            if !columns.insert(column.name.to_ascii_lowercase()) {
                return Err(CliError::invalid_args(format!(
                    "duplicate column {} in table {}",
                    column.name, table.name
                )));
            }
            let _ = normalize_data_type(column.data_type.as_deref())?;
        }
    }

    for relationship in &spec.relationships {
        if !table_has_column(spec, &relationship.from_table, &relationship.from_column) {
            return Err(CliError::invalid_args(format!(
                "relationship references missing from column {}.{}",
                relationship.from_table, relationship.from_column
            )));
        }
        if !table_has_column(spec, &relationship.to_table, &relationship.to_column) {
            return Err(CliError::invalid_args(format!(
                "relationship references missing to column {}.{}",
                relationship.to_table, relationship.to_column
            )));
        }
    }

    for page in &spec.pages {
        for visual in &page.visuals {
            for binding in &visual.bindings {
                if binding.role.trim().is_empty() {
                    return Err(CliError::invalid_args(format!(
                        "visual {} contains a binding with an empty role",
                        visual.title.as_deref().unwrap_or("<untitled>")
                    )));
                }
                match (&binding.column, &binding.measure) {
                    (Some(column), None) => {
                        if !table_has_column(spec, &binding.table, column) {
                            return Err(CliError::invalid_args(format!(
                                "visual {} binding references missing column {}.{}",
                                visual.title.as_deref().unwrap_or("<untitled>"),
                                binding.table,
                                column
                            )));
                        }
                    }
                    (None, Some(measure)) => {
                        if !table_has_measure(spec, &binding.table, measure) {
                            return Err(CliError::invalid_args(format!(
                                "visual {} binding references missing measure {}.{}",
                                visual.title.as_deref().unwrap_or("<untitled>"),
                                binding.table,
                                measure
                            )));
                        }
                    }
                    (None, None) => {
                        return Err(CliError::invalid_args(format!(
                            "visual {} binding role {} must specify column or measure",
                            visual.title.as_deref().unwrap_or("<untitled>"),
                            binding.role
                        )));
                    }
                    (Some(_), Some(_)) => {
                        return Err(CliError::invalid_args(format!(
                            "visual {} binding role {} must not specify both column and measure",
                            visual.title.as_deref().unwrap_or("<untitled>"),
                            binding.role
                        )));
                    }
                }
            }
        }
    }

    Ok(())
}

fn table_has_column(spec: &DashboardSpec, table_name: &str, column_name: &str) -> bool {
    spec.tables.iter().any(|table| {
        table.name.eq_ignore_ascii_case(table_name)
            && table
                .columns
                .iter()
                .any(|column| column.name.eq_ignore_ascii_case(column_name))
    })
}

fn table_has_measure(spec: &DashboardSpec, table_name: &str, measure_name: &str) -> bool {
    spec.tables.iter().any(|table| {
        table.name.eq_ignore_ascii_case(table_name)
            && table
                .measures
                .iter()
                .any(|measure| measure.name.eq_ignore_ascii_case(measure_name))
    })
}

fn directory_has_entries(path: &Path) -> CliResult<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut entries = fs::read_dir(path).map_err(|err| {
        CliError::unexpected(format!("read output directory {}: {err}", path.display()))
    })?;
    Ok(entries.next().is_some())
}

fn remove_previous_scaffold_artifacts(out_dir: &Path) -> CliResult<()> {
    let manifest_path = out_dir.join("powerbi-cli.manifest.copy.json");
    if !manifest_path.is_file() {
        return Err(CliError::invalid_args(format!(
            "refusing --force cleanup in unmarked non-empty directory {}; expected prior scaffold manifest {}",
            out_dir.display(),
            manifest_path.display()
        ))
        .with_hint(
            "Choose an empty --out-dir, or restore the scaffold-generated manifest before using --force.",
        ));
    }

    let previous_value = read_json_value(&manifest_path)?;
    let previous_spec: DashboardSpec = serde_json::from_value(previous_value).map_err(|err| {
        CliError::validation_failed(format!(
            "parse prior scaffold manifest {} before --force cleanup: {err}",
            manifest_path.display()
        ))
    })?;
    let (files, mut directories) = generated_scaffold_artifacts(&previous_spec, out_dir)?;

    for file in files {
        remove_generated_file(&file)?;
    }

    directories.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| right.cmp(left))
    });
    directories.dedup();
    for directory in directories {
        remove_generated_dir_if_empty(&directory)?;
    }
    Ok(())
}

fn generated_scaffold_artifacts(
    spec: &DashboardSpec,
    out_dir: &Path,
) -> CliResult<(Vec<PathBuf>, Vec<PathBuf>)> {
    let project_name = sanitized_file_stem(&spec.name);
    let report_dir = out_dir.join(format!("{project_name}.Report"));
    let report_definition_dir = report_dir.join("definition");
    let pages_dir = report_definition_dir.join("pages");
    let semantic_model_dir = out_dir.join(format!("{project_name}.SemanticModel"));
    let semantic_definition_dir = semantic_model_dir.join("definition");
    let tables_dir = semantic_definition_dir.join("tables");

    let mut files = vec![
        out_dir.join(format!("{project_name}.pbip")),
        out_dir.join(".gitignore"),
        out_dir.join("POWERBI_HANDOFF.md"),
        out_dir.join("powerbi-cli.manifest.copy.json"),
        report_dir.join(".platform"),
        report_dir.join("definition.pbir"),
        report_definition_dir.join("version.json"),
        report_definition_dir.join("report.json"),
        pages_dir.join("pages.json"),
        semantic_model_dir.join(".platform"),
        semantic_model_dir.join("definition.pbism"),
        semantic_definition_dir.join("database.tmdl"),
        semantic_definition_dir.join("model.tmdl"),
        semantic_definition_dir.join("relationships.tmdl"),
    ];
    let mut directories = vec![
        pages_dir.clone(),
        report_definition_dir.clone(),
        report_dir,
        tables_dir.clone(),
        semantic_definition_dir.clone(),
        semantic_model_dir,
    ];

    for table in &spec.tables {
        files.push(tables_dir.join(format!("{}.tmdl", sanitized_file_stem(&table.name))));
    }

    for (page_index, page) in effective_pages(spec).iter().enumerate() {
        let page_name = match page.name.as_deref() {
            Some(name) => scaffold_object_component(name, "page name")?.to_string(),
            None => object_name(
                "ReportSection",
                page.display_name.as_deref().unwrap_or("Page"),
                page_index,
            ),
        };
        let page_dir = pages_dir.join(&page_name);
        let visuals_dir = page_dir.join("visuals");
        files.push(page_dir.join("page.json"));
        directories.push(visuals_dir.clone());
        directories.push(page_dir);

        for (visual_index, visual) in page.visuals.iter().enumerate() {
            let visual_name = match visual.name.as_deref() {
                Some(name) => scaffold_object_component(name, "visual name")?.to_string(),
                None => object_name(
                    "VisualContainer",
                    visual.title.as_deref().unwrap_or("visual"),
                    visual_index,
                ),
            };
            let visual_dir = visuals_dir.join(visual_name);
            files.push(visual_dir.join("visual.json"));
            directories.push(visual_dir);
        }
    }

    Ok((files, directories))
}

fn scaffold_object_component<'a>(value: &'a str, label: &str) -> CliResult<&'a str> {
    let mut components = Path::new(value).components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        return Ok(value);
    }
    Err(CliError::validation_failed(format!(
        "prior scaffold manifest contains unsafe {label}: {value}"
    )))
}

fn remove_generated_file(path: &Path) -> CliResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(CliError::unexpected(format!(
                "inspect generated artifact {}: {err}",
                path.display()
            )));
        }
    };
    if !metadata.file_type().is_file() {
        return Err(CliError::unexpected(format!(
            "refusing to remove non-file at generated artifact path {}",
            path.display()
        )));
    }
    make_writable_on_windows(path)?;
    fs::remove_file(path).map_err(|err| {
        CliError::unexpected(format!(
            "remove previously generated artifact {}: {err}",
            path.display()
        ))
    })
}

fn remove_generated_dir_if_empty(path: &Path) -> CliResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(CliError::unexpected(format!(
                "inspect generated directory {}: {err}",
                path.display()
            )));
        }
    };
    if !metadata.file_type().is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(path).map_err(|err| {
        CliError::unexpected(format!(
            "read generated directory {}: {err}",
            path.display()
        ))
    })?;
    if let Some(entry) = entries.next() {
        let _ = read_dir_entry(path, entry, "inspect generated directory cleanup")?;
        return Ok(());
    }
    drop(entries);
    make_writable_on_windows(path)?;
    fs::remove_dir(path).map_err(|err| {
        CliError::unexpected(format!(
            "remove empty generated directory {}: {err}",
            path.display()
        ))
    })
}

#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn make_writable_on_windows(path: &Path) -> CliResult<()> {
    let mut permissions = fs::metadata(path)
        .map_err(|err| CliError::unexpected(format!("read permissions {}: {err}", path.display())))?
        .permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions).map_err(|err| {
            CliError::unexpected(format!(
                "clear read-only attribute on generated artifact {}: {err}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn make_writable_on_windows(_path: &Path) -> CliResult<()> {
    Ok(())
}

fn write_project(spec: &DashboardSpec, out_dir: &Path) -> CliResult<()> {
    let project_name = sanitized_file_stem(&spec.name);
    let display_name = spec.display_name.as_deref().unwrap_or(&spec.name);
    let report_dir = out_dir.join(format!("{project_name}.Report"));
    let report_definition_dir = report_dir.join("definition");
    let pages_dir = report_definition_dir.join("pages");
    let semantic_model_dir = out_dir.join(format!("{project_name}.SemanticModel"));
    let semantic_definition_dir = semantic_model_dir.join("definition");
    let tables_dir = semantic_definition_dir.join("tables");

    fs::create_dir_all(&pages_dir).map_err(|err| {
        CliError::unexpected(format!(
            "create report pages dir {}: {err}",
            pages_dir.display()
        ))
    })?;
    fs::create_dir_all(&tables_dir).map_err(|err| {
        CliError::unexpected(format!("create tables dir {}: {err}", tables_dir.display()))
    })?;

    write_json_file(
        &out_dir.join(format!("{project_name}.pbip")),
        &json!({
            "$schema": PBIP_SCHEMA,
            "version": "1.0",
            "artifacts": [
                {
                    "report": {
                        "path": format!("./{project_name}.Report")
                    }
                }
            ],
            "settings": {
                "enableAutoRecovery": true
            }
        }),
    )?;

    write_json_file(
        &report_dir.join(".platform"),
        &platform_json("Report", display_name, spec.description.as_deref()),
    )?;
    write_json_file(
        &semantic_model_dir.join(".platform"),
        &platform_json("SemanticModel", display_name, spec.description.as_deref()),
    )?;
    write_json_file(
        &report_dir.join("definition.pbir"),
        &json!({
            "$schema": REPORT_DEFINITION_SCHEMA,
            "version": "4.0",
            "datasetReference": {
                "byPath": {
                    "path": format!("../{project_name}.SemanticModel")
                }
            }
        }),
    )?;
    write_json_file(
        &semantic_model_dir.join("definition.pbism"),
        &json!({
            "$schema": SEMANTIC_MODEL_DEFINITION_SCHEMA,
            "version": "4.0",
            "settings": {}
        }),
    )?;

    write_json_file(
        &report_definition_dir.join("version.json"),
        &json!({
            "$schema": REPORT_VERSION_SCHEMA,
            "version": REPORT_DEFINITION_VERSION
        }),
    )?;
    write_json_file(
        &report_definition_dir.join("report.json"),
        &json!({
            "$schema": REPORT_SCHEMA,
            "themeCollection": {},
            "annotations": [
                {
                    "name": "powerbi-cli.offlineAuthoring",
                    "value": "Generated from schema only; semantic model partitions use dummy #table M rows."
                }
            ]
        }),
    )?;

    let pages = effective_pages(spec);
    let mut page_order = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        let page_name = page.name.clone().unwrap_or_else(|| {
            object_name(
                "ReportSection",
                page.display_name.as_deref().unwrap_or("Page"),
                page_index,
            )
        });
        let page_display_name = page
            .display_name
            .clone()
            .unwrap_or_else(|| format!("Page {}", page_index + 1));
        page_order.push(page_name.clone());
        let page_dir = pages_dir.join(&page_name);
        let visuals_dir = page_dir.join("visuals");
        fs::create_dir_all(&visuals_dir).map_err(|err| {
            CliError::unexpected(format!(
                "create visuals dir {}: {err}",
                visuals_dir.display()
            ))
        })?;
        write_json_file(
            &page_dir.join("page.json"),
            &json!({
                "$schema": PAGE_SCHEMA,
                "name": page_name,
                "displayName": page_display_name,
                "displayOption": "FitToPage",
                "height": page.height.unwrap_or(720.0),
                "width": page.width.unwrap_or(1280.0),
                "annotations": [
                    {
                        "name": "powerbi-cli.layout",
                        "value": "Visual containers are intentionally unbound placeholders unless the source manifest supplies later binding metadata."
                    }
                ]
            }),
        )?;

        for (visual_index, visual) in page.visuals.iter().enumerate() {
            let visual_name = visual.name.clone().unwrap_or_else(|| {
                object_name(
                    "VisualContainer",
                    visual.title.as_deref().unwrap_or("visual"),
                    visual_index,
                )
            });
            write_json_file(
                &visuals_dir.join(&visual_name).join("visual.json"),
                &visual_json(spec, visual, visual_index)?,
            )?;
        }
    }
    write_json_file(
        &pages_dir.join("pages.json"),
        &json!({
            "$schema": PAGES_SCHEMA,
            "pageOrder": page_order,
            "activePageName": page_order.first().cloned().unwrap_or_else(|| "ReportSection".to_string())
        }),
    )?;

    write_text_file(
        &semantic_definition_dir.join("database.tmdl"),
        &database_tmdl(&spec.name),
    )?;
    write_text_file(
        &semantic_definition_dir.join("model.tmdl"),
        &model_tmdl(spec.locale.as_deref().unwrap_or("en-US")),
    )?;
    for table in &spec.tables {
        write_text_file(
            &tables_dir.join(format!("{}.tmdl", sanitized_file_stem(&table.name))),
            &table_tmdl(table)?,
        )?;
    }
    write_text_file(
        &semantic_definition_dir.join("relationships.tmdl"),
        &relationships_tmdl(spec),
    )?;

    write_text_file(&out_dir.join(".gitignore"), gitignore_text())?;
    write_text_file(
        &out_dir.join("POWERBI_HANDOFF.md"),
        &handoff_text(spec, &project_name),
    )?;
    write_json_file(
        &out_dir.join("powerbi-cli.manifest.copy.json"),
        &serde_json::to_value(spec_to_json(spec)).map_err(|err| {
            CliError::unexpected(format!(
                "serialize manifest copy for {}: {err}",
                out_dir.display()
            ))
        })?,
    )?;

    Ok(())
}

fn platform_json(kind: &str, display_name: &str, description: Option<&str>) -> Value {
    let mut metadata = Map::new();
    metadata.insert("type".to_string(), Value::String(kind.to_string()));
    metadata.insert(
        "displayName".to_string(),
        Value::String(display_name.to_string()),
    );
    if let Some(description) = description {
        metadata.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }
    json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/gitIntegration/platformProperties/2.0.0/schema.json",
        "metadata": metadata,
        "config": {
            "version": "2.0",
            "logicalId": stable_guid(&format!("{kind}:{display_name}"))
        }
    })
}

fn effective_pages(spec: &DashboardSpec) -> Vec<PageSpec> {
    if spec.pages.is_empty() {
        vec![PageSpec {
            name: Some("ReportSectionOverview".to_string()),
            display_name: Some("Overview".to_string()),
            width: Some(1280.0),
            height: Some(720.0),
            // A blank page is valid PBIR. Inventing data visuals without model bindings is not:
            // Microsoft's consumed report surface rejects them with PBIR_QUERY_STATE_MISSING.
            visuals: Vec::new(),
        }]
    } else {
        spec.pages.clone()
    }
}

impl Clone for PageSpec {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            width: self.width,
            height: self.height,
            visuals: self.visuals.clone(),
        }
    }
}

impl Clone for VisualSpec {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            visual_type: self.visual_type.clone(),
            title: self.title.clone(),
            mode: self.mode.clone(),
            bindings: self.bindings.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

impl Clone for VisualBindingSpec {
    fn clone(&self) -> Self {
        Self {
            role: self.role.clone(),
            table: self.table.clone(),
            column: self.column.clone(),
            measure: self.measure.clone(),
            display_name: self.display_name.clone(),
            format_string: self.format_string.clone(),
        }
    }
}

fn visual_json(
    dashboard: &DashboardSpec,
    visual: &VisualSpec,
    visual_index: usize,
) -> CliResult<Value> {
    let title = visual
        .title
        .clone()
        .unwrap_or_else(|| format!("Visual {}", visual_index + 1));
    let visual_type = visual
        .visual_type
        .clone()
        .unwrap_or_else(|| "card".to_string());
    let bindings = visual
        .bindings
        .iter()
        .map(|binding| scaffold_visual_binding(dashboard, binding))
        .collect::<Vec<_>>();
    report_visual_mutations::validate_binding_cardinality(&visual_type, &bindings)?;
    let slicer_mode = resolve_slicer_mode(&visual_type, visual.mode.as_deref())?;
    visual_container_json(&VisualBuildSpec {
        name: visual
            .name
            .clone()
            .unwrap_or_else(|| object_name("VisualContainer", &title, visual_index)),
        title,
        visual_type,
        bindings,
        slicer_mode,
        x: visual.x.unwrap_or(40.0 + (visual_index as f64 * 40.0)),
        y: visual.y.unwrap_or(40.0 + (visual_index as f64 * 40.0)),
        z: visual_index as u64,
        height: visual.height.unwrap_or(180.0),
        width: visual.width.unwrap_or(320.0),
        tab_order: visual_index as u64,
    })
}

fn scaffold_visual_binding(
    dashboard: &DashboardSpec,
    binding: &VisualBindingSpec,
) -> VisualBindingResolved {
    if let Some(measure) = &binding.measure {
        VisualBindingResolved {
            role: binding.role.clone(),
            table: binding.table.clone(),
            field: measure.clone(),
            kind: VisualBindingKind::Measure,
            data_type: None,
            display_name: binding.display_name.clone(),
            format_string: binding.format_string.clone(),
        }
    } else if let Some(column) = &binding.column {
        VisualBindingResolved {
            role: binding.role.clone(),
            table: binding.table.clone(),
            field: column.clone(),
            kind: VisualBindingKind::Column,
            data_type: dashboard
                .tables
                .iter()
                .find(|table| table.name.eq_ignore_ascii_case(&binding.table))
                .and_then(|table| {
                    table
                        .columns
                        .iter()
                        .find(|candidate| candidate.name.eq_ignore_ascii_case(column))
                })
                .and_then(|column| normalize_data_type(column.data_type.as_deref()).ok())
                .map(|data_type| data_type.tmdl.to_string()),
            display_name: binding.display_name.clone(),
            format_string: binding.format_string.clone(),
        }
    } else {
        VisualBindingResolved {
            role: binding.role.clone(),
            table: binding.table.clone(),
            field: "<invalid>".to_string(),
            kind: VisualBindingKind::Column,
            data_type: None,
            display_name: binding.display_name.clone(),
            format_string: binding.format_string.clone(),
        }
    }
}

fn database_tmdl(name: &str) -> String {
    format!(
        "database {}\n    compatibilityLevel: 1567\n\n",
        tmdl_object_name(name)
    )
}

fn model_tmdl(locale: &str) -> String {
    format!(
        "model Model\n    culture: {locale}\n    defaultPowerBIDataSourceVersion: powerBI_V3\n    sourceQueryCulture: {locale}\n    discourageImplicitMeasures\n\n"
    )
}

fn table_tmdl(table: &TableSpec) -> CliResult<String> {
    let mut out = String::new();
    out.push_str(&format!("table {}\n", tmdl_object_name(&table.name)));
    out.push_str(&format!(
        "    lineageTag: {}\n",
        stable_guid(&format!("table:{}", table.name))
    ));
    out.push('\n');

    for column in &table.columns {
        let data_type = normalize_data_type(column.data_type.as_deref())?;
        push_tmdl_description(&mut out, "    ", column.description.as_deref());
        out.push_str(&format!("    column {}\n", tmdl_object_name(&column.name)));
        out.push_str(&format!("        dataType: {}\n", data_type.tmdl));
        out.push_str(&format!(
            "        lineageTag: {}\n",
            stable_guid(&format!("column:{}:{}", table.name, column.name))
        ));
        out.push_str(&format!(
            "        summarizeBy: {}\n",
            column
                .summarize_by
                .as_deref()
                .unwrap_or_else(|| default_summarize_by(column, data_type))
        ));
        out.push_str(&format!(
            "        sourceColumn: {}\n",
            tmdl_object_name(column.source_column.as_deref().unwrap_or(&column.name))
        ));
        if column.is_hidden {
            out.push_str("        isHidden\n");
        }
        if column.is_key {
            out.push_str("        isKey\n");
        }
        if let Some(format_string) = column
            .format_string
            .as_deref()
            .or(data_type.default_format_string)
        {
            out.push_str(&format!(
                "        formatString: {}\n",
                tmdl_string_literal(format_string)
            ));
        }
        out.push('\n');
    }

    for measure in &table.measures {
        let definition = tmdl::MeasureDefinition {
            name: measure.name.clone(),
            expression: measure.expression.clone(),
            lineage_tag: Some(stable_guid(&format!(
                "measure:{}:{}",
                table.name, measure.name
            ))),
            format_string: measure.format_string.clone(),
            display_folder: measure.display_folder.clone(),
            description: measure.description.clone(),
        };
        for line in tmdl::measure_block_lines(&table.name, &definition) {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str(&format!(
        "    partition {} = m\n",
        tmdl_object_name(&table.name)
    ));
    out.push_str("        mode: import\n");
    out.push_str("        source =\n");
    for line in m_dummy_table(table)?.lines() {
        out.push_str("            ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');

    Ok(out)
}

fn default_summarize_by(column: &ColumnSpec, data_type: NormalizedDataType) -> &'static str {
    let lower_name = column.name.to_ascii_lowercase();
    if column.is_key
        || lower_name.ends_with("key")
        || lower_name.ends_with("id")
        || matches!(data_type.tmdl, "string" | "boolean" | "dateTime")
    {
        "none"
    } else {
        "sum"
    }
}

fn relationships_tmdl(spec: &DashboardSpec) -> String {
    let mut out = String::new();
    for (index, relationship) in spec.relationships.iter().enumerate() {
        let name = relationship.name.clone().unwrap_or_else(|| {
            format!(
                "{}_{}_to_{}_{}",
                relationship.from_table,
                relationship.from_column,
                relationship.to_table,
                relationship.to_column
            )
        });
        out.push_str(&format!(
            "relationship {}\n",
            tmdl_object_name(&object_name("rel", &name, index))
        ));
        out.push_str(&format!(
            "    fromColumn: {}.{}\n",
            tmdl_object_ref(&relationship.from_table),
            tmdl_object_ref(&relationship.from_column)
        ));
        out.push_str(&format!(
            "    toColumn: {}.{}\n",
            tmdl_object_ref(&relationship.to_table),
            tmdl_object_ref(&relationship.to_column)
        ));
        out.push_str(&format!(
            "    crossFilteringBehavior: {}\n",
            relationship
                .cross_filtering_behavior
                .as_deref()
                .unwrap_or("oneDirection")
        ));
        if relationship.is_active == Some(false) {
            out.push_str("    isActive: false\n");
        }
        out.push('\n');
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct NormalizedDataType {
    tmdl: &'static str,
    m: &'static str,
    default_format_string: Option<&'static str>,
}

fn normalize_data_type(value: Option<&str>) -> CliResult<NormalizedDataType> {
    let normalized = value.unwrap_or("string").trim().to_ascii_lowercase();
    match normalized.as_str() {
        "" | "text" | "string" => Ok(NormalizedDataType {
            tmdl: "string",
            m: "text",
            default_format_string: None,
        }),
        "int" | "integer" | "whole" | "whole_number" | "int64" => Ok(NormalizedDataType {
            tmdl: "int64",
            m: "number",
            default_format_string: None,
        }),
        "double" | "float" | "number" => Ok(NormalizedDataType {
            tmdl: "double",
            m: "number",
            default_format_string: None,
        }),
        "decimal" | "fixed_decimal" | "currency" => Ok(NormalizedDataType {
            tmdl: "decimal",
            m: "number",
            default_format_string: None,
        }),
        "date" => Ok(NormalizedDataType {
            tmdl: "dateTime",
            m: "date",
            default_format_string: Some("Short Date"),
        }),
        "datetime" | "date_time" | "dateTime" => Ok(NormalizedDataType {
            tmdl: "dateTime",
            m: "datetime",
            default_format_string: None,
        }),
        "bool" | "boolean" | "logical" => Ok(NormalizedDataType {
            tmdl: "boolean",
            m: "logical",
            default_format_string: None,
        }),
        other => Err(CliError::unsupported_feature(format!(
            "unsupported column dataType: {other}"
        ))),
    }
}

fn m_dummy_table(table: &TableSpec) -> CliResult<String> {
    let mut type_columns = Vec::new();
    for column in &table.columns {
        let data_type = normalize_data_type(column.data_type.as_deref())?;
        type_columns.push(format!("{} = {}", m_identifier(&column.name), data_type.m));
    }
    let rows = if table.rows.is_empty() {
        vec![dummy_row(table)?]
    } else {
        table
            .rows
            .iter()
            .map(|row| {
                table
                    .columns
                    .iter()
                    .map(|column| {
                        row.get(&column.name)
                            .map(|value| m_literal_for_column(value, column))
                            .unwrap_or_else(|| "null".to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };

    let mut out = String::new();
    out.push_str("let\n");
    out.push_str("    Source = #table(\n");
    out.push_str(&format!(
        "        type table [{}],\n",
        type_columns.join(", ")
    ));
    out.push_str("        {\n");
    for (index, row) in rows.iter().enumerate() {
        let suffix = if index + 1 == rows.len() { "" } else { "," };
        out.push_str(&format!("            {{{}}}{suffix}\n", row.join(", ")));
    }
    out.push_str("        }\n");
    out.push_str("    )\n");
    out.push_str("in\n");
    out.push_str("    Source");
    Ok(out)
}

fn dummy_row(table: &TableSpec) -> CliResult<Vec<String>> {
    table
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let data_type = normalize_data_type(column.data_type.as_deref())?;
            Ok(match data_type.tmdl {
                "int64" => (index + 1).to_string(),
                "double" => format!("{}.25", index + 1),
                "decimal" => format!("{}.99", index + 1),
                "dateTime" => {
                    if column
                        .data_type
                        .as_deref()
                        .unwrap_or_default()
                        .eq_ignore_ascii_case("date")
                    {
                        "#date(2026, 1, 1)".to_string()
                    } else {
                        "#datetime(2026, 1, 1, 0, 0, 0)".to_string()
                    }
                }
                "boolean" => "true".to_string(),
                _ => format!("\"Sample {}\"", m_escape_string(&column.name)),
            })
        })
        .collect()
}

fn m_literal_for_column(value: &Value, column: &ColumnSpec) -> String {
    if value.is_null() {
        return "null".to_string();
    }
    let data_type =
        normalize_data_type(column.data_type.as_deref()).unwrap_or(NormalizedDataType {
            tmdl: "string",
            m: "type text",
            default_format_string: None,
        });
    match (data_type.tmdl, value) {
        ("int64" | "double" | "decimal", Value::Number(number)) => number.to_string(),
        ("boolean", Value::Bool(value)) => value.to_string(),
        ("dateTime", Value::String(text)) => {
            if column
                .data_type
                .as_deref()
                .unwrap_or_default()
                .eq_ignore_ascii_case("date")
            {
                m_date_literal(text).unwrap_or_else(|| format!("\"{}\"", m_escape_string(text)))
            } else {
                m_datetime_literal(text).unwrap_or_else(|| format!("\"{}\"", m_escape_string(text)))
            }
        }
        (_, Value::String(text)) => format!("\"{}\"", m_escape_string(text)),
        (_, other) => format!("\"{}\"", m_escape_string(&other.to_string())),
    }
}

fn m_date_literal(text: &str) -> Option<String> {
    let parts = text.split('-').collect::<Vec<_>>();
    if parts.len() == 3 {
        let year = parts[0].parse::<i32>().ok()?;
        let month = parts[1].parse::<u32>().ok()?;
        let day = parts[2].parse::<u32>().ok()?;
        return Some(format!("#date({year}, {month}, {day})"));
    }
    None
}

fn m_datetime_literal(text: &str) -> Option<String> {
    let normalized = text.trim_end_matches('Z').replace('T', " ");
    if !normalized.contains(' ') {
        let date_parts = normalized.split('-').collect::<Vec<_>>();
        if date_parts.len() == 3 {
            let year = date_parts[0].parse::<i32>().ok()?;
            let month = date_parts[1].parse::<u32>().ok()?;
            let day = date_parts[2].parse::<u32>().ok()?;
            return Some(format!("#datetime({year}, {month}, {day}, 0, 0, 0)"));
        }
        return None;
    }
    let (date, time) = normalized.split_once(' ')?;
    let date_parts = date.split('-').collect::<Vec<_>>();
    let time_parts = time.split(':').collect::<Vec<_>>();
    if date_parts.len() == 3 && time_parts.len() >= 2 {
        let year = date_parts[0].parse::<i32>().ok()?;
        let month = date_parts[1].parse::<u32>().ok()?;
        let day = date_parts[2].parse::<u32>().ok()?;
        let hour = time_parts[0].parse::<u32>().ok()?;
        let minute = time_parts[1].parse::<u32>().ok()?;
        let second = time_parts
            .get(2)
            .and_then(|part| part.split('.').next())
            .and_then(|part| part.parse::<u32>().ok())
            .unwrap_or(0);
        return Some(format!(
            "#datetime({year}, {month}, {day}, {hour}, {minute}, {second})"
        ));
    }
    None
}

#[cfg(test)]
mod m_literal_tests {
    use super::*;

    #[test]
    fn datetime_literal_accepts_a_date_only_iso_value() {
        assert_eq!(
            m_datetime_literal("2015-01-23").as_deref(),
            Some("#datetime(2015, 1, 23, 0, 0, 0)")
        );
    }

    #[test]
    fn datetime_literal_preserves_an_iso_timestamp() {
        assert_eq!(
            m_datetime_literal("2015-01-23T14:05:09Z").as_deref(),
            Some("#datetime(2015, 1, 23, 14, 5, 9)")
        );
    }
}

fn m_identifier(name: &str) -> String {
    if is_simple_identifier(name) {
        name.to_string()
    } else {
        format!("#\"{}\"", name.replace('"', "\"\""))
    }
}

fn m_escape_string(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn tmdl_object_name(name: &str) -> String {
    if is_simple_identifier(name) {
        name.to_string()
    } else {
        tmdl_object_ref(name)
    }
}

fn tmdl_object_ref(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

fn tmdl_string_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn push_tmdl_description(out: &mut String, indent: &str, description: Option<&str>) {
    let Some(description) = description else {
        return;
    };
    for line in description
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
    {
        if line.is_empty() {
            out.push_str(&format!("{indent}///\n"));
        } else {
            out.push_str(&format!("{indent}/// {line}\n"));
        }
    }
}

fn is_simple_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn gitignore_text() -> &'static str {
    r#"# Power BI Desktop local/cache files. Do not move data caches or credentials home.
*.pbix
*.pbit
*.abf
*.log
*.tmp
**/.pbi/
**/localSettings.json
**/cache.abf
"#
}

fn handoff_text(spec: &DashboardSpec, project_name: &str) -> String {
    let mut text = String::new();
    text.push_str(&format!("# {} Power BI Handoff\n\n", spec.name));
    text.push_str(
        "This project was generated for offline authoring from schema/dummy data only.\n\n",
    );
    text.push_str("## At Home\n\n");
    text.push_str("- Keep real data, credentials, gateway names, and exported cache files out of this folder.\n");
    text.push_str("- Edit report layout and semantic model metadata in the PBIP folder.\n");
    text.push_str(
        "- Run `powerbi-cli --json validate <project-dir>` before moving the folder.\n\n",
    );
    text.push_str("## At Work\n\n");
    text.push_str(&format!(
        "1. Open `{project_name}.pbip` in Power BI Desktop.\n"
    ));
    text.push_str("2. In Power Query or TMDL, replace each generated `#table(...)` partition source with the real corporate source.\n");
    text.push_str("3. Configure credentials in Desktop inside the corporate environment.\n");
    text.push_str("4. Refresh, check relationships/measures, then save as PBIP or PBIX according to your workplace process.\n\n");
    text.push_str("## Tables To Rebind\n\n");
    for table in &spec.tables {
        text.push_str(&format!("- `{}`\n", table.name));
    }
    text
}

fn spec_to_json(spec: &DashboardSpec) -> Value {
    let tables = spec
        .tables
        .iter()
        .map(|table| {
            json!({
                "name": table.name,
                "description": table.description,
                "columns": table.columns.iter().map(|column| json!({
                    "name": column.name,
                    "dataType": column.data_type,
                    "description": column.description,
                    "formatString": column.format_string,
                    "sourceColumn": column.source_column,
                    "isHidden": column.is_hidden,
                    "isKey": column.is_key,
                    "summarizeBy": column.summarize_by
                })).collect::<Vec<_>>(),
                "measures": table.measures.iter().map(|measure| json!({
                    "name": measure.name,
                    "expression": measure.expression,
                    "description": measure.description,
                    "formatString": measure.format_string,
                    "displayFolder": measure.display_folder
                })).collect::<Vec<_>>(),
                "rows": table.rows
            })
        })
        .collect::<Vec<_>>();
    json!({
        "name": spec.name,
        "displayName": spec.display_name,
        "description": spec.description,
        "locale": spec.locale,
        "tables": tables,
        "relationships": spec.relationships.iter().map(|relationship| json!({
            "name": relationship.name,
            "fromTable": relationship.from_table,
            "fromColumn": relationship.from_column,
            "toTable": relationship.to_table,
            "toColumn": relationship.to_column,
            "crossFilteringBehavior": relationship.cross_filtering_behavior,
            "isActive": relationship.is_active
        })).collect::<Vec<_>>(),
        "pages": spec.pages.iter().map(|page| json!({
            "name": page.name,
            "displayName": page.display_name,
            "width": page.width,
            "height": page.height,
                "visuals": page.visuals.iter().map(|visual| json!({
                    "name": visual.name,
                    "visualType": visual.visual_type,
                    "title": visual.title,
                    "mode": visual.mode,
                    "bindings": visual.bindings.iter().map(|binding| json!({
                        "role": binding.role,
                        "table": binding.table,
                        "column": binding.column,
                        "measure": binding.measure,
                        "displayName": binding.display_name,
                        "formatString": binding.format_string
                    })).collect::<Vec<_>>(),
                    "x": visual.x,
                    "y": visual.y,
                    "width": visual.width,
                    "height": visual.height
                })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

fn write_json_file(path: &Path, value: &Value) -> CliResult<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
        CliError::unexpected(format!("serialize JSON for {}: {err}", path.display()))
    })?;
    write_bytes(path, &bytes)
}

fn write_text_file(path: &Path, text: &str) -> CliResult<()> {
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            CliError::unexpected(format!("create parent dir {}: {err}", parent.display()))
        })?;
    }
    let tmp = path.with_extension(format!(
        "{}tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| format!("{value}."))
            .unwrap_or_default()
    ));
    {
        let mut file = fs::File::create(&tmp)
            .map_err(|err| CliError::unexpected(format!("create {}: {err}", tmp.display())))?;
        file.write_all(bytes)
            .map_err(|err| CliError::unexpected(format!("write {}: {err}", tmp.display())))?;
    }
    if path.exists() {
        fs::remove_file(path)
            .map_err(|err| CliError::unexpected(format!("replace {}: {err}", path.display())))?;
    }
    fs::rename(&tmp, path).map_err(|err| {
        CliError::unexpected(format!(
            "replace {} with {}: {err}",
            path.display(),
            tmp.display()
        ))
    })
}

pub(crate) fn inspect_command(args: &[String]) -> CliResult<Value> {
    let (path, deep) = parse_inspect_args(args)?;
    let resolved = resolve_project(&path)?;
    let report = validate_project(&resolved)?;
    let mut output = json!({
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "valid": report.errors.is_empty(),
        "counts": {
            "jsonFilesChecked": report.json_files_checked,
            "pages": report.pages,
            "visuals": report.visuals,
            "boundVisuals": report.bound_visuals,
            "tables": report.tables,
            "measures": report.measures,
            "relationships": report.relationships
        },
        "warnings": report.warnings,
        "errors": report.errors
    });
    if deep {
        output["deep"] = inspect::deep_inspect(&resolved, &report)?;
    }
    Ok(output)
}

fn parse_inspect_args(args: &[String]) -> CliResult<(PathBuf, bool)> {
    let mut path = None;
    let mut deep = false;
    for arg in args {
        match arg.as_str() {
            "--deep" => deep = true,
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown inspect flag: {other}"))
                        .with_hint("Run `powerbi-cli inspect --deep <project-dir-or.pbip> --json`.")
                        .with_suggested_command(
                            "powerbi-cli inspect --deep <project-dir-or.pbip> --json",
                        ),
                );
            }
            other => {
                if path.is_some() {
                    return Err(CliError::invalid_args("inspect accepts exactly one path")
                        .with_hint("Run `powerbi-cli inspect <project-dir-or.pbip> --json`.")
                        .with_suggested_command(
                            "powerbi-cli inspect <project-dir-or.pbip> --json",
                        ));
                }
                path = Some(PathBuf::from(other));
            }
        }
    }
    path.map(|path| (path, deep)).ok_or_else(|| {
        CliError::invalid_args("inspect requires a path")
            .with_hint("Run `powerbi-cli inspect <project-dir-or.pbip> --json`.")
            .with_suggested_command("powerbi-cli inspect <project-dir-or.pbip> --json")
    })
}

pub(crate) fn validate_command(args: &[String]) -> CliResult<Value> {
    let options = parse_validate_args(args)?;
    if options.strict && options.backend == ValidationBackend::MicrosoftReport {
        return Err(CliError::invalid_args(
            "--strict is a native lint option and cannot be used with --backend microsoft-report",
        )
        .with_hint(
            "Use --backend all to run strict native lint and the official validator together.",
        )
        .with_suggested_command(
            "powerbi-cli validate <project-dir-or.pbip> --strict --backend all --json",
        ));
    }
    let resolved = resolve_project(&options.path)?;
    match options.backend {
        ValidationBackend::Native => native_validation_output(&resolved, options.strict),
        ValidationBackend::MicrosoftReport => {
            let official = microsoft::validate_official_report(&resolved)?;
            let ok = official["ok"].as_bool().unwrap_or(false);
            Ok(json!({
                "schema": "powerbi-cli.validate.microsoft-report.v1",
                "ok": ok,
                "exitCode": if ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
                "strict": false,
                "backend": "microsoft-report",
                "projectDir": canonical_display(&resolved.project_dir),
                "pbip": canonical_display(&resolved.pbip_path),
                "reportDir": canonical_display(&resolved.report_dir),
                "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
                "counts": official["counts"],
                "warnings": official["warnings"],
                "errors": official["errors"],
                "validators": {
                    "microsoftReport": official
                }
            }))
        }
        ValidationBackend::All => {
            let mut output = native_validation_output(&resolved, options.strict)?;
            let native_ok = output["ok"].as_bool().unwrap_or(false);
            let official = microsoft::validate_official_report(&resolved)?;
            let official_ok = official["ok"].as_bool().unwrap_or(false);
            let overall_ok = native_ok && official_ok;
            output["ok"] = Value::Bool(overall_ok);
            output["exitCode"] = Value::from(if overall_ok {
                EXIT_SUCCESS
            } else {
                EXIT_VALIDATION_FAILED
            });
            output["backend"] = Value::String("all".to_string());
            output["schema"] = Value::String("powerbi-cli.validate.all.v1".to_string());
            output["validators"] = json!({
                "native": {
                    "id": "native",
                    "ok": native_ok,
                    "strict": options.strict
                },
                "microsoftReport": official
            });
            Ok(output)
        }
    }
}

fn native_validation_output(resolved: &ResolvedProject, strict: bool) -> CliResult<Value> {
    let report = validate_project(resolved)?;
    let ok = report.errors.is_empty();
    let lint = if strict && ok {
        Some(lint::lint_project(resolved, &report)?)
    } else {
        None
    };
    let lint_ok = lint
        .as_ref()
        .and_then(|value| value["ok"].as_bool())
        .unwrap_or(true);
    let overall_ok = ok && lint_ok;
    let mut output = json!({
        "ok": overall_ok,
        "exitCode": if overall_ok { EXIT_SUCCESS } else { EXIT_VALIDATION_FAILED },
        "strict": strict,
        "projectDir": canonical_display(&resolved.project_dir),
        "pbip": canonical_display(&resolved.pbip_path),
        "reportDir": canonical_display(&resolved.report_dir),
        "semanticModelDir": canonical_display(&resolved.semantic_model_dir),
        "counts": {
            "jsonFilesChecked": report.json_files_checked,
            "pages": report.pages,
            "visuals": report.visuals,
            "boundVisuals": report.bound_visuals,
            "tables": report.tables,
            "measures": report.measures,
            "relationships": report.relationships
        },
        "warnings": report.warnings,
        "errors": report.errors
    });
    if let Some(lint) = lint {
        output["lint"] = lint;
    }
    output["backend"] = Value::String("native".to_string());
    output["validators"] = json!({
        "native": {
            "id": "native",
            "ok": overall_ok,
            "strict": strict
        }
    });
    Ok(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationBackend {
    Native,
    MicrosoftReport,
    All,
}

impl ValidationBackend {
    fn parse(value: &str) -> CliResult<Self> {
        match value {
            "native" => Ok(Self::Native),
            "microsoft-report" => Ok(Self::MicrosoftReport),
            "all" => Ok(Self::All),
            _ => Err(
                CliError::invalid_args(format!("unknown validation backend: {value}"))
                    .with_hint("Use native, microsoft-report, or all.")
                    .with_suggested_command(
                        "powerbi-cli validate <project-dir-or.pbip> --backend all --json",
                    ),
            ),
        }
    }
}

#[derive(Debug)]
struct ValidateOptions {
    path: PathBuf,
    strict: bool,
    backend: ValidationBackend,
}

fn parse_validate_args(args: &[String]) -> CliResult<ValidateOptions> {
    let mut path = None;
    let mut strict = false;
    let mut backend = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--strict" => {
                if strict {
                    return Err(CliError::invalid_args(
                        "--strict may be specified only once",
                    ));
                }
                strict = true;
                index += 1;
            }
            "--backend" => {
                if backend.is_some() {
                    return Err(CliError::invalid_args(
                        "--backend may be specified only once",
                    ));
                }
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| CliError::invalid_args("--backend requires a value"))?;
                backend = Some(ValidationBackend::parse(value)?);
                index += 2;
            }
            other if other.starts_with('-') => {
                return Err(
                    CliError::invalid_args(format!("unknown validate flag: {other}"))
                        .with_hint(
                            "Run `powerbi-cli validate --strict <project-dir-or.pbip> --json`.",
                        )
                        .with_suggested_command(
                            "powerbi-cli validate --strict <project-dir-or.pbip> --json",
                        ),
                );
            }
            other => {
                if path.is_some() {
                    return Err(CliError::invalid_args("validate accepts exactly one path")
                        .with_hint("Run `powerbi-cli validate <project-dir-or.pbip> --json`.")
                        .with_suggested_command(
                            "powerbi-cli validate <project-dir-or.pbip> --json",
                        ));
                }
                path = Some(PathBuf::from(other));
                index += 1;
            }
        }
    }
    path.map(|path| ValidateOptions {
        path,
        strict,
        backend: backend.unwrap_or(ValidationBackend::Native),
    })
    .ok_or_else(|| {
        CliError::invalid_args("validate requires a path")
            .with_hint("Run `powerbi-cli validate <project-dir-or.pbip> --json`.")
            .with_suggested_command("powerbi-cli validate <project-dir-or.pbip> --json")
    })
}

pub(crate) fn resolve_project(path: &Path) -> CliResult<ResolvedProject> {
    if path.extension().and_then(|value| value.to_str()) == Some("pbip") {
        // `Path::parent()` is an empty path for a root-level relative filename
        // such as `Sales.pbip`. Treat that spelling as the current directory so
        // every command accepts the same convenient project reference.
        let project_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        return resolve_project_from_pbip(&project_dir, path);
    }

    if !path.exists() {
        return Err(CliError::file_not_found(format!(
            "project path does not exist: {}",
            path.display()
        )));
    }
    if !path.is_dir() {
        return Err(CliError::invalid_args(format!(
            "project path must be a directory or .pbip file: {}",
            path.display()
        )));
    }
    let pbips = fs::read_dir(path)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?
        .map(|entry| read_dir_entry(path, entry, "resolve project directory"))
        .collect::<CliResult<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|entry| entry.extension().and_then(|value| value.to_str()) == Some("pbip"))
        .collect::<Vec<_>>();
    match pbips.as_slice() {
        [pbip] => resolve_project_from_pbip(path, pbip),
        [] => Err(CliError::file_not_found(format!(
            "no .pbip file found in {}",
            path.display()
        ))),
        _ => Err(CliError::invalid_args(format!(
            "multiple .pbip files found in {}; pass the intended .pbip path",
            path.display()
        ))),
    }
}

fn resolve_project_from_pbip(project_dir: &Path, pbip_path: &Path) -> CliResult<ResolvedProject> {
    if !pbip_path.exists() {
        return Err(CliError::file_not_found(format!(
            "pbip file does not exist: {}",
            pbip_path.display()
        )));
    }
    let pbip = read_json_value(pbip_path)?;
    let report_rel = pbip["artifacts"]
        .as_array()
        .and_then(|artifacts| artifacts.first())
        .and_then(|artifact| artifact["report"]["path"].as_str())
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} does not contain artifacts[0].report.path",
                pbip_path.display()
            ))
        })?;
    let report_dir =
        resolve_project_reference(project_dir, project_dir, report_rel, "PBIP report artifact")?;
    let pbir = read_json_value(&report_dir.join("definition.pbir"))?;
    let semantic_rel = pbir["datasetReference"]["byPath"]["path"]
        .as_str()
        .ok_or_else(|| {
            CliError::validation_failed(format!(
                "{} does not contain datasetReference.byPath.path",
                report_dir.join("definition.pbir").display()
            ))
        })?;
    let semantic_model_dir = resolve_project_reference(
        project_dir,
        &report_dir,
        semantic_rel,
        "PBIR semantic-model artifact",
    )?;
    Ok(ResolvedProject {
        project_dir: project_dir.to_path_buf(),
        pbip_path: pbip_path.to_path_buf(),
        report_dir,
        semantic_model_dir,
    })
}

fn clean_relative_path(value: &str) -> CliResult<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CliError::validation_failed(
            "project reference path must not be empty",
        ));
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') || trimmed.contains(':') {
        return Err(CliError::validation_failed(format!(
            "project reference must be relative, got {value}"
        )));
    }
    let mut result = PathBuf::new();
    for part in trimmed.split('/') {
        match part {
            "" | "." => {}
            component => result.push(component),
        }
    }
    Ok(result)
}

fn resolve_project_reference(
    project_root: &Path,
    base: &Path,
    value: &str,
    label: &str,
) -> CliResult<PathBuf> {
    let relative = clean_relative_path(value)?;
    let canonical_root = fs::canonicalize(project_root).map_err(|err| {
        CliError::file_not_found(format!(
            "resolve project root {}: {err}",
            project_root.display()
        ))
    })?;
    let canonical_base = fs::canonicalize(base).map_err(|err| {
        CliError::file_not_found(format!("resolve {label} base {}: {err}", base.display()))
    })?;
    if !canonical_base.starts_with(&canonical_root) {
        return Err(project_reference_escape(label, value));
    }

    let mut target = canonical_base;
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(component) => target.push(component),
            std::path::Component::ParentDir => {
                if !target.pop() || !target.starts_with(&canonical_root) {
                    return Err(project_reference_escape(label, value));
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(project_reference_escape(label, value));
            }
        }
        if !target.starts_with(&canonical_root) {
            return Err(project_reference_escape(label, value));
        }
    }

    // Existing links must not redirect the selected artifact closure outside
    // the PBIP project. For a missing final artifact, check its nearest
    // existing ancestor so required-file validation can still report the
    // missing in-project path as a normal structured failure.
    let mut existing_ancestor = target.as_path();
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor
            .parent()
            .ok_or_else(|| project_reference_escape(label, value))?;
    }
    let canonical_ancestor = fs::canonicalize(existing_ancestor).map_err(|err| {
        CliError::file_not_found(format!(
            "resolve {label} ancestor {}: {err}",
            existing_ancestor.display()
        ))
    })?;
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(project_reference_escape(label, value));
    }

    if target.exists() {
        let canonical_target = fs::canonicalize(&target).map_err(|err| {
            CliError::file_not_found(format!("resolve {label} {}: {err}", target.display()))
        })?;
        if !canonical_target.starts_with(&canonical_root) {
            return Err(project_reference_escape(label, value));
        }
        return Ok(canonical_target);
    }
    Ok(target)
}

fn project_reference_escape(label: &str, value: &str) -> CliError {
    CliError::validation_failed(format!(
        "{label} reference escapes the selected PBIP project: {value}"
    ))
    .with_hint(
        "Keep report and semantic-model artifacts inside the selected PBIP project directory.",
    )
}

pub(crate) fn validate_project(resolved: &ResolvedProject) -> CliResult<ValidationReport> {
    validate_project_with_runtime_policy(resolved, false)
}

pub(crate) fn validate_desktop_runtime_project(
    resolved: &ResolvedProject,
) -> CliResult<ValidationReport> {
    validate_project_with_runtime_policy(resolved, true)
}

fn validate_project_with_runtime_policy(
    resolved: &ResolvedProject,
    allow_desktop_runtime_files: bool,
) -> CliResult<ValidationReport> {
    let mut report = ValidationReport::default();
    required_file(&resolved.pbip_path, &mut report);
    required_file(&resolved.report_dir.join("definition.pbir"), &mut report);
    required_file(
        &resolved.semantic_model_dir.join("definition.pbism"),
        &mut report,
    );
    required_file(
        &resolved.report_dir.join("definition").join("version.json"),
        &mut report,
    );
    required_file(
        &resolved.report_dir.join("definition").join("report.json"),
        &mut report,
    );
    required_file(
        &resolved
            .report_dir
            .join("definition")
            .join("pages")
            .join("pages.json"),
        &mut report,
    );
    required_file(
        &resolved
            .semantic_model_dir
            .join("definition")
            .join("database.tmdl"),
        &mut report,
    );
    required_file(
        &resolved
            .semantic_model_dir
            .join("definition")
            .join("model.tmdl"),
        &mut report,
    );
    required_file(
        &resolved
            .semantic_model_dir
            .join("definition")
            .join("relationships.tmdl"),
        &mut report,
    );

    check_json_files(resolved, &mut report, allow_desktop_runtime_files)?;
    check_report_theme(resolved, &mut report)?;
    check_report_pages(resolved, &mut report)?;
    check_report_filter_configs(resolved, &mut report)?;
    check_semantic_model(resolved, &mut report)?;
    check_offline_hazards(resolved, &mut report, allow_desktop_runtime_files)?;
    Ok(report)
}

fn required_file(path: &Path, report: &mut ValidationReport) {
    if !path.is_file() {
        report
            .errors
            .push(format!("missing required file: {}", path.display()));
    }
}

fn check_json_files(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
    allow_desktop_runtime_files: bool,
) -> CliResult<()> {
    check_json_file(&resolved.pbip_path, report)?;
    for artifact_dir in [&resolved.report_dir, &resolved.semantic_model_dir] {
        check_json_files_in(artifact_dir, report, allow_desktop_runtime_files)?;
    }
    Ok(())
}

fn check_json_files_in(
    artifact_dir: &Path,
    report: &mut ValidationReport,
    allow_desktop_runtime_files: bool,
) -> CliResult<()> {
    // Required-file checks already report a missing artifact as structured
    // validation output. Do not turn that expected validation failure into an
    // exit-70 filesystem traversal error.
    if !artifact_dir.is_dir() {
        return Ok(());
    }
    let entries = WalkDir::new(artifact_dir)
        .into_iter()
        .filter_entry(|entry| {
            !allow_desktop_runtime_files
                || entry.depth() != 1
                || !entry.file_type().is_dir()
                || !entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(".pbi")
        });
    for entry in entries {
        let entry = walkdir_entry(artifact_dir, entry, "walk selected artifact JSON files")?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if is_json_like(path) {
            check_json_file(path, report)?;
        }
    }
    Ok(())
}

fn check_json_file(path: &Path, report: &mut ValidationReport) -> CliResult<()> {
    report.json_files_checked += 1;
    if has_utf8_bom(path)? {
        report
            .errors
            .push(format!("JSON-like file has UTF-8 BOM: {}", path.display()));
    }
    if let Err(err) = read_json_value(path) {
        report.errors.push(err.message);
    }
    Ok(())
}

fn is_json_like(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    file_name == ".platform"
        || matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("json" | "pbip" | "pbir" | "pbism")
        )
}

fn has_utf8_bom(path: &Path) -> CliResult<bool> {
    let bytes = fs::read(path)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?;
    Ok(bytes.starts_with(&[0xEF, 0xBB, 0xBF]))
}

fn check_report_theme(resolved: &ResolvedProject, report: &mut ValidationReport) -> CliResult<()> {
    let report_json_path = resolved.report_dir.join("definition").join("report.json");
    if !report_json_path.is_file() {
        return Ok(());
    }
    let report_json = read_json_value(&report_json_path)?;
    let Some(theme_collection) = report_json.get("themeCollection") else {
        report.errors.push(format!(
            "{} is missing required themeCollection",
            report_json_path.display()
        ));
        return Ok(());
    };
    let Some(theme_collection) = theme_collection.as_object() else {
        report.errors.push(format!(
            "{} themeCollection must be an object",
            report_json_path.display()
        ));
        return Ok(());
    };
    let Some(custom_theme) = theme_collection.get("customTheme") else {
        return Ok(());
    };
    let Some(custom_theme) = custom_theme.as_object() else {
        report.errors.push(format!(
            "{} themeCollection.customTheme must be an object",
            report_json_path.display()
        ));
        return Ok(());
    };
    if custom_theme.contains_key("resource") {
        report.errors.push(format!(
            "{} themeCollection.customTheme.resource is not valid PBIR report schema metadata; use customTheme.name/reportVersionAtImport/type plus report resourcePackages",
            report_json_path.display()
        ));
    }
    for field in ["name", "type"] {
        if custom_theme
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            report.errors.push(format!(
                "{} themeCollection.customTheme.{field} must be a non-empty string",
                report_json_path.display()
            ));
        }
    }
    let theme_version_valid = match report_schema_major(&report_json) {
        Some(3) => custom_theme
            .get("reportVersionAtImport")
            .is_some_and(valid_theme_version_object),
        Some(2) => custom_theme
            .get("reportVersionAtImport")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        _ => false,
    };
    if !theme_version_valid {
        report.errors.push(format!(
            "{} themeCollection.customTheme.reportVersionAtImport must match the report schema version",
            report_json_path.display()
        ));
    }
    let theme_type = custom_theme.get("type").and_then(Value::as_str);
    if !matches!(theme_type, Some("RegisteredResources" | "SharedResources")) {
        report.errors.push(format!(
            "{} themeCollection.customTheme.type must be RegisteredResources or SharedResources",
            report_json_path.display()
        ));
    }
    if theme_type == Some("RegisteredResources") {
        check_registered_theme_resource_package(
            &report_json_path,
            &report_json,
            custom_theme,
            report,
        );
    }
    Ok(())
}

fn check_registered_theme_resource_package(
    report_json_path: &Path,
    report_json: &Value,
    custom_theme: &serde_json::Map<String, Value>,
    report: &mut ValidationReport,
) {
    let theme_name = custom_theme
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !theme_name.to_ascii_lowercase().ends_with(".json") {
        report.errors.push(format!(
            "{} RegisteredResources customTheme name `{theme_name}` must include the .json extension",
            report_json_path.display()
        ));
    }
    let Some(packages) = report_json
        .get("resourcePackages")
        .and_then(Value::as_array)
    else {
        report.errors.push(format!(
            "{} RegisteredResources customTheme requires report resourcePackages",
            report_json_path.display()
        ));
        return;
    };
    let Some(package) = packages.iter().find(|package| {
        package["name"].as_str() == Some("RegisteredResources")
            && package["type"].as_str() == Some("RegisteredResources")
    }) else {
        report.errors.push(format!(
            "{} RegisteredResources customTheme requires a RegisteredResources resource package",
            report_json_path.display()
        ));
        return;
    };
    let Some(items) = package["items"].as_array() else {
        report.errors.push(format!(
            "{} RegisteredResources resource package must contain an items array",
            report_json_path.display()
        ));
        return;
    };
    let has_theme_item = items.iter().any(|item| {
        item["type"].as_str() == Some("CustomTheme") && item["name"].as_str() == Some(theme_name)
    });
    if !has_theme_item {
        report.errors.push(format!(
            "{} RegisteredResources customTheme `{theme_name}` has no matching CustomTheme resource package item",
            report_json_path.display()
        ));
        return;
    }
    let theme_item = items.iter().find(|item| {
        item["type"].as_str() == Some("CustomTheme") && item["name"].as_str() == Some(theme_name)
    });
    let Some(theme_item) = theme_item else {
        return;
    };
    let item_path = theme_item["path"].as_str().unwrap_or_default();
    if item_path != theme_name || item_path.contains('/') || item_path.contains('\\') {
        report.errors.push(format!(
            "{} CustomTheme resource path `{item_path}` must be the same filename as `{theme_name}`",
            report_json_path.display()
        ));
        return;
    }
    let Some(report_dir) = report_json_path.parent().and_then(Path::parent) else {
        return;
    };
    let theme_path = report_dir
        .join("StaticResources")
        .join("RegisteredResources")
        .join(item_path);
    if !theme_path.is_file() {
        report.errors.push(format!(
            "{} references missing CustomTheme file {}",
            report_json_path.display(),
            theme_path.display()
        ));
        return;
    }
    match read_json_value(&theme_path) {
        Ok(theme_json) if theme_json["name"].as_str() == Some(theme_name) => {}
        Ok(theme_json) => report.errors.push(format!(
            "{} theme name `{}` does not match report customTheme name `{theme_name}`",
            theme_path.display(),
            theme_json["name"].as_str().unwrap_or_default()
        )),
        Err(err) => report.errors.push(format!(
            "{} could not be read for theme validation: {}",
            theme_path.display(),
            err.message
        )),
    }
}

fn check_report_pages(resolved: &ResolvedProject, report: &mut ValidationReport) -> CliResult<()> {
    let pages_dir = resolved.report_dir.join("definition").join("pages");
    let pages_json_path = pages_dir.join("pages.json");
    if !pages_json_path.exists() {
        return Ok(());
    }
    let pages_json = read_json_value(&pages_json_path)?;
    let mut page_order = Vec::new();
    let mut seen_pages = BTreeSet::new();
    match pages_json["pageOrder"].as_array() {
        Some(items) => {
            for item in items {
                let Some(page_name) = item.as_str() else {
                    report.errors.push(format!(
                        "{} pageOrder contains a non-string entry",
                        pages_json_path.display()
                    ));
                    continue;
                };
                if !seen_pages.insert(page_name.to_string()) {
                    report.errors.push(format!(
                        "{} pageOrder contains duplicate page: {}",
                        pages_json_path.display(),
                        page_name
                    ));
                }
                page_order.push(page_name.to_string());
            }
        }
        None => report.errors.push(format!(
            "{} has no pageOrder array",
            pages_json_path.display()
        )),
    }
    if page_order.is_empty() {
        report.warnings.push(format!(
            "{} has no pageOrder entries",
            pages_json_path.display()
        ));
    }
    if let Some(active_page_name) = pages_json["activePageName"].as_str()
        && !seen_pages.contains(active_page_name)
    {
        report.errors.push(format!(
            "{} activePageName references a page not in pageOrder: {}",
            pages_json_path.display(),
            active_page_name
        ));
    }
    for page_name in &page_order {
        let page_dir = pages_dir.join(page_name);
        let page_json_path = page_dir.join("page.json");
        if !page_json_path.is_file() {
            report.errors.push(format!(
                "pageOrder references missing page.json: {}",
                page_json_path.display()
            ));
            continue;
        }
        report.pages += 1;
        let page_json = read_json_value(&page_json_path)?;
        if page_json["name"].as_str() != Some(page_name) {
            report.errors.push(format!(
                "{} name does not match page folder {}",
                page_json_path.display(),
                page_name
            ));
        }
        check_positive_page_number(&page_json_path, &page_json, "width", report);
        check_positive_page_number(&page_json_path, &page_json, "height", report);
        let visuals_dir = page_dir.join("visuals");
        if visuals_dir.is_dir() {
            for visual_entry in fs::read_dir(&visuals_dir).map_err(|err| {
                CliError::unexpected(format!("read visuals dir {}: {err}", visuals_dir.display()))
            })? {
                let visual_entry = visual_entry.map_err(|err| {
                    CliError::unexpected(format!(
                        "read visual entry {}: {err}",
                        visuals_dir.display()
                    ))
                })?;
                if !visual_entry
                    .file_type()
                    .map_err(|err| {
                        CliError::unexpected(format!(
                            "read visual entry type {}: {err}",
                            visual_entry.path().display()
                        ))
                    })?
                    .is_dir()
                {
                    continue;
                }
                let visual_json = visual_entry.path().join("visual.json");
                if !visual_json.is_file() {
                    report.errors.push(format!(
                        "visual directory is missing visual.json: {}. Remove the empty visual directory or restore its visual.json before retrying",
                        visual_entry.path().display()
                    ));
                    continue;
                }
                report.visuals += 1;
                let visual = read_json_value(&visual_json)?;
                check_visual_query_state_roles(&visual_json, &visual, report);
                check_visual_minimum_size(&visual_json, &visual, report);
                if visual["visual"]["query"]["queryState"]
                    .as_object()
                    .is_some_and(|query_state| {
                        query_state.values().any(|role| {
                            role["projections"]
                                .as_array()
                                .is_some_and(|projections| !projections.is_empty())
                        })
                    })
                {
                    report.bound_visuals += 1;
                }
            }
        }
    }
    for entry in fs::read_dir(&pages_dir).map_err(|err| {
        CliError::unexpected(format!("read pages dir {}: {err}", pages_dir.display()))
    })? {
        let entry = entry.map_err(|err| {
            CliError::unexpected(format!(
                "read pages dir entry {}: {err}",
                pages_dir.display()
            ))
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !seen_pages.contains(name) && path.join("page.json").is_file() {
            report.warnings.push(format!(
                "page directory is not referenced by pageOrder: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn check_visual_minimum_size(
    visual_json_path: &Path,
    visual: &Value,
    report: &mut ValidationReport,
) {
    const SLICER_MIN_HEIGHT: f64 = 76.0;

    if visual["visual"]["visualType"].as_str() != Some("slicer") {
        return;
    }
    let Some(height) = visual["position"]["height"].as_f64() else {
        report.errors.push(format!(
            "{} slicer position.height must be a number of at least {SLICER_MIN_HEIGHT}",
            visual_json_path.display()
        ));
        return;
    };
    if height < SLICER_MIN_HEIGHT {
        report.errors.push(format!(
            "{} slicer height {height} is below the Power BI minimum of {SLICER_MIN_HEIGHT}",
            visual_json_path.display()
        ));
    }
}

fn check_visual_query_state_roles(
    visual_json_path: &Path,
    visual: &Value,
    report: &mut ValidationReport,
) {
    let Some(visual_type) = visual["visual"]["visualType"].as_str() else {
        return;
    };
    let Ok(supported_roles) = visual_catalog::supported_roles(visual_type) else {
        return;
    };
    let Some(query_state) = visual["visual"]["query"]["queryState"].as_object() else {
        return;
    };
    for (role, role_value) in query_state {
        if !role_value["projections"].is_array() || supported_roles.contains(&role.as_str()) {
            continue;
        }
        match visual_catalog::normalize_role(visual_type, role) {
            Ok(canonical) => report.errors.push(format!(
                "{} {} queryState role `{role}` is a CLI input alias, not the Desktop PBIR role; use `{canonical}`. Reapply the visual bindings with `report visuals set-bindings`",
                visual_json_path.display(),
                visual_type
            )),
            Err(_) => report.errors.push(format!(
                "{} {} queryState contains unsupported role `{role}`; supported Desktop PBIR roles are: {}. Reapply the visual bindings with `report visuals set-bindings`",
                visual_json_path.display(),
                visual_type,
                supported_roles.join(", ")
            )),
        }
    }
}

fn check_positive_page_number(
    page_json_path: &Path,
    page_json: &Value,
    field: &str,
    report: &mut ValidationReport,
) {
    if !page_json[field].as_f64().is_some_and(|value| value > 0.0) {
        report.errors.push(format!(
            "{} has invalid nonpositive or missing page {}",
            page_json_path.display(),
            field
        ));
    }
}

fn check_report_filter_configs(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
) -> CliResult<()> {
    let definition_dir = resolved.report_dir.join("definition");
    if !definition_dir.is_dir() {
        return Ok(());
    }
    for entry in WalkDir::new(&definition_dir) {
        let entry = walkdir_entry(
            &definition_dir,
            entry,
            "walk report definition filter configurations",
        )?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let file_name = path.file_name().and_then(|value| value.to_str());
        if !matches!(file_name, Some("report.json" | "page.json" | "visual.json")) {
            continue;
        }
        let value = read_json_value(path)?;
        check_filter_config(path, &value, report);
    }
    Ok(())
}

fn check_filter_config(path: &Path, value: &Value, report: &mut ValidationReport) {
    let Some(filter_config) = value.get("filterConfig") else {
        return;
    };
    let Some(filter_config) = filter_config.as_object() else {
        report
            .errors
            .push(format!("{} filterConfig is not an object", path.display()));
        return;
    };
    let Some(filters) = filter_config.get("filters") else {
        return;
    };
    let Some(filters) = filters.as_array() else {
        report.errors.push(format!(
            "{} filterConfig.filters is not an array",
            path.display()
        ));
        return;
    };
    for (index, filter) in filters.iter().enumerate() {
        check_filter_config_entry(path, index, filter, report);
    }
}

fn check_filter_config_entry(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    match filter.get("name") {
        Some(Value::String(name)) if name.trim().is_empty() || name.len() > 50 => {
            report.errors.push(format!(
                "{} filterConfig.filters[{index}] name must be between 1 and 50 characters for Power BI Desktop",
                path.display()
            ));
        }
        Some(Value::String(_)) => {}
        None => report.errors.push(format!(
            "{} filterConfig.filters[{index}] is missing required name",
            path.display()
        )),
        Some(_) => report.errors.push(format!(
            "{} filterConfig.filters[{index}] name is not a string",
            path.display()
        )),
    }
    if filter["howCreated"].as_str() == Some("powerbi-cli") {
        report.errors.push(format!(
            "{} filterConfig.filters[{index}] has invalid howCreated \"powerbi-cli\"; use a Power BI value such as \"User\"",
            path.display()
        ));
    }
    let Some(filter_type) = filter["type"].as_str() else {
        return;
    };
    if !matches!(
        filter_type,
        "Categorical" | "Advanced" | "TopN" | "RelativeDate"
    ) {
        return;
    }
    check_filter_field(path, index, filter, report);
    if filter_type == "Categorical"
        && filter["howCreated"].as_str() == Some("Drillthrough")
        && filter.get("filter").is_none()
    {
        return;
    }
    let Some(body) = filter.get("filter").and_then(Value::as_object) else {
        // Desktop materializes one field-well placeholder per visual binding when a
        // report is saved. These entries deliberately carry only name, field, and
        // type; they are metadata, not active filter predicates.
        return;
    };
    if filter_type == "Categorical" && body.contains_key("values") {
        report.errors.push(format!(
            "{} categorical filterConfig.filters[{index}] uses legacy filter.values; expected filter.Version, filter.From, and filter.Where",
            path.display()
        ));
    }
    if filter["filter"]["Version"].as_i64() != Some(2) {
        report.errors.push(format!(
            "{} {filter_type} filterConfig.filters[{index}] is missing filter.Version = 2",
            path.display(),
        ));
    }
    let from = filter["filter"]["From"].as_array();
    if from.is_none_or(|items| items.is_empty()) {
        report.errors.push(format!(
            "{} {filter_type} filterConfig.filters[{index}] is missing non-empty filter.From",
            path.display(),
        ));
    }
    let where_clauses = filter["filter"]["Where"].as_array();
    if where_clauses.is_none_or(|items| items.is_empty()) {
        report.errors.push(format!(
            "{} {filter_type} filterConfig.filters[{index}] is missing non-empty filter.Where",
            path.display(),
        ));
    }
    let aliases = filter_from_aliases(filter);
    if let Some(where_clauses) = where_clauses {
        for clause in where_clauses {
            check_filter_where_source_refs(path, index, clause, &aliases, report);
        }
    }
    match filter_type {
        "Categorical" => check_categorical_filter_shape(path, index, filter, report),
        "Advanced" => check_advanced_filter_shape(path, index, filter, report),
        "TopN" => check_topn_filter_shape(path, index, filter, report),
        "RelativeDate" => check_relative_date_filter_shape(path, index, filter, report),
        _ => {}
    }
}

fn check_filter_field(path: &Path, index: usize, filter: &Value, report: &mut ValidationReport) {
    let field = filter
        .pointer("/field/Column")
        .or_else(|| filter.pointer("/field/Measure"));
    let valid = field.is_some_and(|field| {
        field
            .pointer("/Expression/SourceRef/Entity")
            .and_then(Value::as_str)
            .is_some_and(|entity| !entity.is_empty())
            && field
                .get("Property")
                .and_then(Value::as_str)
                .is_some_and(|property| !property.is_empty())
            && field.pointer("/Expression/SourceRef/Source").is_none()
    });
    if !valid {
        report.errors.push(format!(
            "{} filterConfig.filters[{index}] field must be a Column or Measure with top-level SourceRef.Entity and a Property",
            path.display()
        ));
    }
}

fn check_categorical_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let Some(in_condition) = filter.pointer("/filter/Where/0/Condition/In") else {
        report.errors.push(format!(
            "{} categorical filterConfig.filters[{index}] must use Where[0].Condition.In",
            path.display()
        ));
        return;
    };
    if in_condition["Expressions"]
        .as_array()
        .is_none_or(|items| items.is_empty())
        || !in_condition["Values"].is_array()
    {
        report.errors.push(format!(
            "{} categorical filterConfig.filters[{index}] In condition requires non-empty Expressions and a Values array",
            path.display()
        ));
    }
}

fn check_advanced_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let Some(condition) = filter.pointer("/filter/Where/0/Condition") else {
        report.errors.push(format!(
            "{} Advanced filterConfig.filters[{index}] is missing Where[0].Condition",
            path.display()
        ));
        return;
    };
    if !valid_advanced_condition(condition) {
        report.errors.push(format!(
            "{} Advanced filterConfig.filters[{index}] has an invalid or empty Where[0].Condition expression",
            path.display()
        ));
    }
}

fn valid_advanced_condition(condition: &Value) -> bool {
    if let Some(comparison) = condition.get("Comparison") {
        return comparison["ComparisonKind"].as_i64().is_some()
            && comparison.get("Left").is_some_and(Value::is_object)
            && comparison.get("Right").is_some_and(Value::is_object);
    }
    for operator in ["And", "Or"] {
        if let Some(binary) = condition.get(operator) {
            return binary.get("Left").is_some_and(valid_advanced_condition)
                && binary.get("Right").is_some_and(valid_advanced_condition);
        }
    }
    condition
        .as_object()
        .is_some_and(|condition| !condition.is_empty())
}

fn check_topn_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let subquery = filter["filter"]["From"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["Type"].as_i64() == Some(2)
                    && item.pointer("/Expression/Subquery/Query").is_some()
            })
        })
        .and_then(|item| item.pointer("/Expression/Subquery/Query"));
    let Some(query) = subquery else {
        report.errors.push(format!(
            "{} TopN filterConfig.filters[{index}] requires a Type 2 subquery source in filter.From",
            path.display()
        ));
        return;
    };
    let query_from = query["From"].as_array();
    let query_select = query["Select"].as_array();
    let query_order_by = query["OrderBy"].as_array();
    let top = query["Top"].as_u64();
    let direction = query
        .pointer("/OrderBy/0/Direction")
        .and_then(Value::as_i64);
    let has_measure = query.pointer("/OrderBy/0/Expression/Measure").is_some();
    if query["Version"].as_i64() != Some(2)
        || query_from.is_none_or(|items| items.is_empty())
        || query_select.is_none_or(|items| items.is_empty())
        || query_order_by.is_none_or(|items| items.is_empty())
        || top.is_none_or(|top| top == 0)
        || !matches!(direction, Some(1 | 2))
        || !has_measure
    {
        report.errors.push(format!(
            "{} TopN filterConfig.filters[{index}] subquery requires Version 2, From, Select, measure OrderBy with Direction 1 or 2, and positive Top",
            path.display()
        ));
    }
    let query_aliases = query_from
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["Name"].as_str().map(ToOwned::to_owned))
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    if let Some(select) = query_select {
        for expression in select {
            check_filter_where_source_refs(path, index, expression, &query_aliases, report);
        }
    }
    if let Some(order_by) = query_order_by {
        for expression in order_by {
            check_filter_where_source_refs(path, index, expression, &query_aliases, report);
        }
    }
    let topn_alias = filter["filter"]["From"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["Type"].as_i64() == Some(2)
                    && item.pointer("/Expression/Subquery/Query").is_some()
            })
        })
        .and_then(|item| item["Name"].as_str());
    let table_alias = filter
        .pointer("/filter/Where/0/Condition/In/Table/SourceRef/Source")
        .and_then(Value::as_str);
    if filter
        .pointer("/filter/Where/0/Condition/In/Expressions")
        .and_then(Value::as_array)
        .is_none_or(|items| items.is_empty())
        || table_alias.is_none()
        || table_alias != topn_alias
    {
        report.errors.push(format!(
            "{} TopN filterConfig.filters[{index}] Where must use In.Expressions and reference the Type 2 subquery alias through In.Table.SourceRef.Source",
            path.display()
        ));
    }
}

fn check_relative_date_filter_shape(
    path: &Path,
    index: usize,
    filter: &Value,
    report: &mut ValidationReport,
) {
    let Some(between) = filter.pointer("/filter/Where/0/Condition/Between") else {
        report.errors.push(format!(
            "{} RelativeDate filterConfig.filters[{index}] must use Where[0].Condition.Between",
            path.display()
        ));
        return;
    };
    if between.pointer("/Expression/Column").is_none()
        || !contains_expression_key(&between["LowerBound"], "DateSpan")
        || !contains_expression_key(&between["UpperBound"], "DateSpan")
        || !contains_expression_key(&between["LowerBound"], "Now")
        || !contains_expression_key(&between["UpperBound"], "Now")
    {
        report.errors.push(format!(
            "{} RelativeDate filterConfig.filters[{index}] Between requires a column Expression and DateSpan bounds derived from Now",
            path.display()
        ));
    }
}

fn contains_expression_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(key)
                || object
                    .values()
                    .any(|value| contains_expression_key(value, key))
        }
        Value::Array(items) => items
            .iter()
            .any(|value| contains_expression_key(value, key)),
        _ => false,
    }
}

fn filter_from_aliases(filter: &Value) -> BTreeSet<String> {
    filter["filter"]["From"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["Name"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn check_filter_where_source_refs(
    path: &Path,
    index: usize,
    value: &Value,
    aliases: &BTreeSet<String>,
    report: &mut ValidationReport,
) {
    match value {
        Value::Object(object) => {
            if let Some(source_ref) = object.get("SourceRef").and_then(Value::as_object) {
                if source_ref.get("Entity").and_then(Value::as_str).is_some()
                    && source_ref.get("Source").is_none()
                {
                    report.warnings.push(format!(
                        "{} filterConfig.filters[{index}] Where SourceRef uses Entity instead of Source alias",
                        path.display()
                    ));
                }
                if let Some(source) = source_ref.get("Source").and_then(Value::as_str)
                    && !aliases.is_empty()
                    && !aliases.contains(source)
                {
                    report.warnings.push(format!(
                        "{} filterConfig.filters[{index}] Where SourceRef.Source \"{source}\" is not present in filter.From",
                        path.display()
                    ));
                }
            }
            for child in object.values() {
                check_filter_where_source_refs(path, index, child, aliases, report);
            }
        }
        Value::Array(items) => {
            for child in items {
                check_filter_where_source_refs(path, index, child, aliases, report);
            }
        }
        _ => {}
    }
}

fn check_semantic_model(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
) -> CliResult<()> {
    let definition = resolved.semantic_model_dir.join("definition");
    let tables_dir = definition.join("tables");
    if !tables_dir.is_dir() {
        report.errors.push(format!(
            "missing TMDL tables directory: {}",
            tables_dir.display()
        ));
        return Ok(());
    }
    for entry in fs::read_dir(&tables_dir)
        .map_err(|err| CliError::unexpected(format!("read {}: {err}", tables_dir.display())))?
    {
        let entry = entry.map_err(|err| {
            CliError::unexpected(format!("read {} entry: {err}", tables_dir.display()))
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("tmdl") {
            report.tables += 1;
            let text = fs::read_to_string(&path)
                .map_err(|err| CliError::unexpected(format!("read {}: {err}", path.display())))?;
            report.measures += text
                .lines()
                .filter(|line| line.trim_start().starts_with("measure "))
                .count();
            if !text.contains("partition ") {
                report
                    .warnings
                    .push(format!("table has no partition block: {}", path.display()));
            }
            if text.contains("Sql.Database(") || text.contains("Odbc.DataSource(") {
                report.warnings.push(format!(
                    "table partition appears to contain a real connector, review before taking home: {}",
                    path.display()
                ));
            }
        }
    }
    if report.tables == 0 {
        report.errors.push(format!(
            "semantic model contains no table .tmdl files: {}",
            tables_dir.display()
        ));
    }
    check_relationships(resolved, report)?;
    Ok(())
}

fn check_relationships(resolved: &ResolvedProject, report: &mut ValidationReport) -> CliResult<()> {
    let relationships_path = resolved
        .semantic_model_dir
        .join("definition")
        .join("relationships.tmdl");
    if !relationships_path.is_file() {
        return Ok(());
    }

    let (relationship_doc, tables) = relationship_tmdl::load_relationships_and_tables(resolved)?;
    report.relationships = relationship_doc.relationships.len();
    for relationship in &relationship_doc.relationships {
        if !tmdl_column_exists(&tables, &relationship.from_table, &relationship.from_column) {
            report.errors.push(format!(
                "relationship references missing from column {}.{}: {}",
                relationship.from_table,
                relationship.from_column,
                relationship.handle()
            ));
        }
        if !tmdl_column_exists(&tables, &relationship.to_table, &relationship.to_column) {
            report.errors.push(format!(
                "relationship references missing to column {}.{}: {}",
                relationship.to_table,
                relationship.to_column,
                relationship.handle()
            ));
        }
    }
    check_variation_references(&tables, &relationship_doc.relationships, report)?;
    Ok(())
}

fn check_variation_references(
    tables: &[tmdl::TableDocument],
    relationships: &[relationship_tmdl::RelationshipRecord],
    report: &mut ValidationReport,
) -> CliResult<()> {
    let relationship_names = relationships
        .iter()
        .map(|relationship| relationship.name.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let table_names = tables
        .iter()
        .map(|table| table.table.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut hierarchy_names = BTreeMap::<String, BTreeSet<String>>::new();
    for table in tables {
        let text = fs::read_to_string(&table.path)
            .map_err(|err| CliError::unexpected(format!("read {}: {err}", table.path.display())))?;
        let names = text
            .lines()
            .filter_map(|line| line.trim().strip_prefix("hierarchy "))
            .map(unquote_tmdl_reference)
            .map(|name| name.to_ascii_lowercase())
            .collect();
        hierarchy_names.insert(table.table.to_ascii_lowercase(), names);
    }

    for table in tables {
        let text = fs::read_to_string(&table.path)
            .map_err(|err| CliError::unexpected(format!("read {}: {err}", table.path.display())))?;
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("relationship:") {
                let name = unquote_tmdl_reference(value.trim());
                if !name.is_empty() && !relationship_names.contains(&name.to_ascii_lowercase()) {
                    report.errors.push(format!(
                        "{}:{} variation references missing relationship: {}",
                        table.path.display(),
                        index + 1,
                        name
                    ));
                }
            }
            if let Some(value) = trimmed.strip_prefix("defaultHierarchy:")
                && let Some((table_name, hierarchy_name)) = hierarchy_reference(value.trim())
            {
                let table_key = table_name.to_ascii_lowercase();
                if !table_names.contains(&table_key) {
                    report.errors.push(format!(
                        "{}:{} variation defaultHierarchy references missing table: {}",
                        table.path.display(),
                        index + 1,
                        table_name
                    ));
                } else if !hierarchy_names
                    .get(&table_key)
                    .is_some_and(|names| names.contains(&hierarchy_name.to_ascii_lowercase()))
                {
                    report.errors.push(format!(
                        "{}:{} variation defaultHierarchy references missing hierarchy: {}.{}",
                        table.path.display(),
                        index + 1,
                        table_name,
                        hierarchy_name
                    ));
                }
            }
        }
    }
    Ok(())
}

fn hierarchy_reference(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    if value.starts_with('\'') {
        let bytes = value.as_bytes();
        let mut index = 1;
        while index < bytes.len() {
            if bytes[index] == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                if bytes.get(index + 1) != Some(&b'.') {
                    return None;
                }
                return Some((
                    unquote_tmdl_reference(&value[..=index]),
                    unquote_tmdl_reference(&value[index + 2..]),
                ));
            }
            index += 1;
        }
        None
    } else {
        value.split_once('.').map(|(table, hierarchy)| {
            (
                unquote_tmdl_reference(table),
                unquote_tmdl_reference(hierarchy),
            )
        })
    }
}

fn unquote_tmdl_reference(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value[1..value.len() - 1].replace("''", "'")
    } else {
        value.to_string()
    }
}

fn tmdl_column_exists(tables: &[tmdl::TableDocument], table: &str, column: &str) -> bool {
    tables.iter().any(|document| {
        tmdl::same_name(&document.table, table)
            && document
                .columns
                .iter()
                .any(|record| tmdl::same_name(&record.name, column))
    })
}

fn check_offline_hazards(
    resolved: &ResolvedProject,
    report: &mut ValidationReport,
    allow_desktop_runtime_files: bool,
) -> CliResult<()> {
    for artifact_dir in [&resolved.report_dir, &resolved.semantic_model_dir] {
        if !artifact_dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(artifact_dir) {
            let entry = walkdir_entry(
                artifact_dir,
                entry,
                "walk selected artifact offline-safety inputs",
            )?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let normalized = path
                .strip_prefix(artifact_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase();
            let desktop_runtime_file = path
                .strip_prefix(artifact_dir)
                .ok()
                .and_then(|relative| relative.components().next())
                .is_some_and(|component| {
                    component
                        .as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(".pbi")
                });
            if (!allow_desktop_runtime_files || !desktop_runtime_file)
                && (normalized.ends_with(".pbi/cache.abf")
                    || normalized.ends_with("cache.abf")
                    || normalized.ends_with("localsettings.json")
                    || normalized.ends_with(".pbix")
                    || normalized.ends_with(".pbit"))
            {
                report.errors.push(format!(
                    "offline-unsafe data/cache/local file present: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn report_schema_major(report_json: &Value) -> Option<u64> {
    report_json
        .get("$schema")?
        .as_str()?
        .rsplit_once("/report/")?
        .1
        .split('/')
        .next()?
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn valid_theme_version_object(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == 3
        && ["visual", "page", "report"].into_iter().all(|field| {
            object
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(is_three_part_numeric_version)
        })
}

fn is_three_part_numeric_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .into_iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(crate) fn read_json_value(path: &Path) -> CliResult<Value> {
    let text = fs::read_to_string(path)
        .map_err(|err| CliError::file_not_found(format!("read {}: {err}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::validation_failed(format!("parse JSON {}: {err}", path.display())))
}

fn object_name(prefix: &str, label: &str, index: usize) -> String {
    let slug = slug(label);
    let hash = hash_hex(&format!("{prefix}:{label}:{index}"));
    let short_hash = &hash[..10];
    let base = if slug.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{slug}")
    };
    format!("{base}{short_hash}")
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .take(50)
        .collect()
}

fn sanitized_file_stem(value: &str) -> String {
    let slugged = slug(value);
    if slugged.is_empty() {
        "PowerBIProject".to_string()
    } else {
        slugged
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
    out
}

fn stable_guid(value: &str) -> String {
    let a = hash_hex(value);
    let b = hash_hex(&format!("{value}:powerbi-cli"));
    let hex = format!("{a}{b}");
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[16..19],
        &hex[19..31]
    )
}

fn hash_hex(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn command_arg(path: &Path) -> String {
    let value = path.display().to_string();
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '\'' | '"' | '&' | '(' | ')' | ';'))
    {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        value
    }
}

pub(crate) fn canonical_display(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
