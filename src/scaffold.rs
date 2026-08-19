//! Schema-driven PBIP/PBIR/TMDL project scaffolding.

use crate::dashboard_scaffold::{PageSpec, effective_pages, visual_json};
use crate::{
    CliError, CliResult, canonical_display, command_arg, read_dir_entry, read_json_value,
    resolve_project, schema, tmdl, validate_project,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub(crate) const PBIP_SCHEMA: &str =
    "https://developer.microsoft.com/json-schemas/fabric/pbip/pbipProperties/1.0.0/schema.json";
pub(crate) const REPORT_DEFINITION_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json";
pub(crate) const SEMANTIC_MODEL_DEFINITION_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/semanticModel/definitionProperties/1.0.0/schema.json";
const REPORT_VERSION_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json";
const REPORT_DEFINITION_VERSION: &str = "2.0.0";
const REPORT_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/2.0.0/schema.json";
const PAGES_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/pagesMetadata/1.0.0/schema.json";
const PAGE_SCHEMA: &str = "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/page/2.0.0/schema.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DashboardSpec {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    pub(super) tables: Vec<TableSpec>,
    #[serde(default)]
    relationships: Vec<RelationshipSpec>,
    #[serde(default)]
    pub(super) pages: Vec<PageSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TableSpec {
    pub(super) name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    pub(super) columns: Vec<ColumnSpec>,
    #[serde(default)]
    measures: Vec<MeasureSpec>,
    #[serde(default)]
    rows: Vec<Map<String, Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ColumnSpec {
    pub(super) name: String,
    #[serde(default)]
    pub(super) data_type: Option<String>,
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
    #[serde(default)]
    sort_by_column: Option<String>,
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

#[derive(Debug)]
struct ScaffoldOptions {
    schema: PathBuf,
    out_dir: PathBuf,
    force: bool,
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
        &platform_json("Report", display_name),
    )?;
    write_json_file(
        &semantic_model_dir.join(".platform"),
        &platform_json("SemanticModel", display_name),
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
        let mut page_json = json!({
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
        });
        if !page.interactions.is_empty() {
            page_json["visualInteractions"] = Value::Array(
                page.interactions
                    .iter()
                    .map(|interaction| {
                        json!({
                            "source": interaction.source,
                            "target": interaction.target,
                            "type": interaction.interaction_type
                        })
                    })
                    .collect(),
            );
        }
        write_json_file(&page_dir.join("page.json"), &page_json)?;

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

// The Fabric platformProperties 2.0.0 schema defines only `type` and
// `displayName`; a `description` here is an unknown property that risks
// Desktop-version rejection, so schema descriptions never reach .platform.
fn platform_json(kind: &str, display_name: &str) -> Value {
    let mut metadata = Map::new();
    metadata.insert("type".to_string(), Value::String(kind.to_string()));
    metadata.insert(
        "displayName".to_string(),
        Value::String(display_name.to_string()),
    );
    json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/gitIntegration/platformProperties/2.0.0/schema.json",
        "metadata": metadata,
        "config": {
            "version": "2.0",
            "logicalId": stable_guid(&format!("{kind}:{display_name}"))
        }
    })
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
        if let Some(sort_by_column) = column.sort_by_column.as_deref() {
            if sort_by_column.eq_ignore_ascii_case(&column.name) {
                return Err(CliError::invalid_args(format!(
                    "table {} column {} cannot sort by itself",
                    table.name, column.name
                ))
                .with_hint("Set sortByColumn to a different column in the same table."));
            }
            if !table
                .columns
                .iter()
                .any(|candidate| candidate.name.eq_ignore_ascii_case(sort_by_column))
            {
                return Err(CliError::invalid_args(format!(
                    "table {} column {} sortByColumn target {} does not exist",
                    table.name, column.name, sort_by_column
                ))
                .with_hint("Add the sort column to the same table or correct sortByColumn."));
            }
            out.push_str(&format!(
                "        sortByColumn: {}\n",
                tmdl_object_name(sort_by_column)
            ));
        }
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
            is_hidden: false,
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
pub(super) struct NormalizedDataType {
    pub(super) tmdl: &'static str,
    m: &'static str,
    default_format_string: Option<&'static str>,
}

pub(super) fn normalize_data_type(value: Option<&str>) -> CliResult<NormalizedDataType> {
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
                    "summarizeBy": column.summarize_by,
                    "sortByColumn": column.sort_by_column
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
                        "formatString": binding.format_string,
                        "sortDirection": binding.sort_direction
                    })).collect::<Vec<_>>(),
                    "x": visual.x,
                    "y": visual.y,
                    "width": visual.width,
                    "height": visual.height
                })).collect::<Vec<_>>(),
                "interactions": page.interactions.iter().map(|interaction| json!({
                    "source": interaction.source,
                    "target": interaction.target,
                    "type": interaction.interaction_type
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

pub(super) fn object_name(prefix: &str, label: &str, index: usize) -> String {
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

    #[test]
    fn table_manifest_emits_valid_sort_by_column_metadata() {
        let spec: DashboardSpec = serde_json::from_value(json!({
            "name": "SortProof",
            "tables": [{
                "name": "Severity",
                "columns": [
                    {
                        "name": "Label",
                        "dataType": "string",
                        "sortByColumn": "Order"
                    },
                    {
                        "name": "Order",
                        "dataType": "int64"
                    }
                ]
            }]
        }))
        .expect("manifest");

        let tmdl = table_tmdl(&spec.tables[0]).expect("table TMDL");
        assert!(tmdl.contains("sortByColumn: Order"));
    }

    #[test]
    fn table_manifest_rejects_missing_sort_by_column_target() {
        let spec: DashboardSpec = serde_json::from_value(json!({
            "name": "SortProof",
            "tables": [{
                "name": "Severity",
                "columns": [{
                    "name": "Label",
                    "dataType": "string",
                    "sortByColumn": "Missing"
                }]
            }]
        }))
        .expect("manifest");

        let error = table_tmdl(&spec.tables[0]).expect_err("missing sort target");
        assert!(
            error
                .message
                .contains("sortByColumn target Missing does not exist")
        );
    }
}
