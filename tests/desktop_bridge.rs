use serde_json::Value;
#[cfg(windows)]
use serde_json::json;
#[cfg(windows)]
use sha2::{Digest, Sha256};
#[cfg(windows)]
use std::fs;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::process::{Command, Output};
#[cfg(windows)]
use walkdir::WalkDir;

const CACHE_ENV: &str = "POWERBI_CLI_MICROSOFT_CACHE_DIR";

fn run_powerbi(args: &[&str], cache: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
        .env(CACHE_ENV, cache)
        .output()
        .expect("run powerbi-cli")
}

#[cfg(windows)]
fn run_powerbi_with_bridge_timeout(args: &[&str], cache: &Path, timeout_ms: u64) -> Output {
    Command::new(env!("CARGO_BIN_EXE_powerbi-cli"))
        .args(args)
        .env(CACHE_ENV, cache)
        .env("POWERBI_CLI_TEST_BRIDGE_TIMEOUT_MS", timeout_ms.to_string())
        .output()
        .expect("run powerbi-cli with Bridge timeout")
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout JSON")
}

fn stderr_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stderr).expect("stderr JSON")
}

#[test]
fn capabilities_and_help_catalog_all_public_desktop_bridge_commands() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cache = temp.path().join("unused-cache");
    let capabilities = run_powerbi(
        &["capabilities", "--for", "desktop bridge", "--json"],
        &cache,
    );
    assert!(capabilities.status.success());
    let value = stdout_json(&capabilities);
    let paths = value["commands"]
        .as_array()
        .expect("capability commands")
        .iter()
        .map(|command| command["path"].as_str().expect("command path"))
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            "desktop bridge status",
            "desktop bridge reload",
            "desktop bridge screenshot-page",
            "desktop bridge screenshot-all"
        ]
    );

    let help = run_powerbi(&["--json"], &cache);
    assert!(help.status.success());
    let help_value = stdout_json(&help);
    let help_paths = help_value["commands"]
        .as_array()
        .expect("help commands")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    for path in paths {
        assert!(help_paths.contains(&path), "help omitted {path}");
    }
}

#[cfg(not(windows))]
#[test]
fn desktop_bridge_is_honestly_unavailable_off_windows_before_cache_access() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cache = temp.path().join("missing-cache");
    let output = run_powerbi(&["desktop", "bridge", "status", "--json"], &cache);
    assert_eq!(output.status.code(), Some(2));
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "unsupported_feature");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Windows-only")
    );
    assert!(!cache.exists());
}

#[cfg(windows)]
#[test]
fn status_and_clean_reload_use_exact_pid_without_claiming_ownership() {
    let fixture = BridgeFixture::new(41_101, false, false);

    let status = run_powerbi(
        &[
            "desktop",
            "bridge",
            "status",
            "--pid",
            &fixture.pid.to_string(),
            "--json",
        ],
        &fixture.cache,
    );
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_json = stdout_json(&status);
    assert_eq!(
        status_json["schema"],
        "powerbi-cli.desktop.bridge.status.v1"
    );
    assert_eq!(status_json["instances"][0]["pid"], fixture.pid);
    assert_eq!(status_json["backend"]["version"], "0.1.2");
    assert_eq!(status_json["proof"]["level"], "unit-smoke");
    assert_eq!(status_json["proof"]["claimedRefresh"], false);

    let reload = run_powerbi(
        &[
            "desktop",
            "bridge",
            "reload",
            "--project",
            &fixture.pbip_string(),
            "--pid",
            &fixture.pid.to_string(),
            "--json",
        ],
        &fixture.cache,
    );
    assert!(
        reload.status.success(),
        "{}",
        String::from_utf8_lossy(&reload.stderr)
    );
    let reload_json = stdout_json(&reload);
    assert_eq!(reload_json["ownership"]["owned"], false);
    assert_eq!(reload_json["ownership"]["cleanupEligible"], false);
    assert_eq!(reload_json["desktop"]["desktopVersion"], "2.999.0");
    assert_eq!(reload_json["changes"][0]["reloadModelDefinition"], false);
    assert_eq!(reload_json["proof"]["claimedSaveReopen"], false);
    assert!(fixture.actions().iter().any(|action| action == "reload"));
}

#[cfg(windows)]
#[test]
fn dirty_reload_and_wrong_project_are_refused_before_mutation() {
    let dirty = BridgeFixture::new(41_102, true, false);
    let status = run_powerbi(&["desktop", "bridge", "status", "--json"], &dirty.cache);
    assert!(status.status.success());
    assert_eq!(stdout_json(&status)["proof"]["hasUnsavedChanges"], true);
    let reload = run_powerbi(
        &[
            "desktop",
            "bridge",
            "reload",
            "--project",
            &dirty.pbip_string(),
            "--pid",
            &dirty.pid.to_string(),
            "--json",
        ],
        &dirty.cache,
    );
    assert_eq!(reload.status.code(), Some(10));
    assert_eq!(stderr_json(&reload)["error"]["code"], "validation_failed");
    assert!(!dirty.actions().iter().any(|action| action == "reload"));

    let exact = BridgeFixture::new(41_103, false, false);
    let other = create_project(exact.temp.path().join("other"), "Other");
    let wrong = run_powerbi(
        &[
            "desktop",
            "bridge",
            "reload",
            "--project",
            &other.to_string_lossy(),
            "--pid",
            &exact.pid.to_string(),
            "--json",
        ],
        &exact.cache,
    );
    assert_eq!(wrong.status.code(), Some(10));
    assert_eq!(stderr_json(&wrong)["error"]["code"], "validation_failed");
    assert!(!exact.actions().iter().any(|action| action == "reload"));
}

#[cfg(windows)]
#[test]
fn dirty_screenshots_are_hashed_bounded_diagnostics_with_exact_inventory() {
    let fixture = BridgeFixture::new(41_104, true, false);
    let one = fixture.temp.path().join("page-one.png");
    let page = run_powerbi(
        &[
            "desktop",
            "bridge",
            "screenshot-page",
            "--project",
            &fixture.pbip_string(),
            "--pid",
            &fixture.pid.to_string(),
            "--page",
            "PageOne",
            "--out",
            &one.to_string_lossy(),
            "--json",
        ],
        &fixture.cache,
    );
    assert!(
        page.status.success(),
        "{}",
        String::from_utf8_lossy(&page.stderr)
    );
    let page_json = stdout_json(&page);
    assert_eq!(page_json["hasUnsavedChanges"], true);
    assert!(page_json.get("representsOnDiskWorkflowOutput").is_none());
    assert_eq!(page_json["screenshot"]["width"], 1);
    assert_eq!(page_json["screenshot"]["height"], 1);
    assert!(
        page_json["screenshot"]["sha256"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    assert_eq!(page_json["backend"]["readOnly"], false);
    assert_eq!(page_json["proof"]["claimedInteractionOrDrill"], false);

    let all_dir = fixture.temp.path().join("all-pages");
    let all = run_powerbi(
        &[
            "desktop",
            "bridge",
            "screenshot-all",
            "--project",
            &fixture.pbip_string(),
            "--pid",
            &fixture.pid.to_string(),
            "--out-dir",
            &all_dir.to_string_lossy(),
            "--json",
        ],
        &fixture.cache,
    );
    assert!(
        all.status.success(),
        "{}",
        String::from_utf8_lossy(&all.stderr)
    );
    let all_json = stdout_json(&all);
    assert_eq!(all_json["pageInventory"].as_array().map(Vec::len), Some(2));
    assert_eq!(all_json["screenshots"].as_array().map(Vec::len), Some(2));
    assert_eq!(all_json["screenshots"][0]["pageId"], "PageOne");
    assert_eq!(all_json["screenshots"][1]["pageId"], "PageTwo");
}

#[cfg(windows)]
#[test]
fn output_guards_inventory_mismatch_and_pid_lock_fail_closed() {
    let fixture = BridgeFixture::new(41_105, false, false);
    let inside = fixture.project_dir.join("forbidden.png");
    let guarded = run_powerbi(
        &[
            "desktop",
            "bridge",
            "screenshot-page",
            "--project",
            &fixture.pbip_string(),
            "--pid",
            &fixture.pid.to_string(),
            "--page",
            "PageOne",
            "--out",
            &inside.to_string_lossy(),
            "--json",
        ],
        &fixture.cache,
    );
    assert_eq!(guarded.status.code(), Some(10));
    assert!(!inside.exists());
    assert!(
        !fixture
            .actions()
            .iter()
            .any(|action| action == "screenshot")
    );

    let prior_file = fixture.temp.path().join("prior-evidence.png");
    fs::write(&prior_file, b"prior evidence").expect("write prior evidence file");
    let prior_file_attempt = run_screenshot_page(&fixture, &prior_file);
    assert_eq!(prior_file_attempt.status.code(), Some(10));
    assert_eq!(
        fs::read(&prior_file).expect("read prior evidence file"),
        b"prior evidence"
    );
    let prior_dir = fixture.temp.path().join("prior-evidence-dir");
    fs::create_dir(&prior_dir).expect("create prior evidence directory");
    fs::write(prior_dir.join("keep.txt"), b"keep").expect("write prior directory evidence");
    let prior_dir_attempt = run_screenshot_all(&fixture, &prior_dir);
    assert_eq!(prior_dir_attempt.status.code(), Some(10));
    assert_eq!(
        fs::read(prior_dir.join("keep.txt")).expect("read prior directory evidence"),
        b"keep"
    );

    let mismatch = BridgeFixture::new(41_106, false, true);
    let mismatch_dir = mismatch.temp.path().join("mismatch");
    let output = run_powerbi(
        &[
            "desktop",
            "bridge",
            "screenshot-all",
            "--project",
            &mismatch.pbip_string(),
            "--pid",
            &mismatch.pid.to_string(),
            "--out-dir",
            &mismatch_dir.to_string_lossy(),
            "--json",
        ],
        &mismatch.cache,
    );
    assert_eq!(output.status.code(), Some(40));
    assert_eq!(stderr_json(&output)["error"]["code"], "protocol_failed");
    assert!(
        !mismatch_dir.exists(),
        "failed screenshot-all must clean its invocation-owned directory"
    );
    mismatch.clear_fault("mismatch-inventory");
    let retry = run_powerbi(
        &[
            "desktop",
            "bridge",
            "screenshot-all",
            "--project",
            &mismatch.pbip_string(),
            "--pid",
            &mismatch.pid.to_string(),
            "--out-dir",
            &mismatch_dir.to_string_lossy(),
            "--json",
        ],
        &mismatch.cache,
    );
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(mismatch_dir.is_dir());
    assert!(mismatch.ownership_markers().is_empty());

    let empty = BridgeFixture::new(41_108, false, false);
    empty.set_fault("empty-status", "1");
    let empty_status = run_powerbi(
        &[
            "desktop",
            "bridge",
            "status",
            "--pid",
            &empty.pid.to_string(),
            "--json",
        ],
        &empty.cache,
    );
    assert_eq!(empty_status.status.code(), Some(40));
    assert_eq!(
        stderr_json(&empty_status)["error"]["code"],
        "protocol_failed"
    );

    let locked = BridgeFixture::new(41_107, false, false);
    let lock_dir = std::env::temp_dir().join("powerbi-cli-desktop-bridge-locks");
    fs::create_dir_all(&lock_dir).expect("lock dir");
    let lock = lock_dir.join(format!("pid-{}.lock", locked.pid));
    fs::write(&lock, "another-process").expect("preexisting lock");
    let output = run_powerbi(
        &[
            "desktop",
            "bridge",
            "reload",
            "--project",
            &locked.pbip_string(),
            "--pid",
            &locked.pid.to_string(),
            "--json",
        ],
        &locked.cache,
    );
    fs::remove_file(&lock).expect("remove test lock");
    assert_eq!(output.status.code(), Some(10));
    assert!(locked.actions().is_empty());
}

#[cfg(windows)]
#[test]
fn bridge_child_timeout_is_bounded_and_reaped_without_a_proof_claim() {
    let fixture = BridgeFixture::new(41_108, false, false);
    fs::write(fixture.temp.path().join("delay-status-ms"), "250").expect("write fake delay");
    let output = run_powerbi_with_bridge_timeout(
        &[
            "desktop",
            "bridge",
            "status",
            "--pid",
            &fixture.pid.to_string(),
            "--json",
        ],
        &fixture.cache,
        25,
    );
    assert_eq!(output.status.code(), Some(40));
    let error = stderr_json(&output);
    assert_eq!(error["error"]["code"], "backend_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("reaped")
    );
}

#[cfg(windows)]
#[test]
fn response_identity_mismatches_fail_closed_and_owned_outputs_are_retryable() {
    let fixture = BridgeFixture::new(41_109, false, false);

    fixture.set_fault("response-pid-offset-status", "1");
    let status = run_powerbi(
        &[
            "desktop",
            "bridge",
            "status",
            "--pid",
            &fixture.pid.to_string(),
            "--json",
        ],
        &fixture.cache,
    );
    assert_protocol_failure(&status);
    fixture.clear_fault("response-pid-offset-status");

    fixture.set_fault("response-pid-offset-manifest", "1");
    let manifest = run_reload(&fixture);
    assert_protocol_failure(&manifest);
    fixture.clear_fault("response-pid-offset-manifest");

    fixture.set_fault("response-pid-offset-reload", "1");
    let reload = run_reload(&fixture);
    assert_protocol_failure(&reload);
    fixture.clear_fault("response-pid-offset-reload");

    let page_out = fixture.temp.path().join("retry-page.png");
    fixture.set_fault("mismatch-page-id", "1");
    let page = run_screenshot_page(&fixture, &page_out);
    assert_protocol_failure(&page);
    assert!(
        !page_out.exists(),
        "pageId mismatch must clean its invocation-owned file"
    );
    assert!(fixture.ownership_markers().is_empty());
    fixture.clear_fault("mismatch-page-id");
    let page_retry = run_screenshot_page(&fixture, &page_out);
    assert!(
        page_retry.status.success(),
        "{}",
        String::from_utf8_lossy(&page_retry.stderr)
    );
    assert!(page_out.is_file());
    assert!(fixture.ownership_markers().is_empty());

    let pid_page_out = fixture.temp.path().join("pid-mismatch-page.png");
    fixture.set_fault("response-pid-offset-screenshot", "1");
    let pid_page = run_screenshot_page(&fixture, &pid_page_out);
    assert_protocol_failure(&pid_page);
    assert!(!pid_page_out.exists());
    fixture.clear_fault("response-pid-offset-screenshot");

    let all_out = fixture.temp.path().join("pid-mismatch-all");
    fixture.set_fault("response-pid-offset-screenshot-all", "1");
    let all = run_screenshot_all(&fixture, &all_out);
    assert_protocol_failure(&all);
    assert!(
        !all_out.exists(),
        "PID mismatch must clean its invocation-owned directory"
    );
    fixture.clear_fault("response-pid-offset-screenshot-all");
    let all_retry = run_screenshot_all(&fixture, &all_out);
    assert!(
        all_retry.status.success(),
        "{}",
        String::from_utf8_lossy(&all_retry.stderr)
    );
    assert!(all_out.is_dir());
    assert!(fixture.ownership_markers().is_empty());
}

#[cfg(windows)]
fn assert_protocol_failure(output: &Output) {
    assert_eq!(output.status.code(), Some(40));
    assert_eq!(stderr_json(output)["error"]["code"], "protocol_failed");
}

#[cfg(windows)]
fn run_reload(fixture: &BridgeFixture) -> Output {
    run_powerbi(
        &[
            "desktop",
            "bridge",
            "reload",
            "--project",
            &fixture.pbip_string(),
            "--pid",
            &fixture.pid.to_string(),
            "--json",
        ],
        &fixture.cache,
    )
}

#[cfg(windows)]
fn run_screenshot_page(fixture: &BridgeFixture, out: &Path) -> Output {
    run_powerbi(
        &[
            "desktop",
            "bridge",
            "screenshot-page",
            "--project",
            &fixture.pbip_string(),
            "--pid",
            &fixture.pid.to_string(),
            "--page",
            "PageOne",
            "--out",
            &out.to_string_lossy(),
            "--json",
        ],
        &fixture.cache,
    )
}

#[cfg(windows)]
fn run_screenshot_all(fixture: &BridgeFixture, out: &Path) -> Output {
    run_powerbi(
        &[
            "desktop",
            "bridge",
            "screenshot-all",
            "--project",
            &fixture.pbip_string(),
            "--pid",
            &fixture.pid.to_string(),
            "--out-dir",
            &out.to_string_lossy(),
            "--json",
        ],
        &fixture.cache,
    )
}

#[cfg(windows)]
struct BridgeFixture {
    temp: tempfile::TempDir,
    cache: PathBuf,
    project_dir: PathBuf,
    pbip: PathBuf,
    action_log: PathBuf,
    pid: u32,
}

#[cfg(windows)]
impl BridgeFixture {
    fn new(pid: u32, dirty: bool, inventory_mismatch: bool) -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_dir = temp.path().join("project");
        let pbip = create_project(project_dir.clone(), "SyntheticBridge");
        let canonical_pbip = fs::canonicalize(&pbip).expect("canonical PBIP");
        let action_log = temp.path().join("bridge-actions.log");
        let cache = temp.path().join("cache");
        write_fake_cache(&cache, &canonical_pbip, &action_log, pid, dirty);
        let fixture = Self {
            temp,
            cache,
            project_dir,
            pbip,
            action_log,
            pid,
        };
        if inventory_mismatch {
            fixture.set_fault("mismatch-inventory", "1");
        }
        fixture
    }

    fn pbip_string(&self) -> String {
        self.pbip.to_string_lossy().into_owned()
    }

    fn actions(&self) -> Vec<String> {
        fs::read_to_string(&self.action_log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn set_fault(&self, name: &str, value: &str) {
        fs::write(self.temp.path().join(name), value).expect("write fake Bridge fault");
    }

    fn clear_fault(&self, name: &str) {
        fs::remove_file(self.temp.path().join(name)).expect("clear fake Bridge fault");
    }

    fn ownership_markers(&self) -> Vec<PathBuf> {
        WalkDir::new(self.temp.path())
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| {
                        name.contains("powerbi-cli-owner")
                            || name == ".powerbi-cli-output-owner.marker"
                    })
            })
            .collect()
    }
}

#[cfg(windows)]
fn create_project(root: PathBuf, name: &str) -> PathBuf {
    fs::create_dir_all(root.join(format!("{name}.Report"))).expect("report dir");
    fs::create_dir_all(root.join(format!("{name}.SemanticModel"))).expect("model dir");
    let pbip = root.join(format!("{name}.pbip"));
    fs::write(
        &pbip,
        serde_json::to_vec_pretty(&json!({
            "artifacts": [{"report": {"path": format!("{name}.Report")}}]
        }))
        .expect("PBIP JSON"),
    )
    .expect("write PBIP");
    fs::write(
        root.join(format!("{name}.Report/definition.pbir")),
        serde_json::to_vec_pretty(&json!({
            "datasetReference": {"byPath": {"path": format!("../{name}.SemanticModel")}}
        }))
        .expect("PBIR JSON"),
    )
    .expect("write definition.pbir");
    pbip
}

#[cfg(windows)]
fn write_fake_cache(cache: &Path, current_pbip: &Path, action_log: &Path, pid: u32, dirty: bool) {
    let lock_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("integrations/microsoft/integration-lock.json");
    let lock_bytes = fs::read(&lock_path).expect("integration lock");
    let lock: Value = serde_json::from_slice(&lock_bytes).expect("integration lock JSON");
    let lock_id = lock["lockId"].as_str().expect("lock id");
    let artifact = cache.join("artifacts").join(lock_id);
    let package = artifact.join("node_modules/@microsoft/powerbi-desktop-bridge-cli");
    fs::create_dir_all(package.join("dist")).expect("fake package dir");
    fs::write(
        package.join("package.json"),
        serde_json::to_vec_pretty(&json!({
            "name": "@microsoft/powerbi-desktop-bridge-cli",
            "version": "0.1.2",
            "type": "commonjs"
        }))
        .expect("package JSON"),
    )
    .expect("write package JSON");
    let script = FAKE_BRIDGE
        .replace(
            "__CURRENT_FILE__",
            &serde_json::to_string(&current_pbip.to_string_lossy()).expect("current path JSON"),
        )
        .replace(
            "__ACTION_LOG__",
            &serde_json::to_string(&action_log.to_string_lossy()).expect("action log JSON"),
        )
        .replace("__PID__", &pid.to_string())
        .replace("__DIRTY__", if dirty { "true" } else { "false" });
    fs::write(package.join("dist/index.js"), script).expect("fake bridge script");

    let tree_sha256 = tree_sha256(&artifact);
    let package_lock_bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("integrations/microsoft/package-lock.json"),
    )
    .expect("Microsoft package lock");
    let mut lock_digest = Sha256::new();
    lock_digest.update(&lock_bytes);
    lock_digest.update([0]);
    lock_digest.update(&package_lock_bytes);
    let lock_fingerprint = format!("sha256:{:x}", lock_digest.finalize());
    fs::create_dir_all(cache).expect("cache root");
    fs::write(
        cache.join("active.json"),
        serde_json::to_vec_pretty(&json!({
            "schema": "powerbi-cli.microsoft-integrations-active.v1",
            "lockId": lock_id,
            "lockFingerprint": lock_fingerprint,
            "artifactDir": artifact,
            "treeSha256": tree_sha256,
            "installedAtUnixSeconds": 1,
            "nodeVersion": "v22.0.0",
            "components": {"desktop-bridge": "0.1.2"}
        }))
        .expect("active JSON"),
    )
    .expect("write active receipt");
}

#[cfg(windows)]
fn tree_sha256(root: &Path) -> String {
    let mut entries = WalkDir::new(root)
        .into_iter()
        .map(|entry| entry.expect("walk fake cache"))
        .filter(|entry| entry.file_type().is_file())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| relative(root, entry.path()));
    let mut digest = Sha256::new();
    for entry in entries {
        digest.update(relative(root, entry.path()).as_bytes());
        digest.update([0]);
        digest.update(b"file\0");
        digest.update(fs::read(entry.path()).expect("read fake cache file"));
        digest.update([0]);
    }
    format!("sha256:{:x}", digest.finalize())
}

#[cfg(windows)]
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("relative fake cache path")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(windows)]
const FAKE_BRIDGE: &str = r#"
const fs = require('fs');
const path = require('path');
const currentFilePath = __CURRENT_FILE__;
const actionLog = __ACTION_LOG__;
const pid = __PID__;
const dirty = __DIRTY__;
const args = process.argv.slice(2);
const action = args.shift();
fs.appendFileSync(actionLog, `${action}\n`);
const delayPath = path.join(path.dirname(actionLog), `delay-${action}-ms`);
if (fs.existsSync(delayPath)) {
  const delayMs = Number(fs.readFileSync(delayPath, 'utf8'));
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, delayMs);
}
const option = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const faultPath = (name) => path.join(path.dirname(actionLog), name);
const responsePid = (name) => {
  const file = faultPath(`response-pid-offset-${name}`);
  return fs.existsSync(file) ? pid + Number(fs.readFileSync(file, 'utf8')) : pid;
};
const pages = [
  { id: 'PageOne', displayName: 'Page One' },
  { id: 'PageTwo', displayName: 'Page Two' }
];
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=', 'base64');
if (action === 'status') {
  const instances = fs.existsSync(faultPath('empty-status'))
    ? []
    : [{ pid: responsePid('status'), bridgeStatus: 'connected', currentFilePath, hasUnsavedChanges: dirty, pages }];
  console.log(JSON.stringify({
    status: 'ready',
    instances
  }));
} else if (action === 'manifest') {
  console.log(JSON.stringify({
    status: 'ok',
    pid: responsePid('manifest'),
    manifest: {
      version: '2.999.0',
      methods: [
        { name: 'application.state.get/v1' },
        { name: 'file.reload/v1' },
        { name: 'report.snapshot.capture/v1' }
      ]
    }
  }));
} else if (action === 'reload') {
  console.log(JSON.stringify({ status: 'ok', pid: responsePid('reload'), result: { success: true } }));
} else if (action === 'screenshot') {
  const pageId = args[0];
  const resultPageId = fs.existsSync(faultPath('mismatch-page-id')) ? 'WrongPage' : pageId;
  const outputPath = option('--output');
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, png);
  console.log(JSON.stringify({
    status: 'ok', pid: responsePid('screenshot'), pageId: resultPageId, pageDisplayName: pageId === 'PageOne' ? 'Page One' : 'Page Two',
    mimeType: 'image/png', outputPath: path.resolve(outputPath)
  }));
} else if (action === 'screenshot-all') {
  const outputDir = option('--output-dir');
  fs.mkdirSync(outputDir, { recursive: true });
  const selected = fs.existsSync(faultPath('mismatch-inventory')) ? pages.slice(0, 1) : pages;
  const screenshots = selected.map((pageInfo) => {
    const outputPath = path.resolve(outputDir, `${pageInfo.id}.png`);
    fs.writeFileSync(outputPath, png);
    return { ...pageInfo, pageId: pageInfo.id, pageDisplayName: pageInfo.displayName, outputPath };
  });
  console.log(JSON.stringify({ status: 'ok', pid: responsePid('screenshot-all'), screenshots, failures: [] }));
} else if (action === 'open') {
  console.log(JSON.stringify({ status: 'launched', pid, bridgeStatus: 'connected' }));
} else {
  console.log(JSON.stringify({ status: 'error', error: { code: 'UNKNOWN_ACTION' } }));
  process.exitCode = 1;
}
"#;
