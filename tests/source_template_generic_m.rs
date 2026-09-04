mod common;

use common::{RunOutput, run_powerbi, run_powerbi_owned, scaffold_sales, stderr_json, stdout_json};
use serde_json::Value;
use std::fs;
use std::path::Path;

fn project_arg(project: &Path) -> String {
    project.to_str().expect("project path").to_string()
}

fn add_template(project: &Path, table: &str, m_template: &str, mode: &str) -> RunOutput {
    let args = vec![
        "source-template".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project_arg(project),
        "--table".to_string(),
        table.to_string(),
        "--kind".to_string(),
        "generic-m".to_string(),
        "--m-template".to_string(),
        m_template.to_string(),
        mode.to_string(),
        "--json".to_string(),
    ];
    run_powerbi_owned(&args)
}

fn assert_recovery_contract(output: &RunOutput, message_fragment: &str) -> Value {
    assert_eq!(output.code, 2, "stderr: {}", output.stderr);
    let error = stderr_json(output)["error"].clone();
    assert_eq!(error["code"], "invalid_args");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains(message_fragment),
        "message did not contain {message_fragment:?}: {error}"
    );
    assert!(
        error["hint"].as_str().is_some_and(|hint| !hint.is_empty()),
        "refusal must include a hint: {error}"
    );
    let pointer = error["pointer"].as_str().expect("M pointer");
    assert!(
        pointer.starts_with("/mTemplate/"),
        "unexpected pointer: {pointer}"
    );
    let suggested = error["suggestedCommands"]
        .as_array()
        .expect("suggested commands");
    assert!(!suggested.is_empty());
    assert!(suggested.iter().all(|command| {
        command
            .as_str()
            .is_some_and(|command| command.starts_with("powerbi-cli "))
    }));
    error
}

#[test]
fn generic_m_accepts_each_allowlisted_root_and_is_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let cases = [
        (
            "FactSales",
            "let Source = Sql.Database(\"{{powerbi-cli.placeholder:server}}\", \"{{powerbi-cli.placeholder:database}}\"), Navigation = Source{[Schema=\"dbo\", Item=\"FactSales\"]}[Data] in Navigation",
        ),
        (
            "DimDate",
            "let Source = PostgreSQL.Database(\"{{powerbi-cli.placeholder:server}}\", \"{{powerbi-cli.placeholder:database}}\"), Navigation = Source{[Schema=\"public\", Item=\"DimDate\"]}[Data] in Navigation",
        ),
        (
            "DimCustomer",
            "let Source = Odbc.DataSource(\"{{powerbi-cli.placeholder:dsn}}\", [HierarchicalNavigation = true]) in Source",
        ),
        (
            "FactSales",
            "let Source = Excel.Workbook(File.Contents(\"{{powerbi-cli.resourcePath:workbook}}\"), null, true), Headers = Table.PromoteHeaders(Source, [PromoteAllScalars = true]) in Headers",
        ),
        (
            "DimDate",
            "let Source = Csv.Document(File.Contents(\"{{powerbi-cli.resourcePath:file}}\"), [Delimiter=\",\", Encoding=65001]) in Source",
        ),
        (
            "DimCustomer",
            "let Source = Folder.Files(\"{{powerbi-cli.placeholder:folder}}\"), Filtered = Table.SelectRows(Source, each Text.EndsWith([Name], \".csv\", Comparer.OrdinalIgnoreCase)) in Filtered",
        ),
        (
            "FactSales",
            "let Source = SharePoint.Files(\"https://contoso.sharepoint.com/sites/Finance\", [ApiVersion=15]), Selected = Table.SelectRows(Source, each Text.Contains([Folder Path], \"/Documents/Exports/\", Comparer.OrdinalIgnoreCase)) in Selected",
        ),
    ];

    for (table, m_template) in cases {
        let output = add_template(&project, table, m_template, "--dry-run");
        assert_eq!(output.code, 0, "{table}: {}", output.stderr);
        let value = stdout_json(&output);
        assert_eq!(value["schema"], "powerbi-cli.source-template.mutation.v1");
        assert_eq!(value["mode"], "dry-run");
        assert_eq!(value["changes"][0]["after"]["kind"], "generic-m");
        assert_eq!(value["changes"][0]["after"]["mTemplate"], m_template);
        assert_eq!(
            value["changes"][0]["after"]["safety"]["credentialFree"],
            true
        );
    }

    let sql = cases[0].1;
    let first = add_template(&project, "FactSales", sql, "--dry-run");
    let second = add_template(&project, "FactSales", sql, "--dry-run");
    assert_eq!(
        first.stdout, second.stdout,
        "generic M dry-runs must be byte deterministic"
    );
    assert!(
        !project
            .join(".powerbi-cli")
            .join("source-templates.json")
            .exists(),
        "dry-run must not persist generic M metadata"
    );
}

#[test]
fn generic_m_file_input_and_apply_preserve_safety_boundary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let m_file = temp.path().join("template.m");
    let m_template = "let Source = Sql.Database(\"{{powerbi-cli.placeholder:server}}\", \"{{powerbi-cli.placeholder:database}}\") in Source";
    fs::write(&m_file, format!("{m_template}\n")).expect("M file");
    let staged = temp.path().join("staged");
    let args = vec![
        "source-template".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project_arg(&project),
        "--table".to_string(),
        "DimDate".to_string(),
        "--kind".to_string(),
        "m".to_string(),
        "--m-file".to_string(),
        m_file.to_str().expect("M path").to_string(),
        "--out-dir".to_string(),
        project_arg(&staged),
        "--json".to_string(),
    ];
    let added = run_powerbi_owned(&args);
    assert_eq!(added.code, 0, "stderr: {}", added.stderr);
    let added_json = stdout_json(&added);
    assert_eq!(added_json["mode"], "out-dir");
    assert_eq!(added_json["changes"][0]["after"]["mTemplate"], m_template);
    assert!(!project.join(".powerbi-cli/source-templates.json").exists());
    let staged_store = staged.join(".powerbi-cli/source-templates.json");
    assert!(staged_store.exists());
    assert!(
        fs::read_to_string(staged_store)
            .expect("staged store")
            .contains("generic-m")
    );

    let concrete = "let Source = Sql.Database(\"work-db\", \"Sales\"), Navigation = Source{[Schema=\"dbo\", Item=\"DimDate\"]}[Data] in Navigation";
    let apply_args = vec![
        "source-template".to_string(),
        "apply".to_string(),
        "--project".to_string(),
        project_arg(&staged),
        "--handle".to_string(),
        "source-template:DimDate:DimDate".to_string(),
        "--m-template".to_string(),
        concrete.to_string(),
        "--dry-run".to_string(),
        "--json".to_string(),
    ];
    let applied = run_powerbi_owned(&apply_args);
    assert_eq!(applied.code, 0, "stderr: {}", applied.stderr);
    let applied_json = stdout_json(&applied);
    assert_eq!(applied_json["connection"]["kind"], "generic-m");
    assert_eq!(applied_json["requiresDesktopAuthentication"], true);
    assert_eq!(applied_json["credentialsEmbedded"], false);
    assert_eq!(applied_json["projectModified"], false);
    assert_eq!(applied_json["changes"][0]["afterSource"], concrete);
}

#[test]
fn generic_m_rejects_credentials_paths_unknown_calls_and_computed_connectors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let cases = [
        (
            "let Source = Web.Contents(\"https://evil.invalid\") in Source",
            "closed root allowlist",
        ),
        (
            "let Source = Sql.Database(\"Password=secret\", \"Sales\") in Source",
            "credential-like",
        ),
        (
            "let Source = Sql.Database(\"server\", \"Sales\"), F = ([Run = Sql.Database][Run])(\"other\", \"db\") in Source",
            "computed or postfix",
        ),
        (
            "let Source = Excel.Workbook(File.Contents(\"C:\\\\data\\\\book.xlsx\"), null, true) in Source",
            "hard-coded file",
        ),
        (
            "let Source = Sql.Database(\"prod-{{powerbi-cli.placeholder:server}}\", \"Sales\") in Source",
            "embedded in other string",
        ),
        (
            "let Source = Sql.Database(\"server\", \"Sales\"), Leak = Mystery.Cloud(\"x\") in Source",
            "outside the closed M grammar",
        ),
        (
            "let Source = Table.FromRecords({}) in Source",
            "closed root allowlist",
        ),
    ];

    for (m_template, message) in cases {
        let output = add_template(&project, "FactSales", m_template, "--in-place");
        let error = assert_recovery_contract(&output, message);
        assert!(
            !output.stderr.contains("secret"),
            "credential text leaked in diagnostics: {}",
            output.stderr
        );
        assert_eq!(error["pointer"].as_str().unwrap().split('/').count(), 3);
    }
    assert!(
        !project
            .join(".powerbi-cli")
            .join("source-templates.json")
            .exists(),
        "all refused generic M templates must leave the sidecar absent"
    );

    let refused_out = temp.path().join("refused-generic-m");
    let refused_args = vec![
        "source-template".to_string(),
        "add".to_string(),
        "--project".to_string(),
        project_arg(&project),
        "--table".to_string(),
        "FactSales".to_string(),
        "--kind".to_string(),
        "m".to_string(),
        "--m-template".to_string(),
        "let Source = Web.Contents(\"https://evil.invalid\") in Source".to_string(),
        "--out-dir".to_string(),
        project_arg(&refused_out),
        "--json".to_string(),
    ];
    let refused = run_powerbi_owned(&refused_args);
    assert_recovery_contract(&refused, "closed root allowlist");
    assert!(
        !refused_out.exists(),
        "invalid M must be rejected before staging"
    );
}

#[test]
fn generic_m_source_template_capability_advertises_kind_and_flags() {
    let capabilities = run_powerbi(&["capabilities", "--json", "--for", "source-template"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let value = stdout_json(&capabilities);
    let commands = value["commands"].as_array().expect("commands");
    let add = commands
        .iter()
        .find(|command| command["path"] == "source-template add")
        .expect("add command");
    assert!(
        add["usage"]
            .as_str()
            .unwrap_or_default()
            .contains("generic-m")
    );
    assert!(
        add["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--m-template <M-expression>")
    );
    let features = run_powerbi(&["features", "list", "--for", "source-template", "--json"]);
    assert_eq!(features.code, 0, "stderr: {}", features.stderr);
    assert_eq!(
        stdout_json(&features)["features"][0]["supportedKinds"]
            .as_array()
            .expect("supported kinds")
            .iter()
            .any(|kind| kind == "generic-m"),
        true
    );
}
