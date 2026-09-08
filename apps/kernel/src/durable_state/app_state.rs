//! Structured App operations share the durable writer and its current signer
//! fence. The worker service retains admission until completion, including when
//! an awaiting SDK call is cancelled. No App payload provides its own catalog.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use crate::runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped};
use chariox_app_runtime::{
    app_catalog::CatalogError,
    app_outbox::{AppOutbox, EventCatalog, Occurrence, OutboxError, Receipt},
    managed_state::{ManagedStateStore, StateChanges, StateError, StateRecord, StateScope},
};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::{mpsc, Arc};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppStateError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Outbox(#[from] OutboxError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}

pub(crate) enum AppStateOperation {
    Get {
        key: String,
    },
    Transaction {
        changes: StateChanges,
        occurrences: Vec<Occurrence>,
    },
    Emit(Occurrence),
    Status {
        receipt_id: String,
    },
    Retry {
        receipt_id: String,
    },
}
impl AppStateOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::Get { .. } => "get",
            Self::Transaction { .. } => "transaction",
            Self::Emit(_) => "emit",
            Self::Status { .. } => "status",
            Self::Retry { .. } => "retry",
        }
    }
}
impl std::fmt::Debug for AppStateOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum AppStateOutcome {
    Value(Option<StateRecord>),
    Transaction {
        revision: u64,
        receipts: Vec<Receipt>,
    },
    Receipt(Receipt),
}

pub(super) struct AppStateRequest {
    owner: String,
    catalog: Arc<EventCatalog>,
    operation: AppStateOperation,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<AppStateOutcome, AppStateError>>,
}

impl std::fmt::Debug for AppStateRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppStateRequest")
            .field("installation_id", &self.catalog.installation_id())
            .field("generation", &self.catalog.generation())
            .field("operation", &self.operation.name())
            .finish_non_exhaustive()
    }
}

impl DurableKernelStateStore {
    /// Worker-internal operation. `catalog` belongs to that admitted worker,
    /// and owner comes from kernel authentication, never an App argument.
    /// This blocking method grants no capability or workflow target. Event
    /// receipts and structured state are acknowledged only after their commit.
    pub(crate) fn execute_app_state(
        &self,
        trusted_owner: &str,
        catalog: Arc<EventCatalog>,
        operation: AppStateOperation,
        budget: AppOperationBudget,
    ) -> Result<AppStateOutcome, AppStateError> {
        budget.check()?;
        // Validate before allocating/queuing owner state. Exact enrollment and
        // active generation are repeated on the writer, not trusted from here.
        StateScope::new(
            trusted_owner,
            catalog.installation_id(),
            catalog.generation(),
        )?;
        match &operation {
            AppStateOperation::Transaction { occurrences, .. } if !occurrences.is_empty() => {
                AppOutbox::validate_occurrences(occurrences)?;
            }
            AppStateOperation::Emit(occurrence) => {
                AppOutbox::validate_occurrences(std::slice::from_ref(occurrence))?;
            }
            _ => {}
        }
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppState(Box::new(AppStateRequest {
                owner: trusted_owner.into(),
                catalog,
                operation,
                budget,
                response,
            })))?;
        receiver
            .recv()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "durable_state.await_app_state",
                message: error.to_string(),
            })?
    }
}

pub(super) fn initialize(connection: &mut Connection) -> Result<(), DaemonError> {
    ManagedStateStore::new(connection)
        .initialize()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "durable_state.migrate_app_state",
            message: error.to_string(),
        })
}

pub(super) fn execute(connection: &mut Connection, request: AppStateRequest) {
    let result = apply(
        connection,
        &request.owner,
        &request.catalog,
        request.operation,
        &request.budget,
    );
    let _ = request.response.send(result);
}

fn apply(
    connection: &mut Connection,
    owner: &str,
    catalog: &Arc<EventCatalog>,
    operation: AppStateOperation,
    budget: &AppOperationBudget,
) -> Result<AppStateOutcome, AppStateError> {
    // A request may have waited in the FIFO writer queue after IPC expiry.
    budget.check()?;
    let scope = StateScope::new(owner, catalog.installation_id(), catalog.generation())?;
    let mut transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(StateError::from)?;
    // Publisher revoke/re-enroll can invalidate an otherwise unchanged active
    // generation. Hold its exact retained verification fence through commit.
    catalog.app_catalog().require_current(&transaction, owner)?;
    // BEGIN IMMEDIATE may itself wait for a writer lock. Check again after
    // acquiring it, immediately before state work. Once this check admits the
    // transaction, cancellation cannot promise that its commit was undone.
    budget.check()?;
    let outcome = match operation {
        AppStateOperation::Get { key } => {
            AppStateOutcome::Value(ManagedStateStore::read_in(&transaction, scope, &key)?)
        }
        AppStateOperation::Transaction {
            changes,
            occurrences,
        } => {
            let revision = ManagedStateStore::apply_in(&mut transaction, scope, &changes)?;
            let receipts = events::accept(&mut transaction, catalog, owner, &occurrences)?;
            AppStateOutcome::Transaction { revision, receipts }
        }
        event => AppStateOutcome::Receipt(events::apply(&mut transaction, catalog, owner, event)?),
    };
    transaction.commit().map_err(StateError::from)?;
    Ok(outcome)
}

mod events;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn fixture_catalog(
    store: &DurableKernelStateStore,
) -> Arc<chariox_app_runtime::app_catalog::AppCatalog> {
    tests::catalog(store).app_catalog().clone()
}

#[cfg(test)]
pub(crate) fn fixture_event_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    tests::catalog(store)
}

#[cfg(test)]
pub(crate) fn fixture_event_package() -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::package()
}

#[cfg(test)]
pub(crate) fn fixture_tool_catalog(store: &DurableKernelStateStore) -> Arc<EventCatalog> {
    tests::tool_catalog(store)
}

#[cfg(test)]
pub(crate) fn fixture_tool_package() -> (Vec<u8>, chariox_app_package::TrustedPublisher) {
    tests::tool_package()
}
