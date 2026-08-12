use crate::{CliError, ResolvedProject, resolve_project};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const CONTRACT: &str = include_str!("../testdata/project-resolution-contract.v1.json");

struct Fixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    pbip: PathBuf,
    report: PathBuf,
    semantic_model: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary resolver fixture");
        let project = temp.path().join("project");
        let pbip = project.join("Project.pbip");
        let report = project.join("Project.Report");
        let semantic_model = project.join("Project.SemanticModel");
        fs::create_dir_all(&report).expect("report directory");
        fs::create_dir_all(&semantic_model).expect("semantic-model directory");
        write_json(
            &pbip,
            &json!({"artifacts": [{"report": {"path": "Project.Report"}}]}),
        );
        write_json(
            &report.join("definition.pbir"),
            &json!({
                "datasetReference": {"byPath": {"path": "../Project.SemanticModel"}}
            }),
        );
        Self {
            _temp: temp,
            project,
            pbip,
            report,
            semantic_model,
        }
    }

    fn write_pbip(&self, value: Value) {
        write_json(&self.pbip, &value);
    }

    fn write_pbir(&self, value: Value) {
        write_json(&self.report.join("definition.pbir"), &value);
    }
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("JSON parent");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize fixture JSON"),
    )
    .expect("write fixture JSON");
}

fn expected(case_id: &str) -> Value {
    let contract: Value = serde_json::from_str(CONTRACT).expect("resolution contract JSON");
    contract["cases"][case_id].clone()
}

fn assert_error(case_id: &str, result: Result<ResolvedProject, CliError>) {
    let expected = expected(case_id);
    assert_eq!(expected["outcome"], "error");
    let error = result.expect_err("case must fail");
    assert_eq!(error.code, expected["code"].as_str().expect("error code"));
    assert!(
        error.message.contains(
            expected["messageContains"]
                .as_str()
                .expect("message substring")
        ),
        "unexpected error message: {}",
        error.message
    );
}

fn assert_resolved(case_id: &str, fixture: &Fixture, resolved: ResolvedProject) {
    let expected = expected(case_id);
    assert_eq!(expected["outcome"], "resolved");
    let canonical_project = fs::canonicalize(&fixture.project).expect("canonical project");
    let path_for = |field: &str| {
        let suffix = expected[field].as_str().expect("path suffix");
        if suffix == "." {
            canonical_project.clone()
        } else {
            canonical_project.join(suffix)
        }
    };
    assert_eq!(
        fs::canonicalize(&resolved.project_dir).expect("canonical resolved project"),
        path_for("project")
    );
    assert_eq!(resolved.pbip_path, path_for("pbip"));
    assert_eq!(resolved.report_dir, path_for("report"));
    assert_eq!(resolved.semantic_model_dir, path_for("semanticModel"));
}

#[test]
fn directory_single_pbip_resolves() {
    let fixture = Fixture::new();
    let resolved = resolve_project(&fixture.project).expect("resolve directory");
    assert_resolved("directory_single_pbip_resolves", &fixture, resolved);
}

#[test]
fn explicit_pbip_resolves() {
    let fixture = Fixture::new();
    let resolved = resolve_project(&fixture.pbip).expect("resolve PBIP");
    assert_resolved("explicit_pbip_resolves", &fixture, resolved);
}

#[test]
fn missing_path_is_file_not_found() {
    let fixture = Fixture::new();
    assert_error(
        "missing_path_is_file_not_found",
        resolve_project(&fixture.project.join("missing")),
    );
}

#[test]
fn non_directory_is_invalid_args() {
    let fixture = Fixture::new();
    let path = fixture.project.join("ordinary.txt");
    fs::write(&path, b"ordinary").expect("ordinary file");
    assert_error("non_directory_is_invalid_args", resolve_project(&path));
}

#[test]
fn directory_without_pbip_is_file_not_found() {
    let fixture = Fixture::new();
    fs::remove_file(&fixture.pbip).expect("remove PBIP");
    assert_error(
        "directory_without_pbip_is_file_not_found",
        resolve_project(&fixture.project),
    );
}

#[test]
fn directory_with_multiple_pbips_is_invalid_args() {
    let fixture = Fixture::new();
    fs::copy(&fixture.pbip, fixture.project.join("Other.pbip")).expect("second PBIP");
    assert_error(
        "directory_with_multiple_pbips_is_invalid_args",
        resolve_project(&fixture.project),
    );
}

#[test]
fn missing_explicit_pbip_is_file_not_found() {
    let fixture = Fixture::new();
    let path = fixture.project.join("Missing.pbip");
    assert_error(
        "missing_explicit_pbip_is_file_not_found",
        resolve_project(&path),
    );
}

#[test]
fn missing_report_reference_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbip(json!({"artifacts": [{}]}));
    assert_error(
        "missing_report_reference_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[test]
fn empty_report_reference_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbip(json!({"artifacts": [{"report": {"path": "  "}}]}));
    assert_error(
        "empty_report_reference_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[test]
fn absolute_report_reference_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbip(json!({"artifacts": [{"report": {"path": "/tmp/outside"}}]}));
    assert_error(
        "absolute_report_reference_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[test]
fn colon_report_reference_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbip(json!({"artifacts": [{"report": {"path": "C:/outside"}}]}));
    assert_error(
        "colon_report_reference_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[test]
fn lexical_report_escape_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbip(json!({"artifacts": [{"report": {"path": "../outside.Report"}}]}));
    assert_error(
        "lexical_report_escape_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[cfg(unix)]
#[test]
fn report_symlink_escape_is_validation_failed() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let outside = fixture
        .project
        .parent()
        .expect("project parent")
        .join("outside.Report");
    fs::create_dir_all(&outside).expect("outside report");
    symlink(&outside, fixture.project.join("Linked.Report")).expect("outside report link");
    fixture.write_pbip(json!({"artifacts": [{"report": {"path": "Linked.Report"}}]}));
    assert_error(
        "report_symlink_escape_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[cfg(unix)]
#[test]
fn report_symlink_inside_resolves_canonical_target() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let canonical = fixture.project.join("Canonical.Report");
    fs::rename(&fixture.report, &canonical).expect("rename report");
    symlink(&canonical, &fixture.report).expect("inside report link");
    fixture.write_pbip(json!({"artifacts": [{"report": {"path": "Project.Report"}}]}));
    let resolved = resolve_project(&fixture.pbip).expect("resolve inside report link");
    assert_resolved(
        "report_symlink_inside_resolves_canonical_target",
        &fixture,
        resolved,
    );
}

#[test]
fn missing_definition_pbir_is_file_not_found() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.report.join("definition.pbir")).expect("remove definition.pbir");
    assert_error(
        "missing_definition_pbir_is_file_not_found",
        resolve_project(&fixture.pbip),
    );
}

#[test]
fn missing_semantic_reference_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbir(json!({"datasetReference": {}}));
    assert_error(
        "missing_semantic_reference_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[test]
fn by_connection_semantic_reference_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbir(json!({"datasetReference": {"byConnection": {"connectionString": "x"}}}));
    assert_error(
        "by_connection_semantic_reference_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[test]
fn semantic_lexical_escape_is_validation_failed() {
    let fixture = Fixture::new();
    fixture.write_pbir(json!({
        "datasetReference": {"byPath": {"path": "../../outside.SemanticModel"}}
    }));
    assert_error(
        "semantic_lexical_escape_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[cfg(unix)]
#[test]
fn semantic_symlink_escape_is_validation_failed() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let outside = fixture
        .project
        .parent()
        .expect("project parent")
        .join("outside.SemanticModel");
    fs::create_dir_all(&outside).expect("outside semantic model");
    symlink(&outside, fixture.project.join("Linked.SemanticModel"))
        .expect("outside semantic-model link");
    fixture.write_pbir(json!({
        "datasetReference": {"byPath": {"path": "../Linked.SemanticModel"}}
    }));
    assert_error(
        "semantic_symlink_escape_is_validation_failed",
        resolve_project(&fixture.pbip),
    );
}

#[cfg(unix)]
#[test]
fn semantic_symlink_inside_resolves_canonical_target() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let canonical = fixture.project.join("Canonical.SemanticModel");
    fs::rename(&fixture.semantic_model, &canonical).expect("rename semantic model");
    symlink(&canonical, &fixture.semantic_model).expect("inside semantic-model link");
    let resolved = resolve_project(&fixture.pbip).expect("resolve inside semantic-model link");
    assert_resolved(
        "semantic_symlink_inside_resolves_canonical_target",
        &fixture,
        resolved,
    );
}

#[test]
fn missing_semantic_target_remains_in_project() {
    let fixture = Fixture::new();
    fixture.write_pbir(json!({
        "datasetReference": {"byPath": {"path": "../Missing.SemanticModel"}}
    }));
    let resolved = resolve_project(&fixture.pbip).expect("resolve missing in-project target");
    assert_resolved(
        "missing_semantic_target_remains_in_project",
        &fixture,
        resolved,
    );
}

#[test]
fn relative_components_are_normalized() {
    let fixture = Fixture::new();
    fixture.write_pbip(json!({
        "artifacts": [{"report": {"path": "./nested/../Project.Report//"}}]
    }));
    fixture.write_pbir(json!({
        "datasetReference": {"byPath": {"path": "./../Project.SemanticModel"}}
    }));
    let resolved = resolve_project(&fixture.pbip).expect("resolve normalized references");
    assert_resolved("relative_components_are_normalized", &fixture, resolved);
}

#[test]
fn semantic_negative_control_escape_should_resolve() {
    let fixture = Fixture::new();
    if std::env::var_os("POWERBI_RESOLUTION_NEGATIVE_CONTROL").is_some() {
        fixture.write_pbip(json!({
            "artifacts": [{"report": {"path": "../outside.Report"}}]
        }));
    }
    let resolved = resolve_project(&fixture.pbip)
        .expect("negative control deliberately predicts that the escaped reference resolves");
    assert_resolved("explicit_pbip_resolves", &fixture, resolved);
}
