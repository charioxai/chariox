//! Structured App operations share the durable writer and its current signer
//! fence. The worker service retains admission until completion, including when
//! an awaiting SDK call is cancelled. No App payload provides its own catalog.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::error::DaemonError;
use crate::runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped};
use chariox_app_runtime::{
    app_catalog::{AppCatalog, CatalogError},
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
    Storage(#[from] DaemonError),
}

#[derive(Debug)]
pub(crate) enum AppStateOperation {
    Get { key: String },
    Transaction(StateChanges),
}

#[derive(Debug, PartialEq)]
pub(crate) enum AppStateOutcome {
    Value(Option<StateRecord>),
    Revision(u64),
}

pub(super) struct AppStateRequest {
    owner: String,
    catalog: Arc<AppCatalog>,
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
            .field(
                "operation",
                &match self.operation {
                    AppStateOperation::Get { .. } => "get",
                    AppStateOperation::Transaction(_) => "transaction",
                },
            )
            .finish_non_exhaustive()
    }
}

impl DurableKernelStateStore {
    /// Worker-internal operation. `catalog` belongs to that admitted worker,
    /// and owner comes from kernel authentication, never an App argument.
    /// This blocking method does not grant a capability or accept an event.
    pub(crate) fn execute_app_state(
        &self,
        trusted_owner: &str,
        catalog: Arc<AppCatalog>,
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
    catalog: &AppCatalog,
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
    catalog.require_current(&transaction, owner)?;
    // BEGIN IMMEDIATE may itself wait for a writer lock. Check again after
    // acquiring it, immediately before state work. Once this check admits the
    // transaction, cancellation cannot promise that its commit was undone.
    budget.check()?;
    let outcome = match operation {
        AppStateOperation::Get { key } => {
            AppStateOutcome::Value(ManagedStateStore::read_in(&transaction, scope, &key)?)
        }
        AppStateOperation::Transaction(changes) => AppStateOutcome::Revision(
            ManagedStateStore::apply_in(&mut transaction, scope, &changes)?,
        ),
    };
    transaction.commit().map_err(StateError::from)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn fixture_catalog(store: &DurableKernelStateStore) -> Arc<AppCatalog> {
    tests::catalog(store)
}
