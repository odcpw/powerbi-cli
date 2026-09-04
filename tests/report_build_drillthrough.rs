//! Dashboard-spec drillthrough compilation and CLI-kernel parity tests.

mod common;

use common::{RunOutput, run_powerbi_owned, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

fn path_arg(path: &Path) -> String {
    path.to_str().expect("test path is UTF-8").to_string()
}

fn read_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.expect("walk artifact tree").into_path())
        .filter(|path| path.is_file())
        .map(|path| {
            let relative = path
                .strip_prefix(root)
                .expect("relative artifact path")
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            (relative, fs::read(path).expect("read artifact file"))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn write_spec(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize spec");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write spec");
}

fn build_args(spec: &Path, out_dir: &Path) -> Vec<String> {
    vec![
        "report".into(),
        "build".into(),
        "--schema".into(),
        "examples/archetypes/regional-sales.schema.json".into(),
        "--profile".into(),
        "examples/archetypes/regional-sales.profile.json".into(),
        "--spec".into(),
        path_arg(spec),
        "--out-dir".into(),
        path_arg(out_dir),
        "--json".into(),
    ]
}

fn regional_fixture() -> Value {
    serde_json::from_str(
        &fs::read_to_string("examples/archetypes/regional-sales.dashboard.json")
            .expect("regional fixture"),
    )
    .expect("regional fixture JSON")
}

fn build_fixture(spec: &Path, out_dir: &Path) -> RunOutput {
    run_powerbi_owned(&build_args(spec, out_dir))
}

fn page_json(project: &Path, page: &str) -> Value {
    let path = project
        .join("RegionalSales.Report")
        .join("definition")
        .join("pages")
        .join(page)
        .join("page.json");
    serde_json::from_str(&fs::read_to_string(path).expect("page JSON")).expect("page JSON value")
}

#[test]
fn dashboard_v2_drillthrough_compiles_hidden_pages_and_pending_back_button_warning() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = Path::new("examples/archetypes/regional-sales.dashboard.json");
    let first_dir = temp.path().join("first");
    let first = build_fixture(fixture, &first_dir);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let first_json = stdout_json(&first);
    assert_eq!(
        first_json["operationOutcomes"]
            .as_array()
            .expect("operation outcomes")
            .iter()
            .filter(|outcome| {
                outcome["changes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|change| change["jsonPointer"] == "/pageBinding")
            })
            .count(),
        2
    );
    let warnings = first_json["warnings"].as_array().expect("warnings");
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| warning["code"] == "spec.feature_pending")
            .count(),
        2
    );
    assert!(warnings.iter().all(|warning| {
        warning["owningBead"] == "pbi-t4-pbir-catalog-expansion-sn2.8"
            && warning["pointer"]
                .as_str()
                .is_some_and(|pointer| pointer.ends_with("/backButton"))
    }));

    for (page, column) in [
        ("ReportSectionCustomerDetail", "Customer"),
        ("ReportSectionSegmentDetail", "Segment"),
    ] {
        let page = page_json(&first_dir, page);
        assert_eq!(page["type"], "Drillthrough");
        assert_eq!(page["visibility"], "HiddenInViewMode");
        assert_eq!(
            page["pageBinding"]["parameters"][0]["fieldExpr"]["Column"]["Property"],
            column
        );
        assert_eq!(
            page["pageBinding"]["parameters"][0]["fieldExpr"]["Column"]["Expression"]["SourceRef"]
                ["Entity"],
            "DimCustomer"
        );
        assert!(
            !page["visuals"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|visual| visual["visualType"] == "actionButton")
        );
    }

    let second_dir = temp.path().join("second");
    let second = build_fixture(fixture, &second_dir);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(read_tree(&first_dir), read_tree(&second_dir));
}

#[test]
fn dashboard_v2_drillthrough_compilation_matches_the_two_cli_set_kernels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let full = regional_fixture();
    let full_spec = temp.path().join("full.json");
    write_spec(&full_spec, &full);
    let mut base = full;
    for page in base["pages"].as_array_mut().expect("pages") {
        page.as_object_mut()
            .expect("page object")
            .remove("drillthrough");
    }
    let base_spec = temp.path().join("base.json");
    write_spec(&base_spec, &base);

    let compiler_dir = temp.path().join("compiler");
    let compiler = build_fixture(&full_spec, &compiler_dir);
    assert_eq!(compiler.code, 0, "stderr: {}", compiler.stderr);

    let base_dir = temp.path().join("base");
    let base_build = build_fixture(&base_spec, &base_dir);
    assert_eq!(base_build.code, 0, "stderr: {}", base_build.stderr);
    let customer_dir = temp.path().join("customer");
    let customer = run_powerbi_owned(&[
        "report".into(),
        "drillthrough".into(),
        "set".into(),
        "--project".into(),
        path_arg(&base_dir),
        "--page".into(),
        "page:ReportSectionCustomerDetail".into(),
        "--target".into(),
        "DimCustomer[Customer]".into(),
        "--out-dir".into(),
        path_arg(&customer_dir),
        "--json".into(),
    ]);
    assert_eq!(customer.code, 0, "stderr: {}", customer.stderr);
    let cli_dir = temp.path().join("cli");
    let segment = run_powerbi_owned(&[
        "report".into(),
        "drillthrough".into(),
        "set".into(),
        "--project".into(),
        path_arg(&customer_dir),
        "--page".into(),
        "page:ReportSectionSegmentDetail".into(),
        "--target".into(),
        "DimCustomer[Segment]".into(),
        "--out-dir".into(),
        path_arg(&cli_dir),
        "--json".into(),
    ]);
    assert_eq!(segment.code, 0, "stderr: {}", segment.stderr);
    assert_eq!(read_tree(&compiler_dir), read_tree(&cli_dir));
}

#[test]
fn dashboard_v2_drillthrough_target_must_be_an_existing_column_with_a_pointer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut fixture = regional_fixture();
    fixture["pages"][1]["drillthrough"]["target"] = json!("FactSales[Total Revenue]");
    let spec = temp.path().join("invalid.json");
    write_spec(&spec, &fixture);
    let output = run_powerbi_owned(&[
        "report".into(),
        "build".into(),
        "--schema".into(),
        "examples/archetypes/regional-sales.schema.json".into(),
        "--profile".into(),
        "examples/archetypes/regional-sales.profile.json".into(),
        "--spec".into(),
        path_arg(&spec),
        "--dry-run".into(),
        "--json".into(),
    ]);
    assert_eq!(output.code, 2, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "invalid_args");
    assert_eq!(error["pointer"], "/pages/1/drillthrough/target");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("filter target does not exist in schema")
    );
}

#[test]
fn dashboard_v2_drillthrough_unknown_key_is_rejected_before_output_creation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut fixture = regional_fixture();
    fixture["pages"][1]["drillthrough"]["back_button"] = json!(true);
    let spec = temp.path().join("unknown.json");
    write_spec(&spec, &fixture);
    let out = temp.path().join("unknown-output");
    let output = run_powerbi_owned(&[
        "report".into(),
        "build".into(),
        "--schema".into(),
        "examples/archetypes/regional-sales.schema.json".into(),
        "--spec".into(),
        path_arg(&spec),
        "--out-dir".into(),
        path_arg(&out),
        "--json".into(),
    ]);
    assert_eq!(output.code, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "spec.unknown_field");
    assert_eq!(error["pointer"], "/pages/1/drillthrough/back_button");
    assert!(!out.exists());
}

#[test]
fn dashboard_v2_drillthrough_defaults_hidden_when_hidden_is_omitted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut fixture = regional_fixture();
    fixture["pages"][1]["drillthrough"]
        .as_object_mut()
        .expect("drillthrough object")
        .remove("hidden");
    fixture["pages"][1]["drillthrough"]
        .as_object_mut()
        .expect("drillthrough object")
        .remove("backButton");
    let spec = temp.path().join("default-hidden.json");
    write_spec(&spec, &fixture);
    let out = temp.path().join("default-hidden");
    let output = build_fixture(&spec, &out);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert_eq!(
        page_json(&out, "ReportSectionSegmentDetail")["visibility"],
        "HiddenInViewMode"
    );
}
