//! HTTP namespace on the existing worker peer. Current authority is checked
//! by the durable writer before every socket start or stream operation enqueue.
use super::{decode, policy::AppHttpPolicy, streams::HttpStreams, HttpError, HttpLimits};
use crate::{
    durable_state::DurableKernelStateStore, runtime::app_operation_budget::AppOperationBudget,
};
use chariox_app_package::VerifiedPackage;
use chariox_app_runtime::worker_peer::ResponsePublication;
use chariox_app_runtime::{
    app_catalog::AppCatalog, wire::RemoteError, worker_peer::BrokerRequest,
    worker_process::PrivateData,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::{runtime::Handle, sync::Semaphore};

pub(crate) struct HttpContext {
    pub(crate) limits: Arc<HttpLimits>,
    pub(crate) runtime: Handle,
}
#[derive(Clone)]
pub(crate) struct AppHttpBroker {
    publications: Arc<Mutex<Publications>>,
    store: DurableKernelStateStore,
    policy: Arc<AppHttpPolicy>,
    streams: HttpStreams,
    admission: Arc<Semaphore>,
}
struct Publications {
    closed: bool,
    pending: BTreeMap<String, Box<dyn ResponsePublication>>,
}
impl AppHttpBroker {
    pub(crate) fn new(
        store: DurableKernelStateStore,
        owner: String,
        catalog: Arc<AppCatalog>,
        package: &VerifiedPackage<'_>,
        data: PrivateData,
        admission: Arc<Semaphore>,
        context: HttpContext,
    ) -> super::Result<Self> {
        let policy = Arc::new(AppHttpPolicy::compile(package, catalog.clone())?);
        let streams = HttpStreams::new(owner, catalog, data, context.limits, context.runtime)?;
        Ok(Self {
            publications: Arc::new(Mutex::new(Publications {
                closed: false,
                pending: BTreeMap::new(),
            })),
            store,
            policy,
            streams,
            admission,
        })
    }
    #[cfg(test)]
    pub(crate) fn fixture_network(&self, fixture: Arc<super::fixture::NetworkFixture>) {
        self.streams.fixture_network(fixture);
    }
    pub(crate) fn take_response_guard(&self, id: &str) -> Option<Box<dyn ResponsePublication>> {
        self.publications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending
            .remove(id)
    }
    pub(crate) fn begin_draining(&self) {
        let pending = {
            let mut publications = self
                .publications
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            publications.closed = true;
            std::mem::take(&mut publications.pending)
        };
        drop(pending);
        self.streams.begin_draining();
    }
    pub(crate) async fn drain(&self) {
        self.streams.join().await;
    }
    pub(crate) async fn dispatch(&self, request: BrokerRequest) -> Result<Value, RemoteError> {
        if !matches!(
            request.method.as_str(),
            "http.open" | "http.write" | "http.headers" | "http.read" | "http.cancel"
        ) {
            return Err(error("METHOD_UNAVAILABLE", "HTTP method is unavailable"));
        }
        let request_id = request.id.clone();
        let budget = AppOperationBudget::from_broker(&request);
        budget.check().map_err(|_| remote(HttpError::Cancelled))?;
        let command = decode::command(&request.method, request.params).map_err(remote)?;
        if let decode::Command::Cancel(id) = command {
            // Cleanup needs no new authority/admission; a stale or saturated App
            // must still be able to stop its own epoch's stream. Never crosses
            // the per-worker map and never exposes another worker's handles.
            return match self.streams.cancel(&id) {
                Ok(()) | Err(HttpError::Invalid | HttpError::Cancelled) => Ok(Value::Null),
                Err(failure) => Err(remote(failure)),
            };
        }
        // This waits only for an already completed operation's frame callback;
        // actual concurrent I/O returns Busy. Peer handler admission bounds
        // these metadata waiters, while the prior guard still owns shared8.
        self.streams
            .await_publication(&command, request.deadline, request.cancellation.clone())
            .await
            .map_err(remote)?;
        let permit = self
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| remote(HttpError::Busy))?;
        let broker = self.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let (job, receiver) = broker.streams.job(
                &broker.policy,
                command,
                budget,
                request.deadline,
                request.cancellation,
                permit,
            )?;
            broker.store.enqueue_app_http(job);
            Ok::<_, HttpError>(receiver)
        })
        .await
        .map_err(|_| {
            error(
                "APP_HTTP_OUTCOME_UNCERTAIN",
                "HTTP operation completion is unknown; do not replay body data",
            )
        })?
        .map_err(remote)?;
        let reply = prepared
            .await
            .map_err(|_| {
                error(
                    "APP_HTTP_OUTCOME_UNCERTAIN",
                    "HTTP operation completion is unknown; do not replay body data",
                )
            })?
            .map_err(remote)?;
        let (value, guard) = reply.into_publication();
        if let Some(guard) = guard {
            let mut publications = self
                .publications
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if publications.closed
                || publications.pending.len() >= 16
                || publications.pending.contains_key(&request_id)
            {
                drop(publications);
                drop(guard);
                return Err(remote(HttpError::Cancelled));
            }
            publications.pending.insert(request_id, guard);
        }
        Ok(value)
    }
}
fn remote(failure: HttpError) -> RemoteError {
    let (code, message) = match failure {
        HttpError::Invalid => ("INVALID_ARGUMENT", "Invalid HTTP operation"),
        HttpError::Destination => (
            "APP_HTTP_DESTINATION_DENIED",
            "HTTP destination is not permitted",
        ),
        HttpError::ConnectionAuthority => (
            "APP_HTTP_CONNECTION_UNSUPPORTED",
            "Opaque connection authority is not available for this HTTP operation",
        ),
        HttpError::ProtectedEffect => (
            "APP_HTTP_EFFECT_UNSUPPORTED",
            "Protected HTTP effects require kernel effect authorization",
        ),
        HttpError::Provenance => ("APP_STALE", "App installation authority changed"),
        HttpError::Limit => ("APP_HTTP_LIMIT", "HTTP resource limit reached"),
        HttpError::Cancelled => ("APP_OPERATION_STOPPED", "HTTP operation was cancelled"),
        HttpError::Deadline => ("APP_DEADLINE", "HTTP operation deadline expired"),
        HttpError::Busy => (
            "APP_BUSY",
            "HTTP operation is already in progress or capacity is occupied",
        ),
        HttpError::Network | HttpError::Tls => {
            ("APP_HTTP_NETWORK", "HTTP transport did not complete")
        }
    };
    error(code, message)
}
fn error(code: &str, message: &str) -> RemoteError {
    RemoteError {
        code: code.into(),
        message: message.into(),
        retryable: Some(false),
    }
}

#[cfg(test)]
mod tests;
