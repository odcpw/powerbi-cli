use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use walkdir::WalkDir;

const CACHE_ENV: &str = "POWERBI_CLI_MICROSOFT_CACHE_DIR";

fn run_powerbi(
    args: &[&str],
    cache: &Path,
    fake_bin: Option<&Path>,
    timeout_ms: Option<u64>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"));
    command.args(args).env(CACHE_ENV, cache);
    if let Some(fake_bin) = fake_bin {
        command.env("PATH", fake_bin);
    }
    if let Some(timeout_ms) = timeout_ms {
        command.env("POWERBI_CLI_TEST_REPORT_TIMEOUT_MS", timeout_ms.to_string());
    } else {
        command.env_remove("POWERBI_CLI_TEST_REPORT_TIMEOUT_MS");
    }
    command.output().expect("run powerbi-cli")
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout JSON")
}

fn stderr_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stderr).expect("stderr JSON")
}

#[test]
fn native_validation_remains_the_default_without_microsoft_cache() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = write_minimal_project(temp.path(), false);
    let cache = temp.path().join("missing-cache");
    let output = run_powerbi(
        &["validate", project.to_str().expect("project"), "--json"],
        &cache,
        None,
        None,
    );

    assert_eq!(output.status.code(), Some(10));
    let value = stdout_json(&output);
    assert!(value.get("schema").is_none());
    assert_eq!(value["backend"], "native");
    assert_eq!(value["validators"]["native"]["id"], "native");
    assert!(output.stderr.is_empty());
    assert!(!cache.exists());
}

#[test]
fn microsoft_backend_missing_tool_is_explicit_dependency_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = write_minimal_project(temp.path(), false);
    let cache = temp.path().join("missing-cache");
    let output = run_powerbi(
        &[
            "validate",
            project.to_str().expect("project"),
            "--backend",
            "microsoft-report",
            "--json",
        ],
        &cache,
        None,
        None,
    );

    assert_eq!(output.status.code(), Some(30));
    assert_eq!(
        stderr_json(&output)["error"]["code"],
        "dependency_unavailable"
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn microsoft_report_normalizes_diagnostics_and_disables_schema_downloads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = write_minimal_project(temp.path(), false);
    let fake = FakeToolchain::new(
        temp.path(),
        &json!({
            "data": {
                "result": "failed",
                "errorCount": 1,
                "warningCount": 0,
                "reportPath": "ignored",
                "diagnostics": {
                    "PBIR_NEUTRAL_TEST": {
                        "severity": "error",
                        "items": [{
                            "message": "password=super-secret in the neutral fixture",
                            "file": project.join("Neutral.Report/definition/report.json"),
                            "path": "visual.objects"
                        }]
                    }
                }
            }
        }),
        1,
        FakeMode::Normal,
    );
    let output = run_powerbi(
        &[
            "validate",
            project.to_str().expect("project"),
            "--backend",
            "microsoft-report",
            "--json",
        ],
        &fake.cache,
        Some(&fake.bin),
        None,
    );

    assert_eq!(output.status.code(), Some(10));
    let value = stdout_json(&output);
    assert_eq!(value["schema"], "powerbi-cli.validate.microsoft-report.v1");
    assert_eq!(value["backend"], "microsoft-report");
    let validator = &value["validators"]["microsoftReport"];
    assert_eq!(validator["id"], "microsoft-report");
    assert_eq!(validator["version"], "0.1.4");
    assert_eq!(validator["schemaValidation"], false);
    assert_eq!(validator["diagnostics"][0]["code"], "PBIR_NEUTRAL_TEST");
    assert_eq!(
        validator["diagnostics"][0]["file"],
        "definition/report.json"
    );
    assert_eq!(validator["diagnostics"][0]["message"], "[redacted]");
    let args = fs::read_to_string(&fake.marker).expect("child args");
    assert!(args.contains("validate"));
    assert!(args.contains("--no-schema"));
    assert!(args.contains("--format json") || args.contains("--format  json"));
    assert!(!args.contains("--schema"));
}

#[test]
fn validation_contract_versions_native_and_composed_backend_envelopes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cache = temp.path().join("missing-cache");
    let capabilities = run_powerbi(&["capabilities", "--json"], &cache, None, None);
    assert!(capabilities.status.success());
    let capabilities = stdout_json(&capabilities);
    let validate = capabilities["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|command| command["path"] == "validate")
        .expect("validate contract");
    assert_eq!(validate["outputSchema"], "validateResult.v1");
    assert_eq!(validate["outputSchemas"]["native"], "validateResult.v1");
    assert_eq!(
        validate["outputSchemas"]["microsoft-report"],
        "powerbi-cli.validate.microsoft-report.v1"
    );
    assert_eq!(
        validate["outputSchemas"]["all"],
        "powerbi-cli.validate.all.v1"
    );

    let project = write_minimal_project(temp.path(), false);
    let fake = FakeToolchain::new(temp.path(), &success_payload(), 0, FakeMode::Normal);
    let all = run_powerbi(
        &[
            "validate",
            project.to_str().expect("project"),
            "--backend",
            "all",
            "--json",
        ],
        &fake.cache,
        Some(&fake.bin),
        None,
    );
    assert_eq!(all.status.code(), Some(10));
    assert_eq!(stdout_json(&all)["schema"], "powerbi-cli.validate.all.v1");
}

#[test]
fn ambiguous_report_artifacts_fail_before_the_official_child_launches() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = write_minimal_project(temp.path(), true);
    let fake = FakeToolchain::new(temp.path(), &success_payload(), 0, FakeMode::Normal);
    let output = run_powerbi(
        &[
            "validate",
            project.to_str().expect("project"),
            "--backend",
            "microsoft-report",
            "--json",
        ],
        &fake.cache,
        Some(&fake.bin),
        None,
    );

    assert_eq!(output.status.code(), Some(10));
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "validation_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("exactly one")
    );
    assert!(!fake.marker.exists());
}

#[test]
fn malformed_and_oversized_vendor_json_fail_closed_with_protocol_code() {
    for (name, output_text) in [
        ("malformed", "{not-json".to_string()),
        (
            "oversized",
            format!("{{\"padding\":\"{}\"}}", "x".repeat(70_000)),
        ),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let project = write_minimal_project(temp.path(), false);
        let fake = FakeToolchain::new_text(temp.path(), &output_text, 0, FakeMode::Normal);
        let output = run_powerbi(
            &[
                "validate",
                project.to_str().expect("project"),
                "--backend",
                "microsoft-report",
                "--json",
            ],
            &fake.cache,
            Some(&fake.bin),
            None,
        );
        assert_eq!(output.status.code(), Some(40), "case {name}");
        let error = stderr_json(&output);
        assert_eq!(error["error"]["code"], "protocol_failed", "case {name}");
        assert!(
            error["error"]["message"]
                .as_str()
                .expect("message")
                .contains("stdoutSha256"),
            "case {name}: {error}"
        );
    }
}

#[test]
fn vendor_error_envelope_is_bounded_and_redacted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = write_minimal_project(temp.path(), false);
    let fake = FakeToolchain::new(
        temp.path(),
        &json!({
            "error": {
                "code": "REPORT_NEUTRAL_FAILURE",
                "message": "password=super-secret",
                "retryable": false
            }
        }),
        1,
        FakeMode::Normal,
    );
    let output = run_powerbi(
        &[
            "validate",
            project.to_str().expect("project"),
            "--backend",
            "microsoft-report",
            "--json",
        ],
        &fake.cache,
        Some(&fake.bin),
        None,
    );

    assert_eq!(output.status.code(), Some(40));
    assert!(output.stdout.is_empty());
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "backend_failed");
    let message = error["error"]["message"].as_str().expect("message");
    assert!(message.contains("REPORT_NEUTRAL_FAILURE"));
    assert!(message.contains("[redacted]"));
    assert!(!message.contains("super-secret"));
}

#[test]
fn official_timeout_is_bounded_and_fails_the_combined_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = write_minimal_project(temp.path(), false);
    let fake = FakeToolchain::new(temp.path(), &success_payload(), 0, FakeMode::Sleep);
    let output = run_powerbi(
        &[
            "validate",
            project.to_str().expect("project"),
            "--backend",
            "microsoft-report",
            "--json",
        ],
        &fake.cache,
        Some(&fake.bin),
        Some(50),
    );
    assert_eq!(output.status.code(), Some(40));
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "backend_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("reaped")
    );

    let missing_cache = temp.path().join("missing-cache");
    let all = run_powerbi(
        &[
            "validate",
            project.to_str().expect("project"),
            "--backend",
            "all",
            "--json",
        ],
        &missing_cache,
        None,
        None,
    );
    assert_eq!(all.status.code(), Some(30));
    assert_eq!(stderr_json(&all)["error"]["code"], "dependency_unavailable");
    assert!(all.stdout.is_empty());
}

fn success_payload() -> Value {
    json!({
        "data": {
            "result": "succeeded",
            "errorCount": 0,
            "warningCount": 0,
            "reportPath": "ignored"
        }
    })
}

fn write_minimal_project(root: &Path, ambiguous: bool) -> PathBuf {
    let project = root.join("project");
    let report = project.join("Neutral.Report");
    let semantic = project.join("Neutral.SemanticModel");
    fs::create_dir_all(report.join("definition")).expect("report definition");
    fs::create_dir_all(semantic.join("definition")).expect("semantic definition");
    let mut artifacts = vec![json!({"report": {"path": "Neutral.Report"}})];
    if ambiguous {
        artifacts.push(json!({"report": {"path": "Other.Report"}}));
    }
    fs::write(
        project.join("Neutral.pbip"),
        serde_json::to_vec(&json!({"artifacts": artifacts})).expect("pbip"),
    )
    .expect("write pbip");
    fs::write(
        report.join("definition.pbir"),
        serde_json::to_vec(&json!({
            "datasetReference": {"byPath": {"path": "../Neutral.SemanticModel"}}
        }))
        .expect("pbir"),
    )
    .expect("write pbir");
    project
}

#[derive(Clone, Copy)]
enum FakeMode {
    Normal,
    Sleep,
}

struct FakeToolchain {
    cache: PathBuf,
    bin: PathBuf,
    marker: PathBuf,
}

impl FakeToolchain {
    fn new(root: &Path, payload: &Value, exit_code: i32, mode: FakeMode) -> Self {
        Self::new_text(
            root,
            &serde_json::to_string(payload).expect("payload"),
            exit_code,
            mode,
        )
    }

    fn new_text(root: &Path, stdout: &str, exit_code: i32, mode: FakeMode) -> Self {
        let cache = root.join("cache");
        let artifact = cache
            .join("artifacts")
            .join("microsoft-powerbi-2026-07-17-v2");
        let package = artifact
            .join("node_modules")
            .join("@microsoft")
            .join("powerbi-report-authoring-cli");
        fs::create_dir_all(package.join("dist")).expect("fake package");
        fs::write(
            package.join("package.json"),
            serde_json::to_vec(&json!({
                "name": "@microsoft/powerbi-report-authoring-cli",
                "version": "0.1.4"
            }))
            .expect("manifest"),
        )
        .expect("write manifest");
        fs::write(
            package.join("dist/cli.js"),
            "// exact fake child entrypoint\n",
        )
        .expect("write entrypoint");

        let bin = root.join("fake-bin");
        fs::create_dir_all(&bin).expect("fake bin");
        let output = root.join("vendor-stdout.json");
        let marker = root.join("child-args.txt");
        fs::write(&output, stdout).expect("vendor output");
        write_fake_node(&bin, &output, &marker, exit_code, mode);

        let receipt = json!({
            "schema": "powerbi-cli.microsoft-integrations-active.v1",
            "lockId": "microsoft-powerbi-2026-07-17-v2",
            "lockFingerprint": integration_fingerprint(),
            "artifactDir": artifact,
            "treeSha256": tree_sha256(&artifact),
            "installedAtUnixSeconds": 1,
            "nodeVersion": "v22.0.0",
            "components": {"report-authoring": "0.1.4"}
        });
        fs::create_dir_all(&cache).expect("cache");
        fs::write(
            cache.join("active.json"),
            serde_json::to_vec_pretty(&receipt).expect("receipt"),
        )
        .expect("active receipt");
        Self { cache, bin, marker }
    }
}

fn integration_fingerprint() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut digest = Sha256::new();
    digest
        .update(fs::read(root.join("integrations/microsoft/integration-lock.json")).expect("lock"));
    digest.update([0]);
    digest
        .update(fs::read(root.join("integrations/microsoft/package-lock.json")).expect("npm lock"));
    format!("sha256:{}", hex_digest(&digest.finalize()))
}

fn tree_sha256(root: &Path) -> String {
    let mut files = WalkDir::new(root)
        .into_iter()
        .map(|entry| entry.expect("walk fake cache"))
        .filter(|entry| entry.file_type().is_file())
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| normalized_relative(root, entry.path()));
    let mut digest = Sha256::new();
    for entry in files {
        digest.update(normalized_relative(root, entry.path()).as_bytes());
        digest.update([0]);
        digest.update(b"file\0");
        digest.update(fs::read(entry.path()).expect("cache file"));
        digest.update([0]);
    }
    format!("sha256:{}", hex_digest(&digest.finalize()))
}

fn normalized_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("relative")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(windows)]
fn write_fake_node(bin: &Path, output: &Path, marker: &Path, exit_code: i32, mode: FakeMode) {
    let sleep = match mode {
        FakeMode::Normal => String::new(),
        FakeMode::Sleep => "%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe -NoProfile -Command \"Start-Sleep -Seconds 2\"\r\n".to_string(),
    };
    fs::write(
        bin.join("node.cmd"),
        format!(
            "@echo off\r\necho %*>\"{}\"\r\n{}type \"{}\"\r\nexit /b {}\r\n",
            marker.display(),
            sleep,
            output.display(),
            exit_code
        ),
    )
    .expect("fake node");
}

#[cfg(not(windows))]
fn write_fake_node(bin: &Path, output: &Path, marker: &Path, exit_code: i32, mode: FakeMode) {
    use std::os::unix::fs::PermissionsExt;
    let sleep = match mode {
        FakeMode::Normal => String::new(),
        FakeMode::Sleep => "/bin/sleep 2\n".to_string(),
    };
    let node = bin.join("node");
    fs::write(
        &node,
        format!(
            "#!/bin/sh\nprintf '%s' \"$*\" > '{}'\n{}/bin/cat '{}'\nexit {}\n",
            marker.display(),
            sleep,
            output.display(),
            exit_code
        ),
    )
    .expect("fake node");
    fs::set_permissions(&node, fs::Permissions::from_mode(0o755)).expect("node executable");
}
