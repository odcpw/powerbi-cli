//! Table-driven parity between each registered operation kernel and its CLI
//! mutation path.

mod common;

use common::{
    ArchetypeFixture, OperationEquivalenceCase, OperationExecution, assert_tree_equal,
    first_page_name, first_two_visual_names, first_visual_json, run_direct_operation,
    run_operation_equivalence, run_powerbi, run_powerbi_owned, scaffold_fixture, scaffold_sales,
    stdout_json,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn visual_handle(project: &Path) -> String {
    let visual_path = first_visual_json(project);
    let visual: Value =
        serde_json::from_str(&fs::read_to_string(&visual_path).expect("read visual"))
            .expect("parse visual");
    let page = first_page_name(project);
    format!(
        "visual:{page}:{}",
        visual["name"].as_str().expect("visual name")
    )
}

macro_rules! legacy_case {
    ($name:ident, $tag:literal) => {
        OperationEquivalenceCase {
            name: concat!($tag, "/sales"),
            fixture: "sales",
            operation_tag: $tag,
            execute: $name,
        }
    };
}

#[test]
fn set_object_op_replays_are_deterministic_and_preserve_cli_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = scaffold_sales(&temp.path().join("first"));
    let second = scaffold_sales(&temp.path().join("second"));
    let first_handle = visual_handle(&first);
    let second_handle = visual_handle(&second);
    assert_eq!(first_handle, second_handle);

    let first_out = temp.path().join("first-out");
    let second_out = temp.path().join("second-out");
    for (project, handle, out) in [
        (&first, first_handle, &first_out),
        (&second, second_handle, &second_out),
    ] {
        let output = run_powerbi(&[
            "report",
            "visuals",
            "set-object",
            "--project",
            project.to_str().expect("project path"),
            "--handle",
            &handle,
            "--object",
            "categoryLabels",
            "--property",
            "fontSize",
            "--value",
            "20",
            "--out-dir",
            out.to_str().expect("out path"),
            "--json",
        ]);
        assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
        assert_eq!(
            stdout_json(&output)["schema"],
            "powerbi-cli.report.visuals.objectMutation.v1"
        );
    }
    assert_tree_equal(&first_out, &second_out, "set-object CLI determinism");
}

#[test]
fn set_object_batch_matches_replaying_each_registered_set_object_operation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let page = first_page_name(&source);
    let (first, second) = first_two_visual_names(&source);
    let first = format!("visual:{page}:{first}");
    let second = format!("visual:{page}:{second}");
    let operations = json!({
        "schema": "powerbi-cli.ops.v1",
        "ops": [
            {
                "op": "setObject",
                "visual": first,
                "object": "categoryLabels",
                "property": "fontSize",
                "value": {"expr": {"Literal": {"Value": "20D"}}}
            },
            {
                "op": "setObject",
                "visual": second,
                "object": "title",
                "property": "show",
                "value": {"expr": {"Literal": {"Value": "false"}}}
            }
        ]
    });
    let batch_file = temp.path().join("set-object.ops.json");
    fs::write(
        &batch_file,
        serde_json::to_vec_pretty(&operations).expect("serialize operation plan"),
    )
    .expect("write operation plan");

    let batch_out = temp.path().join("batch-out");
    let batch = run_powerbi_owned(&[
        "report".to_string(),
        "visuals".to_string(),
        "set-object".to_string(),
        "--project".to_string(),
        source.to_string_lossy().into_owned(),
        "--batch".to_string(),
        batch_file.to_string_lossy().into_owned(),
        "--out-dir".to_string(),
        batch_out.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    assert_eq!(batch.exit, 0, "batch stderr: {}", batch.stderr);

    let first_out = temp.path().join("first-out");
    let first_call = run_powerbi_owned(&[
        "report".to_string(),
        "visuals".to_string(),
        "set-object".to_string(),
        "--project".to_string(),
        source.to_string_lossy().into_owned(),
        "--handle".to_string(),
        first,
        "--object".to_string(),
        "categoryLabels".to_string(),
        "--property".to_string(),
        "fontSize".to_string(),
        "--value".to_string(),
        "20".to_string(),
        "--out-dir".to_string(),
        first_out.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    assert_eq!(first_call.exit, 0, "first stderr: {}", first_call.stderr);
    let replay_out = temp.path().join("replay-out");
    let second_call = run_powerbi_owned(&[
        "report".to_string(),
        "visuals".to_string(),
        "set-object".to_string(),
        "--project".to_string(),
        first_out.to_string_lossy().into_owned(),
        "--handle".to_string(),
        second,
        "--object".to_string(),
        "title".to_string(),
        "--property".to_string(),
        "show".to_string(),
        "--value".to_string(),
        "false".to_string(),
        "--out-dir".to_string(),
        replay_out.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    assert_eq!(second_call.exit, 0, "second stderr: {}", second_call.stderr);

    assert_tree_equal(
        &batch_out,
        &replay_out,
        "set-object batch and registered kernel replay",
    );
}

#[test]
fn set_position_op_replays_are_deterministic_and_preserve_cli_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = scaffold_sales(&temp.path().join("first"));
    let second = scaffold_sales(&temp.path().join("second"));
    let first_handle = visual_handle(&first);
    let second_handle = visual_handle(&second);
    assert_eq!(first_handle, second_handle);
    let page = first_page_name(&first);

    let first_out = temp.path().join("first-out");
    let second_out = temp.path().join("second-out");
    for (project, handle, out) in [
        (&first, first_handle, &first_out),
        (&second, second_handle, &second_out),
    ] {
        let output = run_powerbi(&[
            "report",
            "visuals",
            "set-position",
            "--project",
            project.to_str().expect("project path"),
            "--page",
            &page,
            "--visual",
            handle.rsplit(':').next().expect("visual name"),
            "--x",
            "120",
            "--y",
            "140",
            "--width",
            "360",
            "--height",
            "220",
            "--z",
            "5",
            "--tab-order",
            "4",
            "--out-dir",
            out.to_str().expect("out path"),
            "--json",
        ]);
        assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
        assert_eq!(
            stdout_json(&output)["schema"],
            "powerbi-cli.report.visuals.positionMutation.v1"
        );
    }
    assert_tree_equal(&first_out, &second_out, "set-position CLI determinism");
}

#[test]
fn registered_operations_have_cli_and_typed_kernel_equivalence_cases() {
    run_operation_equivalence(&equivalence_cases());
}

#[test]
fn every_registered_kernel_replays_through_the_public_bounded_plan_loader() {
    let mut failures = Vec::new();
    for case in equivalence_cases() {
        let fixture = common::load_archetype(case.fixture);
        let workspace = tempfile::tempdir().unwrap();
        let execution =
            match std::panic::catch_unwind(|| (case.execute)(&fixture, workspace.path())) {
                Ok(execution) => execution,
                Err(_) => {
                    failures.push(format!("{}: existing CLI/direct fixture failed", case.name));
                    continue;
                }
            };
        assert_eq!(
            execution.cli.exit, 0,
            "{}: {}",
            case.name, execution.cli.stderr
        );
        let plan = workspace.path().join("public.ops.json");
        fs::write(
            &plan,
            serde_json::to_vec(
                &json!({"schema":"powerbi-cli.ops.v1", "ops":[execution.operation]}),
            )
            .unwrap(),
        )
        .unwrap();
        let source = execution
            .op
            .argv
            .windows(2)
            .find(|args| args[0] == "--project")
            .unwrap()[1]
            .clone();
        let output = workspace.path().join("public-output");
        let result = run_powerbi(&[
            "ops",
            "apply",
            "--project",
            &source,
            "--ops",
            plan.to_str().unwrap(),
            "--out-dir",
            output.to_str().unwrap(),
            "--json",
        ]);
        if result.exit != 0 {
            failures.push(format!("{} public replay: {}", case.name, result.stderr));
            continue;
        }
        assert_tree_equal(
            &execution.cli_tree,
            &output,
            &format!("{} public replay", case.name),
        );
    }
    assert!(
        failures.is_empty(),
        "{} of 37 replay cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn equivalence_cases() -> [OperationEquivalenceCase; 37] {
    [
        legacy_case!(add_calculated_column, "addCalculatedColumn"),
        OperationEquivalenceCase {
            name: "addFilter/sales",
            fixture: "sales",
            operation_tag: "addFilter",
            execute: add_filter_case,
        },
        OperationEquivalenceCase {
            name: "addMeasure/sales",
            fixture: "sales",
            operation_tag: "addMeasure",
            execute: add_measure_case,
        },
        legacy_case!(add_page, "addPage"),
        OperationEquivalenceCase {
            name: "addRelationship/sales",
            fixture: "sales",
            operation_tag: "addRelationship",
            execute: add_relationship_case,
        },
        legacy_case!(add_static_table, "addStaticTable"),
        OperationEquivalenceCase {
            name: "addVisual/sales",
            fixture: "sales",
            operation_tag: "addVisual",
            execute: add_visual_case,
        },
        legacy_case!(apply_style_bundle, "applyStyleBundle"),
        legacy_case!(apply_theme_bundle, "applyThemeBundle"),
        OperationEquivalenceCase {
            name: "applyThemePreset/sales",
            fixture: "sales",
            operation_tag: "applyThemePreset",
            execute: apply_theme_preset_case,
        },
        legacy_case!(bookmark_metadata, "bookmarkMetadata"),
        legacy_case!(clear_filter, "clearFilter"),
        legacy_case!(clone_page, "clonePage"),
        legacy_case!(clone_visual, "cloneVisual"),
        legacy_case!(delete_empty_page, "deleteEmptyPage"),
        legacy_case!(delete_filter, "deleteFilter"),
        legacy_case!(delete_visual, "deleteVisual"),
        legacy_case!(formatting_apply, "formattingApply"),
        legacy_case!(reorder_pages, "reorderPages"),
        OperationEquivalenceCase {
            name: "resetInteraction/sales",
            fixture: "sales",
            operation_tag: "resetInteraction",
            execute: reset_interaction_case,
        },
        legacy_case!(sanitize_action, "sanitizeAction"),
        legacy_case!(set_active_page, "setActivePage"),
        legacy_case!(set_bindings, "setBindings"),
        legacy_case!(set_color, "setColor"),
        legacy_case!(set_display_name, "setDisplayName"),
        legacy_case!(set_drilldown_hierarchy, "setDrilldownHierarchy"),
        OperationEquivalenceCase {
            name: "setDrillthrough/sales",
            fixture: "sales",
            operation_tag: "setDrillthrough",
            execute: set_drillthrough_case,
        },
        OperationEquivalenceCase {
            name: "setInteraction/sales",
            fixture: "sales",
            operation_tag: "setInteraction",
            execute: set_interaction_case,
        },
        OperationEquivalenceCase {
            name: "setObject/sales",
            fixture: "sales",
            operation_tag: "setObject",
            execute: set_object_case,
        },
        OperationEquivalenceCase {
            name: "setPosition/sales",
            fixture: "sales",
            operation_tag: "setPosition",
            execute: set_position_case,
        },
        legacy_case!(set_sort_by, "setSortBy"),
        legacy_case!(set_text, "setText"),
        legacy_case!(set_topn_guard, "setTopNGuard"),
        legacy_case!(slicer_clear, "slicerClear"),
        legacy_case!(source_template_apply, "sourceTemplateApply"),
        legacy_case!(update_filter, "updateFilter"),
        legacy_case!(update_page, "updatePage"),
    ]
}

fn legacy_sources(
    fixture: &ArchetypeFixture,
    workspace: &Path,
    label: &str,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let cli_source = workspace.join(format!("{label}-cli-source"));
    let op_source = workspace.join(format!("{label}-op-source"));
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);
    (cli_source, op_source)
}

fn finish_legacy_case(
    workspace: &Path,
    label: &str,
    cli_source: &Path,
    op_source: &Path,
    operation: Value,
    command: &[&str],
    flags: &[String],
) -> OperationExecution {
    let cli_tree = workspace.join(format!("{label}-cli-output"));
    let op_tree = workspace.join(format!("{label}-op-output"));
    let mut argv = command
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    argv.extend([
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
    ]);
    argv.extend_from_slice(flags);
    argv.extend([
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let cli = run_powerbi_owned(&argv);
    let op = run_direct_operation(&operation, op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn run_in_place(project: &Path, command: &[&str], flags: &[String]) {
    let mut argv = command
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    argv.extend([
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
    ]);
    argv.extend_from_slice(flags);
    argv.extend(["--in-place".to_string(), "--json".to_string()]);
    let run = run_powerbi_owned(&argv);
    assert_eq!(
        run.exit, 0,
        "seed command {:?} failed: {}",
        command, run.stderr
    );
}

fn add_calculated_column(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "add-calculated-column");
    let operation = json!({
        "op": "addCalculatedColumn", "table": "FactSales", "name": "Equivalence Calc",
        "expression": "1", "dataType": "int64"
    });
    let flags = strings(&[
        "--table",
        "FactSales",
        "--name",
        "Equivalence Calc",
        "--expression",
        "1",
        "--data-type",
        "int64",
    ]);
    finish_legacy_case(
        workspace,
        "add-calculated-column",
        &cli_source,
        &op_source,
        operation,
        &["model", "calculated-columns", "add"],
        &flags,
    )
}

fn add_static_table(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "add-static-table");
    let operation = json!({
        "op": "addStaticTable", "table": "Equivalence Static", "column": "Label",
        "valuesJson": "[\"A\",\"B\"]"
    });
    let flags = strings(&[
        "--table",
        "Equivalence Static",
        "--column",
        "Label",
        "--values-json",
        "[\"A\",\"B\"]",
    ]);
    finish_legacy_case(
        workspace,
        "add-static-table",
        &cli_source,
        &op_source,
        operation,
        &["model", "tables", "add-static"],
        &flags,
    )
}

fn set_sort_by(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-sort-by");
    let operation = json!({
        "op": "setSortBy", "table": "DimDate", "column": "Month", "by": "FiscalYear"
    });
    let flags = strings(&[
        "--table",
        "DimDate",
        "--column",
        "Month",
        "--by",
        "FiscalYear",
    ]);
    finish_legacy_case(
        workspace,
        "set-sort-by",
        &cli_source,
        &op_source,
        operation,
        &["model", "columns", "set-sort-by"],
        &flags,
    )
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn page_handles(project: &Path) -> Vec<String> {
    let run = run_powerbi_owned(&[
        "report".into(),
        "pages".into(),
        "list".into(),
        "--project".into(),
        project.to_string_lossy().into_owned(),
        "--json".into(),
    ]);
    assert_eq!(run.exit, 0, "list pages failed: {}", run.stderr);
    stdout_json(&run)["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|page| page["handle"].as_str().expect("page handle").to_string())
        .collect()
}

fn seed_empty_page(project: &Path) {
    run_in_place(
        project,
        &["report", "pages", "add"],
        &strings(&[
            "--name",
            "ReportSectionEquivalenceEmpty",
            "--display-name",
            "Equivalence Empty",
        ]),
    );
}

fn add_page(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "add-page");
    let operation =
        json!({"op": "addPage", "name": "ReportSectionEquivalence", "displayName": "Equivalence"});
    let flags = strings(&[
        "--name",
        "ReportSectionEquivalence",
        "--display-name",
        "Equivalence",
    ]);
    finish_legacy_case(
        workspace,
        "add-page",
        &cli_source,
        &op_source,
        operation,
        &["report", "pages", "add"],
        &flags,
    )
}

fn update_page(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "update-page");
    let handle = page_handles(&cli_source)[0].clone();
    assert_eq!(handle, page_handles(&op_source)[0]);
    let operation =
        json!({"op": "updatePage", "handle": handle, "displayName": "Equivalence Updated"});
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--display-name",
        "Equivalence Updated",
    ]);
    finish_legacy_case(
        workspace,
        "update-page",
        &cli_source,
        &op_source,
        operation,
        &["report", "pages", "update"],
        &flags,
    )
}

fn reorder_pages(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "reorder-pages");
    seed_empty_page(&cli_source);
    seed_empty_page(&op_source);
    let handles = page_handles(&cli_source);
    assert_eq!(handles, page_handles(&op_source));
    let order = handles.iter().rev().cloned().collect::<Vec<_>>().join(",");
    let operation = json!({"op": "reorderPages", "order": order});
    let flags = strings(&["--order", operation["order"].as_str().unwrap()]);
    finish_legacy_case(
        workspace,
        "reorder-pages",
        &cli_source,
        &op_source,
        operation,
        &["report", "pages", "reorder"],
        &flags,
    )
}

fn set_active_page(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-active-page");
    seed_empty_page(&cli_source);
    seed_empty_page(&op_source);
    let handle = page_handles(&cli_source)[1].clone();
    assert_eq!(handle, page_handles(&op_source)[1]);
    let operation = json!({"op": "setActivePage", "handle": handle});
    let flags = strings(&["--handle", operation["handle"].as_str().unwrap()]);
    finish_legacy_case(
        workspace,
        "set-active-page",
        &cli_source,
        &op_source,
        operation,
        &["report", "pages", "set-active"],
        &flags,
    )
}

fn delete_empty_page(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "delete-empty-page");
    seed_empty_page(&cli_source);
    seed_empty_page(&op_source);
    let handle = page_handles(&cli_source)[1].clone();
    assert_eq!(handle, page_handles(&op_source)[1]);
    let operation = json!({"op": "deleteEmptyPage", "handle": handle, "confirm": handle});
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--confirm",
        operation["confirm"].as_str().unwrap(),
    ]);
    finish_legacy_case(
        workspace,
        "delete-empty-page",
        &cli_source,
        &op_source,
        operation,
        &["report", "pages", "delete-empty"],
        &flags,
    )
}

fn clone_page(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "clone-page");
    let from = page_handles(&cli_source)[0].clone();
    assert_eq!(from, page_handles(&op_source)[0]);
    let operation = json!({
        "op": "clonePage", "from": from, "newName": "ReportSectionEquivalenceClone",
        "displayName": "Equivalence Clone", "visualPrefix": "Equivalence"
    });
    let flags = strings(&[
        "--from",
        operation["from"].as_str().unwrap(),
        "--new-name",
        "ReportSectionEquivalenceClone",
        "--display-name",
        "Equivalence Clone",
        "--visual-prefix",
        "Equivalence",
    ]);
    finish_legacy_case(
        workspace,
        "clone-page",
        &cli_source,
        &op_source,
        operation,
        &["report", "pages", "clone"],
        &flags,
    )
}

fn source_template_apply(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "source-template-apply");
    let seed = strings(&[
        "--table",
        "FactSales",
        "--name",
        "EquivalenceSource",
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
    ]);
    run_in_place(&cli_source, &["source-template", "add"], &seed);
    run_in_place(&op_source, &["source-template", "add"], &seed);
    let operation = json!({
        "op": "sourceTemplateApply", "handle": "source-template:FactSales:EquivalenceSource",
        "server": "db.example.internal", "database": "analytics"
    });
    let flags = strings(&[
        "--handle",
        "source-template:FactSales:EquivalenceSource",
        "--server",
        "db.example.internal",
        "--database",
        "analytics",
    ]);
    finish_legacy_case(
        workspace,
        "source-template-apply",
        &cli_source,
        &op_source,
        operation,
        &["source-template", "apply"],
        &flags,
    )
}

fn visual_handle_by_type(project: &Path, visual_type: &str) -> String {
    let run = run_powerbi_owned(&[
        "report".into(),
        "visuals".into(),
        "list".into(),
        "--project".into(),
        project.to_string_lossy().into_owned(),
        "--json".into(),
    ]);
    assert_eq!(run.exit, 0, "list visuals failed: {}", run.stderr);
    stdout_json(&run)["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == visual_type)
        .and_then(|visual| visual["handle"].as_str())
        .expect("visual type handle")
        .to_string()
}

fn set_bindings(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-bindings");
    let handle = visual_handle_by_type(&cli_source, "lineChart");
    assert_eq!(handle, visual_handle_by_type(&op_source, "lineChart"));
    let category = "role=Category,table=DimDate,column=Month";
    let value = "role=Y,table=FactSales,measure=Total Revenue";
    let operation = json!({"op": "setBindings", "handle": handle, "binding": [category, value]});
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--binding",
        category,
        "--binding",
        value,
    ]);
    finish_legacy_case(
        workspace,
        "set-bindings",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "set-bindings"],
        &flags,
    )
}

fn set_display_name(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-display-name");
    let handle = visual_handle_by_type(&cli_source, "card");
    assert_eq!(handle, visual_handle_by_type(&op_source, "card"));
    let operation = json!({
        "op": "setDisplayName", "handle": handle, "role": "Values",
        "displayName": "Equivalence Revenue"
    });
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--role",
        "Values",
        "--display-name",
        "Equivalence Revenue",
    ]);
    finish_legacy_case(
        workspace,
        "set-display-name",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "set-display-name"],
        &flags,
    )
}

fn set_topn_guard(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-topn-guard");
    let handle = visual_handle_by_type(&cli_source, "lineChart");
    assert_eq!(handle, visual_handle_by_type(&op_source, "lineChart"));
    let operation = json!({
        "op": "setTopNGuard", "handle": handle, "field": "DimDate.FiscalYear",
        "orderBy": "FactSales.Total Revenue", "top": "10"
    });
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--field",
        "DimDate.FiscalYear",
        "--order-by",
        "FactSales.Total Revenue",
        "--top",
        "10",
    ]);
    finish_legacy_case(
        workspace,
        "set-topn-guard",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "set-topn-guard"],
        &flags,
    )
}

fn set_drilldown_hierarchy(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-drilldown-hierarchy");
    let handle = visual_handle_by_type(&cli_source, "lineChart");
    assert_eq!(handle, visual_handle_by_type(&op_source, "lineChart"));
    let operation = json!({
        "op": "setDrilldownHierarchy", "handle": handle,
        "field": ["DimDate[FiscalYear]", "DimDate[Month]"]
    });
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--field",
        "DimDate[FiscalYear]",
        "--field",
        "DimDate[Month]",
    ]);
    finish_legacy_case(
        workspace,
        "set-drilldown-hierarchy",
        &cli_source,
        &op_source,
        operation,
        &["report", "drilldown", "set-hierarchy"],
        &flags,
    )
}

fn clone_visual(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "clone-visual");
    let handle = visual_handle_by_type(&cli_source, "card");
    assert_eq!(handle, visual_handle_by_type(&op_source, "card"));
    let operation = json!({
        "op": "cloneVisual", "handle": handle, "name": "VisualContainerEquivalenceClone",
        "title": "Equivalence Clone", "x": "360", "y": "32"
    });
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--name",
        "VisualContainerEquivalenceClone",
        "--title",
        "Equivalence Clone",
        "--x",
        "360",
        "--y",
        "32",
    ]);
    finish_legacy_case(
        workspace,
        "clone-visual",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "clone"],
        &flags,
    )
}

fn delete_visual(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "delete-visual");
    let handle = visual_handle_by_type(&cli_source, "card");
    assert_eq!(handle, visual_handle_by_type(&op_source, "card"));
    let operation = json!({"op": "deleteVisual", "handle": handle, "confirm": handle});
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--confirm",
        operation["confirm"].as_str().unwrap(),
    ]);
    finish_legacy_case(
        workspace,
        "delete-visual",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "delete"],
        &flags,
    )
}

fn seed_report_filter(project: &Path) {
    run_in_place(
        project,
        &["report", "filters", "add"],
        &strings(&[
            "--scope",
            "report",
            "--target",
            "DimCustomer[Segment]",
            "--name",
            "EquivalenceFilter",
            "--value",
            "Enterprise",
        ]),
    );
}

fn update_filter(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "update-filter");
    seed_report_filter(&cli_source);
    seed_report_filter(&op_source);
    let handle = "filter:report:main:EquivalenceFilter";
    let operation =
        json!({"op": "updateFilter", "handle": handle, "displayName": "Equivalence Segment"});
    let flags = strings(&["--handle", handle, "--display-name", "Equivalence Segment"]);
    finish_legacy_case(
        workspace,
        "update-filter",
        &cli_source,
        &op_source,
        operation,
        &["report", "filters", "update"],
        &flags,
    )
}

fn delete_filter(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "delete-filter");
    seed_report_filter(&cli_source);
    seed_report_filter(&op_source);
    let handle = "filter:report:main:EquivalenceFilter";
    let operation = json!({"op": "deleteFilter", "handle": handle, "confirm": handle});
    let flags = strings(&["--handle", handle, "--confirm", handle]);
    finish_legacy_case(
        workspace,
        "delete-filter",
        &cli_source,
        &op_source,
        operation,
        &["report", "filters", "delete"],
        &flags,
    )
}

fn clear_filter(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "clear-filter");
    seed_report_filter(&cli_source);
    seed_report_filter(&op_source);
    let confirm = "clear:filters:report:report:main:1";
    let operation = json!({"op": "clearFilter", "scope": "report", "confirm": confirm});
    let flags = strings(&["--scope", "report", "--confirm", confirm]);
    finish_legacy_case(
        workspace,
        "clear-filter",
        &cli_source,
        &op_source,
        operation,
        &["report", "filters", "clear"],
        &flags,
    )
}

fn seed_slicer(project: &Path) {
    let page = page_handles(project)[0].clone();
    run_in_place(
        project,
        &["report", "visuals", "add-slicer"],
        &strings(&[
            "--page",
            &page,
            "--name",
            "VisualContainerEquivalenceSlicer",
            "--title",
            "Segment",
            "--field",
            "DimCustomer.Segment",
            "--x",
            "960",
            "--y",
            "32",
            "--width",
            "240",
            "--height",
            "120",
        ]),
    );
}

fn slicer_clear(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "slicer-clear");
    seed_slicer(&cli_source);
    seed_slicer(&op_source);
    let visual_handle = visual_handle_by_type(&cli_source, "slicer");
    assert_eq!(visual_handle, visual_handle_by_type(&op_source, "slicer"));
    let handle = visual_handle.replacen("visual:", "slicer:", 1);
    let confirm = format!("clear:slicer:{handle}:0");
    let operation = json!({"op": "slicerClear", "handle": handle, "confirm": confirm});
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--confirm",
        operation["confirm"].as_str().unwrap(),
    ]);
    finish_legacy_case(
        workspace,
        "slicer-clear",
        &cli_source,
        &op_source,
        operation,
        &["report", "slicers", "clear"],
        &flags,
    )
}

fn set_text(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-text");
    let handle = visual_handle_by_type(&cli_source, "card");
    assert_eq!(handle, visual_handle_by_type(&op_source, "card"));
    let operation = json!({"op": "setText", "handle": handle, "title": "Equivalence KPI"});
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--title",
        "Equivalence KPI",
    ]);
    finish_legacy_case(
        workspace,
        "set-text",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "formatting", "set-text"],
        &flags,
    )
}

fn set_color(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "set-color");
    let handle = visual_handle_by_type(&cli_source, "card");
    assert_eq!(handle, visual_handle_by_type(&op_source, "card"));
    let operation = json!({
        "op": "setColor", "handle": handle, "slot": "title.fontColor", "color": "#112233"
    });
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--slot",
        "title.fontColor",
        "--color",
        "#112233",
    ]);
    finish_legacy_case(
        workspace,
        "set-color",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "formatting", "set-color"],
        &flags,
    )
}

fn export_bundle(project: &Path, command: &[&str], out: &Path, extra: &[&str]) {
    let mut argv = command
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    argv.extend([
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--out".to_string(),
        out.to_string_lossy().into_owned(),
    ]);
    argv.extend(extra.iter().map(|value| (*value).to_string()));
    argv.push("--json".to_string());
    let run = run_powerbi_owned(&argv);
    assert_eq!(
        run.exit, 0,
        "bundle export {:?} failed: {}",
        command, run.stderr
    );
}

fn formatting_apply(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "formatting-apply");
    let handle = visual_handle_by_type(&cli_source, "card");
    assert_eq!(handle, visual_handle_by_type(&op_source, "card"));
    let seed = strings(&[
        "--handle",
        &handle,
        "--slot",
        "title.fontColor",
        "--color",
        "#445566",
    ]);
    run_in_place(
        &cli_source,
        &["report", "visuals", "formatting", "set-color"],
        &seed,
    );
    run_in_place(
        &op_source,
        &["report", "visuals", "formatting", "set-color"],
        &seed,
    );
    let bundle = workspace.join("formatting-bundle.json");
    let extract_handle = handle.clone();
    let extract = ["--handle", extract_handle.as_str()];
    export_bundle(
        &cli_source,
        &["report", "visuals", "formatting", "extract"],
        &bundle,
        &extract,
    );
    let bundle_text = bundle.to_string_lossy().into_owned();
    let operation = json!({
        "op": "formattingApply", "handle": handle, "bundle": bundle_text,
        "allowLiteralText": true
    });
    let flags = strings(&[
        "--handle",
        operation["handle"].as_str().unwrap(),
        "--bundle",
        operation["bundle"].as_str().unwrap(),
        "--allow-literal-text",
    ]);
    finish_legacy_case(
        workspace,
        "formatting-apply",
        &cli_source,
        &op_source,
        operation,
        &["report", "visuals", "formatting", "apply"],
        &flags,
    )
}

fn apply_theme_bundle(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "apply-theme-bundle");
    let bundle = workspace.join("theme-bundle.json");
    export_bundle(&cli_source, &["report", "themes", "extract"], &bundle, &[]);
    let bundle_text = bundle.to_string_lossy().into_owned();
    let operation = json!({"op": "applyThemeBundle", "bundle": bundle_text});
    let flags = strings(&["--bundle", operation["bundle"].as_str().unwrap()]);
    finish_legacy_case(
        workspace,
        "apply-theme-bundle",
        &cli_source,
        &op_source,
        operation,
        &["report", "themes", "apply"],
        &flags,
    )
}

fn apply_style_bundle(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "apply-style-bundle");
    let bundle = workspace.join("style-bundle.json");
    export_bundle(
        &cli_source,
        &["report", "style", "extract"],
        &bundle,
        &["--include-literal-text"],
    );
    let bundle_text = bundle.to_string_lossy().into_owned();
    let operation = json!({
        "op": "applyStyleBundle", "bundle": bundle_text, "allowLiteralText": true
    });
    let flags = strings(&[
        "--bundle",
        operation["bundle"].as_str().unwrap(),
        "--allow-literal-text",
    ]);
    finish_legacy_case(
        workspace,
        "apply-style-bundle",
        &cli_source,
        &op_source,
        operation,
        &["report", "style", "apply"],
        &flags,
    )
}

fn install_equivalence_bookmark(project: &Path) {
    let report_dir = fs::read_dir(project)
        .expect("project dir")
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().expect("entry type").is_dir()
                && entry.file_name().to_string_lossy().ends_with(".Report")
        })
        .expect("report dir")
        .path();
    let bookmarks = report_dir.join("definition").join("bookmarks");
    fs::create_dir_all(&bookmarks).expect("bookmarks dir");
    let metadata = json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmarksMetadata/1.0.0/schema.json",
        "items": [{"name": "BookmarkEquivalence"}]
    });
    let bookmark = json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/2.1.0/schema.json",
        "displayName": "Equivalence", "name": "BookmarkEquivalence", "options": {},
        "explorationState": {"version": "1.3", "activeSection": first_page_name(project), "sections": {}}
    });
    fs::write(
        bookmarks.join("bookmarks.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    fs::write(
        bookmarks.join("BookmarkEquivalence.bookmark.json"),
        serde_json::to_vec_pretty(&bookmark).unwrap(),
    )
    .unwrap();
}

fn bookmark_metadata(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "bookmark-metadata");
    install_equivalence_bookmark(&cli_source);
    install_equivalence_bookmark(&op_source);
    let operation = json!({
        "op": "bookmarkMetadata", "action": "set-display-name",
        "handle": "bookmark:BookmarkEquivalence", "displayName": "Equivalence Updated"
    });
    let flags = strings(&[
        "--handle",
        "bookmark:BookmarkEquivalence",
        "--display-name",
        "Equivalence Updated",
    ]);
    finish_legacy_case(
        workspace,
        "bookmark-metadata",
        &cli_source,
        &op_source,
        operation,
        &["report", "bookmarks", "set-display-name"],
        &flags,
    )
}

fn sanitize_action(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let (cli_source, op_source) = legacy_sources(fixture, workspace, "sanitize-action");
    let cli_plan = run_powerbi_owned(&strings(&[
        "report",
        "sanitize",
        "plan",
        "--project",
        cli_source.to_str().expect("CLI source path"),
        "--profile",
        "agent-safe",
        "--json",
    ]));
    assert_eq!(cli_plan.exit, 0, "stderr: {}", cli_plan.stderr);
    let confirm = stdout_json(&cli_plan)["confirmToken"]
        .as_str()
        .expect("sanitize confirmation token")
        .to_string();
    let op_plan = run_powerbi_owned(&strings(&[
        "report",
        "sanitize",
        "plan",
        "--project",
        op_source.to_str().expect("operation source path"),
        "--profile",
        "agent-safe",
        "--json",
    ]));
    assert_eq!(op_plan.exit, 0, "stderr: {}", op_plan.stderr);
    assert_eq!(
        stdout_json(&op_plan)["confirmToken"].as_str(),
        Some(confirm.as_str()),
        "equivalent source trees must produce the same sanitize confirmation token"
    );
    let operation = json!({"op": "sanitizeAction", "profile": "agent-safe", "confirm": confirm});
    let flags = strings(&[
        "--profile",
        "agent-safe",
        "--confirm",
        operation["confirm"].as_str().unwrap(),
    ]);
    finish_legacy_case(
        workspace,
        "sanitize-action",
        &cli_source,
        &op_source,
        operation,
        &["report", "sanitize", "apply"],
        &flags,
    )
}

fn add_measure_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("add-measure-cli-source");
    let op_source = workspace.join("add-measure-op-source");
    let cli_tree = workspace.join("add-measure-cli-output");
    let op_tree = workspace.join("add-measure-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let operation = json!({
        "op": "addMeasure",
        "handle": "measure:FactSales:Equivalence Revenue",
        "table": "FactSales",
        "name": "Equivalence Revenue",
        "expression": "SUM('FactSales'[Revenue])",
        "formatString": "$#,0.00",
        "description": "Equivalence test measure",
        "displayFolder": "Equivalence"
    });
    let cli = run_powerbi_owned(&[
        "model".to_string(),
        "measures".to_string(),
        "add".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--table".to_string(),
        operation["table"]
            .as_str()
            .expect("measure table")
            .to_string(),
        "--name".to_string(),
        operation["name"]
            .as_str()
            .expect("measure name")
            .to_string(),
        "--expression".to_string(),
        operation["expression"]
            .as_str()
            .expect("measure expression")
            .to_string(),
        "--format-string".to_string(),
        operation["formatString"]
            .as_str()
            .expect("measure format")
            .to_string(),
        "--description".to_string(),
        operation["description"]
            .as_str()
            .expect("measure description")
            .to_string(),
        "--display-folder".to_string(),
        operation["displayFolder"]
            .as_str()
            .expect("measure display folder")
            .to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn add_relationship_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("add-relationship-cli-source");
    let op_source = workspace.join("add-relationship-op-source");
    let cli_tree = workspace.join("add-relationship-cli-output");
    let op_tree = workspace.join("add-relationship-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let operation = json!({
        "op": "addRelationship",
        "handle": "relationship:EquivalenceDateCustomer",
        "fromTable": "DimDate",
        "fromColumn": "DateKey",
        "toTable": "DimCustomer",
        "toColumn": "CustomerKey",
        "fromCardinality": "many",
        "toCardinality": "one",
        "crossFilteringBehavior": "oneDirection",
        "isActive": true
    });
    let cli = run_powerbi_owned(&[
        "model".to_string(),
        "relationships".to_string(),
        "add".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--name".to_string(),
        "EquivalenceDateCustomer".to_string(),
        "--from-table".to_string(),
        operation["fromTable"]
            .as_str()
            .expect("from table")
            .to_string(),
        "--from-column".to_string(),
        operation["fromColumn"]
            .as_str()
            .expect("from column")
            .to_string(),
        "--to-table".to_string(),
        operation["toTable"].as_str().expect("to table").to_string(),
        "--to-column".to_string(),
        operation["toColumn"]
            .as_str()
            .expect("to column")
            .to_string(),
        "--from-cardinality".to_string(),
        operation["fromCardinality"]
            .as_str()
            .expect("from cardinality")
            .to_string(),
        "--to-cardinality".to_string(),
        operation["toCardinality"]
            .as_str()
            .expect("to cardinality")
            .to_string(),
        "--cross-filtering-behavior".to_string(),
        operation["crossFilteringBehavior"]
            .as_str()
            .expect("cross filtering behavior")
            .to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn add_visual_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("add-visual-cli-source");
    let op_source = workspace.join("add-visual-op-source");
    let cli_tree = workspace.join("add-visual-cli-output");
    let op_tree = workspace.join("add-visual-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let page = first_page_name(&cli_source);
    let operation = json!({
        "op": "addVisual",
        "handle": format!("visual:{page}:EquivalenceCard"),
        "page": format!("page:{page}"),
        "visualType": "card",
        "name": "EquivalenceCard",
        "title": "Equivalence Card",
        "position": {"x": 40.0, "y": 500.0, "width": 320.0, "height": 180.0},
        "bindings": [{"role": "Values", "table": "FactSales", "measure": "Total Revenue"}]
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "visuals".to_string(),
        "add".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--page".to_string(),
        operation["page"].as_str().expect("visual page").to_string(),
        "--name".to_string(),
        operation["name"].as_str().expect("visual name").to_string(),
        "--type".to_string(),
        operation["visualType"]
            .as_str()
            .expect("visual type")
            .to_string(),
        "--title".to_string(),
        operation["title"]
            .as_str()
            .expect("visual title")
            .to_string(),
        "--binding".to_string(),
        "role=Values,table=FactSales,measure=Total Revenue".to_string(),
        "--x".to_string(),
        "40".to_string(),
        "--y".to_string(),
        "500".to_string(),
        "--width".to_string(),
        "320".to_string(),
        "--height".to_string(),
        "180".to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn add_filter_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("add-filter-cli-source");
    let op_source = workspace.join("add-filter-op-source");
    let cli_tree = workspace.join("add-filter-cli-output");
    let op_tree = workspace.join("add-filter-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let operation = json!({
        "op": "addFilter",
        "handle": "filter:report:main:EquivalenceFilter",
        "scope": "report",
        "owner": "report:main",
        "filterType": "Categorical",
        "target": {"table": "DimCustomer", "column": "Segment"},
        "name": "EquivalenceFilter",
        "condition": {"values": ["Enterprise"]},
        "values": ["Enterprise"]
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "filters".to_string(),
        "add".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--scope".to_string(),
        "report".to_string(),
        "--target".to_string(),
        "DimCustomer[Segment]".to_string(),
        "--name".to_string(),
        "EquivalenceFilter".to_string(),
        "--value".to_string(),
        "Enterprise".to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn set_drillthrough_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("set-drillthrough-cli-source");
    let op_source = workspace.join("set-drillthrough-op-source");
    let cli_tree = workspace.join("set-drillthrough-cli-output");
    let op_tree = workspace.join("set-drillthrough-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let page = first_page_name(&cli_source);
    let page_handle = format!("page:{page}");
    let operation = json!({
        "op": "setDrillthrough",
        "page": page_handle,
        "target": "DimCustomer[Segment]",
        "fields": ["DimCustomer[Segment]"],
        "table": "DimCustomer",
        "column": "Segment",
        "keepVisible": false,
        "hidden": true
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "drillthrough".to_string(),
        "set".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--page".to_string(),
        operation["page"]
            .as_str()
            .expect("drillthrough page")
            .to_string(),
        "--target".to_string(),
        operation["target"]
            .as_str()
            .expect("drillthrough target")
            .to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn set_interaction_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("set-interaction-cli-source");
    let op_source = workspace.join("set-interaction-op-source");
    let cli_tree = workspace.join("set-interaction-cli-output");
    let op_tree = workspace.join("set-interaction-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let page = first_page_name(&cli_source);
    let (source, target) = first_two_visual_names(&cli_source);
    let page_handle = format!("page:{page}");
    let source_handle = format!("visual:{page}:{source}");
    let target_handle = format!("visual:{page}:{target}");
    let operation = json!({
        "op": "setInteraction",
        "page": page_handle,
        "source": source_handle,
        "target": target_handle,
        "interactionType": "DataFilter"
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "interactions".to_string(),
        "set".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--page".to_string(),
        operation["page"].as_str().expect("page handle").to_string(),
        "--source".to_string(),
        operation["source"]
            .as_str()
            .expect("source handle")
            .to_string(),
        "--target".to_string(),
        operation["target"]
            .as_str()
            .expect("target handle")
            .to_string(),
        "--type".to_string(),
        "DataFilter".to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn reset_interaction_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("reset-interaction-cli-source");
    let op_source = workspace.join("reset-interaction-op-source");
    let cli_tree = workspace.join("reset-interaction-cli-output");
    let op_tree = workspace.join("reset-interaction-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let page = first_page_name(&cli_source);
    let (source, target) = first_two_visual_names(&cli_source);
    let page_handle = format!("page:{page}");
    let source_handle = format!("visual:{page}:{source}");
    let target_handle = format!("visual:{page}:{target}");
    for project in [&cli_source, &op_source] {
        let seed = run_powerbi_owned(&[
            "report".to_string(),
            "interactions".to_string(),
            "set".to_string(),
            "--project".to_string(),
            project.to_string_lossy().into_owned(),
            "--page".to_string(),
            page_handle.clone(),
            "--source".to_string(),
            source_handle.clone(),
            "--target".to_string(),
            target_handle.clone(),
            "--type".to_string(),
            "NoFilter".to_string(),
            "--in-place".to_string(),
            "--json".to_string(),
        ]);
        assert_eq!(seed.exit, 0, "seed interaction failed: {}", seed.stderr);
    }
    let operation = json!({
        "op": "resetInteraction",
        "page": page_handle,
        "source": source_handle,
        "target": target_handle
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "interactions".to_string(),
        "reset".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--page".to_string(),
        operation["page"].as_str().expect("page handle").to_string(),
        "--source".to_string(),
        operation["source"]
            .as_str()
            .expect("source handle")
            .to_string(),
        "--target".to_string(),
        operation["target"]
            .as_str()
            .expect("target handle")
            .to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn apply_theme_preset_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("theme-preset-cli-source");
    let op_source = workspace.join("theme-preset-op-source");
    let cli_tree = workspace.join("theme-preset-cli-output");
    let op_tree = workspace.join("theme-preset-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let operation = json!({
        "op": "applyThemePreset",
        "preset": "risk-dashboard"
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "themes".to_string(),
        "apply-preset".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--preset".to_string(),
        operation["preset"]
            .as_str()
            .expect("theme preset")
            .to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn set_object_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("set-object-cli-source");
    let op_source = workspace.join("set-object-op-source");
    let cli_tree = workspace.join("set-object-cli-output");
    let op_tree = workspace.join("set-object-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let cli_handle = visual_handle(&cli_source);
    let op_handle = visual_handle(&op_source);
    assert_eq!(
        cli_handle, op_handle,
        "fixture visual handles must be stable"
    );
    let operation = json!({
        "op": "setObject",
        "visual": op_handle,
        "object": "categoryLabels",
        "property": "fontSize",
        "value": {"expr": {"Literal": {"Value": "20D"}}}
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "visuals".to_string(),
        "set-object".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--handle".to_string(),
        cli_handle,
        "--object".to_string(),
        operation["object"]
            .as_str()
            .expect("object name")
            .to_string(),
        "--property".to_string(),
        operation["property"]
            .as_str()
            .expect("object property")
            .to_string(),
        "--value".to_string(),
        "20".to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

fn set_position_case(fixture: &ArchetypeFixture, workspace: &Path) -> OperationExecution {
    let cli_source = workspace.join("set-position-cli-source");
    let op_source = workspace.join("set-position-op-source");
    let cli_tree = workspace.join("set-position-cli-output");
    let op_tree = workspace.join("set-position-op-output");
    scaffold_fixture(fixture, &cli_source);
    scaffold_fixture(fixture, &op_source);

    let cli_handle = visual_handle(&cli_source);
    let op_handle = visual_handle(&op_source);
    assert_eq!(
        cli_handle, op_handle,
        "fixture visual handles must be stable"
    );
    let page = first_page_name(&cli_source);
    let visual_name = cli_handle.rsplit(':').next().expect("visual name");
    let operation = json!({
        "op": "setPosition",
        "visual": op_handle,
        "x": 120.0,
        "y": 140.0,
        "width": 360.0,
        "height": 220.0,
        "z": 5,
        "tabOrder": 4,
        "allowOutsidePage": false
    });
    let cli = run_powerbi_owned(&[
        "report".to_string(),
        "visuals".to_string(),
        "set-position".to_string(),
        "--project".to_string(),
        cli_source.to_string_lossy().into_owned(),
        "--page".to_string(),
        page,
        "--visual".to_string(),
        visual_name.to_string(),
        "--x".to_string(),
        "120".to_string(),
        "--y".to_string(),
        "140".to_string(),
        "--width".to_string(),
        "360".to_string(),
        "--height".to_string(),
        "220".to_string(),
        "--z".to_string(),
        "5".to_string(),
        "--tab-order".to_string(),
        "4".to_string(),
        "--out-dir".to_string(),
        cli_tree.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]);
    let op = run_direct_operation(&operation, &op_source, &op_tree);
    OperationExecution {
        operation,
        cli,
        op,
        cli_tree,
        op_tree,
    }
}

#[test]
fn add_filter_ops_equivalence_fixture_is_byte_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let source_arg = source.to_str().expect("source path");
    let first = temp.path().join("first-filter");
    let second = temp.path().join("second-filter");
    for output in [&first, &second] {
        let output_arg = output.to_str().expect("output path");
        let run = run_powerbi(&[
            "report",
            "filters",
            "add",
            "--project",
            source_arg,
            "--scope",
            "report",
            "--target",
            "DimCustomer[Segment]",
            "--value",
            "Enterprise",
            "--out-dir",
            output_arg,
            "--json",
        ]);
        assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    }
    assert_tree_equal(&first, &second, "add-filter CLI determinism");
}

#[test]
fn set_drillthrough_ops_equivalence_fixture_is_byte_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let source_arg = source.to_str().expect("source path");
    let page_handle = format!("page:{}", first_page_name(&source));
    let first = temp.path().join("first-drillthrough");
    let second = temp.path().join("second-drillthrough");
    for output in [&first, &second] {
        let output_arg = output.to_str().expect("output path");
        let run = run_powerbi(&[
            "report",
            "drillthrough",
            "set",
            "--project",
            source_arg,
            "--page",
            &page_handle,
            "--target",
            "DimCustomer[Segment]",
            "--out-dir",
            output_arg,
            "--json",
        ]);
        assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    }
    assert_tree_equal(&first, &second, "set-drillthrough CLI determinism");
}

#[test]
fn remaining_mutation_commands_expose_replayable_operation_kinds() {
    let run = run_powerbi(&["--json", "capabilities"]);
    assert_eq!(run.code, 0, "capabilities: {}", run.stderr);
    let document: serde_json::Value = serde_json::from_str(&run.stdout).expect("capabilities JSON");
    let commands = document["commands"].as_array().expect("command catalog");
    let expected = [
        ("model calculated-columns add", "addCalculatedColumn"),
        ("model tables add-static", "addStaticTable"),
        ("model columns set-sort-by", "setSortBy"),
        ("source-template apply", "sourceTemplateApply"),
        ("report pages add", "addPage"),
        ("report pages update", "updatePage"),
        ("report pages reorder", "reorderPages"),
        ("report pages set-active", "setActivePage"),
        ("report pages delete-empty", "deleteEmptyPage"),
        ("report pages clone", "clonePage"),
        ("report visuals set-bindings", "setBindings"),
        ("report visuals set-display-name", "setDisplayName"),
        ("report visuals set-topn-guard", "setTopNGuard"),
        ("report drilldown set-hierarchy", "setDrilldownHierarchy"),
        ("report visuals clone", "cloneVisual"),
        ("report visuals delete", "deleteVisual"),
        ("report filters update", "updateFilter"),
        ("report filters delete", "deleteFilter"),
        ("report filters clear", "clearFilter"),
        ("report slicers clear", "slicerClear"),
        ("report visuals formatting set-text", "setText"),
        ("report visuals formatting set-color", "setColor"),
        ("report visuals formatting apply", "formattingApply"),
        ("report themes apply", "applyThemeBundle"),
        ("report style apply", "applyStyleBundle"),
        ("report bookmarks set-display-name", "bookmarkMetadata"),
        ("report bookmarks reorder", "bookmarkMetadata"),
        ("report bookmarks delete", "bookmarkMetadata"),
        ("report sanitize apply", "sanitizeAction"),
    ];
    for (path, op_kind) in expected {
        let command = commands
            .iter()
            .find(|command| command["path"] == path)
            .unwrap_or_else(|| panic!("missing catalog path {path}"));
        assert_eq!(command["mutates"], true, "{path} must remain a mutation");
        assert_eq!(command["opKind"], op_kind, "wrong opKind for {path}");
    }
}
