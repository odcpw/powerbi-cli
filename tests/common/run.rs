//! Process runner for integration tests.

use serde_json::json;
use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Once;
use std::time::{Duration, Instant};

thread_local! {
    static RUN_HISTORY: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

static INSTALL_PANIC_LOGGER: Once = Once::new();

/// Complete observable result of one `powerbi-cli` invocation.
///
/// The compatibility `code` field equals `exit`; new tests should use `exit`.
#[derive(Debug)]
pub struct CliRun {
    pub argv: Vec<String>,
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
    pub elapsed: Duration,
    pub code: i32,
}

/// Backward-compatible name used by existing integration tests.
pub type RunOutput = CliRun;

/// Configures one logged `powerbi-cli` subprocess invocation.
#[derive(Debug)]
pub struct CliCommand {
    args: Vec<OsString>,
    current_dir: Option<PathBuf>,
    environment: Vec<(OsString, Option<OsString>)>,
}

/// Create a configurable, logged CLI command.
pub fn cli_command<I, S>(args: I) -> CliCommand
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    CliCommand {
        args: args
            .into_iter()
            .map(|arg| arg.as_ref().to_os_string())
            .collect(),
        current_dir: None,
        environment: Vec::new(),
    }
}

impl CliCommand {
    /// Run from a specific working directory.
    pub fn current_dir(mut self, path: impl AsRef<Path>) -> Self {
        self.current_dir = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set one environment variable for this invocation.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.environment.push((
            key.as_ref().to_os_string(),
            Some(value.as_ref().to_os_string()),
        ));
        self
    }

    /// Remove one inherited environment variable for this invocation.
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.environment.push((key.as_ref().to_os_string(), None));
        self
    }

    /// Execute and return the structured test result.
    pub fn run(self) -> CliRun {
        let (argv, elapsed, output) = self.execute();
        let exit = output.status.code().unwrap_or(-1);
        CliRun {
            argv,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit,
            elapsed,
            code: exit,
        }
    }

    /// Execute while retaining `std::process::Output` compatibility.
    ///
    /// Prefer [`Self::run`] in new tests. This form exists for tests that need
    /// byte-oriented output or `ExitStatus` APIs.
    pub fn output(self) -> Output {
        let (_, _, output) = self.execute();
        output
    }

    fn execute(self) -> (Vec<String>, Duration, Output) {
        install_panic_logger();
        let executable = env!("CARGO_BIN_EXE_powerbi-cli");
        let argv = std::iter::once(OsString::from(executable))
            .chain(self.args.iter().cloned())
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let mut command = Command::new(executable);
        command.args(&self.args);
        if let Some(current_dir) = self.current_dir {
            command.current_dir(current_dir);
        }
        for (key, value) in self.environment {
            if let Some(value) = value {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }

        let started = Instant::now();
        let output = command
            .output()
            .unwrap_or_else(|error| panic!("run powerbi-cli binary for argv {argv:?}: {error}"));
        let elapsed = started.elapsed();
        record_invocation(&argv, &output, elapsed);
        (argv, elapsed, output)
    }
}

/// Run one CLI invocation from the repository root.
pub fn run_powerbi(args: &[&str]) -> CliRun {
    cli_command(args).run()
}

/// Owned-string counterpart to [`run_powerbi`].
pub fn run_powerbi_owned(args: &[String]) -> CliRun {
    cli_command(args).run()
}

fn record_invocation(argv: &[String], output: &Output, elapsed: Duration) {
    let line = serde_json::to_string(&json!({
        "schema": "powerbi-cli.test-run.v1",
        "argv": argv,
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "exit": output.status.code().unwrap_or(-1),
        "elapsedMs": elapsed.as_millis(),
    }))
    .expect("serialize CLI test log");

    RUN_HISTORY.with(|history| {
        history.borrow_mut().push(line.clone());
    });
    if std::env::var("POWERBI_CLI_TEST_LOG").as_deref() == Ok("1") {
        eprintln!("{line}");
    }
}

fn install_panic_logger() {
    INSTALL_PANIC_LOGGER.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            RUN_HISTORY.with(|history| {
                for line in history.borrow().iter() {
                    eprintln!("{line}");
                }
            });
            previous(info);
        }));
    });
}
