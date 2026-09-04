mod common;

use common::{run_powerbi, stdout_json};
use std::fs;
use std::path::{Path, PathBuf};

struct SynthesizeFixture {
    project: PathBuf,
    pbip: PathBuf,
    expressions: PathBuf,
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

fn install_scaled_expressions(path: &Path) {
    fs::write(
        path,
        r#"expression SynthCustomers = (rowScale as number, seed as number) as table =>
    let
        RowCount = Number.RoundDown(rowScale),
        Rows = List.Transform({0..RowCount - 1}, each {Number.Mod(_ + seed, 1000)})
    in
        #table(type table [Value = Int64.Type], Rows)

expression SynthOrders = (rowScale as number, seed as number) as table =>
    let
        RowCount = Number.RoundDown(rowScale),
        Rows = List.Transform({0..RowCount - 1}, each {Number.Mod((_ * 17) + seed, 1000)})
    in
        #table(type table [Value = Int64.Type], Rows)
"#,
    )
    .expect("scaled synthetic expressions");
}

fn synthesize_with_scale(
    fixture: &SynthesizeFixture,
    output: &Path,
    row_scale: &str,
    seed: &str,
) -> common::RunOutput {
    run_powerbi(&[
        "workflow",
        "synthesize",
        "--project",
        fixture.pbip.to_str().expect("PBIP path"),
        "--expressions",
        fixture.expressions.to_str().expect("expressions path"),
        "--out-dir",
        output.to_str().expect("output path"),
        "--row-scale",
        row_scale,
        "--seed",
        seed,
        "--json",
    ])
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
    assert!(result["generationParameters"].is_null());

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
fn workflow_synthesize_row_scale_and_seed_match_two_m_goldens() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = scaffold_live_fixture(temp.path());
    install_scaled_expressions(&fixture.expressions);

    for (row_scale, seed, golden) in [
        (
            "10",
            "42",
            include_str!("../testdata/golden/workflow-synthesize/row-scale-10-seed-42.database.m"),
        ),
        (
            "1000",
            "42",
            include_str!(
                "../testdata/golden/workflow-synthesize/row-scale-1000-seed-42.database.m"
            ),
        ),
    ] {
        let output = temp.path().join(format!("scaled-{row_scale}"));
        let command = synthesize_with_scale(&fixture, &output, row_scale, seed);
        assert_eq!(
            command.code, 0,
            "stdout: {}\nstderr: {}",
            command.stdout, command.stderr
        );
        let result = stdout_json(&command);
        assert_eq!(
            result["generationParameters"]["rowScale"],
            row_scale.parse::<u64>().unwrap()
        );
        assert_eq!(
            result["generationParameters"]["seed"],
            seed.parse::<u64>().unwrap()
        );
        let fact_sales = table_text(&output, "FactSales");
        assert_eq!(database_line(&fact_sales), golden.trim());

        let lint = run_powerbi(&["lint", output.to_str().expect("output path"), "--json"]);
        assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
        assert!(
            stdout_json(&lint)["findings"]
                .as_array()
                .expect("lint findings")
                .iter()
                .all(|finding| !finding["code"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("m.")),
            "generated synthetic partition M must lint clean"
        );
    }
}

#[test]
fn workflow_synthesize_scaled_output_is_byte_deterministic_and_seed_sensitive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = scaffold_live_fixture(temp.path());
    install_scaled_expressions(&fixture.expressions);
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    let changed_seed = temp.path().join("changed-seed");

    for (output, seed) in [(&first, "7"), (&second, "7"), (&changed_seed, "8")] {
        let command = synthesize_with_scale(&fixture, output, "250", seed);
        assert_eq!(command.code, 0, "stderr: {}", command.stderr);
    }

    let first_bytes = fs::read(
        first
            .join("SalesOperations.SemanticModel")
            .join("definition")
            .join("tables")
            .join("FactSales.tmdl"),
    )
    .expect("first table bytes");
    let second_bytes = fs::read(
        second
            .join("SalesOperations.SemanticModel")
            .join("definition")
            .join("tables")
            .join("FactSales.tmdl"),
    )
    .expect("second table bytes");
    let changed_seed_bytes = fs::read(
        changed_seed
            .join("SalesOperations.SemanticModel")
            .join("definition")
            .join("tables")
            .join("FactSales.tmdl"),
    )
    .expect("changed-seed table bytes");
    assert_eq!(first_bytes, second_bytes);
    assert_ne!(first_bytes, changed_seed_bytes);
}

#[test]
fn workflow_synthesize_refuses_invalid_generation_parameters_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases: &[&[&str]] = &[
        &["--row-scale", "0"],
        &["--row-scale", "-1"],
        &["--row-scale", "9007199254740992"],
        &["--seed", "-1"],
        &["--seed", "9007199254740992"],
        &["--row-scale", "2", "--row-scale", "3"],
        &["--seed", "2", "--seed", "3"],
    ];
    for (case_index, flags) in cases.iter().enumerate() {
        let output = temp.path().join(format!("missing-output-{case_index}"));
        let mut args = vec![
            "workflow",
            "synthesize",
            "--project",
            "missing.pbip",
            "--expressions",
            "missing.tmdl",
            "--out-dir",
            output.to_str().expect("output path"),
        ];
        args.extend_from_slice(flags);
        args.push("--json");
        let command = run_powerbi(&args);
        assert_eq!(command.code, 2, "stdout: {}", command.stdout);
        let error: serde_json::Value =
            serde_json::from_str(command.stderr.trim()).expect("error JSON");
        assert_eq!(error["error"]["code"], "invalid_args");
        assert!(error["error"]["hint"].is_string());
        assert!(
            error["error"]["suggestedCommands"]
                .as_array()
                .expect("suggested commands")
                .iter()
                .all(|command| command
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("powerbi-cli "))
        );
        assert!(!output.exists());
    }
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
    assert!(
        contract["usage"]
            .as_str()
            .expect("usage")
            .contains("--row-scale")
    );
    assert!(
        contract["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag.as_str().unwrap_or_default().starts_with("--seed"))
    );
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

    let features = run_powerbi(&[
        "features",
        "list",
        "--for",
        "workflow.synthetic-source",
        "--json",
    ]);
    assert_eq!(features.code, 0, "stderr: {}", features.stderr);
    let feature = stdout_json(&features)["features"]
        .as_array()
        .expect("features")
        .first()
        .expect("synthetic source feature")
        .clone();
    assert_eq!(feature["id"], "workflow.synthetic-source");
    assert_eq!(feature["proofLevel"], "schema-golden");

    let help = run_powerbi(&["--help"]);
    assert_eq!(help.code, 0, "stderr: {}", help.stderr);
    assert!(help.stdout.contains("--row-scale <n>"));
    assert!(help.stdout.contains("--seed <s>"));
}
