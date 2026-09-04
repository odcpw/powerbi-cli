mod common;

use common::{run_powerbi_owned, run_powerbi_owned_with_peak_memory, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::time::Duration;

const TWENTY_TABLE_TEN_PAGE_LIMIT: Duration = Duration::from_secs(3);
const HUNDRED_TABLE_FIFTY_INCLUDE_LIMIT: Duration = Duration::from_secs(10);
const HUNDRED_TABLE_FIFTY_INCLUDE_MEMORY_LIMIT_BYTES: u64 = 512 * 1024 * 1024;

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
fn hundred_table_schema_with_fifty_includes_builds_under_ten_seconds() {
    let temp = tempfile::tempdir().expect("composition perf tempdir");
    let parts = temp.path().join("parts");
    fs::create_dir_all(&parts).expect("composition parts directory");
    let mut includes = Vec::new();
    for fragment_index in 0..50 {
        let tables = (0..2)
            .map(|table_offset| {
                let table_index = fragment_index * 2 + table_offset;
                json!({
                    "name": format!("IncludeTable{table_index:03}"),
                    "columns": [{"name": "Value", "dataType": "int64"}],
                    "rows": []
                })
            })
            .collect::<Vec<_>>();
        let name = format!("fragment-{fragment_index:02}.json");
        write_json(&parts.join(&name), &json!({"tables": tables}));
        includes.push(format!("parts/{name}"));
    }
    let schema_path = temp.path().join("hundred-table.schema.json");
    write_json(
        &schema_path,
        &json!({
            "schemaVersion": "1",
            "name": "HundredTableIncludePerf",
            "displayName": "Hundred Table Include Perf",
            "$include": includes,
            "relationships": []
        }),
    );
    let spec_path = temp.path().join("empty.dashboard.json");
    write_json(
        &spec_path,
        &json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": {"name": "HundredTableIncludePerf"},
            "pages": []
        }),
    );
    let normalized_path = temp.path().join("hundred-table.normalized.json");
    let normalize = run_powerbi_owned(&[
        "schema".into(),
        "normalize".into(),
        path_arg(&schema_path),
        "--out".into(),
        path_arg(&normalized_path),
        "--json".into(),
    ]);
    assert_eq!(
        normalize.exit, 0,
        "normalize failed\nstdout: {}\nstderr: {}",
        normalize.stdout, normalize.stderr
    );
    assert!(
        normalize.elapsed < HUNDRED_TABLE_FIFTY_INCLUDE_LIMIT,
        "100-table/50-include normalize took {:?}, limit is {:?}",
        normalize.elapsed,
        HUNDRED_TABLE_FIFTY_INCLUDE_LIMIT
    );

    let project = temp.path().join("hundred-table-project");
    let (build, peak_memory_bytes) = run_powerbi_owned_with_peak_memory(&[
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
        build.exit, 0,
        "build failed\nstdout: {}\nstderr: {}",
        build.stdout, build.stderr
    );
    assert_eq!(stdout_json(&build)["compiled"]["counts"]["tables"], 100);
    assert!(
        build.elapsed < HUNDRED_TABLE_FIFTY_INCLUDE_LIMIT,
        "100-table/50-include build took {:?}, limit is {:?}",
        build.elapsed,
        HUNDRED_TABLE_FIFTY_INCLUDE_LIMIT
    );
    assert!(
        peak_memory_bytes > 0,
        "peak RSS sampler did not observe the child process"
    );
    assert!(
        peak_memory_bytes < HUNDRED_TABLE_FIFTY_INCLUDE_MEMORY_LIMIT_BYTES,
        "100-table/50-include build used {peak_memory_bytes} bytes RSS, limit is {HUNDRED_TABLE_FIFTY_INCLUDE_MEMORY_LIMIT_BYTES}"
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
