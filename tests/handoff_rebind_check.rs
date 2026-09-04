mod common;

use common::{assert_json_snapshot, run_powerbi, scaffold_sales, stderr_json, stdout_json};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn project_arg(project: &Path) -> &str {
    project.to_str().expect("project path")
}

fn table_path(project: &Path, table: &str) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join(format!("{table}.tmdl"))
}

fn replace_source(project: &Path, table: &str, source: Option<&str>) {
    let path = table_path(project, table);
    let text = fs::read_to_string(&path).expect("read table");
    let source_start = text.find("        source =").expect("source declaration");
    let prefix = &text[..source_start];
    let replacement = source.map(|source| {
        let indented = source
            .lines()
            .map(|line| format!("            {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("        source =\n{indented}\n")
    });
    let updated = match replacement {
        Some(replacement) => format!("{prefix}{replacement}"),
        None => prefix.to_string(),
    };
    fs::write(path, updated).expect("write table");
}

fn check(project: &Path, extra: &[&str]) -> (i32, Value, String) {
    let mut args = vec!["handoff", "rebind-check", project_arg(project)];
    args.extend_from_slice(extra);
    args.push("--json");
    let output = run_powerbi(&args);
    let value = stdout_json(&output);
    (output.code, value, output.stdout)
}

fn has_code(value: &Value, code: &str) -> bool {
    value["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .any(|finding| finding["code"] == code)
}

#[test]
fn missing_project_uses_the_standard_error_envelope_and_live_suggestion() {
    let output = run_powerbi(&["handoff", "rebind-check", "--json"]);
    assert_eq!(output.code, 2);
    assert!(output.stdout.trim().is_empty());
    let envelope = stderr_json(&output);
    assert_eq!(envelope["error"]["code"], "invalid_args");
    assert!(envelope["error"]["hint"].is_string());
    assert!(
        envelope["error"]["suggestedCommands"]
            .as_array()
            .expect("suggestions")
            .iter()
            .all(|command| command
                .as_str()
                .is_some_and(|command| command.starts_with("powerbi-cli handoff rebind-check")))
    );
}

#[test]
fn unmatched_partition_selector_is_a_structured_validation_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let output = run_powerbi(&[
        "handoff",
        "rebind-check",
        project_arg(&project),
        "--partition",
        "partition:FactSales:Missing",
        "--json",
    ]);
    assert_eq!(output.code, 10);
    assert!(output.stdout.trim().is_empty());
    let envelope = stderr_json(&output);
    assert_eq!(envelope["error"]["code"], "validation_failed");
    assert!(envelope["error"]["hint"].is_string());
    assert!(
        envelope["error"]["suggestedCommands"]
            .as_array()
            .expect("suggestions")
            .iter()
            .all(|command| command
                .as_str()
                .is_some_and(|command| command.starts_with("powerbi-cli model partitions list")))
    );
}

#[test]
fn generated_dummy_partition_is_reported_as_placeholder() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert_eq!(value["schema"], "powerbi-cli.handoff.rebind-check.v1");
    assert_eq!(value["ok"], false);
    assert_eq!(value["partitions"][0]["state"], "placeholder");
    assert!(has_code(&value, "rebindCheck.partition_placeholder"));
}

#[test]
fn missing_source_is_reported_without_guessing_a_connector() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    replace_source(&project, "FactSales", None);
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert_eq!(value["partitions"][0]["state"], "unresolved");
    assert!(has_code(&value, "rebindCheck.partition_source_missing"));
}

#[test]
fn unresolved_source_placeholder_is_reported_and_not_materialized() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    replace_source(
        &project,
        "FactSales",
        Some("Sql.Database(\"<server>\", \"Sales\")"),
    );
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert_eq!(value["partitions"][0]["state"], "placeholder");
    assert!(has_code(&value, "rebindCheck.source_placeholder"));
}

#[test]
fn incomplete_sql_source_is_reported_without_connection_attempt() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    replace_source(&project, "FactSales", Some("Sql.Database(\"server\")"));
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert_eq!(value["partitions"][0]["sourceKind"], "sqlDatabase");
    assert!(has_code(&value, "rebindCheck.source_syntax_incomplete"));
    assert_eq!(value["connectionsOpened"], false);
    assert_eq!(value["refresh"]["status"], "not-run");
}

#[test]
fn local_file_missing_and_wrong_type_are_distinct_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let missing = temp.path().join("missing.csv");
    replace_source(
        &project,
        "FactSales",
        Some(&format!(
            "Csv.Document(File.Contents(\"{}\"), [Delimiter=\",\"])",
            missing.display()
        )),
    );
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert!(has_code(&value, "rebindCheck.local_path_missing"));

    let wrong_type = temp.path().join("folder.csv");
    fs::create_dir(&wrong_type).expect("wrong type directory");
    replace_source(
        &project,
        "FactSales",
        Some(&format!(
            "Csv.Document(File.Contents(\"{}\"), [Delimiter=\",\"])",
            wrong_type.display()
        )),
    );
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert!(has_code(&value, "rebindCheck.local_path_unreadable"));
}

#[test]
fn existing_local_file_and_folder_sources_are_probed_without_reading_rows() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let file = temp.path().join("sales.csv");
    fs::write(&file, "synthetic,not-a-refresh\n").expect("write synthetic file");
    replace_source(
        &project,
        "FactSales",
        Some(&format!(
            "Csv.Document(File.Contents(\"{}\"), [Delimiter=\",\"])",
            file.display()
        )),
    );
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 0);
    assert_eq!(value["partitions"][0]["sourceKind"], "externalFile");
    assert_eq!(value["partitions"][0]["paths"][0]["exists"], true);
    assert_eq!(value["partitions"][0]["paths"][0]["readable"], true);
    assert_eq!(value["partitions"][0]["paths"][0]["status"], "ok");

    let folder = temp.path().join("sales-folder");
    fs::create_dir(&folder).expect("create synthetic folder");
    replace_source(
        &project,
        "FactSales",
        Some(&format!("Folder.Files(\"{}\")", folder.display())),
    );
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 0);
    assert_eq!(value["partitions"][0]["paths"][0]["kind"], "folder");
    assert_eq!(value["partitions"][0]["paths"][0]["readable"], true);
}

#[test]
fn sharepoint_source_requires_credential_free_https_site_shape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    replace_source(
        &project,
        "FactSales",
        Some("SharePoint.Files(\"http://contoso.sharepoint.com/sites/Sales\", [ApiVersion = 15])"),
    );
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert_eq!(value["partitions"][0]["sourceKind"], "sharePointFiles");
    assert!(has_code(&value, "rebindCheck.source_syntax_incomplete"));
}

#[test]
fn unknown_source_is_refused_instead_of_being_treated_as_materialized() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    replace_source(&project, "FactSales", Some("Table.FromRecords({})"));
    let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert_eq!(value["partitions"][0]["sourceKind"], "unknown");
    assert!(has_code(&value, "rebindCheck.source_unrecognized"));
}

#[test]
fn supported_live_connectors_are_checked_syntactically_without_network_access() {
    let cases = [
        ("sqlDatabase", "Sql.Database(\"server\", \"Sales\")"),
        (
            "postgresqlDatabase",
            "PostgreSQL.Database(\"server\", \"Sales\")",
        ),
        (
            "odbcDataSource",
            "Odbc.DataSource(\"dsn=SalesDsn\", [HierarchicalNavigation = true])",
        ),
        (
            "sharePointFiles",
            "SharePoint.Files(\"https://contoso.sharepoint.com/sites/Sales\", [ApiVersion = 15])",
        ),
    ];
    for (source_kind, source) in cases {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold_sales(temp.path());
        replace_source(&project, "FactSales", Some(source));
        let (code, value, _) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
        assert_eq!(code, 0, "{source_kind}: {}", value);
        assert_eq!(value["partitions"][0]["sourceKind"], source_kind);
        assert_eq!(value["partitions"][0]["state"], "materialized");
        assert_eq!(value["connectionsOpened"], false);
        assert_eq!(value["refresh"]["performed"], false);
    }
}

#[test]
fn top_level_handoff_rebind_check_alias_matches_nested_dispatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let nested = run_powerbi(&["handoff", "rebind-check", project_arg(&project), "--json"]);
    let alias = run_powerbi(&["handoff-rebind-check", project_arg(&project), "--json"]);
    assert_eq!(nested.code, 10);
    assert_eq!(alias.code, 10);
    assert_eq!(stdout_json(&nested), stdout_json(&alias));
}

#[test]
fn all_partitions_materialized_is_deterministic_and_reports_validation_refresh_boundary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    for table in ["FactSales", "DimCustomer", "DimDate"] {
        replace_source(
            &project,
            table,
            Some("Sql.Database(\"server.example.internal\", \"Sales\")"),
        );
    }
    let (first_code, first, first_stdout) = check(&project, &[]);
    let (second_code, second, second_stdout) = check(&project, &[]);
    assert_eq!(first_code, 0);
    assert_eq!(second_code, 0);
    assert_eq!(first, second);
    assert_eq!(first_stdout, second_stdout);
    assert_eq!(first["ok"], true);
    assert_eq!(first["status"], "safe");
    assert_eq!(first["counts"]["partitions"], 3);
    assert_eq!(first["counts"]["materializedPartitions"], 3);
    assert_eq!(first["counts"]["unresolvedPartitions"], 0);
    assert_eq!(first["validation"]["strict"], true);
    assert_eq!(first["validation"]["ok"], true);
    assert_eq!(first["refresh"]["status"], "not-run");
    assert_eq!(first["refresh"]["connectionOpened"], false);
    assert!(
        first["next"]
            .as_array()
            .expect("next")
            .iter()
            .any(|command| command
                .as_str()
                .is_some_and(|command| command.contains("desktop open")))
    );
    assert_json_snapshot(
        "handoff-rebind-check-safe",
        &serde_json::json!({
            "schema": first["schema"],
            "ok": first["ok"],
            "status": first["status"],
            "offline": first["offline"],
            "credentialsEmbedded": first["credentialsEmbedded"],
            "connectionsOpened": first["connectionsOpened"],
            "counts": first["counts"],
            "partitions": first["partitions"]
                .as_array()
                .expect("partitions")
                .iter()
                .map(|partition| serde_json::json!({
                    "handle": partition["handle"],
                    "table": partition["table"],
                    "partition": partition["partition"],
                    "state": partition["state"],
                    "materialized": partition["materialized"],
                    "resolved": partition["resolved"],
                    "sourceKind": partition["sourceKind"],
                    "findings": partition["findings"]
                }))
                .collect::<Vec<_>>(),
            "refresh": {
                "requested": first["refresh"]["requested"],
                "performed": first["refresh"]["performed"],
                "available": first["refresh"]["available"],
                "status": first["refresh"]["status"],
                "connectionOpened": first["refresh"]["connectionOpened"]
            }
        }),
    );
}

#[test]
fn credential_like_partition_text_is_redacted_from_rebind_check_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    replace_source(
        &project,
        "FactSales",
        Some("Sql.Database(\"server\", \"Sales\", [Password=\"SuperSecret123!\"])"),
    );
    let (code, value, stdout) = check(&project, &["--partition", "partition:FactSales:FactSales"]);
    assert_eq!(code, 10);
    assert!(has_code(&value, "partition.credential_like_text"));
    assert!(!stdout.contains("SuperSecret123!"));
    assert_eq!(value["credentialsEmbedded"], false);
    assert_eq!(value["connectionsOpened"], false);
}
