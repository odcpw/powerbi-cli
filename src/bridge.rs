use crate::microsoft::{
    BoundedChildOutput, MicrosoftComponent, minimal_child_command, resolve_installed_component,
    run_bounded,
};
use crate::{
    CliError, CliResult, EXIT_ORACLE_FAILED, EXIT_SUCCESS, ResolvedProject, canonical_display,
    resolve_project,
};
use file_id::FileId;
#[cfg(windows)]
use file_id::get_file_id;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(75);
const SCREENSHOT_ALL_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_SCREENSHOT_COUNT: usize = 50;
const MAX_SCREENSHOT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SCREENSHOT_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
static OUTPUT_GUARD_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BridgePage {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) display_name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BridgeInstance {
    pub(crate) pid: u32,
    pub(crate) bridge_status: String,
    #[serde(default)]
    pub(crate) current_file_path: String,
    #[serde(default)]
    pub(crate) has_unsaved_changes: bool,
    #[serde(default)]
    pub(crate) pages: Vec<BridgePage>,
}

#[derive(Debug)]
struct VendorExecution {
    value: Value,
    output: BoundedChildOutput,
    version: String,
}

#[derive(Debug)]
struct ExactInstance {
    instance: BridgeInstance,
    canonical_project: PathBuf,
}

#[derive(Debug)]
struct GuardedPng {
    path: PathBuf,
    width: u32,
    height: u32,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Default)]
struct BridgeArgs {
    project: Option<PathBuf>,
    pid: Option<u32>,
    page: Option<String>,
    out: Option<PathBuf>,
    out_dir: Option<PathBuf>,
}

pub(crate) fn desktop_bridge_command(args: &[String]) -> CliResult<Value> {
    ensure_bridge_platform()?;
    let Some((action, rest)) = args.split_first() else {
        return Err(CliError::invalid_args(
            "desktop bridge requires status, reload, screenshot-page, or screenshot-all",
        )
        .with_hint("Desktop Bridge commands require the pinned Microsoft preview integration.")
        .with_suggested_command("powerbi-cli desktop bridge status --json"));
    };
    match action.as_str() {
        "status" => status_command(rest),
        "reload" => reload_command(rest),
        "screenshot-page" => screenshot_page_command(rest),
        "screenshot-all" => screenshot_all_command(rest),
        _ => Err(
            CliError::invalid_args(format!("unknown desktop bridge command: {action}"))
                .with_hint("Use status, reload, screenshot-page, or screenshot-all.")
                .with_suggested_command("powerbi-cli desktop bridge status --json"),
        ),
    }
}

fn status_command(args: &[String]) -> CliResult<Value> {
    let options = parse_bridge_args("desktop bridge status", args, &["pid"])?;
    let execution = bridge_status_execution(options.pid)?;
    let instances = parse_status_instances(&execution.value)?;
    let has_unsaved_changes = instances
        .iter()
        .any(|instance| instance.has_unsaved_changes);
    Ok(json!({
        "schema": "powerbi-cli.desktop.bridge.status.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "ready": instances.iter().any(|instance| instance.bridge_status == "connected"),
        "status": execution.value["status"],
        "instances": instances,
        "backend": backend_json(&execution, true),
        "proof": proof_limitations("state-inventory", has_unsaved_changes),
        "next": []
    }))
}

fn reload_command(args: &[String]) -> CliResult<Value> {
    let options = parse_bridge_args("desktop bridge reload", args, &["project", "pid"])?;
    let project = required_project(&options, "desktop bridge reload")?;
    let pid = required_pid(&options, "desktop bridge reload")?;
    let resolved = resolve_project(project)?;
    let _lock = PidOperationLock::acquire(pid)?;
    let exact = require_exact_instance(&resolved, pid, true)?;
    let manifest = bridge_manifest(pid)?;
    let execution = run_bridge_vendor(
        "reload",
        &[
            "--pid".into(),
            pid.to_string(),
            "--wait-seconds".into(),
            "60".into(),
        ],
        OPERATION_TIMEOUT,
    )?;
    require_vendor_pid(&execution.value, pid, "reload")?;
    let success = execution.value["result"]["success"]
        .as_bool()
        .ok_or_else(|| protocol_failed("Desktop Bridge reload response omitted result.success"))?;
    if !success {
        return Err(backend_failed(
            "Desktop Bridge reload reported success=false",
        ));
    }
    Ok(json!({
        "schema": "powerbi-cli.desktop.bridge.reload.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "project": canonical_display(&exact.canonical_project),
        "pid": pid,
        "hasUnsavedChanges": false,
        "ownership": external_ownership_json(pid),
        "desktop": manifest,
        "backend": backend_json(&execution, false),
        "changes": [{
            "kind": "desktop-report-reload",
            "pid": pid,
            "mutatesProject": false,
            "reloadModelDefinition": false
        }],
        "proof": proof_limitations("reload-request-completed", false),
        "next": []
    }))
}

fn screenshot_page_command(args: &[String]) -> CliResult<Value> {
    let options = parse_bridge_args(
        "desktop bridge screenshot-page",
        args,
        &["project", "pid", "page", "out"],
    )?;
    let project = required_project(&options, "desktop bridge screenshot-page")?;
    let pid = required_pid(&options, "desktop bridge screenshot-page")?;
    let page = required_text(options.page.as_deref(), "--page")?;
    let resolved = resolve_project(project)?;
    let _lock = PidOperationLock::acquire(pid)?;
    let exact = require_exact_instance(&resolved, pid, false)?;
    let manifest = bridge_manifest(pid)?;
    validate_page_inventory(&exact.instance.pages)?;
    if !exact.instance.pages.iter().any(|item| item.id == page) {
        return Err(CliError::validation_failed(format!(
            "page `{page}` is not in the exact Desktop instance page inventory"
        ))
        .with_hint(
            "Run `powerbi-cli desktop bridge status --pid <pid> --json` and use an exact page id.",
        ));
    }
    let out = guarded_new_png(
        options.out.as_deref().ok_or_else(|| {
            CliError::invalid_args("desktop bridge screenshot-page requires --out <new.png>")
        })?,
        &resolved,
    )?;
    let mut output_guard = OwnedOutputGuard::reserve_file(&out)?;
    let execution = run_bridge_vendor(
        "screenshot",
        &[
            page.to_string(),
            "--pid".into(),
            pid.to_string(),
            "--output".into(),
            out.to_string_lossy().into_owned(),
            "--wait-seconds".into(),
            "60".into(),
        ],
        OPERATION_TIMEOUT,
    )?;
    require_vendor_pid(&execution.value, pid, "screenshot")?;
    require_vendor_page_id(&execution.value, page)?;
    require_vendor_output_path(&execution.value, "outputPath", &out)?;
    let png = output_guard.inspect_png(&out)?;
    let response = json!({
        "schema": "powerbi-cli.desktop.bridge.screenshotPage.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "project": canonical_display(&exact.canonical_project),
        "pid": pid,
        "page": {"id": page, "displayName": execution.value["pageDisplayName"]},
        "hasUnsavedChanges": exact.instance.has_unsaved_changes,
        "screenshot": png_json(&png),
        "ownership": external_ownership_json(pid),
        "desktop": manifest,
        "backend": backend_json(&execution, false),
        "proof": proof_limitations("page-screenshot-captured", exact.instance.has_unsaved_changes),
        "next": []
    });
    output_guard.disarm()?;
    Ok(response)
}

fn screenshot_all_command(args: &[String]) -> CliResult<Value> {
    let options = parse_bridge_args(
        "desktop bridge screenshot-all",
        args,
        &["project", "pid", "out-dir"],
    )?;
    let project = required_project(&options, "desktop bridge screenshot-all")?;
    let pid = required_pid(&options, "desktop bridge screenshot-all")?;
    let resolved = resolve_project(project)?;
    let _lock = PidOperationLock::acquire(pid)?;
    let exact = require_exact_instance(&resolved, pid, false)?;
    let manifest = bridge_manifest(pid)?;
    validate_page_inventory(&exact.instance.pages)?;
    let out_dir = guarded_new_directory(
        options.out_dir.as_deref().ok_or_else(|| {
            CliError::invalid_args("desktop bridge screenshot-all requires --out-dir <new-dir>")
        })?,
        &resolved,
    )?;
    let mut output_guard = OwnedOutputGuard::reserve_directory(&out_dir)?;
    let execution = run_bridge_vendor(
        "screenshot-all",
        &[
            "--pid".into(),
            pid.to_string(),
            "--output-dir".into(),
            out_dir.to_string_lossy().into_owned(),
            "--wait-seconds".into(),
            "60".into(),
        ],
        SCREENSHOT_ALL_TIMEOUT,
    )?;
    require_vendor_pid(&execution.value, pid, "screenshot-all")?;
    let captures = execution.value["screenshots"]
        .as_array()
        .ok_or_else(|| protocol_failed("Desktop Bridge screenshot-all omitted screenshots[]"))?;
    if !execution.value["failures"]
        .as_array()
        .is_some_and(Vec::is_empty)
    {
        return Err(backend_failed(
            "Desktop Bridge screenshot-all returned one or more page failures",
        ));
    }
    let expected_ids = exact
        .instance
        .pages
        .iter()
        .map(|page| page.id.as_str())
        .collect::<Vec<_>>();
    let actual_ids = captures
        .iter()
        .map(|capture| capture["pageId"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    if actual_ids != expected_ids {
        return Err(protocol_failed(
            "Desktop Bridge screenshot inventory did not exactly match the status page inventory",
        ));
    }
    let mut screenshots = Vec::with_capacity(captures.len());
    let mut output_paths = BTreeSet::new();
    let mut total_bytes = 0_u64;
    for capture in captures {
        let path = capture["outputPath"].as_str().ok_or_else(|| {
            protocol_failed("Desktop Bridge screenshot-all item omitted outputPath")
        })?;
        let path = exact_output_below(&out_dir, Path::new(path))?;
        if !output_paths.insert(normalized_path_key(&path)) {
            return Err(protocol_failed(
                "Desktop Bridge returned the same screenshot file for more than one page",
            ));
        }
        let png = output_guard.inspect_png(&path)?;
        total_bytes = total_bytes.saturating_add(png.bytes);
        if total_bytes > MAX_SCREENSHOT_TOTAL_BYTES {
            return Err(CliError::validation_failed(format!(
                "Desktop Bridge screenshots exceed the {} byte total limit",
                MAX_SCREENSHOT_TOTAL_BYTES
            )));
        }
        screenshots.push(json!({
            "pageId": capture["pageId"],
            "pageDisplayName": capture["pageDisplayName"],
            "file": png_json(&png)
        }));
    }
    let response = json!({
        "schema": "powerbi-cli.desktop.bridge.screenshotAll.v1",
        "ok": true,
        "exitCode": EXIT_SUCCESS,
        "project": canonical_display(&exact.canonical_project),
        "pid": pid,
        "hasUnsavedChanges": exact.instance.has_unsaved_changes,
        "pageInventory": exact.instance.pages,
        "screenshots": screenshots,
        "outputDirectory": canonical_display(&out_dir),
        "ownership": external_ownership_json(pid),
        "desktop": manifest,
        "backend": backend_json(&execution, false),
        "proof": proof_limitations("all-page-screenshots-captured", exact.instance.has_unsaved_changes),
        "next": []
    });
    output_guard.disarm()?;
    Ok(response)
}

pub(crate) fn bridge_manifest(pid: u32) -> CliResult<Value> {
    ensure_bridge_platform()?;
    let execution = run_bridge_vendor(
        "manifest",
        &[
            "--pid".into(),
            pid.to_string(),
            "--wait-seconds".into(),
            "0".into(),
        ],
        STATUS_TIMEOUT,
    )?;
    require_vendor_pid(&execution.value, pid, "manifest")?;
    let manifest = execution
        .value
        .get("manifest")
        .ok_or_else(|| protocol_failed("Desktop Bridge manifest response omitted manifest"))?;
    let mut methods = manifest["methods"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|method| method["name"].as_str().map(str::to_string))
        .collect::<Vec<_>>();
    methods.sort();
    methods.dedup();
    Ok(json!({
        "pid": pid,
        "methods": methods,
        "desktopVersion": desktop_version(manifest),
        "backend": backend_json(&execution, true)
    }))
}

fn bridge_status_execution(pid: Option<u32>) -> CliResult<VendorExecution> {
    let mut args = vec!["--wait-seconds".into(), "0".into()];
    if let Some(pid) = pid {
        args.push("--pid".into());
        args.push(pid.to_string());
    }
    let execution = run_bridge_vendor("status", &args, STATUS_TIMEOUT)?;
    if let Some(pid) = pid {
        let instances = parse_status_instances(&execution.value)?;
        require_status_pid(&instances, pid)?;
    }
    Ok(execution)
}

fn run_bridge_vendor(
    action: &str,
    args: &[String],
    timeout: Duration,
) -> CliResult<VendorExecution> {
    let tool = resolve_installed_component(MicrosoftComponent::DesktopBridge)?;
    let node = tool.node.as_ref().ok_or_else(|| {
        CliError::new(
            "dependency_unavailable",
            30,
            "Node 20+ is required for the pinned Desktop Bridge integration",
        )
    })?;
    let mut command = minimal_child_command(node, &[parent_dir(node)]);
    pass_desktop_environment(&mut command);
    command.arg(&tool.entrypoint).arg(action).args(args);
    let output = run_bounded(command, effective_bridge_timeout(timeout)).map_err(|error| {
        backend_failed(format!(
            "Desktop Bridge {action} failed or exceeded its bounded deadline: {error}"
        ))
    })?;
    if output.stdout_truncated {
        return Err(protocol_failed(format!(
            "Desktop Bridge {action} stdout exceeded the bounded response limit"
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout_bytes).map_err(|_| {
        protocol_failed(format!(
            "Desktop Bridge {action} returned invalid machine-readable JSON (stdout {})",
            output.stdout_sha256
        ))
    })?;
    if !output.status.success() || value["status"] == "error" {
        let vendor_code = value["error"]["code"].as_str().unwrap_or("unknown");
        return Err(backend_failed(format!(
            "Desktop Bridge {action} failed with vendor code {vendor_code} (stderr {})",
            output.stderr_sha256
        )));
    }
    Ok(VendorExecution {
        value,
        output,
        version: tool.version,
    })
}

fn effective_bridge_timeout(default: Duration) -> Duration {
    if cfg!(debug_assertions)
        && let Ok(value) = env::var("POWERBI_CLI_TEST_BRIDGE_TIMEOUT_MS")
        && let Ok(milliseconds) = value.parse::<u64>()
        && (1..=5_000).contains(&milliseconds)
    {
        return Duration::from_millis(milliseconds);
    }
    default
}

fn require_exact_instance(
    resolved: &ResolvedProject,
    pid: u32,
    require_clean: bool,
) -> CliResult<ExactInstance> {
    let canonical_project = fs::canonicalize(&resolved.pbip_path).map_err(|error| {
        CliError::file_not_found(format!(
            "canonicalize exact Desktop project {}: {error}",
            resolved.pbip_path.display()
        ))
    })?;
    let execution = bridge_status_execution(Some(pid))?;
    let mut instances = parse_status_instances(&execution.value)?;
    if instances.len() != 1 || instances[0].pid != pid {
        return Err(CliError::validation_failed(format!(
            "Desktop Bridge status did not return exactly PID {pid}"
        ))
        .with_hint("Run bridge status and pass the exact connected Power BI Desktop PID."));
    }
    let instance = instances.remove(0);
    if instance.bridge_status != "connected" {
        return Err(CliError::validation_failed(format!(
            "Desktop PID {pid} is not connected to the Desktop Bridge"
        )));
    }
    let current = fs::canonicalize(&instance.current_file_path).map_err(|error| {
        CliError::validation_failed(format!(
            "Desktop PID {pid} current file cannot be canonicalized exactly: {error}"
        ))
    })?;
    if !paths_equal(&current, &canonical_project) {
        return Err(CliError::validation_failed(format!(
            "Desktop PID {pid} does not have the exact requested PBIP open"
        ))
        .with_hint(format!(
            "Expected {}. Refusing to operate on a different Desktop instance.",
            canonical_display(&canonical_project)
        )));
    }
    if require_clean && instance.has_unsaved_changes {
        return Err(CliError::validation_failed(format!(
            "Desktop PID {pid} has unsaved changes; reload is refused"
        ))
        .with_hint("Save or discard the Desktop changes, then rerun status and reload."));
    }
    Ok(ExactInstance {
        instance,
        canonical_project,
    })
}

fn parse_status_instances(value: &Value) -> CliResult<Vec<BridgeInstance>> {
    let instances = value["instances"]
        .as_array()
        .ok_or_else(|| protocol_failed("Desktop Bridge status response omitted instances[]"))?;
    instances
        .iter()
        .map(|instance| {
            serde_json::from_value(instance.clone()).map_err(|_| {
                protocol_failed("Desktop Bridge status returned an invalid instance shape")
            })
        })
        .collect()
}

fn require_status_pid(instances: &[BridgeInstance], requested_pid: u32) -> CliResult<()> {
    if instances.len() != 1
        || instances
            .iter()
            .any(|instance| instance.pid != requested_pid)
    {
        return Err(protocol_failed(format!(
            "Desktop Bridge status for PID {requested_pid} returned a different or ambiguous PID"
        )));
    }
    Ok(())
}

fn require_vendor_pid(value: &Value, requested_pid: u32, action: &str) -> CliResult<()> {
    let actual = value["pid"].as_u64().ok_or_else(|| {
        protocol_failed(format!(
            "Desktop Bridge {action} response omitted a numeric pid"
        ))
    })?;
    if actual != u64::from(requested_pid) {
        return Err(protocol_failed(format!(
            "Desktop Bridge {action} returned PID {actual}, expected exact PID {requested_pid}"
        )));
    }
    Ok(())
}

fn require_vendor_page_id(value: &Value, requested_page_id: &str) -> CliResult<()> {
    let actual = value["pageId"]
        .as_str()
        .ok_or_else(|| protocol_failed("Desktop Bridge screenshot response omitted pageId"))?;
    if actual != requested_page_id {
        return Err(protocol_failed(format!(
            "Desktop Bridge screenshot returned pageId `{actual}`, expected exact pageId `{requested_page_id}`"
        )));
    }
    Ok(())
}

fn validate_page_inventory(pages: &[BridgePage]) -> CliResult<()> {
    if pages.is_empty() {
        return Err(CliError::validation_failed(
            "the exact Desktop instance has no PBIR page inventory",
        ));
    }
    if pages.len() > MAX_SCREENSHOT_COUNT {
        return Err(CliError::validation_failed(format!(
            "Desktop page inventory has {} pages; the bounded limit is {MAX_SCREENSHOT_COUNT}",
            pages.len()
        )));
    }
    let mut ids = BTreeSet::new();
    for page in pages {
        if page.id.is_empty() || !ids.insert(&page.id) {
            return Err(protocol_failed(
                "Desktop Bridge returned an empty or duplicate page id",
            ));
        }
    }
    Ok(())
}

fn parse_bridge_args(command: &str, args: &[String], allowed: &[&str]) -> CliResult<BridgeArgs> {
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    let mut parsed = BridgeArgs::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let key = flag.strip_prefix("--").ok_or_else(|| {
            CliError::invalid_args(format!(
                "unexpected positional argument for {command}: {flag}"
            ))
        })?;
        if !allowed.contains(key) {
            return Err(CliError::invalid_args(format!(
                "unknown option for {command}: {flag}"
            )));
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a value")))?;
        match key {
            "project" => set_once(&mut parsed.project, PathBuf::from(value), flag)?,
            "pid" => {
                let pid = value.parse::<u32>().map_err(|_| {
                    CliError::invalid_args("--pid must be an integer from 1 to 4294967295")
                })?;
                if pid == 0 {
                    return Err(CliError::invalid_args("--pid must be greater than zero"));
                }
                set_once(&mut parsed.pid, pid, flag)?;
            }
            "page" => set_once(&mut parsed.page, value.clone(), flag)?,
            "out" => set_once(&mut parsed.out, PathBuf::from(value), flag)?,
            "out-dir" => set_once(&mut parsed.out_dir, PathBuf::from(value), flag)?,
            _ => unreachable!(),
        }
        index += 2;
    }
    Ok(parsed)
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> CliResult<()> {
    if slot.is_some() {
        return Err(CliError::invalid_args(format!(
            "{flag} may be specified only once"
        )));
    }
    *slot = Some(value);
    Ok(())
}

fn required_project<'a>(options: &'a BridgeArgs, command: &str) -> CliResult<&'a Path> {
    options.project.as_deref().ok_or_else(|| {
        CliError::invalid_args(format!("{command} requires --project <project.pbip>"))
    })
}

fn required_pid(options: &BridgeArgs, command: &str) -> CliResult<u32> {
    options
        .pid
        .ok_or_else(|| CliError::invalid_args(format!("{command} requires --pid <pid>")))
}

fn required_text<'a>(value: Option<&'a str>, flag: &str) -> CliResult<&'a str> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CliError::invalid_args(format!("{flag} requires a non-empty value")))
}

fn guarded_new_png(path: &Path, resolved: &ResolvedProject) -> CliResult<PathBuf> {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("png"))
    {
        return Err(CliError::invalid_args(
            "Desktop Bridge screenshot output must use a .png extension",
        ));
    }
    guarded_new_path(path, resolved)
}

fn guarded_new_directory(path: &Path, resolved: &ResolvedProject) -> CliResult<PathBuf> {
    guarded_new_path(path, resolved)
}

fn guarded_new_path(path: &Path, resolved: &ResolvedProject) -> CliResult<PathBuf> {
    if path.exists() {
        return Err(CliError::validation_failed(format!(
            "Desktop Bridge evidence output already exists: {}",
            path.display()
        ))
        .with_hint("Choose a new output path; prior evidence is never replaced or deleted."));
    }
    let name = path.file_name().ok_or_else(|| {
        CliError::invalid_args("Desktop Bridge evidence output requires a final path component")
    })?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        CliError::file_not_found(format!(
            "Desktop Bridge evidence parent does not exist: {} ({error})",
            parent.display()
        ))
    })?;
    let canonical_project_dir = fs::canonicalize(&resolved.project_dir).map_err(|error| {
        CliError::file_not_found(format!(
            "canonicalize PBIP project directory {}: {error}",
            resolved.project_dir.display()
        ))
    })?;
    if path_is_within(&canonical_parent, &canonical_project_dir) {
        return Err(CliError::validation_failed(
            "Desktop Bridge evidence output must be outside the PBIP project directory",
        ));
    }
    Ok(canonical_parent.join(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnedOutputKind {
    File,
    Directory,
}

struct OwnedPathHandle {
    path: PathBuf,
    identity: FileId,
    handle: Option<File>,
}

impl OwnedPathHandle {
    fn new(path: PathBuf, handle: File, kind: OwnedOutputKind, label: &str) -> CliResult<Self> {
        #[cfg(windows)]
        let identity = path_file_id(&path, kind, label)?;
        #[cfg(not(windows))]
        let identity = file_id_from_handle(&handle, label)?;
        let current = path_file_id(&path, kind, label)?;
        if current != identity {
            return Err(CliError::validation_failed(format!(
                "{label} path changed while its stable identity was captured"
            )));
        }
        Ok(Self {
            path,
            identity,
            handle: Some(handle),
        })
    }

    fn path_matches(&self, kind: OwnedOutputKind, label: &str) -> bool {
        path_file_id(&self.path, kind, label).as_ref().ok() == Some(&self.identity)
    }

    fn marker_matches(&mut self, nonce: &str) -> bool {
        if !self.path_matches(OwnedOutputKind::File, "Desktop Bridge ownership marker") {
            return false;
        }
        let Some(handle) = self.handle.as_mut() else {
            return false;
        };
        if handle.seek(SeekFrom::Start(0)).is_err() {
            return false;
        }
        let mut value = String::new();
        handle
            .take((nonce.len() as u64).saturating_add(1))
            .read_to_string(&mut value)
            .is_ok()
            && value == nonce
    }

    fn release_handle(&mut self) {
        drop(self.handle.take());
    }
}

struct OwnedOutputGuard {
    output: OwnedPathHandle,
    marker: OwnedPathHandle,
    nonce: String,
    kind: OwnedOutputKind,
    bound_files: Vec<OwnedPathHandle>,
    armed: bool,
}

impl OwnedOutputGuard {
    fn reserve_file(path: &Path) -> CliResult<Self> {
        let nonce = output_guard_nonce();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("screenshot.png");
        let marker_id = nonce.replace(':', "-");
        let marker_path =
            path.with_file_name(format!(".{file_name}.powerbi-cli-owner-{marker_id}.marker"));
        let mut marker = write_new_ownership_marker(&marker_path, &nonce)?;
        let output_handle = match create_stable_file(path) {
            Ok(file) => file,
            Err(error) => {
                marker.release_handle();
                let cleanup = cleanup_owned_path(
                    &marker.path,
                    &marker.identity,
                    OwnedOutputKind::File,
                    &cleanup_tombstone(&marker.path, &nonce, "marker"),
                );
                return Err(CliError::validation_failed(format!(
                    "reserve new Desktop Bridge screenshot output {}: {error}; marker cleanup={cleanup}",
                    path.display()
                ))
                .with_hint(
                    "Choose a new output path; prior evidence is never replaced or deleted.",
                ));
            }
        };
        let output = OwnedPathHandle::new(
            path.to_path_buf(),
            output_handle,
            OwnedOutputKind::File,
            "Desktop Bridge screenshot output",
        )?;
        Ok(Self {
            output,
            marker,
            nonce,
            kind: OwnedOutputKind::File,
            bound_files: Vec::new(),
            armed: true,
        })
    }

    fn reserve_directory(path: &Path) -> CliResult<Self> {
        fs::create_dir(path).map_err(|error| {
            CliError::validation_failed(format!(
                "reserve new Desktop Bridge screenshot directory {}: {error}",
                path.display()
            ))
            .with_hint(
                "Choose a new output directory; prior evidence is never replaced or deleted.",
            )
        })?;
        let nonce = output_guard_nonce();
        let output_handle = open_stable_directory(path).map_err(|error| {
            CliError::unexpected(format!(
                "open stable Desktop Bridge screenshot directory {}: {error}",
                path.display()
            ))
        })?;
        let mut output = OwnedPathHandle::new(
            path.to_path_buf(),
            output_handle,
            OwnedOutputKind::Directory,
            "Desktop Bridge screenshot directory",
        )?;
        let marker_path = path.join(".powerbi-cli-output-owner.marker");
        let marker = match write_new_ownership_marker(&marker_path, &nonce) {
            Ok(marker) => marker,
            Err(error) => {
                output.release_handle();
                let _ = cleanup_owned_path(
                    &output.path,
                    &output.identity,
                    OwnedOutputKind::Directory,
                    &cleanup_tombstone(&output.path, &nonce, "directory"),
                );
                return Err(error);
            }
        };
        Ok(Self {
            output,
            marker,
            nonce,
            kind: OwnedOutputKind::Directory,
            bound_files: Vec::new(),
            armed: true,
        })
    }

    fn inspect_png(&mut self, path: &Path) -> CliResult<GuardedPng> {
        if self.kind == OwnedOutputKind::File {
            if path != self.output.path {
                return Err(CliError::validation_failed(
                    "Desktop Bridge screenshot path differs from its reserved output",
                ));
            }
            let identity = self.output.identity;
            let handle = self.output.handle.as_mut().ok_or_else(|| {
                CliError::unexpected("reserved Desktop Bridge screenshot handle is unavailable")
            })?;
            return inspect_png_handle(path, handle, &identity);
        }
        if !self.output.path_matches(
            OwnedOutputKind::Directory,
            "Desktop Bridge screenshot directory",
        ) {
            return Err(CliError::validation_failed(
                "Desktop Bridge screenshot directory identity changed before inspection",
            ));
        }
        let file = open_stable_file(path).map_err(|error| {
            CliError::validation_failed(format!("open Desktop Bridge screenshot: {error}"))
        })?;
        let mut owned = OwnedPathHandle::new(
            path.to_path_buf(),
            file,
            OwnedOutputKind::File,
            "Desktop Bridge screenshot",
        )?;
        let png = inspect_png_handle(
            path,
            owned.handle.as_mut().expect("new screenshot handle"),
            &owned.identity,
        )?;
        self.bound_files.push(owned);
        Ok(png)
    }

    fn disarm(&mut self) -> CliResult<()> {
        if !self.marker.marker_matches(&self.nonce)
            || !self.output.path_matches(self.kind, "Desktop Bridge output")
            || self
                .bound_files
                .iter()
                .any(|file| !file.path_matches(OwnedOutputKind::File, "Desktop Bridge screenshot"))
        {
            return Err(CliError::validation_failed(
                "Desktop Bridge output filesystem identity changed before publication",
            )
            .with_hint(
                "The invocation cannot safely publish or clean an output it no longer owns.",
            ));
        }
        self.marker.release_handle();
        let tombstone = cleanup_tombstone(&self.marker.path, &self.nonce, "publish-marker");
        if !cleanup_owned_path(
            &self.marker.path,
            &self.marker.identity,
            OwnedOutputKind::File,
            &tombstone,
        ) {
            return Err(CliError::validation_failed(
                "Desktop Bridge ownership marker could not be removed by exact identity",
            ));
        }
        self.armed = false;
        Ok(())
    }

    fn cleanup_owned(&mut self) {
        if !self.marker.marker_matches(&self.nonce)
            || !self.output.path_matches(self.kind, "Desktop Bridge output")
        {
            return;
        }
        for file in &mut self.bound_files {
            file.release_handle();
        }
        self.marker.release_handle();
        self.output.release_handle();
        match self.kind {
            OwnedOutputKind::File => {
                if cleanup_owned_path(
                    &self.output.path,
                    &self.output.identity,
                    OwnedOutputKind::File,
                    &cleanup_tombstone(&self.output.path, &self.nonce, "output"),
                ) {
                    let _ = cleanup_owned_path(
                        &self.marker.path,
                        &self.marker.identity,
                        OwnedOutputKind::File,
                        &cleanup_tombstone(&self.marker.path, &self.nonce, "marker"),
                    );
                }
            }
            OwnedOutputKind::Directory => {
                let _ = cleanup_owned_path(
                    &self.output.path,
                    &self.output.identity,
                    OwnedOutputKind::Directory,
                    &cleanup_tombstone(&self.output.path, &self.nonce, "directory"),
                );
            }
        }
    }

    #[cfg(test)]
    fn release_handles_for_test(&mut self) {
        self.marker.release_handle();
        self.output.release_handle();
        for file in &mut self.bound_files {
            file.release_handle();
        }
    }
}

impl Drop for OwnedOutputGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cleanup_owned();
        }
    }
}

fn output_guard_nonce() -> String {
    format!(
        "{}:{}:{}",
        std::process::id(),
        OUTPUT_GUARD_COUNTER.fetch_add(1, Ordering::Relaxed),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn write_new_ownership_marker(path: &Path, nonce: &str) -> CliResult<OwnedPathHandle> {
    let file = create_stable_file(path).map_err(|error| {
        CliError::validation_failed(format!(
            "reserve Desktop Bridge output ownership marker {}: {error}",
            path.display()
        ))
    })?;
    let mut marker = OwnedPathHandle::new(
        path.to_path_buf(),
        file,
        OwnedOutputKind::File,
        "Desktop Bridge ownership marker",
    )?;
    let write_result = marker
        .handle
        .as_mut()
        .expect("new ownership marker handle")
        .write_all(nonce.as_bytes())
        .and_then(|()| {
            marker
                .handle
                .as_ref()
                .expect("new ownership marker handle")
                .sync_all()
        });
    if let Err(error) = write_result {
        marker.release_handle();
        let _ = cleanup_owned_path(
            &marker.path,
            &marker.identity,
            OwnedOutputKind::File,
            &cleanup_tombstone(&marker.path, nonce, "failed-marker"),
        );
        return Err(CliError::unexpected(format!(
            "write Desktop Bridge output ownership marker {}: {error}",
            path.display()
        )));
    }
    Ok(marker)
}

fn create_stable_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    options.open(path)
}

fn open_stable_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    options.open(path)
}

#[cfg(windows)]
fn open_stable_directory(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

#[cfg(not(windows))]
fn open_stable_directory(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(not(windows))]
fn file_id_from_handle(file: &File, label: &str) -> CliResult<FileId> {
    let metadata = file.metadata().map_err(|error| {
        CliError::unexpected(format!("read {label} stable filesystem identity: {error}"))
    })?;
    use std::os::unix::fs::MetadataExt;
    Ok(FileId::new_inode(metadata.dev(), metadata.ino()))
}

fn stable_file_identity(path: &Path, file: &File, label: &str) -> CliResult<FileId> {
    #[cfg(windows)]
    {
        let _ = file;
        path_file_id(path, OwnedOutputKind::File, label)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        file_id_from_handle(file, label)
    }
}

fn path_file_id(path: &Path, kind: OwnedOutputKind, label: &str) -> CliResult<FileId> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::validation_failed(format!("inspect {label} {}: {error}", path.display()))
    })?;
    let valid = match kind {
        OwnedOutputKind::File => metadata.file_type().is_file(),
        OwnedOutputKind::Directory => metadata.file_type().is_dir(),
    } && !metadata.file_type().is_symlink();
    if !valid {
        return Err(CliError::validation_failed(format!(
            "{label} is no longer the expected ordinary filesystem object"
        )));
    }
    #[cfg(windows)]
    {
        get_file_id(path).map_err(|error| {
            CliError::validation_failed(format!(
                "read {label} filesystem identity {}: {error}",
                path.display()
            ))
        })
    }
    #[cfg(not(windows))]
    {
        let handle = match kind {
            OwnedOutputKind::File => open_stable_file(path),
            OwnedOutputKind::Directory => open_stable_directory(path),
        }
        .map_err(|error| {
            CliError::validation_failed(format!("open {label} {}: {error}", path.display()))
        })?;
        file_id_from_handle(&handle, label)
    }
}

fn cleanup_tombstone(path: &Path, nonce: &str, label: &str) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bridge-output");
    path.with_file_name(format!(
        ".{name}.powerbi-cli-{label}-{}",
        nonce.replace(':', "-")
    ))
}

fn cleanup_owned_path(
    path: &Path,
    identity: &FileId,
    kind: OwnedOutputKind,
    tombstone: &Path,
) -> bool {
    if fs::symlink_metadata(tombstone).is_ok()
        || path_file_id(path, kind, "Desktop Bridge cleanup candidate")
            .as_ref()
            .ok()
            != Some(identity)
        || fs::rename(path, tombstone).is_err()
    {
        return false;
    }
    if path_file_id(tombstone, kind, "Desktop Bridge cleanup tombstone")
        .as_ref()
        .ok()
        != Some(identity)
    {
        if fs::symlink_metadata(path).is_err() {
            let _ = fs::rename(tombstone, path);
        }
        return false;
    }
    match kind {
        OwnedOutputKind::File => fs::remove_file(tombstone).is_ok(),
        OwnedOutputKind::Directory => fs::remove_dir_all(tombstone).is_ok(),
    }
}

fn exact_output_below(root: &Path, candidate: &Path) -> CliResult<PathBuf> {
    let canonical = fs::canonicalize(candidate).map_err(|error| {
        CliError::validation_failed(format!(
            "Desktop Bridge screenshot output is missing: {} ({error})",
            candidate.display()
        ))
    })?;
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        CliError::validation_failed(format!(
            "Desktop Bridge screenshot directory is missing: {} ({error})",
            root.display()
        ))
    })?;
    if canonical.parent() != Some(canonical_root.as_path())
        || !path_is_within(&canonical, &canonical_root)
    {
        return Err(protocol_failed(
            "Desktop Bridge returned a screenshot path outside the exact output directory",
        ));
    }
    Ok(canonical)
}

fn require_vendor_output_path(value: &Value, field: &str, expected: &Path) -> CliResult<()> {
    let actual = value[field]
        .as_str()
        .ok_or_else(|| protocol_failed(format!("Desktop Bridge response omitted {field}")))?;
    let actual = fs::canonicalize(actual).map_err(|error| {
        CliError::validation_failed(format!(
            "Desktop Bridge output is missing: {actual} ({error})"
        ))
    })?;
    let expected = fs::canonicalize(expected).map_err(|error| {
        CliError::validation_failed(format!(
            "expected Desktop Bridge output is missing: {} ({error})",
            expected.display()
        ))
    })?;
    if !paths_equal(&actual, &expected) {
        return Err(protocol_failed(
            "Desktop Bridge returned a path different from the guarded output path",
        ));
    }
    Ok(())
}

fn inspect_png_handle(
    path: &Path,
    file: &mut File,
    expected_identity: &FileId,
) -> CliResult<GuardedPng> {
    let metadata = file.metadata().map_err(|error| {
        CliError::validation_failed(format!("read screenshot handle metadata: {error}"))
    })?;
    if !metadata.file_type().is_file() {
        return Err(CliError::validation_failed(
            "Desktop Bridge screenshot must be a regular non-symlink PNG file",
        ));
    }
    if &stable_file_identity(path, file, "Desktop Bridge screenshot")? != expected_identity {
        return Err(CliError::validation_failed(
            "Desktop Bridge screenshot handle identity changed before inspection",
        ));
    }
    if metadata.len() == 0 || metadata.len() > MAX_SCREENSHOT_BYTES {
        return Err(CliError::validation_failed(format!(
            "Desktop Bridge screenshot size {} is outside the 1..={MAX_SCREENSHOT_BYTES} byte bound",
            metadata.len()
        )));
    }
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        CliError::validation_failed(format!("seek Desktop Bridge screenshot: {error}"))
    })?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_SCREENSHOT_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliError::validation_failed(format!("read Desktop Bridge PNG: {error}"))
        })?;
    if bytes.len() as u64 != metadata.len() {
        return Err(CliError::validation_failed(
            "Desktop Bridge screenshot changed while it was being inspected",
        ));
    }
    let final_metadata = file.metadata().map_err(|error| {
        CliError::validation_failed(format!("re-read screenshot handle metadata: {error}"))
    })?;
    if final_metadata.len() != metadata.len()
        || &stable_file_identity(path, file, "Desktop Bridge screenshot")? != expected_identity
    {
        return Err(CliError::validation_failed(
            "Desktop Bridge screenshot identity or size changed while it was being inspected",
        ));
    }
    let (width, height) = validate_png_structure(&bytes)?;
    if path_file_id(path, OwnedOutputKind::File, "Desktop Bridge screenshot")?.ne(expected_identity)
    {
        return Err(CliError::validation_failed(
            "Desktop Bridge screenshot path changed before publication",
        ));
    }
    let mut digest = Sha256::new();
    digest.update(&bytes);
    Ok(GuardedPng {
        path: fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        width,
        height,
        bytes: metadata.len(),
        sha256: format!("sha256:{:x}", digest.finalize()),
    })
}

fn validate_png_structure(bytes: &[u8]) -> CliResult<(u32, u32)> {
    if bytes.len() < 33 || &bytes[..8] != PNG_SIGNATURE {
        return Err(CliError::validation_failed(
            "Desktop Bridge screenshot is not a PNG",
        ));
    }
    let mut offset = 8_usize;
    let mut dimensions = None;
    let mut saw_idat = false;
    let mut saw_iend = false;
    while offset < bytes.len() {
        let header_end = offset.checked_add(8).ok_or_else(|| {
            CliError::validation_failed("Desktop Bridge PNG chunk offset overflow")
        })?;
        if header_end > bytes.len() {
            return Err(CliError::validation_failed(
                "Desktop Bridge PNG has a truncated chunk header",
            ));
        }
        let length = u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("PNG chunk length slice"),
        ) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length).ok_or_else(|| {
            CliError::validation_failed("Desktop Bridge PNG chunk length overflow")
        })?;
        let chunk_end = data_end
            .checked_add(4)
            .ok_or_else(|| CliError::validation_failed("Desktop Bridge PNG CRC offset overflow"))?;
        if chunk_end > bytes.len() {
            return Err(CliError::validation_failed(
                "Desktop Bridge PNG has a truncated chunk",
            ));
        }
        let expected_crc = u32::from_be_bytes(
            bytes[data_end..chunk_end]
                .try_into()
                .expect("PNG CRC slice"),
        );
        if png_crc32(&bytes[offset + 4..data_end]) != expected_crc {
            return Err(CliError::validation_failed(
                "Desktop Bridge PNG has an invalid chunk checksum",
            ));
        }
        match kind {
            b"IHDR" => {
                if dimensions.is_some() || offset != 8 || length != 13 {
                    return Err(CliError::validation_failed(
                        "Desktop Bridge PNG has an invalid IHDR chunk",
                    ));
                }
                let width = u32::from_be_bytes(
                    bytes[data_start..data_start + 4]
                        .try_into()
                        .expect("PNG width slice"),
                );
                let height = u32::from_be_bytes(
                    bytes[data_start + 4..data_start + 8]
                        .try_into()
                        .expect("PNG height slice"),
                );
                if width == 0
                    || height == 0
                    || bytes[data_start + 10] != 0
                    || bytes[data_start + 11] != 0
                    || bytes[data_start + 12] > 1
                {
                    return Err(CliError::validation_failed(
                        "Desktop Bridge PNG has invalid dimensions or encoding methods",
                    ));
                }
                dimensions = Some((width, height));
            }
            b"IDAT" => saw_idat = true,
            b"IEND" => {
                if length != 0 || chunk_end != bytes.len() {
                    return Err(CliError::validation_failed(
                        "Desktop Bridge PNG has an invalid IEND chunk",
                    ));
                }
                saw_iend = true;
            }
            _ => {}
        }
        offset = chunk_end;
    }
    if !saw_idat || !saw_iend {
        return Err(CliError::validation_failed(
            "Desktop Bridge PNG is missing image data or its end marker",
        ));
    }
    dimensions
        .ok_or_else(|| CliError::validation_failed("Desktop Bridge PNG is missing dimensions"))
}

fn png_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn png_json(png: &GuardedPng) -> Value {
    json!({
        "path": canonical_display(&png.path),
        "format": "png",
        "width": png.width,
        "height": png.height,
        "bytes": png.bytes,
        "sha256": png.sha256
    })
}

fn backend_json(execution: &VendorExecution, read_only: bool) -> Value {
    json!({
        "requested": "desktop-bridge",
        "resolved": "desktop-bridge",
        "version": execution.version,
        "transport": "node",
        "readOnly": read_only,
        "stdoutSha256": execution.output.stdout_sha256,
        "stderrSha256": execution.output.stderr_sha256,
        "stdoutTruncated": execution.output.stdout_truncated,
        "stderrTruncated": execution.output.stderr_truncated,
        "stderr": execution.output.stderr
    })
}

fn proof_limitations(observed_stage: &str, unsaved: bool) -> Value {
    json!({
        "level": "unit-smoke",
        "observedStage": observed_stage,
        "hasUnsavedChanges": unsaved,
        "claimedCanvasRender": false,
        "claimedRefresh": false,
        "claimedSaveReopen": false,
        "claimedInteractionOrDrill": false,
        "claimedIssueDialogAbsence": false,
        "claimedSemanticCorrectness": false,
        "limitations": if unsaved {
            vec![
                "The Desktop instance inventory includes unsaved changes; any screenshots represent only the observed in-memory Desktop state.",
                "Bridge state, reload completion, and PNG capture do not prove refresh, save/reopen, interactions, drill behavior, issue-dialog absence, or semantic correctness."
            ]
        } else {
            vec![
                "Bridge state, reload completion, and PNG capture do not prove refresh, save/reopen, interactions, drill behavior, issue-dialog absence, or semantic correctness."
            ]
        }
    })
}

fn external_ownership_json(pid: u32) -> Value {
    json!({
        "kind": "externally-supplied-exact-path-pid",
        "pid": pid,
        "owned": false,
        "cleanupEligible": false
    })
}

fn desktop_version(manifest: &Value) -> Option<&str> {
    [
        "desktopVersion",
        "applicationVersion",
        "productVersion",
        "version",
    ]
    .iter()
    .find_map(|key| manifest[*key].as_str())
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn normalized_path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let path = path.to_string_lossy().to_ascii_lowercase();
        let root = root.to_string_lossy().to_ascii_lowercase();
        path == root
            || path
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with(['\\', '/']))
    } else {
        path == root || path.starts_with(root)
    }
}

fn pass_desktop_environment(command: &mut Command) {
    for key in ["ProgramFiles", "ProgramFiles(x86)", "PBI_DESKTOP_PATH"] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
}

fn parent_dir(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn ensure_bridge_platform() -> CliResult<()> {
    if cfg!(windows) {
        return Ok(());
    }
    Err(CliError::unsupported_feature(format!(
        "Microsoft Desktop Bridge is Windows-only; current platform is {}-{}",
        env::consts::OS,
        env::consts::ARCH
    ))
    .with_hint(
        "PBIP/PBIR/TMDL file authoring remains available. Run live Desktop evidence on Windows.",
    ))
}

fn backend_failed(message: impl Into<String>) -> CliError {
    CliError::new("backend_failed", EXIT_ORACLE_FAILED, message)
        .with_hint("Inspect bounded backend diagnostics and rerun status.")
}

fn protocol_failed(message: impl Into<String>) -> CliError {
    CliError::new("protocol_failed", EXIT_ORACLE_FAILED, message)
        .with_hint("The pinned Desktop Bridge response did not match the consumed contract.")
}

struct PidOperationLock {
    path: PathBuf,
    nonce: String,
}

impl PidOperationLock {
    fn acquire(pid: u32) -> CliResult<Self> {
        let root = env::temp_dir().join("powerbi-cli-desktop-bridge-locks");
        fs::create_dir_all(&root).map_err(|error| {
            CliError::unexpected(format!("create Desktop Bridge lock directory: {error}"))
        })?;
        let path = root.join(format!("pid-{pid}.lock"));
        let nonce = format!(
            "{}:{}:{}",
            std::process::id(),
            pid,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                CliError::validation_failed(format!(
                    "Desktop Bridge PID {pid} already has an operation lock: {error}"
                ))
                .with_hint(format!(
                    "Run operations serially. If no operation is active after a crashed command, remove {}.",
                    path.display()
                ))
            })?;
        file.write_all(nonce.as_bytes()).map_err(|error| {
            let _ = fs::remove_file(&path);
            CliError::unexpected(format!("write Desktop Bridge PID lock: {error}"))
        })?;
        Ok(Self { path, nonce })
    }
}

impl Drop for PidOperationLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).is_ok_and(|value| value == self.nonce) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_png() -> Vec<u8> {
        fn push_chunk(bytes: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
            bytes.extend_from_slice(kind);
            bytes.extend_from_slice(data);
            let mut crc_input = Vec::with_capacity(kind.len() + data.len());
            crc_input.extend_from_slice(kind);
            crc_input.extend_from_slice(data);
            bytes.extend_from_slice(&png_crc32(&crc_input).to_be_bytes());
        }

        let mut bytes = PNG_SIGNATURE.to_vec();
        push_chunk(
            &mut bytes,
            b"IHDR",
            &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0],
        );
        push_chunk(&mut bytes, b"IDAT", &[0]);
        push_chunk(&mut bytes, b"IEND", &[]);
        bytes
    }

    #[test]
    fn file_guard_rejects_replaced_output_and_same_nonce_marker() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let output = temp.path().join("capture.png");
        let mut guard = OwnedOutputGuard::reserve_file(&output).expect("reserve output");
        let marker = guard.marker.path.clone();
        let nonce = guard.nonce.clone();
        let original_output = temp.path().join("original-capture.png");
        let original_marker = temp.path().join("original-owner.marker");

        guard.release_handles_for_test();
        fs::rename(&output, &original_output).expect("move original output");
        fs::rename(&marker, &original_marker).expect("move original marker");
        fs::write(&output, b"replacement output").expect("write replacement output");
        fs::write(&marker, nonce).expect("write same-nonce replacement marker");

        assert!(guard.disarm().is_err());
        drop(guard);
        assert_eq!(
            fs::read(&output).expect("replacement output remains"),
            b"replacement output"
        );
        assert!(marker.exists(), "replacement marker must not be removed");
        assert!(
            original_output.exists(),
            "original output remains quarantined"
        );
        assert!(
            original_marker.exists(),
            "original marker remains quarantined"
        );
    }

    #[test]
    fn directory_guard_does_not_delete_replacement_tree() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let output = temp.path().join("captures");
        let mut guard = OwnedOutputGuard::reserve_directory(&output).expect("reserve directory");
        let nonce = guard.nonce.clone();
        let original = temp.path().join("original-captures");

        guard.release_handles_for_test();
        fs::rename(&output, &original).expect("move original directory");
        fs::create_dir(&output).expect("create replacement directory");
        fs::write(output.join("keep.txt"), b"replacement tree")
            .expect("write replacement evidence");
        fs::write(output.join(".powerbi-cli-output-owner.marker"), nonce)
            .expect("write same-nonce replacement marker");

        drop(guard);
        assert_eq!(
            fs::read(output.join("keep.txt")).expect("replacement tree remains"),
            b"replacement tree"
        );
        assert!(original.exists(), "original directory remains quarantined");
    }

    #[test]
    fn png_inspection_rejects_path_swapped_away_from_open_handle() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let output = temp.path().join("capture.png");
        let moved = temp.path().join("moved-capture.png");
        let bytes = test_png();
        let mut handle = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&output)
            .expect("create screenshot");
        handle.write_all(&bytes).expect("write screenshot");
        handle.sync_all().expect("sync screenshot");
        let identity = path_file_id(
            &output,
            OwnedOutputKind::File,
            "test Desktop Bridge screenshot",
        )
        .expect("capture screenshot identity");

        fs::rename(&output, &moved).expect("move open screenshot");
        fs::write(&output, &bytes).expect("install replacement screenshot");

        assert!(inspect_png_handle(&output, &mut handle, &identity).is_err());
        assert_eq!(
            fs::read(&output).expect("replacement screenshot remains"),
            bytes
        );
        assert!(
            moved.exists(),
            "opened screenshot remains at its moved path"
        );
    }

    #[test]
    fn file_guard_publishes_png_inspected_from_reserved_handle() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let output = temp.path().join("capture.png");
        let mut guard = OwnedOutputGuard::reserve_file(&output).expect("reserve output");
        let marker = guard.marker.path.clone();
        let bytes = test_png();
        let handle = guard
            .output
            .handle
            .as_mut()
            .expect("reserved output handle");
        handle.write_all(&bytes).expect("write screenshot");
        handle.sync_all().expect("sync screenshot");

        let png = guard.inspect_png(&output).expect("inspect screenshot");
        assert_eq!(png.bytes, bytes.len() as u64);
        assert_eq!((png.width, png.height), (1, 1));
        guard.disarm().expect("publish screenshot");
        drop(guard);

        assert_eq!(fs::read(&output).expect("published screenshot"), bytes);
        assert!(!marker.exists(), "owned marker is removed at publication");
    }
}
