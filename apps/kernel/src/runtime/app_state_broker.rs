//! Strict SDK state and event operations through the existing kernel durable writer.
//! This internal delegate neither acknowledges worker readiness nor publishes a
//! protocol endpoint. The admitted worker owns its catalog and identity.

mod decode;
mod errors;
mod events;
mod receipt;
#[cfg(test)]
mod tests;

use super::app_operation_budget::{AppOperationBudget, AppOperationStopped};
use crate::durable_state::{
    app_state::{AppStateOperation, AppStateOutcome},
    DurableKernelStateStore,
};
use chariox_app_runtime::{
    app_outbox::EventCatalog, wire::RemoteError, worker_peer::BrokerRequest,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};

#[derive(Clone)]
pub(crate) struct AppStorageBroker {
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    fence: Arc<RwLock<()>>,
    #[cfg(test)]
    budget_observer: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl AppStorageBroker {
    /// `admission` is AppControl's existing shared eight-operation semaphore.
    /// Waiting stays inside the peer's bounded broker handlers.
    pub(crate) fn new(
        store: DurableKernelStateStore,
        trusted_owner: String,
        catalog: Arc<EventCatalog>,
        admission: Arc<Semaphore>,
        fence: Arc<RwLock<()>>,
    ) -> Self {
        Self {
            store,
            owner: trusted_owner,
            catalog,
            admission,
            fence,
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
        // A burst from one App can occupy all eight shared slots. Keep the
        // neighbour in its already bounded peer handler instead of refusing it
        // at that instant. FIFO semaphore admission preserves the global cap.
        let mut cancellation = request.cancellation.clone();
        let permit = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(errors::stopped(AppOperationStopped::Cancelled)),
            _ = tokio::time::sleep_until(request.deadline) => return Err(errors::stopped(AppOperationStopped::Deadline)),
            permit = self.admission.clone().acquire_owned() => permit.map_err(|_| errors::unavailable())?,
        };
        // Snapshots take admission before the exclusive fence. Waiting storage
        // must use the same order so it cannot prevent a snapshot from finishing.
        let write = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(errors::stopped(AppOperationStopped::Cancelled)),
            _ = tokio::time::sleep_until(request.deadline) => return Err(errors::stopped(AppOperationStopped::Deadline)),
            write = self.fence.clone().read_owned() => write,
        };
        budget.check().map_err(errors::stopped)?;
        let service = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            // The closure owns admission even if the async caller disappears.
            // Writer-side budget checks stop newly starting expired work; once
            // a commit starts we await its actual outcome, without rollback claims.
            let _permit = permit;
            let _write = write;
            service
                .store
                .execute_app_state(&service.owner, service.catalog, operation, budget)
        })
        .await
        .map_err(|_| errors::unavailable())?
        .map_err(errors::state)?;
        Ok(match result {
            // The SDK has no rewind; only the lifecycle issues one.
            AppStateOutcome::Value(None) | AppStateOutcome::Rewound(_) => Value::Null,
            AppStateOutcome::Value(Some(record)) => {
                json!({"value":record.value,"version":record.version})
            }
            AppStateOutcome::Transaction { revision, receipts } => {
                let receipts: Vec<_> = receipts.iter().map(receipt::value).collect();
                json!({"revision":revision,"receipts":receipts})
            }
            AppStateOutcome::Receipt(receipt) => receipt::value(&receipt),
            AppStateOutcome::Wakes(wakes) => json!({ "wakes": wakes }),
        })
    }
}
