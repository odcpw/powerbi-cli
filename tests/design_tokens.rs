mod common;

use common::{assert_tree_equal, run_powerbi, scaffold_sales, stderr_json, stdout_json};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

fn write_style_spec(path: &Path, tokens: Value) {
    let mut spec: Value = serde_json::from_str(
        &fs::read_to_string("examples/sales.dashboard.v2.json").expect("v2 dashboard spec"),
    )
    .expect("parse dashboard spec");
    spec["style"] = json!({"tokens": tokens});
    fs::write(
        path,
        serde_json::to_string_pretty(&spec).expect("serialize style spec"),
    )
    .expect("write style spec");
}

#[test]
fn token_catalog_show_is_complete_and_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let args = [
        "report",
        "style",
        "tokens",
        "show",
        "--project",
        project_arg,
        "--json",
    ];
    let first = run_powerbi(&args);
    let second = run_powerbi(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "catalog output must be deterministic"
    );
    let value = stdout_json(&first);
    assert_eq!(value["schema"], "powerbi-cli.report.style.tokens.show.v1");
    assert_eq!(
        value["catalog"]["sets"]
            .as_array()
            .expect("token sets")
            .iter()
            .map(|set| set["id"].as_str().expect("set id"))
            .collect::<Vec<_>>(),
        vec!["corporate-neutral", "high-contrast", "dark", "print"]
    );
    for key in ["good", "bad", "neutral", "warning", "emphasis"] {
        assert!(value["selected"]["tokens"]["semantic"][key].is_string());
    }
    assert!(value["selected"]["tokens"]["ramps"]["sequential"].is_array());
    assert!(value["selected"]["tokens"]["ramps"]["diverging"].is_array());
}

#[test]
fn every_builtin_compiles_to_its_exact_theme_golden() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let expected = [
        (
            "corporate-neutral",
            "21ccbeb65ea11ec5fcde5fa8293c88f238a89f97bd1ac96cca4d7ce9dde32079",
        ),
        (
            "high-contrast",
            "fe9c95f8ab7f35aba4f0663c77d7af38e065f4fd558cc58624642c5caa7c5824",
        ),
        (
            "dark",
            "9be612bfa36d32c53cea8be9c4668486759f1c0b0092d4b975277287aa1fa0c7",
        ),
        (
            "print",
            "b918b545765bafaa04fcd7f5da5295c64b9b1a3680ae3c125a8227be38a2c127",
        ),
    ];
    for (preset, expected_sha256) in expected {
        let output = run_powerbi(&[
            "report",
            "style",
            "tokens",
            "show",
            "--project",
            project_arg,
            "--preset",
            preset,
            "--json",
        ]);
        assert_eq!(output.code, 0, "{preset}: {}", output.stderr);
        let theme = stdout_json(&output)["selected"]["theme"].clone();
        let bytes = serde_json::to_vec(&theme).expect("serialize canonical theme golden");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected_sha256,
            "theme lowering changed for built-in {preset}"
        );
    }
}

#[test]
fn existing_theme_presets_remain_byte_identical_without_style_tokens() {
    let temp = tempfile::tempdir().expect("tempdir");
    let scaffold_source = temp.path().join("scaffold-source");
    let build_source = temp.path().join("build-source");
    for (command, output) in [
        ("scaffold", &scaffold_source),
        ("report-build", &build_source),
    ] {
        let output_arg = output.to_str().expect("source path");
        let result = if command == "scaffold" {
            run_powerbi(&[
                "scaffold",
                "--schema",
                "examples/sales.schema.json",
                "--out-dir",
                output_arg,
                "--json",
            ])
        } else {
            run_powerbi(&[
                "report",
                "build",
                "--schema",
                "examples/sales.schema.json",
                "--out-dir",
                output_arg,
                "--json",
            ])
        };
        assert_eq!(result.code, 0, "{command}: {}", result.stderr);
    }

    for preset in ["risk-dashboard", "neutral-ops"] {
        let scaffold_themed = temp.path().join(format!("scaffold-{preset}"));
        let build_themed = temp.path().join(format!("build-{preset}"));
        for (source, output) in [
            (&scaffold_source, &scaffold_themed),
            (&build_source, &build_themed),
        ] {
            let result = run_powerbi(&[
                "report",
                "themes",
                "apply-preset",
                "--project",
                source.to_str().expect("source path"),
                "--preset",
                preset,
                "--out-dir",
                output.to_str().expect("output path"),
                "--json",
            ]);
            assert_eq!(result.code, 0, "{preset}: {}", result.stderr);
        }
        assert_tree_equal(
            &scaffold_themed,
            &build_themed,
            &format!("existing theme preset {preset} changed without style.tokens"),
        );
    }
}

#[test]
fn contrast_failure_names_pair_and_explicit_waiver_is_recorded() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("low-contrast.dashboard.json");
    write_style_spec(&spec, json!({"semantic": {"good": "#FFFFFF"}}));
    let spec_arg = spec.to_str().expect("spec path");
    let failure = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(failure.code, 10, "stderr: {}", failure.stderr);
    let error = stderr_json(&failure)["error"].clone();
    assert_eq!(error["code"], "design.contrast_below_aa");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("semantic.good")
    );

    let allowed_spec = temp.path().join("allowed.dashboard.json");
    write_style_spec(
        &allowed_spec,
        json!({"semantic": {"good": "#FFFFFF"}, "allowContrastBelowAA": true}),
    );
    let project = temp.path().join("allowed-project");
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        allowed_spec.to_str().expect("allowed spec path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let build = stdout_json(&output);
    assert_eq!(build["styleTokens"]["allowContrastBelowAA"], true);
    assert_eq!(
        build["scorecard"]["styleTokens"]["allowContrastBelowAA"],
        true
    );
    assert_eq!(build["warnings"][0]["code"], "design.contrast_below_aa");
    let handoff = fs::read_to_string(project.join("POWERBI_HANDOFF.md")).expect("handoff");
    assert!(handoff.contains("style.tokens.allowContrastBelowAA"));
}

#[test]
fn style_tokens_build_registered_theme_formats_and_derive_without_literals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = temp.path().join("dark.dashboard.json");
    write_style_spec(&spec, json!({"preset": "dark"}));
    let project = temp.path().join("dark-project");
    let output = run_powerbi(&[
        "report",
        "build",
        "--schema",
        "examples/sales.schema.json",
        "--spec",
        spec.to_str().expect("spec path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let build = stdout_json(&output);
    assert_eq!(build["styleTokens"]["id"], "dark");
    let report_dir = project.join("SalesOperations.Report");
    let report: Value = serde_json::from_str(
        &fs::read_to_string(report_dir.join("definition/report.json")).expect("report"),
    )
    .expect("parse report");
    assert_eq!(
        report["themeCollection"]["customTheme"]["name"],
        "powerbi-cli-tokens-dark.json"
    );
    let theme: Value = serde_json::from_str(
        &fs::read_to_string(
            report_dir.join("StaticResources/RegisteredResources/powerbi-cli-tokens-dark.json"),
        )
        .expect("theme resource"),
    )
    .expect("parse theme");
    assert!(theme["dataColors"].is_array());
    assert!(theme["textClasses"].is_object());
    assert!(theme["visualStyles"]["*"]["labels"][0]["displayUnits"].is_number());
    let tmdl = fs::read_to_string(
        project.join("SalesOperations.SemanticModel/definition/tables/FactSales.tmdl"),
    )
    .expect("fact sales tmdl");
    assert!(tmdl.contains("formatString:"));

    let derive = run_powerbi(&[
        "report",
        "style",
        "tokens",
        "derive",
        "--project",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(derive.code, 0, "stderr: {}", derive.stderr);
    let derived = stdout_json(&derive);
    assert_eq!(
        derived["schema"],
        "powerbi-cli.report.style.tokens.derive.v1"
    );
    let serialized = serde_json::to_string(&derived["tokens"]).expect("tokens JSON");
    assert!(
        !serialized.contains("Revenue"),
        "derive must not copy literal titles"
    );
}
