//! Power BI Desktop process baselining and window-title observation.

#[cfg(any(windows, test))]
use serde::Deserialize;
#[cfg(windows)]
use std::collections::BTreeSet;
#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use super::launch::{
    Timed, Watchdog, ensure_powershell_success, parse_powershell_json, run_powershell,
};

#[cfg(windows)]
pub(super) const WINDOW_POLL_INTERVAL_MS: u64 = 250;

#[cfg(any(windows, test))]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessWindow {
    id: u32,
    process_name: String,
    #[serde(default)]
    main_window_title: String,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
pub(super) struct WindowObservation {
    pub(super) attempted: bool,
    pub(super) window_observed: Option<bool>,
    pub(super) title_matched: Option<bool>,
    pub(super) observed_window_title: Option<String>,
    pub(super) observed_process_id: Option<u32>,
    pub(super) observed_process_name: Option<String>,
    pub(super) observed_at_ms: Option<u64>,
    pub(super) launch_elapsed_ms: Option<u64>,
    pub(super) elapsed_ms: u64,
    pub(super) timed_out: bool,
    pub(super) completed_reason: &'static str,
    pub(super) polls: u64,
    pub(super) candidate_process_ids: Vec<u32>,
    pub(super) exact_title_candidate_count: usize,
    pub(super) selection_reason: Option<&'static str>,
}

#[cfg(windows)]
impl WindowObservation {
    pub(super) fn not_attempted() -> Self {
        Self {
            attempted: false,
            window_observed: None,
            title_matched: None,
            observed_window_title: None,
            observed_process_id: None,
            observed_process_name: None,
            observed_at_ms: None,
            launch_elapsed_ms: None,
            elapsed_ms: 0,
            timed_out: false,
            completed_reason: "not-attempted",
            polls: 0,
            candidate_process_ids: Vec::new(),
            exact_title_candidate_count: 0,
            selection_reason: None,
        }
    }

    pub(super) fn timed_out(watchdog: &Watchdog, launch_elapsed_ms: u64) -> Self {
        Self {
            attempted: true,
            window_observed: Some(false),
            title_matched: None,
            observed_window_title: None,
            observed_process_id: None,
            observed_process_name: None,
            observed_at_ms: None,
            launch_elapsed_ms: Some(launch_elapsed_ms),
            elapsed_ms: watchdog.elapsed_ms(),
            timed_out: true,
            completed_reason: "timeout",
            polls: 0,
            candidate_process_ids: Vec::new(),
            exact_title_candidate_count: 0,
            selection_reason: None,
        }
    }
}

#[cfg(windows)]
pub(super) fn unproven_signals(observation: &WindowObservation) -> Vec<&'static str> {
    let mut signals = Vec::new();
    if observation.window_observed.is_none() {
        signals.push("windowObserved");
    }
    if observation.title_matched.is_none() {
        signals.push("titleMatched");
    }
    signals.extend([
        "issuesDialogObserved",
        "canvasRendered",
        "blankCanvasRejected",
        "refreshCompleted",
    ]);
    signals
}

#[cfg(windows)]
pub(super) fn observe_window(
    launched_pid: u32,
    baseline_process_ids: &[u32],
    project_name: &str,
    watchdog: &Watchdog,
    launch_elapsed_ms: u64,
) -> io::Result<WindowObservation> {
    let mut observation = WindowObservation::timed_out(watchdog, launch_elapsed_ms);
    observation.timed_out = false;
    observation.completed_reason = "polling";
    let baseline = baseline_process_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut candidate_ids = BTreeSet::new();

    loop {
        let remaining = watchdog.remaining();
        if remaining.is_zero() {
            observation.timed_out = true;
            observation.completed_reason = "timeout";
            break;
        }
        observation.polls += 1;
        let processes = match query_desktop_windows(remaining)? {
            Timed::Completed(processes) => processes,
            Timed::TimedOut => {
                observation.timed_out = true;
                observation.completed_reason = "timeout";
                break;
            }
        };
        let mut titled_candidates = processes
            .into_iter()
            .filter(|process| is_power_bi_desktop_process(&process.process_name))
            .filter(|process| {
                process.id == launched_pid
                    || !baseline.contains(&process.id)
                    || title_matches_project(&process.main_window_title, project_name)
            })
            .filter(|process| !process.main_window_title.trim().is_empty())
            .collect::<Vec<_>>();
        titled_candidates.sort_by_key(|process| process.id);
        for process in &titled_candidates {
            candidate_ids.insert(process.id);
        }
        let selection = select_window_candidate(
            &titled_candidates,
            launched_pid,
            baseline_process_ids,
            project_name,
        );
        observation.exact_title_candidate_count = observation
            .exact_title_candidate_count
            .max(selection.exact_title_candidate_count);
        if let Some(process) = selection.process {
            let matched = title_matches_project(&process.main_window_title, project_name);
            observation.window_observed = Some(true);
            observation.title_matched = Some(matched);
            observation.observed_window_title = Some(process.main_window_title);
            observation.observed_process_id = Some(process.id);
            observation.observed_process_name = Some(process.process_name);
            observation.selection_reason = selection.reason;
            observation
                .observed_at_ms
                .get_or_insert_with(|| watchdog.elapsed_ms());
            if matched {
                observation.completed_reason = "title-matched";
                break;
            }
        }
        let sleep_for = watchdog
            .remaining()
            .min(Duration::from_millis(WINDOW_POLL_INTERVAL_MS));
        if sleep_for.is_zero() {
            observation.timed_out = true;
            observation.completed_reason = "timeout";
            break;
        }
        std::thread::sleep(sleep_for);
    }

    observation.elapsed_ms = watchdog.elapsed_ms();
    observation.candidate_process_ids = candidate_ids.into_iter().collect();
    Ok(observation)
}

#[cfg(any(windows, test))]
struct WindowCandidateSelection {
    process: Option<ProcessWindow>,
    exact_title_candidate_count: usize,
    reason: Option<&'static str>,
}

#[cfg(any(windows, test))]
fn select_window_candidate(
    processes: &[ProcessWindow],
    launched_pid: u32,
    baseline_process_ids: &[u32],
    project_name: &str,
) -> WindowCandidateSelection {
    let exact = processes
        .iter()
        .filter(|process| title_matches_project(&process.main_window_title, project_name))
        .collect::<Vec<_>>();
    let exact_title_candidate_count = exact.len();

    let selected = exact
        .iter()
        .find(|process| process.id == launched_pid)
        .map(|process| ((*process).clone(), "association-launch-pid"))
        .or_else(|| {
            exact
                .iter()
                .find(|process| !baseline_process_ids.contains(&process.id))
                .map(|process| ((*process).clone(), "new-desktop-process"))
        })
        .or_else(|| (exact.len() == 1).then(|| (exact[0].clone(), "unique-title-fallback")))
        .or_else(|| {
            processes
                .iter()
                .find(|process| process.id == launched_pid)
                .map(|process| (process.clone(), "association-launch-diagnostic"))
        })
        .or_else(|| {
            processes
                .iter()
                .find(|process| !baseline_process_ids.contains(&process.id))
                .map(|process| (process.clone(), "new-process-diagnostic"))
        });

    WindowCandidateSelection {
        process: selected.as_ref().map(|(process, _)| process.clone()),
        exact_title_candidate_count,
        reason: selected.map(|(_, reason)| reason),
    }
}

#[cfg(any(windows, test))]
pub(super) fn managed_session_process_id(
    title_matched: Option<bool>,
    observed_process_id: Option<u32>,
    baseline_process_ids: &[u32],
) -> Option<u32> {
    if title_matched != Some(true) {
        return None;
    }
    observed_process_id.filter(|process_id| !baseline_process_ids.contains(process_id))
}

#[cfg(any(windows, test))]
fn title_matches_project(title: &str, project_name: &str) -> bool {
    let title = normalize_window_title(title);
    let project_name = normalize_window_title(project_name);
    if project_name.is_empty() {
        return false;
    }
    if title == project_name {
        return true;
    }
    [" - ", " – ", " — "].iter().any(|separator| {
        title
            .rsplit_once(separator)
            .is_some_and(|(stem, suffix)| stem == project_name && suffix == "power bi desktop")
    })
}

#[cfg(any(windows, test))]
fn normalize_window_title(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(any(windows, test))]
fn is_power_bi_desktop_process(process_name: &str) -> bool {
    process_name
        .trim()
        .to_ascii_lowercase()
        .starts_with("pbidesktop")
}

#[cfg(windows)]
pub(super) fn snapshot_desktop_process_ids(timeout: Duration) -> io::Result<Timed<Vec<u32>>> {
    let script = render_process_snapshot_script();
    match run_powershell(&script, timeout)? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "Desktop process snapshot")?;
            let processes: Vec<ProcessWindow> = parse_powershell_json(&output.stdout)?;
            Ok(Timed::Completed(
                processes.into_iter().map(|process| process.id).collect(),
            ))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const PROCESS_SNAPSHOT_SCRIPT: &str = r#"
$items = @(
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessName -like 'PBIDesktop*' -or $_.ProcessName -eq 'msmdsrv' } |
        ForEach-Object {
            [pscustomobject]@{
                id = [int]$_.Id
                processName = [string]$_.ProcessName
                mainWindowTitle = [string]$_.MainWindowTitle
            }
        }
)
[Console]::Out.Write((ConvertTo-Json -InputObject $items -Compress))
"#;

#[cfg(any(windows, test))]
pub(super) fn render_process_snapshot_script() -> String {
    PROCESS_SNAPSHOT_SCRIPT.to_string()
}

#[cfg(windows)]
fn query_desktop_windows(timeout: Duration) -> io::Result<Timed<Vec<ProcessWindow>>> {
    let script = render_window_query_script();
    match run_powershell(&script, timeout)? {
        Timed::Completed(output) => {
            ensure_powershell_success(&output, "Desktop window query")?;
            Ok(Timed::Completed(parse_powershell_json(&output.stdout)?))
        }
        Timed::TimedOut => Ok(Timed::TimedOut),
    }
}

#[cfg(any(windows, test))]
const WINDOW_QUERY_SCRIPT: &str = r#"
$items = @(
    Get-Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessName -like 'PBIDesktop*' } |
        ForEach-Object {
            [pscustomobject]@{
                id = [int]$_.Id
                processName = [string]$_.ProcessName
                mainWindowTitle = [string]$_.MainWindowTitle
            }
        }
)
[Console]::Out.Write((ConvertTo-Json -InputObject $items -Compress))
"#;

#[cfg(any(windows, test))]
pub(super) fn render_window_query_script() -> String {
    WINDOW_QUERY_SCRIPT.to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        ProcessWindow, is_power_bi_desktop_process, managed_session_process_id,
        select_window_candidate, title_matches_project,
    };

    #[test]
    fn title_matching_uses_exact_normalized_project_stem() {
        // Committed Desktop proof artifacts record the plain project stem.
        assert!(title_matches_project(
            "WorkshopOperations",
            "workshopoperations"
        ));
        assert!(title_matches_project(
            "WorkshopOperations - Power BI Desktop",
            "workshopoperations"
        ));
        assert!(title_matches_project(
            "  WorkshopOperations   –   Power BI Desktop  ",
            "WorkshopOperations"
        ));
        assert!(title_matches_project(
            "WorkshopOperations — Power BI Desktop",
            "WorkshopOperations"
        ));
        assert!(!title_matches_project(
            "OtherReport - Power BI Desktop",
            "WorkshopOperations"
        ));
        assert!(!title_matches_project(
            "AnnualSales - Power BI Desktop",
            "Sales"
        ));
        assert!(!title_matches_project(
            "Sales Dashboard - Power BI Desktop",
            "Sales"
        ));
        assert!(!title_matches_project(
            "Sales - Power BI Desktop Preview",
            "Sales"
        ));
        assert!(!title_matches_project("Power BI Desktop", ""));
    }

    #[test]
    fn every_window_candidate_must_be_a_desktop_process() {
        assert!(is_power_bi_desktop_process("PBIDesktop"));
        assert!(is_power_bi_desktop_process("PBIDesktopStore"));
        assert!(!is_power_bi_desktop_process("explorer"));
        assert!(!is_power_bi_desktop_process("msmdsrv"));
    }

    #[test]
    fn duplicate_titles_prefer_the_new_process_instead_of_the_oldest_pid() {
        let processes = vec![
            ProcessWindow {
                id: 17264,
                process_name: "PBIDesktop".to_string(),
                main_window_title: "SafetyDashboard".to_string(),
            },
            ProcessWindow {
                id: 37004,
                process_name: "PBIDesktop".to_string(),
                main_window_title: "SafetyDashboard - Power BI Desktop".to_string(),
            },
        ];
        let selected = select_window_candidate(&processes, 999, &[17264], "SafetyDashboard");

        let process = selected.process.expect("new process");
        assert_eq!(process.id, 37004);
        assert_eq!(process.process_name, "PBIDesktop");
        assert_eq!(selected.reason, Some("new-desktop-process"));
        assert_eq!(selected.exact_title_candidate_count, 2);
    }

    #[test]
    fn duplicate_baseline_titles_are_ambiguous_instead_of_guessed() {
        let processes = vec![
            ProcessWindow {
                id: 100,
                process_name: "PBIDesktop".to_string(),
                main_window_title: "SameReport".to_string(),
            },
            ProcessWindow {
                id: 200,
                process_name: "PBIDesktopStore".to_string(),
                main_window_title: "SameReport".to_string(),
            },
        ];
        let selected = select_window_candidate(&processes, 999, &[100, 200], "SameReport");

        assert!(selected.process.is_none());
        assert_eq!(selected.reason, None);
        assert_eq!(selected.exact_title_candidate_count, 2);
    }

    #[test]
    fn managed_session_never_owns_a_unique_baseline_window() {
        assert_eq!(
            managed_session_process_id(Some(true), Some(41), &[41]),
            None
        );
        assert_eq!(
            managed_session_process_id(Some(true), Some(42), &[41]),
            Some(42)
        );
        assert_eq!(managed_session_process_id(Some(false), Some(42), &[]), None);
    }
}
