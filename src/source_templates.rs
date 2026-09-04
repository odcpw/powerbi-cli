use crate::input_safety::{InputKind, read_utf8};
use crate::project_io::write_json_atomic;
use crate::rules;
use crate::safety_scan::{
    contains_credential_like_text_str, redact_credential_parameter, redact_credential_values,
};
use crate::{CliError, CliResult, ResolvedProject, canonical_display};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const SOURCE_TEMPLATES_SCHEMA: &str = "powerbi-cli.source-templates.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceTemplateStore {
    pub(crate) schema: String,
    #[serde(default)]
    pub(crate) templates: Vec<SourceTemplateRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceTemplateRecord {
    pub(crate) handle: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    pub(crate) partition_handle: String,
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) parameters: BTreeMap<String, String>,
    pub(crate) m_template: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) requirements: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceTemplateFinding {
    pub(crate) code: String,
    pub(crate) severity: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SqlSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) server: String,
    pub(crate) database: String,
    pub(crate) schema: String,
    pub(crate) object: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PostgresSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) server: String,
    pub(crate) database: String,
    pub(crate) schema: String,
    pub(crate) object: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct OdbcSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) dsn: String,
    pub(crate) database: String,
    pub(crate) schema: String,
    pub(crate) object: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExcelSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) file: String,
    pub(crate) item: String,
    pub(crate) item_kind: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CsvSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) file: String,
    pub(crate) delimiter: String,
    pub(crate) encoding: u32,
    pub(crate) has_header: bool,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct FolderSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) path: String,
    pub(crate) pattern: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SharePointSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) site_url: String,
    pub(crate) library: String,
    pub(crate) path: String,
    pub(crate) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct GenericMSourceTemplateInput {
    pub(crate) table: String,
    pub(crate) partition: String,
    pub(crate) name: Option<String>,
    pub(crate) m_template: String,
    pub(crate) description: Option<String>,
}

impl Default for SourceTemplateStore {
    fn default() -> Self {
        Self {
            schema: SOURCE_TEMPLATES_SCHEMA.to_string(),
            templates: Vec::new(),
        }
    }
}

pub(crate) fn source_templates_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join(".powerbi-cli")
        .join("source-templates.json")
}

pub(crate) fn load_source_template_store(
    resolved: &ResolvedProject,
) -> CliResult<SourceTemplateStore> {
    let path = source_templates_path(&resolved.project_dir);
    if !path.exists() {
        return Ok(SourceTemplateStore::default());
    }
    let text = read_utf8(&path, InputKind::JsonArtifact)?;
    let mut store: SourceTemplateStore = serde_json::from_str(&text).map_err(|err| {
        CliError::validation_failed(format!("parse JSON {}: {err}", path.display()))
    })?;
    if store.schema.trim().is_empty() {
        store.schema = SOURCE_TEMPLATES_SCHEMA.to_string();
    }
    if store.schema != SOURCE_TEMPLATES_SCHEMA {
        return Err(CliError::validation_failed(format!(
            "unsupported source template schema in {}: {}",
            path.display(),
            store.schema
        )));
    }
    sort_templates(&mut store);
    Ok(store)
}

pub(crate) fn save_source_template_store(
    resolved: &ResolvedProject,
    store: &SourceTemplateStore,
) -> CliResult<()> {
    let path = source_templates_path(&resolved.project_dir);
    let parent = path
        .parent()
        .ok_or_else(|| CliError::unexpected(format!("path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|err| CliError::unexpected(format!("create {}: {err}", parent.display())))?;
    let value = serde_json::to_value(store)
        .map_err(|err| CliError::unexpected(format!("serialize source templates: {err}")))?;
    if path.exists() {
        write_json_atomic(&path, &value)
    } else {
        let text = serde_json::to_string_pretty(&value).map_err(|err| {
            CliError::unexpected(format!(
                "serialize source templates for {}: {err}",
                path.display()
            ))
        })?;
        fs::write(&path, text)
            .map_err(|err| CliError::unexpected(format!("write {}: {err}", path.display())))
    }
}

pub(crate) fn upsert_template(
    store: &mut SourceTemplateStore,
    record: SourceTemplateRecord,
) -> Option<SourceTemplateRecord> {
    let previous = store
        .templates
        .iter()
        .position(|template| template.partition_handle == record.partition_handle)
        .map(|index| store.templates.remove(index));
    store.templates.push(record);
    sort_templates(store);
    previous
}

pub(crate) fn find_template<'a>(
    store: &'a SourceTemplateStore,
    partition_handle: &str,
) -> Option<&'a SourceTemplateRecord> {
    store
        .templates
        .iter()
        .find(|template| template.partition_handle == partition_handle)
}

pub(crate) fn source_template_handle(table: &str, partition: &str) -> String {
    format!("source-template:{table}:{partition}")
}

pub(crate) fn sql_source_template(input: SqlSourceTemplateInput) -> SourceTemplateRecord {
    let mut parameters = BTreeMap::new();
    parameters.insert("server".to_string(), input.server.clone());
    parameters.insert("database".to_string(), input.database.clone());
    parameters.insert("schema".to_string(), input.schema.clone());
    parameters.insert("object".to_string(), input.object.clone());
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "sql".to_string(),
        parameters,
        m_template: render_sql_m_template(
            &input.server,
            &input.database,
            &input.schema,
            &input.object,
        ),
        description: input.description,
        requirements: Vec::new(),
    }
}

pub(crate) fn postgres_source_template(input: PostgresSourceTemplateInput) -> SourceTemplateRecord {
    let mut parameters = BTreeMap::new();
    parameters.insert("server".to_string(), input.server.clone());
    parameters.insert("database".to_string(), input.database.clone());
    parameters.insert("schema".to_string(), input.schema.clone());
    parameters.insert("object".to_string(), input.object.clone());
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "postgres".to_string(),
        parameters,
        m_template: render_postgres_m_template(
            &input.server,
            &input.database,
            &input.schema,
            &input.object,
        ),
        description: input.description,
        requirements: vec![
            "Current Power BI Desktop releases include the Npgsql provider. Install Npgsql separately only for Power BI Desktop releases before December 2019 or on-premises data gateway releases before June 2025."
                .to_string(),
        ],
    }
}

pub(crate) fn odbc_source_template(input: OdbcSourceTemplateInput) -> SourceTemplateRecord {
    let mut parameters = BTreeMap::new();
    parameters.insert("dsn".to_string(), input.dsn.clone());
    parameters.insert("database".to_string(), input.database.clone());
    parameters.insert("schema".to_string(), input.schema.clone());
    parameters.insert("object".to_string(), input.object.clone());
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "odbc".to_string(),
        parameters,
        m_template: render_odbc_m_template(
            &input.dsn,
            &input.database,
            &input.schema,
            &input.object,
        ),
        description: input.description,
        requirements: vec![
            "The configured ODBC DSN must exist on the work machine before rebinding.".to_string(),
        ],
    }
}

pub(crate) fn excel_source_template(input: ExcelSourceTemplateInput) -> SourceTemplateRecord {
    let mut parameters = BTreeMap::new();
    parameters.insert("file".to_string(), input.file.clone());
    parameters.insert("item".to_string(), input.item.clone());
    parameters.insert("itemKind".to_string(), input.item_kind.clone());
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "excel".to_string(),
        parameters,
        m_template: render_excel_m_template(&input.file, &input.item, &input.item_kind),
        description: input.description,
        requirements: vec![
            "The Excel workbook must exist at the configured path on the machine that refreshes the project."
                .to_string(),
        ],
    }
}

pub(crate) fn csv_source_template(input: CsvSourceTemplateInput) -> SourceTemplateRecord {
    let mut parameters = BTreeMap::new();
    parameters.insert("file".to_string(), input.file.clone());
    parameters.insert("delimiter".to_string(), input.delimiter.clone());
    parameters.insert("encoding".to_string(), input.encoding.to_string());
    parameters.insert("hasHeader".to_string(), input.has_header.to_string());
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "csv".to_string(),
        parameters,
        m_template: render_csv_m_template(
            &input.file,
            &input.delimiter,
            input.encoding,
            input.has_header,
        ),
        description: input.description,
        requirements: vec![
            "The CSV file must exist at the configured path on the machine that refreshes the project."
                .to_string(),
        ],
    }
}

pub(crate) fn folder_source_template(input: FolderSourceTemplateInput) -> SourceTemplateRecord {
    let mut parameters = BTreeMap::new();
    parameters.insert("path".to_string(), input.path.clone());
    parameters.insert("pattern".to_string(), input.pattern.clone());
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "folder".to_string(),
        parameters,
        m_template: render_folder_m_template(&input.path, &input.pattern),
        description: input.description,
        requirements: vec![
            "The folder must exist at the configured path on the machine that refreshes the project."
                .to_string(),
        ],
    }
}

pub(crate) fn sharepoint_source_template(
    input: SharePointSourceTemplateInput,
) -> SourceTemplateRecord {
    let mut parameters = BTreeMap::new();
    parameters.insert("siteUrl".to_string(), input.site_url.clone());
    parameters.insert("library".to_string(), input.library.clone());
    parameters.insert("path".to_string(), input.path.clone());
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "sharepoint".to_string(),
        parameters,
        m_template: render_sharepoint_m_template(
            &input.site_url,
            &input.library,
            &input.path,
        ),
        description: input.description,
        requirements: vec![
            "Authenticate to the SharePoint or OneDrive site in Power BI Desktop on the work machine."
                .to_string(),
        ],
    }
}

pub(crate) fn generic_m_source_template(
    input: GenericMSourceTemplateInput,
) -> SourceTemplateRecord {
    SourceTemplateRecord {
        handle: source_template_handle(
            &input.table,
            input.name.as_deref().unwrap_or(&input.partition),
        ),
        name: input.name,
        partition_handle: crate::tmdl::partition_handle(&input.table, &input.partition),
        table: input.table,
        partition: input.partition,
        kind: "generic-m".to_string(),
        parameters: BTreeMap::new(),
        m_template: input.m_template,
        description: input.description,
        requirements: vec![
            "The generic M expression is validated against the closed connector grammar; authenticate only in Power BI Desktop on the work machine.".to_string(),
        ],
    }
}

pub(crate) fn source_template_json(record: &SourceTemplateRecord, path: &Path) -> Value {
    let safety = source_template_safety_json(record);
    let redact = safety["credentialFree"] == Value::Bool(false);
    let parameters = record
        .parameters
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                if redact {
                    redact_credential_parameter(key, value)
                } else {
                    value.clone()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let m_template = if redact {
        redact_credential_values(&record.m_template)
    } else {
        record.m_template.clone()
    };
    let description = record.description.as_ref().map(|description| {
        if redact {
            redact_credential_values(description)
        } else {
            description.clone()
        }
    });
    let requirements = record
        .requirements
        .iter()
        .map(|requirement| {
            if redact {
                redact_credential_values(requirement)
            } else {
                requirement.clone()
            }
        })
        .collect::<Vec<_>>();
    json!({
        "handle": record.handle,
        "name": record.name,
        "partitionHandle": record.partition_handle,
        "table": record.table,
        "partition": record.partition,
        "kind": record.kind,
        "parameters": parameters,
        "mTemplate": m_template,
        "description": description,
        "requirements": requirements,
        "safety": safety,
        "path": canonical_display(path)
    })
}

pub(crate) fn source_template_safety_json(record: &SourceTemplateRecord) -> Value {
    let findings = source_template_findings(record);
    let status = if findings.iter().any(|finding| finding.severity == "error") {
        "unsafe"
    } else if findings.is_empty() {
        "safe"
    } else {
        "review"
    };
    json!({
        "status": status,
        "safeForHome": status != "unsafe",
        "credentialFree": !findings.iter().any(|finding| finding.code == rules::SOURCE_TEMPLATE_CREDENTIAL_LIKE_TEXT),
        "containsPlaceholders": template_contains_placeholders(record),
        "findings": findings.iter().map(|finding| json!({
            "code": finding.code,
            "severity": finding.severity,
            "message": finding.message
        })).collect::<Vec<_>>()
    })
}

pub(crate) fn source_template_findings_json(
    record: &SourceTemplateRecord,
    path: &Path,
) -> Vec<Value> {
    source_template_findings(record)
        .into_iter()
        .map(|finding| {
            json!({
                "code": finding.code,
                "severity": finding.severity,
                "message": finding.message,
                "handle": record.handle,
                "path": canonical_display(path)
            })
        })
        .collect()
}

pub(crate) fn source_template_findings(
    record: &SourceTemplateRecord,
) -> Vec<SourceTemplateFinding> {
    let mut findings = Vec::new();
    let mut searchable = format!("{} {}", record.kind, record.m_template);
    for (key, value) in &record.parameters {
        searchable.push(' ');
        searchable.push_str(key);
        searchable.push('=');
        searchable.push_str(value);
    }
    if let Some(description) = &record.description {
        searchable.push(' ');
        searchable.push_str(description);
    }
    for requirement in &record.requirements {
        searchable.push(' ');
        searchable.push_str(requirement);
    }
    if contains_credential_like_text_str(&searchable) {
        findings.push(SourceTemplateFinding {
            code: rules::SOURCE_TEMPLATE_CREDENTIAL_LIKE_TEXT.to_string(),
            severity: "error".to_string(),
            message: "source template contains credential-like text".to_string(),
        });
    }
    if record.kind.eq_ignore_ascii_case("odbc")
        && record
            .parameters
            .get("dsn")
            .is_some_and(|dsn| dsn.contains(';') || dsn.contains('='))
    {
        findings.push(SourceTemplateFinding {
            code: rules::SOURCE_TEMPLATE_ODBC_DSN_ATTRIBUTES.to_string(),
            severity: "error".to_string(),
            message: "ODBC DSN must be a bare DSN name without ';' or '=' attributes; configure credentials in the ODBC manager or Power BI Desktop".to_string(),
        });
    }
    if record.kind.eq_ignore_ascii_case("generic-m")
        && crate::workflow::validate_generic_m_template(&record.m_template).is_err()
    {
        findings.push(SourceTemplateFinding {
            code: rules::SOURCE_TEMPLATE_M_GRAMMAR.to_string(),
            severity: "error".to_string(),
            message: "generic M source template is outside the closed connector grammar"
                .to_string(),
        });
    }
    if !template_contains_placeholders(record) {
        findings.push(SourceTemplateFinding {
            code: rules::SOURCE_TEMPLATE_SPECIFIC_VALUES.to_string(),
            severity: "warning".to_string(),
            message: "source template stores specific source identifiers; placeholders are safer for home handoff".to_string(),
        });
    }
    findings
}

pub(crate) fn template_has_errors(record: &SourceTemplateRecord) -> bool {
    source_template_findings(record)
        .iter()
        .any(|finding| finding.severity == "error")
}

fn render_sql_m_template(server: &str, database: &str, schema: &str, object: &str) -> String {
    format!(
        "let\n    Source = Sql.Database(\"{}\", \"{}\"),\n    Navigation = Source{{[Schema=\"{}\",Item=\"{}\"]}}[Data]\nin\n    Navigation",
        m_string(server),
        m_string(database),
        m_string(schema),
        m_string(object)
    )
}

fn render_postgres_m_template(server: &str, database: &str, schema: &str, object: &str) -> String {
    format!(
        "let\n    Source = PostgreSQL.Database(\"{}\", \"{}\"),\n    Navigation = Source{{[Schema=\"{}\",Item=\"{}\"]}}[Data]\nin\n    Navigation",
        m_string(server),
        m_string(database),
        m_string(schema),
        m_string(object)
    )
}

fn render_odbc_m_template(dsn: &str, database: &str, schema: &str, object: &str) -> String {
    format!(
        "let\n    Source = Odbc.DataSource(\"dsn={}\", [HierarchicalNavigation = true]),\n    Navigation = Source{{[Name=\"{}\"]}}[Data]{{[Name=\"{}\"]}}[Data]{{[Name=\"{}\"]}}[Data]\nin\n    Navigation",
        m_string(dsn),
        m_string(database),
        m_string(schema),
        m_string(object)
    )
}

fn render_excel_m_template(file: &str, item: &str, item_kind: &str) -> String {
    format!(
        "let\n    Source = Excel.Workbook(File.Contents(\"{}\"), null, true),\n    Navigation = Source{{[Item=\"{}\",Kind=\"{}\"]}}[Data],\n    PromotedHeaders = Table.PromoteHeaders(Navigation, [PromoteAllScalars = true])\nin\n    PromotedHeaders",
        m_string(file),
        m_string(item),
        m_string(item_kind)
    )
}

fn render_csv_m_template(file: &str, delimiter: &str, encoding: u32, has_header: bool) -> String {
    let source = format!(
        "Csv.Document(File.Contents(\"{}\"), [Delimiter=\"{}\", Encoding={}, QuoteStyle=QuoteStyle.Csv])",
        m_string(file),
        m_string(delimiter),
        encoding
    );
    if has_header {
        format!(
            "let\n    Source = {source},\n    PromotedHeaders = Table.PromoteHeaders(Source, [PromoteAllScalars = true])\nin\n    PromotedHeaders"
        )
    } else {
        format!("let\n    Source = {source}\nin\n    Source")
    }
}

fn render_folder_m_template(path: &str, pattern: &str) -> String {
    let predicate = if let Some(suffix) = pattern.strip_prefix('*') {
        format!(
            "Text.EndsWith([Name], \"{}\", Comparer.OrdinalIgnoreCase)",
            m_string(suffix)
        )
    } else {
        format!("[Name] = \"{}\"", m_string(pattern))
    };
    format!(
        "let\n    Source = Folder.Files(\"{}\"),\n    FilteredFiles = Table.SelectRows(Source, each {predicate})\nin\n    FilteredFiles",
        m_string(path)
    )
}

fn render_sharepoint_m_template(site_url: &str, library: &str, path: &str) -> String {
    let relative_path = format!("{}/{}", library.trim_matches('/'), path.trim_matches('/'));
    format!(
        "let\n    Source = SharePoint.Files(\"{}\", [ApiVersion = 15]),\n    SelectedFiles = Table.SelectRows(Source, each Text.Contains([Folder Path], \"/{}/\", Comparer.OrdinalIgnoreCase))\nin\n    SelectedFiles",
        m_string(site_url),
        m_string(&relative_path)
    )
}

fn m_string(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn template_contains_placeholders(record: &SourceTemplateRecord) -> bool {
    record
        .parameters
        .values()
        .any(|value| value.contains('<') && value.contains('>'))
        || (record.kind.eq_ignore_ascii_case("generic-m")
            && (record.m_template.contains("{{powerbi-cli.")
                || (record.m_template.contains('<') && record.m_template.contains('>'))))
}

fn sort_templates(store: &mut SourceTemplateStore) {
    store
        .templates
        .sort_by(|left, right| left.handle.cmp(&right.handle));
}
