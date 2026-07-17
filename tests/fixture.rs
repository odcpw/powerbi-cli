use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct RunOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_powerbi(args: &[&str]) -> RunOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
        .output()
        .expect("run powerbi-cli binary");
    RunOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn run_powerbi_without_oracle(args: &[&str]) -> RunOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .env_remove("POWERBI_DESKTOP_ORACLE")
        .args(args)
        .output()
        .expect("run powerbi-cli binary");
    RunOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

fn scaffold_sales(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

fn scaffold_sales_with_desktop_filter_contract(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_desktop_filter_contract");
    let out = out_dir.to_str().expect("output path");
    let scaffold = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(scaffold.code, 0, "stderr: {}", scaffold.stderr);

    let report_filter = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        out,
        "--scope",
        "report",
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--in-place",
        "--json",
    ]);
    assert_eq!(report_filter.code, 0, "stderr: {}", report_filter.stderr);

    let page_filter = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        out,
        "--page",
        "page:ReportSectionOverview",
        "--target",
        "DimDate[Month]",
        "--value",
        "Jan",
        "--in-place",
        "--json",
    ]);
    assert_eq!(page_filter.code, 0, "stderr: {}", page_filter.stderr);

    out_dir
}

fn report_pages_json(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join("pages.json")
}

#[cfg(windows)]
fn sales_report_version_json(project: &Path) -> PathBuf {
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("version.json")
}

fn first_page_json(project: &Path) -> PathBuf {
    let pages_json: Value =
        serde_json::from_str(&fs::read_to_string(report_pages_json(project)).expect("pages json"))
            .expect("parse pages json");
    let page_name = pages_json["pageOrder"][0]
        .as_str()
        .expect("first page name");
    project
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join(page_name)
        .join("page.json")
}

fn first_two_visual_names(project: &Path) -> (String, String) {
    let page_json = first_page_json(project);
    let visuals_dir = page_json.parent().expect("page dir").join("visuals");
    let mut visual_json_paths = fs::read_dir(visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("file type").is_dir())
        .map(|entry| entry.path().join("visual.json"))
        .collect::<Vec<_>>();
    visual_json_paths.sort();
    let names = visual_json_paths
        .iter()
        .take(2)
        .map(|path| {
            let value: Value =
                serde_json::from_str(&fs::read_to_string(path).expect("visual json"))
                    .expect("parse visual json");
            value["name"]
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| {
                    path.parent()
                        .and_then(Path::file_name)
                        .and_then(|name| name.to_str())
                        .map(ToOwned::to_owned)
                })
                .expect("visual name")
        })
        .collect::<Vec<_>>();
    assert!(names.len() >= 2, "sales fixture should contain two visuals");
    (names[0].clone(), names[1].clone())
}

fn patch_json(path: &Path, patch: impl FnOnce(&mut Value)) {
    let mut value: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("json text")).expect("parse json");
    patch(&mut value);
    fs::write(
        path,
        serde_json::to_string_pretty(&value).expect("json pretty"),
    )
    .expect("write json");
}

#[test]
fn golden_directory_has_no_stale_actual_artifacts() {
    let mut actual_paths = Vec::new();
    collect_actual_paths(Path::new("testdata/golden"), &mut actual_paths);
    assert!(
        actual_paths.is_empty(),
        "remove stale fixture mismatch artifacts before committing: {actual_paths:?}"
    );
}

fn collect_actual_paths(dir: &Path, actual_paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read golden dir") {
        let entry = entry.expect("read golden entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("golden entry file type");
        if file_type.is_dir() {
            collect_actual_paths(&path, actual_paths);
        } else if path.extension().and_then(|value| value.to_str()) == Some("actual") {
            actual_paths.push(path);
        }
    }
}

#[test]
fn fixture_normalize_matches_committed_sales_golden() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let first = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let second = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "fixture normalize is not deterministic"
    );
    assert!(
        !first
            .stdout
            .contains(temp.path().to_str().expect("temp path")),
        "fixture summary leaked an absolute temp path"
    );

    let actual = stdout_json(&first);
    assert!(
        actual["report"]["pages"][0]["visuals"][0]["fingerprints"]["visualContainerObjects"]
            .is_string(),
        "fixture summaries should fingerprint shared visual-container formatting separately"
    );
    let expected: Value =
        serde_json::from_str(include_str!("../testdata/golden/sales.summary.json"))
            .expect("sales golden JSON");
    assert_eq!(actual, expected);
}

#[test]
fn fixture_normalize_matches_committed_sales_desktop_filter_contract_golden() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_with_desktop_filter_contract(temp.path());
    let project_arg = project.to_str().expect("project path");

    let first = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let second = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "sales desktop filter contract fixture normalize is not deterministic"
    );
    assert!(
        !first
            .stdout
            .contains(temp.path().to_str().expect("temp path")),
        "fixture summary leaked an absolute temp path"
    );

    let actual = stdout_json(&first);
    let expected: Value = serde_json::from_str(include_str!(
        "../testdata/golden/sales-desktop-filter-contract.summary.json"
    ))
    .expect("sales desktop filter contract golden JSON");
    assert_eq!(actual, expected);
}

#[test]
fn fixture_summary_captures_desktop_filter_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales_with_desktop_filter_contract(temp.path());
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["pbir"]["reportDefinitionVersion"],
        Value::from("2.0.0")
    );
    assert_eq!(value["pbir"]["filters"]["counts"]["total"], Value::from(2));
    assert_eq!(value["pbir"]["filters"]["counts"]["report"], Value::from(1));
    assert_eq!(value["pbir"]["filters"]["counts"]["page"], Value::from(1));
    assert_eq!(
        value["pbir"]["filters"]["counts"]["unsupported"],
        Value::from(0)
    );

    let filters = value["pbir"]["filters"]["items"]
        .as_array()
        .expect("filter items");
    assert_eq!(filters.len(), 2);
    for filter in filters {
        assert_eq!(filter["filterType"], Value::from("Categorical"));
        assert_eq!(filter["desktopSafeName"], Value::Bool(true));
        assert_eq!(filter["categoricalVersion"], Value::from(2));
        assert_eq!(filter["fromCount"], Value::from(1));
        assert_eq!(filter["whereCount"], Value::from(1));
        assert_eq!(filter["whereUsesSourceAlias"], Value::Bool(true));
        assert!(filter.get("path").is_none(), "filter summary leaked path");
        assert!(
            filter.get("raw").is_none(),
            "filter summary leaked raw PBIR"
        );
    }
    assert!(filters.iter().any(|filter| {
        filter["scope"] == "report" && filter["target"]["table"] == "DimCustomer"
    }));
    assert!(filters.iter().any(|filter| {
        filter["scope"] == "page"
            && filter["owner"]["handle"] == "page:ReportSectionOverview"
            && filter["target"]["table"] == "DimDate"
    }));
}

#[test]
fn fixture_normalize_captures_page_interactions_without_raw_pbir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let (source, target) = first_two_visual_names(&project);

    patch_json(&first_page_json(&project), |page| {
        page["visualInteractions"] = json!([
            {
                "source": source.clone(),
                "target": target.clone(),
                "type": "NoFilter"
            },
            {
                "source": target.clone(),
                "target": "MissingVisualForFixtureSummary",
                "type": "SurpriseMode"
            }
        ]);
    });

    let output = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(value["counts"]["explicitInteractions"], Value::from(2));
    assert_eq!(value["counts"]["unsupportedInteractions"], Value::from(1));
    assert_eq!(
        value["counts"]["staleInteractionVisualReferences"],
        Value::from(1)
    );
    assert_eq!(
        value["report"]["interactionSemantics"]["mode"],
        Value::from("explicit-overrides")
    );
    assert_eq!(
        value["report"]["pages"][0]["interactionCount"],
        Value::from(2)
    );
    let interactions = value["report"]["pages"][0]["interactions"]
        .as_array()
        .expect("interactions");
    assert_eq!(interactions.len(), 2);
    assert_eq!(interactions[0]["sourceName"], Value::from(source.clone()));
    assert_eq!(interactions[0]["targetName"], Value::from(target.clone()));
    assert_eq!(interactions[0]["interactionType"], Value::from("NoFilter"));
    assert_eq!(interactions[0]["source"]["found"], Value::Bool(true));
    assert_eq!(interactions[0]["source"]["name"], Value::from(source));
    assert_eq!(interactions[0]["target"]["found"], Value::Bool(true));
    assert_eq!(
        interactions[0]["target"]["name"],
        Value::from(target.clone())
    );
    assert_eq!(interactions[0]["unsupported"], Value::Bool(false));
    assert_eq!(interactions[0]["staleVisualReference"], Value::Bool(false));
    assert_eq!(interactions[1]["sourceName"], Value::from(target.clone()));
    assert_eq!(
        interactions[1]["targetName"],
        Value::from("MissingVisualForFixtureSummary")
    );
    assert_eq!(
        interactions[1]["interactionType"],
        Value::from("SurpriseMode")
    );
    assert_eq!(interactions[1]["source"]["found"], Value::Bool(true));
    assert_eq!(interactions[1]["source"]["name"], Value::from(target));
    assert_eq!(interactions[1]["target"]["found"], Value::Bool(false));
    assert_eq!(
        interactions[1]["target"]["name"],
        Value::from("MissingVisualForFixtureSummary")
    );
    assert_eq!(interactions[1]["staleVisualReference"], Value::Bool(true));
    assert_eq!(interactions[1]["unsupported"], Value::Bool(true));
    assert!(
        !output.stdout.contains("\"visualInteractions\":"),
        "fixture summary should not copy raw PBIR page properties"
    );
    assert!(
        !output.stdout.contains("\"path\""),
        "fixture summary should not include source PBIR paths"
    );
    assert!(
        !output.stdout.contains("\"raw\""),
        "fixture summary should not include raw PBIR blobs"
    );
}

#[test]
fn fixture_verify_accepts_golden_and_reports_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let expected = Path::new("testdata/golden/sales.summary.json");
    let expected_arg = expected.to_str().expect("expected path");

    let ok = run_powerbi(&[
        "fixture",
        "verify",
        project_arg,
        "--expected",
        expected_arg,
        "--json",
    ]);
    assert_eq!(ok.code, 0, "stderr: {}", ok.stderr);
    let ok_json = stdout_json(&ok);
    assert_eq!(
        ok_json["schema"],
        Value::from("powerbi-cli.fixture.summary.v1")
    );
    assert_eq!(ok_json["verification"]["same"], Value::Bool(true));

    let mut wrong: Value =
        serde_json::from_str(include_str!("../testdata/golden/sales.summary.json"))
            .expect("sales golden JSON");
    wrong["counts"]["visuals"] = Value::from(99);
    let wrong_path = temp.path().join("wrong.summary.json");
    fs::write(
        &wrong_path,
        serde_json::to_string_pretty(&wrong).expect("serialize wrong golden"),
    )
    .expect("write wrong golden");
    let wrong_arg = wrong_path.to_str().expect("wrong expected path");
    let mismatch = run_powerbi(&[
        "fixture",
        "verify",
        project_arg,
        "--expected",
        wrong_arg,
        "--json",
    ]);
    assert_eq!(mismatch.code, 10);
    let mismatch_json = stdout_json(&mismatch);
    assert_eq!(mismatch_json["ok"], Value::Bool(false));
    assert_eq!(mismatch_json["verification"]["same"], Value::Bool(false));
    assert!(
        mismatch_json["verification"]["differences"]
            .as_array()
            .expect("differences")
            .iter()
            .any(|difference| difference["path"] == "/counts/visuals")
    );
    assert_eq!(
        mismatch_json["verification"]["actualWritten"],
        Value::Null,
        "fixture verify must be read-only unless --write-actual is explicit"
    );
    assert_eq!(
        mismatch_json["verification"]["actual"]["counts"]["visuals"],
        Value::from(3)
    );
    assert!(
        !wrong_path.with_extension("actual").exists(),
        "default mismatch must not create an implicit .actual file"
    );

    let explicit_actual = temp.path().join("explicit-actual.json");
    let explicit_actual_arg = explicit_actual.to_str().expect("actual output path");
    let mismatch_with_artifact = run_powerbi(&[
        "fixture",
        "verify",
        project_arg,
        "--expected",
        wrong_arg,
        "--write-actual",
        explicit_actual_arg,
        "--json",
    ]);
    assert_eq!(mismatch_with_artifact.code, 10);
    let artifact_json = stdout_json(&mismatch_with_artifact);
    let actual_written = artifact_json["verification"]["actualWritten"]
        .as_str()
        .expect("explicit actual written path");
    assert_eq!(
        fs::canonicalize(actual_written).expect("canonical actualWritten"),
        fs::canonicalize(&explicit_actual).expect("canonical explicit actual")
    );
    let written_actual: Value = serde_json::from_str(
        &fs::read_to_string(&explicit_actual).expect("read explicit actual artifact"),
    )
    .expect("parse explicit actual artifact");
    assert_eq!(written_actual, artifact_json["verification"]["actual"]);
}

#[test]
fn fixture_verify_reports_interaction_pointer_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let (source, target) = first_two_visual_names(&project);

    patch_json(&first_page_json(&project), |page| {
        page["visualInteractions"] = json!([{
            "source": source,
            "target": target,
            "type": "NoFilter"
        }]);
    });

    let normalize = run_powerbi(&["fixture", "normalize", project_arg, "--json"]);
    assert_eq!(normalize.code, 0, "stderr: {}", normalize.stderr);
    let mut wrong = stdout_json(&normalize);
    wrong["report"]["pages"][0]["interactions"][0]["interactionType"] =
        Value::from("HighlightFilter");
    let wrong_path = temp.path().join("wrong-interactions.summary.json");
    fs::write(
        &wrong_path,
        serde_json::to_string_pretty(&wrong).expect("serialize wrong golden"),
    )
    .expect("write wrong golden");
    let wrong_arg = wrong_path.to_str().expect("wrong expected path");

    let mismatch = run_powerbi(&[
        "fixture",
        "verify",
        project_arg,
        "--expected",
        wrong_arg,
        "--json",
    ]);
    assert_eq!(mismatch.code, 10);
    let mismatch_json = stdout_json(&mismatch);
    assert!(
        mismatch_json["verification"]["differences"]
            .as_array()
            .expect("differences")
            .iter()
            .any(|difference| {
                difference["path"] == "/report/pages/0/interactions/0/interactionType"
            })
    );
}

#[test]
#[cfg(windows)]
fn desktop_open_check_is_structured_when_oracle_is_disabled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let output = run_powerbi_without_oracle(&["desktop", "open-check", project_arg, "--json"]);
    assert_eq!(output.code, 30, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.desktop.openCheck.v1")
    );
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(
        value["oracle"]["detection"]["oracleEnabled"],
        Value::Bool(false)
    );
    assert_eq!(value["validation"]["ok"], Value::Bool(true));
    assert_eq!(value["validation"]["strict"]["enabled"], Value::Bool(true));
    assert_eq!(value["validation"]["strict"]["ok"], Value::Bool(true));
    assert!(value["validation"]["strict"]["lint"]["findings"].is_array());
    assert_eq!(value["changes"], json!([]));
    assert_eq!(value["proof"]["level"], Value::from("unit-smoke"));
    assert_eq!(
        value["proof"]["observedStage"],
        Value::from("not-attempted")
    );
    assert_eq!(value["proof"]["claimedCompatibility"], Value::Bool(false));
    assert_eq!(
        value["proof"]["requiredCompatibilityLevel"],
        Value::from("desktop-canvas-refresh")
    );
    assert_eq!(value["proof"]["requiresManualReview"], Value::Bool(true));
    assert_eq!(
        value["proof"]["signals"]["cleanup"]["requested"],
        Value::Bool(true)
    );
    assert_eq!(
        value["proof"]["signals"]["cleanup"]["attempted"],
        Value::Bool(false)
    );
    assert_eq!(value["proof"]["signals"]["canvasRendered"], Value::Null);
    assert!(
        value["proof"]["unprovenSignals"]
            .as_array()
            .expect("unproven signals")
            .iter()
            .any(|signal| signal == "canvasRendered")
    );
    assert_eq!(
        value["proof"]["compatibility"]["claimed"],
        Value::Bool(false)
    );
    assert!(
        value["proof"]["manualReview"]["checklist"]
            .as_array()
            .expect("manual review checklist")
            .iter()
            .any(|step| step.as_str().unwrap_or_default().contains("visuals render"))
    );
    assert!(
        value["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|diagnostic| diagnostic["code"] == "oracle_disabled")
    );
}

#[test]
#[cfg(windows)]
fn desktop_screenshot_is_structured_when_oracle_is_disabled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let screenshot = temp.path().join("proof").join("sales.png");
    let screenshot_arg = screenshot.to_str().expect("screenshot path");

    let output = run_powerbi_without_oracle(&[
        "desktop",
        "screenshot",
        project_arg,
        "--out",
        screenshot_arg,
        "--json",
    ]);
    assert_eq!(output.code, 30, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["schema"],
        Value::from("powerbi-cli.desktop.screenshot.v1")
    );
    assert_eq!(value["ok"], Value::Bool(false));
    assert_eq!(value["exitCode"], Value::from(30));
    assert_eq!(
        value["oracle"]["detection"]["oracleEnabled"],
        Value::Bool(false)
    );
    assert_eq!(value["changes"], json!([]));
    assert_eq!(value["proof"]["level"], Value::from("unit-smoke"));
    assert_eq!(
        value["proof"]["observedStage"],
        Value::from("not-attempted")
    );
    assert_eq!(value["proof"]["signals"]["windowObserved"], Value::Null);
    assert_eq!(value["proof"]["signals"]["titleMatched"], Value::Null);
    assert_eq!(
        value["proof"]["signals"]["screenshotCaptured"],
        Value::Bool(false)
    );
    assert_eq!(value["screenshot"]["captured"], Value::Bool(false));
    assert_eq!(value["screenshot"]["foregroundVerified"], Value::Null);
    assert_eq!(
        value["screenshot"]["allowUnverifiedCapture"],
        Value::Bool(false)
    );
    assert_eq!(
        value["screenshot"]["automatedCompatibilityProof"],
        Value::Bool(false)
    );
    assert!(
        value["screenshot"]["purpose"]
            .as_str()
            .expect("purpose")
            .contains("manual/agent review")
    );
    assert!(!screenshot.exists());
    assert!(
        value["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|diagnostic| diagnostic["code"] == "oracle_disabled")
    );
}

#[test]
#[cfg(windows)]
fn desktop_flag_errors_exit_two_with_hints() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let bad_timeout = run_powerbi_without_oracle(&[
        "desktop",
        "open-check",
        project_arg,
        "--timeout-ms",
        "soon",
        "--json",
    ]);
    assert_eq!(bad_timeout.code, 2);
    let bad_timeout_json = stderr_json(&bad_timeout);
    assert_eq!(
        bad_timeout_json["error"]["code"],
        Value::from("invalid_args")
    );
    assert!(bad_timeout_json["error"]["hint"].is_string());
    assert!(
        bad_timeout_json["error"]["suggestedCommands"]
            .as_array()
            .is_some_and(|commands| !commands.is_empty())
    );

    let missing_out = run_powerbi_without_oracle(&["desktop", "screenshot", project_arg, "--json"]);
    assert_eq!(missing_out.code, 2);
    let missing_out_json = stderr_json(&missing_out);
    assert!(
        missing_out_json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--out")
    );
    assert!(missing_out_json["error"]["hint"].is_string());

    let inside_project = project.join("desktop-evidence.png");
    let inside_project_arg = inside_project.to_str().expect("inside path");
    let rejected_out = run_powerbi_without_oracle(&[
        "desktop",
        "screenshot",
        project_arg,
        "--out",
        inside_project_arg,
        "--json",
    ]);
    assert_eq!(rejected_out.code, 2);
    let rejected_out_json = stderr_json(&rejected_out);
    assert!(
        rejected_out_json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("outside")
    );
    assert!(
        rejected_out_json["error"]["hint"]
            .as_str()
            .expect("hint")
            .contains("handoff")
    );
    assert!(!inside_project.exists());
}

#[test]
#[cfg(windows)]
fn desktop_open_check_refuses_strict_lint_failures_before_oracle_launch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    patch_json(&sales_report_version_json(&project), |version| {
        version["version"] = Value::from("9.9.9");
    });

    let output = run_powerbi_without_oracle(&["desktop", "open-check", project_arg, "--json"]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    assert_eq!(
        value["proof"]["status"],
        Value::from("strict-validation-failed")
    );
    assert_eq!(
        value["oracle"]["detection"]["oracleEnabled"],
        Value::Bool(false)
    );
    assert_eq!(value["validation"]["ok"], Value::Bool(true));
    assert_eq!(value["validation"]["strict"]["ok"], Value::Bool(false));
    assert!(
        value["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|diagnostic| diagnostic["code"] == "strict_preflight_failed")
    );
    assert!(
        value["diagnostics"][0]["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "pbir.report_definition_version")
    );
}

#[test]
#[cfg(not(windows))]
fn desktop_oracle_is_an_unsupported_feature_before_opt_in_on_non_windows() {
    let output = run_powerbi_without_oracle(&["desktop", "open-check", "missing.pbip", "--json"]);
    assert_eq!(output.code, 2, "stdout: {}", output.stdout);
    let value = stderr_json(&output);
    assert_eq!(value["error"]["code"], Value::from("unsupported_feature"));
    assert!(
        value["error"]["message"]
            .as_str()
            .expect("message")
            .contains("requires Windows")
    );
}

#[test]
fn capabilities_advertise_fixture_and_desktop_oracle_commands() {
    let output = run_powerbi(&["capabilities", "--json", "--for", "fixture"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let fixture_value = stdout_json(&output);
    let fixture_paths = fixture_value["commands"]
        .as_array()
        .expect("fixture commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(fixture_paths.contains(&"fixture normalize"));
    assert!(fixture_paths.contains(&"fixture verify"));
    let fixture_verify = fixture_value["commands"]
        .as_array()
        .expect("fixture commands")
        .iter()
        .find(|command| command["path"] == "fixture verify")
        .expect("fixture verify command");
    assert_eq!(fixture_verify["readOnly"], Value::Bool(true));
    assert_eq!(fixture_verify["readOnlyByDefault"], Value::Bool(true));
    assert!(
        fixture_verify["mutatingFlags"]
            .as_array()
            .expect("mutating flags")
            .iter()
            .any(|flag| flag == "--write-actual <path>")
    );
    assert!(
        fixture_verify["followUpFields"]
            .as_array()
            .expect("follow up fields")
            .iter()
            .any(|field| field == "verification.actual")
    );
    assert!(
        fixture_value["schemaManifest"]["fixtureSummaryFields"]
            .as_array()
            .expect("fixture summary fields")
            .iter()
            .any(|field| field == "fingerprint")
    );
    assert!(
        fixture_value["schemaManifest"]["fixtureReportInteractionFields"]
            .as_array()
            .expect("fixture report interaction fields")
            .iter()
            .any(|field| field == "staleVisualReference")
    );
    assert!(
        fixture_value["schemaManifest"]["fixturePbirFilterFields"]
            .as_array()
            .expect("fixture PBIR filter fields")
            .iter()
            .any(|field| field == "whereUsesSourceAlias")
    );

    let desktop = run_powerbi(&["capabilities", "--json", "--for", "desktop open-check"]);
    assert_eq!(desktop.code, 0, "stderr: {}", desktop.stderr);
    let desktop_value = stdout_json(&desktop);
    let command = desktop_value["commands"]
        .as_array()
        .expect("desktop commands")
        .iter()
        .find(|command| command["path"] == "desktop open-check")
        .expect("desktop open-check command");
    assert_eq!(
        command["outputSchema"],
        Value::from("powerbi-cli.desktop.openCheck.v1")
    );
    assert_eq!(command["readOnly"], Value::Bool(true));
    assert_eq!(command["proofLevel"], Value::from("unit-smoke"));
    assert_eq!(command["observedStage"], Value::from("desktop-window"));
    assert!(
        command["ciPolicy"]
            .as_str()
            .expect("ci policy")
            .contains("not a canvas/render/refresh compatibility claim")
    );
    assert!(
        command["followUpFields"]
            .as_array()
            .expect("follow up fields")
            .iter()
            .any(|field| field == "proof.claimedCompatibility")
    );
    assert!(
        command["followUpFields"]
            .as_array()
            .expect("follow up fields")
            .iter()
            .any(|field| field == "proof.observedStage")
    );
    assert!(
        command["flags"]
            .as_array()
            .expect("flags")
            .iter()
            .any(|flag| flag == "--desktop-path <PBIDesktop.exe>")
    );

    let desktop_catalog = run_powerbi(&["capabilities", "--json", "--for", "desktop"]);
    assert_eq!(
        desktop_catalog.code, 0,
        "stderr: {}",
        desktop_catalog.stderr
    );
    let desktop_catalog_value = stdout_json(&desktop_catalog);
    let desktop_paths = desktop_catalog_value["commands"]
        .as_array()
        .expect("desktop command catalog")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(desktop_paths.contains(&"desktop open-check"));
    assert!(desktop_paths.contains(&"desktop screenshot"));
    let screenshot_command = desktop_catalog_value["commands"]
        .as_array()
        .expect("desktop command catalog")
        .iter()
        .find(|command| command["path"] == "desktop screenshot")
        .expect("desktop screenshot command");
    assert_eq!(screenshot_command["proofLevel"], Value::from("unit-smoke"));
    assert!(
        screenshot_command["flags"]
            .as_array()
            .expect("screenshot flags")
            .iter()
            .any(|flag| flag == "--allow-unverified-capture")
    );
    assert!(
        screenshot_command["captureSafety"]
            .as_str()
            .expect("capture safety")
            .contains("sensitive screen content")
    );
    assert_eq!(
        desktop_catalog_value["proofLevels"]
            .as_array()
            .expect("proof levels")
            .iter()
            .map(|level| level["name"].as_str().expect("proof level name"))
            .collect::<Vec<_>>(),
        vec![
            "unit-smoke",
            "schema-golden",
            "desktop-golden-pending",
            "manual-desktop-canvas-refresh",
            "desktop-canvas-refresh",
        ]
    );
    assert!(
        desktop_catalog_value["schemaManifest"]["desktopScreenshotFields"]
            .as_array()
            .expect("desktop screenshot fields")
            .iter()
            .any(|field| field == "screenshot.automatedCompatibilityProof")
    );
    assert!(
        desktop_catalog_value["schemaManifest"]["desktopScreenshotFields"]
            .as_array()
            .expect("desktop screenshot fields")
            .iter()
            .any(|field| field == "screenshot.foregroundVerified")
    );

    let features = run_powerbi(&["features", "list", "--for", "desktop", "--json"]);
    assert_eq!(features.code, 0, "stderr: {}", features.stderr);
    let features_value = stdout_json(&features);
    let desktop_feature = features_value["features"]
        .as_array()
        .expect("desktop features")
        .iter()
        .find(|feature| feature["id"] == "desktop.window-evidence")
        .expect("desktop window evidence feature");
    assert_eq!(desktop_feature["status"], Value::from("supported"));
    assert_eq!(desktop_feature["proofLevel"], Value::from("unit-smoke"));
    assert!(
        desktop_feature["commands"]
            .as_array()
            .expect("feature commands")
            .iter()
            .any(|command| command == "desktop screenshot")
    );
}
