//! Ownership verification and guarded cleanup for CLI-launched Desktop processes.

use crate::{CliError, CliResult};
#[cfg(any(windows, test))]
use serde::Deserialize;
#[cfg(windows)]
use serde_json::Value;
#[cfg(windows)]
use serde_json::json;
#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use super::{Timed, ensure_powershell_success, parse_powershell_json, run_powershell};

#[cfg(windows)]
pub(crate) const CLEANUP_TIMEOUT_MS: u64 = 15_000;

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessIdentity {
    pub(crate) process_id: u32,
    pub(crate) creation_time_utc: String,
    #[cfg(windows)]
    pub(crate) executable_path: Option<String>,
}

#[cfg(any(windows, test))]
pub(super) fn cleanup_unresolved_after_launch(launch_attempted: bool, cleanup: &Value) -> bool {
    launch_attempted
        && cleanup["requested"].as_bool() == Some(true)
        && cleanup["closed"].as_bool() != Some(true)
}

#[cfg(windows)]
pub(super) fn cleanup_after_launch(
    launch_attempted: bool,
    association_identity: Option<&ProcessIdentity>,
    observed_identity: Option<&ProcessIdentity>,
    baseline_process_ids: &[u32],
    close_after: bool,
    launch_timestamp_unix_ms: Option<u64>,
) -> Value {
    let association_process_id = association_identity.map(|identity| identity.process_id);
    let observed_process_id = observed_identity.map(|identity| identity.process_id);
    if !launch_attempted || !close_after {
        return json!({
            "requested": close_after,
            "attempted": false,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "closed": Value::Null,
            "skipped": [],
            "refusedReason": Value::Null,
            "errors": []
        });
    }
    if association_identity.is_none() && observed_identity.is_none() {
        return cleanup_refused(
            baseline_process_ids,
            launch_timestamp_unix_ms,
            "cleanup refused because no exact launch process identity was confirmed",
        );
    }
    let Some(launch_timestamp_unix_ms) = launch_timestamp_unix_ms else {
        return cleanup_refused(
            baseline_process_ids,
            None,
            "cleanup refused because the launch timestamp was unavailable",
        );
    };
    match cleanup_spawned_processes(
        association_identity,
        observed_identity,
        baseline_process_ids,
        launch_timestamp_unix_ms,
    ) {
        Ok(Timed::Completed(cleanup)) => json!({
            "requested": true,
            "attempted": true,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": cleanup["targeted"],
            "targetedProcessIds": cleanup["targetedProcessIds"],
            "remainingProcessIds": cleanup["remainingProcessIds"],
            "closed": cleanup["closed"],
            "skipped": cleanup["skipped"],
            "refusedReason": Value::Null,
            "errors": cleanup["errors"]
        }),
        Ok(Timed::TimedOut) => json!({
            "requested": true,
            "attempted": true,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "closed": false,
            "skipped": [],
            "refusedReason": Value::Null,
            "errors": [format!("spawned-process cleanup exceeded {CLEANUP_TIMEOUT_MS} ms")]
        }),
        Err(err) => json!({
            "requested": true,
            "attempted": true,
            "associationProcessId": association_process_id,
            "observedProcessId": observed_process_id,
            "baselineProcessIds": baseline_process_ids,
            "launchTimestampUnixMs": launch_timestamp_unix_ms,
            "targeted": [],
            "targetedProcessIds": [],
            "remainingProcessIds": [],
            "closed": false,
            "skipped": [],
            "refusedReason": Value::Null,
            "errors": [err.to_string()]
        }),
    }
}

#[cfg(windows)]
fn cleanup_refused(
    baseline_process_ids: &[u32],
    launch_timestamp_unix_ms: Option<u64>,
    reason: &str,
) -> Value {
    json!({
        "requested": true,
        "attempted": false,
        "associationProcessId": Value::Null,
        "baselineProcessIds": baseline_process_ids,
        "launchTimestampUnixMs": launch_timestamp_unix_ms,
        "targeted": [],
        "targetedProcessIds": [],
        "remainingProcessIds": [],
        "closed": Value::Null,
        "skipped": [],
        "refusedReason": reason,
        "errors": []
    })
}

#[cfg(windows)]
pub(crate) fn cleanup_spawned_processes(
    association_identity: Option<&ProcessIdentity>,
    observed_identity: Option<&ProcessIdentity>,
    baseline_process_ids: &[u32],
    launch_timestamp_unix_ms: u64,
) -> io::Result<Timed<Value>> {
    let script = render_cleanup_script(
        association_identity,
        observed_identity,
        baseline_process_ids,
        launch_timestamp_unix_ms,
    );
    match run_powershell(&script, Duration::from_millis(CLEANUP_TIMEOUT_MS))? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "spawned Desktop process cleanup")?;
            Ok(Timed::Completed(parse_powershell_json(&output.stdout)?))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const CLEANUP_SCRIPT: &str = r#"
$baseline = @(__BASELINE_IDS__)
$associationPid = __ASSOCIATION_PID__
$associationCreationTimeUtc = __ASSOCIATION_CREATION_TIME_UTC__
$observedPid = __OBSERVED_PID__
$observedCreationTimeUtc = __OBSERVED_CREATION_TIME_UTC__
$launchTimeUtc = [DateTimeOffset]::FromUnixTimeMilliseconds(__LAUNCH_TIME_UNIX_MS__).UtcDateTime
$targetReasons = @{}
$targetCreationUtc = @{}
$lineageRoots = [System.Collections.Generic.HashSet[int]]::new()
$skipped = [System.Collections.Generic.List[object]]::new()
$errors = [System.Collections.Generic.List[string]]::new()
try {
    $rows = @(Get-CimInstance Win32_Process -ErrorAction Stop)
} catch {
    $rows = @()
    [void]$errors.Add("process inventory failed: $($_.Exception.Message)")
}
$rowsById = @{}
foreach ($row in $rows) {
    $rowsById[[int]$row.ProcessId] = $row
}

function Add-OwnedTarget {
    param(
        [int]$ProcessId,
        [string]$Reason,
        [string]$ExpectedCreationTimeUtc = '',
        [bool]$RequireDesktop = $false
    )
    if ($ProcessId -le 0 -or $targetReasons.ContainsKey($ProcessId)) {
        return $false
    }
    if ($baseline -contains $ProcessId) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'baseline-pid' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: process existed in the pre-launch baseline")
        return $false
    }
    if (-not $rowsById.ContainsKey($ProcessId)) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-unavailable' })
        [void]$errors.Add("PID ${ProcessId} ownership unresolved: CIM process row unavailable")
        return $false
    }
    $row = $rowsById[$ProcessId]
    if ($RequireDesktop -and [string]$row.Name -notlike 'PBIDesktop*') {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'owned-root-is-not-desktop' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: recorded root is not a PBIDesktop process")
        return $false
    }
    if ($null -eq $row.CreationDate) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-unavailable' })
        [void]$errors.Add("PID ${ProcessId} ownership unresolved: CreationDate unavailable")
        return $false
    }
    try {
        $createdAtUtc = ([DateTime]$row.CreationDate).ToUniversalTime()
    } catch {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-invalid' })
        [void]$errors.Add("PID ${ProcessId} ownership unresolved: invalid CreationDate")
        return $false
    }
    if ($createdAtUtc -le $launchTimeUtc) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'created-before-or-at-launch' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: CreationDate predates or equals launch")
        return $false
    }
    if (-not [string]::IsNullOrWhiteSpace($ExpectedCreationTimeUtc) -and $createdAtUtc -ne ([DateTime]::Parse($ExpectedCreationTimeUtc)).ToUniversalTime()) {
        [void]$skipped.Add([pscustomobject]@{ pid = $ProcessId; reason = 'creation-time-no-longer-matches-recorded-identity' })
        [void]$errors.Add("PID ${ProcessId} cleanup refused: CreationDate no longer matches the recorded launch identity")
        return $false
    }
    $targetReasons[$ProcessId] = $Reason
    $targetCreationUtc[$ProcessId] = $createdAtUtc
    [void]$lineageRoots.Add($ProcessId)
    return $true
}

if ($associationPid -gt 0) {
    if ([string]::IsNullOrWhiteSpace($associationCreationTimeUtc)) {
        [void]$errors.Add("PID ${associationPid} cleanup refused: recorded association creation time is unavailable")
    } elseif (Get-Process -Id $associationPid -ErrorAction SilentlyContinue) {
        [void](Add-OwnedTarget -ProcessId $associationPid -Reason 'association-launch-pid' -ExpectedCreationTimeUtc $associationCreationTimeUtc -RequireDesktop $true)
    } else {
        [void]$skipped.Add([pscustomobject]@{ pid = $associationPid; reason = 'association-pid-already-exited' })
    }
}
if ($observedPid -gt 0 -and $observedPid -ne $associationPid) {
    if ([string]::IsNullOrWhiteSpace($observedCreationTimeUtc)) {
        [void]$errors.Add("PID ${observedPid} cleanup refused: recorded observed creation time is unavailable")
    } elseif (Get-Process -Id $observedPid -ErrorAction SilentlyContinue) {
        [void](Add-OwnedTarget -ProcessId $observedPid -Reason 'exact-observed-pid' -ExpectedCreationTimeUtc $observedCreationTimeUtc -RequireDesktop $true)
    } else {
        [void]$skipped.Add([pscustomobject]@{ pid = $observedPid; reason = 'observed-pid-already-exited' })
    }
}
$changed = $true
while ($changed) {
    $changed = $false
    foreach ($row in $rows) {
        $parentId = [int]$row.ParentProcessId
        $childId = [int]$row.ProcessId
        if ($lineageRoots.Contains($parentId) -and -not $targetReasons.ContainsKey($childId)) {
            if (Add-OwnedTarget -ProcessId $childId -Reason "descendant-of-$parentId") {
                $changed = $true
            }
        }
    }
}
$orderedTargets = @($targetReasons.Keys | ForEach-Object { [int]$_ } | Sort-Object -Descending)
$targeted = @(
    $orderedTargets | ForEach-Object {
        $ownedCreationTime = [DateTime]($targetCreationUtc[[int]$_])
        [pscustomobject]@{
            pid = [int]$_
            reason = [string]$targetReasons[[int]$_]
            creationTimeUtc = $ownedCreationTime.ToString('o')
        }
    }
)
foreach ($targetId in $orderedTargets) {
    if ($baseline -contains [int]$targetId) {
        [void]$errors.Add("PID ${targetId} kill refused: baseline PID")
        continue
    }
    $currentRows = @(Get-CimInstance Win32_Process -Filter "ProcessId = $targetId" -ErrorAction SilentlyContinue)
    if ($currentRows.Count -eq 0) {
        continue
    }
    $currentRow = $currentRows[0]
    if ($null -eq $currentRow.CreationDate) {
        [void]$errors.Add("PID ${targetId} kill refused: current CreationDate unavailable")
        continue
    }
    $currentCreatedAtUtc = ([DateTime]$currentRow.CreationDate).ToUniversalTime()
    $ownedCreatedAtUtc = [DateTime]($targetCreationUtc[[int]$targetId])
    if (
        $currentCreatedAtUtc -le $launchTimeUtc -or
        $currentCreatedAtUtc -ne $ownedCreatedAtUtc
    ) {
        [void]$errors.Add("PID ${targetId} kill refused: creation time no longer matches owned process")
        continue
    }
    try {
        Stop-Process -Id $targetId -Force -ErrorAction Stop
    } catch {
        if (Get-Process -Id $targetId -ErrorAction SilentlyContinue) {
            [void]$errors.Add("PID ${targetId} stop failed: $($_.Exception.Message)")
        }
    }
}
Start-Sleep -Milliseconds 200
$remaining = @(
    $orderedTargets |
        Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue } |
        ForEach-Object { [int]$_ }
)
$result = [pscustomobject]@{
    targeted = @($targeted)
    targetedProcessIds = @($orderedTargets)
    remainingProcessIds = @($remaining)
    closed = ($remaining.Count -eq 0 -and $errors.Count -eq 0)
    skipped = @($skipped)
    errors = @($errors)
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Compress -Depth 6))
"#;

#[cfg(any(windows, test))]
pub(super) fn render_cleanup_script(
    association_identity: Option<&ProcessIdentity>,
    observed_identity: Option<&ProcessIdentity>,
    baseline_process_ids: &[u32],
    launch_timestamp_unix_ms: u64,
) -> String {
    let baseline = baseline_process_ids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    CLEANUP_SCRIPT
        .replace("__BASELINE_IDS__", &baseline)
        .replace(
            "__ASSOCIATION_PID__",
            &association_identity
                .map(|identity| identity.process_id)
                .unwrap_or_default()
                .to_string(),
        )
        .replace(
            "__ASSOCIATION_CREATION_TIME_UTC__",
            &super::powershell_single_quoted(
                association_identity
                    .map(|identity| identity.creation_time_utc.as_str())
                    .unwrap_or_default(),
            ),
        )
        .replace(
            "__OBSERVED_PID__",
            &observed_identity
                .map(|identity| identity.process_id)
                .unwrap_or_default()
                .to_string(),
        )
        .replace(
            "__OBSERVED_CREATION_TIME_UTC__",
            &super::powershell_single_quoted(
                observed_identity
                    .map(|identity| identity.creation_time_utc.as_str())
                    .unwrap_or_default(),
            ),
        )
        .replace(
            "__LAUNCH_TIME_UNIX_MS__",
            &launch_timestamp_unix_ms.to_string(),
        )
}

#[cfg(windows)]
pub(crate) fn read_process_identity(process_id: u32) -> CliResult<Option<ProcessIdentity>> {
    let script = PROCESS_IDENTITY_SCRIPT.replace("__PROCESS_ID__", &process_id.to_string());
    match run_powershell(&script, Duration::from_millis(5_000)).map_err(|error| {
        CliError::unexpected(format!(
            "inspect Power BI Desktop process {process_id}: {error}"
        ))
    })? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "Power BI Desktop process identity")
                .map_err(|error| CliError::unexpected(error.to_string()))?;
            parse_powershell_json(&output.stdout).map_err(|error| {
                CliError::unexpected(format!(
                    "parse Power BI Desktop process {process_id} identity: {error}"
                ))
            })
        }
        Timed::TimedOut => Err(CliError::unexpected(format!(
            "Power BI Desktop process {process_id} identity check exceeded 5000 ms"
        ))),
    }
}

#[cfg(any(windows, test))]
const PROCESS_IDENTITY_SCRIPT: &str = r#"
$row = @(Get-CimInstance Win32_Process -Filter "ProcessId = __PROCESS_ID__" -ErrorAction Stop)
if ($row.Count -eq 0) {
    [Console]::Out.Write('null')
    return
}
$process = $row[0]
$result = [pscustomobject]@{
    processId = [int]$process.ProcessId
    creationTimeUtc = ([DateTime]$process.CreationDate).ToUniversalTime().ToString('o')
    executablePath = if ([string]::IsNullOrWhiteSpace([string]$process.ExecutablePath)) { $null } else { [string]$process.ExecutablePath }
}
[Console]::Out.Write((ConvertTo-Json -InputObject $result -Compress))
"#;

#[cfg(test)]
pub(super) fn render_process_identity_script(process_id: u32) -> String {
    PROCESS_IDENTITY_SCRIPT.replace("__PROCESS_ID__", &process_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        ProcessIdentity, cleanup_unresolved_after_launch, render_cleanup_script,
        render_process_identity_script,
    };
    use crate::desktop::evidence::render_screenshot_script;
    use crate::desktop::observe::{render_process_snapshot_script, render_window_query_script};
    use crate::desktop::{powershell_single_quoted, render_launch_script, render_version_script};
    use serde_json::json;

    fn process_identity(process_id: u32, creation_time_utc: &str) -> ProcessIdentity {
        ProcessIdentity {
            process_id,
            creation_time_utc: creation_time_utc.to_string(),
            #[cfg(windows)]
            executable_path: Some(r"C:\Program Files\Power BI\PBIDesktop.exe".to_string()),
        }
    }

    #[test]
    fn launched_one_shot_requires_verified_cleanup() {
        assert!(cleanup_unresolved_after_launch(
            true,
            &json!({"requested": true, "attempted": false, "closed": null})
        ));
        assert!(cleanup_unresolved_after_launch(
            true,
            &json!({"requested": true, "attempted": true, "closed": false})
        ));
        assert!(!cleanup_unresolved_after_launch(
            true,
            &json!({"requested": true, "attempted": true, "closed": true})
        ));
        assert!(!cleanup_unresolved_after_launch(
            true,
            &json!({"requested": false, "attempted": false, "closed": null})
        ));
        assert!(!cleanup_unresolved_after_launch(
            false,
            &json!({"requested": true, "attempted": false, "closed": null})
        ));
    }

    #[test]
    fn generated_powershell_scripts_are_fully_substituted_and_safely_quoted() {
        let adversarial = r"C:\Power BI\März $facts`tick O'Brien\Sales.pbip";
        let creation_time = "2026-07-22T10:15:31.1234567Z";
        let quoted_path = powershell_single_quoted(adversarial);
        let quoted_creation_time = powershell_single_quoted(creation_time);
        assert_eq!(
            quoted_path,
            r"'C:\Power BI\März $facts`tick O''Brien\Sales.pbip'"
        );

        let snapshot = render_process_snapshot_script();
        let windows = render_window_query_script();
        let launch = render_launch_script(adversarial);
        let screenshot = render_screenshot_script(adversarial, Some(4242), 4000, false);
        let association = process_identity(4242, creation_time);
        let observed = process_identity(5252, creation_time);
        let cleanup = render_cleanup_script(
            Some(&association),
            Some(&observed),
            &[7, 11, 4242],
            1_725_000_000_123,
        );
        let identity = render_process_identity_script(5252);
        let version = render_version_script(adversarial);

        for (name, script) in [
            ("snapshot", snapshot.as_str()),
            ("window query", windows.as_str()),
            ("launch", launch.as_str()),
            ("screenshot", screenshot.as_str()),
            ("cleanup", cleanup.as_str()),
            ("identity", identity.as_str()),
            ("version", version.as_str()),
        ] {
            assert!(
                !script.contains("__"),
                "{name} left a placeholder: {script}"
            );
            assert!(
                !has_powershell_variable_colon_trap(script),
                "{name} contains a $identifier: parsing trap: {script}"
            );
        }

        for script in [&launch, &screenshot, &version] {
            assert!(script.contains(&quoted_path));
        }
        assert!(cleanup.contains(&quoted_creation_time));
        assert!(cleanup.contains("$observedPid = 5252"));
        assert!(cleanup.contains("$baseline = @(7,11,4242)"));
        assert!(identity.contains("ProcessId = 5252"));
        assert!(screenshot.contains("$allowUnverifiedCapture = $false"));
    }

    #[test]
    fn generated_cleanup_script_guards_every_kill_with_owned_creation_and_baseline_checks() {
        let association = process_identity(501, "2026-07-22T10:15:31.1234567Z");
        let observed = process_identity(777, "2026-07-22T10:15:32.1234567Z");
        let script = render_cleanup_script(
            Some(&association),
            Some(&observed),
            &[100, 501],
            1_725_000_000_123,
        );
        assert_eq!(script.matches("Stop-Process").count(), 1);
        let baseline_guard = script
            .find("if ($baseline -contains [int]$targetId)")
            .expect("per-kill baseline guard");
        let creation_guard = script
            .find("$currentCreatedAtUtc -le $launchTimeUtc")
            .expect("per-kill creation-time guard");
        let stop = script.find("Stop-Process").expect("bounded kill");
        assert!(baseline_guard < stop);
        assert!(creation_guard < stop);
        assert!(script.contains("if ($baseline -contains $ProcessId)"));
        assert!(script.contains("if ($createdAtUtc -le $launchTimeUtc)"));
        assert!(script.contains("'association-launch-pid'"));
        assert!(script.contains("'exact-observed-pid'"));
        assert!(script.contains("'creation-time-no-longer-matches-recorded-identity'"));
        assert!(script.contains("$associationCreationTimeUtc"));
        assert!(script.contains("$observedCreationTimeUtc"));
        assert!(script.contains("-RequireDesktop $true"));
        assert!(script.contains("[string]$row.Name -notlike 'PBIDesktop*'"));
        assert!(!script.contains("'exact-project-title-match'"));
        assert!(!script.contains("'executable-path-and-created-after-launch'"));
        assert!(script.contains("descendant-of-$parentId"));
        assert!(script.contains("targeted = @($targeted)"));
        assert!(!script.contains("$targetIds.Add"));
    }

    fn has_powershell_variable_colon_trap(script: &str) -> bool {
        let bytes = script.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'$' {
                index += 1;
                continue;
            }
            let mut end = index + 1;
            if end >= bytes.len() || !(bytes[end].is_ascii_alphabetic() || bytes[end] == b'_') {
                index += 1;
                continue;
            }
            end += 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b':' {
                return true;
            }
            index = end;
        }
        false
    }
}
