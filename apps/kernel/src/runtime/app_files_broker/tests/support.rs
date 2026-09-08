use super::*;
use crate::{
    durable_state::app_state::{fixture_event_catalog, fixture_event_package},
    runtime::{app_backend_broker, app_worker::AppWorkerOwner},
};
use chariox_app_package::{verify, VerificationPolicy};
pub(super) use chariox_app_runtime::worker_process::test_fixture::Mode;
use chariox_app_runtime::{
    wire::{Channel, Message, Outcome, Sender, WIRE_VERSION},
    worker_peer::{Broker, BrokerFuture, ControlEvent, PeerLimits, PeerTask, WorkerPeer},
    worker_process::{
        test_fixture::{Fixture as NativeFixture, Observation},
        PrivateData,
    },
};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::DuplexStream,
    runtime::{Builder, Runtime},
    sync::mpsc::Receiver,
    time::timeout,
};

pub(super) const WAIT: Duration = Duration::from_secs(5);
pub(super) struct Fixture {
    owner: Option<AppWorkerOwner>,
    data: Option<PrivateData>,
    pub(super) observed: Observation,
    events: Receiver<ControlEvent>,
    pub(super) store: DurableKernelStateStore,
    pub(super) catalog: Arc<EventCatalog>,
    pub(super) admission: Arc<Semaphore>,
    pub(super) runtime: Runtime,
    _scratch: Scratch,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    pub(super) fn new(mode: Mode) -> Self {
        let scratch = Scratch(
            std::env::temp_dir().join(format!("chariox-app-files-{:016x}", rand::random::<u64>())),
        );
        std::fs::create_dir(&scratch.0).unwrap();
        let store = DurableKernelStateStore::open_owned(scratch.0.join("kernel.sqlite")).unwrap();
        let catalog = fixture_event_catalog(&store);
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let native = NativeFixture::compile().unwrap();
        let (bytes, publisher) = fixture_event_package();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        let (process, observed) = native.spawn_blocking(mode, &package).unwrap();
        let data = process.private_data().unwrap();
        let admission = Arc::new(Semaphore::new(8));
        let delegate = app_backend_broker::broker(
            store.clone(),
            "alice".into(),
            catalog.clone(),
            admission.clone(),
            &process,
        )
        .unwrap();
        let (starting, events) = AppWorkerOwner::start_blocking(
            process,
            &package,
            catalog.clone(),
            delegate,
            PeerLimits::default(),
            runtime.handle().clone(),
        )
        .unwrap();
        let mut registered = starting
            .await_registered_blocking(Duration::from_secs(2))
            .unwrap();
        let proof = store
            .confirm_app_activation(
                "alice",
                catalog.clone(),
                registered.take_activation_budget().unwrap(),
            )
            .unwrap();
        let (owner, _) = registered.activate_blocking(proof).unwrap();
        let mut result = Self {
            owner: Some(owner),
            data: Some(data),
            observed,
            events,
            store,
            catalog,
            admission,
            runtime,
            _scratch: scratch,
        };
        result.wait_event("worker.fixture.ready_ack");
        result
    }
    pub(super) fn service(&self) -> AppFilesBroker {
        AppFilesBroker::new(
            self.store.clone(),
            "alice".into(),
            self.catalog.clone(),
            self.admission.clone(),
            self.data.as_ref().unwrap().clone(),
        )
    }
    pub(super) fn wait_event(&mut self, name: &str) {
        self.runtime.block_on(async {
            timeout(WAIT, async {
                loop {
                    if self.events.recv().await.expect("fixture peer closed").name == name {
                        break;
                    }
                }
            })
            .await
            .unwrap();
        });
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

struct Delegate(AppFilesBroker);
impl Broker for Delegate {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        let service = self.0.clone();
        Box::pin(async move { service.dispatch(request).await })
    }
}
pub(super) struct TestPeer {
    peer: WorkerPeer,
    worker: Channel<DuplexStream>,
    task: Option<PeerTask>,
}
impl TestPeer {
    pub(super) fn start(service: AppFilesBroker) -> Self {
        let (host, worker) = tokio::io::duplex(64 * 1024);
        let (peer, _events, task) = WorkerPeer::start(
            Channel::new(host, "1".into(), Sender::Worker).unwrap(),
            Arc::new(Delegate(service)),
            PeerLimits::default(),
        )
        .unwrap();
        Self {
            peer,
            worker: Channel::new(worker, "1".into(), Sender::Supervisor).unwrap(),
            task: Some(task),
        }
    }
    pub(super) async fn send(&mut self, id: &str, method: &str, params: Value) {
        let message = Message::Request {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: id.into(),
            method: method.into(),
            params,
            deadline_ms: crate::session::unix_epoch_ms() + 15_000,
            context: None,
        };
        timeout(WAIT, self.worker.send(&message, WAIT))
            .await
            .unwrap()
            .unwrap();
    }
    pub(super) async fn replace(&mut self, id: &str, bytes: &[u8]) {
        self.send(
            id,
            "files.atomic_replace",
            serde_json::json!({"path":"fixture-file","contentsBase64":STANDARD.encode(bytes)}),
        )
        .await;
    }
    pub(super) async fn cancel(&mut self, id: &str) {
        timeout(
            WAIT,
            self.worker.send(
                &Message::Cancel {
                    version: WIRE_VERSION,
                    generation: "1".into(),
                    id: id.into(),
                },
                WAIT,
            ),
        )
        .await
        .unwrap()
        .unwrap();
    }
    pub(super) async fn response(&mut self) -> (String, Result<Value, RemoteError>) {
        match timeout(WAIT, self.worker.receive(WAIT))
            .await
            .unwrap()
            .unwrap()
        {
            Message::Response {
                id,
                outcome: Outcome::Success(result),
                ..
            } => (id, Ok(result.result)),
            Message::Response {
                id,
                outcome: Outcome::Failure(failure),
                ..
            } => (id, Err(failure.error)),
            _ => panic!("broker response required"),
        }
    }
    pub(super) async fn close(mut self) {
        self.peer.close();
        timeout(WAIT, self.task.take().unwrap().join())
            .await
            .unwrap()
            .unwrap();
    }
}
impl Drop for TestPeer {
    fn drop(&mut self) {
        self.peer.close();
    }
}
