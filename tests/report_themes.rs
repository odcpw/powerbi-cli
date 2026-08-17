//! Report theme extraction, preset, and application integration tests.

mod common;

use common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn install_registered_theme(project: &Path, theme_name: &str, colors: &[&str]) -> PathBuf {
    let report_dir = project.join("SalesOperations.Report");
    let resource_dir = report_dir
        .join("StaticResources")
        .join("RegisteredResources");
    fs::create_dir_all(&resource_dir).expect("theme resource dir");
    let theme_path = resource_dir.join("CorpTheme.json");
    fs::write(
        &theme_path,
        serde_json::to_string_pretty(&json!({
            "name": theme_name,
            "dataColors": colors,
            "background": "#FFFFFF",
            "foreground": "#222222",
            "tableAccent": colors.first().copied().unwrap_or("#4472C4")
        }))
        .expect("theme json"),
    )
    .expect("write theme resource");

    let report_json_path = report_dir.join("definition").join("report.json");
    let mut report_json: Value =
        serde_json::from_str(&fs::read_to_string(&report_json_path).expect("report json"))
            .expect("parse report json");
    report_json["themeCollection"] = json!({
        "customTheme": {
            "name": theme_name,
            "resource": "StaticResources/RegisteredResources/CorpTheme.json"
        }
    });
    fs::write(
        &report_json_path,
        serde_json::to_string_pretty(&report_json).expect("report json text"),
    )
    .expect("write report json");
    theme_path
}
#[test]
fn report_themes_extract_and_apply_raw_bundle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(&temp.path().join("source"));
    let target = scaffold_sales(&temp.path().join("target"));
    let source_theme = install_registered_theme(
        &source,
        "Corporate Safety",
        &["#004B87", "#E87722", "#4A7729"],
    );
    let source_arg = source.to_str().expect("source path");
    let target_arg = target.to_str().expect("target path");

    let show_empty = run_powerbi(&[
        "report",
        "themes",
        "show",
        "--project",
        target_arg,
        "--json",
    ]);
    assert_eq!(show_empty.code, 0, "stderr: {}", show_empty.stderr);
    let show_empty_json = stdout_json(&show_empty);
    assert_eq!(
        show_empty_json["schema"],
        Value::from("powerbi-cli.report.themes.show.v1")
    );
    assert_eq!(show_empty_json["theme"]["state"], Value::from("none"));

    let bundle_path = temp.path().join("theme-bundle.json");
    let bundle_arg = bundle_path.to_str().expect("bundle path");
    let extract = run_powerbi(&[
        "report",
        "themes",
        "extract",
        "--project",
        source_arg,
        "--out",
        bundle_arg,
        "--json",
    ]);
    assert_eq!(extract.code, 0, "stderr: {}", extract.stderr);
    let extract_json = stdout_json(&extract);
    assert_eq!(
        extract_json["bundle"]["schema"],
        Value::from("powerbi-cli.report.theme-bundle.v1")
    );
    assert!(bundle_path.is_file());
    assert!(
        extract_json["bundle"]["sourceFingerprint"]
            .as_str()
            .unwrap_or_default()
            .starts_with("fnv64:")
    );
    assert!(
        extract_json["bundle"]["registeredThemes"][0]["relativePath"]
            .as_str()
            .unwrap()
            .contains("StaticResources/RegisteredResources/CorpTheme.json")
    );

    let target_report_json = target
        .join("SalesOperations.Report")
        .join("definition")
        .join("report.json");
    let target_before = fs::read_to_string(&target_report_json).expect("target before");
    let dry_run = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        bundle_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(
        dry_json["schema"],
        Value::from("powerbi-cli.report.themes.mutation.v1")
    );
    assert_eq!(dry_json["dryRun"], Value::Bool(true));
    assert_eq!(
        fs::read_to_string(&target_report_json).expect("target after dry-run"),
        target_before
    );

    let themed = temp.path().join("target-themed");
    let themed_arg = themed.to_str().expect("themed path");
    let apply = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        bundle_arg,
        "--out-dir",
        themed_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let apply_json = stdout_json(&apply);
    assert_eq!(apply_json["ok"], Value::Bool(true));
    assert_eq!(apply_json["mode"], Value::from("out-dir"));
    assert_eq!(
        fs::read_to_string(&target_report_json).expect("target after out-dir"),
        target_before
    );

    let readback = run_powerbi(&[
        "report",
        "themes",
        "show",
        "--project",
        themed_arg,
        "--json",
    ]);
    assert_eq!(readback.code, 0, "stderr: {}", readback.stderr);
    let readback_json = stdout_json(&readback);
    assert_eq!(readback_json["theme"]["state"], Value::from("referenced"));
    assert_eq!(
        readback_json["theme"]["registeredThemes"][0]["name"],
        Value::from("CorpTheme.json")
    );
    let copied_theme = themed
        .join("SalesOperations.Report")
        .join("StaticResources")
        .join("RegisteredResources")
        .join("CorpTheme.json");
    let copied_theme_json: Value =
        serde_json::from_str(&fs::read_to_string(&copied_theme).expect("copied theme"))
            .expect("copied theme json");
    let source_theme_json: Value =
        serde_json::from_str(&fs::read_to_string(&source_theme).expect("source theme"))
            .expect("source theme json");
    assert_eq!(copied_theme_json["name"], Value::from("CorpTheme.json"));
    assert_eq!(
        copied_theme_json["dataColors"], source_theme_json["dataColors"],
        "theme application should normalize only the host-managed name"
    );
}

#[test]
fn report_theme_preset_uses_schema_three_version_object() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = scaffold_sales(temp.path());
    let source_arg = source.to_str().expect("source path");
    let source_report = report_json(&source);
    let mut report: Value =
        serde_json::from_str(&fs::read_to_string(&source_report).expect("source report JSON"))
            .expect("parse source report JSON");
    report["$schema"] = Value::from(
        "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.3.0/schema.json",
    );
    fs::write(
        &source_report,
        serde_json::to_string_pretty(&report).expect("schema-three report JSON"),
    )
    .expect("write schema-three report JSON");

    let themed = temp.path().join("schema-three-themed");
    let themed_arg = themed.to_str().expect("themed path");
    let apply = run_powerbi(&[
        "report",
        "themes",
        "apply-preset",
        "--project",
        source_arg,
        "--preset",
        "risk-dashboard",
        "--out-dir",
        themed_arg,
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);

    let themed_report: Value = serde_json::from_str(
        &fs::read_to_string(report_json(&themed)).expect("themed report JSON"),
    )
    .expect("parse themed report JSON");
    assert_eq!(
        themed_report["themeCollection"]["customTheme"]["reportVersionAtImport"],
        json!({
            "visual": "2.10.0",
            "page": "2.3.1",
            "report": "3.4.0"
        })
    );

    let validation = run_powerbi(&["validate", themed_arg, "--strict", "--json"]);
    assert_eq!(validation.code, 0, "stderr: {}", validation.stderr);

    let mut malformed = themed_report;
    malformed["themeCollection"]["customTheme"]["reportVersionAtImport"] = json!({
        "visual": "2.10.0",
        "report": "not-a-version"
    });
    fs::write(
        report_json(&themed),
        serde_json::to_string_pretty(&malformed).expect("malformed report JSON"),
    )
    .expect("write malformed report JSON");
    let rejected = run_powerbi(&["validate", themed_arg, "--json"]);
    assert_eq!(rejected.code, 10);
    assert!(
        stdout_json(&rejected)["errors"]
            .as_array()
            .expect("validation errors")
            .iter()
            .any(|error| error
                .as_str()
                .unwrap_or_default()
                .contains("reportVersionAtImport must match"))
    );
}

#[test]
fn report_themes_apply_rejects_unsafe_or_wrong_bundle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = scaffold_sales(temp.path());
    let target_arg = target.to_str().expect("target path");
    let unsafe_bundle = temp.path().join("unsafe-theme-bundle.json");
    fs::write(
        &unsafe_bundle,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.report.theme-bundle.v1",
            "themeCollection": {
                "customTheme": {
                    "name": "Unsafe",
                    "note": "https://example.invalid/theme.json"
                }
            },
            "registeredThemes": []
        }))
        .expect("unsafe bundle json"),
    )
    .expect("write unsafe bundle");
    let unsafe_arg = unsafe_bundle.to_str().expect("unsafe bundle path");
    let rejected = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        unsafe_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(rejected.code, 10);
    assert!(
        stderr_json(&rejected)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("external URI")
    );

    let safe_bundle = temp.path().join("safe-theme-bundle.json");
    fs::write(
        &safe_bundle,
        serde_json::to_string_pretty(&json!({
            "schema": "powerbi-cli.report.theme-bundle.v1",
            "themeCollection": {},
            "registeredThemes": []
        }))
        .expect("safe bundle json"),
    )
    .expect("write safe bundle");
    let safe_arg = safe_bundle.to_str().expect("safe bundle path");
    let missing_mode = run_powerbi(&[
        "report",
        "themes",
        "apply",
        "--project",
        target_arg,
        "--bundle",
        safe_arg,
        "--json",
    ]);
    assert_eq!(missing_mode.code, 2);
    assert!(
        stderr_json(&missing_mode)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("requires --dry-run")
    );
}
