//! Dashboard-spec filter compilation and CLI-kernel parity tests.

mod common;

use common::{RunOutput, run_powerbi_owned, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
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

fn build_args(spec: &Path, out_dir: &Path) -> Vec<String> {
    vec![
        "report".into(),
        "build".into(),
        "--schema".into(),
        "examples/sales.schema.json".into(),
        "--spec".into(),
        path_arg(spec),
        "--out-dir".into(),
        path_arg(out_dir),
        "--json".into(),
    ]
}

fn write_spec(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize spec");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write spec");
}

fn build_filter_fixture(root: &Path, spec: &Path, name: &str) -> (RunOutput, PathBuf) {
    let out_dir = root.join(name);
    let output = run_powerbi_owned(&build_args(spec, &out_dir));
    (output, out_dir)
}

#[test]
fn dashboard_v2_filter_fixture_compiles_every_supported_kind_and_is_deterministic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = Path::new("examples/filter-kinds.dashboard.v2.json");

    let (first, first_dir) = build_filter_fixture(temp.path(), fixture, "first");
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let first_json = stdout_json(&first);
    assert_eq!(first_json["ok"], Value::Bool(true));
    assert_eq!(
        first_json["operationOutcomes"].as_array().map(Vec::len),
        Some(4)
    );
    assert_eq!(first_json["changes"].as_array().map(Vec::len), Some(5));

    let operations = first_json["operations"].as_array().expect("operations");
    assert_eq!(
        operations
            .iter()
            .filter(|operation| operation["op"] == "addFilter")
            .count(),
        4
    );
    assert_eq!(operations[1]["filterType"], "Categorical");
    assert_eq!(operations[2]["filterType"], "Advanced");
    assert_eq!(operations[3]["filterType"], "RelativeDate");
    assert_eq!(operations[4]["filterType"], "TopN");
    let first_path = path_arg(&first_dir);
    assert!(
        first_json["operationOutcomes"]
            .as_array()
            .expect("outcomes")
            .iter()
            .flat_map(|outcome| outcome["readback"].as_array().into_iter().flatten())
            .all(|command| command
                .as_str()
                .is_some_and(|command| command.contains(&first_path)))
    );

    let report_path = first_dir
        .join("FilterKinds.Report")
        .join("definition")
        .join("report.json");
    let page_path = first_dir
        .join("FilterKinds.Report")
        .join("definition")
        .join("pages")
        .join("ReportSectionOverview")
        .join("page.json");
    let visual_path = first_dir
        .join("FilterKinds.Report")
        .join("definition")
        .join("pages")
        .join("ReportSectionOverview")
        .join("visuals")
        .join("VisualContainerCustomerDetail")
        .join("visual.json");
    let report: Value = serde_json::from_str(&fs::read_to_string(report_path).expect("report"))
        .expect("report JSON");
    let page: Value =
        serde_json::from_str(&fs::read_to_string(page_path).expect("page")).expect("page JSON");
    let visual: Value = serde_json::from_str(&fs::read_to_string(visual_path).expect("visual"))
        .expect("visual JSON");
    assert_eq!(
        report["filterConfig"]["filters"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        page["filterConfig"]["filters"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        visual["filterConfig"]["filters"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(report["filterConfig"]["filters"][0]["type"], "Categorical");
    assert_eq!(page["filterConfig"]["filters"][0]["type"], "Advanced");
    assert_eq!(page["filterConfig"]["filters"][1]["type"], "RelativeDate");
    assert_eq!(visual["filterConfig"]["filters"][0]["type"], "TopN");

    let second_dir = temp.path().join("second");
    let second = run_powerbi_owned(&build_args(fixture, &second_dir));
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(read_tree(&first_dir), read_tree(&second_dir));
}

#[test]
fn dashboard_v2_filter_compilation_matches_the_four_cli_add_filter_kernels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string("examples/filter-kinds.dashboard.v2.json").expect("fixture"),
    )
    .expect("fixture JSON");
    let filtered_spec = temp.path().join("filtered.json");
    write_spec(&filtered_spec, &fixture);

    let mut base = fixture.clone();
    base.as_object_mut()
        .expect("fixture object")
        .remove("filters");
    let pages = base["pages"].as_array_mut().expect("pages");
    let page = pages[0].as_object_mut().expect("page");
    page.remove("filters");
    page["visuals"][0]
        .as_object_mut()
        .expect("visual")
        .remove("filters");
    let base_spec = temp.path().join("base.json");
    write_spec(&base_spec, &base);

    let compiler_dir = temp.path().join("compiler");
    let compiler = run_powerbi_owned(&build_args(&filtered_spec, &compiler_dir));
    assert_eq!(compiler.code, 0, "stderr: {}", compiler.stderr);

    let mut current = temp.path().join("base");
    let base_build = run_powerbi_owned(&build_args(&base_spec, &current));
    assert_eq!(base_build.code, 0, "stderr: {}", base_build.stderr);
    let commands = [
        vec![
            "report",
            "filters",
            "add",
            "--project",
            "",
            "--scope",
            "report",
            "--target",
            "DimCustomer[Segment]",
            "--value",
            "Enterprise",
            "--display-name",
            "Segment",
            "--out-dir",
            "",
            "--json",
        ],
        vec![
            "report",
            "filters",
            "add",
            "--project",
            "",
            "--scope",
            "page",
            "--page",
            "page:ReportSectionOverview",
            "--target",
            "FactSales[Units]",
            "--min",
            "1",
            "--max",
            "50",
            "--out-dir",
            "",
            "--json",
        ],
        vec![
            "report",
            "filters",
            "add",
            "--project",
            "",
            "--scope",
            "page",
            "--page",
            "page:ReportSectionOverview",
            "--target",
            "DimDate[Date]",
            "--relative",
            "last",
            "--unit",
            "months",
            "--span",
            "12",
            "--out-dir",
            "",
            "--json",
        ],
        vec![
            "report",
            "filters",
            "add",
            "--project",
            "",
            "--scope",
            "visual",
            "--visual",
            "visual:ReportSectionOverview:VisualContainerCustomerDetail",
            "--target",
            "DimCustomer[CustomerName]",
            "--top",
            "10",
            "--by",
            "FactSales[Total Revenue]",
            "--out-dir",
            "",
            "--json",
        ],
    ];
    for command in commands {
        let next = current.with_file_name(format!(
            "{}-next",
            current
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project")
        ));
        let mut args = command.into_iter().map(String::from).collect::<Vec<_>>();
        args[4] = path_arg(&current);
        let out_index = args
            .iter()
            .position(|arg| arg == "--out-dir")
            .expect("out flag")
            + 1;
        args[out_index] = path_arg(&next);
        let output = run_powerbi_owned(&args);
        assert_eq!(output.code, 0, "stderr: {}", output.stderr);
        current = next;
    }
    assert_eq!(read_tree(&compiler_dir), read_tree(&current));
}

#[test]
fn dashboard_v2_range_filter_on_text_column_reports_the_filter_pointer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut fixture: Value = serde_json::from_str(
        &fs::read_to_string("examples/filter-kinds.dashboard.v2.json").expect("fixture"),
    )
    .expect("fixture JSON");
    fixture["pages"][0]["filters"][0]["target"] = json!("DimCustomer[Segment]");
    let spec = temp.path().join("invalid-range.json");
    write_spec(&spec, &fixture);
    let output = run_powerbi_owned(&[
        "report".into(),
        "build".into(),
        "--schema".into(),
        "examples/sales.schema.json".into(),
        "--spec".into(),
        path_arg(&spec),
        "--dry-run".into(),
        "--json".into(),
    ]);
    assert_eq!(output.code, 2, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "invalid_args");
    assert_eq!(error["pointer"], "/pages/0/filters/0");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("must have a numeric TMDL dataType")
    );
}

#[test]
fn dashboard_v2_filter_unknown_key_is_rejected_before_output_creation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut fixture: Value = serde_json::from_str(
        &fs::read_to_string("examples/filter-kinds.dashboard.v2.json").expect("fixture"),
    )
    .expect("fixture JSON");
    fixture["filters"][0]["valuesExtra"] = json!(["SMB"]);
    let spec = temp.path().join("unknown-filter-key.json");
    let out = temp.path().join("must-not-exist");
    write_spec(&spec, &fixture);
    let output = run_powerbi_owned(&[
        "report".into(),
        "build".into(),
        "--schema".into(),
        "examples/sales.schema.json".into(),
        "--spec".into(),
        path_arg(&spec),
        "--out-dir".into(),
        path_arg(&out),
        "--json".into(),
    ]);
    assert_eq!(output.code, 10, "stdout: {}", output.stdout);
    let error = stderr_json(&output)["error"].clone();
    assert_eq!(error["code"], "spec.unknown_field");
    assert_eq!(error["pointer"], "/filters/0/valuesExtra");
    assert!(!out.exists(), "unknown filter key must not create output");
}
