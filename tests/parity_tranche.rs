mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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

fn page_dir(project: &Path, page_name: &str) -> PathBuf {
    report_dir(project)
        .join("definition")
        .join("pages")
        .join(page_name)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("json file")).expect("parse json")
}

fn page_visual_names(project: &Path, page_name: &str) -> Vec<String> {
    let mut names = fs::read_dir(page_dir(project, page_name).join("visuals"))
        .expect("visuals dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().expect("file type").is_dir())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn project_file_snapshot(project: &Path) -> BTreeMap<String, Vec<u8>> {
    walkdir::WalkDir::new(project)
        .into_iter()
        .map(|entry| entry.expect("walk project"))
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let relative = entry
                .path()
                .strip_prefix(project)
                .expect("relative project file")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, fs::read(entry.path()).expect("project file"))
        })
        .collect()
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

fn column_field(table: &str, column: &str) -> Value {
    json!({
        "Column": {
            "Expression": {"SourceRef": {"Entity": table}},
            "Property": column
        }
    })
}

fn aggregation_field(table: &str, column: &str) -> Value {
    json!({
        "Aggregation": {
            "Expression": {
                "Column": {
                    "Expression": {"SourceRef": {"Entity": table}},
                    "Property": column
                }
            },
            "Function": 0
        }
    })
}

fn scatter_projection(field: Value, native_query_ref: &str, query_ref: &str) -> Value {
    json!({
        "field": field,
        "nativeQueryRef": native_query_ref,
        "queryRef": query_ref
    })
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

#[test]
fn scatter_with_category_rejects_bare_columns_in_all_numeric_roles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    patch_json(&first_visual_json(&project), |visual| {
        visual["visual"]["visualType"] = Value::from("scatterChart");
        visual["visual"]["query"]["queryState"] = json!({
            "Category": {"projections": [scatter_projection(column_field("DimCustomer", "CustomerName"), "CustomerName", "DimCustomer.CustomerName")]},
            "X": {"projections": [scatter_projection(column_field("FactSales", "Revenue"), "Revenue", "FactSales.Revenue")]},
            "Y": {"projections": [scatter_projection(column_field("FactSales", "Units"), "Units", "FactSales.Units")]},
            "Size": {"projections": [scatter_projection(column_field("FactSales", "CustomerKey"), "CustomerKey", "FactSales.CustomerKey")]}
        });
    });

    let output = run_powerbi(&[
        "validate",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 10, "stderr: {}", output.stderr);
    let output_json = stdout_json(&output);
    let errors = output_json["errors"]
        .as_array()
        .expect("validation errors")
        .iter()
        .filter_map(|error| error["message"].as_str())
        .collect::<Vec<_>>();
    for role in ["X", "Y", "Size"] {
        assert!(errors.iter().any(|error| {
            error.contains("PBIR_ROLE_KIND_MISMATCH")
                && error.contains(&format!("queryState.{role}.projections[0].field"))
                && error.contains("found Column")
        }));
    }
}

#[test]
fn scatter_with_category_accepts_aggregation_wrapped_columns() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    patch_json(&first_visual_json(&project), |visual| {
        visual["visual"]["visualType"] = Value::from("scatterChart");
        visual["visual"]["query"]["queryState"] = json!({
            "Category": {"projections": [scatter_projection(column_field("DimCustomer", "CustomerName"), "CustomerName", "DimCustomer.CustomerName")]},
            "X": {"projections": [scatter_projection(aggregation_field("FactSales", "Revenue"), "Summe von Revenue", "Sum(FactSales.Revenue)")]},
            "Y": {"projections": [scatter_projection(aggregation_field("FactSales", "Units"), "Summe von Units", "Sum(FactSales.Units)")]},
            "Size": {"projections": [scatter_projection(aggregation_field("FactSales", "CustomerKey"), "Summe von CustomerKey", "Sum(FactSales.CustomerKey)")]}
        });
    });

    let output = run_powerbi(&[
        "validate",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    assert_eq!(stdout_json(&output)["ok"], Value::Bool(true));
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
fn report_pages_clone_round_trips_and_validates_with_deterministic_visual_suffixes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let source_page = first_page_name(&project);
    let source_visuals = page_visual_names(&project, &source_page);
    let cloned_project = temp.path().join("cloned_project");

    let clone = run_powerbi(&[
        "report",
        "pages",
        "clone",
        "--project",
        project_arg,
        "--from",
        &format!("page:{source_page}"),
        "--new-name",
        "ReportSectionClone",
        "--out-dir",
        cloned_project.to_str().expect("clone output"),
        "--json",
    ]);
    assert_eq!(clone.code, 0, "stderr: {}", clone.stderr);
    let clone_json = stdout_json(&clone);
    assert_eq!(
        clone_json["schema"],
        Value::from("powerbi-cli.report.pages.cloneMutation.v1")
    );
    assert_eq!(clone_json["validation"]["ok"], Value::Bool(true));
    assert_eq!(
        clone_json["target"]["displayName"],
        Value::from("Overview (Kopie)")
    );
    assert_eq!(
        clone_json["counts"]["visualsCloned"],
        Value::from(source_visuals.len())
    );

    let cloned_page_dir = page_dir(&cloned_project, "ReportSectionClone");
    let cloned_page = read_json(&cloned_page_dir.join("page.json"));
    assert_eq!(
        cloned_page["name"],
        Value::from("ReportSectionClone"),
        "page name must never be dropped during clone"
    );
    assert_eq!(cloned_page["displayName"], Value::from("Overview (Kopie)"));

    let visual_renames = clone_json["clonePlan"]["visualRenames"]
        .as_array()
        .expect("visual renames");
    assert_eq!(visual_renames.len(), source_visuals.len());
    for rename in visual_renames {
        let before = rename["before"].as_str().expect("before name");
        let after = rename["after"].as_str().expect("after name");
        assert!(source_visuals.iter().any(|name| name == before));
        assert!(after.starts_with(before));
        assert_eq!(after.len(), before.len() + 8);
        let visual_json = read_json(
            &cloned_page_dir
                .join("visuals")
                .join(after)
                .join("visual.json"),
        );
        assert_eq!(visual_json["name"], Value::from(after));
    }

    let pages = read_json(&pages_json(&cloned_project));
    assert_eq!(
        pages["pageOrder"].as_array().expect("page order").last(),
        Some(&Value::from("ReportSectionClone"))
    );
    let validate = run_powerbi(&[
        "validate",
        "--strict",
        cloned_project.to_str().expect("cloned project"),
        "--json",
    ]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));
}

#[test]
fn report_pages_clone_regenerates_filters_and_retargets_or_drops_interactions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let source_page = first_page_name(&project);
    let source_page_handle = format!("page:{source_page}");
    let source_visuals = page_visual_names(&project, &source_page);
    assert!(source_visuals.len() >= 2);
    let source_visual_handle = first_visual_handle(project_arg);

    let page_filter = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "page",
        "--page",
        &source_page_handle,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "Enterprise",
        "--in-place",
        "--json",
    ]);
    assert_eq!(page_filter.code, 0, "stderr: {}", page_filter.stderr);
    let visual_filter = run_powerbi(&[
        "report",
        "filters",
        "add",
        "--project",
        project_arg,
        "--scope",
        "visual",
        "--visual",
        &source_visual_handle,
        "--target",
        "DimCustomer[Segment]",
        "--value",
        "SMB",
        "--in-place",
        "--json",
    ]);
    assert_eq!(visual_filter.code, 0, "stderr: {}", visual_filter.stderr);

    let source_page_json = page_dir(&project, &source_page).join("page.json");
    patch_json(&source_page_json, |page| {
        page["filterConfig"]["filters"][0]["name"] = Value::from("P".repeat(50));
        page["visualInteractions"] = json!([
            {
                "source": source_visuals[0],
                "target": source_visuals[1],
                "type": "DataFilter"
            },
            {
                "source": source_visuals[0],
                "target": "MissingVisual",
                "type": "HighlightFilter"
            },
            {
                "source": "MissingSource",
                "target": source_visuals[1],
                "type": "NoFilter"
            }
        ]);
        page["annotations"]
            .as_array_mut()
            .expect("page annotations")
            .push(json!({
                "name": "test.visualReferences",
                "value": format!("{} -> {}", source_visuals[0], source_visuals[1])
            }));
    });
    let source_visual_json = PathBuf::from(
        stdout_json(&visual_filter)["owner"]["path"]
            .as_str()
            .expect("visual owner path"),
    );
    patch_json(&source_visual_json, |visual| {
        visual["filterConfig"]["filters"][0]["name"] = Value::from("V".repeat(50));
    });

    let clone = run_powerbi(&[
        "report",
        "pages",
        "clone",
        "--project",
        project_arg,
        "--from",
        &source_page_handle,
        "--new-name",
        "ReportSectionRateCopy",
        "--display-name",
        "Rate Copy",
        "--visual-prefix",
        "Rate",
        "--in-place",
        "--json",
    ]);
    assert_eq!(clone.code, 0, "stderr: {}", clone.stderr);
    let clone_json = stdout_json(&clone);
    assert_eq!(clone_json["validation"]["ok"], Value::Bool(true));
    assert_eq!(clone_json["counts"]["filtersRenamed"], Value::from(2));
    assert_eq!(clone_json["counts"]["interactionsDropped"], Value::from(2));
    assert_eq!(
        clone_json["warnings"].as_array().expect("warnings").len(),
        2
    );
    assert!(
        clone_json["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .all(|warning| {
                warning["code"] == "page_clone.stale_visual_interaction_dropped"
                    && warning["severity"] == "warning"
            })
    );
    assert_eq!(
        clone_json["changes"]
            .as_array()
            .expect("changes")
            .iter()
            .filter(|change| change["action"] == "drop-stale")
            .count(),
        2
    );

    let rename_map = clone_json["clonePlan"]["visualRenames"]
        .as_array()
        .expect("visual renames")
        .iter()
        .map(|rename| {
            (
                rename["before"].as_str().expect("before").to_string(),
                rename["after"].as_str().expect("after").to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let cloned_page_dir = page_dir(&project, "ReportSectionRateCopy");
    let cloned_page = read_json(&cloned_page_dir.join("page.json"));
    let interactions = cloned_page["visualInteractions"]
        .as_array()
        .expect("interactions");
    assert_eq!(interactions.len(), 1);
    assert_eq!(
        interactions[0]["source"],
        Value::from(rename_map[&source_visuals[0]].as_str())
    );
    assert_eq!(
        interactions[0]["target"],
        Value::from(rename_map[&source_visuals[1]].as_str())
    );
    assert_eq!(
        cloned_page["annotations"]
            .as_array()
            .expect("annotations")
            .last()
            .expect("reference annotation")["value"],
        Value::from(format!(
            "{} -> {}",
            rename_map[&source_visuals[0]], rename_map[&source_visuals[1]]
        ))
    );

    let page_filter_name = cloned_page["filterConfig"]["filters"][0]["name"]
        .as_str()
        .expect("page filter name");
    assert_ne!(page_filter_name, "P".repeat(50));
    assert!(page_filter_name.starts_with("PowerBICliPage"));
    assert!(page_filter_name.len() <= 50);
    let source_visual_name = source_visual_json
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .expect("source visual name");
    let cloned_visual_name = &rename_map[source_visual_name];
    let cloned_visual = read_json(
        &cloned_page_dir
            .join("visuals")
            .join(cloned_visual_name)
            .join("visual.json"),
    );
    assert_eq!(
        cloned_visual["name"],
        Value::from(cloned_visual_name.as_str())
    );
    let visual_filter_name = cloned_visual["filterConfig"]["filters"][0]["name"]
        .as_str()
        .expect("visual filter name");
    assert_ne!(visual_filter_name, "V".repeat(50));
    assert!(visual_filter_name.starts_with("PowerBICliVisual"));
    assert!(visual_filter_name.len() <= 50);

    let validate = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(validate.code, 0, "stderr: {}", validate.stderr);
    assert_eq!(stdout_json(&validate)["ok"], Value::Bool(true));
}

#[test]
fn report_pages_clone_refuses_duplicates_and_dry_run_writes_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");
    let source_page = first_page_name(&project);
    let before = project_file_snapshot(&project);

    let dry_run = run_powerbi(&[
        "report",
        "pages",
        "clone",
        "--project",
        project_arg,
        "--from",
        &source_page,
        "--new-name",
        "ReportSectionDryRun",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    assert_eq!(stdout_json(&dry_run)["dryRun"], Value::Bool(true));
    assert_eq!(project_file_snapshot(&project), before);
    assert!(!page_dir(&project, "ReportSectionDryRun").exists());

    let duplicate = run_powerbi(&[
        "report",
        "pages",
        "clone",
        "--project",
        project_arg,
        "--from",
        &source_page,
        "--new-name",
        &source_page.to_ascii_lowercase(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(duplicate.code, 2);
    let error = stderr_json(&duplicate);
    assert_eq!(error["error"]["code"], Value::from("invalid_args"));
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("page already exists")
    );
    assert_eq!(project_file_snapshot(&project), before);
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

    let source_template = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--kind",
        "postgres",
        "--server",
        "<server>",
        "--database",
        "<database>",
        "--schema",
        "public",
        "--object",
        "fact_sales",
        "--in-place",
        "--json",
    ]);
    assert_eq!(
        source_template.code, 0,
        "stderr: {}",
        source_template.stderr
    );

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
    assert!(
        imported
            .join(".powerbi-cli")
            .join("source-templates.json")
            .is_file()
    );

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
fn dax_lint_accepts_extension_and_grouping_virtual_columns() {
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
        "Ranked Revenue",
        "--expression",
        "VAR Grouped = SUMMARIZECOLUMNS('DimCustomer'[Segment], \"__GroupedValue\", [Total Revenue]) VAR Ranked = ADDCOLUMNS(Grouped, \"__RankValue\", [__GroupedValue]) VAR Projected = SELECTCOLUMNS(Ranked, \"__ProjectedValue\", [__RankValue]) RETURN MAXX(Projected, [__ProjectedValue])",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let lint = run_powerbi(&["model", "dax", "lint", "--project", project_arg, "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    let lint_json = stdout_json(&lint);
    assert_eq!(lint_json["counts"]["errors"], 0);
    assert!(
        lint_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .all(|finding| finding["code"] != "dax.reference_missing_measure")
    );
}

#[test]
fn dax_lint_accepts_groupby_and_summarize_virtual_columns() {
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
        "Grouped Revenue",
        "--expression",
        "VAR Grouped = GROUPBY(SUMMARIZECOLUMNS('DimCustomer'[Segment], \"__GroupedValue\", [Total Revenue]), [__GroupedValue], \"__GroupSum\", SUMX(CURRENTGROUP(), [__GroupedValue])) VAR Summarized = SUMMARIZE(FactSales, 'DimCustomer'[Segment], \"__SummaryValue\", [Total Revenue]) RETURN MAXX(Grouped, [__GroupSum]) + MAXX(Summarized, [__SummaryValue])",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let lint = run_powerbi(&["model", "dax", "lint", "--project", project_arg, "--json"]);
    assert_eq!(lint.code, 0, "stderr: {}", lint.stderr);
    let lint_json = stdout_json(&lint);
    assert_eq!(lint_json["counts"]["errors"], 0);
    assert!(
        lint_json["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .all(|finding| finding["code"] != "dax.reference_missing_measure")
    );
}

#[test]
fn dax_lint_does_not_treat_summarizecolumns_string_values_as_aliases() {
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
        "Broken Grouping Reference",
        "--expression",
        "VAR Grouped = SUMMARIZECOLUMNS('DimCustomer'[Segment], \"__GroupedValue\", \"constant\") RETURN MAXX(Grouped, [constant])",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);

    let lint = run_powerbi(&["model", "dax", "lint", "--project", project_arg, "--json"]);
    assert_ne!(lint.code, 0, "DAX lint should reject the missing reference");
    assert!(
        stdout_json(&lint)["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| {
                finding["code"] == "dax.reference_missing_measure"
                    && finding["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("[constant]"))
            })
    );
}

#[test]
fn dax_lint_rejects_scalar_if_variable_used_as_a_table() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = scaffold_sales(temp.path());
    let project_arg = project.to_str().expect("project path");

    let broken = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "Broken Table Choice",
        "--expression",
        "VAR Candidate = IF(TRUE(), VALUES('DimCustomer'[Segment]), ALL('DimCustomer'[Segment])) RETURN COUNTROWS(TREATAS(Candidate, 'DimCustomer'[Segment]))",
        "--in-place",
        "--json",
    ]);
    assert_eq!(broken.code, 0, "stderr: {}", broken.stderr);

    let valid_scalar = run_powerbi(&[
        "model",
        "measures",
        "add",
        "--project",
        project_arg,
        "--table",
        "FactSales",
        "--name",
        "Valid Scalar Choice",
        "--expression",
        "VAR Candidate = IF(TRUE(), 1, 0) RETURN Candidate",
        "--in-place",
        "--json",
    ]);
    assert_eq!(valid_scalar.code, 0, "stderr: {}", valid_scalar.stderr);

    let lint = run_powerbi(&["model", "dax", "lint", "--project", project_arg, "--json"]);
    assert_eq!(lint.code, 10, "stderr: {}", lint.stderr);
    let lint_json = stdout_json(&lint);
    let table_if_findings = lint_json["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|finding| finding["code"] == "dax.table_variable_scalar_if")
        .collect::<Vec<_>>();
    assert_eq!(table_if_findings.len(), 1);
    assert!(
        table_if_findings[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("TREATAS")
    );

    let strict = run_powerbi(&["validate", "--strict", project_arg, "--json"]);
    assert_eq!(strict.code, 10, "stderr: {}", strict.stderr);
    assert!(
        stdout_json(&strict)["lint"]["findings"]
            .as_array()
            .expect("strict lint findings")
            .iter()
            .any(|finding| finding["code"] == "dax.table_variable_scalar_if")
    );
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
        "report pages clone",
    ] {
        assert!(paths.contains(&expected), "missing command {expected}");
    }
}
