mod common;

use common::{run_powerbi_owned, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::time::Duration;

const TWENTY_TABLE_TEN_PAGE_LIMIT: Duration = Duration::from_secs(3);
const HUNDRED_THOUSAND_ROW_PROFILE_LIMIT: Duration = Duration::from_secs(3);

#[test]
#[ignore = "nightly performance gate"]
fn report_build_twenty_tables_ten_pages_completes_under_three_seconds() {
    let temp = tempfile::tempdir().expect("perf tempdir");
    let schema_path = temp.path().join("twenty-tables.schema.json");
    let spec_path = temp.path().join("ten-pages.dashboard.json");
    let project = temp.path().join("project");
    write_json(&schema_path, &generated_schema(20));
    write_json(&spec_path, &generated_spec(10));

    let run = run_powerbi_owned(&[
        "report".into(),
        "build".into(),
        "--schema".into(),
        path_arg(&schema_path),
        "--spec".into(),
        path_arg(&spec_path),
        "--out-dir".into(),
        path_arg(&project),
        "--json".into(),
    ]);
    assert_eq!(
        run.exit, 0,
        "perf build failed\nstdout: {}\nstderr: {}",
        run.stdout, run.stderr
    );
    assert_eq!(stdout_json(&run)["ok"], Value::Bool(true));
    assert!(
        run.elapsed < TWENTY_TABLE_TEN_PAGE_LIMIT,
        "20-table/10-page report build took {:?}, limit is {:?}",
        run.elapsed,
        TWENTY_TABLE_TEN_PAGE_LIMIT
    );
}

#[test]
#[ignore = "nightly performance gate"]
fn profile_infer_one_hundred_thousand_rows_completes_under_three_seconds() {
    let temp = tempfile::tempdir().expect("perf tempdir");
    let schema_path = temp.path().join("rows.schema.json");
    let rows_path = temp.path().join("rows.csv");
    write_json(
        &schema_path,
        &json!({
            "name": "RowsPerf",
            "displayName": "Rows Performance",
            "tables": [{
                "name": "FactRows",
                "columns": [
                    {"name": "Id", "dataType": "int64", "isKey": true},
                    {"name": "EventDate", "dataType": "date"},
                    {"name": "Amount", "dataType": "decimal"},
                    {"name": "Category", "dataType": "string"}
                ]
            }]
        }),
    );
    let mut rows = String::from("Id,EventDate,Amount,Category\n");
    for index in 0..99_999 {
        rows.push_str(&format!(
            "{index},2026-01-01,{},Synthetic{}\n",
            index as f64 / 10.0,
            index % 10
        ));
    }
    fs::write(&rows_path, rows).expect("write rows perf fixture");
    let run = run_powerbi_owned(&[
        "profile".into(),
        "infer".into(),
        "--schema".into(),
        path_arg(&schema_path),
        "--rows".into(),
        path_arg(&rows_path),
        "--json".into(),
    ]);
    assert_eq!(run.exit, 0, "profile perf failed: {}", run.stderr);
    assert_eq!(stdout_json(&run)["profile"]["source"]["rowCount"], 99_999);
    assert!(
        run.elapsed < HUNDRED_THOUSAND_ROW_PROFILE_LIMIT,
        "100k-row profile inference took {:?}, limit is {:?}",
        run.elapsed,
        HUNDRED_THOUSAND_ROW_PROFILE_LIMIT
    );
}

fn generated_schema(table_count: usize) -> Value {
    let tables = (0..table_count)
        .map(|index| {
            let table = format!("PerfTable{index:02}");
            json!({
                "name": table,
                "columns": [
                    {"name": "Category", "dataType": "string"},
                    {"name": "Value", "dataType": "int64", "formatString": "#,##0"}
                ],
                "measures": [{
                    "name": "Total Value",
                    "expression": format!("SUM('{table}'[Value])"),
                    "formatString": "#,##0"
                }],
                "rows": [{"Category": "Example", "Value": 1}]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "name": "PerfHarness",
        "displayName": "Performance Harness",
        "locale": "en-US",
        "tables": tables,
        "pages": []
    })
}

fn generated_spec(page_count: usize) -> Value {
    let pages = (0..page_count)
        .map(|index| {
            json!({
                "id": format!("page_{index:02}"),
                "displayName": format!("Page {}", index + 1),
                "size": {"width": 1280, "height": 720},
                "visuals": [{
                    "id": format!("value_card_{index:02}"),
                    "type": "card",
                    "title": format!("Value {}", index + 1),
                    "bindings": [{
                        "role": "Values",
                        "field": format!("PerfTable{index:02}[Total Value]")
                    }],
                    "layout": {"x": 32, "y": 32, "width": 280, "height": 120}
                }]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema": "powerbi-cli.dashboard.v1",
        "report": {
            "name": "PerfHarness",
            "displayName": "Performance Harness",
            "locale": "en-US",
            "audience": "test harness",
            "questions": ["Does compilation stay within its wall-time budget?"]
        },
        "pages": pages
    })
}

fn write_json(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize perf fixture");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write perf fixture");
}

fn path_arg(path: &Path) -> String {
    path.to_str().expect("test path is UTF-8").to_string()
}
