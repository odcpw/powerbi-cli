mod common;
use common::{
    assert_json_snapshot, assert_tree_equal, canonical_display, forward_slashes_after,
    replace_in_strings, run_powerbi, stdout_json,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const PROOF_LADDER: [&str; 5] = [
    "unit-smoke",
    "schema-golden",
    "desktop-golden-pending",
    "manual-desktop-canvas-refresh",
    "desktop-canvas-refresh",
];
const UNAVAILABLE_REASONS: [&str; 3] = [
    "platform_non_windows",
    "missing_desktop",
    "missing_reference",
];
const VALIDATOR_STATUSES: [&str; 2] = ["not-installed", "unsupported-platform"];

fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn success(args: &[&str]) -> Value {
    let run = run_powerbi(args);
    assert_eq!(run.code, 0, "{}", run.stderr);
    stdout_json(&run)
}
/// Drop timing fields and redact the temporary root in its raw, canonical,
/// and Windows verbatim spellings so the snapshot is identical on Linux and
/// Windows.
fn normalize(value: &mut Value, root: &Path) {
    strip_timings(value);
    let display = canonical_display(root);
    for spelling in [
        format!(r"\\?\{display}"),
        display.clone(),
        root.to_str().unwrap().to_string(),
    ] {
        replace_in_strings(value, &spelling, "<root>");
    }
    forward_slashes_after(value, "<root>");
}
fn strip_timings(value: &mut Value) {
    match value {
        Value::Object(items) => {
            items.remove("ms");
            for item in items.values_mut() {
                strip_timings(item);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_timings(item);
            }
        }
        _ => {}
    }
}

#[test]
fn star_and_flat_compose_build_validate_and_export_deterministically() {
    for kind in ["star", "flat"] {
        let root = tempfile::tempdir().unwrap();
        let fixture: Value = serde_json::from_slice(
            &fs::read(format!("testdata/golden/planner-narrative/{kind}.json")).unwrap(),
        )
        .unwrap();
        let schema = root.path().join("schema.json");
        write(&schema, &fixture["schema"]);
        let output = root.path().join("first");
        let first = success(&[
            "report",
            "compose",
            "--schema",
            schema.to_str().unwrap(),
            "--objective",
            "Executive overview",
            "--out-dir",
            output.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(first["ok"], true);
        assert!(!first["proofPlan"].is_null());
        assert_eq!(first["stages"].as_array().unwrap().len(), 7);
        let sidecar = root.path().join("first-compose");
        for file in [
            "profile.json",
            "plan.json",
            "spec.json",
            "validate.json",
            "explain.json",
            "ops.json",
            "build.json",
            "triage.json",
            "scorecard.json",
            "wireframes.json",
        ] {
            assert!(sidecar.join(file).is_file(), "{file}");
        }
        assert!(fs::read_dir(sidecar.join("wireframes")).unwrap().count() >= 4);
        success(&["validate", output.to_str().unwrap(), "--json"]);
        let second = root.path().join("second");
        success(&[
            "dashboard",
            "new",
            "--schema",
            schema.to_str().unwrap(),
            "--objective",
            "Executive overview",
            "--out-dir",
            second.to_str().unwrap(),
            "--json",
        ]);
        assert_tree_equal(&output, &second, "composed projects are deterministic");
        // `achievableHere` and `unavailable[]` depend on the platform and on
        // Desktop being installed, so they are checked against the documented
        // value sets here and only the platform-independent proof-plan fields
        // enter the snapshot.
        let proof = &first["proofPlan"];
        assert!(
            PROOF_LADDER.contains(&proof["achievableHere"].as_str().unwrap_or_default()),
            "achievableHere: {}",
            proof["achievableHere"]
        );
        for entry in proof["unavailable"].as_array().unwrap() {
            assert!(
                UNAVAILABLE_REASONS.contains(&entry["why"].as_str().unwrap_or_default()),
                "unavailable entry: {entry}"
            );
        }
        let mut summary = json!({
            "schema": first["schema"],
            "stages": first["stages"],
            "scorecard": first["scorecard"],
            "decisions": first["decisions"],
            "proofPlan": {
                "requestedLevel": proof["requestedLevel"],
                "commands": proof["commands"]
            }
        });
        // `scorecard.microsoftValidator` reports the host toolchain state
        // (Linux: unsupported-platform; Windows: not-installed or ready), so
        // it is checked against the documented statuses and left out of the
        // platform-neutral snapshot.
        let validator = summary["scorecard"]
            .as_object_mut()
            .unwrap()
            .remove("microsoftValidator")
            .expect("scorecard.microsoftValidator");
        assert!(
            VALIDATOR_STATUSES.contains(&validator["status"].as_str().unwrap_or_default()),
            "microsoftValidator: {validator}"
        );
        assert!(
            validator["reason"].is_string(),
            "microsoftValidator: {validator}"
        );
        normalize(&mut summary, root.path());
        assert_json_snapshot(&format!("report-compose-{kind}"), &summary);
    }
}

#[test]
fn dry_run_writes_nothing_and_in_place_requires_confirmation_and_keeps_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    success(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        project.to_str().unwrap(),
        "--json",
    ]);
    let dry = success(&[
        "report",
        "compose",
        "--schema",
        "examples/sales.schema.json",
        "--objective",
        "Overview",
        "--project",
        project.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry["changed"], false);
    assert!(!root.path().join("project-compose").exists());
    let refusal = run_powerbi(&[
        "report",
        "compose",
        "--schema",
        "examples/sales.schema.json",
        "--objective",
        "Overview",
        "--project",
        project.to_str().unwrap(),
        "--in-place",
        "--json",
    ]);
    assert_ne!(refusal.code, 0);
    let written = success(&[
        "report",
        "compose",
        "--schema",
        "examples/sales.schema.json",
        "--objective",
        "Overview",
        "--project",
        project.to_str().unwrap(),
        "--in-place",
        "--confirm",
        dry["confirmToken"].as_str().unwrap(),
        "--json",
    ]);
    assert!(Path::new(written["snapshotDir"].as_str().unwrap()).exists());
    success(&["validate", project.to_str().unwrap(), "--json"]);
}

#[test]
fn weak_signal_errors_are_unchanged_and_never_publish_a_project() {
    let root = tempfile::tempdir().unwrap();
    let schema = root.path().join("weak.json");
    write(
        &schema,
        &json!({"name":"Weak","tables":[{"name":"Events","columns":[{"name":"Amount","dataType":"decimal"}]}]}),
    );
    let output = root.path().join("project");
    let direct = run_powerbi(&[
        "report",
        "plan",
        "--schema",
        schema.to_str().unwrap(),
        "--objective",
        "Overview",
        "--json",
    ]);
    let composed = run_powerbi(&[
        "report",
        "compose",
        "--schema",
        schema.to_str().unwrap(),
        "--objective",
        "Overview",
        "--out-dir",
        output.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(composed.code, direct.code);
    assert_eq!(composed.stderr, direct.stderr);
    assert!(!output.exists());
    assert!(root.path().join("project-compose/profile.json").exists());
}

#[test]
fn invalid_modes_and_unsupported_variants_refuse_before_sidecars() {
    let root = tempfile::tempdir().unwrap();
    for extra in [
        vec!["--dry-run", "--in-place"],
        vec!["--variants", "2", "--dry-run"],
        vec![
            "--rows",
            "absent.json",
            "--profile",
            "absent.json",
            "--dry-run",
        ],
    ] {
        let mut args = vec![
            "report",
            "compose",
            "--schema",
            "examples/sales.schema.json",
        ];
        args.extend(extra);
        args.push("--json");
        assert_ne!(run_powerbi(&args).code, 0);
    }
    let nested = root.path().join("project/artifacts");
    let target = root.path().join("project");
    assert_ne!(
        run_powerbi(&[
            "report",
            "compose",
            "--schema",
            "examples/sales.schema.json",
            "--objective",
            "Overview",
            "--out-dir",
            target.to_str().unwrap(),
            "--artifacts-dir",
            nested.to_str().unwrap(),
            "--json"
        ])
        .code,
        0
    );
    assert!(!target.exists());
}

#[test]
fn rows_are_profiled_without_values_and_preset_style_reaches_the_compiler() {
    let root = tempfile::tempdir().unwrap();
    let fixture: Value =
        serde_json::from_slice(&fs::read("testdata/golden/planner-narrative/flat.json").unwrap())
            .unwrap();
    let schema = root.path().join("schema.json");
    write(&schema, &fixture["schema"]);
    let rows = root.path().join("rows.json");
    write(
        &rows,
        &json!([
            {"Date":"2025-01-01","Amount":10,"Category":"Synthetic A","Ticket":"Dummy 1"},
            {"Date":"2025-01-02","Amount":20,"Category":"Synthetic B","Ticket":"Dummy 2"}
        ]),
    );
    let args = [
        "report",
        "compose",
        "--schema",
        schema.to_str().unwrap(),
        "--rows",
        rows.to_str().unwrap(),
        "--objective",
        "Overview",
        "--style",
        "corporate-neutral",
        "--dry-run",
        "--json",
    ];
    let mut first = success(&args);
    let mut second = success(&args);
    assert_eq!(
        first["artifacts"]["spec.json"]["style"]["tokens"]["preset"],
        "corporate-neutral"
    );
    assert_eq!(first["artifacts"]["profile.json"]["source"]["rowCount"], 2);
    assert!(
        !serde_json::to_string(&first["artifacts"]["profile.json"])
            .unwrap()
            .contains("Synthetic A")
    );
    normalize(&mut first, root.path());
    normalize(&mut second, root.path());
    assert_eq!(first, second, "only observational timings vary");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    let tokens = root.path().join("tokens.json");
    write(&tokens, &json!({"preset":"corporate-neutral"}));
    let file_style = success(&[
        "report",
        "compose",
        "--schema",
        schema.to_str().unwrap(),
        "--objective",
        "Overview",
        "--style",
        tokens.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(
        file_style["artifacts"]["spec.json"]["style"]["tokens"]["preset"],
        "corporate-neutral"
    );
}

#[test]
fn unsupported_style_propagates_the_native_spec_validation_document() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let composed = run_powerbi(&[
        "report",
        "compose",
        "--schema",
        "examples/sales.schema.json",
        "--objective",
        "Overview",
        "--style",
        "not-a-preset",
        "--out-dir",
        project.to_str().unwrap(),
        "--json",
    ]);
    assert_ne!(composed.code, 0);
    assert!(!project.exists());
    let sidecar = root.path().join("project-compose");
    let native = run_powerbi(&[
        "report",
        "spec",
        "validate",
        "--schema",
        "examples/sales.schema.json",
        "--profile",
        sidecar.join("profile.json").to_str().unwrap(),
        "--spec",
        sidecar.join("spec.json").to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(composed.code, native.code);
    assert_eq!(composed.stdout, native.stdout);
    assert_eq!(composed.stderr, native.stderr);
}

#[test]
#[ignore = "explicit performance check on the shared build machine"]
fn twenty_table_planner_input_composes_under_three_seconds() {
    let root = tempfile::tempdir().unwrap();
    let tables = (0..20)
        .map(|index| {
            let name = format!("Table{index:02}");
            json!({"name":name,"columns":[
            {"name":"Date","dataType":"date"},
            {"name":"Category","dataType":"string"},
            {"name":"Amount","dataType":"decimal"}
        ],"measures":[{"name":"Total","expression":format!("SUM('{name}'[Amount])")}]})
        })
        .collect::<Vec<_>>();
    let schema = root.path().join("schema.json");
    write(&schema, &json!({"name":"ComposePerf","tables":tables}));
    let intent = root.path().join("intent.json");
    write(
        &intent,
        &json!({"questions":["Overview"],"model":{"factTable":"Table00"}}),
    );
    let output = root.path().join("project");
    let run = run_powerbi(&[
        "report",
        "compose",
        "--schema",
        schema.to_str().unwrap(),
        "--intent",
        intent.to_str().unwrap(),
        "--out-dir",
        output.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let response = stdout_json(&run);
    eprintln!(
        "compose perf: {:?}, pages={}",
        run.elapsed, response["artifacts"]["build.json"]["compiled"]["counts"]["pages"]
    );
    assert!(
        run.elapsed < std::time::Duration::from_secs(3),
        "compose took {:?}",
        run.elapsed
    );
}
