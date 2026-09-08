use super::*;
use crate::{
    durable_state::{
        app_state::{fixture_http_catalog, fixture_http_package},
        DurableKernelStateStore,
    },
    runtime::{app_backend_broker, app_worker::AppWorkerOwner},
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    app_outbox::EventCatalog,
    wire::{Channel, Message, Sender, WIRE_VERSION},
    worker_peer::{
        Broker, BrokerCancellation, BrokerFuture, BrokerRequest, PeerLimits, PeerTask, WorkerPeer,
    },
    worker_process::test_fixture::{Fixture as NativeFixture, Mode, Observation},
};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::DuplexStream,
    runtime::{Builder, Runtime},
    sync::{oneshot, Semaphore},
    time::timeout,
};

pub(super) const WAIT: Duration = Duration::from_secs(5);
pub(super) struct Fixture {
    owner: Option<AppWorkerOwner>,
    data: Option<PrivateData>,
    pub(super) observed: Observation,
    pub(super) store: DurableKernelStateStore,
    pub(super) catalog: Arc<EventCatalog>,
    pub(super) policy: super::super::super::policy::AppHttpPolicy,
    pub(super) limits: Arc<HttpLimits>,
    pub(super) runtime: Runtime,
    scratch: Scratch,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    pub(super) fn new() -> Self {
        let scratch = Scratch(
            std::env::temp_dir().join(format!("chariox-http-owner-{:016x}", rand::random::<u64>())),
        );
        std::fs::create_dir(&scratch.0).unwrap();
        let store = DurableKernelStateStore::open_owned(scratch.0.join("kernel.sqlite")).unwrap();
        let catalog = fixture_http_catalog(&store);
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let native = NativeFixture::compile().unwrap();
        let (bytes, publisher) = fixture_http_package();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        let policy = super::super::super::policy::AppHttpPolicy::compile(
            &package,
            catalog.app_catalog().clone(),
        )
        .unwrap();
        let (process, observed) = native.spawn_blocking(Mode::Ready, &package).unwrap();
        let data = process.private_data().unwrap();
        let delegate = app_backend_broker::broker(
            store.clone(),
            "alice".into(),
            catalog.clone(),
            Arc::new(Semaphore::new(8)),
            &process,
            &package,
            crate::runtime::app_http::HttpContext {
                limits: Arc::new(crate::runtime::app_http::HttpLimits::default()),
                runtime: runtime.handle().clone(),
            },
        )
        .unwrap();
        let (starting, mut events) = AppWorkerOwner::start_blocking(
            process,
            &package,
            catalog.clone(),
            delegate,
            PeerLimits::default(),
            runtime.handle().clone(),
        )
        .unwrap();
        let mut registered = starting.await_registered_blocking(WAIT).unwrap();
        let proof = store
            .confirm_app_activation(
                "alice",
                catalog.clone(),
                registered.take_activation_budget().unwrap(),
            )
            .unwrap();
        let (owner, _) = registered.activate_blocking(proof).unwrap();
        runtime.block_on(async {
            timeout(WAIT, async {
                loop {
                    if events
                        .recv()
                        .await
                        .expect("native fixture peer closed")
                        .name
                        == "worker.fixture.ready_ack"
                    {
                        break;
                    }
                }
            })
            .await
            .unwrap();
        });
        Self {
            owner: Some(owner),
            data: Some(data),
            observed,
            store,
            catalog,
            policy,
            limits: Arc::new(HttpLimits::default()),
            runtime,
            scratch,
        }
    }
    pub(super) fn group(&self) -> HttpStreams {
        self.group_on(self.runtime.handle().clone())
    }
    pub(super) fn group_on(&self, runtime: tokio::runtime::Handle) -> HttpStreams {
        HttpStreams::new(
            "alice".into(),
            self.catalog.app_catalog().clone(),
            self.data.as_ref().unwrap().clone(),
            self.limits.clone(),
            runtime,
        )
        .unwrap()
    }
    pub(super) fn target(&self) -> ApprovedTarget {
        self.policy
            .anonymous_target("https://api.example.com/fixture", "POST", &[], None, None)
            .unwrap()
    }
    pub(super) fn start<F, Fut>(
        &self,
        pending: PendingStart,
        budget: &AppOperationBudget,
        run: F,
    ) -> Result<String>
    where
        F: FnOnce(
                HttpTransport,
                ApprovedTarget,
                Exchange,
                watch::Receiver<bool>,
                LifetimeLease,
                Instant,
            ) -> Fut
            + Send
            + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        // An actual transaction against the same durable SQLite authority. This
        // private fixture invokes the production admission fence directly; the
        // public SDK never receives a connection or selects a driver.
        let mut connection =
            rusqlite::Connection::open(self.scratch.0.join("kernel.sqlite")).unwrap();
        connection.busy_timeout(WAIT).unwrap();
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let result = pending.start_with(&transaction, budget, run);
        transaction.rollback().unwrap();
        result
    }
    pub(super) fn shutdown(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.shutdown_blocking();
        }
        self.data.take();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// Cancellation tokens come from a real framed WorkerPeer request, never a
// public unchecked constructor. This peer performs no network operation.
struct Capture(Mutex<Option<oneshot::Sender<BrokerCancellation>>>);
impl Broker for Capture {
    fn handle(&self, mut request: BrokerRequest) -> BrokerFuture {
        assert!(self
            .0
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(request.cancellation.clone())
            .is_ok());
        Box::pin(async move {
            request.cancellation.cancelled().await;
            Ok(serde_json::Value::Null)
        })
    }
}
pub(super) struct Cancellation {
    pub(super) token: BrokerCancellation,
    peer: WorkerPeer,
    worker: Channel<DuplexStream>,
    task: Option<PeerTask>,
}
impl Cancellation {
    pub(super) async fn new() -> Self {
        let (host, worker) = tokio::io::duplex(64 * 1024);
        let (send, receive) = oneshot::channel();
        let (peer, _, task) = WorkerPeer::start(
            Channel::new(host, "1".into(), Sender::Worker).unwrap(),
            Arc::new(Capture(Mutex::new(Some(send)))),
            PeerLimits::default(),
        )
        .unwrap();
        let mut worker = Channel::new(worker, "1".into(), Sender::Supervisor).unwrap();
        worker
            .send(
                &Message::Request {
                    version: WIRE_VERSION,
                    generation: "1".into(),
                    id: "http-fixture".into(),
                    method: "http.fixture".into(),
                    params: serde_json::Value::Null,
                    deadline_ms: crate::session::unix_epoch_ms() + 30_000,
                    context: None,
                },
                WAIT,
            )
            .await
            .unwrap();
        let token = timeout(WAIT, receive).await.unwrap().unwrap();
        Self {
            token,
            peer,
            worker,
            task: Some(task),
        }
    }
    pub(super) async fn cancel(&mut self) {
        self.worker
            .send(
                &Message::Cancel {
                    version: WIRE_VERSION,
                    generation: "1".into(),
                    id: "http-fixture".into(),
                },
                WAIT,
            )
            .await
            .unwrap();
        timeout(WAIT, self.token.cancelled()).await.unwrap();
    }
    pub(super) async fn close(mut self) {
        self.peer.close();
        timeout(WAIT, self.task.take().unwrap().join())
            .await
            .unwrap()
            .unwrap();
    }
}
impl Drop for Cancellation {
    fn drop(&mut self) {
        self.peer.close();
    }
}

pub(super) fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
pub(super) fn head() -> ResponseHead {
    ResponseHead {
        status: 200,
        headers: vec![],
        url: "https://api.example.com/fixture".into(),
    }
}
pub(super) async fn wait_stopped(mut stopped: watch::Receiver<bool>) -> Result<()> {
    super::super::super::cancelled(&mut stopped).await;
    Err(HttpError::Cancelled)
}
