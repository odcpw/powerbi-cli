use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

fn run_powerbi(args: &[&str]) -> (i32, Value, String) {
    run_powerbi_in(args, None)
}

fn run_powerbi_in(args: &[&str], current_dir: Option<&Path>) -> (i32, Value, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"));
    command.args(args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    let output = command.output().expect("run powerbi-cli binary");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let value = if stdout.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(stdout.trim()).expect("stdout JSON")
    };
    (code, value, stderr)
}

#[test]
fn capabilities_advertise_scaffold_and_validate() {
    let (code, stdout, stderr) = run_powerbi(&["--json", "capabilities"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let paths = stdout["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|command| command["path"].as_str().expect("path"))
        .collect::<Vec<_>>();
    assert!(paths.contains(&"scaffold"));
    assert!(paths.contains(&"diff"));
    assert!(paths.contains(&"model tables add-static"));
    assert!(paths.contains(&"model calculated-columns add"));
    assert!(paths.contains(&"report visuals catalog"));
    assert!(paths.contains(&"report spec fields"));
    assert!(paths.contains(&"validate"));
}

#[test]
fn capabilities_advertise_generated_visual_contract_and_proof_statuses() {
    let (code, stdout, stderr) = run_powerbi(&["--json", "capabilities"]);
    assert_eq!(code, 0, "stderr: {stderr}");

    let visual_contract = &stdout["generatedVisualContract"];
    assert!(
        visual_contract["supportedVisualTypes"]
            .as_array()
            .expect("supported visual types")
            .iter()
            .any(|visual_type| visual_type == "scatterChart")
    );
    for visual_type in ["pieChart", "donutChart", "pivotTable", "slicer"] {
        assert!(
            visual_contract["bindingManualDesktopCanvasRefreshVisualTypes"]
                .as_array()
                .expect("manually canvas-proven binding types")
                .iter()
                .any(|proven| proven == visual_type)
        );
    }
    assert!(
        visual_contract["desktopGoldenPendingVisualTypes"]
            .as_array()
            .expect("title-bearing Desktop-pending visual types")
            .iter()
            .any(|pending| pending == "card")
    );
    let scatter = visual_contract["visualTypes"]
        .as_array()
        .expect("visual type contracts")
        .iter()
        .find(|visual_type| visual_type["visualType"] == "scatterChart")
        .expect("scatter contract");
    assert!(
        scatter["roles"]
            .as_array()
            .expect("scatter roles")
            .iter()
            .any(|role| role["role"] == "X" && role["min"] == 1 && role["max"] == 1)
    );
    assert!(
        scatter["roles"]
            .as_array()
            .expect("scatter roles")
            .iter()
            .any(|role| role["role"] == "Series")
    );
    assert!(
        scatter["roles"]
            .as_array()
            .expect("scatter roles")
            .iter()
            .all(|role| role["role"] != "Legend")
    );
    let line = visual_contract["visualTypes"]
        .as_array()
        .expect("visual type contracts")
        .iter()
        .find(|visual_type| visual_type["visualType"] == "lineChart")
        .expect("line contract");
    assert!(
        line["roles"]
            .as_array()
            .expect("line roles")
            .iter()
            .any(|role| role["role"] == "Tooltips")
    );

    let archetypes = stdout["desktopProofedArchetypes"]
        .as_array()
        .expect("desktop proofed archetypes");
    assert!(archetypes.iter().any(|item| {
        item["id"] == "flat-ops"
            && item["proofLevel"] == "desktop-golden-pending"
            && item["bindingProofLevel"] == "manual-desktop-canvas-refresh"
    }));
    assert!(archetypes.iter().any(|item| {
        item["id"] == "scatter-bubble"
            && item["proofLevel"] == "desktop-golden-pending"
            && item["bindingProofLevel"] == "manual-desktop-canvas-refresh"
    }));
    assert!(archetypes.iter().any(|item| {
        item["id"] == "catalog-proof"
            && item["proofLevel"] == "desktop-golden-pending"
            && item["bindingProofLevel"] == "manual-desktop-canvas-refresh"
            && item["desktopProof"]
                == "testdata/desktop-proof/canvas-proof.2026-07-10.refresh-session.json"
            && item["status"] == "title-reverification-pending"
    }));
}

#[test]
fn report_spec_capabilities_are_honest_about_shape_only_validation() {
    let (code, stdout, stderr) = run_powerbi(&["capabilities", "--for", "report spec", "--json"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    let command = stdout["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "report spec validate")
        .expect("report spec validate command");
    assert!(
        command["usage"]
            .as_str()
            .expect("usage")
            .contains("[--schema <schema.json>]"),
        "usage should show schema as optional: {command:?}"
    );
    assert!(
        command["validationLevels"]
            .as_array()
            .expect("validation levels")
            .iter()
            .any(|level| level["level"] == "shape-only" && level["ok"].is_null())
    );
}

#[test]
fn scaffold_generates_offline_safe_pbip_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let (code, stdout, stderr) = run_powerbi(&[
        "--json",
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
    ]);
    assert_eq!(code, 0, "stdout: {stdout:?}\nstderr: {stderr}");
    assert_eq!(stdout["ok"], Value::Bool(true));
    assert!(out_dir.join("SalesOperations.pbip").is_file());
    assert!(
        out_dir
            .join("SalesOperations.Report")
            .join("definition")
            .join("pages")
            .join("ReportSectionOverview")
            .join("visuals")
            .exists()
    );
    assert!(
        out_dir
            .join("SalesOperations.SemanticModel")
            .join("definition")
            .join("tables")
            .join("FactSales.tmdl")
            .is_file()
    );
    let report_version: Value = serde_json::from_str(
        &fs::read_to_string(
            out_dir
                .join("SalesOperations.Report")
                .join("definition")
                .join("version.json"),
        )
        .expect("read report version"),
    )
    .expect("parse report version");
    assert_eq!(report_version["version"], Value::from("2.0.0"));
    assert_no_data_cache(&out_dir);

    let (validate_code, validate_stdout, validate_stderr) =
        run_powerbi(&["--json", "validate", out]);
    assert_eq!(
        validate_code, 0,
        "stdout: {validate_stdout:?}\nstderr: {validate_stderr}"
    );
    assert_eq!(validate_stdout["ok"], Value::Bool(true));
    assert_eq!(validate_stdout["counts"]["tables"], Value::from(3));
    assert_eq!(validate_stdout["counts"]["pages"], Value::from(1));
    assert_eq!(validate_stdout["counts"]["visuals"], Value::from(3));

    let visuals_dir = out_dir
        .join("SalesOperations.Report")
        .join("definition")
        .join("pages")
        .join("ReportSectionOverview")
        .join("visuals");
    let first_visual_path = fs::read_dir(&visuals_dir)
        .expect("read generated visuals")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("visual file type").is_dir())
        .map(|entry| entry.path().join("visual.json"))
        .find(|path| path.is_file())
        .expect("first generated visual.json");
    let visual_json: Value = serde_json::from_str(
        &fs::read_to_string(&first_visual_path).expect("read generated visual.json"),
    )
    .expect("parse generated visual.json");
    assert!(
        visual_json.get("objects").is_none(),
        "Power BI Desktop rejects root-level visual container objects in enhanced PBIR"
    );
    assert!(
        visual_json
            .pointer(
                "/visual/visualContainerObjects/general/0/properties/altText/expr/Literal/Value",
            )
            .is_some(),
        "generated alt text should live under /visual/visualContainerObjects/general"
    );

    let (strict_code, strict_stdout, strict_stderr) =
        run_powerbi(&["validate", "--strict", out, "--json"]);
    assert_eq!(
        strict_code, 0,
        "stdout: {strict_stdout:?}\nstderr: {strict_stderr}"
    );
    assert_eq!(strict_stdout["ok"], Value::Bool(true));
    assert_eq!(strict_stdout["strict"], Value::Bool(true));
    assert!(strict_stdout["lint"]["findings"].is_array());

    let (lint_code, lint_stdout, lint_stderr) = run_powerbi(&["lint", out, "--json"]);
    assert_eq!(
        lint_code, 0,
        "stdout: {lint_stdout:?}\nstderr: {lint_stderr}"
    );
    assert_eq!(lint_stdout["schema"], Value::from("powerbi-cli.lint.v1"));
    assert!(
        !lint_stdout["findings"]
            .as_array()
            .expect("lint findings")
            .iter()
            .any(|finding| finding["code"] == "report.visual_unbound")
    );

    let (inspect_code, inspect_stdout, inspect_stderr) = run_powerbi(&["--json", "inspect", out]);
    assert_eq!(
        inspect_code, 0,
        "stdout: {inspect_stdout:?}\nstderr: {inspect_stderr}"
    );
    assert_eq!(inspect_stdout["valid"], Value::Bool(true));

    let (deep_code, deep_stdout, deep_stderr) = run_powerbi(&["inspect", "--deep", out, "--json"]);
    assert_eq!(
        deep_code, 0,
        "stdout: {deep_stdout:?}\nstderr: {deep_stderr}"
    );
    assert_eq!(deep_stdout["valid"], Value::Bool(true));
    assert!(
        deep_stdout["deep"]["handles"]
            .as_array()
            .expect("deep handles")
            .iter()
            .any(|handle| handle["handle"] == "table:FactSales")
    );
    assert!(
        deep_stdout["deep"]["model"]["tables"]
            .as_array()
            .expect("deep tables")
            .iter()
            .flat_map(|table| table["measures"].as_array().into_iter().flatten())
            .any(|measure| measure["expression"] == "SUM('FactSales'[Revenue])")
    );
    assert!(
        deep_stdout["deep"]["report"]["pages"]
            .as_array()
            .expect("deep pages")
            .iter()
            .any(|page| page["handle"] == "page:ReportSectionOverview")
    );

    let (wireframe_code, wireframe_stdout, wireframe_stderr) =
        run_powerbi(&["report", "wireframe", "export", out, "--json"]);
    assert_eq!(
        wireframe_code, 0,
        "stdout: {wireframe_stdout:?}\nstderr: {wireframe_stderr}"
    );
    assert_eq!(
        wireframe_stdout["schema"],
        Value::from("powerbi-cli.report.wireframe.v1")
    );
    assert_eq!(wireframe_stdout["counts"]["visuals"], Value::from(3));
    assert!(
        wireframe_stdout["handles"]
            .as_array()
            .expect("wireframe handles")
            .iter()
            .any(|handle| handle["kind"] == "visual")
    );
    assert!(
        wireframe_stdout["pages"][0]["visuals"]
            .as_array()
            .expect("wireframe visuals")[0]["position"]
            .is_object()
    );
}

#[test]
fn scaffold_force_removes_only_artifacts_from_the_prior_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let out = out_dir.to_str().expect("output path");
    let (first_code, _, first_stderr) = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out,
        "--json",
    ]);
    assert_eq!(first_code, 0, "stderr: {first_stderr}");

    let tables_dir = out_dir
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables");
    let stale_table = tables_dir.join("DimDate.tmdl");
    assert!(stale_table.is_file());
    let user_file = tables_dir.join("user-notes.txt");
    fs::write(&user_file, "preserve this user-added file").expect("write user file");

    let mut reduced_schema: Value =
        serde_json::from_str(include_str!("../examples/sales.schema.json"))
            .expect("parse sales schema");
    reduced_schema["tables"]
        .as_array_mut()
        .expect("tables")
        .retain(|table| table["name"] != "DimDate");
    reduced_schema["relationships"]
        .as_array_mut()
        .expect("relationships")
        .retain(|relationship| {
            relationship["fromTable"] != "DimDate" && relationship["toTable"] != "DimDate"
        });
    reduced_schema["pages"][0]["visuals"]
        .as_array_mut()
        .expect("visuals")
        .retain(|visual| {
            !visual["bindings"].as_array().is_some_and(|bindings| {
                bindings.iter().any(|binding| binding["table"] == "DimDate")
            })
        });
    let reduced_path = temp.path().join("sales-reduced.schema.json");
    fs::write(
        &reduced_path,
        serde_json::to_string_pretty(&reduced_schema).expect("serialize reduced schema"),
    )
    .expect("write reduced schema");

    let (force_code, force_stdout, force_stderr) = run_powerbi(&[
        "scaffold",
        "--schema",
        reduced_path.to_str().expect("schema path"),
        "--out-dir",
        out,
        "--force",
        "--json",
    ]);
    assert_eq!(
        force_code, 0,
        "stdout: {force_stdout:?}\nstderr: {force_stderr}"
    );
    assert!(
        !stale_table.exists(),
        "removed table must not survive --force rebuild"
    );
    assert!(
        user_file.is_file(),
        "--force must preserve user-added files"
    );

    let (validate_code, validate_stdout, validate_stderr) =
        run_powerbi(&["validate", "--strict", out, "--json"]);
    assert_eq!(
        validate_code, 0,
        "stdout: {validate_stdout:?}\nstderr: {validate_stderr}"
    );
    assert_eq!(validate_stdout["counts"]["tables"], Value::from(2));
}

#[test]
fn root_relative_pbip_selects_only_its_referenced_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sales.schema.json");
    let (scaffold_code, scaffold_stdout, scaffold_stderr) = run_powerbi(&[
        "scaffold",
        "--schema",
        schema.to_str().expect("schema path"),
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(
        scaffold_code, 0,
        "stdout: {scaffold_stdout:?}\nstderr: {scaffold_stderr}"
    );

    let unrelated = out_dir.join("nested-unrelated-project");
    fs::create_dir_all(&unrelated).expect("create unrelated nested project");
    fs::write(unrelated.join("invalid.json"), "{ definitely not JSON")
        .expect("write unrelated invalid JSON");

    let (code, stdout, stderr) = run_powerbi_in(
        &["validate", "--strict", "SalesOperations.pbip", "--json"],
        Some(&out_dir),
    );
    assert_eq!(code, 0, "stdout: {stdout:?}\nstderr: {stderr}");
    assert_eq!(stdout["ok"], Value::Bool(true));
}

#[test]
fn pbip_artifact_reference_cannot_escape_selected_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sales.schema.json");
    let (scaffold_code, _, scaffold_stderr) = run_powerbi(&[
        "scaffold",
        "--schema",
        schema.to_str().expect("schema path"),
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(scaffold_code, 0, "stderr: {scaffold_stderr}");

    let pbip_path = out_dir.join("SalesOperations.pbip");
    let mut pbip: Value =
        serde_json::from_str(&fs::read_to_string(&pbip_path).expect("read generated PBIP"))
            .expect("parse generated PBIP");
    pbip["artifacts"][0]["report"]["path"] = Value::String("../outside.Report".to_string());
    fs::write(
        &pbip_path,
        serde_json::to_string_pretty(&pbip).expect("serialize escaped PBIP"),
    )
    .expect("write escaped PBIP");

    let (code, stdout, stderr) =
        run_powerbi(&["validate", pbip_path.to_str().expect("PBIP path"), "--json"]);
    assert_eq!(code, 10, "stdout: {stdout:?}\nstderr: {stderr}");
    let error: Value = serde_json::from_str(stderr.trim()).expect("error envelope");
    assert_eq!(error["error"]["code"], "validation_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("escapes the selected PBIP project")
    );
}

#[cfg(unix)]
#[test]
fn linked_report_artifact_cannot_escape_selected_project() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("sales_project");
    let schema = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sales.schema.json");
    let (scaffold_code, _, scaffold_stderr) = run_powerbi(&[
        "scaffold",
        "--schema",
        schema.to_str().expect("schema path"),
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(scaffold_code, 0, "stderr: {scaffold_stderr}");

    let outside = temp.path().join("outside.Report");
    fs::create_dir_all(&outside).expect("create outside report");
    symlink(&outside, out_dir.join("linked.Report")).expect("link outside report");
    let pbip_path = out_dir.join("SalesOperations.pbip");
    let mut pbip: Value =
        serde_json::from_str(&fs::read_to_string(&pbip_path).expect("read generated PBIP"))
            .expect("parse generated PBIP");
    pbip["artifacts"][0]["report"]["path"] = Value::String("linked.Report".to_string());
    fs::write(
        &pbip_path,
        serde_json::to_string_pretty(&pbip).expect("serialize linked PBIP"),
    )
    .expect("write linked PBIP");

    let (code, _, stderr) =
        run_powerbi(&["validate", pbip_path.to_str().expect("PBIP path"), "--json"]);
    assert_eq!(code, 10, "stderr: {stderr}");
    assert!(stderr.contains("escapes the selected PBIP project"));
}

#[test]
fn scaffold_force_refuses_unmarked_nonempty_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("user_directory");
    fs::create_dir_all(&out_dir).expect("create user directory");
    let user_file = out_dir.join("keep.txt");
    fs::write(&user_file, "keep").expect("write user file");

    let (code, _, stderr) = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--force",
        "--json",
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("refusing --force cleanup in unmarked non-empty directory"));
    assert!(
        user_file.is_file(),
        "refused cleanup must preserve user data"
    );
}

#[test]
fn scaffold_without_declared_pages_creates_one_empty_official_safe_page() {
    let temp = tempfile::tempdir().expect("tempdir");
    let schema_path = temp.path().join("empty-pages.schema.json");
    fs::write(
        &schema_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "EmptyPageProject",
            "tables": [{
                "name": "Facts",
                "columns": [{"name": "Value", "dataType": "int64"}],
                "rows": [{"Value": 1}]
            }],
            "pages": []
        }))
        .expect("schema JSON"),
    )
    .expect("write schema");
    let project = temp.path().join("empty-page-project");
    let (scaffold_code, scaffold_json, scaffold_stderr) = run_powerbi(&[
        "scaffold",
        "--schema",
        schema_path.to_str().expect("schema path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(
        scaffold_code, 0,
        "stdout: {scaffold_json:?}\nstderr: {scaffold_stderr}"
    );

    let (inspect_code, inspect_json, inspect_stderr) = run_powerbi(&[
        "inspect",
        "--deep",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(
        inspect_code, 0,
        "stdout: {inspect_json:?}\nstderr: {inspect_stderr}"
    );
    assert_eq!(inspect_json["counts"]["pages"], Value::from(1));
    assert_eq!(inspect_json["counts"]["visuals"], Value::from(0));

    let (validate_code, validate_json, validate_stderr) = run_powerbi(&[
        "validate",
        "--strict",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(
        validate_code, 0,
        "stdout: {validate_json:?}\nstderr: {validate_stderr}"
    );
}

fn assert_no_data_cache(root: &Path) {
    let forbidden = [".pbix", ".pbit", "cache.abf", "localSettings.json"];
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let name = entry.path().to_string_lossy();
        assert!(
            !forbidden.iter().any(|suffix| name.ends_with(suffix)),
            "generated offline-unsafe file: {}",
            entry.path().display()
        );
    }
    let gitignore = fs::read_to_string(root.join(".gitignore")).expect("gitignore");
    assert!(gitignore.contains("cache.abf"));
    assert!(gitignore.contains("localSettings.json"));
}
