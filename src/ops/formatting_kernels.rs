//! Operation-kernel registration for visual text, color, and formatting
//! bundle mutations.

use super::{MutationPayload, Op, OpKernel, OpOutcome, Transaction, apply_legacy};
use crate::{CliError, CliResult};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FormattingKernel;

impl OpKernel for FormattingKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let (payload, command): (&MutationPayload, &[&str]) = match operation {
            Op::SetText(payload) => (payload, &["report", "visuals", "formatting", "set-text"]),
            Op::SetColor(payload) => (payload, &["report", "visuals", "formatting", "set-color"]),
            Op::FormattingApply(payload) => {
                (payload, &["report", "visuals", "formatting", "apply"])
            }
            _ => {
                return Err(CliError::invalid_args(format!(
                    "FormattingKernel cannot apply operation `{}`",
                    operation.tag()
                )));
            }
        };
        apply_legacy(operation, payload, transaction, command)
    }
}
