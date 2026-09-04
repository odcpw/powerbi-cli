//! Operation-kernel registration for report page mutations.

use super::{MutationPayload, Op, OpKernel, OpOutcome, Transaction, apply_legacy};
use crate::{CliError, CliResult};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PageKernel;

impl OpKernel for PageKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let (payload, command): (&MutationPayload, &[&str]) = match operation {
            Op::AddPage(payload) => (payload, &["report", "pages", "add"]),
            Op::UpdatePage(payload) => (payload, &["report", "pages", "update"]),
            Op::ReorderPages(payload) => (payload, &["report", "pages", "reorder"]),
            Op::SetActivePage(payload) => (payload, &["report", "pages", "set-active"]),
            Op::DeleteEmptyPage(payload) => (payload, &["report", "pages", "delete-empty"]),
            Op::ClonePage(payload) => (payload, &["report", "pages", "clone"]),
            _ => {
                return Err(CliError::invalid_args(format!(
                    "PageKernel cannot apply operation `{}`",
                    operation.tag()
                )));
            }
        };
        apply_legacy(operation, payload, transaction, command)
    }
}
