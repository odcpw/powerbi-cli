mod common;

use common::{run_powerbi, stderr_json, stdout_json};
use serde_json::json;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

fn scaffold_single_table(root: &Path, name: &str) -> PathBuf {
    let schema = root.join(format!("{name}.schema.json"));
    fs::write(
        &schema,
        serde_json::to_vec_pretty(&json!({
            "name": name,
            "displayName": "Work Pack Fixture",
            "tables": [{
                "name": "Metrics",
                "columns": [{"name": "Key", "dataType": "int64", "isKey": true}],
                "rows": [{"Key": 0}]
            }]
        }))
        .expect("schema JSON"),
    )
    .expect("write schema");
    let project = root.join(format!("{name}_project"));
    let output = run_powerbi(&[
        "scaffold",
        "--schema",
        schema.to_str().expect("schema path"),
        "--out-dir",
        project.to_str().expect("project path"),
        "--json",
    ]);
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    project
}

fn materialize_sql_source(root: &Path, name: &str) -> PathBuf {
    let project = scaffold_single_table(root, name);
    let project_arg = project.to_str().expect("project path");
    let add = run_powerbi(&[
        "source-template",
        "add",
        "--project",
        project_arg,
        "--table",
        "Metrics",
        "--kind",
        "sql",
        "--server",
        "<server>",
        "--database",
        "<database>",
        "--schema",
        "dbo",
        "--object",
        "Metrics",
        "--in-place",
        "--json",
    ]);
    assert_eq!(add.code, 0, "stderr: {}", add.stderr);
    let apply = run_powerbi(&[
        "source-template",
        "apply",
        "--project",
        project_arg,
        "--handle",
        "source-template:Metrics:Metrics",
        "--server",
        "db.internal",
        "--database",
        "analytics",
        "--in-place",
        "--json",
    ]);
    assert_eq!(apply.code, 0, "stderr: {}", apply.stderr);
    let handoff = run_powerbi(&[
        "handoff",
        "check",
        project_arg,
        "--target",
        "work",
        "--json",
    ]);
    assert_eq!(handoff.code, 0, "stderr: {}", handoff.stderr);
    assert_eq!(stdout_json(&handoff)["safeForWorkHandoff"], true);
    project
}

fn archive_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let file = fs::File::open(path).expect("open archive");
    let mut archive = zip::ZipArchive::new(file).expect("ZIP archive");
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("archive entry");
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read archive entry");
        entries.push((entry.name().replace('\\', "/"), bytes));
    }
    entries
}

#[test]
fn work_pack_is_deterministic_distinct_and_contains_only_safe_source_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = materialize_sql_source(temp.path(), "WorkPackSafe");
    let project_arg = project.to_str().expect("project path");
    let default_archive = temp.path().join("WorkPackSafe_project-work.pbit");

    let dry_run = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        project_arg,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(dry_run.code, 0, "stderr: {}", dry_run.stderr);
    let dry_json = stdout_json(&dry_run);
    assert_eq!(dry_json["changed"], false);
    assert_eq!(dry_json["dryRun"], true);
    assert_eq!(dry_json["packageClass"], "work-package");
    assert_eq!(
        dry_json["sourcePolicy"],
        "recognized-credential-free-materialized-live-partitions-only"
    );
    assert_eq!(
        dry_json["package"],
        default_archive.to_str().expect("default archive path")
    );
    assert!(!default_archive.exists());

    let first = run_powerbi(&["package", "work-pack", "--project", project_arg, "--json"]);
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    assert!(default_archive.is_file());
    let original_bytes = fs::read(&default_archive).expect("original archive");
    let overwrite_refused =
        run_powerbi(&["package", "work-pack", "--project", project_arg, "--json"]);
    assert_eq!(overwrite_refused.code, 2);
    assert_eq!(
        fs::read(&default_archive).expect("unchanged archive"),
        original_bytes
    );
    let forced = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        project_arg,
        "--force",
        "--json",
    ]);
    assert_eq!(forced.code, 0, "stderr: {}", forced.stderr);
    assert_eq!(
        fs::read(&default_archive).expect("forced archive"),
        original_bytes,
        "forced deterministic replacement must preserve archive bytes"
    );

    let nested_archive = project.join("nested-work.pbit");
    let nested = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        project_arg,
        "--out",
        nested_archive.to_str().expect("nested archive"),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(nested.code, 2);
    assert!(!nested_archive.exists());

    let second_archive = temp.path().join("explicit-work.pbit");
    let second = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        project_arg,
        "--out",
        second_archive.to_str().expect("second archive"),
        "--json",
    ]);
    assert_eq!(second.code, 0, "stderr: {}", second.stderr);
    assert_eq!(
        fs::read(&default_archive).expect("first archive"),
        fs::read(&second_archive).expect("second archive"),
        "identical input projects must produce byte-identical work packs"
    );

    let entries = archive_entries(&default_archive);
    assert!(
        entries
            .iter()
            .any(|(name, _)| name == "powerbi-cli.work-pack.json")
    );
    for (name, bytes) in &entries {
        let lower_name = name.to_ascii_lowercase();
        assert!(!lower_name.contains("/.pbi/"), "cache entry: {name}");
        assert!(!lower_name.ends_with("cache.abf"), "cache entry: {name}");
        assert!(
            !lower_name.ends_with("localsettings.json"),
            "local settings entry: {name}"
        );
        assert!(!lower_name.ends_with(".pbix"), "PBIX entry: {name}");
        let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
        assert!(!text.contains("password="), "credential in {name}");
        assert!(!text.contains("authorization:"), "credential in {name}");
        if lower_name.ends_with(".tmdl") {
            assert!(!text.contains("#table"), "embedded rows in {name}");
            assert!(!text.contains("table.fromrows("), "embedded rows in {name}");
            assert!(
                !text.contains("table.fromrecords("),
                "embedded rows in {name}"
            );
            assert!(
                !text.contains("table.fromcolumns("),
                "embedded rows in {name}"
            );
        }
    }

    let inspect = run_powerbi(&[
        "package",
        "inspect",
        default_archive.to_str().expect("archive path"),
        "--json",
    ]);
    assert_eq!(inspect.code, 0, "stderr: {}", inspect.stderr);
    assert_eq!(stdout_json(&inspect)["packageClass"], "work-package");

    let imported = temp.path().join("imported_work");
    let import = run_powerbi(&[
        "package",
        "import",
        default_archive.to_str().expect("archive path"),
        "--out-dir",
        imported.to_str().expect("imported project"),
        "--json",
    ]);
    assert_eq!(import.code, 0, "stderr: {}", import.stderr);
    let import_json = stdout_json(&import);
    assert_eq!(import_json["packageClass"], "work-package");
    assert!(imported.join("powerbi-cli.work-pack.json").is_file());
    let imported_handoff = run_powerbi(&[
        "handoff",
        "check",
        imported.to_str().expect("imported project"),
        "--target",
        "work",
        "--json",
    ]);
    assert_eq!(
        imported_handoff.code, 0,
        "stderr: {}",
        imported_handoff.stderr
    );

    let capability = run_powerbi(&[
        "capabilities",
        "--for",
        "package work-pack",
        "--compact",
        "--json",
    ]);
    assert_eq!(capability.code, 0, "stderr: {}", capability.stderr);
    assert_eq!(stdout_json(&capability)["path"], "package work-pack");
    let features = run_powerbi(&[
        "features",
        "list",
        "--for",
        "package.pbix-pbit-boundary",
        "--json",
    ]);
    assert_eq!(features.code, 0, "stderr: {}", features.stderr);
    let feature_json = stdout_json(&features);
    assert_eq!(feature_json["matchedFeatures"], 1);
    assert!(
        feature_json["features"][0]["commands"]
            .as_array()
            .expect("feature commands")
            .contains(&json!("package work-pack"))
    );
}

#[test]
fn work_pack_uses_the_source_pack_allowlist_for_unknown_and_dot_directory_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = materialize_sql_source(temp.path(), "WorkPackAllowlist");
    fs::create_dir_all(project.join(".git")).expect("dot directory");
    fs::write(project.join(".git/config"), b"[core]\n").expect("dot file");
    fs::write(project.join(".env"), b"SAFE_NAME=value\n").expect("root dot file");
    fs::write(project.join("notes.txt"), b"safe-looking notes\n").expect("unknown file");
    let archive = temp.path().join("allowlist-work.pbit");
    let output = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        project.to_str().expect("project"),
        "--out",
        archive.to_str().expect("archive"),
        "--json",
    ]);
    assert_eq!(output.code, 10);
    assert!(!archive.exists());
    assert_eq!(
        stderr_json(&output)["error"]["message"],
        "project contains unapproved work-package files: .env, .git/config, notes.txt"
    );
}

#[test]
fn work_pack_refuses_dummy_credentials_embedded_rows_and_unsafe_project_files() {
    let temp = tempfile::tempdir().expect("tempdir");

    let dummy = scaffold_single_table(temp.path(), "WorkPackDummy");
    let dummy_archive = temp.path().join("dummy-work.pbit");
    let dummy_output = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        dummy.to_str().expect("dummy project"),
        "--out",
        dummy_archive.to_str().expect("dummy archive"),
        "--json",
    ]);
    assert_eq!(dummy_output.code, 10);
    assert!(!dummy_archive.exists());
    assert!(
        stderr_json(&dummy_output)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not a recognized credential-free materialized live connector")
    );

    let credential_project = materialize_sql_source(temp.path(), "WorkPackCredential");
    fs::write(
        credential_project.join("POWERBI_HANDOFF.md"),
        "Password=hunter2\n",
    )
    .expect("credential file");
    let credential_archive = temp.path().join("credential-work.pbit");
    let credential_output = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        credential_project.to_str().expect("credential project"),
        "--out",
        credential_archive.to_str().expect("credential archive"),
        "--json",
    ]);
    assert_eq!(credential_output.code, 10);
    assert!(!credential_archive.exists());
    assert!(
        stderr_json(&credential_output)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("credential-like content")
    );

    let row_project = materialize_sql_source(temp.path(), "WorkPackRows");
    let table = row_project
        .join("WorkPackRows.SemanticModel")
        .join("definition")
        .join("tables")
        .join("Metrics.tmdl");
    let source = fs::read_to_string(&table).expect("table TMDL");
    let injected = source.replace(
        "                Navigation = Source{[Schema=\"dbo\",Item=\"Metrics\"]}[Data]\n",
        "                Navigation = Source{[Schema=\"dbo\",Item=\"Metrics\"]}[Data],\n                EmbeddedRows = #table({\"Key\"}, {{7}})\n",
    );
    assert_ne!(injected, source, "fixture must inject embedded rows");
    fs::write(&table, injected).expect("inject embedded rows");
    let row_archive = temp.path().join("rows-work.pbit");
    let row_output = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        row_project.to_str().expect("row project"),
        "--out",
        row_archive.to_str().expect("row archive"),
        "--json",
    ]);
    assert_eq!(row_output.code, 10);
    assert!(!row_archive.exists());
    assert!(
        stderr_json(&row_output)["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("embedded M row constructors")
    );

    let unsafe_project = materialize_sql_source(temp.path(), "WorkPackUnsafe");
    fs::create_dir_all(unsafe_project.join(".pbi")).expect("cache dir");
    fs::write(unsafe_project.join(".pbi/cache.abf"), b"cache").expect("cache file");
    fs::write(unsafe_project.join("localSettings.json"), b"{}\n").expect("local settings");
    fs::write(unsafe_project.join("materialized.pbix"), b"binary").expect("PBIX file");
    let unsafe_archive = temp.path().join("unsafe-work.pbit");
    let unsafe_output = run_powerbi(&[
        "package",
        "work-pack",
        "--project",
        unsafe_project.to_str().expect("unsafe project"),
        "--out",
        unsafe_archive.to_str().expect("unsafe archive"),
        "--json",
    ]);
    assert_eq!(unsafe_output.code, 10);
    assert!(!unsafe_archive.exists());
    assert_eq!(
        stderr_json(&unsafe_output)["error"]["code"],
        "validation_failed"
    );
}
