//! Strict SDK state and event operations through the existing kernel durable writer.
//! This internal delegate neither acknowledges worker readiness nor publishes a
//! protocol endpoint. The admitted worker owns its catalog and identity.

mod decode;
mod errors;
mod events;
mod receipt;
#[cfg(test)]
mod tests;

use super::app_operation_budget::AppOperationBudget;
use crate::durable_state::{
    app_state::{AppStateOperation, AppStateOutcome},
    DurableKernelStateStore,
};
use chariox_app_runtime::{
    app_outbox::EventCatalog, wire::RemoteError, worker_peer::BrokerRequest,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(crate) struct AppStorageBroker {
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    #[cfg(test)]
    budget_observer: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl AppStorageBroker {
    /// `admission` is AppControl's existing shared eight-operation semaphore.
    /// No new queue or per-worker permit pool is created by this delegate.
    pub(crate) fn new(
        store: DurableKernelStateStore,
        trusted_owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
    ) -> Self {
        Self {
            store,
            owner: trusted_owner,
            catalog,
            admission,
            #[cfg(test)]
            budget_observer: None,
        }
    }

    /// The common worker broker delegates the SDK state and events namespaces.
    /// Params cannot choose owner, installation, generation, path or deadline.
    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        let budget = AppOperationBudget::from_broker(&request);
        #[cfg(test)]
        let budget = match &self.budget_observer {
            Some(observe) => budget.fixture_observe_checks(observe.clone()),
            None => budget,
        };
        budget.check().map_err(errors::stopped)?;
        let operation = decode::operation(&request.method, request.params)?;
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| errors::busy())?;
        let service = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            // The closure owns admission even if the async caller disappears.
            // Writer-side budget checks stop newly starting expired work; once
            // a commit starts we await its actual outcome, without rollback claims.
            let _permit = permit;
            service
                .store
                .execute_app_state(&service.owner, service.catalog, operation, budget)
        })
        .await
        .map_err(|_| errors::unavailable())?
        .map_err(errors::state)?;
        Ok(match result {
            AppStateOutcome::Value(None) => Value::Null,
            AppStateOutcome::Value(Some(record)) => {
                json!({"value":record.value,"version":record.version})
            }
            AppStateOutcome::Transaction { revision, receipts } => {
                let receipts: Vec<_> = receipts.iter().map(receipt::value).collect();
                json!({"revision":revision,"receipts":receipts})
            }
            AppStateOutcome::Receipt(receipt) => receipt::value(&receipt),
        })
    }
}
