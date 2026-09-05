//! Operation-kernel registration for filter and slicer mutations.

use super::{MutationPayload, Op, OpKernel, OpOutcome, Transaction, apply_legacy};
use crate::{CliError, CliResult};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FilterKernel;

impl OpKernel for FilterKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let (payload, command): (&MutationPayload, &[&str]) = match operation {
            Op::UpdateFilter(payload) => (payload, &["report", "filters", "update"]),
            Op::DeleteFilter(payload) => (payload, &["report", "filters", "delete"]),
            Op::ClearFilter(payload) => (payload, &["report", "filters", "clear"]),
            Op::SlicerClear(payload) => (payload, &["report", "slicers", "clear"]),
            _ => {
                return Err(CliError::invalid_args(format!(
                    "FilterKernel cannot apply operation `{}`",
                    operation.tag()
                )));
            }
        };
        apply_legacy(operation, payload, transaction, command)
    }
}
