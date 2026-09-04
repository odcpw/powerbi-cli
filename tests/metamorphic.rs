//! Table-driven spec compiler versus typed operation metamorphic tests.

mod common;

use common::{
    ArchetypeFixture, MetamorphicCase, MetamorphicExecution, build_fixture_with_spec,
    first_page_name, run_direct_operation, run_metamorphic_cases,
};
use serde_json::{Value, json};
use std::path::Path;

#[test]
fn compiled_dashboard_sections_match_their_operation_equivalents() {
    let cases = [MetamorphicCase {
        name: "pages.interactions/setInteraction/sales",
        fixture: "sales",
        fragment_pointer: "/pages/0/interactions",
        operation_tag: "setInteraction",
        execute: set_interaction_case,
    }];
    run_metamorphic_cases(&cases);
}

fn set_interaction_case(fixture: &ArchetypeFixture, workspace: &Path) -> MetamorphicExecution {
    let spec_with_path = workspace.join("sales-with-interaction.dashboard.json");
    let spec_without_path = workspace.join("sales-without-interaction.dashboard.json");
    let spec_tree = workspace.join("sales-spec-with-interaction");
    let base_tree = workspace.join("sales-spec-without-interaction");
    let applied_tree = workspace.join("sales-operation-applied");
    let fragment = json!([{
        "source": "revenue_card",
        "target": "revenue_trend",
        "type": "DataFilter"
    }]);
    fixture
        .spec_builder()
        .set_interactions("overview", fragment.clone())
        .write_to(&spec_with_path);
    fixture.spec_builder().write_to(&spec_without_path);

    let spec_build = build_fixture_with_spec(fixture, &spec_with_path, &spec_tree);
    let base_build = build_fixture_with_spec(fixture, &spec_without_path, &base_tree);
    let page = first_page_name(&base_tree);
    let page_handle = format!("page:{page}");
    let operation = json!({
        "op": "setInteraction",
        "page": page_handle,
        "source": format!("visual:{page}:VisualContainerRevenueCard"),
        "target": format!("visual:{page}:VisualContainerRevenueTrend"),
        "interactionType": "DataFilter"
    });
    let op = run_direct_operation(&operation, &base_tree, &applied_tree);

    MetamorphicExecution {
        fragment: Value::Array(fragment.as_array().expect("interaction fragment").clone()),
        operation,
        spec_build,
        base_build,
        op,
        spec_tree,
        applied_tree,
    }
}
