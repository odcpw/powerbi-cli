//! Atomic batch coverage for `report visuals set-object --batch`.

mod common;

use common::{
    assert_json_snapshot, assert_strict_valid, assert_tree_equal, first_page_name,
    first_two_visual_names, hash_tree, run_powerbi_owned, scaffold_sales, stderr_json, stdout_json,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn handles(project: &Path) -> (String, String) {
    let page = first_page_name(project);
    let (first, second) = first_two_visual_names(project);
    (
        format!("visual:{page}:{first}"),
        format!("visual:{page}:{second}"),
    )
}

fn set_object(handle: &str, object: &str, property: &str, value: Value) -> Value {
    json!({
        "op": "setObject",
        "visual": handle,
        "object": object,
        "property": property,
        "value": value
    })
}

fn write_batch(path: &Path, ops: Vec<Value>) {
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "schema": "powerbi-cli.ops.v1",
        "ops": ops
    }))
    .expect("serialize batch");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write batch");
}

fn batch_args(project: &Path, batch: &Path, mode: &[String]) -> Vec<String> {
    let mut args = vec![
        "report".to_string(),
        "visuals".to_string(),
        "set-object".to_string(),
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--batch".to_string(),
        batch.to_string_lossy().into_owned(),
    ];
    args.extend_from_slice(mode);
    args.push("--json".to_string());
    args
}

fn literal(value: &str) -> Value {
    json!({"expr": {"Literal": {"Value": value}}})
}

#[test]
fn batch_field_errors_are_pointer_precise_before_any_output_mode_writes() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("not-accessed");
    let batch = temp.path().join("invalid.ops.json");
    let out = temp.path().join("not-created");
    let base = set_object("visual:Page:Visual", "title", "show", literal("true"));
    let mut cases = Vec::new();
    for field in ["visual", "object", "property", "value"] {
        let mut invalid = base.clone();
        invalid.as_object_mut().unwrap().remove(field);
        cases.push((invalid, format!("/ops/0/{field}")));
    }
    let mut unknown = base.clone();
    unknown["a/b~c"] = json!(true);
    cases.push((unknown, "/ops/0/a~1b~0c".into()));
    let mut conflict = base.clone();
    conflict["kind"] = json!("setPosition");
    cases.push((conflict, "/ops/0/kind".into()));
    let mut null = base;
    null["value"] = Value::Null;
    cases.push((null, "/ops/0/value".into()));
    for (invalid, pointer) in cases {
        write_batch(&batch, vec![invalid]);
        for mode in [
            vec!["--dry-run".into()],
            vec!["--in-place".into()],
            vec!["--out-dir".into(), out.display().to_string()],
        ] {
            let result = run_powerbi_owned(&batch_args(&project, &batch, &mode));
            assert_eq!(result.exit, 10, "{}", result.stderr);
            assert!(result.stdout.is_empty());
            let error = stderr_json(&result);
            assert_eq!(error["error"]["code"], "input_safety_violation");
            assert_eq!(error["error"]["pointer"], pointer);
            assert!(error["error"]["hint"].is_string());
            assert!(
                !error["error"]["suggestedCommands"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert!(!project.exists());
            assert!(!out.exists());
        }
    }
    fs::write(
        &batch,
        br#"{"schema":"powerbi-cli.ops.v1","ops":[],"a/b~c":1}"#,
    )
    .unwrap();
    let result = run_powerbi_owned(&batch_args(&project, &batch, &["--dry-run".into()]));
    assert_eq!(stderr_json(&result)["error"]["pointer"], "/a~1b~0c");
    fs::write(&batch, "{").unwrap();
    let result = run_powerbi_owned(&batch_args(&project, &batch, &["--dry-run".into()]));
    assert_eq!(stderr_json(&result)["error"]["pointer"], "");
}

fn snapshot_shape(response: &Value) -> Value {
    json!({
        "schema": response["schema"],
        "ok": response["ok"],
        "exitCode": response["exitCode"],
        "action": response["action"],
        "dryRun": response["dryRun"],
        "mode": response["mode"],
        "batch": response["batch"],
        "count": response["count"],
        "changedCount": response["changedCount"],
        "operationOutcomes": response["operationOutcomes"]
            .as_array()
            .expect("operation outcomes")
            .iter()
            .map(|outcome| json!({
                "index": outcome["index"],
                "operation": outcome["operation"],
                "changed": outcome["changed"],
                "changes": outcome["changes"]
                    .as_array()
                    .expect("changes")
                    .iter()
                    .map(|change| json!({
                        "kind": change["kind"],
                        "action": change["action"],
                        "object": change["object"],
                        "property": change["property"],
                        "jsonPointer": change["jsonPointer"],
                        "before": change["before"],
                        "after": change["after"]
                    }))
                    .collect::<Vec<_>>()
            }))
            .collect::<Vec<_>>()
    })
}

#[test]
fn set_object_batch_dry_run_reports_each_entry_and_leaves_source_byte_identical() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (first, second) = handles(&project);
    let batch = temp.path().join("objects.ops.json");
    write_batch(
        &batch,
        vec![
            set_object(&first, "categoryLabels", "fontSize", literal("20D")),
            set_object(&second, "title", "show", literal("false")),
        ],
    );
    let before = hash_tree(&project);

    let output = run_powerbi_owned(&batch_args(&project, &batch, &["--dry-run".to_string()]));
    assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
    assert!(output.stderr.is_empty(), "stderr: {}", output.stderr);
    assert_eq!(hash_tree(&project), before);
    let response = stdout_json(&output);
    assert_eq!(response["count"], 2);
    assert_eq!(response["changedCount"], 2);
    assert_eq!(response["operationOutcomes"][0]["index"], 0);
    assert_eq!(response["operationOutcomes"][1]["index"], 1);
    assert!(
        response["operationOutcomes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| {
                item["readback"][0]
                    .as_str()
                    .is_some_and(|command| command.contains("report visuals show"))
            })
    );
    assert_json_snapshot("report-visual-set-object-batch", &snapshot_shape(&response));
}

#[test]
fn set_object_batch_out_dir_equals_the_same_single_mutations_byte_for_byte() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (first, second) = handles(&project);
    let batch = temp.path().join("objects.ops.json");
    write_batch(
        &batch,
        vec![
            set_object(&first, "categoryLabels", "fontSize", literal("20D")),
            set_object(&second, "title", "show", literal("false")),
        ],
    );
    let batch_out = temp.path().join("batch-out");
    let batch_run = run_powerbi_owned(&batch_args(
        &project,
        &batch,
        &[
            "--out-dir".to_string(),
            batch_out.to_string_lossy().into_owned(),
        ],
    ));
    assert_eq!(batch_run.exit, 0, "stderr: {}", batch_run.stderr);

    let first_out = temp.path().join("single-1");
    let first_run = run_powerbi_owned(&single_args(
        &project,
        &first,
        "categoryLabels",
        "fontSize",
        "20",
        &first_out,
    ));
    assert_eq!(first_run.exit, 0, "stderr: {}", first_run.stderr);
    let second_out = temp.path().join("single-2");
    let second_run = run_powerbi_owned(&single_args(
        &first_out,
        &second,
        "title",
        "show",
        "false",
        &second_out,
    ));
    assert_eq!(second_run.exit, 0, "stderr: {}", second_run.stderr);

    assert_tree_equal(
        &batch_out,
        &second_out,
        "batch equals N single set-object calls",
    );
    assert_strict_valid(&batch_out);
}

fn single_args(
    project: &Path,
    handle: &str,
    object: &str,
    property: &str,
    value: &str,
    out_dir: &Path,
) -> Vec<String> {
    vec![
        "report".to_string(),
        "visuals".to_string(),
        "set-object".to_string(),
        "--project".to_string(),
        project.to_string_lossy().into_owned(),
        "--handle".to_string(),
        handle.to_string(),
        "--object".to_string(),
        object.to_string(),
        "--property".to_string(),
        property.to_string(),
        "--value".to_string(),
        value.to_string(),
        "--out-dir".to_string(),
        out_dir.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]
}

#[test]
fn set_object_batch_in_place_commits_all_entries_and_validates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (first, second) = handles(&project);
    let batch = temp.path().join("objects.ops.json");
    write_batch(
        &batch,
        vec![
            set_object(&first, "categoryLabels", "fontSize", literal("21D")),
            set_object(&second, "title", "show", literal("false")),
        ],
    );
    let before = hash_tree(&project);
    let output = run_powerbi_owned(&batch_args(&project, &batch, &["--in-place".to_string()]));
    assert_eq!(output.exit, 0, "stderr: {}", output.stderr);
    let response = stdout_json(&output);
    assert_eq!(response["mode"], "in-place");
    assert_eq!(response["validation"]["ok"], true);
    assert_ne!(hash_tree(&project), before);
    assert_strict_valid(&project);
}

#[test]
fn set_object_batch_failure_at_entry_k_leaves_the_project_untouched() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let (first, second) = handles(&project);
    let batch = temp.path().join("invalid.ops.json");
    write_batch(
        &batch,
        vec![
            set_object(&first, "categoryLabels", "fontSize", literal("20D")),
            set_object(&second, "legend", "show", literal("true")),
        ],
    );
    let before = hash_tree(&project);
    let output = run_powerbi_owned(&batch_args(&project, &batch, &["--in-place".to_string()]));
    assert_eq!(output.exit, 2);
    assert!(output.stdout.is_empty(), "stdout: {}", output.stdout);
    assert_eq!(hash_tree(&project), before);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "unsupported_feature");
    assert_eq!(error["error"]["pointer"], "/ops/1/property");
    assert!(error["error"]["hint"].as_str().unwrap().contains("entry 1"));
    assert_batch_template(&error);
}

#[test]
fn set_object_batch_refuses_other_operation_kinds_before_project_access() {
    let temp = tempfile::tempdir().expect("tempdir");
    let batch = temp.path().join("mixed.ops.json");
    write_batch(
        &batch,
        vec![json!({
            "op": "setPosition",
            "visual": "visual:missing:missing",
            "x": 0.0,
            "y": 0.0,
            "width": 100.0,
            "height": 100.0
        })],
    );
    let output = run_powerbi_owned(&batch_args(
        &PathBuf::from("missing-project.pbip"),
        &batch,
        &["--dry-run".to_string()],
    ));
    assert_eq!(output.exit, 10);
    assert!(output.stdout.is_empty(), "stdout: {}", output.stdout);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "input_safety_violation");
    assert_eq!(error["error"]["pointer"], "/ops/0/op");
    assert!(
        error["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("only setObject")
    );
    assert_batch_template(&error);
}

#[test]
fn set_object_batch_requires_exactly_one_guarded_output_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let batch = temp.path().join("empty.ops.json");
    write_batch(&batch, Vec::new());
    let output = run_powerbi_owned(&batch_args(&PathBuf::from("missing"), &batch, &[]));
    assert_eq!(output.exit, 2);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("Start with `--dry-run`")
    );
    assert_batch_template(&error);
}

fn assert_batch_template(error: &Value) {
    let commands = error["error"]["suggestedCommands"]
        .as_array()
        .expect("suggested commands");
    assert_eq!(commands.len(), 1);
    let command = commands[0].as_str().expect("command string");
    assert!(command.starts_with("powerbi-cli report visuals set-object "));
    assert!(command.contains("--batch <ops.v1.json>"));
    assert!(command.ends_with("--dry-run --json"));
}
