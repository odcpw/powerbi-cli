mod common;

use common::{
    first_visual_json, patch_json, run_powerbi, scaffold_sales, stderr_json, stdout_json,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn first_visual_handle(project: &Path) -> String {
    let project = project.to_str().expect("project path");
    let output = run_powerbi(&["report", "visuals", "list", "--project", project, "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    stdout_json(&output)["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string()
}

fn harvest_args(project: &Path, handle: &str, out: &Path) -> Vec<String> {
    vec![
        "desktop".to_string(),
        "harvest-reference".to_string(),
        "--project".to_string(),
        project.display().to_string(),
        "--visual".to_string(),
        handle.to_string(),
        "--out".to_string(),
        out.display().to_string(),
        "--json".to_string(),
    ]
}

#[test]
fn harvest_reference_archives_visual_with_provenance_and_stable_bytes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let handle = first_visual_handle(&project);
    let out = temp.path().join("references").join("sales-visual.json");
    let args = harvest_args(&project, &handle, &out);

    let first = run_powerbi_owned(&args);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let first_json = stdout_json(&first);
    assert_eq!(first_json["proofLevel"], "desktop-golden-pending");
    assert_eq!(first_json["provenance"]["desktopVersion"], "unknown");
    assert_eq!(
        first_json["provenance"]["sourceFingerprint"]
            .as_str()
            .unwrap_or_default()
            .len(),
        71
    );
    assert_eq!(
        first_json["provenance"]["licenseNote"],
        "Source-project license and redistribution terms must be preserved with this reference."
    );
    assert_eq!(first_json["source"]["handle"], handle);
    let bytes = fs::read(&out).expect("reference output");
    let archived: Value = serde_json::from_slice(&bytes).expect("archived JSON");
    assert_eq!(archived["schema"], "powerbi-cli.desktop-reference.v1");
    assert_eq!(archived["fragment"]["visual"]["visualType"], "tableEx");

    let second = run_powerbi_owned(&args);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(bytes, fs::read(&out).expect("stable reference output"));
}

#[test]
fn harvest_reference_refuses_persisted_filter_state_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let handle = first_visual_handle(&project);
    let visual_path = project
        .join("SalesOperations.Report/definition/pages/ReportSectionOverview/visuals")
        .read_dir()
        .expect("visuals")
        .find_map(Result::ok)
        .expect("visual directory")
        .path()
        .join("visual.json");
    patch_json(&visual_path, |value| {
        value["visual"]["objects"]["general"] = json!([{
            "properties": {
                "filter": {
                    "filter": {
                        "Where": [{
                            "Condition": {
                                "In": {"Values": [[{"Literal": {"Value": "'persisted'"}}]]}
                            }
                        }]
                    }
                }
            }
        }]);
    });
    let out = temp.path().join("refused.json");
    let args = harvest_args(&project, &handle, &out);
    let output = run_powerbi_owned(&args);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "input_safety_violation");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("persisted data values")
    );
    assert!(!out.exists(), "refused state must not publish an archive");
}

#[test]
fn harvest_reference_refuses_oversized_fragment_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let handle = first_visual_handle(&project);
    let visual_path = first_visual_json(&project);
    patch_json(&visual_path, |value| {
        value["inputSafetyPadding"] = Value::String("x".repeat(4 * 1024 * 1024));
    });
    let out = temp.path().join("oversized-reference.json");
    let output = run_powerbi_owned(&harvest_args(&project, &handle, &out));
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "input_safety_violation");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("maximum is 4194304 bytes")
    );
    assert!(!out.exists(), "refused state must not publish an archive");
}

#[test]
fn harvest_reference_refuses_oversized_project_file_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let handle = first_visual_handle(&project);
    fs::write(
        project.join("oversized-project-file.txt"),
        vec![b'x'; 16 * 1024 * 1024 + 1],
    )
    .expect("oversized project file");
    let out = temp.path().join("oversized-project-reference.json");
    let output = run_powerbi_owned(&harvest_args(&project, &handle, &out));
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "input_safety_violation");
    assert!(!out.exists(), "refused state must not publish an archive");
}

#[test]
fn harvest_reference_archives_page_and_report_handles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let page_out = temp.path().join("references").join("overview-page.json");
    let report_out = temp.path().join("references").join("report.json");

    let page = run_powerbi_owned(&harvest_args(
        &project,
        "page:ReportSectionOverview",
        &page_out,
    ));
    assert_eq!(page.code, 0, "stderr: {}", page.stderr);
    assert_eq!(stdout_json(&page)["source"]["kind"], "page");
    assert_eq!(
        stdout_json(&page)["source"]["handle"],
        "page:ReportSectionOverview"
    );

    let report = run_powerbi_owned(&harvest_args(&project, "report:main", &report_out));
    assert_eq!(report.code, 0, "stderr: {}", report.stderr);
    assert_eq!(stdout_json(&report)["source"]["kind"], "report");
    assert_eq!(stdout_json(&report)["source"]["handle"], "report:main");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(report_out).expect("report archive"))
            .expect("report archive JSON")["kind"],
        "report"
    );
}

#[test]
fn harvest_reference_dry_run_previews_without_creating_destination() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let parent = temp.path().join("not-created").join("references");
    let out = parent.join("preview.json");
    let mut args = harvest_args(&project, "report:main", &out);
    args.push("--dry-run".to_string());

    let output = run_powerbi_owned(&args);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["action"], "preview");
    assert_eq!(value["dryRun"], true);
    assert!(!out.exists(), "dry-run must not publish an archive");
    assert!(
        !parent.exists(),
        "dry-run must not create output directories"
    );
}

fn run_powerbi_owned(args: &[String]) -> common::RunOutput {
    common::run_powerbi_owned(args)
}
