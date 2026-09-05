mod common;

use common::{run_powerbi, stdout_json};
use serde_json::Value;

fn explanation<'a>(value: &'a Value, id: &str) -> &'a Value {
    value["ruleExplanations"]
        .as_array()
        .expect("rule explanations")
        .iter()
        .find(|rule| rule["ruleId"] == id)
        .unwrap_or_else(|| panic!("missing fired rule {id}"))
}

#[test]
fn report_plan_explains_fired_rules_with_evidence_and_slot_only_v2_candidate() {
    let args = [
        "report",
        "plan",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--objective",
        "Executive sales overview",
        "--explain-rules",
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "planner output must be deterministic"
    );
    let alias = run_powerbi(&[
        "report",
        "plan",
        "explain",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        "examples/sales.profile.json",
        "--objective",
        "Executive sales overview",
        "--json",
    ]);
    assert_eq!(alias.code, 0, "alias stderr: {}", alias.stderr);
    assert_eq!(
        first.stdout, alias.stdout,
        "explain alias must be equivalent"
    );
    let value = stdout_json(&first);
    assert_eq!(value["planner"]["schema"], "powerbi-cli.planner-rules.v1");
    assert_eq!(value["explainRules"], true);
    for id in [
        "planner.time-series",
        "planner.category-ranking",
        "planner.scatter-focus",
        "planner.overview",
    ] {
        let rule = explanation(&value, id);
        assert!(rule["score"].as_i64().is_some(), "score for {id}");
        assert!(!rule["evidence"].as_array().expect("evidence").is_empty());
        let proposal = &rule["proposal"];
        assert_eq!(proposal["ruleIds"][0], id);
        assert!(
            !proposal["evidence"]
                .as_array()
                .expect("proposal evidence")
                .is_empty()
        );
        assert!(proposal["priority"].as_i64().is_some());
        assert!(proposal["sizeClass"].is_string());
    }
    assert_eq!(value["specV2"]["schema"], "powerbi-cli.dashboard.v2");
    assert_eq!(value["specV2"]["style"]["preset"], "planner-default");
    assert_eq!(value["specV2"]["layout"]["grid"]["columns"], 12);
    assert_eq!(
        value["specV2"]["proof"]["desktop"]["level"],
        "desktop-golden-pending"
    );
    for visual in value["specV2"]["pages"]
        .as_array()
        .expect("v2 pages")
        .iter()
        .flat_map(|page| page["visuals"].as_array().into_iter().flatten())
    {
        assert!(visual["slot"].is_string());
        assert!(visual.get("layout").is_none());
        assert!(visual.get("x").is_none());
        assert!(visual.get("y").is_none());
        assert!(visual.get("width").is_none());
        assert!(visual.get("height").is_none());
    }
}

#[test]
fn planner_catalog_is_discoverable_with_expected_scores() {
    let capabilities = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(capabilities.code, 0, "stderr: {}", capabilities.stderr);
    let value = stdout_json(&capabilities);
    let catalog = &value["schemaManifest"]["plannerRuleCatalog"];
    assert_eq!(catalog["schema"], "powerbi-cli.planner-rules.v1");
    let rules = catalog["rules"].as_array().expect("catalog rules");
    assert_eq!(rules.len(), 13);
    let expected = [
        ("planner.time-series", 92),
        ("planner.category-ranking", 84),
        ("planner.scatter-focus", 88),
        ("planner.detail-table", 78),
        ("planner.measure-target", 86),
        ("planner.measure-total", 74),
        ("planner.alert-exception-list", 89),
        ("planner.high-cardinality-drillthrough", 72),
        ("planner.shape-flat-template", 61),
        ("planner.shape-snowflake-template", 61),
        ("planner.shape-multi-fact-template", 61),
        ("planner.shape-ambiguous-template", 61),
        ("planner.overview", 60),
    ];
    for (id, score) in expected {
        let rule = rules
            .iter()
            .find(|rule| rule["id"] == id)
            .unwrap_or_else(|| panic!("catalog missing {id}"));
        assert_eq!(rule["score"], score);
    }
}

fn table(name: &str, columns: Value) -> Value {
    serde_json::json!({"name": name, "columns": columns})
}

fn relation(from_table: &str, from_column: &str, to_table: &str, to_column: &str) -> Value {
    serde_json::json!({
        "fromTable": from_table,
        "fromColumn": from_column,
        "toTable": to_table,
        "toColumn": to_column,
        "cardinality": "manyToOne"
    })
}

fn planner_shape_schema(kind: &str) -> Value {
    match kind {
        "flat" => serde_json::json!({
            "name": "FlatEvents",
            "tables": [table("Events", serde_json::json!([
                {"name":"EventDate","dataType":"date"},
                {"name":"Amount","dataType":"decimal"},
                {"name":"Category","dataType":"string"}
            ]))]
        }),
        "star" => serde_json::json!({
            "name": "StarSales",
            "tables": [
                table("FactSales", serde_json::json!([
                    {"name":"DateKey","dataType":"int64"},
                    {"name":"CustomerKey","dataType":"int64"},
                    {"name":"Revenue","dataType":"decimal"}
                ])),
                table("DimDate", serde_json::json!([
                    {"name":"DateKey","dataType":"int64","isKey":true},
                    {"name":"Date","dataType":"date"}
                ])),
                table("DimCustomer", serde_json::json!([
                    {"name":"CustomerKey","dataType":"int64","isKey":true},
                    {"name":"CustomerName","dataType":"string"}
                ]))
            ],
            "relationships": [
                relation("FactSales", "DateKey", "DimDate", "DateKey"),
                relation("FactSales", "CustomerKey", "DimCustomer", "CustomerKey")
            ]
        }),
        "snowflake" => serde_json::json!({
            "name": "SnowflakeSales",
            "tables": [
                table("FactSales", serde_json::json!([
                    {"name":"CustomerKey","dataType":"int64"},
                    {"name":"Revenue","dataType":"decimal"}
                ])),
                table("DimCustomer", serde_json::json!([
                    {"name":"CustomerKey","dataType":"int64","isKey":true},
                    {"name":"RegionKey","dataType":"int64"},
                    {"name":"CustomerName","dataType":"string"}
                ])),
                table("DimRegion", serde_json::json!([
                    {"name":"RegionKey","dataType":"int64","isKey":true},
                    {"name":"Region","dataType":"string"}
                ]))
            ],
            "relationships": [
                relation("FactSales", "CustomerKey", "DimCustomer", "CustomerKey"),
                relation("DimCustomer", "RegionKey", "DimRegion", "RegionKey")
            ]
        }),
        "multi-fact" => serde_json::json!({
            "name": "MultiFactSales",
            "tables": [
                table("FactSales", serde_json::json!([
                    {"name":"DateKey","dataType":"int64"},
                    {"name":"Revenue","dataType":"decimal"}
                ])),
                table("FactTargets", serde_json::json!([
                    {"name":"DateKey","dataType":"int64"},
                    {"name":"Target","dataType":"decimal"}
                ])),
                table("DimDate", serde_json::json!([
                    {"name":"DateKey","dataType":"int64","isKey":true},
                    {"name":"Date","dataType":"date"}
                ]))
            ],
            "relationships": [
                relation("FactSales", "DateKey", "DimDate", "DateKey"),
                relation("FactTargets", "DateKey", "DimDate", "DateKey")
            ]
        }),
        "no-date" => serde_json::json!({
            "name": "NoDateSales",
            "tables": [
                table("FactSales", serde_json::json!([
                    {"name":"CustomerKey","dataType":"int64"},
                    {"name":"Revenue","dataType":"decimal"}
                ])),
                table("DimCustomer", serde_json::json!([
                    {"name":"CustomerKey","dataType":"int64","isKey":true},
                    {"name":"CustomerName","dataType":"string"}
                ]))
            ],
            "relationships": [relation("FactSales", "CustomerKey", "DimCustomer", "CustomerKey")]
        }),
        "ambiguous" => serde_json::json!({
            "name": "AmbiguousModel",
            "tables": [
                table("Events", serde_json::json!([
                    {"name":"Value","dataType":"decimal"},
                    {"name":"Label","dataType":"string"}
                ])),
                table("Lookup", serde_json::json!([
                    {"name":"Code","dataType":"string"},
                    {"name":"Description","dataType":"string"}
                ]))
            ]
        }),
        other => panic!("unknown planner shape fixture {other}"),
    }
}

#[test]
fn six_shape_planner_fixtures_require_evidence_before_emitting_distinct_layouts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        ("flat", "flat"),
        ("star", "star"),
        ("snowflake", "snowflake"),
        ("multi-fact", "multi-fact"),
        ("no-date", "star"),
        ("ambiguous", "ambiguous"),
    ];
    let mut signatures = Vec::new();
    for (fixture_name, expected_kind) in cases {
        let schema_path = temp.path().join(format!("{fixture_name}.schema.json"));
        let mut schema = planner_shape_schema(fixture_name);
        let table_name = schema["tables"][0]["name"].as_str().unwrap().to_string();
        schema["tables"][0]["measures"] = serde_json::json!([{
            "name": "Row Count", "expression": format!("COUNTROWS('{table_name}')")
        }]);
        std::fs::write(
            &schema_path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&schema).expect("schema JSON")
            ),
        )
        .expect("write schema fixture");
        let args = [
            "report",
            "plan",
            "--schema",
            schema_path.to_str().expect("UTF-8 schema path"),
            "--objective",
            "shape golden",
            "--json",
        ];
        let mut result = run_powerbi(&args);
        if matches!(
            fixture_name,
            "snowflake" | "no-date" | "ambiguous" | "multi-fact"
        ) {
            assert_eq!(result.code, 10, "{fixture_name}: {}", result.stderr);
            let error = common::stderr_json(&result);
            assert_eq!(error["error"]["code"], "plan.missing_input");
            assert_eq!(
                error["error"]["pointer"],
                if fixture_name == "multi-fact" {
                    "/intent/model/factTable"
                } else {
                    "/schema/tables"
                }
            );
            if fixture_name != "multi-fact" {
                schema["tables"][0]["columns"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({"name":"EventDate", "dataType":"date"}));
            }
            std::fs::write(&schema_path, serde_json::to_vec_pretty(&schema).unwrap()).unwrap();
            let intent = serde_json::json!({
                "questions": ["shape golden"], "model": {"factTable": table_name}
            })
            .to_string();
            result = run_powerbi(&[
                "report",
                "plan",
                "--schema",
                schema_path.to_str().unwrap(),
                "--intent",
                &intent,
                "--json",
            ]);
        }
        assert_eq!(
            result.code, 0,
            "{fixture_name} planner stderr: {}",
            result.stderr
        );
        let value = stdout_json(&result);
        assert_eq!(
            value["shape"]["kind"], expected_kind,
            "shape verdict for {fixture_name}"
        );
        let pages = value["specV2"]["pages"].as_array().expect("v2 pages");
        let signature = serde_json::json!({
            "templates": pages.iter().map(|page| page["template"].clone()).collect::<Vec<_>>(),
            "slots": pages.iter().map(|page| {
                serde_json::json!({
                    "page": page["id"].clone(),
                    "slots": page["visuals"].as_array().into_iter().flatten()
                        .map(|visual| visual["slot"].clone()).collect::<Vec<_>>()
                })
            }).collect::<Vec<_>>()
        });
        if fixture_name == "no-date" {
            assert_eq!(
                signature, signatures[1],
                "supplying the missing date restores the star layout"
            );
        } else {
            assert!(
                signatures.iter().all(|previous| previous != &signature),
                "{fixture_name} planner structure duplicates another shape"
            );
            signatures.push(signature);
        }
    }
    assert_eq!(signatures.len(), cases.len() - 1);
}
