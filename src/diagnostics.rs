//! Structured diagnostics shared by native validation and spec compilation.
//!
//! A finding always carries the source file and an RFC 6901 JSON pointer.  An
//! empty pointer identifies the document root; it is intentionally used for
//! filesystem/TMDL findings and malformed JSON, where a more specific JSON
//! node cannot be resolved.

use serde::Serialize;
use std::path::Path;

/// One actionable validation or compilation diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Finding {
    pub(crate) code: String,
    pub(crate) message: String,
    /// The source file associated with this finding.
    pub(crate) path: String,
    /// RFC 6901 pointer into `path`; `""` denotes the document root.
    pub(crate) pointer: String,
    pub(crate) severity: String,
}

impl Finding {
    pub(crate) fn new(
        code: impl Into<String>,
        severity: impl Into<String>,
        message: impl Into<String>,
        path: impl Into<String>,
        pointer: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path: path.into(),
            pointer: pointer.into(),
            severity: severity.into(),
        }
    }

    pub(crate) fn error(
        code: impl Into<String>,
        message: impl Into<String>,
        path: &Path,
        pointer: impl Into<String>,
    ) -> Self {
        Self::new(
            code,
            "error",
            message,
            path.to_string_lossy().into_owned(),
            pointer,
        )
    }

    pub(crate) fn warning(
        code: impl Into<String>,
        message: impl Into<String>,
        path: &Path,
        pointer: impl Into<String>,
    ) -> Self {
        Self::new(
            code,
            "warning",
            message,
            path.to_string_lossy().into_owned(),
            pointer,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn info(
        code: impl Into<String>,
        message: impl Into<String>,
        path: &Path,
        pointer: impl Into<String>,
    ) -> Self {
        Self::new(
            code,
            "info",
            message,
            path.to_string_lossy().into_owned(),
            pointer,
        )
    }
}

/// Escape one JSON Pointer token according to RFC 6901.
#[allow(dead_code)]
pub(crate) fn escape_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::{Finding, escape_pointer_token};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn finding_serializes_all_stable_fields() {
        let finding = Finding::error(
            "validation.example",
            "message remains byte-identical",
            Path::new("report.json"),
            "/themeCollection/custom~0Theme/a~1b",
        );
        assert_eq!(
            serde_json::to_value(finding).expect("serialize finding"),
            json!({
                "code": "validation.example",
                "message": "message remains byte-identical",
                "path": "report.json",
                "pointer": "/themeCollection/custom~0Theme/a~1b",
                "severity": "error"
            })
        );
    }

    #[test]
    fn pointer_tokens_escape_reserved_characters() {
        assert_eq!(escape_pointer_token("a~/b"), "a~0~1b");
    }
}
