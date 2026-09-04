//! Operation-kernel registration for report visual mutations.

use super::{MutationPayload, Op, OpKernel, OpOutcome, Transaction, apply_legacy};
use crate::{CliError, CliResult};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct VisualKernel;

impl OpKernel for VisualKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let (payload, command): (&MutationPayload, &[&str]) = match operation {
            Op::SetBindings(payload) => (payload, &["report", "visuals", "set-bindings"]),
            Op::SetDisplayName(payload) => (payload, &["report", "visuals", "set-display-name"]),
            Op::SetTopNGuard(payload) => (payload, &["report", "visuals", "set-topn-guard"]),
            Op::SetDrilldownHierarchy(payload) => {
                (payload, &["report", "drilldown", "set-hierarchy"])
            }
            _ => {
                return Err(CliError::invalid_args(format!(
                    "VisualKernel cannot apply operation `{}`",
                    operation.tag()
                )));
            }
        };
        apply_legacy(operation, payload, transaction, command)
    }
}
