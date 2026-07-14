use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct RunOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_powerbi(args: &[&str]) -> RunOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
        .output()
        .expect("run powerbi-cli binary");
    RunOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

fn scaffold_sales_project(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

fn fact_sales_tmdl(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("FactSales.tmdl")
}

#[test]
fn model_partitions_list_show_and_inspect_dummy_sources() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    let list = run_powerbi(&[
        "model",
        "partitions",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(
        list_json["schema"],
        Value::from("powerbi-cli.model.partitions.list.v1")
    );
    assert_eq!(list_json["counts"]["partitions"], Value::from(3));
    assert_eq!(list_json["counts"]["safePartitions"], Value::from(3));
    let fact_partition = list_json["partitions"]
        .as_array()
        .expect("partitions")
        .iter()
        .find(|partition| partition["handle"] == "partition:FactSales:FactSales")
        .expect("FactSales partition");
    assert_eq!(fact_partition["sourceKind"], Value::from("dummyMTable"));
    assert_eq!(
        fact_partition["offlineSafety"]["safeForHome"],
        Value::Bool(true)
    );
    assert!(
        fact_partition["sourcePreview"]
            .as_str()
            .unwrap_or_default()
            .contains("#table(")
    );

    let show = run_powerbi(&[
        "model",
        "partition",
        "show",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "FactSales",
        "--json",
    ]);
    assert_eq!(show.code, 0, "stderr: {}", show.stderr);
    let show_json = stdout_json(&show);
    assert_eq!(
        show_json["schema"],
        Value::from("powerbi-cli.model.partitions.show.v1")
    );
    assert_eq!(
        show_json["partition"]["handle"],
        Value::from("partition:FactSales:FactSales")
    );
    assert!(
        show_json["partition"]["source"]
            .as_str()
            .unwrap_or_default()
            .contains("#table(")
    );
    assert_eq!(show_json["partition"]["sourceIncluded"], Value::Bool(false));
    assert_eq!(show_json["block"], Value::Null);
    assert!(show_json["partition"]["lineRange"]["start"].is_number());

    let raw_show = run_powerbi(&[
        "model",
        "partition",
        "show",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "FactSales",
        "--include-source",
        "--json",
    ]);
    assert_eq!(raw_show.code, 0, "stderr: {}", raw_show.stderr);
    let raw_show_json = stdout_json(&raw_show);
    assert_eq!(
        raw_show_json["partition"]["sourceIncluded"],
        Value::Bool(true)
    );
    assert!(
        raw_show_json["block"]
            .as_str()
            .unwrap_or_default()
            .contains("partition FactSales = m")
    );

    let inspect = run_powerbi(&["inspect", "--deep", project_arg, "--json"]);
    assert_eq!(inspect.code, 0, "stderr: {}", inspect.stderr);
    let inspect_json = stdout_json(&inspect);
    assert!(
        inspect_json["deep"]["model"]["tables"]
            .as_array()
            .expect("tables")
            .iter()
            .flat_map(|table| table["partitions"].as_array().into_iter().flatten())
            .any(|partition| {
                partition["handle"] == "partition:FactSales:FactSales"
                    && partition["sourceKind"] == "dummyMTable"
                    && partition["offlineSafety"]["status"] == "safe"
            })
    );
}

#[test]
fn handoff_check_passes_scaffolded_project_and_accepts_project_flag() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    let check = run_powerbi(&["handoff", "check", "--project", project_arg, "--json"]);
    assert_eq!(check.code, 0, "stderr: {}", check.stderr);
    let check_json = stdout_json(&check);
    assert_eq!(
        check_json["schema"],
        Value::from("powerbi-cli.handoff.check.v1")
    );
    assert_eq!(check_json["ok"], Value::Bool(true));
    assert_eq!(check_json["safeForOfflineHandoff"], Value::Bool(true));
    assert_eq!(check_json["counts"]["safePartitions"], Value::from(3));
    assert_eq!(check_json["counts"]["errors"], Value::from(0));

    let alias = run_powerbi(&["handoff-check", project_arg, "--json"]);
    assert_eq!(alias.code, 0, "stderr: {}", alias.stderr);
    assert_eq!(stdout_json(&alias)["ok"], Value::Bool(true));
}

#[test]
fn handoff_check_fails_unsafe_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    fs::write(project.join("unsafe.pbix"), b"not a real pbix").expect("write pbix");
    let pbi_dir = project.join(".pbi");
    fs::create_dir_all(&pbi_dir).expect("create .pbi");
    fs::write(pbi_dir.join("cache.abf"), b"cache").expect("write cache");
    fs::write(project.join("dummy.csv"), b"real,data\n1,2\n").expect("write csv");

    let check = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(check.code, 10);
    let check_json = stdout_json(&check);
    assert_eq!(check_json["ok"], Value::Bool(false));
    let codes = check_json["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["code"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"handoff.binary_powerbi_file"));
    assert!(codes.contains(&"handoff.powerbi_cache_folder"));
    assert!(codes.contains(&"handoff.embedded_data_file"));
}

#[test]
fn handoff_check_fails_external_connector_partition() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let path = fact_sales_tmdl(&project);
    let text = fs::read_to_string(&path).expect("FactSales.tmdl before");
    let source_start = text.find("        source =").expect("source block");
    let replacement = r#"        source =
            let
                Source = Sql.Database("corp-sql", "Claims", [Password = "secret"])
            in
                Source

"#;
    fs::write(&path, format!("{}{}", &text[..source_start], replacement))
        .expect("write unsafe source");

    let list = run_powerbi(&[
        "model",
        "partitions",
        "list",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(list_json["partitions"][0]["sourceKind"], "sqlDatabase");
    assert_eq!(
        list_json["partitions"][0]["offlineSafety"]["safeForHome"],
        Value::Bool(false)
    );

    let check = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(check.code, 10);
    let check_json = stdout_json(&check);
    assert_eq!(check_json["ok"], Value::Bool(false));
    let findings = check_json["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|finding| {
        finding["code"] == "partition.real_connector.sql"
            && finding["handle"] == "partition:FactSales:FactSales"
    }));
    assert!(findings.iter().any(|finding| {
        finding["code"] == "partition.credential_like_text"
            && finding["handle"] == "partition:FactSales:FactSales"
    }));
}

#[test]
fn handoff_does_not_certify_arbitrary_table_substrings_or_mismatched_columns() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let path = fact_sales_tmdl(&project);
    let text = fs::read_to_string(&path).expect("FactSales.tmdl before");
    let source_start = text.find("        source =").expect("source block");
    let replacement = r#"        source =
            let
                Note = "the text #table( is not a safety marker",
                Source = #table(type table [Name = text], {{"Alice Smith"}})
            in
                Source

"#;
    fs::write(&path, format!("{}{}", &text[..source_start], replacement))
        .expect("write unverified table source");

    let list = run_powerbi(&[
        "model",
        "partitions",
        "list",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let partition = &stdout_json(&list)["partitions"][0];
    assert_eq!(partition["sourceKind"], "unknown");
    assert_eq!(partition["offlineSafety"]["status"], "review");
    assert!(
        partition["offlineSafety"]["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "partition.dummy_table_shape_unverified")
    );

    let handoff = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(handoff.code, 10);
    let handoff_json = stdout_json(&handoff);
    assert_eq!(handoff_json["status"], "unsafe");
    assert_eq!(handoff_json["safeForOfflineHandoff"], Value::Bool(false));
}

#[test]
fn handoff_marks_structurally_valid_pii_suspect_rows_for_review() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let path = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("DimCustomer.tmdl");
    let text = fs::read_to_string(&path).expect("DimCustomer.tmdl before");
    fs::write(&path, text.replace("Sample Customer", "Alice Smith"))
        .expect("write PII-suspect row");

    let list = run_powerbi(&[
        "model",
        "partitions",
        "list",
        "--project",
        project_arg,
        "--table",
        "DimCustomer",
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let partition = &stdout_json(&list)["partitions"][0];
    assert_eq!(partition["sourceKind"], "dummyMTable");
    assert_eq!(partition["offlineSafety"]["status"], "review");
    assert!(
        partition["offlineSafety"]["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "partition.pii_suspect_literal")
    );

    let handoff = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(handoff.code, 10);
    let handoff_json = stdout_json(&handoff);
    assert_eq!(handoff_json["status"], "review");
    assert_eq!(handoff_json["safeForOfflineHandoff"], Value::Bool(false));
}

#[test]
fn partition_credentials_are_redacted_and_block_raw_output_and_runbooks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let path = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("DimCustomer.tmdl");
    let text = fs::read_to_string(&path).expect("DimCustomer.tmdl before");
    fs::write(&path, text.replace("Sample Customer", "Password=hunter2"))
        .expect("write credential row");

    let list = run_powerbi(&[
        "model",
        "partitions",
        "list",
        "--project",
        project_arg,
        "--table",
        "DimCustomer",
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    assert!(!list.stdout.contains("hunter2"));
    assert!(list.stdout.contains("Password=***"));
    let partition = &stdout_json(&list)["partitions"][0];
    assert_eq!(partition["offlineSafety"]["status"], "unsafe");

    let raw = run_powerbi(&[
        "model",
        "partitions",
        "show",
        "--project",
        project_arg,
        "--table",
        "DimCustomer",
        "--name",
        "DimCustomer",
        "--include-source",
        "--json",
    ]);
    assert_eq!(raw.code, 2);
    assert!(!raw.stderr.contains("hunter2"));
    assert!(
        stderr_json(&raw)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("--include-source is refused")
    );

    let runbook = temp.path().join("unsafe-runbook.md");
    let plan = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project_arg,
        "--allow-unmapped",
        "--out",
        runbook.to_str().expect("runbook"),
        "--json",
    ]);
    assert_eq!(plan.code, 10);
    assert!(!runbook.exists());
    assert!(!plan.stdout.contains("hunter2"));
    let plan_json = stdout_json(&plan);
    assert_eq!(plan_json["materializationBlocked"], Value::Bool(true));
    assert_eq!(
        plan_json["materializationBlockReasons"]["partitionCredential"],
        Value::Bool(true)
    );
    assert_eq!(plan_json["runbookWritten"], Value::Bool(false));
}

#[test]
fn handoff_scans_markdown_and_json_text_for_credentials_and_pii() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let notes = project.join("notes.md");
    fs::write(&notes, "Authorization: Bearer abc.def.ghi\n").expect("credential notes");

    let unsafe_check = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(unsafe_check.code, 10);
    let unsafe_json = stdout_json(&unsafe_check);
    assert_eq!(unsafe_json["status"], "unsafe");
    assert!(
        unsafe_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "handoff.credential_like_text")
    );

    fs::remove_file(notes).expect("remove notes");
    fs::write(
        project.join("review.json"),
        serde_json::to_string(&json!({"rows": [{"Name": "Alice Smith"}]})).expect("review json"),
    )
    .expect("PII review json");
    let review_check = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(review_check.code, 10);
    let review_json = stdout_json(&review_check);
    assert_eq!(review_json["status"], "review");
    assert_eq!(review_json["safeForOfflineHandoff"], Value::Bool(false));
    assert!(
        review_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "handoff.pii_suspect_text")
    );
}

#[test]
fn capabilities_advertise_partitions_handoff_and_empty_filter_hints() {
    let output = run_powerbi(&["capabilities", "--json", "--for", "partition"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let paths = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(paths.contains(&"model partitions list"));
    assert!(paths.contains(&"model partitions show"));
    assert!(paths.contains(&"handoff check"));
    assert!(paths.contains(&"handoff rebind-plan"));

    let handoff = run_powerbi(&["capabilities", "--json", "--for", "handoff"]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert!(
        stdout_json(&handoff)["commands"]
            .as_array()
            .expect("handoff commands")
            .iter()
            .any(|command| command["path"] == "handoff check")
    );
    assert!(
        stdout_json(&handoff)["commands"]
            .as_array()
            .expect("handoff commands")
            .iter()
            .any(|command| command["path"] == "handoff rebind-plan")
    );
    let source = run_powerbi(&["capabilities", "--json", "--for", "source-template"]);
    assert_eq!(source.code, 0, "stderr: {}", source.stderr);
    let source_json = stdout_json(&source);
    let source_paths = source_json["commands"]
        .as_array()
        .expect("source commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(source_paths.contains(&"source-template list"));
    assert!(source_paths.contains(&"source-template show"));
    assert!(source_paths.contains(&"source-template add"));
    assert!(source_paths.contains(&"source-template apply"));
    let add_contract = source_json["commands"]
        .as_array()
        .expect("source commands")
        .iter()
        .find(|command| command["path"] == "source-template add")
        .expect("source-template add contract");
    assert!(
        add_contract["usage"]
            .as_str()
            .unwrap_or_default()
            .contains("sql|postgres|odbc")
    );
    assert!(
        add_contract["flags"]
            .as_array()
            .expect("source-template add flags")
            .iter()
            .any(|flag| flag == "--dsn <placeholder>")
    );

    let source_feature = run_powerbi(&["features", "list", "--for", "source-template", "--json"]);
    assert_eq!(source_feature.code, 0, "stderr: {}", source_feature.stderr);
    let source_feature_json = stdout_json(&source_feature);
    assert_eq!(source_feature_json["matchedFeatures"], Value::from(1));
    assert_eq!(
        source_feature_json["features"][0]["supportedKinds"],
        json!(["sql", "postgres", "odbc"])
    );

    let empty = run_powerbi(&["capabilities", "--json", "--for", "does-not-exist"]);
    assert_eq!(empty.code, 0, "stderr: {}", empty.stderr);
    let empty_json = stdout_json(&empty);
    assert_eq!(empty_json["matchedCommands"], Value::from(0));
    assert!(
        empty_json["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("No live command matched")
    );

    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let bad_handle = run_powerbi(&[
        "model",
        "partitions",
        "show",
        "--project",
        project_arg,
        "--handle",
        "bad",
        "--json",
    ]);
    assert_ne!(bad_handle.code, 0);
    assert!(stderr_json(&bad_handle)["error"]["suggestedCommands"].is_array());
}

#[test]
fn source_template_add_out_dir_feeds_rebind_plan_without_breaking_handoff_check() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let out_dir = temp.path().join("sales_with_template");
    let out_arg = out_dir.to_str().expect("out path");

    let dry_run = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "CorpSql",
        "--kind",
        "sql",
        "--server",
        "<server>",
        "--database",
        "<database>",
        "--schema",
        "dbo",
        "--object",
        "FactSales",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.source-template.mutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert!(
        !project
            .join(".powerbi-cli")
            .join("source-templates.json")
            .exists(),
        "dry-run must not write template store"
    );

    let add = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "CorpSql",
        "--kind",
        "sql",
        "--server",
        "<server>",
        "--database",
        "<database>",
        "--schema",
        "dbo",
        "--object",
        "FactSales",
        "--out-dir",
        out_arg,
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let add_json = stdout_json(&add);
    assert_eq!(
        add_json["target"]["handle"],
        "source-template:FactSales:CorpSql"
    );
    assert!(
        add_json["rebindPlanCommand"]
            .as_str()
            .unwrap()
            .contains("handoff rebind-plan")
    );
    assert!(
        out_dir
            .join(".powerbi-cli")
            .join("source-templates.json")
            .is_file()
    );

    let list_alias = run_powerbi(&["source-templates", "list", "--project", out_arg, "--json"]);
    assert_eq!(list_alias.code, 0, "stderr: {}", list_alias.stderr);
    assert_eq!(
        stdout_json(&list_alias)["counts"]["templates"],
        Value::from(1)
    );

    let show_alias = run_powerbi(&[
        "sourceTemplate",
        "show",
        "--project",
        out_arg,
        "--name",
        "CorpSql",
        "--json",
    ]);
    assert_eq!(show_alias.code, 0, "stderr: {}", show_alias.stderr);
    assert!(
        stdout_json(&show_alias)["sourceTemplate"]["mTemplate"]
            .as_str()
            .unwrap()
            .contains("Sql.Database(\"<server>\", \"<database>\")")
    );

    let plan = run_powerbi(&[
        "handoff",
        "rebind-plan",
        out_arg,
        "--allow-unmapped",
        "--json",
    ]);
    assert_eq!(plan.code, 0, "stderr: {}", plan.stderr);
    let plan_json = stdout_json(&plan);
    assert_eq!(
        plan_json["schema"],
        Value::from("powerbi-cli.handoff.rebind-plan.v1")
    );
    assert_eq!(plan_json["counts"]["mappedPartitions"], Value::from(1));
    assert_eq!(plan_json["counts"]["unmappedPartitions"], Value::from(2));
    assert!(
        plan_json["plans"]
            .as_array()
            .expect("plans")
            .iter()
            .any(
                |plan| plan["partitionHandle"] == "partition:FactSales:FactSales"
                    && plan["mTemplate"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("Sql.Database(\"<server>\", \"<database>\")")
            )
    );

    let check = run_powerbi(&["handoff", "check", out_arg, "--json"]);
    assert_eq!(check.code, 0, "stderr: {}", check.stderr);
    let check_json = stdout_json(&check);
    assert_eq!(check_json["ok"], Value::Bool(true));
    assert_eq!(check_json["counts"]["sourceTemplates"], Value::from(1));
}

#[test]
fn source_template_apply_materializes_a_live_connection_without_credentials() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let add = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--kind",
        "postgres",
        "--server",
        "<WORK_POSTGRES_HOST:PORT>",
        "--database",
        "<WORK_DATABASE>",
        "--schema",
        "public",
        "--object",
        "fact_accidents",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let before = fs::read_to_string(fact_sales_tmdl(&project)).expect("TMDL before apply");

    let missing_override = run_powerbi(&[
        "source-template",
        "apply",
        "--project",
        project_arg,
        "--handle",
        "source-template:FactSales:FactSales",
        "--server",
        "postgres.example.internal:55117",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(missing_override.code, 2);
    assert!(
        stderr_json(&missing_override)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("concrete --database")
    );

    let dry_run = run_powerbi(&[
        "source-template",
        "materialize",
        "--project",
        project_arg,
        "--handle",
        "source-template:FactSales:FactSales",
        "--server",
        "postgres.example.internal:55117",
        "--database",
        "safety_analytics",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.source-template.apply.v1")
    );
    assert_eq!(dry_json["projectModified"], Value::Bool(false));
    assert_eq!(dry_json["credentialsEmbedded"], Value::Bool(false));
    assert_eq!(dry_json["requiresDesktopAuthentication"], Value::Bool(true));
    assert!(
        dry_json["changes"][0]["afterSource"]
            .as_str()
            .unwrap_or_default()
            .contains(
                "PostgreSQL.Database(\"postgres.example.internal:55117\", \"safety_analytics\")"
            )
    );
    assert_eq!(
        fs::read_to_string(fact_sales_tmdl(&project)).expect("TMDL after dry run"),
        before
    );

    let live_project = temp.path().join("sales_live");
    let live_arg = live_project.to_str().expect("live output path");
    let apply = run_powerbi(&[
        "source-template",
        "apply",
        "--project",
        project_arg,
        "--handle",
        "source-template:FactSales:FactSales",
        "--server",
        "postgres.example.internal:55117",
        "--database",
        "safety_analytics",
        "--out-dir",
        live_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["projectModified"], Value::Bool(true));
    assert_eq!(apply_json["connection"]["kind"], "postgres");
    assert_eq!(
        apply_json["connection"]["parameters"]["server"],
        "postgres.example.internal:55117"
    );
    assert_eq!(
        fs::read_to_string(fact_sales_tmdl(&project)).expect("source project remains dummy"),
        before,
        "--out-dir must leave the source project unchanged"
    );
    let live_text = fs::read_to_string(fact_sales_tmdl(&live_project)).expect("live TMDL");
    assert!(live_text.contains(
        "PostgreSQL.Database(\"postgres.example.internal:55117\", \"safety_analytics\")"
    ));
    assert!(!live_text.contains("<WORK_POSTGRES_HOST:PORT>"));
    assert!(!live_text.to_ascii_lowercase().contains("password"));

    let partition = run_powerbi(&[
        "model",
        "partitions",
        "show",
        "--project",
        live_arg,
        "--handle",
        "partition:FactSales:FactSales",
        "--json",
    ]);
    assert_eq!(partition.code, 0, "stderr: {}", partition.stderr);
    assert_eq!(
        stdout_json(&partition)["partition"]["sourceKind"],
        "postgresqlDatabase"
    );

    let refused_overwrite = run_powerbi(&[
        "source-template",
        "apply",
        "--project",
        live_arg,
        "--handle",
        "source-template:FactSales:FactSales",
        "--server",
        "postgres.example.internal:55117",
        "--database",
        "safety_analytics",
        "--in-place",
        "--json",
    ]);
    assert_eq!(refused_overwrite.code, 2);
    assert!(
        stderr_json(&refused_overwrite)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("only replaces a safe generated dummy partition")
    );
}

#[test]
fn postgres_and_odbc_source_templates_round_trip_dry_run_and_out_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let template_store = project.join(".powerbi-cli").join("source-templates.json");

    let postgres_m = r#"let
    Source = PostgreSQL.Database("<server>", "<database>"),
    Navigation = Source{[Schema="public",Item="<object>"]}[Data]
in
    Navigation"#;
    let postgres_dry_run = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--kind",
        "postgres",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        postgres_dry_run.code, 0,
        "stderr: {}",
        postgres_dry_run.stderr
    );
    let postgres_dry_json = stdout_json(&postgres_dry_run);
    assert_eq!(
        postgres_dry_json["changes"][0]["after"]["mTemplate"],
        postgres_m
    );
    assert_eq!(
        postgres_dry_json["changes"][0]["after"]["parameters"]["schema"],
        "public"
    );
    assert!(!template_store.exists(), "dry-run must not write a store");

    let postgres_out = temp.path().join("sales_postgres");
    let postgres_out_arg = postgres_out.to_str().expect("postgres out path");
    let postgres_add = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--kind",
        "postgresql",
        "--out-dir",
        postgres_out_arg,
        "--json",
    ]);
    assert_eq!(postgres_add.code, 0, "stderr: {}", postgres_add.stderr);
    let postgres_show = run_powerbi(&[
        "source-template",
        "show",
        "--project",
        postgres_out_arg,
        "--handle",
        "source-template:FactSales:FactSales",
        "--json",
    ]);
    assert_eq!(postgres_show.code, 0, "stderr: {}", postgres_show.stderr);
    let postgres_show_json = stdout_json(&postgres_show);
    assert_eq!(postgres_show_json["sourceTemplate"]["kind"], "postgres");
    assert_eq!(
        postgres_show_json["sourceTemplate"]["mTemplate"],
        postgres_m
    );
    assert!(
        postgres_show_json["sourceTemplate"]["requirements"]
            .as_array()
            .expect("postgres requirements")
            .iter()
            .any(|requirement| requirement.as_str().unwrap_or_default().contains("Npgsql"))
    );

    let odbc_m = r#"let
    Source = Odbc.DataSource("dsn=<dsn>", [HierarchicalNavigation = true]),
    Navigation = Source{[Name="<database>"]}[Data]{[Name="<schema>"]}[Data]{[Name="<object>"]}[Data]
in
    Navigation"#;
    let odbc_dry_run = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--kind",
        "odbc",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(odbc_dry_run.code, 0, "stderr: {}", odbc_dry_run.stderr);
    assert_eq!(
        stdout_json(&odbc_dry_run)["changes"][0]["after"]["mTemplate"],
        odbc_m
    );
    assert!(!template_store.exists(), "dry-run must not write a store");

    let odbc_out = temp.path().join("sales_odbc");
    let odbc_out_arg = odbc_out.to_str().expect("odbc out path");
    let odbc_add = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--kind",
        "odbc",
        "--out-dir",
        odbc_out_arg,
        "--json",
    ]);
    assert_eq!(odbc_add.code, 0, "stderr: {}", odbc_add.stderr);
    let odbc_show = run_powerbi(&[
        "source-template",
        "show",
        "--project",
        odbc_out_arg,
        "--handle",
        "source-template:DimDate:DimDate",
        "--json",
    ]);
    assert_eq!(odbc_show.code, 0, "stderr: {}", odbc_show.stderr);
    let odbc_show_json = stdout_json(&odbc_show);
    assert_eq!(odbc_show_json["sourceTemplate"]["kind"], "odbc");
    assert_eq!(odbc_show_json["sourceTemplate"]["mTemplate"], odbc_m);
    assert!(
        odbc_show_json["sourceTemplate"]["requirements"]
            .as_array()
            .expect("odbc requirements")
            .iter()
            .any(|requirement| requirement.as_str().unwrap_or_default().contains("DSN"))
    );
}

#[test]
fn handoff_rebind_plan_is_complete_and_deterministic_when_all_templates_exist() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    for (table, kind, source_flag, source, schema) in [
        ("FactSales", "sql", "--server", "<server>", "dbo"),
        ("DimDate", "postgres", "--server", "<server>", "public"),
        ("DimCustomer", "odbc", "--dsn", "<dsn>", "<schema>"),
    ] {
        let add = run_powerbi(&[
            "source-template",
            "add",
            "--project",
            project_arg,
            "--table",
            table,
            "--kind",
            kind,
            source_flag,
            source,
            "--database",
            "<database>",
            "--schema",
            schema,
            "--object",
            table,
            "--in-place",
            "--json",
        ]);
        assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    }

    let plan_a = run_powerbi(&["handoff", "rebind", project_arg, "--json"]);
    assert_eq!(plan_a.code, 0, "stderr: {}", plan_a.stderr);
    let plan_b = run_powerbi(&["handoff-rebind-plan", project_arg, "--json"]);
    assert_eq!(plan_b.code, 0, "stderr: {}", plan_b.stderr);
    assert_eq!(
        plan_a.stdout, plan_b.stdout,
        "rebind plan JSON should be deterministic across aliases"
    );
    let plan_json = stdout_json(&plan_a);
    assert_eq!(plan_json["complete"], Value::Bool(true));
    assert_eq!(plan_json["counts"]["mappedPartitions"], Value::from(3));
    assert_eq!(plan_json["counts"]["unmappedPartitions"], Value::from(0));
    assert!(
        plan_json["instructionsMarkdown"]
            .as_str()
            .unwrap_or_default()
            .contains("# Power BI Rebind Plan")
    );
    let markdown = plan_json["instructionsMarkdown"]
        .as_str()
        .expect("instructions markdown");
    assert!(markdown.contains("Current Power BI Desktop releases include the Npgsql provider"));
    assert!(markdown.contains("on-premises data gateway releases before June 2025"));
    assert!(markdown.contains("ODBC DSN"));
    assert!(markdown.contains("Refresh completes successfully"));
    assert!(markdown.contains("Every report page canvas renders"));
    assert!(markdown.contains("No Power BI Desktop issue, warning, or error banners"));
    assert!(markdown.contains("Optional, if `powerbi-cli` is available at work"));
    assert!(markdown.contains("Credentials must live only in Power BI Desktop"));

    let runbook = project.join("work-machine-rebind.md");
    let runbook_arg = runbook.to_str().expect("runbook path");
    let write_runbook = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project_arg,
        "--out",
        runbook_arg,
        "--json",
    ]);
    assert_eq!(write_runbook.code, 0, "stderr: {}", write_runbook.stderr);
    let write_json = stdout_json(&write_runbook);
    assert_eq!(write_json["runbookWritten"], Value::Bool(true));
    assert!(write_json["runbookPath"].as_str().is_some());
    assert_eq!(
        fs::read_to_string(&runbook).expect("read runbook"),
        write_json["instructionsMarkdown"]
            .as_str()
            .expect("runbook markdown")
    );

    fs::write(&runbook, "keep this runbook").expect("replace runbook sentinel");
    let refused = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project_arg,
        "--out",
        runbook_arg,
        "--json",
    ]);
    assert_eq!(refused.code, 2);
    assert!(
        stderr_json(&refused)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("already exists")
    );
    assert_eq!(
        fs::read_to_string(&runbook).expect("read preserved runbook"),
        "keep this runbook"
    );

    let forced = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project_arg,
        "--out",
        runbook_arg,
        "--force",
        "--json",
    ]);
    assert_eq!(forced.code, 0, "stderr: {}", forced.stderr);
    assert!(
        fs::read_to_string(&runbook)
            .expect("read forced runbook")
            .contains("# Power BI Rebind Plan")
    );

    let check = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(check.code, 0, "stderr: {}", check.stderr);
    let check_json = stdout_json(&check);
    assert_eq!(check_json["ok"], Value::Bool(true));
    assert_eq!(check_json["counts"]["sourceTemplates"], Value::from(3));
}

#[test]
fn source_templates_reject_credentials_and_handoff_check_fails_manual_unsafe_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");

    let rejected = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--kind",
        "sql",
        "--server",
        "Password=secret",
        "--database",
        "<database>",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(rejected.code, 2);
    assert!(
        stderr_json(&rejected)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("credential-like")
    );

    let postgres_rejected = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimDate",
        "--kind",
        "postgres",
        "--server",
        "host;pwd=secret",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(postgres_rejected.code, 2);
    assert!(
        stderr_json(&postgres_rejected)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("credential-like")
    );

    let odbc_rejected = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimCustomer",
        "--kind",
        "odbc",
        "--dsn",
        "CorpDsn;uid=work-user;pwd=secret",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(odbc_rejected.code, 2);
    let odbc_error = stderr_json(&odbc_rejected);
    assert_eq!(
        odbc_error["error"]["message"],
        "source-template --dsn must be a bare ODBC DSN name without ';' or '=' attributes"
    );
    assert!(
        odbc_error["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("ODBC manager or Power BI Desktop credential UI")
    );

    let refused_out = temp.path().join("refused-odbc-output");
    let refused_out_arg = refused_out.to_str().expect("refused output path");
    let odbc_out_rejected = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimCustomer",
        "--kind",
        "odbc",
        "--dsn",
        "CorpDsn;User=alice;Pass=hunter2",
        "--out-dir",
        refused_out_arg,
        "--json",
    ]);
    assert_eq!(odbc_out_rejected.code, 2);
    assert!(
        !refused_out.exists(),
        "invalid DSN must be rejected before out-dir copying"
    );
    assert!(!odbc_out_rejected.stderr.contains("hunter2"));

    let attributed_odbc = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimCustomer",
        "--kind",
        "odbc",
        "--dsn",
        "CorpDsn;Trusted_Connection=yes",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(attributed_odbc.code, 2);
    assert_eq!(
        stderr_json(&attributed_odbc)["error"]["message"],
        "source-template --dsn must be a bare ODBC DSN name without ';' or '=' attributes"
    );

    let multiline_odbc = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "DimCustomer",
        "--kind",
        "odbc",
        "--dsn",
        "CorpDsn\nSecondLine",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(multiline_odbc.code, 2);
    assert!(
        stderr_json(&multiline_odbc)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("single line")
    );

    let store_dir = project.join(".powerbi-cli");
    fs::create_dir_all(&store_dir).expect("create source template dir");
    fs::write(
        store_dir.join("source-templates.json"),
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.source-templates.v1",
            "templates": [{
                "handle": "source-template:FactSales:CorpSql",
                "name": "CorpSql",
                "partitionHandle": "partition:FactSales:FactSales",
                "table": "FactSales",
                "partition": "FactSales",
                "kind": "sql",
                "parameters": {
                    "server": "Password=secret",
                    "password": "hunter2",
                    "database": "<database>",
                    "schema": "dbo",
                    "object": "FactSales"
                },
                "mTemplate": "let Source = Sql.Database(\"Password=secret\", \"<database>\") in Source"
            }]
        }))
        .expect("serialize unsafe store"),
    )
    .expect("write unsafe store");

    let check = run_powerbi(&["handoff", "check", project_arg, "--json"]);
    assert_eq!(check.code, 10);
    let check_json = stdout_json(&check);
    assert!(
        check_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "sourceTemplate.credential_like_text")
    );

    let runbook = temp.path().join("credential-runbook.md");
    let plan = run_powerbi(&[
        "handoff",
        "rebind-plan",
        project_arg,
        "--allow-unmapped",
        "--out",
        runbook.to_str().expect("runbook path"),
        "--json",
    ]);
    assert_eq!(plan.code, 10);
    assert!(!runbook.exists());
    assert!(!plan.stdout.contains("hunter2"));
    assert!(!plan.stdout.contains("Password=secret"));
    assert!(plan.stdout.contains("Password=***"));
    let plan_json = stdout_json(&plan);
    assert_eq!(plan_json["materializationBlocked"], Value::Bool(true));
    assert_eq!(
        plan_json["materializationBlockReasons"]["unsafeTemplate"],
        Value::Bool(true)
    );
    assert_eq!(plan_json["templates"][0]["parameters"]["password"], "***");
    assert_eq!(plan_json["runbookWritten"], Value::Bool(false));
}

#[test]
fn mixed_dummy_and_postgres_connector_partition_is_unsafe() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_project(temp.path());
    let project_arg = project.to_str().expect("project path");
    let path = fact_sales_tmdl(&project);
    let text = fs::read_to_string(&path).expect("FactSales.tmdl before");
    let source_start = text.find("        source =").expect("source block");
    let replacement = r#"        source =
            let
                Dummy = #table(type table [SalesKey = text], {}),
                Real = PostgreSQL.Database("corp-postgres", "Claims")
            in
                Dummy

"#;
    fs::write(&path, format!("{}{}", &text[..source_start], replacement))
        .expect("write mixed source");

    let list = run_powerbi(&[
        "model",
        "partitions",
        "list",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--json",
    ]);
    assert_eq!(list.code, 0, "stderr: {}", list.stderr);
    let list_json = stdout_json(&list);
    assert_eq!(
        list_json["partitions"][0]["sourceKind"],
        "postgresqlDatabase"
    );
    assert_eq!(
        list_json["partitions"][0]["offlineSafety"]["safeForHome"],
        Value::Bool(false)
    );
    assert_eq!(
        list_json["partitions"][0]["offlineSafety"]["externalConnector"],
        Value::Bool(true)
    );
}
