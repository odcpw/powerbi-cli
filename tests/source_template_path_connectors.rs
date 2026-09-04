mod common;

use common::{run_powerbi, scaffold_sales, stderr_json, stdout_json};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn project_arg(project: &Path) -> &str {
    project.to_str().expect("project path")
}

fn add_template(project: &Path, kind: &str, parameters: &[&str], mode: &[&str]) -> Value {
    let mut args = vec![
        "source-template",
        "add",
        "--project",
        project_arg(project),
        "--table",
        "FactSales",
        "--kind",
        kind,
    ];
    args.extend_from_slice(parameters);
    args.extend_from_slice(mode);
    args.push("--json");
    let output = run_powerbi(&args);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    stdout_json(&output)
}

fn apply_template(project: &Path, parameters: &[&str], mode: &[&str]) -> Value {
    let mut args = vec![
        "source-template",
        "apply",
        "--project",
        project_arg(project),
        "--handle",
        "source-template:FactSales:FactSales",
    ];
    args.extend_from_slice(parameters);
    args.extend_from_slice(mode);
    args.push("--json");
    let output = run_powerbi(&args);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    stdout_json(&output)
}

fn fact_sales_tmdl(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("FactSales.tmdl")
}

#[test]
fn csv_template_dry_run_and_apply_are_typed_deterministic_and_work_safe() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let store = project.join(".powerbi-cli").join("source-templates.json");

    let first = add_template(
        &project,
        "csv",
        &[
            "--file",
            "<file.csv>",
            "--delimiter",
            ";",
            "--encoding",
            "65001",
            "--has-header",
            "true",
        ],
        &["--dry-run"],
    );
    let second = add_template(
        &project,
        "csv",
        &[
            "--file",
            "<file.csv>",
            "--delimiter",
            ";",
            "--encoding",
            "65001",
            "--has-header",
            "true",
        ],
        &["--dry-run"],
    );
    assert_eq!(first, second);
    assert_eq!(first["schema"], "powerbi-cli.source-template.mutation.v1");
    assert_eq!(first["mode"], "dry-run");
    assert!(!store.exists(), "dry-run must not write the template store");
    assert!(
        first["changes"][0]["after"]["mTemplate"]
            .as_str()
            .expect("M template")
            .contains("Csv.Document(File.Contents")
    );

    add_template(&project, "csv", &["--file", "<file.csv>"], &["--in-place"]);
    let plan = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project_arg(&project),
        "--allow-unmapped",
        "--json",
    ]);
    assert_eq!(plan.code, 0, "stderr: {}", plan.stderr);
    assert!(
        stdout_json(&plan)["plans"]
            .as_array()
            .expect("plans")
            .iter()
            .any(|item| item["mTemplate"]
                .as_str()
                .unwrap_or_default()
                .contains("Csv.Document"))
    );
    let applied = apply_template(
        &project,
        &[
            "--file",
            "C:\\Data\\sales.csv",
            "--delimiter",
            ",",
            "--encoding",
            "65001",
            "--has-header",
            "false",
        ],
        &["--in-place"],
    );
    assert_eq!(applied["connection"]["kind"], "csv");
    assert_eq!(applied["credentialsEmbedded"], false);
    assert_eq!(applied["requiresDesktopAuthentication"], false);
    let tmdl = fs::read_to_string(fact_sales_tmdl(&project)).expect("FactSales TMDL");
    assert!(tmdl.contains("Csv.Document(File.Contents"));
    assert!(tmdl.contains("Table.RenameColumns(Source"));
    assert!(tmdl.contains("Table.TransformColumnTypes(NamedColumns"));

    let handoff = run_powerbi(&[
        "handoff",
        "check",
        project_arg(&project),
        "--target",
        "work",
        "--json",
    ]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(stdout_json(&handoff)["safeForWorkHandoff"], true);
}

#[test]
fn folder_template_out_dir_and_apply_emit_folder_files_and_explicit_types() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let staged = temp.path().join("folder-staged");
    let staged_arg = project_arg(&staged).to_string();
    let added = add_template(
        &source,
        "folder",
        &["--path", "<folder>", "--pattern", "*.csv"],
        &["--out-dir", &staged_arg],
    );
    assert_eq!(added["mode"], "out-dir");
    assert!(!source.join(".powerbi-cli/source-templates.json").exists());
    assert!(staged.join(".powerbi-cli/source-templates.json").exists());

    let plan = run_powerbi(&[
        "handoff",
        "rebind-plan",
        &staged_arg,
        "--allow-unmapped",
        "--json",
    ]);
    assert_eq!(plan.code, 0, "stderr: {}", plan.stderr);
    assert_eq!(stdout_json(&plan)["counts"]["mappedPartitions"], 1);

    let applied = apply_template(
        &staged,
        &["--path", "C:\\Data\\Exports", "--pattern", "*.csv"],
        &["--in-place"],
    );
    let source_m = applied["changes"][0]["afterSource"]
        .as_str()
        .expect("after source");
    assert!(source_m.contains("Folder.Files"));
    assert!(source_m.contains("Text.EndsWith([Name], \".csv\""));
    assert!(source_m.contains("Table.TransformColumnTypes(FilteredFiles"));
    let handoff = run_powerbi(&[
        "handoff",
        "check",
        &staged_arg,
        "--target",
        "work",
        "--json",
    ]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(stdout_json(&handoff)["safeForWorkHandoff"], true);
}

#[test]
fn sharepoint_template_apply_emits_typed_m_and_preserves_replacement_gate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    add_template(
        &project,
        "sharepoint",
        &[
            "--site-url",
            "<siteUrl>",
            "--library",
            "<library>",
            "--path",
            "<path>",
        ],
        &["--in-place"],
    );
    let plan = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project_arg(&project),
        "--allow-unmapped",
        "--json",
    ]);
    assert_eq!(plan.code, 0, "stderr: {}", plan.stderr);
    assert!(
        stdout_json(&plan)["plans"]
            .as_array()
            .expect("plans")
            .iter()
            .any(|item| item["mTemplate"]
                .as_str()
                .unwrap_or_default()
                .contains("SharePoint.Files"))
    );
    let parameters = [
        "--site-url",
        "https://contoso.sharepoint.com/sites/Finance",
        "--library",
        "Documents",
        "--path",
        "Published/Exports",
    ];
    let first = apply_template(&project, &parameters, &["--in-place"]);
    let source_m = first["changes"][0]["afterSource"]
        .as_str()
        .expect("after source");
    assert!(source_m.contains("SharePoint.Files"));
    assert!(source_m.contains("/Documents/Published/Exports/"));
    assert!(source_m.contains("Table.TransformColumnTypes(SelectedFiles"));
    assert_eq!(first["requiresDesktopAuthentication"], true);
    let handoff = run_powerbi(&[
        "handoff",
        "check",
        project_arg(&project),
        "--target",
        "work",
        "--json",
    ]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(stdout_json(&handoff)["safeForWorkHandoff"], true);

    let mut args = vec![
        "source-template",
        "apply",
        "--project",
        project_arg(&project),
        "--handle",
        "source-template:FactSales:FactSales",
    ];
    args.extend_from_slice(&parameters);
    args.extend_from_slice(&["--dry-run", "--json"]);
    let refused = run_powerbi(&args);
    assert_ne!(refused.code, 0);
    let error = stderr_json(&refused);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("only replaces a safe generated dummy partition by default")
    );

    args.splice(
        args.len() - 2..args.len() - 2,
        [
            "--replace-existing",
            "--confirm",
            "partition:FactSales:FactSales",
        ],
    );
    let confirmed = run_powerbi(&args);
    assert_eq!(confirmed.code, 0, "stderr: {}", confirmed.stderr);
    assert_eq!(
        stdout_json(&confirmed)["replacementMode"],
        "confirmed-existing"
    );
}

#[test]
fn path_connector_templates_refuse_malformed_credential_like_and_cross_kind_inputs() {
    let cases: &[(&str, &[&str], &str)] = &[
        ("csv", &["--file", "sales.xlsx"], "must end in"),
        ("csv", &["--delimiter", "||"], "exactly one character"),
        ("csv", &["--encoding", "0"], "greater than zero"),
        ("csv", &["--has-header", "yes"], "true or false"),
        ("folder", &["--pattern", "20??/*.csv"], "exact file name"),
        (
            "sharepoint",
            &["--site-url", "http://contoso.sharepoint.com/sites/Finance"],
            "must use https://",
        ),
        (
            "sharepoint",
            &[
                "--site-url",
                "https://contoso.sharepoint.com/sites/Finance?token=secret",
            ],
            "credential-free",
        ),
        ("sharepoint", &["--path", "../Finance"], "without traversal"),
        ("folder", &["--server", "db"], "not valid"),
    ];
    for (kind, parameters, message) in cases {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = scaffold_sales(temp.path());
        let mut args = vec![
            "source-template",
            "add",
            "--project",
            project_arg(&project),
            "--table",
            "FactSales",
            "--kind",
            kind,
        ];
        args.extend_from_slice(parameters);
        args.extend_from_slice(&["--dry-run", "--json"]);
        let output = run_powerbi(&args);
        assert_ne!(output.code, 0, "case {kind:?} {parameters:?}");
        let error = stderr_json(&output);
        assert_eq!(error["error"]["code"], "invalid_args");
        assert!(
            error["error"]["message"]
                .as_str()
                .expect("message")
                .contains(message),
            "case {kind:?} {parameters:?}: {error}"
        );
        assert!(error["error"]["hint"].is_string());
        assert!(
            error["error"]["suggestedCommands"]
                .as_array()
                .is_some_and(|commands| !commands.is_empty())
        );
        assert!(!project.join(".powerbi-cli/source-templates.json").exists());
    }
}
