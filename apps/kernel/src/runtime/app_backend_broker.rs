//! Namespace routing on the worker's one existing SDK channel. Identity and
//! data descriptors come from the actual process, never from request fields.
use super::{
    app_files_broker::AppFilesBroker, app_state_broker::AppStorageBroker,
    app_worker::AppWorkerError,
};
use crate::durable_state::DurableKernelStateStore;
use chariox_app_runtime::{
    app_outbox::EventCatalog,
    wire::RemoteError,
    worker_peer::{Broker, BrokerFuture, BrokerRequest},
    worker_process::WorkerProcess,
};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub(crate) fn broker(
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    process: &WorkerProcess,
) -> Result<Arc<dyn Broker>, AppWorkerError> {
    let data = process
        .private_data()
        .map_err(|_| AppWorkerError::Unavailable)?;
    if data.installation_id() != catalog.installation_id()
        || data.generation() != catalog.generation()
        || data.release_digest() != catalog.app_catalog().package_digest()
    {
        return Err(AppWorkerError::Identity);
    }
    Ok(Arc::new(BackendBroker {
        state: AppStorageBroker::new(
            store.clone(),
            owner.clone(),
            catalog.clone(),
            admission.clone(),
        ),
        files: AppFilesBroker::new(store, owner, catalog, admission, data),
    }))
}
#[derive(Clone)]
struct BackendBroker {
    state: AppStorageBroker,
    files: AppFilesBroker,
}
impl Broker for BackendBroker {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let delegate = self.clone();
        Box::pin(async move {
            match request.method.as_str() {
                name if name.starts_with("state.") || name.starts_with("events.") => {
                    delegate.state.dispatch(request).await
                }
                "files.atomic_replace" => delegate.files.dispatch(request).await,
                _ => Err(RemoteError {
                    code: "METHOD_UNAVAILABLE".into(),
                    message: "App capability is unavailable".into(),
                    retryable: Some(false),
                }),
            }
        })
    }
}
