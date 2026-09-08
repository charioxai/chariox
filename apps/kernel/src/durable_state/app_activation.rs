//! A single-use confirmation of the committed generation on the kernel writer.
//! This is combined with the actual supervised worker and its SDK registration;
//! it does not replace continuing invocation checks or confer process ownership.

use super::{DurableKernelStateStore, DurableWriterRequest};
use crate::{
    error::DaemonError,
    runtime::app_operation_budget::{AppOperationBudget, AppOperationStopped},
};
use chariox_app_runtime::{app_catalog::CatalogError, app_outbox::EventCatalog};
use rusqlite::{Connection, TransactionBehavior};
use std::sync::{mpsc, Arc};

pub(crate) struct CommittedAppActivation {
    owner: String,
    catalog: Arc<EventCatalog>,
}
impl CommittedAppActivation {
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
    pub(crate) fn installation_id(&self) -> &str {
        self.catalog.installation_id()
    }
    pub(crate) fn generation(&self) -> u64 {
        self.catalog.generation()
    }
    pub(crate) fn package_digest(&self) -> &str {
        self.catalog.app_catalog().package_digest()
    }
    pub(crate) fn catalog_digest(&self) -> &str {
        self.catalog.app_catalog().catalog_digest()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppActivationError {
    #[error(transparent)]
    Stopped(#[from] AppOperationStopped),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("app_activation_database")]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Storage(#[from] DaemonError),
}
type Result<T> = std::result::Result<T, AppActivationError>;

pub(super) struct AppActivationRequest {
    owner: String,
    catalog: Arc<EventCatalog>,
    budget: AppOperationBudget,
    response: mpsc::Sender<Result<CommittedAppActivation>>,
}
impl std::fmt::Debug for AppActivationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppActivationRequest")
            .finish_non_exhaustive()
    }
}
impl DurableKernelStateStore {
    /// Blocking, called by the retained activation owner after SDK registration
    /// and the verified installation commit. An offline binding cannot call this
    /// into a running worker: the returned proof carries no process or channel.
    pub(crate) fn confirm_app_activation(
        &self,
        owner: &str,
        catalog: Arc<EventCatalog>,
        budget: AppOperationBudget,
    ) -> Result<CommittedAppActivation> {
        budget.check()?;
        if owner.is_empty() || owner.len() > 128 || owner.chars().any(char::is_control) {
            return Err(CatalogError::Stale.into());
        }
        let (response, receiver) = mpsc::channel();
        self.writer
            .enqueue(DurableWriterRequest::AppActivation(Box::new(
                AppActivationRequest {
                    owner: owner.into(),
                    catalog,
                    budget,
                    response,
                },
            )))?;
        receiver.recv().map_err(|_| DaemonError::LocalTransport {
            operation: "durable_state.await_app_activation",
            message: "App activation confirmation did not complete".into(),
        })?
    }
}
pub(super) fn execute(connection: &mut Connection, request: AppActivationRequest) {
    let result = (|| {
        request.budget.check()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Includes retained publisher enrollment, exact active release, generation
        // and recorded capability approval. Recheck after any SQLite lock wait.
        request
            .catalog
            .app_catalog()
            .require_current(&tx, &request.owner)?;
        request.budget.check()?;
        tx.commit()?;
        Ok(CommittedAppActivation {
            owner: request.owner,
            catalog: request.catalog,
        })
    })();
    let _ = request.response.send(result);
}

#[cfg(test)]
mod tests;
