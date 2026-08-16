use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct RunOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

struct SynthesizeFixture {
    project: PathBuf,
    pbip: PathBuf,
    expressions: PathBuf,
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

fn scaffold_live_fixture(root: &Path) -> SynthesizeFixture {
    let project = root.join("live-project");
    let scaffold = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(scaffold.code, 0, "stderr: {}", scaffold.stderr);

    install_live_partition(&project, "FactSales", "sales", "orders");
    install_live_partition(&project, "DimCustomer", "crm", "customers");
    let model = project.join("SalesOperations.SemanticModel");
    fs::create_dir_all(model.join(".pbi")).expect("model runtime directory");
    fs::write(model.join(".pbi").join("cache.abf"), b"private cache").expect("cache fixture");
    fs::write(model.join("localSettings.json"), b"{}\n").expect("local settings fixture");
    fs::create_dir_all(project.join("notes")).expect("notes directory");
    fs::write(
        project.join("notes").join("keep.txt"),
        "copied whole-project file\n",
    )
    .expect("whole-project fixture");

    let expressions = root.join("synthetic-expressions.tmdl");
    fs::write(
        &expressions,
        "expression SynthCustomers = #table({}, {})\n\nexpression SynthOrders = #table({}, {})\n\nexpression QaOrders = #table({}, {})\n",
    )
    .expect("synthetic expressions");
    SynthesizeFixture {
        pbip: project.join("SalesOperations.pbip"),
        project,
        expressions,
    }
}

fn install_live_partition(project: &Path, table: &str, schema: &str, item: &str) {
    let path = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join(format!("{table}.tmdl"));
    let text = fs::read_to_string(&path).expect("scaffolded table");
    let marker = "        source =\n";
    let source_start = text.find(marker).expect("partition source marker");
    let mut live = text[..source_start].to_string();
    live.push_str(marker);
    live.push_str("            let\n");
    live.push_str(
        "                Database = PostgreSQL.Database(\"prod-db.internal\", \"warehouse\"),\n",
    );
    live.push_str(&format!(
        "                Navigation = Database{{[Schema = \"{schema}\", Item = \"{item}\"]}}[Data],\n"
    ));
    live.push_str("                KeepRows = Table.SelectRows(Navigation, each true)\n");
    live.push_str("            in\n");
    live.push_str("                KeepRows\n\n");
    fs::write(path, live).expect("live partition");
}

fn table_text(project: &Path, table: &str) -> String {
    fs::read_to_string(
        project
            .join("SalesOperations.SemanticModel")
            .join("definition")
            .join("tables")
            .join(format!("{table}.tmdl")),
    )
    .expect("output table")
}

fn database_line(text: &str) -> &str {
    text.lines()
        .find(|line| line.trim_start().starts_with("Database ="))
        .expect("Database shim line")
        .trim()
}

#[test]
fn workflow_synthesize_swaps_all_partitions_and_validates_offline_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = scaffold_live_fixture(temp.path());
    let output = temp.path().join("offline-project");
    let command = run_powerbi(&[
        "workflow",
        "synthesize",
        "--project",
        fixture.pbip.to_str().expect("PBIP path"),
        "--expressions",
        fixture.expressions.to_str().expect("expressions path"),
        "--out-dir",
        output.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(
        command.code, 0,
        "stdout: {}\nstderr: {}",
        command.stdout, command.stderr
    );
    let result = stdout_json(&command);
    assert_eq!(result["schema"], "powerbi-cli.workflow-synthesize.v1");
    assert_eq!(result["validation"]["ok"], true);
    assert_eq!(result["offlineSafety"]["ok"], true);
    assert_eq!(result["counts"]["navigationPairs"], 2);
    assert_eq!(result["counts"]["partitionsModified"], 2);

    let orders = table_text(&output, "FactSales");
    let customers = table_text(&output, "DimCustomer");
    assert_eq!(database_line(&orders), database_line(&customers));
    assert!(database_line(&orders).contains(
        "{{\"crm\", \"customers\", SynthCustomers}, {\"sales\", \"orders\", SynthOrders}}"
    ));
    assert!(orders.contains("KeepRows = Table.SelectRows(Navigation, each true)"));
    assert!(customers.contains("KeepRows = Table.SelectRows(Navigation, each true)"));
    assert!(!orders.contains("PostgreSQL.Database"));
    assert!(!customers.contains("PostgreSQL.Database"));
    assert!(!orders.contains("prod-db.internal"));
    assert_eq!(
        fs::read(
            output
                .join("SalesOperations.SemanticModel")
                .join("definition")
                .join("expressions.tmdl")
        )
        .expect("installed expressions"),
        fs::read(&fixture.expressions).expect("source expressions")
    );
    assert_eq!(
        fs::read_to_string(output.join("notes").join("keep.txt")).expect("copied note"),
        "copied whole-project file\n"
    );
    assert!(
        !output
            .join("SalesOperations.SemanticModel")
            .join(".pbi")
            .join("cache.abf")
            .exists()
    );
    assert!(
        !output
            .join("SalesOperations.SemanticModel")
            .join("localSettings.json")
            .exists()
    );

    let validate = run_powerbi(&[
        "validate",
        output.to_str().expect("output project"),
        "--json",
    ]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], true);
}

#[test]
fn workflow_synthesize_lists_every_missing_expression_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = scaffold_live_fixture(temp.path());
    fs::write(&fixture.expressions, "expression Unrelated = 1\n").expect("incomplete expressions");
    let output = temp.path().join("missing-output");
    let command = run_powerbi(&[
        "workflow",
        "synthesize",
        "--project",
        fixture.project.to_str().expect("project path"),
        "--expressions",
        fixture.expressions.to_str().expect("expressions path"),
        "--out-dir",
        output.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(command.code, 10, "stdout: {}", command.stdout);
    assert!(command.stderr.contains("SynthCustomers"));
    assert!(command.stderr.contains("SynthOrders"));
    assert!(!output.exists());
}

#[test]
fn workflow_synthesize_honors_pair_mapping_override() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = scaffold_live_fixture(temp.path());
    let output = temp.path().join("mapped-output");
    let command = run_powerbi(&[
        "workflow",
        "synthesize",
        "--project",
        fixture.project.to_str().expect("project path"),
        "--expressions",
        fixture.expressions.to_str().expect("expressions path"),
        "--out-dir",
        output.to_str().expect("output path"),
        "--map",
        "sales.orders=QaOrders",
        "--json",
    ]);
    assert_eq!(command.code, 0, "stderr: {}", command.stderr);
    let orders = table_text(&output, "FactSales");
    assert!(orders.contains("{\"sales\", \"orders\", QaOrders}"));
    assert!(!orders.contains("{\"sales\", \"orders\", SynthOrders}"));
    assert!(
        stdout_json(&command)["mappings"]
            .as_array()
            .expect("mappings")
            .iter()
            .any(|mapping| mapping["schema"] == "sales"
                && mapping["item"] == "orders"
                && mapping["expression"] == "QaOrders")
    );
}

#[test]
fn workflow_synthesize_refuses_output_inside_source_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = scaffold_live_fixture(temp.path());
    let output = fixture.project.join("offline-copy");
    let command = run_powerbi(&[
        "workflow",
        "synthesize",
        "--project",
        fixture.project.to_str().expect("project path"),
        "--expressions",
        fixture.expressions.to_str().expect("expressions path"),
        "--out-dir",
        output.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(command.code, 2, "stdout: {}", command.stdout);
    assert!(command.stderr.contains("outside the source project tree"));
    assert!(!output.exists());
}

#[test]
fn capabilities_publish_synthesize_schema_golden_contract() {
    let command = run_powerbi(&["capabilities", "--for", "workflow synthesize", "--json"]);
    assert_eq!(command.code, 0, "stderr: {}", command.stderr);
    let result = stdout_json(&command);
    let contract = result["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "workflow synthesize")
        .expect("workflow synthesize contract");
    assert_eq!(contract["proofLevel"], "schema-golden");
    assert!(contract["usage"].as_str().expect("usage").contains("--map"));
    assert!(contract["flags"].as_array().expect("flags").len() >= 6);
    assert!(
        !contract["examples"]
            .as_array()
            .expect("examples")
            .is_empty()
    );
    assert!(
        !contract["limitations"]
            .as_array()
            .expect("limitations")
            .is_empty()
    );
}
