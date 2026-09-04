use crate::tmdl::ColumnRecord;
use crate::{CliError, CliResult};
use std::collections::BTreeMap;

pub(crate) fn add_connector_column_types(
    source: &str,
    kind: &str,
    parameters: &BTreeMap<String, String>,
    columns: &[ColumnRecord],
) -> String {
    let transformations = columns
        .iter()
        .filter(|column| !column.is_calculated())
        .filter_map(|column| {
            let data_type = m_type(column.data_type.as_deref()?)?;
            let source_column = column.source_column.as_deref().unwrap_or(&column.name);
            Some(format!(
                "{{\"{}\", {data_type}}}",
                source_column.replace('"', "\"\"")
            ))
        })
        .collect::<Vec<_>>();
    if transformations.is_empty() {
        return source.to_string();
    }

    let final_step = match kind {
        "excel" => "PromotedHeaders",
        "csv"
            if parameters
                .get("hasHeader")
                .is_some_and(|value| value == "true") =>
        {
            "PromotedHeaders"
        }
        "csv" => "NamedColumns",
        "folder" => "FilteredFiles",
        "sharepoint" => "SelectedFiles",
        _ => return source.to_string(),
    };
    let mut source = source.to_string();
    if kind == "csv" && final_step == "NamedColumns" {
        let names = columns
            .iter()
            .filter(|column| !column.is_calculated())
            .map(|column| {
                format!(
                    "\"{}\"",
                    column
                        .source_column
                        .as_deref()
                        .unwrap_or(&column.name)
                        .replace('"', "\"\"")
                )
            })
            .collect::<Vec<_>>();
        let replacement = format!(
            ",\n    NamedColumns = Table.RenameColumns(Source, List.Zip({{Table.ColumnNames(Source), {{{}}}}}), MissingField.Ignore)\nin\n    NamedColumns",
            names.join(", ")
        );
        source = source.replacen("\nin\n    Source", &replacement, 1);
    }
    let marker = format!("\nin\n    {final_step}");
    let replacement = format!(
        ",\n    TypedColumns = Table.TransformColumnTypes({final_step}, {{{}}}, \"en-US\")\nin\n    TypedColumns",
        transformations.join(", ")
    );
    source.replacen(&marker, &replacement, 1)
}

pub(crate) fn parse_bool_flag(flag: &str, value: &str) -> CliResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(usage_error(
            format!("{flag} must be true or false; got {value}"),
            "Use an explicit lowercase boolean value.",
        )),
    }
}

pub(crate) fn parse_csv_encoding(value: &str) -> CliResult<u32> {
    let encoding = value.parse::<u32>().map_err(|_| {
        usage_error(
            format!("source-template --encoding must be a positive numeric code page; got {value}"),
            "Use 65001 for UTF-8.",
        )
    })?;
    if encoding == 0 {
        return Err(usage_error(
            "source-template --encoding must be greater than zero",
            "Use 65001 for UTF-8.",
        ));
    }
    Ok(encoding)
}

pub(crate) fn is_placeholder(value: &str) -> bool {
    value.contains('<') && value.contains('>')
}

pub(crate) fn validate_csv_file(file: &str) -> CliResult<()> {
    if [".csv", ".tsv", ".txt"]
        .iter()
        .any(|extension| file.to_ascii_lowercase().ends_with(extension))
    {
        return Ok(());
    }
    Err(usage_error(
        "source-template CSV --file must end in .csv, .tsv, or .txt",
        "Use --kind excel for workbook files.",
    ))
}

pub(crate) fn validate_csv_delimiter(delimiter: &str) -> CliResult<()> {
    if delimiter.chars().count() == 1 && !delimiter.contains(['\r', '\n']) {
        return Ok(());
    }
    Err(usage_error(
        "source-template CSV --delimiter must be exactly one character",
        "Common delimiters are comma, tab, semicolon, and pipe.",
    ))
}

pub(crate) fn validate_folder_pattern(pattern: &str) -> CliResult<()> {
    let wildcard_count = pattern
        .chars()
        .filter(|character| *character == '*')
        .count();
    let valid_wildcard = wildcard_count == 0
        || (wildcard_count == 1 && pattern.starts_with('*') && pattern.len() > 1);
    if !pattern.trim().is_empty()
        && valid_wildcard
        && !pattern.contains(['?', '/', '\\', '\r', '\n'])
        && !matches!(pattern, "." | "..")
    {
        return Ok(());
    }
    Err(usage_error(
        "source-template folder --pattern must be an exact file name or one leading-wildcard suffix such as *.csv",
        "Patterns with directories, ?, or multiple/interior * wildcards are refused.",
    ))
}

pub(crate) fn validate_sharepoint_site_url(site_url: &str) -> CliResult<()> {
    let Some(authority_and_path) = site_url.strip_prefix("https://") else {
        return Err(usage_error(
            "source-template SharePoint --site-url must use https://",
            "Pass the SharePoint site root, without credentials, query, or fragment.",
        ));
    };
    let authority = authority_and_path.split('/').next().unwrap_or_default();
    if authority.to_ascii_lowercase().ends_with(".sharepoint.com")
        && !authority.contains(['@', ':'])
        && !site_url.contains(['?', '#', '\r', '\n'])
    {
        return Ok(());
    }
    Err(usage_error(
        "source-template SharePoint --site-url must be a credential-free *.sharepoint.com site URL without query or fragment",
        "Example: https://contoso.sharepoint.com/sites/Finance",
    ))
}

pub(crate) fn validate_sharepoint_library(library: &str) -> CliResult<()> {
    if is_placeholder(library)
        || (!library.trim().is_empty()
            && !matches!(library, "." | "..")
            && !library.contains(['/', '\\', '\r', '\n']))
    {
        return Ok(());
    }
    Err(usage_error(
        "source-template SharePoint --library must be one library name",
        "Do not include slashes; put folders in --path.",
    ))
}

pub(crate) fn validate_relative_source_path(path: &str) -> CliResult<()> {
    if is_placeholder(path) {
        return Ok(());
    }
    let valid = !path.trim().is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.ends_with(['/', '\\'])
        && !path.contains(['\\', '\r', '\n', '?', '#'])
        && path
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."));
    if valid {
        return Ok(());
    }
    Err(usage_error(
        "source-template SharePoint --path must be a relative folder path without traversal",
        "Example: Published/Exports",
    ))
}

fn m_type(data_type: &str) -> Option<&'static str> {
    match data_type.trim().to_ascii_lowercase().as_str() {
        "int64" => Some("Int64.Type"),
        "double" => Some("type number"),
        "decimal" => Some("Decimal.Type"),
        "currency" => Some("Currency.Type"),
        "datetime" => Some("type datetime"),
        "datetimezone" => Some("type datetimezone"),
        "date" => Some("type date"),
        "time" => Some("type time"),
        "boolean" => Some("type logical"),
        "string" => Some("type text"),
        "binary" => Some("type binary"),
        _ => None,
    }
}

fn usage_error(message: impl Into<String>, hint: impl Into<String>) -> CliError {
    CliError::invalid_args(message)
        .with_hint(hint)
        .with_suggested_command("powerbi-cli --json capabilities --for source-template")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parameters_accept_explicit_safe_values_and_refuse_ambiguous_values() {
        assert_eq!(parse_csv_encoding("65001").unwrap(), 65_001);
        assert!(parse_csv_encoding("0").is_err());
        assert!(validate_csv_delimiter(";").is_ok());
        assert!(validate_csv_delimiter("||").is_err());
        assert!(validate_csv_file("C:\\Data\\sales.CSV").is_ok());
        assert!(validate_csv_file("sales.xlsx").is_err());
    }

    #[test]
    fn folder_pattern_grammar_is_closed_and_deterministic() {
        assert!(validate_folder_pattern("*.csv").is_ok());
        assert!(validate_folder_pattern("sales.csv").is_ok());
        assert!(validate_folder_pattern("sales-*.csv").is_err());
        assert!(validate_folder_pattern("../*.csv").is_err());
    }

    #[test]
    fn sharepoint_identifiers_refuse_insecure_urls_and_path_traversal() {
        assert!(
            validate_sharepoint_site_url("https://contoso.sharepoint.com/sites/Finance").is_ok()
        );
        assert!(
            validate_sharepoint_site_url("http://contoso.sharepoint.com/sites/Finance").is_err()
        );
        assert!(validate_sharepoint_library("Documents").is_ok());
        assert!(validate_sharepoint_library("Documents/Finance").is_err());
        assert!(validate_relative_source_path("Published/Exports").is_ok());
        assert!(validate_relative_source_path("../Exports").is_err());
    }
}
