//! Dashboard-spec style compilation and CLI-kernel parity tests.

mod common;

use common::{
    assert_json_snapshot, assert_tree_equal, run_powerbi_owned, stderr_json, stdout_json,
};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn path_arg(path: &Path) -> String {
    path.to_str().expect("test path is UTF-8").to_string()
}

fn sales_spec() -> Value {
    serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.v2.json").expect("sales v2 spec"),
    )
    .expect("sales v2 JSON")
}

fn write_spec(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize spec");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write spec");
}

fn build(spec: &Path, mode: &[String]) -> common::CliRun {
    let mut args = vec![
        "report".into(),
        "build".into(),
        "--schema".into(),
        "examples/sales.schema.json".into(),
        "--spec".into(),
        path_arg(spec),
    ];
    args.extend_from_slice(mode);
    args.push("--json".into());
    run_powerbi_owned(&args)
}

fn build_out(spec: &Path, out: &Path) -> common::CliRun {
    build(spec, &["--out-dir".into(), path_arg(out)])
}

#[test]
fn style_preset_compiles_last_and_matches_apply_theme_preset() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_spec_path = temp.path().join("base.json");
    let styled_spec_path = temp.path().join("styled.json");
    let base_spec = sales_spec();
    let mut styled_spec = base_spec.clone();
    styled_spec["style"] = json!({"preset": "risk-dashboard"});
    write_spec(&base_spec_path, &base_spec);
    write_spec(&styled_spec_path, &styled_spec);

    let dry_run = build(&styled_spec_path, &["--dry-run".into()]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let repeated_dry_run = build(&styled_spec_path, &["--dry-run".into()]);
    assert_eq!(
        repeated_dry_run.code, 0,
        "stderr: {}",
        repeated_dry_run.stderr
    );
    assert_eq!(dry_run.stdout, repeated_dry_run.stdout);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(dry_json["dryRun"], true);
    assert_eq!(dry_json["operations"][1]["op"], "applyThemePreset");
    assert_json_snapshot(
        "report-build-style-preset",
        &json!({
            "compiledOps": dry_json["compiled"]["ops"],
            "styleOperation": dry_json["operations"][1]
        }),
    );

    let compiler = temp.path().join("compiler");
    let compiled = build_out(&styled_spec_path, &compiler);
    assert_eq!(compiled.code, 0, "stderr: {}", compiled.stderr);
    let compiled_json = stdout_json(&compiled);
    assert_eq!(
        compiled_json["operationOutcomes"].as_array().map(Vec::len),
        Some(1)
    );

    let base = temp.path().join("base");
    let base_build = build_out(&base_spec_path, &base);
    assert_eq!(base_build.code, 0, "stderr: {}", base_build.stderr);
    let direct = temp.path().join("direct");
    let applied = run_powerbi_owned(&[
        "report".into(),
        "themes".into(),
        "apply-preset".into(),
        "--project".into(),
        path_arg(&base),
        "--preset".into(),
        "risk-dashboard".into(),
        "--out-dir".into(),
        path_arg(&direct),
        "--json".into(),
    ]);
    assert_eq!(applied.code, 0, "stderr: {}", applied.stderr);
    assert_tree_equal(&compiler, &direct, "style preset compiler parity");
}

fn literal_style_bundle(root: &Path, base_spec: &Path) -> PathBuf {
    let source = root.join("style-source");
    let built = build_out(base_spec, &source);
    assert_eq!(built.code, 0, "stderr: {}", built.stderr);
    let visuals = run_powerbi_owned(&[
        "report".into(),
        "visuals".into(),
        "list".into(),
        "--project".into(),
        path_arg(&source),
        "--json".into(),
    ]);
    assert_eq!(visuals.code, 0, "stderr: {}", visuals.stderr);
    let handle = stdout_json(&visuals)["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string();
    let styled = run_powerbi_owned(&[
        "report".into(),
        "visuals".into(),
        "set-object".into(),
        "--project".into(),
        path_arg(&source),
        "--handle".into(),
        handle,
        "--object".into(),
        "categoryLabels".into(),
        "--property".into(),
        "fontSize".into(),
        "--value".into(),
        "11".into(),
        "--in-place".into(),
        "--json".into(),
    ]);
    assert_eq!(styled.code, 0, "stderr: {}", styled.stderr);
    let bundle = root.join("style.bundle.json");
    let extracted = run_powerbi_owned(&[
        "report".into(),
        "style".into(),
        "extract".into(),
        "--project".into(),
        path_arg(&source),
        "--out".into(),
        path_arg(&bundle),
        "--include-literal-text".into(),
        "--json".into(),
    ]);
    assert_eq!(extracted.code, 0, "stderr: {}", extracted.stderr);
    bundle
}

#[test]
fn style_bundle_requires_literal_opt_in_and_matches_apply_style_bundle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let base_spec_path = temp.path().join("base.json");
    let base_spec = sales_spec();
    write_spec(&base_spec_path, &base_spec);
    let bundle = literal_style_bundle(temp.path(), &base_spec_path);

    let mut unsafe_spec = base_spec.clone();
    unsafe_spec["style"] = json!({"bundle": path_arg(&bundle)});
    let unsafe_path = temp.path().join("unsafe.json");
    write_spec(&unsafe_path, &unsafe_spec);
    let refused = build(&unsafe_path, &["--dry-run".into()]);
    assert_eq!(refused.code, 2);
    let refusal = stderr_json(&refused);
    assert_eq!(refusal["error"]["code"], "invalid_args");
    assert_eq!(refusal["error"]["pointer"], "/style/allowLiteralText");
    assert!(
        refusal["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("literal text"))
    );

    let mut safe_spec = unsafe_spec;
    safe_spec["style"]["allowLiteralText"] = Value::Bool(true);
    let safe_path = temp.path().join("safe.json");
    write_spec(&safe_path, &safe_spec);
    let compiler = temp.path().join("compiler");
    let compiled = build_out(&safe_path, &compiler);
    assert_eq!(compiled.code, 0, "stderr: {}", compiled.stderr);
    let compiled_json = stdout_json(&compiled);
    assert_eq!(compiled_json["operations"][1]["op"], "applyStyleBundle");
    assert_eq!(compiled_json["operations"][1]["allowLiteralText"], true);

    let base = temp.path().join("base");
    let base_build = build_out(&base_spec_path, &base);
    assert_eq!(base_build.code, 0, "stderr: {}", base_build.stderr);
    let direct = temp.path().join("direct");
    let applied = run_powerbi_owned(&[
        "report".into(),
        "style".into(),
        "apply".into(),
        "--project".into(),
        path_arg(&base),
        "--bundle".into(),
        path_arg(&bundle),
        "--allow-literal-text".into(),
        "--out-dir".into(),
        path_arg(&direct),
        "--json".into(),
    ]);
    assert_eq!(applied.code, 0, "stderr: {}", applied.stderr);
    assert_tree_equal(&compiler, &direct, "style bundle compiler parity");
}

#[test]
fn style_tokens_and_defaults_refuse_with_the_follow_up_bead() {
    let temp = tempfile::tempdir().expect("tempdir");
    for (name, style, pointer) in [
        (
            "tokens",
            json!({"tokens": {"semantic": {"good": "#2E7D32"}}}),
            "/style/tokens",
        ),
        (
            "defaults",
            json!({"defaults": {"card": {"title": true}}}),
            "/style/defaults",
        ),
    ] {
        let mut spec = sales_spec();
        spec["style"] = style;
        let path = temp.path().join(format!("{name}.json"));
        write_spec(&path, &spec);
        let output = build(&path, &["--dry-run".into()]);
        assert_eq!(output.code, 2);
        let error = stderr_json(&output);
        assert_eq!(error["error"]["code"], "unsupported_feature");
        assert_eq!(error["error"]["pointer"], pointer);
        assert!(
            error["error"]["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("pbi-t3-compiler-completeness-1qi.13"))
        );
        assert!(
            !error["error"]["suggestedCommands"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn invalid_style_operation_inputs_are_pointer_rich_and_do_not_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let missing_bundle = temp.path().join("missing-style.json");
    let cases = [
        (
            "both",
            json!({"preset": "neutral-ops", "bundle": path_arg(&missing_bundle)}),
            "/style",
            2,
        ),
        (
            "unknown-preset",
            json!({"preset": "unknown"}),
            "/style/preset",
            2,
        ),
        (
            "allow-without-bundle",
            json!({"allowLiteralText": true}),
            "/style/allowLiteralText",
            2,
        ),
        (
            "missing-bundle",
            json!({"bundle": path_arg(&missing_bundle)}),
            "/style/bundle",
            3,
        ),
    ];
    for (name, style, pointer, exit) in cases {
        let mut spec = sales_spec();
        spec["style"] = style;
        let spec_path = temp.path().join(format!("{name}.json"));
        let out = temp.path().join(format!("{name}-out"));
        write_spec(&spec_path, &spec);
        let result = build_out(&spec_path, &out);
        assert_eq!(result.code, exit, "stderr: {}", result.stderr);
        assert_eq!(stderr_json(&result)["error"]["pointer"], pointer);
        assert!(!out.exists(), "refused style input wrote {name} output");
    }
}
