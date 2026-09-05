//! Operation-kernel registration for semantic-model and source-template
//! mutations. The compatibility implementation lives in `legacy`; this
//! family wrapper keeps model dispatch isolated from report families.

use super::{MutationPayload, Op, OpKernel, OpOutcome, Transaction, apply_legacy};
use crate::{CliError, CliResult};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ModelKernel;

impl OpKernel for ModelKernel {
    fn apply(&mut self, operation: &Op, transaction: &mut Transaction) -> CliResult<OpOutcome> {
        let (payload, command): (&MutationPayload, &[&str]) = match operation {
            Op::AddCalculatedColumn(payload) => (payload, &["model", "calculated-columns", "add"]),
            Op::AddStaticTable(payload) => (payload, &["model", "tables", "add-static"]),
            Op::SetSortBy(payload) => (payload, &["model", "columns", "set-sort-by"]),
            Op::SourceTemplateApply(payload) => (payload, &["source-template", "apply"]),
            _ => {
                return Err(CliError::invalid_args(format!(
                    "ModelKernel cannot apply operation `{}`",
                    operation.tag()
                )));
            }
        };
        apply_legacy(operation, payload, transaction, command)
    }
}
