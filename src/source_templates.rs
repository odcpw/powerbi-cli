use crate::project_io::write_json_atomic;
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
    let text = fs::read_to_string(&path)
        .map_err(|err| CliError::file_not_found(format!("read {}: {err}", path.display())))?;
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
            "The work machine needs the Npgsql driver installed for the Power BI PostgreSQL connector."
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
        "credentialFree": !findings.iter().any(|finding| finding.code == "sourceTemplate.credential_like_text"),
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
            code: "sourceTemplate.credential_like_text".to_string(),
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
            code: "sourceTemplate.odbc_dsn_attributes".to_string(),
            severity: "error".to_string(),
            message: "ODBC DSN must be a bare DSN name without ';' or '=' attributes; configure credentials in the ODBC manager or Power BI Desktop".to_string(),
        });
    }
    if !template_contains_placeholders(record) {
        findings.push(SourceTemplateFinding {
            code: "sourceTemplate.specific_values".to_string(),
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

fn m_string(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn template_contains_placeholders(record: &SourceTemplateRecord) -> bool {
    record
        .parameters
        .values()
        .any(|value| value.contains('<') && value.contains('>'))
}

fn sort_templates(store: &mut SourceTemplateStore) {
    store
        .templates
        .sort_by(|left, right| left.handle.cmp(&right.handle));
}
