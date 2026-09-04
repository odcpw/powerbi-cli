mod common;

use common::{
    assert_strict_valid, assert_unsupported_feature, build_scatter_bubble, patch_json, run_powerbi,
    run_powerbi_owned, scaffold_sales, stderr_json, stdout_json,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const SCATTER_HANDLE: &str = "visual:ReportSectionPortfolio:VisualContainerPortfolioBubble";

fn scatter_visual_path(project: &Path) -> PathBuf {
    let path = project
        .join("FacilityPortfolio.Report")
        .join("definition")
        .join("pages")
        .join("ReportSectionPortfolio")
        .join("visuals")
        .join("VisualContainerPortfolioBubble")
        .join("visual.json");
    assert!(
        path.is_file(),
        "missing scatter fixture at {}",
        path.display()
    );
    path
}

fn install_runtime_repair_fixture(project: &Path) -> PathBuf {
    let path = scatter_visual_path(project);
    patch_json(&path, |visual| {
        let query_state = visual["visual"]["query"]["queryState"]
            .as_object_mut()
            .expect("scatter queryState");
        let category = query_state.remove("Category").expect("Category role");
        query_state.insert("Details".to_string(), category);

        for (role, column) in [
            ("X", "RiskScore"),
            ("Y", "IncidentRate"),
            ("Size", "ExposureHours"),
        ] {
            let projection = &mut query_state.get_mut(role).expect("value role")["projections"][0];
            projection["field"] = json!({
                "Column": {
                    "Expression": { "SourceRef": { "Entity": "Facilities" } },
                    "Property": column
                }
            });
            projection["queryRef"] = Value::from(format!("Facilities.{column}"));
            projection["nativeQueryRef"] = Value::from(column);
        }
    });
    path
}

#[test]
fn visual_catalog_has_one_complete_fixture_backed_rule_per_generated_type() {
    let output = run_powerbi(&["report", "visuals", "catalog", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let catalog = stdout_json(&output);
    assert_eq!(catalog["schema"], "powerbi-cli.report.visuals.catalog.v2");

    let supported = catalog["supportedVisualTypes"]
        .as_array()
        .expect("supported types");
    let rules = catalog["rules"].as_array().expect("role rules");
    assert_eq!(rules.len(), supported.len());
    let supported_names = supported
        .iter()
        .map(|value| value.as_str().expect("type name"))
        .collect::<BTreeSet<_>>();
    let rule_names = rules
        .iter()
        .map(|rule| rule["visualType"].as_str().expect("rule type"))
        .collect::<BTreeSet<_>>();
    assert_eq!(rule_names, supported_names);

    for rule in rules {
        for field in ["required", "optional", "measureOnly", "mutuallyExclusive"] {
            assert!(rule[field].is_array(), "missing {field}: {rule}");
        }
        assert!(rule["maxProjections"].is_object(), "{rule}");
        assert!(
            rule["runtimeParity"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "{rule}"
        );
        assert_eq!(rule["refusalCode"], "unsupported_feature");
        assert!(
            rule["evidence"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "{rule}"
        );
    }

    let desktop_reference_types = rules
        .iter()
        .filter(|rule| rule["fixtureKind"] == "desktop-authored-reference")
        .map(|rule| rule["visualType"].as_str().expect("reference type"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        desktop_reference_types,
        ["donutChart", "pieChart", "pivotTable", "slicer"]
            .into_iter()
            .collect()
    );

    let scatter = rules
        .iter()
        .find(|rule| rule["visualType"] == "scatterChart")
        .expect("scatter rule");
    assert_eq!(scatter["required"], json!(["X", "Y"]));
    assert_eq!(scatter["maxProjections"]["Category"], 1);
    let runtime_ids = scatter["runtimeParity"]
        .as_array()
        .expect("runtime parity")
        .iter()
        .map(|rule| rule["id"].as_str().expect("rule id"))
        .collect::<BTreeSet<_>>();
    assert!(runtime_ids.contains("scatter.details-role-refused"));
    assert!(runtime_ids.contains("scatter.category-aggregated-value-axes"));
}

#[test]
fn repair_bindings_dry_run_is_deterministic_minimal_and_applies_via_typed_op() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_scatter_bubble(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visual_path = install_runtime_repair_fixture(&project);
    let source_before = fs::read(&visual_path).expect("source visual bytes");

    let missing_dry_run = run_powerbi(&[
        "report",
        "visuals",
        "repair-bindings",
        "--project",
        project_arg,
        "--handle",
        SCATTER_HANDLE,
        "--json",
    ]);
    assert_eq!(missing_dry_run.code, 2);
    assert_eq!(
        stderr_json(&missing_dry_run)["error"]["code"],
        "invalid_args"
    );

    let repair_args = [
        "report",
        "visuals",
        "repair-bindings",
        "--project",
        project_arg,
        "--handle",
        SCATTER_HANDLE,
        "--dry-run",
        "--json",
    ];
    let first = run_powerbi(&repair_args);
    let second = run_powerbi(&repair_args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "repair plan must be deterministic"
    );
    assert_eq!(
        fs::read(&visual_path).expect("source visual after dry run"),
        source_before,
        "dry-run must not write"
    );

    let plan = stdout_json(&first);
    assert_eq!(
        plan["schema"],
        "powerbi-cli.report.visuals.bindingRepair.v1"
    );
    assert_eq!(plan["dryRun"], true);
    assert_eq!(plan["changed"], true);
    assert_eq!(plan["repairs"].as_array().expect("repairs").len(), 4);
    let repair_rule_ids = plan["repairs"]
        .as_array()
        .expect("repairs")
        .iter()
        .map(|repair| repair["ruleId"].as_str().expect("repair rule"))
        .collect::<Vec<_>>();
    assert_eq!(
        repair_rule_ids,
        vec![
            "scatter.details-role-refused",
            "scatter.category-aggregated-value-axes",
            "scatter.category-aggregated-value-axes",
            "scatter.category-aggregated-value-axes"
        ]
    );
    let op = &plan["repairPlan"]["op"];
    assert_eq!(
        op["schema"],
        "powerbi-cli.op.report.visuals.set-bindings.v1"
    );
    assert_eq!(op["kind"], "report.visuals.setBindings");
    assert_eq!(op["target"], SCATTER_HANDLE);
    let bindings = op["bindings"].as_array().expect("op bindings");
    assert!(bindings.iter().any(|binding| binding["role"] == "Category"));
    assert!(!bindings.iter().any(|binding| binding["role"] == "Details"));
    assert!(
        plan["previewCommand"].as_str().is_some_and(
            |command| command.contains("set-bindings") && command.contains("--dry-run")
        )
    );
    assert!(
        plan["applyCommand"].as_str().is_some_and(
            |command| command.contains("set-bindings") && command.contains("--in-place")
        )
    );

    let repaired = temp.path().join("repaired");
    let bindings_json = serde_json::to_string(&op["bindings"]).expect("bindings JSON");
    let apply_args = vec![
        "report".to_string(),
        "visuals".to_string(),
        "set-bindings".to_string(),
        "--project".to_string(),
        project_arg.to_string(),
        "--handle".to_string(),
        SCATTER_HANDLE.to_string(),
        "--bindings-json".to_string(),
        bindings_json,
        "--out-dir".to_string(),
        repaired.to_str().expect("repaired path").to_string(),
        "--json".to_string(),
    ];
    let applied = run_powerbi_owned(&apply_args);
    assert_eq!(applied.code, 0, "stderr: {}", applied.stderr);
    assert_strict_valid(&repaired);
    let repaired_visual: Value = serde_json::from_str(
        &fs::read_to_string(scatter_visual_path(&repaired)).expect("repaired visual"),
    )
    .expect("repaired visual JSON");
    let query_state = &repaired_visual["visual"]["query"]["queryState"];
    assert!(query_state["Category"].is_object());
    assert!(query_state["Details"].is_null());
    for role in ["X", "Y", "Size"] {
        assert!(
            query_state[role]["projections"][0]["field"]["Aggregation"]["Expression"]["Column"]
                .is_object(),
            "{role} was not wrapped as Sum: {}",
            query_state[role]
        );
    }
}

#[test]
fn repair_bindings_refuses_to_invent_a_missing_required_field() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = build_scatter_bubble(temp.path());
    let project_arg = project.to_str().expect("project path");
    patch_json(&scatter_visual_path(&project), |visual| {
        visual["visual"]["query"]["queryState"]
            .as_object_mut()
            .expect("queryState")
            .remove("Y");
    });

    let output = run_powerbi(&[
        "report",
        "visuals",
        "repair-bindings",
        "--project",
        project_arg,
        "--handle",
        SCATTER_HANDLE,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 2);
    let error = assert_unsupported_feature(&output.stderr, "cannot propose a deterministic");
    assert!(
        error["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("never invents"))
    );
    assert_eq!(
        error["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .len(),
        2
    );
}

#[test]
fn spec_validate_refuses_bare_columns_in_every_measure_only_reference_family() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        (
            "combo",
            "combo",
            json!([
                { "role": "Category", "field": "DimDate[Month]" },
                { "role": "Y", "field": "FactSales[Revenue]" },
                { "role": "Y2", "field": "FactSales[Total Units]" }
            ]),
            "bare columns are not fixture-proven",
        ),
        (
            "pie",
            "pie",
            json!([
                { "role": "Category", "field": "DimCustomer[Segment]" },
                { "role": "Y", "field": "FactSales[Revenue]" }
            ]),
            "bare columns are not proven by the Desktop-authored reference",
        ),
        (
            "matrix",
            "matrix",
            json!([
                { "role": "Rows", "field": "DimCustomer[Segment]" },
                { "role": "Values", "field": "FactSales[Revenue]" }
            ]),
            "bare columns are not proven by the Desktop-authored reference",
        ),
    ];

    for (slug, visual_type, bindings, expected) in cases {
        let spec = temp.path().join(format!("{slug}.dashboard.json"));
        fs::write(
            &spec,
            serde_json::to_string_pretty(&json!({
                "schema": "powerbi-cli.dashboard.v1",
                "report": { "name": "SalesOperations" },
                "pages": [{
                    "id": "overview",
                    "visuals": [{
                        "id": slug,
                        "type": visual_type,
                        "bindings": bindings
                    }]
                }]
            }))
            .expect("spec JSON"),
        )
        .expect("write spec");
        let output = run_powerbi(&[
            "report",
            "spec",
            "validate",
            "--schema",
            "examples/sales.schema.json",
            "--spec",
            spec.to_str().expect("spec path"),
            "--json",
        ]);
        assert_eq!(
            output.code, 10,
            "{slug} stdout: {}\nstderr: {}",
            output.stdout, output.stderr
        );
        let result = stdout_json(&output);
        assert_eq!(result["ok"], false);
        assert!(
            result["errors"]
                .as_array()
                .expect("validation errors")
                .iter()
                .any(|error| error.as_str().is_some_and(|text| text.contains(expected))),
            "{slug} did not return the measure-only refusal: {result}"
        );
    }
}

#[test]
fn set_bindings_enforces_the_catalog_measure_only_role_map() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let visuals = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let line_handle = stdout_json(&visuals)["visuals"]
        .as_array()
        .expect("visuals")
        .iter()
        .find(|visual| visual["visualType"] == "lineChart")
        .and_then(|visual| visual["handle"].as_str())
        .expect("line visual")
        .to_string();

    let output = run_powerbi(&[
        "report",
        "visuals",
        "set-bindings",
        "--project",
        project_arg,
        "--handle",
        &line_handle,
        "--binding",
        "role=Category,table=DimDate,column=Month",
        "--binding",
        "role=Y,table=FactSales,column=Revenue",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(output.code, 2);
    let error = assert_unsupported_feature(&output.stderr, "not Desktop-proven");
    assert!(
        error["error"]["suggestedCommands"]
            .as_array()
            .expect("suggested commands")
            .iter()
            .any(|command| command
                .as_str()
                .is_some_and(|text| text.contains("report visuals catalog")))
    );
}
