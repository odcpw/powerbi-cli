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
    let cases = [
        OperationEquivalenceCase {
            name: "addMeasure/sales",
            fixture: "sales",
            operation_tag: "addMeasure",
            execute: add_measure_case,
        },
        OperationEquivalenceCase {
            name: "addRelationship/sales",
            fixture: "sales",
            operation_tag: "addRelationship",
            execute: add_relationship_case,
        },
        OperationEquivalenceCase {
            name: "addVisual/sales",
            fixture: "sales",
            operation_tag: "addVisual",
            execute: add_visual_case,
        },
        OperationEquivalenceCase {
            name: "addFilter/sales",
            fixture: "sales",
            operation_tag: "addFilter",
            execute: add_filter_case,
        },
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
            name: "resetInteraction/sales",
            fixture: "sales",
            operation_tag: "resetInteraction",
            execute: reset_interaction_case,
        },
        OperationEquivalenceCase {
            name: "applyThemePreset/sales",
            fixture: "sales",
            operation_tag: "applyThemePreset",
            execute: apply_theme_preset_case,
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
    ];
    run_operation_equivalence(&cases);
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
