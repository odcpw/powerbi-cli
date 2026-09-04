mod common;

use common::{run_powerbi_owned, stdout_json};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusManifest {
    schema: String,
    source_revision: String,
    artifact_algorithm: String,
    closure: String,
    nonclaims: Vec<String>,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusCase {
    name: String,
    schema: BoundInput,
    profile: Option<BoundInput>,
    spec: BoundInput,
    expected: TreeFingerprint,
}

#[derive(Debug, Deserialize)]
struct BoundInput {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct TreeFingerprint {
    files: usize,
    bytes: u64,
    sha256: String,
}

#[test]
fn generated_artifact_trees_are_byte_deterministic_and_match_the_bound_corpus() {
    let manifest: CorpusManifest =
        serde_json::from_str(include_str!("../testdata/golden/artifact-parity.v1.json"))
            .expect("artifact parity manifest");
    assert_eq!(manifest.schema, "powerbi-cli.artifact-parity-corpus.v1");
    assert_eq!(manifest.artifact_algorithm, "powerbi-cli.artifact-tree.v1");
    assert_eq!(manifest.closure, "complete_for_declared_corpus");
    assert_eq!(manifest.source_revision.len(), 40);
    assert!(!manifest.nonclaims.is_empty());

    let temp = tempfile::tempdir().expect("parity tempdir");
    for case in manifest.cases {
        verify_input(&case.schema);
        if let Some(profile) = &case.profile {
            verify_input(profile);
        }
        verify_input(&case.spec);
        let first = temp.path().join(format!("{}-first", case.name));
        let second = temp.path().join(format!("{}-second", case.name));
        build_case(&case, &first);
        build_case(&case, &second);

        let first_fingerprint = fingerprint_tree(&first);
        let second_fingerprint = fingerprint_tree(&second);
        assert_eq!(
            first_fingerprint, second_fingerprint,
            "{} changed across two executions of the same binary",
            case.name
        );

        assert_eq!(
            first_fingerprint, case.expected,
            "{} no longer matches the checksum-bound artifact corpus",
            case.name
        );
    }
}

#[test]
fn normalized_schema_input_keeps_include_and_inline_artifact_fingerprints_equal() {
    let temp = tempfile::tempdir().expect("normalized parity tempdir");
    let parts = temp.path().join("parts");
    fs::create_dir_all(&parts).expect("normalized parity parts");
    let table = json!({
        "name": "Fact",
        "columns": [{"name": "Value", "dataType": "int64"}],
        "rows": []
    });
    write_json(
        &parts.join("table.json"),
        &json!({"tables": [table.clone()]}),
    );
    let included_schema = temp.path().join("included.schema.json");
    write_json(
        &included_schema,
        &json!({
            "schemaVersion": "1",
            "name": "NormalizedParity",
            "$include": "parts/table.json"
        }),
    );
    let inline_schema = temp.path().join("inline.schema.json");
    write_json(
        &inline_schema,
        &json!({"schemaVersion": "1", "name": "NormalizedParity", "tables": [table]}),
    );
    let spec = temp.path().join("parity.dashboard.json");
    write_json(
        &spec,
        &json!({
            "schema": "powerbi-cli.dashboard.v1",
            "report": {"name": "NormalizedParity"},
            "pages": []
        }),
    );

    let included_project = temp.path().join("included-project");
    let inline_project = temp.path().join("inline-project");
    build_paths(&included_schema, &spec, &included_project);
    build_paths(&inline_schema, &spec, &inline_project);
    assert_eq!(
        fingerprint_tree(&included_project),
        fingerprint_tree(&inline_project),
        "artifact parity must fingerprint normalized schema content"
    );
}

fn build_paths(schema: &Path, spec: &Path, out_dir: &Path) {
    let output = run_powerbi_owned(&[
        "report".to_string(),
        "build".to_string(),
        "--schema".to_string(),
        path_arg(schema),
        "--spec".to_string(),
        path_arg(spec),
        "--out-dir".to_string(),
        path_arg(out_dir),
        "--json".to_string(),
    ]);
    assert_eq!(
        output.exit, 0,
        "normalized parity build failed: {}",
        output.stderr
    );
    assert_eq!(stdout_json(&output)["ok"], Value::Bool(true));
}

fn write_json(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize normalized parity input");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write normalized parity input");
}

fn build_case(case: &CorpusCase, out_dir: &Path) {
    let mut args = vec![
        "report".to_string(),
        "build".to_string(),
        "--schema".to_string(),
        case.schema.path.clone(),
    ];
    if let Some(profile) = &case.profile {
        args.extend(["--profile".to_string(), profile.path.clone()]);
    }
    args.extend([
        "--spec".to_string(),
        case.spec.path.clone(),
        "--out-dir".to_string(),
        path_arg(out_dir),
        "--json".to_string(),
    ]);
    let output = run_powerbi_owned(&args);
    assert_eq!(output.exit, 0, "{} failed: {}", case.name, output.stderr);
    let response: Value = stdout_json(&output);
    assert_eq!(response["ok"], Value::Bool(true));
    assert_eq!(
        response["proof"]["claimedDesktopCompatibility"],
        Value::Bool(false)
    );
}

fn verify_input(input: &BoundInput) {
    let bytes = fs::read(&input.path).expect("read parity input");
    let text = String::from_utf8(bytes).expect("parity inputs are UTF-8 repository text");
    let canonical = text.replace("\r\n", "\n");
    let actual = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    assert_eq!(actual, input.sha256, "parity input drifted: {}", input.path);
}

fn fingerprint_tree(root: &Path) -> TreeFingerprint {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|entry| entry.expect("walk generated artifact tree"))
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| normalized_relative(root, path));

    let mut hasher = Sha256::new();
    hasher.update(b"powerbi-cli.artifact-tree.v1\0");
    let mut total_bytes = 0_u64;
    for path in &files {
        let relative = normalized_relative(root, path);
        let bytes = fs::read(path).expect("read generated artifact");
        let relative_bytes = relative.as_bytes();
        hasher.update((relative_bytes.len() as u64).to_le_bytes());
        hasher.update(relative_bytes);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        total_bytes += bytes.len() as u64;
    }

    TreeFingerprint {
        files: files.len(),
        bytes: total_bytes,
        sha256: format!("{:x}", hasher.finalize()),
    }
}

fn normalized_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("artifact under tree root")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn path_arg(path: &Path) -> String {
    PathBuf::from(path).to_string_lossy().into_owned()
}
