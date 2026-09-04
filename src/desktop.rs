//! Power BI Desktop command façade.

mod cleanup;
mod evidence;
mod harvest_reference;
mod launch;
mod observe;

#[cfg(windows)]
pub(crate) use cleanup::ProcessIdentity;
#[cfg(windows)]
pub(crate) use cleanup::{CLEANUP_TIMEOUT_MS, cleanup_spawned_processes, read_process_identity};
#[allow(unused_imports)]
pub(crate) use launch::PowerBiDesktopDetection;
#[cfg(windows)]
pub(crate) use launch::{Timed, run_command_with_timeout};
pub(crate) use launch::{desktop_command, detect_power_bi_desktop, ensure_desktop_platform};
