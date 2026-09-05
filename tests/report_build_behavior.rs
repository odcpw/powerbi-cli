mod common;
use common::*;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn fixture_spec() -> Value {
    serde_json::from_str(include_str!(
        "../examples/visual-behavior.dashboard.v2.json"
    ))
    .unwrap()
}

fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn case(fixture: &ArchetypeFixture, root: &Path, section: &str, tag: &str) -> MetamorphicExecution {
    let mut without = fixture_spec();
    let vi = usize::from(section == "sort");
    let fragment = without["pages"][0]["visuals"][vi][section].clone();
    for visual in without["pages"][0]["visuals"].as_array_mut().unwrap() {
        for key in ["sort", "drilldown", "topnGuard", "filters"] {
            visual.as_object_mut().unwrap().remove(key);
        }
    }
    let mut with = without.clone();
    with["pages"][0]["visuals"][vi][section] = fragment.clone();
    let with_path = root.join("with.json");
    let without_path = root.join("without.json");
    write(&with_path, &with);
    write(&without_path, &without);
    let spec_tree = root.join("compiled");
    let base_tree = root.join("base");
    let applied_tree = root.join("typed");
    let spec_build = build_fixture_with_spec(fixture, &with_path, &spec_tree);
    assert_eq!(spec_build.code, 0, "{}", spec_build.stderr);
    let base_build = build_fixture_with_spec(fixture, &without_path, &base_tree);
    assert_eq!(base_build.code, 0, "{}", base_build.stderr);
    let output = stdout_json(&spec_build);
    let operation = output["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["op"] == tag)
        .unwrap()
        .clone();
    assert_json_snapshot(&format!("compile-visual-{section}"), &operation);
    let op = run_direct_operation(&operation, &base_tree, &applied_tree);

    let handle = if section == "sort" {
        "visual:ReportSectionOverview:VisualContainerShare"
    } else {
        "visual:ReportSectionOverview:VisualContainerTrend"
    };
    let mut cli = match section {
        "drilldown" => vec![
            "report",
            "drilldown",
            "set-hierarchy",
            "--handle",
            handle,
            "--field",
            "DimCustomer[Segment]",
            "--field",
            "DimCustomer[CustomerName]",
        ],
        "topnGuard" => vec![
            "report",
            "visuals",
            "set-topn-guard",
            "--handle",
            handle,
            "--field",
            "DimCustomer[Segment]",
            "--order-by",
            "FactSales[Total Revenue]",
            "--top",
            "5",
        ],
        "sort" => vec![
            "report",
            "visuals",
            "set-bindings",
            "--handle",
            handle,
            "--binding",
            "role=Category,table=DimCustomer,column=Segment",
            "--binding",
            "role=Y,table=FactSales,measure=Total Revenue,sortDirection=Descending",
        ],
        "filters" => vec![
            "report",
            "filters",
            "add",
            "--scope",
            "visual",
            "--visual",
            handle,
            "--table",
            "DimCustomer",
            "--column",
            "Segment",
            "--values-json",
            "[\"Example\"]",
        ],
        _ => unreachable!(),
    };
    cli.extend([
        "--project",
        base_tree.to_str().unwrap(),
        "--in-place",
        "--json",
    ]);
    let mutated = run_powerbi(&cli);
    assert_eq!(mutated.code, 0, "{}", mutated.stderr);
    assert_tree_equal_with_ignored(
        &spec_tree,
        &base_tree,
        section,
        &["powerbi-cli.manifest.copy.json"],
    );
    MetamorphicExecution {
        fragment,
        operation,
        spec_build,
        base_build,
        op,
        spec_tree,
        applied_tree,
    }
}

#[test]
fn each_visual_behavior_has_registered_kernel_and_cli_byte_parity() {
    run_metamorphic_cases(&[
        MetamorphicCase {
            name: "visual.drilldown",
            fixture: "sales",
            fragment_pointer: "/pages/0/visuals/0/drilldown",
            operation_tag: "setDrilldownHierarchy",
            execute: |f, p| case(f, p, "drilldown", "setDrilldownHierarchy"),
        },
        MetamorphicCase {
            name: "visual.sort",
            fixture: "sales",
            fragment_pointer: "/pages/0/visuals/1/sort",
            operation_tag: "setBindings",
            execute: |f, p| case(f, p, "sort", "setBindings"),
        },
        MetamorphicCase {
            name: "visual.topnGuard",
            fixture: "sales",
            fragment_pointer: "/pages/0/visuals/0/topnGuard",
            operation_tag: "setTopNGuard",
            execute: |f, p| case(f, p, "topnGuard", "setTopNGuard"),
        },
        MetamorphicCase {
            name: "visual.filters",
            fixture: "sales",
            fragment_pointer: "/pages/0/visuals/0/filters",
            operation_tag: "addFilter",
            execute: |f, p| case(f, p, "filters", "addFilter"),
        },
    ]);
}

#[test]
fn visual_behavior_fixture_is_deterministic_and_dry_run_is_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let f = load_archetype("sales");
    let spec = Path::new("examples/visual-behavior.dashboard.v2.json");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for path in [&first, &second] {
        let built = build_fixture_with_spec(&f, spec, path);
        assert_eq!(built.code, 0, "{}", built.stderr);
    }
    assert_eq!(hash_tree(&first), hash_tree(&second));
    let before = hash_tree(temp.path());
    let dry = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry.code, 0, "{}", dry.stderr);
    assert_eq!(hash_tree(temp.path()), before);
    assert_eq!(stdout_json(&dry)["operations"].as_array().unwrap().len(), 5);
    let explain = run_powerbi(&[
        "report",
        "spec",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(explain.code, 0, "{}", explain.stderr);
    for tag in [
        "setDrilldownHierarchy",
        "setTopNGuard",
        "setBindings",
        "addFilter",
    ] {
        assert!(explain.stdout.contains(tag), "{}", explain.stdout);
    }
}

#[test]
fn invalid_visual_behavior_refuses_before_writes_with_exact_pointers() {
    for (vi, key, fragment, suffix, code) in [
        (
            0,
            "drilldown",
            json!({"fields":["DimCustomer[Segment]", "DimCustomer[Missing]"]}),
            "/fields/1",
            "invalid_args",
        ),
        (
            0,
            "drilldown",
            json!({"fields":["DimCustomer[Segment]"]}),
            "/fields",
            "invalid_args",
        ),
        (
            0,
            "drilldown",
            json!({"fields":["DimCustomer[Segment]", "DimCustomer[Segment]"]}),
            "/fields/1",
            "invalid_args",
        ),
        (
            0,
            "drilldown",
            json!({"fields":["DimCustomer[Segment]", "FactSales[Total Revenue]"]}),
            "/fields/1",
            "invalid_args",
        ),
        (
            0,
            "topnGuard",
            json!({"orderBy":"FactSales[Total Revenue]","top":0}),
            "/top",
            "invalid_args",
        ),
        (
            0,
            "topnGuard",
            json!({"orderBy":"DimCustomer[Segment]","top":5}),
            "/orderBy",
            "invalid_args",
        ),
        (
            1,
            "sort",
            json!({"field":"FactSales[Total Revenue]","direction":"Ascending"}),
            "/direction",
            "unsupported_feature",
        ),
        (
            1,
            "sort",
            json!({"field":"FactSales[Total Units]","direction":"Descending"}),
            "/field",
            "invalid_args",
        ),
        (
            0,
            "sort",
            json!({"field":"FactSales[Total Revenue]","direction":"Descending"}),
            "",
            "unsupported_feature",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut spec = fixture_spec();
        spec["pages"][0]["visuals"][vi][key] = fragment;
        let path = temp.path().join("bad.json");
        write(&path, &spec);
        let out = temp.path().join("absent");
        for mode in ["--dry-run", "--out-dir"] {
            let mut args = vec![
                "report",
                "build",
                "--schema",
                "examples/sales.schema.json",
                "--spec",
                path.to_str().unwrap(),
                mode,
            ];
            if mode == "--out-dir" {
                args.push(out.to_str().unwrap());
            }
            args.push("--json");
            let result = run_powerbi(&args);
            assert_ne!(result.code, 0);
            let error = stderr_json(&result);
            assert_eq!(error["error"]["code"], code, "{error}");
            assert_eq!(
                error["error"]["pointer"],
                format!("/pages/0/visuals/{vi}/{key}{suffix}"),
                "{error}"
            );
            assert!(error["error"]["hint"].is_string(), "{error}");
            assert!(error["error"]["suggestedCommands"].is_array(), "{error}");
            assert!(!out.exists());
        }
    }
}

#[test]
fn every_visual_behavior_rejects_unknown_keys_with_rfc6901_pointers() {
    for (vi, section, pointer) in [
        (0, "drilldown", "/pages/0/visuals/0/drilldown/x~1~0"),
        (0, "topnGuard", "/pages/0/visuals/0/topnGuard/x~1~0"),
        (1, "sort", "/pages/0/visuals/1/sort/x~1~0"),
        (0, "filters", "/pages/0/visuals/0/filters/0/x~1~0"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut spec = fixture_spec();
        let section = &mut spec["pages"][0]["visuals"][vi][section];
        let object = if section.is_array() {
            &mut section[0]
        } else {
            section
        };
        object["x/~"] = json!(true);
        let path = temp.path().join("unknown.json");
        write(&path, &spec);
        let out = temp.path().join("absent");
        let result = run_powerbi(&[
            "report",
            "build",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            path.to_str().unwrap(),
            "--out-dir",
            out.to_str().unwrap(),
            "--json",
        ]);
        let error = stderr_json(&result);
        assert_eq!(error["error"]["code"], "spec.unknown_field", "{error}");
        assert_eq!(error["error"]["pointer"], pointer, "{error}");
        assert!(!out.exists());
    }
}

#[test]
fn scatter_hierarchies_and_table_sorts_are_explicitly_refused() {
    for (kind, bindings, section) in [
        (
            "scatterChart",
            json!([
                {"role":"Category","field":"DimCustomer[Segment]"},
                {"role":"X","field":"FactSales[Total Units]"},
                {"role":"Y","field":"FactSales[Total Revenue]"}
            ]),
            "drilldown",
        ),
        (
            "tableEx",
            json!([
                {"role":"Values","field":"DimCustomer[Segment]"},
                {"role":"Values","field":"FactSales[Total Revenue]"}
            ]),
            "sort",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut spec = fixture_spec();
        spec["pages"][0]["visuals"] = json!([{
            "id":"unsupported", "type":kind, "bindings":bindings,
            section: if section == "sort" {json!({"field":"FactSales[Total Revenue]","direction":"Descending"})}
                else {json!({"fields":["DimCustomer[Segment]","DimCustomer[CustomerName]"]})}
        }]);
        let path = temp.path().join("unsupported.json");
        write(&path, &spec);
        let result = run_powerbi(&[
            "report",
            "build",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            path.to_str().unwrap(),
            "--dry-run",
            "--json",
        ]);
        let error = stderr_json(&result);
        assert_eq!(error["error"]["code"], "unsupported_feature", "{error}");
        assert_eq!(
            error["error"]["pointer"],
            format!("/pages/0/visuals/0/{section}")
        );
    }
}

#[test]
fn behavior_handles_resolve_when_optional_page_and_visual_ids_are_omitted() {
    let temp = tempfile::tempdir().unwrap();
    let mut spec = fixture_spec();
    spec["pages"][0].as_object_mut().unwrap().remove("id");
    // Filters have their own established owner-name resolver; this case
    // exercises the newly lowered mutations against scaffold default names.
    for visual in spec["pages"][0]["visuals"].as_array_mut().unwrap() {
        visual.as_object_mut().unwrap().remove("id");
        visual.as_object_mut().unwrap().remove("filters");
    }
    let path = temp.path().join("anonymous.json");
    write(&path, &spec);
    let built = build_fixture_with_spec(
        &load_archetype("sales"),
        &path,
        &temp.path().join("project"),
    );
    assert_eq!(built.code, 0, "{}", built.stderr);
    let explain = run_powerbi(&[
        "report",
        "spec",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        path.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(explain.code, 0, "{}", explain.stderr);
}
