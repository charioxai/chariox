//! Namespace routing on the worker's one existing SDK channel. Identity and
//! data descriptors come from the actual process, never from request fields.
use super::{
    app_files_broker::AppFilesBroker,
    app_http::{AppHttpBroker, HttpContext},
    app_state_broker::AppStorageBroker,
    app_worker::AppWorkerError,
};
use crate::durable_state::DurableKernelStateStore;
use chariox_app_package::VerifiedPackage;
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
    package: &VerifiedPackage<'_>,
    http: HttpContext,
) -> Result<Arc<dyn Broker>, AppWorkerError> {
    Ok(Arc::new(build(
        store, owner, catalog, admission, process, package, http,
    )?))
}
fn build(
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    process: &WorkerProcess,
    package: &VerifiedPackage<'_>,
    http: HttpContext,
) -> Result<BackendBroker, AppWorkerError> {
    let data = process
        .private_data()
        .map_err(|_| AppWorkerError::Unavailable)?;
    if data.installation_id() != catalog.installation_id()
        || data.generation() != catalog.generation()
        || data.release_digest() != catalog.app_catalog().package_digest()
    {
        return Err(AppWorkerError::Identity);
    }
    Ok(BackendBroker {
        state: AppStorageBroker::new(
            store.clone(),
            owner.clone(),
            catalog.clone(),
            admission.clone(),
        ),
        http: AppHttpBroker::new(
            store.clone(),
            owner.clone(),
            catalog.app_catalog().clone(),
            package,
            data.clone(),
            admission.clone(),
            http,
        )
        .map_err(|_| AppWorkerError::Identity)?,
        files: AppFilesBroker::new(store, owner, catalog, admission, data),
    })
}
#[derive(Clone)]
struct BackendBroker {
    state: AppStorageBroker,
    files: AppFilesBroker,
    http: AppHttpBroker,
}
impl Broker for BackendBroker {
    fn take_response_guard(
        &self,
        id: &str,
    ) -> Option<Box<dyn chariox_app_runtime::worker_peer::ResponsePublication>> {
        self.http.take_response_guard(id)
    }
    fn begin_draining(&self) {
        self.http.begin_draining();
    }
    fn drain(&self) -> chariox_app_runtime::worker_peer::BrokerDrainFuture {
        let http = self.http.clone();
        Box::pin(async move {
            http.drain().await;
        })
    }
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let delegate = self.clone();
        Box::pin(async move {
            match request.method.as_str() {
                name if name.starts_with("state.") || name.starts_with("events.") => {
                    delegate.state.dispatch(request).await
                }
                name if name.starts_with("http.") => delegate.http.dispatch(request).await,
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

#[cfg(test)]
pub(crate) fn fixture_http(
    store: DurableKernelStateStore,
    owner: String,
    catalog: Arc<EventCatalog>,
    admission: Arc<Semaphore>,
    process: &WorkerProcess,
    package: &VerifiedPackage<'_>,
    http: HttpContext,
    network: Arc<super::app_http::fixture::NetworkFixture>,
) -> Result<Arc<dyn Broker>, AppWorkerError> {
    let broker = build(store, owner, catalog, admission, process, package, http)?;
    broker.http.fixture_network(network);
    Ok(Arc::new(broker))
}
