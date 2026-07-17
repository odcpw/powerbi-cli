use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use walkdir::WalkDir;

fn run_powerbi(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
        .output()
        .expect("run powerbi-cli")
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout JSON")
}

#[test]
#[ignore = "requires the exact installed Microsoft report-authoring package; normal CI runs this after integrations install"]
fn exact_official_validator_accepts_valid_fixture_and_preserves_invalid_diagnostic_identity() {
    let cases: Value = serde_json::from_slice(
        &fs::read("testdata/conformance/microsoft/official-report-cases.json")
            .expect("conformance cases"),
    )
    .expect("conformance case JSON");
    let source_schema = cases["fixture"]["sourceSchema"]
        .as_str()
        .expect("source schema");
    let expected_invalid_code = cases["cases"][1]["expectedDiagnosticCode"]
        .as_str()
        .expect("expected diagnostic code");
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("neutral-sales");
    let scaffold = run_powerbi(&[
        "scaffold",
        "--schema",
        source_schema,
        "--out-dir",
        project.to_str().expect("project"),
        "--json",
    ]);
    assert!(
        scaffold.status.success(),
        "scaffold stderr: {}",
        String::from_utf8_lossy(&scaffold.stderr)
    );

    let valid = run_powerbi(&[
        "validate",
        project.to_str().expect("project"),
        "--backend",
        "microsoft-report",
        "--json",
    ]);
    assert!(
        valid.status.success(),
        "valid stdout: {}\nvalid stderr: {}",
        String::from_utf8_lossy(&valid.stdout),
        String::from_utf8_lossy(&valid.stderr)
    );
    let valid_json = stdout_json(&valid);
    assert_eq!(valid_json["backend"], "microsoft-report");
    assert_eq!(
        valid_json["validators"]["microsoftReport"]["version"],
        "0.1.4"
    );
    assert_eq!(
        valid_json["validators"]["microsoftReport"]["counts"]["errors"],
        0
    );
    add_neutral_legacy_alt_text(&project);
    let invalid = run_powerbi(&[
        "validate",
        project.to_str().expect("project"),
        "--backend",
        "microsoft-report",
        "--json",
    ]);
    assert_eq!(invalid.status.code(), Some(10));
    let invalid_json = stdout_json(&invalid);
    let diagnostics = invalid_json["validators"]["microsoftReport"]["diagnostics"]
        .as_array()
        .expect("official diagnostics");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == expected_invalid_code
            && diagnostic["severity"] == "error"
            && diagnostic["file"]
                .as_str()
                .is_some_and(|file| file.ends_with("visual.json"))
    }));

    let all = run_powerbi(&[
        "validate",
        project.to_str().expect("project"),
        "--backend",
        "all",
        "--json",
    ]);
    assert_eq!(all.status.code(), Some(10));
    let all_json = stdout_json(&all);
    assert_eq!(all_json["backend"], "all");
    assert_eq!(all_json["validators"]["native"]["ok"], true);
    assert_eq!(all_json["validators"]["microsoftReport"]["ok"], false);
}

fn add_neutral_legacy_alt_text(project: &Path) {
    let visual_path = WalkDir::new(project)
        .into_iter()
        .map(|entry| entry.expect("walk fixture"))
        .find(|entry| entry.file_type().is_file() && entry.file_name() == "visual.json")
        .expect("generated visual")
        .into_path();
    let mut visual: Value =
        serde_json::from_slice(&fs::read(&visual_path).expect("read visual")).expect("visual JSON");
    visual["visual"]["objects"]["general"] = serde_json::json!([{
        "properties": {
            "altText": {"expr": {"Literal": {"Value": "'Neutral legacy placement'"}}}
        }
    }]);
    fs::write(
        &visual_path,
        serde_json::to_vec_pretty(&visual).expect("serialize visual"),
    )
    .expect("write visual");
}
