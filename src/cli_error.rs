use std::fs;
use std::path::Path;

pub(crate) const EXIT_SUCCESS: i32 = 0;
pub(crate) const EXIT_INVALID_ARGS: i32 = 2;
pub(crate) const EXIT_FILE_NOT_FOUND: i32 = 3;
pub(crate) const EXIT_VALIDATION_FAILED: i32 = 10;
pub(crate) const EXIT_PROOF_INCOMPLETE: i32 = 20;
pub(crate) const EXIT_ORACLE_UNAVAILABLE: i32 = 30;
pub(crate) const EXIT_ORACLE_FAILED: i32 = 40;
pub(crate) const EXIT_UNEXPECTED: i32 = 70;

#[derive(Debug)]
pub(crate) struct CliError {
    pub(crate) code: &'static str,
    pub(crate) exit_code: i32,
    pub(crate) message: String,
    pub(crate) hint: Option<String>,
    pub(crate) suggested_commands: Vec<String>,
    // Keep the frequently-carried error type below clippy's `result_large_err`
    // threshold. These diagnostics are optional and therefore pay for their
    // allocation only when a caller needs to attach one.
    pub(crate) pointer: Option<Box<str>>,
    pub(crate) did_you_mean: Option<Box<str>>,
}

impl CliError {
    pub(crate) fn invalid_args(message: impl Into<String>) -> Self {
        Self::new("invalid_args", EXIT_INVALID_ARGS, message)
    }

    pub(crate) fn file_not_found(message: impl Into<String>) -> Self {
        Self::new("file_not_found", EXIT_FILE_NOT_FOUND, message)
    }

    pub(crate) fn validation_failed(message: impl Into<String>) -> Self {
        Self::new("validation_failed", EXIT_VALIDATION_FAILED, message)
    }

    pub(crate) fn unsupported_feature(message: impl Into<String>) -> Self {
        Self::new("unsupported_feature", EXIT_INVALID_ARGS, message)
    }

    pub(crate) fn unexpected(message: impl Into<String>) -> Self {
        Self::new("unexpected", EXIT_UNEXPECTED, message)
    }

    pub(crate) fn new(code: &'static str, exit_code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            exit_code,
            message: message.into(),
            hint: None,
            suggested_commands: Vec::new(),
            pointer: None,
            did_you_mean: None,
        }
    }

    pub(crate) fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub(crate) fn with_suggested_command(mut self, command: impl Into<String>) -> Self {
        self.suggested_commands.push(command.into());
        self
    }

    pub(crate) fn with_pointer(mut self, pointer: impl Into<String>) -> Self {
        self.pointer = Some(pointer.into().into_boxed_str());
        self
    }

    pub(crate) fn with_did_you_mean(mut self, suggestion: impl Into<String>) -> Self {
        self.did_you_mean = Some(suggestion.into().into_boxed_str());
        self
    }
}

pub(crate) type CliResult<T> = Result<T, CliError>;

pub(crate) fn walkdir_entry(
    root: &Path,
    entry: Result<walkdir::DirEntry, walkdir::Error>,
    operation: &str,
) -> CliResult<walkdir::DirEntry> {
    entry.map_err(|err| {
        let failing_path = err.path().unwrap_or(root);
        CliError::unexpected(format!(
            "{operation} failed at {}: {err}",
            failing_path.display()
        ))
    })
}

pub(crate) fn read_dir_entry(
    directory: &Path,
    entry: std::io::Result<fs::DirEntry>,
    operation: &str,
) -> CliResult<fs::DirEntry> {
    entry.map_err(|err| {
        CliError::unexpected(format!(
            "{operation} failed while reading {}: {err}",
            directory.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use walkdir::WalkDir;

    #[test]
    fn walkdir_entry_accepts_accessible_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let raw_entry = WalkDir::new(temp.path())
            .into_iter()
            .next()
            .expect("root entry");
        let entry = walkdir_entry(temp.path(), raw_entry, "test walk").expect("walk entry");
        assert_eq!(entry.path(), temp.path());
    }

    #[test]
    fn walkdir_entry_reports_the_failing_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("missing");
        let raw_entry = WalkDir::new(&missing)
            .into_iter()
            .next()
            .expect("missing root error");
        let error =
            walkdir_entry(&missing, raw_entry, "test walk").expect_err("missing root must fail");
        assert!(error.message.contains("test walk failed at"));
        assert!(error.message.contains(&missing.display().to_string()));
    }
}
