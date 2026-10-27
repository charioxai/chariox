use super::*;
use crate::{
    durable_state::app_state::{fixture_http_catalog, fixture_http_package},
    runtime::{app_backend_broker, app_worker::AppWorkerOwner},
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::worker_peer::{ControlEvent, PeerLimits};
use chariox_app_runtime::worker_process::test_fixture::{
    Fixture as NativeFixture, Mode, Observation,
};
use std::path::PathBuf;
use tokio::{
    runtime::{Builder, Runtime},
    sync::mpsc::Receiver,
};
pub(super) const WAIT: std::time::Duration = std::time::Duration::from_secs(5);
pub(super) struct Fixture {
    pub owner: Option<AppWorkerOwner>,
    pub runtime: Runtime,
    pub events: Receiver<ControlEvent>,
    pub observed: Observation,
    pub network: Arc<NetworkFixture>,
    pub admission: Arc<Semaphore>,
    pub limits: Arc<HttpLimits>,
    _store: DurableKernelStateStore,
    _scratch: Scratch,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    pub(super) fn new(paused: bool) -> Self {
        let scratch = Scratch(std::env::temp_dir().join(format!(
            "chariox-http-channel-{:016x}",
            rand::random::<u64>()
        )));
        std::fs::create_dir(&scratch.0).unwrap();
        let store = DurableKernelStateStore::open_owned(scratch.0.join("kernel.sqlite")).unwrap();
        let catalog = fixture_http_catalog(&store);
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let (bytes, publisher) = fixture_http_package();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        let native = NativeFixture::compile().unwrap();
        let (process, observed) = native
            .spawn_blocking(if paused { Mode::HttpPaused } else { Mode::Http }, &package)
            .unwrap();
        let network = NetworkFixture::new(if paused {
            FixedNetwork::Paused
        } else {
            FixedNetwork::Echo
        });
        let admission = Arc::new(Semaphore::new(8));
        let limits = Arc::new(HttpLimits::default());
        let delegate = app_backend_broker::fixture_http(
            store.clone(),
            "alice".into(),
            catalog.clone(),
            admission.clone(),
            &process,
            &package,
            HttpContext {
                limits: limits.clone(),
                runtime: runtime.handle().clone(),
            },
            network.clone(),
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
        let mut registered = starting.await_registered_blocking(WAIT).unwrap();
        let proof = store
            .confirm_app_activation(
                "alice",
                catalog,
                registered.take_activation_budget().unwrap(),
            )
            .unwrap();
        let (owner, _) = registered.activate_blocking(proof).unwrap();
        Self {
            owner: Some(owner),
            runtime,
            events,
            observed,
            network,
            admission,
            limits,
            _store: store,
            _scratch: scratch,
        }
    }
    pub(super) fn event(&mut self, name: &str) {
        self.runtime.block_on(async {
            tokio::time::timeout(WAIT, async {
                loop {
                    if self
                        .events
                        .recv()
                        .await
                        .expect("native HTTP channel closed")
                        .name
                        == name
                    {
                        break;
                    }
                }
            })
            .await
            .unwrap();
        });
    }
    pub(super) fn shutdown(&mut self) {
        self.network.released.notify_one();
        if let Some(owner) = self.owner.take() {
            owner.shutdown_blocking();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown();
    }
}
