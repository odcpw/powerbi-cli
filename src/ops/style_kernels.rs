//! Operation-kernel registration for theme, style, bookmark, and sanitise
//! mutations.

use super::{MutationPayload, Op, OpKernel, OpOutcome, Transaction, apply_legacy};
use crate::{CliError, CliResult};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct StyleKernel;

impl OpKernel for StyleKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let (payload, command): (&MutationPayload, &[&str]) = match operation {
            Op::ApplyThemeBundle(payload) => (payload, &["report", "themes", "apply"]),
            Op::ApplyStyleBundle(payload) => (payload, &["report", "style", "apply"]),
            Op::BookmarkMetadata(payload) => {
                let action = payload
                    .fields
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("set-display-name");
                (payload, &["report", "bookmarks", action])
            }
            Op::SanitizeAction(payload) => (payload, &["report", "sanitize", "apply"]),
            _ => {
                return Err(CliError::invalid_args(format!(
                    "StyleKernel cannot apply operation `{}`",
                    operation.tag()
                )));
            }
        };
        apply_legacy(operation, payload, transaction, command)
    }
}
