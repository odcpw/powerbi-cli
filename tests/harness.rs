mod common;

use common::{archetype_names, assert_json_snapshot, load_archetype, run_powerbi, stdout_json};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn fixture_loader_resolves_every_complete_archetype() {
    for name in archetype_names() {
        let fixture = load_archetype(name);
        assert_eq!(fixture.name, *name);
        assert!(fixture.schema.is_file());
        assert!(fixture.profile.is_file());
        assert!(fixture.spec.is_file());
        assert!(fixture.expected_summary.is_file());
    }

    let cataloged = archetype_names()
        .iter()
        .copied()
        .filter(|name| *name != "sales")
        .collect::<BTreeSet<_>>();
    let discovered = std::fs::read_dir("examples/archetypes")
        .expect("archetype directory")
        .map(|entry| entry.expect("archetype entry").path())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".dashboard.json"))
                .map(str::to_string)
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        discovered,
        cataloged.into_iter().map(str::to_string).collect(),
        "archetype_names must include every checked-in dashboard fixture"
    );
}

#[test]
fn shared_runner_captures_argv_streams_exit_and_elapsed_time() {
    let run = run_powerbi(&["version", "--json"]);
    assert_eq!(run.exit, 0, "stderr: {}", run.stderr);
    assert_eq!(
        std::path::Path::new(&run.argv[0])
            .file_stem()
            .and_then(|stem| stem.to_str()),
        Some("powerbi-cli")
    );
    assert_eq!(&run.argv[1..], ["version", "--json"]);
    let value = stdout_json(&run);
    assert_eq!(value["binary"], "powerbi-cli");
    assert_eq!(
        value["contractVersion"],
        "powerbi-cli.agent-capabilities.v1"
    );
    assert!(run.stderr.is_empty());
    assert!(run.elapsed < std::time::Duration::from_secs(5));
}

#[test]
fn v2_spec_builder_starts_from_a_real_fixture_and_authors_sections() {
    let fixture = load_archetype("sales");
    let spec = fixture
        .v2_spec_builder()
        .add_visual(
            "overview",
            json!({
                "id": "margin_card",
                "type": "card",
                "title": "Margin",
                "bindings": [{"role": "Values", "field": "FactSales[Total Revenue]"}],
                "layout": {"x": 328, "y": 32, "width": 280, "height": 120}
            }),
        )
        .add_filter(
            "overview",
            json!({"field": "DimCustomer[Segment]", "operator": "in", "values": ["Example"]}),
        )
        .set_style(json!({"preset": "corporate-neutral"}))
        .build();

    assert_eq!(spec["schema"], "powerbi-cli.dashboard.v2");
    assert_eq!(spec["pages"][0]["visuals"].as_array().unwrap().len(), 4);
    assert_eq!(spec["pages"][0]["filters"].as_array().unwrap().len(), 1);
    assert_eq!(spec["style"]["preset"], "corporate-neutral");
    assert_json_snapshot(
        "harness-v2-builder",
        &json!({
            "schema": spec["schema"],
            "pageId": spec["pages"][0]["id"],
            "visualIds": spec["pages"][0]["visuals"]
                .as_array()
                .unwrap()
                .iter()
                .map(|visual| visual["id"].clone())
                .collect::<Vec<_>>(),
            "filterCount": spec["pages"][0]["filters"].as_array().unwrap().len(),
            "jsonPointer": "/pages/0",
            "style": spec["style"]
        }),
    );
}

#[test]
#[should_panic(expected = "contains an absolute path")]
fn snapshot_helper_refuses_machine_specific_paths() {
    assert_json_snapshot("never-written", &json!({"path": "/tmp/machine-specific"}));
}
