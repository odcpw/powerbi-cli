use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

struct CorpusCase {
    name: &'static str,
    schema: &'static str,
    profile: Option<&'static str>,
    spec: &'static str,
    expected_files: usize,
    expected_bytes: u64,
    expected_sha256: &'static str,
}

#[derive(Debug, Eq, PartialEq)]
struct TreeFingerprint {
    files: usize,
    bytes: u64,
    sha256: String,
}

#[test]
fn generated_artifact_trees_are_byte_deterministic_and_match_the_bound_corpus() {
    let cases = [
        CorpusCase {
            name: "sales",
            schema: "examples/sales.schema.json",
            profile: Some("examples/sales.profile.json"),
            spec: "examples/sales.dashboard.json",
            expected_files: 21,
            expected_bytes: 21_518,
            expected_sha256: "9cbf2eb2694e9691740405bb71043d16798bd82f815416c6cc3a67c464b73c85",
        },
        CorpusCase {
            name: "flat-ops",
            schema: "examples/archetypes/flat-ops.schema.json",
            profile: Some("examples/archetypes/flat-ops.profile.json"),
            spec: "examples/archetypes/flat-ops.dashboard.json",
            expected_files: 19,
            expected_bytes: 17_724,
            expected_sha256: "6fe83dc969470bfbd4d751df752660a5d83aa90fc3c08df6972b0fe820feab29",
        },
        CorpusCase {
            name: "scatter-bubble",
            schema: "examples/archetypes/scatter-bubble.schema.json",
            profile: Some("examples/archetypes/scatter-bubble.profile.json"),
            spec: "examples/archetypes/scatter-bubble.dashboard.json",
            expected_files: 18,
            expected_bytes: 19_562,
            expected_sha256: "067da0322222a38ffe02593568d2206a7897e1f058351582c114747becf69f3d",
        },
        CorpusCase {
            name: "catalog-proof",
            schema: "examples/archetypes/catalog-proof.schema.json",
            profile: Some("examples/archetypes/catalog-proof.profile.json"),
            spec: "examples/archetypes/catalog-proof.dashboard.json",
            expected_files: 27,
            expected_bytes: 31_190,
            expected_sha256: "131646503088d3c861a92f4e79bbb9db10dbe3a3ed53961f172e7b782216aa7c",
        },
        CorpusCase {
            name: "regional-sales",
            schema: "examples/archetypes/regional-sales.schema.json",
            profile: Some("examples/archetypes/regional-sales.profile.json"),
            spec: "examples/archetypes/regional-sales.dashboard.json",
            expected_files: 28,
            expected_bytes: 37_788,
            expected_sha256: "58bff63041ae99bd760e05f5f9839e19c5236faef7539254ea3ed4652dd9f90a",
        },
    ];

    let temp = tempfile::tempdir().expect("parity tempdir");
    for case in cases {
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

        let expected = TreeFingerprint {
            files: case.expected_files,
            bytes: case.expected_bytes,
            sha256: case.expected_sha256.to_string(),
        };
        assert_eq!(
            first_fingerprint, expected,
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
        case.schema.to_string(),
    ];
    if let Some(profile) = case.profile {
        args.extend(["--profile".to_string(), profile.to_string()]);
    }
    args.extend([
        "--spec".to_string(),
        case.spec.to_string(),
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
