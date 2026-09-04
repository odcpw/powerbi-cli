mod common;

use common::{assert_json_snapshot, run_powerbi_owned, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(value).expect("serialize fixture")
        ),
    )
    .expect("write fixture");
}

fn table(name: &str, columns: Value) -> Value {
    json!({"name": name, "columns": columns})
}

fn relation(from_table: &str, from_column: &str, to_table: &str, to_column: &str) -> Value {
    json!({
        "fromTable": from_table,
        "fromColumn": from_column,
        "toTable": to_table,
        "toColumn": to_column,
        "cardinality": "manyToOne"
    })
}

fn profile_shape(root: &Path, name: &str, schema: Value) -> Value {
    let schema_path = root.join(format!("{name}.schema.json"));
    let profile_path = root.join(format!("{name}.profile.json"));
    write_json(&schema_path, &schema);
    let args = vec![
        "profile".to_string(),
        "infer".to_string(),
        "--schema".to_string(),
        path_arg(&schema_path),
        "--out".to_string(),
        path_arg(&profile_path),
        "--json".to_string(),
    ];
    let infer = run_powerbi_owned(&args);
    assert_eq!(infer.code, 0, "profile infer stderr: {}", infer.stderr);
    let summarize_args = vec![
        "profile".to_string(),
        "summarize".to_string(),
        path_arg(&profile_path),
        "--json".to_string(),
    ];
    let summarize = run_powerbi_owned(&summarize_args);
    assert_eq!(
        summarize.code, 0,
        "profile summarize stderr: {}",
        summarize.stderr
    );
    stdout_json(&summarize)["summary"]["shape"].clone()
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("UTF-8 test path").to_string()
}

fn flat_schema() -> Value {
    json!({
        "name": "FlatEvents",
        "tables": [table("Events", json!([
            {"name":"EventId","dataType":"int64","isKey":true},
            {"name":"EventDate","dataType":"date"},
            {"name":"Amount","dataType":"decimal"},
            {"name":"Category","dataType":"string"}
        ]))]
    })
}

fn star_schema() -> Value {
    json!({
        "name": "StarSales",
        "tables": [
            table("FactSales", json!([
                {"name":"DateKey","dataType":"int64"},
                {"name":"CustomerKey","dataType":"int64"},
                {"name":"Revenue","dataType":"decimal"}
            ])),
            table("DimDate", json!([
                {"name":"DateKey","dataType":"int64","isKey":true},
                {"name":"Date","dataType":"date"}
            ])),
            table("DimCustomer", json!([
                {"name":"CustomerKey","dataType":"int64","isKey":true},
                {"name":"CustomerName","dataType":"string"}
            ]))
        ],
        "relationships": [
            relation("FactSales", "DateKey", "DimDate", "DateKey"),
            relation("FactSales", "CustomerKey", "DimCustomer", "CustomerKey")
        ]
    })
}

fn snowflake_schema() -> Value {
    json!({
        "name": "SnowflakeSales",
        "tables": [
            table("FactSales", json!([
                {"name":"CustomerKey","dataType":"int64"},
                {"name":"Revenue","dataType":"decimal"}
            ])),
            table("DimCustomer", json!([
                {"name":"CustomerKey","dataType":"int64","isKey":true},
                {"name":"RegionKey","dataType":"int64"},
                {"name":"CustomerName","dataType":"string"}
            ])),
            table("DimRegion", json!([
                {"name":"RegionKey","dataType":"int64","isKey":true},
                {"name":"Region","dataType":"string"}
            ]))
        ],
        "relationships": [
            relation("FactSales", "CustomerKey", "DimCustomer", "CustomerKey"),
            relation("DimCustomer", "RegionKey", "DimRegion", "RegionKey")
        ]
    })
}

fn multi_fact_schema() -> Value {
    json!({
        "name": "MultiFactSales",
        "tables": [
            table("FactSales", json!([
                {"name":"DateKey","dataType":"int64"},
                {"name":"Revenue","dataType":"decimal"}
            ])),
            table("FactTargets", json!([
                {"name":"DateKey","dataType":"int64"},
                {"name":"Target","dataType":"decimal"}
            ])),
            table("DimDate", json!([
                {"name":"DateKey","dataType":"int64","isKey":true},
                {"name":"Date","dataType":"date"}
            ]))
        ],
        "relationships": [
            relation("FactSales", "DateKey", "DimDate", "DateKey"),
            relation("FactTargets", "DateKey", "DimDate", "DateKey")
        ]
    })
}

fn no_date_schema() -> Value {
    json!({
        "name": "NoDateSales",
        "tables": [
            table("FactSales", json!([
                {"name":"CustomerKey","dataType":"int64"},
                {"name":"Revenue","dataType":"decimal"}
            ])),
            table("DimCustomer", json!([
                {"name":"CustomerKey","dataType":"int64","isKey":true},
                {"name":"CustomerName","dataType":"string"}
            ]))
        ],
        "relationships": [relation("FactSales", "CustomerKey", "DimCustomer", "CustomerKey")]
    })
}

fn ambiguous_schema() -> Value {
    json!({
        "name": "AmbiguousModel",
        "tables": [
            table("Events", json!([
                {"name":"Value","dataType":"decimal"},
                {"name":"Label","dataType":"string"}
            ])),
            table("Lookup", json!([
                {"name":"Code","dataType":"string"},
                {"name":"Description","dataType":"string"}
            ]))
        ]
    })
}

#[test]
fn shape_classification_has_six_deterministic_schema_goldens() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cases = [
        ("flat", flat_schema()),
        ("star", star_schema()),
        ("snowflake", snowflake_schema()),
        ("multi-fact", multi_fact_schema()),
        ("no-date", no_date_schema()),
        ("ambiguous", ambiguous_schema()),
    ];
    for (name, schema) in cases {
        let shape = profile_shape(temp.path(), name, schema);
        assert!(shape["kind"].is_string(), "{name} shape kind");
        for field in [
            "facts",
            "dimensions",
            "dateTables",
            "keyCandidates",
            "highCardinality",
            "warnings",
        ] {
            assert!(shape.get(field).is_some(), "{name} missing shape.{field}");
        }
        assert_json_snapshot(&format!("profile-shape-{name}"), &shape);
    }
}

#[test]
fn report_plan_reuses_shape_and_model_shape_decision_deterministically() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema_path = temp.path().join("sales.schema.json");
    let profile_path = temp.path().join("sales.profile.json");
    write_json(&schema_path, &star_schema());
    let infer_args = vec![
        "profile".to_string(),
        "infer".to_string(),
        "--schema".to_string(),
        path_arg(&schema_path),
        "--out".to_string(),
        path_arg(&profile_path),
        "--json".to_string(),
    ];
    let infer = run_powerbi_owned(&infer_args);
    assert_eq!(infer.code, 0, "profile infer stderr: {}", infer.stderr);
    let args = vec![
        "report".to_string(),
        "plan".to_string(),
        "--schema".to_string(),
        path_arg(&schema_path),
        "--profile".to_string(),
        path_arg(&profile_path),
        "--objective".to_string(),
        "Executive sales overview".to_string(),
        "--json".to_string(),
    ];
    let first = run_powerbi_owned(&args);
    let second = run_powerbi_owned(&args);
    assert_eq!(first.code, 0, "report plan stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "report plan stderr: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout);
    let value = stdout_json(&first);
    assert_eq!(value["shape"]["kind"], "star");
    let decision = value["decisions"]
        .as_array()
        .expect("decisions")
        .iter()
        .find(|decision| decision["kind"] == "model-shape")
        .expect("model-shape decision");
    assert_eq!(decision["shape"], value["shape"]);
    assert_eq!(value["profileSummary"]["shape"], value["shape"]);
}
