//! Event operations borrow the same committing transaction as structured state.
//! They never enqueue another writer request, select a workflow, or dispatch it.

use super::{AppStateError, AppStateOperation};
use chariox_app_runtime::app_outbox::{AppOutbox, EventCatalog, Occurrence, OutboxError, Receipt};
use rusqlite::Transaction;
use std::sync::Arc;

pub(super) fn accept(
    tx: &mut Transaction<'_>,
    catalog: &Arc<EventCatalog>,
    owner: &str,
    occurrences: &[Occurrence],
) -> Result<Vec<Receipt>, AppStateError> {
    if occurrences.is_empty() {
        return Ok(Vec::new());
    }
    Ok(AppOutbox::apply_current_in(
        tx,
        catalog.clone(),
        owner,
        occurrences,
        crate::session::unix_epoch_ms(),
    )?)
}

pub(super) fn apply(
    tx: &mut Transaction<'_>,
    catalog: &Arc<EventCatalog>,
    owner: &str,
    operation: AppStateOperation,
) -> Result<Receipt, AppStateError> {
    match operation {
        AppStateOperation::Emit(occurrence) => accept(tx, catalog, owner, &[occurrence])?
            .pop()
            .ok_or_else(|| OutboxError::Corrupt.into()),
        AppStateOperation::Status { receipt_id } => {
            Ok(AppOutbox::status_in(tx, catalog, owner, &receipt_id)?)
        }
        AppStateOperation::Retry { receipt_id } => Ok(AppOutbox::reconcile_retry_in(
            tx,
            catalog,
            owner,
            &receipt_id,
            crate::session::unix_epoch_ms(),
        )?),
        _ => Err(OutboxError::Invalid.into()),
    }
}
