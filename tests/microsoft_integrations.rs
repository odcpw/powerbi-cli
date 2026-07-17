use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CACHE_ENV: &str = "POWERBI_CLI_MICROSOFT_CACHE_DIR";

fn run_powerbi(args: &[&str], cache: &Path, path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"));
    command.args(args).env(CACHE_ENV, cache);
    if let Some(path) = path {
        command.env("PATH", path);
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
fn committed_microsoft_graph_has_exact_top_level_and_platform_pins() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let integration: Value = serde_json::from_slice(
        &fs::read(root.join("integrations/microsoft/integration-lock.json"))
            .expect("integration lock"),
    )
    .expect("integration lock JSON");
    let npm: Value = serde_json::from_slice(
        &fs::read(root.join("integrations/microsoft/package-lock.json")).expect("npm lock"),
    )
    .expect("npm lock JSON");

    assert_eq!(
        integration["schema"],
        "powerbi-cli.microsoft-integrations-lock.v1"
    );
    assert_eq!(npm["lockfileVersion"], 3);
    let components = integration["components"].as_array().expect("components");
    assert_eq!(components.len(), 3);
    for component in components {
        let package = component["package"].as_str().expect("package");
        let entry = &npm["packages"][format!("node_modules/{package}")];
        assert_eq!(entry["version"], component["version"]);
        assert_eq!(entry["license"], component["license"]);
        assert_eq!(entry["integrity"], component["integrity"]);
        if component["id"] == "modeling-mcp" {
            assert_eq!(component["protocolVersion"], "2025-06-18");
            assert_eq!(component["serverName"], "powerbi-modeling-mcp");
            assert_eq!(component["serverVersion"], "0.5.0.0");
            assert_eq!(component["toolsCount"], 21);
            assert_eq!(
                component["toolsListSha256"],
                "sha256:f5e3e88621f210e798472278fccba9cd398522c2a18e7a08ee617a2ec9d6f78c"
            );
        }
    }
    for artifact in integration["platformArtifacts"]
        .as_array()
        .expect("platform artifacts")
    {
        let package = artifact["package"].as_str().expect("package");
        let entry = &npm["packages"][format!("node_modules/{package}")];
        assert_eq!(entry["version"], artifact["version"]);
        assert_eq!(entry["license"], artifact["license"]);
        assert_eq!(entry["integrity"], artifact["integrity"]);
    }
    for (path, package) in npm["packages"].as_object().expect("packages") {
        if path.is_empty() {
            continue;
        }
        assert!(package["version"].is_string(), "missing version: {path}");
        assert!(
            package["integrity"]
                .as_str()
                .is_some_and(|value| value.starts_with("sha512-")),
            "missing exact integrity: {path}"
        );
    }
}

#[test]
fn shallow_status_and_doctor_launch_no_child_and_preserve_doctor_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).expect("bin dir");
    let marker = temp.path().join("node-ran");
    write_fake_node(&bin, &marker);
    let cache = temp.path().join("cache");

    let status = run_powerbi(&["integrations", "status", "--json"], &cache, Some(&bin));
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_json = stdout_json(&status);
    assert_eq!(status_json["mode"], "shallow");
    assert_eq!(status_json["childProcessesLaunched"], 0);
    assert!(
        !marker.exists(),
        "shallow status executed the fake Node child"
    );

    let doctor = run_powerbi(&["doctor", "--json"], &cache, Some(&bin));
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let doctor_json = stdout_json(&doctor);
    let ids = doctor_json["checks"]
        .as_array()
        .expect("doctor checks")
        .iter()
        .map(|check| check["id"].as_str().expect("check id"))
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "platform",
            "powerBiDesktop",
            "desktopProofLevel",
            "offlineSafety",
            "microsoftIntegrations"
        ]
    );
    assert_eq!(
        doctor_json["microsoftIntegrations"]["childProcessesLaunched"],
        0
    );
    assert!(!marker.exists(), "doctor executed the fake Node child");
}

#[test]
fn shallow_report_readiness_requires_a_resolved_node_executable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cache = temp.path().join("cache");
    let artifact = cache
        .join("artifacts")
        .join("microsoft-powerbi-2026-07-17-v2");
    let entrypoint = artifact
        .join("node_modules")
        .join("@microsoft")
        .join("powerbi-report-authoring-cli")
        .join("dist")
        .join("cli.js");
    fs::create_dir_all(entrypoint.parent().expect("entrypoint parent")).expect("artifact");
    fs::write(&entrypoint, "// exact entrypoint placeholder").expect("entrypoint");
    let mut lock_digest = Sha256::new();
    lock_digest.update(include_bytes!(
        "../integrations/microsoft/integration-lock.json"
    ));
    lock_digest.update([0]);
    lock_digest.update(include_bytes!(
        "../integrations/microsoft/package-lock.json"
    ));
    let fingerprint = format!("sha256:{:x}", lock_digest.finalize());
    fs::create_dir_all(&cache).expect("cache");
    fs::write(
        cache.join("active.json"),
        serde_json::to_vec_pretty(&json!({
            "schema": "powerbi-cli.microsoft-integrations-active.v1",
            "lockId": "microsoft-powerbi-2026-07-17-v2",
            "lockFingerprint": fingerprint,
            "artifactDir": artifact,
            "treeSha256": "sha256:not-read-by-shallow-status",
            "installedAtUnixSeconds": 1,
            "nodeVersion": "v22.14.0",
            "components": {"report-authoring": "0.1.4"}
        }))
        .expect("receipt"),
    )
    .expect("active receipt");
    let empty_path = temp.path().join("empty-path");
    fs::create_dir(&empty_path).expect("empty PATH");

    let status = run_powerbi(
        &[
            "integrations",
            "status",
            "--component",
            "report-authoring",
            "--json",
        ],
        &cache,
        Some(&empty_path),
    );
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status = stdout_json(&status);
    assert_eq!(status["node"]["present"], false);
    assert_eq!(status["node"]["meetsFloor"], false);
    assert_eq!(status["ready"], false);
    assert_eq!(status["components"][0]["ready"], false);
    assert_eq!(status["components"][0]["state"], "node-unavailable");
}

#[cfg(not(windows))]
#[test]
fn explicitly_selected_unsupported_desktop_bridge_is_not_ready() {
    let temp = tempfile::tempdir().expect("tempdir");
    let status = run_powerbi(
        &[
            "integrations",
            "status",
            "--component",
            "desktop-bridge",
            "--json",
        ],
        &temp.path().join("cache"),
        None,
    );
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status = stdout_json(&status);
    assert_eq!(status["components"][0]["supported"], false);
    assert_eq!(status["components"][0]["ready"], false);
    assert_eq!(status["ready"], false);
}

#[test]
#[ignore = "requires integrations install --allow-network for the exact Microsoft graph"]
fn exact_install_has_supported_ready_components_and_final_receipt_identity() {
    let report = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args([
            "integrations",
            "status",
            "--component",
            "report-authoring",
            "--deep",
            "--json",
        ])
        .output()
        .expect("report status");
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report = stdout_json(&report);
    assert_eq!(report["components"][0]["supported"], true);
    assert_eq!(report["components"][0]["ready"], true);
    assert_eq!(report["ready"], true);

    let modeling = Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args([
            "integrations",
            "status",
            "--component",
            "modeling-mcp",
            "--deep",
            "--json",
        ])
        .output()
        .expect("modeling status");
    assert!(
        modeling.status.success(),
        "{}",
        String::from_utf8_lossy(&modeling.stderr)
    );
    let modeling = stdout_json(&modeling);
    if modeling["components"][0]["supported"] == true {
        assert_eq!(modeling["components"][0]["ready"], true);
        assert_eq!(modeling["ready"], true);
    }

    let cache = PathBuf::from(report["cache"]["root"].as_str().expect("cache root"));
    let active: Value =
        serde_json::from_slice(&fs::read(cache.join("active.json")).expect("active receipt"))
            .expect("active JSON");
    let artifact = PathBuf::from(active["artifactDir"].as_str().expect("artifactDir"));
    let install: Value = serde_json::from_slice(
        &fs::read(artifact.join("powerbi-cli-install.json")).expect("install receipt"),
    )
    .expect("install JSON");
    assert_eq!(install["artifactDir"], active["artifactDir"]);
    assert_eq!(
        PathBuf::from(install["artifactDir"].as_str().unwrap()),
        artifact
    );
}

#[test]
fn install_without_network_consent_fails_before_cache_or_child_access() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).expect("bin dir");
    let marker = temp.path().join("node-ran");
    write_fake_node(&bin, &marker);
    let cache = temp.path().join("cache");

    let output = run_powerbi(&["integrations", "install", "--json"], &cache, Some(&bin));
    assert_eq!(output.status.code(), Some(2));
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "invalid_args");
    assert!(
        error["error"]["message"]
            .as_str()
            .expect("message")
            .contains("--allow-network")
    );
    assert!(!cache.exists());
    assert!(!marker.exists());
}

#[test]
fn failed_explicit_install_keeps_active_receipt_byte_identical_and_redacts_stderr() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).expect("bin dir");
    write_fake_node(&bin, &temp.path().join("node-ran"));
    write_failing_npm(&bin);
    let cache = temp.path().join("cache");
    fs::create_dir_all(&cache).expect("cache");
    let active = cache.join("active.json");
    let prior = serde_json::to_vec_pretty(&json!({
        "schema": "powerbi-cli.microsoft-integrations-active.v1",
        "lockId": "prior-active",
        "lockFingerprint": "sha256:prior",
        "artifactDir": temp.path().join("prior-artifact"),
        "treeSha256": "sha256:prior",
        "installedAtUnixSeconds": 1,
        "nodeVersion": "v22.0.0",
        "components": {}
    }))
    .expect("prior receipt");
    fs::write(&active, &prior).expect("write prior receipt");

    let output = run_powerbi(
        &["integrations", "install", "--allow-network", "--json"],
        &cache,
        Some(&bin),
    );
    assert_eq!(output.status.code(), Some(40));
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "backend_failed");
    let serialized = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(!serialized.contains("super-secret"));
    assert!(serialized.contains("[redacted]"));
    assert_eq!(fs::read(&active).expect("active receipt"), prior);
}

#[cfg(windows)]
fn write_fake_node(bin: &Path, marker: &Path) {
    fs::write(
        bin.join("node.cmd"),
        format!(
            "@echo off\r\necho ran>\"{}\"\r\necho v22.14.0\r\n",
            marker.display()
        ),
    )
    .expect("fake node");
}

#[cfg(not(windows))]
fn write_fake_node(bin: &Path, marker: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let path = bin.join("node");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf ran > '{}'\nprintf 'v22.14.0\\n'\n",
            marker.display()
        ),
    )
    .expect("fake node");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("node executable");
}

#[cfg(windows)]
fn write_failing_npm(bin: &Path) {
    fs::write(
        bin.join("npm.cmd"),
        "@echo off\r\necho password=super-secret 1>&2\r\nexit /b 17\r\n",
    )
    .expect("fake npm");
}

#[cfg(not(windows))]
fn write_failing_npm(bin: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let path = bin.join("npm");
    fs::write(
        &path,
        "#!/bin/sh\nprintf 'password=super-secret\\n' >&2\nexit 17\n",
    )
    .expect("fake npm");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("npm executable");
}
