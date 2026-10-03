//! Final single-file publication on the existing installation/signer writer.
//! File contents were staged and synced before queueing. The DB transaction is
//! read-only: it fences identity and never claims a filesystem/state transaction.
use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::{
    error::DaemonError,
    runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped},
};
use chariox_app_runtime::{
    app_catalog::CatalogError,
    app_outbox::EventCatalog,
    worker_process::{PreparedDataReplace, PrivateDataError},
};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::{mpsc, Arc};
#[derive(Debug, thiserror::Error)]
pub(crate) enum AppFileError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    File(#[from] PrivateDataError),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}
pub(super) struct AppFileRequest {
    owner: String,
    catalog: Arc<EventCatalog>,
    staged: PreparedDataReplace,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<(), AppFileError>>,
    #[cfg(test)]
    fault: Option<Arc<TestPublicationFault>>,
}
/// Request-local fault injection after a real successful filesystem effect.
/// Neither this type nor a fault selector exists in production or SDK params.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestPublicationFault {
    pub(crate) drop_reply: bool,
    pub(crate) publications: std::sync::atomic::AtomicUsize,
    pub(crate) queued: Option<Arc<dyn Fn() + Send + Sync>>,
}
impl std::fmt::Debug for AppFileRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppFileRequest")
            .field("installation_id", &self.catalog.installation_id())
            .field("generation", &self.catalog.generation())
            .finish_non_exhaustive()
    }
}
impl DurableKernelStateStore {
    pub(crate) fn publish_app_file(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        staged: PreparedDataReplace,
        budget: AppOperationBudget,
    ) -> Result<(), AppFileError> {
        self.publish_app_file_inner(
            owner,
            catalog,
            staged,
            budget,
            #[cfg(test)]
            None,
        )
    }
    #[cfg(test)]
    pub(crate) fn fixture_publish_app_file(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        staged: PreparedDataReplace,
        budget: AppOperationBudget,
        fault: Arc<TestPublicationFault>,
    ) -> Result<(), AppFileError> {
        self.publish_app_file_inner(owner, catalog, staged, budget, Some(fault))
    }
    fn publish_app_file_inner(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        staged: PreparedDataReplace,
        budget: AppOperationBudget,
        #[cfg(test)] fault: Option<Arc<TestPublicationFault>>,
    ) -> Result<(), AppFileError> {
        budget.check()?;
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppFile(Box::new(AppFileRequest {
                owner: owner.into(),
                catalog,
                staged,
                budget,
                response,
                #[cfg(test)]
                fault: fault.clone(),
            })))?;
        #[cfg(test)]
        if let Some(queued) = fault.as_ref().and_then(|fault| fault.queued.as_ref()) {
            queued();
        }
        receiver
            .recv()
            .map_err(|_| PrivateDataError::OutcomeUncertain)?
    }
}
pub(super) fn execute(connection: &mut Connection, request: AppFileRequest) {
    let result = apply(
        connection,
        &request.owner,
        &request.catalog,
        request.staged,
        &request.budget,
    );
    #[cfg(test)]
    if result.is_ok() {
        if let Some(fault) = &request.fault {
            fault
                .publications
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if fault.drop_reply {
                return;
            }
        }
    }
    let _ = request.response.send(result);
}
fn apply(
    connection: &mut Connection,
    owner: &str,
    catalog: &EventCatalog,
    staged: PreparedDataReplace,
    budget: &AppOperationBudget,
) -> Result<(), AppFileError> {
    budget.check()?;
    if staged.data().installation_id() != catalog.installation_id()
        || staged.data().generation() != catalog.generation()
        || staged.data().release_digest() != catalog.app_catalog().package_digest()
    {
        return Err(PrivateDataError::Identity.into());
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| DaemonError::LocalTransport {
            operation: "durable_state.app_file",
            message: "Cannot fence current App identity".into(),
        })?;
    catalog.app_catalog().require_current(&transaction, owner)?;
    budget.check()?;
    staged.publish()?;
    // No DB mutation/commit follows the effect. The held transaction simply
    // releases the exact current-generation fence after parent-directory sync.
    drop(transaction);
    Ok(())
}
