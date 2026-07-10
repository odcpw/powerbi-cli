use serde_json::{Value, json};
use std::fs;
use std::io::Write;
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

fn stdout_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stdout.trim()).expect("stdout JSON")
}

fn stderr_json(output: &RunOutput) -> Value {
    serde_json::from_str(output.stderr.trim()).expect("stderr JSON")
}

fn scaffold_sales(root: &Path) -> PathBuf {
    let out_dir = root.join("sales_project");
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        "examples/sales.schema.json",
        "--out-dir",
        out_dir.to_str().expect("output path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    out_dir
}

fn report_dir(project: &Path) -> PathBuf {
    project.join("SalesOperations.Report")
}

fn semantic_model_dir(project: &Path) -> PathBuf {
    project.join("SalesOperations.SemanticModel")
}

fn pages_json(project: &Path) -> PathBuf {
    report_dir(project)
        .join("definition")
        .join("pages")
        .join("pages.json")
}

fn first_page_name(project: &Path) -> String {
    let pages: Value =
        serde_json::from_str(&fs::read_to_string(pages_json(project)).expect("pages json"))
            .expect("parse pages");
    pages["pageOrder"][0]
        .as_str()
        .expect("first page")
        .to_string()
}

fn first_visual_json(project: &Path) -> PathBuf {
    let visuals_dir = report_dir(project)
        .join("definition")
        .join("pages")
        .join(first_page_name(project))
        .join("visuals");
    fs::read_dir(visuals_dir)
        .expect("visuals dir")
        .filter_map(Result::ok)
        .find(|entry| entry.file_type().expect("file type").is_dir())
        .expect("first visual")
        .path()
        .join("visual.json")
}

fn patch_json(path: &Path, patch: impl FnOnce(&mut Value)) {
    let mut value: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("json")).expect("parse json");
    patch(&mut value);
    fs::write(
        path,
        serde_json::to_string_pretty(&value).expect("json text"),
    )
    .expect("write json");
}

fn first_visual_handle(project_arg: &str) -> String {
    let output = run_powerbi(&[
        "report",
        "visuals",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    stdout_json(&output)["visuals"][0]["handle"]
        .as_str()
        .expect("visual handle")
        .to_string()
}

fn install_conditional_formatting_fixture(project: &Path) {
    patch_json(&first_visual_json(project), |visual| {
        visual["visual"]["objects"]["dataPoint"] = json!([{
            "properties": {
                "fill": { "solid": { "color": "#4472C4" } },
                "conditionalFormatting": {
                    "rules": [{
                        "condition": { "min": 0, "max": 1000 },
                        "color": "#70AD47"
                    }],
                    "gradient": {
                        "min": "#F4B183",
                        "max": "#70AD47"
                    }
                }
            }
        }]);
    });
}

fn install_flat_bookmarks(project: &Path) {
    let bookmarks_dir = report_dir(project).join("definition").join("bookmarks");
    fs::create_dir_all(&bookmarks_dir).expect("bookmarks dir");
    fs::write(
        bookmarks_dir.join("bookmarks.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmarksMetadata/1.0.0/schema.json",
            "items": [
                { "name": "BookmarkA" },
                { "name": "BookmarkB" }
            ]
        }))
        .expect("metadata json"),
    )
    .expect("write metadata");
    for (name, display_name) in [("BookmarkA", "First View"), ("BookmarkB", "Second View")] {
        fs::write(
            bookmarks_dir.join(format!("{name}.bookmark.json")),
            serde_json::to_string_pretty(&json!({
                "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/bookmark/2.1.0/schema.json",
                "displayName": display_name,
                "name": name,
                "options": {},
                "explorationState": {
                    "version": "1.3",
                    "activeSection": first_page_name(project),
                    "sections": {}
                }
            }))
            .expect("bookmark json"),
        )
        .expect("write bookmark");
    }
}

fn write_test_package(path: &Path) {
    let file = fs::File::create(path).expect("create package");
    let mut zip = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in [
        ("Sample.pbip", "{}"),
        ("Sample.Report/definition/report.json", "{}"),
        (
            "Sample.SemanticModel/definition/tables/Fact.tmdl",
            "table Fact\n",
        ),
        (
            "Sample.Report/StaticResources/RegisteredResources/Theme.json",
            "{}",
        ),
        ("DataModel", "opaque"),
    ] {
        zip.start_file(name, options).expect("start zip file");
        zip.write_all(body.as_bytes()).expect("write zip file");
    }
    zip.finish().expect("finish package");
}

fn write_package_bytes(
    path: &Path,
    compression: zip::CompressionMethod,
    entries: &[(&str, Vec<u8>)],
) {
    let file = fs::File::create(path).expect("create package");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(compression);
    for (name, body) in entries {
        zip.start_file(*name, options).expect("start zip file");
        zip.write_all(body).expect("write zip file");
    }
    zip.finish().expect("finish package");
}

#[test]
fn package_inspect_and_extract_are_metadata_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let package = temp.path().join("sample.pbit");
    write_test_package(&package);
    let package_arg = package.to_str().expect("package path");

    let inspect = run_powerbi(&["package", "inspect", package_arg, "--json"]);
    assert_eq!(inspect.code, 0, "stderr: {}", inspect.stderr);
    let inspect_json = stdout_json(&inspect);
    assert_eq!(
        inspect_json["schema"],
        Value::from("powerbi-cli.package.inspect.v1")
    );
    assert_eq!(
        inspect_json["support"]["canExportPbixOrPbit"],
        Value::Bool(false)
    );
    assert_eq!(
        inspect_json["support"]["canImportPbipSource"],
        Value::Bool(true)
    );

    let out_dir = temp.path().join("extracted");
    let extract = run_powerbi(&[
        "package",
        "extract",
        package_arg,
        "--out-dir",
        out_dir.to_str().expect("out dir"),
        "--json",
    ]);
    assert_eq!(extract.code, 0, "stderr: {}", extract.stderr);
    assert!(out_dir.join("Sample.pbip").is_file());
    assert!(
        out_dir
            .join("Sample.Report/definition/report.json")
            .is_file()
    );
    assert!(!out_dir.join("DataModel").exists());
    assert_eq!(stdout_json(&extract)["counts"]["skipped"], Value::from(1));
}

#[test]
fn package_source_pack_import_round_trips_scaffolded_source_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let package = temp.path().join("sales-source.pbit");
    let package_arg = package.to_str().expect("package path");
    let imported = temp.path().join("imported_sales");
    let imported_arg = imported.to_str().expect("imported path");

    let source_pack = run_powerbi(&[
        "package",
        "source-pack",
        "--project",
        project_arg,
        "--out",
        package_arg,
        "--json",
    ]);
    assert_eq!(source_pack.code, 0, "stderr: {}", source_pack.stderr);
    let source_pack_json = stdout_json(&source_pack);
    assert_eq!(
        source_pack_json["schema"],
        Value::from("powerbi-cli.package.sourcePack.v1")
    );
    assert_eq!(
        source_pack_json["packageClass"],
        Value::from("source-package")
    );
    assert_eq!(
        source_pack_json["desktopBinaryCompatible"],
        Value::Bool(false)
    );
    assert!(package.is_file());

    let inspect = run_powerbi(&["package", "inspect", package_arg, "--json"]);
    assert_eq!(inspect.code, 0, "stderr: {}", inspect.stderr);
    let inspect_json = stdout_json(&inspect);
    assert_eq!(inspect_json["packageClass"], Value::from("source-package"));
    assert_eq!(
        inspect_json["support"]["canImportSourceProject"],
        Value::Bool(true)
    );
    assert_eq!(
        inspect_json["archive"]["hasUnsafeDataModel"],
        Value::Bool(false)
    );

    let import = run_powerbi(&[
        "package",
        "import",
        package_arg,
        "--out-dir",
        imported_arg,
        "--json",
    ]);
    assert_eq!(import.code, 0, "stderr: {}", import.stderr);
    let import_json = stdout_json(&import);
    assert_eq!(
        import_json["schema"],
        Value::from("powerbi-cli.package.import.v1")
    );
    assert_eq!(import_json["sourceRoot"], Value::Null);
    assert_eq!(import_json["validation"]["ok"], Value::Bool(true));

    let validate = run_powerbi(&["validate", "--strict", imported_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));

    let handoff = run_powerbi(&["handoff", "check", imported_arg, "--json"]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(
        stdout_json(&handoff)["safeForOfflineHandoff"],
        Value::Bool(true)
    );
}

#[test]
fn package_source_pack_refuses_data_bearing_project_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    fs::write(project.join("DataModel"), "opaque model cache").expect("write datamodel");
    let package = temp.path().join("unsafe-source.pbit");

    let source_pack = run_powerbi(&[
        "package",
        "source-pack",
        "--project",
        project.to_str().expect("project path"),
        "--out",
        package.to_str().expect("package path"),
        "--json",
    ]);
    assert_eq!(source_pack.code, 10);
    assert!(!package.exists());
    let value = stderr_json(&source_pack);
    assert_eq!(value["error"]["code"], Value::from("validation_failed"));
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("DataModel")
    );
}

#[test]
fn package_source_pack_refuses_unknown_and_dot_directory_files_exactly() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    fs::create_dir_all(project.join(".git")).expect("git dir");
    fs::write(project.join(".git").join("config"), "[remote]\n").expect("git config");
    fs::write(project.join(".env"), "SAFE_LOOKING=value\n").expect("env");
    fs::write(project.join("datastructure.txt.txt"), "notes\n").expect("stray text");
    fs::write(project.join("stray.csv"), "name,value\nexample,1\n").expect("csv");
    fs::create_dir_all(project.join("Other.Report").join("definition"))
        .expect("unrelated report dir");
    fs::write(
        project
            .join("Other.Report")
            .join("definition")
            .join("report.json"),
        "{}",
    )
    .expect("unrelated report json");
    fs::write(
        project
            .join("SalesOperations.Report")
            .join("definition")
            .join("stray.json"),
        "{}",
    )
    .expect("stray report json");
    let package = temp.path().join("salted-source.pbit");

    let source_pack = run_powerbi(&[
        "package",
        "source-pack",
        "--project",
        project.to_str().expect("project path"),
        "--out",
        package.to_str().expect("package path"),
        "--json",
    ]);
    assert_eq!(source_pack.code, 10);
    assert!(!package.exists());
    let value = stderr_json(&source_pack);
    assert_eq!(value["error"]["code"], "validation_failed");
    assert_eq!(
        value["error"]["message"],
        "project contains unapproved source-package files: .env, .git/config, Other.Report/definition/report.json, SalesOperations.Report/definition/stray.json, datastructure.txt.txt, stray.csv"
    );
}

#[test]
fn package_source_pack_scans_approved_content_before_creating_archive() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let package = temp.path().join("credential-source.pbit");
    fs::write(
        project.join("POWERBI_HANDOFF.md"),
        "temporary connection Password=hunter2\n",
    )
    .expect("credential sidecar");

    let credential_scan = run_powerbi(&[
        "package",
        "source-pack",
        "--project",
        project.to_str().expect("project path"),
        "--out",
        package.to_str().expect("package path"),
        "--json",
    ]);
    assert_eq!(credential_scan.code, 10);
    assert!(!package.exists());
    assert_eq!(
        stderr_json(&credential_scan)["error"]["message"],
        "source package content scan failed: credential-like content in POWERBI_HANDOFF.md"
    );

    let handoff = fs::read_to_string(project.join("POWERBI_HANDOFF.md")).expect("handoff");
    fs::write(
        project.join("POWERBI_HANDOFF.md"),
        handoff.replace("temporary connection Password=hunter2", "offline handoff"),
    )
    .expect("safe sidecar");
    let customer_tmdl = project
        .join("SalesOperations.SemanticModel")
        .join("definition")
        .join("tables")
        .join("DimCustomer.tmdl");
    let customer = fs::read_to_string(&customer_tmdl).expect("customer tmdl");
    fs::write(
        &customer_tmdl,
        customer.replace("Sample Customer", "Alice Smith"),
    )
    .expect("PII-like row");

    let pii_scan = run_powerbi(&[
        "package",
        "source-pack",
        "--project",
        project.to_str().expect("project path"),
        "--out",
        package.to_str().expect("package path"),
        "--json",
    ]);
    assert_eq!(pii_scan.code, 10);
    assert!(!package.exists());
    assert_eq!(
        stderr_json(&pii_scan)["error"]["message"],
        "source package content scan failed: PII-suspect row literals requiring review in SalesOperations.SemanticModel/definition/tables/DimCustomer.tmdl"
    );
}

#[test]
fn package_source_pack_refuses_unverified_partition_sources() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let fact_sales = semantic_model_dir(&project)
        .join("definition")
        .join("tables")
        .join("FactSales.tmdl");
    let text = fs::read_to_string(&fact_sales).expect("FactSales TMDL");
    let source_start = text.find("        source =").expect("source block");
    let replacement = r#"        source =
            let
                Source = #table(type table [Unexpected = text], {{"Acme"}})
            in
                Source
"#;
    fs::write(
        &fact_sales,
        format!("{}{}", &text[..source_start], replacement),
    )
    .expect("unverified partition source");
    let package = temp.path().join("unverified-source.pbit");

    let output = run_powerbi(&[
        "package",
        "source-pack",
        "--project",
        project.to_str().expect("project path"),
        "--out",
        package.to_str().expect("package path"),
        "--json",
    ]);
    assert_eq!(output.code, 10);
    assert!(!package.exists());
    assert_eq!(
        stderr_json(&output)["error"]["message"],
        "source package content scan failed: non-dummy or unverified partition source in SalesOperations.SemanticModel/definition/tables/FactSales.tmdl"
    );
}

#[test]
fn package_extract_enforces_streaming_budgets_and_cleans_partial_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let package = temp.path().join("budgeted.pbit");
    write_package_bytes(
        &package,
        zip::CompressionMethod::Stored,
        &[
            ("Sample.pbip", b"{}".to_vec()),
            (
                "Sample.Report/definition/report.json",
                b"0123456789abcdefghijklmnopqrstuvwxyz".to_vec(),
            ),
        ],
    );
    let out_dir = temp.path().join("too-small");
    let failed = run_powerbi(&[
        "package",
        "extract",
        package.to_str().expect("package"),
        "--out-dir",
        out_dir.to_str().expect("out dir"),
        "--max-entry-bytes",
        "16",
        "--json",
    ]);
    assert_eq!(failed.code, 10);
    assert!(
        stderr_json(&failed)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("per-entry extraction limit of 16 bytes")
    );
    assert!(
        !out_dir.exists(),
        "partial extraction directory was removed"
    );

    let successful_out = temp.path().join("large-enough");
    let successful = run_powerbi(&[
        "package",
        "extract",
        package.to_str().expect("package"),
        "--out-dir",
        successful_out.to_str().expect("out dir"),
        "--max-entry-bytes",
        "64",
        "--max-total-bytes",
        "64",
        "--json",
    ]);
    assert_eq!(successful.code, 0, "stderr: {}", successful.stderr);
    assert!(
        successful_out
            .join("Sample.Report/definition/report.json")
            .is_file()
    );

    let total_out = temp.path().join("total-too-small");
    let total_failed = run_powerbi(&[
        "package",
        "extract",
        package.to_str().expect("package"),
        "--out-dir",
        total_out.to_str().expect("out dir"),
        "--max-entry-bytes",
        "64",
        "--max-total-bytes",
        "20",
        "--json",
    ]);
    assert_eq!(total_failed.code, 10);
    assert!(
        stderr_json(&total_failed)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("total uncompressed limit of 20 bytes")
    );
    assert!(!total_out.exists());
}

#[test]
fn package_extract_enforces_entry_count_and_compression_ratio() {
    let temp = tempfile::tempdir().expect("tempdir");
    let count_package = temp.path().join("entry-count.pbit");
    write_package_bytes(
        &count_package,
        zip::CompressionMethod::Stored,
        &[
            ("Sample.pbip", b"{}".to_vec()),
            ("Sample.Report/definition/report.json", b"{}".to_vec()),
            (
                "Sample.SemanticModel/definition/model.tmdl",
                b"model Model\n".to_vec(),
            ),
        ],
    );
    let count_out = temp.path().join("count-out");
    let count_failed = run_powerbi(&[
        "package",
        "extract",
        count_package.to_str().expect("package"),
        "--out-dir",
        count_out.to_str().expect("out dir"),
        "--max-entries",
        "2",
        "--json",
    ]);
    assert_eq!(count_failed.code, 10);
    assert_eq!(
        stderr_json(&count_failed)["error"]["message"],
        "archive contains 3 entries, exceeding the extraction limit of 2"
    );
    assert!(!count_out.exists());

    let ratio_package = temp.path().join("ratio.pbit");
    let compressible = (0..8_192)
        .map(|index| b'a' + (index % 16) as u8)
        .collect::<Vec<_>>();
    write_package_bytes(
        &ratio_package,
        zip::CompressionMethod::Deflated,
        &[("Sample.Report/definition/report.json", compressible)],
    );
    let ratio_out = temp.path().join("ratio-out");
    let ratio_failed = run_powerbi(&[
        "package",
        "extract",
        ratio_package.to_str().expect("package"),
        "--out-dir",
        ratio_out.to_str().expect("out dir"),
        "--max-compression-ratio",
        "2",
        "--json",
    ]);
    assert_eq!(ratio_failed.code, 10);
    assert!(
        stderr_json(&ratio_failed)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("compression-ratio limit of 2:1")
    );
    assert!(!ratio_out.exists());
}

#[test]
fn package_extract_keeps_zip_slip_and_nonempty_destination_guards() {
    let temp = tempfile::tempdir().expect("tempdir");
    let package = temp.path().join("paths.pbit");
    write_package_bytes(
        &package,
        zip::CompressionMethod::Stored,
        &[
            ("../escape.json", b"leak".to_vec()),
            ("Sample.pbip", b"{}".to_vec()),
        ],
    );
    let out_dir = temp.path().join("safe-out");
    let extracted = run_powerbi(&[
        "package",
        "extract",
        package.to_str().expect("package"),
        "--out-dir",
        out_dir.to_str().expect("out dir"),
        "--json",
    ]);
    assert_eq!(extracted.code, 0, "stderr: {}", extracted.stderr);
    assert!(!temp.path().join("escape.json").exists());
    assert!(out_dir.join("Sample.pbip").is_file());
    assert!(
        stdout_json(&extracted)["skipped"]
            .as_array()
            .expect("skipped")
            .iter()
            .any(|entry| entry["name"] == "../escape.json" && entry["skipReason"] == "unsafe-path")
    );

    let nonempty = temp.path().join("nonempty");
    fs::create_dir_all(&nonempty).expect("nonempty dir");
    fs::write(nonempty.join("keep.txt"), "keep").expect("sentinel");
    let refused = run_powerbi(&[
        "package",
        "extract",
        package.to_str().expect("package"),
        "--out-dir",
        nonempty.to_str().expect("out dir"),
        "--json",
    ]);
    assert_eq!(refused.code, 2);
    assert_eq!(
        fs::read_to_string(nonempty.join("keep.txt")).expect("sentinel"),
        "keep"
    );
}

#[test]
fn dax_dependencies_and_lint_report_static_reference_failures() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let add = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "Broken Measure",
        "--expression",
        "[Missing Measure] + 'FactSales'[NoSuchColumn]",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let dependencies = run_powerbi(&[
        "model",
        "dax",
        "dependencies",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(dependencies.code, 0, "stderr: {}", dependencies.stderr);
    let deps_json = stdout_json(&dependencies);
    assert_eq!(
        deps_json["analysisBoundary"]["daxEngineValidated"],
        Value::Bool(false)
    );
    assert!(
        deps_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "dax.reference_missing_column")
    );

    let lint = run_powerbi(&["model", "dax", "lint", "--project", project_arg, "--json"]);
    assert_ne!(lint.code, 0, "DAX lint should fail for broken refs");
    let lint_json = stdout_json(&lint);
    let codes = lint_json["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter_map(|finding| finding["code"].as_str())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"dax.reference_missing_column"));
    assert!(codes.contains(&"dax.reference_missing_measure"));
}

#[test]
fn advanced_model_inventory_reads_roles_perspectives_cultures_and_expressions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let definition = semantic_model_dir(&project).join("definition");
    fs::create_dir_all(definition.join("roles")).expect("roles dir");
    fs::create_dir_all(definition.join("perspectives")).expect("perspectives dir");
    fs::create_dir_all(definition.join("cultures")).expect("cultures dir");
    fs::write(
        definition.join("roles").join("Safety.tmdl"),
        "role Safety\n\tmodelPermission: read\n\ttablePermission FactSales\n",
    )
    .expect("role");
    fs::write(
        definition.join("perspectives").join("Executive.tmdl"),
        "perspective Executive\n\tperspectiveTable FactSales\n",
    )
    .expect("perspective");
    fs::write(
        definition.join("cultures").join("de-CH.tmdl"),
        "culture 'de-CH'\n\ttranslation FactSales\n",
    )
    .expect("culture");
    fs::write(
        definition.join("expressions.tmdl"),
        "expression RefreshDate = DateTime.LocalNow()\n",
    )
    .expect("expressions");

    let inventory = run_powerbi(&[
        "model",
        "advanced",
        "inventory",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(inventory.code, 0, "stderr: {}", inventory.stderr);
    assert_eq!(
        stdout_json(&inventory)["schema"],
        Value::from("powerbi-cli.model.advanced.inventory.v1")
    );

    let roles = run_powerbi(&["model", "roles", "list", "--project", project_arg, "--json"]);
    assert_eq!(roles.code, 0, "stderr: {}", roles.stderr);
    let roles_json = stdout_json(&roles);
    assert_eq!(
        roles_json["records"][0]["handle"],
        Value::from("role:Safety")
    );
    assert_eq!(
        roles_json["records"][0]["summary"]["tablePermissions"],
        Value::from(1)
    );
}

#[test]
fn conditional_formatting_readback_and_style_bundle_round_trip() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_conditional_formatting_fixture(&project);
    let project_arg = project.to_str().expect("project path");
    let visual_handle = first_visual_handle(project_arg);

    let cf_list = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "conditional-formatting",
        "list",
        "--project",
        project_arg,
        "--json",
    ]);
    assert_eq!(cf_list.code, 0, "stderr: {}", cf_list.stderr);
    let cf_json = stdout_json(&cf_list);
    assert_eq!(
        cf_json["schema"],
        Value::from("powerbi-cli.report.visuals.conditionalFormatting.list.v1")
    );
    assert!(
        cf_json["counts"]["conditionalFormattingSignals"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let cf_show = run_powerbi(&[
        "report",
        "visuals",
        "formatting",
        "cf",
        "show",
        "--project",
        project_arg,
        "--handle",
        &visual_handle,
        "--include-raw",
        "--json",
    ]);
    assert_eq!(cf_show.code, 0, "stderr: {}", cf_show.stderr);
    assert_eq!(
        stdout_json(&cf_show)["conditionalFormatting"]["rawIncluded"],
        Value::Bool(true)
    );

    let style_path = temp.path().join("style.json");
    let extract = run_powerbi(&[
        "report",
        "style",
        "extract",
        "--project",
        project_arg,
        "--out",
        style_path.to_str().expect("style path"),
        "--json",
    ]);
    assert_eq!(extract.code, 0, "stderr: {}", extract.stderr);
    assert!(style_path.is_file());
    assert_eq!(
        stdout_json(&extract)["bundle"]["schema"],
        Value::from("powerbi-cli.report.style-bundle.v1")
    );

    let styled = temp.path().join("styled_project");
    let apply = run_powerbi(&[
        "report",
        "style",
        "apply",
        "--project",
        project_arg,
        "--bundle",
        style_path.to_str().expect("style path"),
        "--out-dir",
        styled.to_str().expect("styled path"),
        "--allow-literal-text",
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    assert_eq!(
        stdout_json(&apply)["schema"],
        Value::from("powerbi-cli.report.style.apply.v1")
    );
}

#[test]
fn bookmark_metadata_mutations_round_trip_without_capturing_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    install_flat_bookmarks(&project);
    let project_arg = project.to_str().expect("project path");

    let renamed = temp.path().join("renamed_project");
    let rename = run_powerbi(&[
        "report",
        "bookmarks",
        "set-display-name",
        "--project",
        project_arg,
        "--handle",
        "bookmark:BookmarkA",
        "--display-name",
        "Renamed View",
        "--out-dir",
        renamed.to_str().expect("renamed path"),
        "--json",
    ]);
    assert_eq!(rename.code, 0, "stderr: {}", rename.stderr);
    assert_eq!(
        stdout_json(&rename)["schema"],
        Value::from("powerbi-cli.report.bookmarks.mutation.v1")
    );
    let renamed_bookmark: Value = serde_json::from_str(
        &fs::read_to_string(
            report_dir(&renamed)
                .join("definition")
                .join("bookmarks")
                .join("BookmarkA.bookmark.json"),
        )
        .expect("renamed bookmark"),
    )
    .expect("parse bookmark");
    assert_eq!(renamed_bookmark["displayName"], Value::from("Renamed View"));
    assert!(renamed_bookmark["explorationState"].is_object());

    let reordered = temp.path().join("reordered_project");
    let reorder = run_powerbi(&[
        "report",
        "bookmarks",
        "reorder",
        "--project",
        renamed.to_str().expect("renamed path"),
        "--order",
        "bookmark:BookmarkB,bookmark:BookmarkA",
        "--out-dir",
        reordered.to_str().expect("reordered path"),
        "--json",
    ]);
    assert_eq!(reorder.code, 0, "stderr: {}", reorder.stderr);
    let metadata: Value = serde_json::from_str(
        &fs::read_to_string(
            report_dir(&reordered)
                .join("definition")
                .join("bookmarks")
                .join("bookmarks.json"),
        )
        .expect("bookmarks metadata"),
    )
    .expect("parse metadata");
    assert_eq!(metadata["items"][0]["name"], Value::from("BookmarkB"));

    let deleted = temp.path().join("deleted_project");
    let delete = run_powerbi(&[
        "report",
        "bookmarks",
        "delete",
        "--project",
        reordered.to_str().expect("reordered path"),
        "--handle",
        "bookmark:BookmarkA",
        "--out-dir",
        deleted.to_str().expect("deleted path"),
        "--json",
    ]);
    assert_eq!(delete.code, 0, "stderr: {}", delete.stderr);
    assert!(
        !report_dir(&deleted)
            .join("definition")
            .join("bookmarks")
            .join("BookmarkA.bookmark.json")
            .exists()
    );
}

#[test]
fn capabilities_expose_new_agent_first_surfaces() {
    let output = run_powerbi(&["capabilities", "--json"]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    let value = stdout_json(&output);
    let paths = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .filter_map(|command| command["path"].as_str())
        .collect::<Vec<_>>();
    for expected in [
        "package inspect",
        "package source-pack",
        "package export-plan",
        "model dax dependencies",
        "model dax lint",
        "model advanced inventory",
        "report style extract",
        "report style apply",
        "report visuals formatting conditional-formatting list",
        "report bookmarks set-display-name",
        "report bookmarks reorder",
        "report bookmarks delete",
    ] {
        assert!(paths.contains(&expected), "missing command {expected}");
    }
}
