use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
    let output = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(&args)
        .output()
        .expect("run powerbi-cli");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{} failed: {}",
        case.name,
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).expect("build response JSON");
    assert_eq!(response["ok"], Value::Bool(true));
    assert_eq!(
        response["proof"]["claimedDesktopCompatibility"],
        Value::Bool(false)
    );
}

fn verify_input(input: &BoundInput) {
    let bytes = fs::read(&input.path).expect("read parity input");
    let actual = format!("{:x}", Sha256::digest(bytes));
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
