use super::*;
use crate::durable_state::{
    app_state::{fixture_event_catalog, fixture_event_package},
    DurableKernelStateStore,
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    worker_peer::{Broker, BrokerFuture, BrokerRequest, ControlEvent, PeerLimits},
    worker_process::test_fixture::{Fixture as NativeFixture, Mode},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::{
    runtime::{Builder, Runtime},
    sync::{mpsc::Receiver, oneshot, Semaphore},
};

const WAIT: Duration = Duration::from_secs(3);
mod identity;
mod tools;
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = PathBuf::from("/tmp").join(format!(
            "chariox-worker-owner-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(1)
        .enable_io()
        .enable_time()
        .build()
        .unwrap()
}
struct Handler<F>(F);
impl<F: Fn(BrokerRequest) -> BrokerFuture + Send + Sync + 'static> Broker for Handler<F> {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        (self.0)(request)
    }
}
fn broker(f: impl Fn(BrokerRequest) -> BrokerFuture + Send + Sync + 'static) -> Arc<dyn Broker> {
    Arc::new(Handler(f))
}
fn start(
    fixture: &NativeFixture,
    mode: Mode,
    runtime: &Runtime,
    catalog: Arc<EventCatalog>,
    delegate: Arc<dyn Broker>,
) -> (
    startup::StartingAppWorker,
    Receiver<ControlEvent>,
    chariox_app_runtime::worker_process::test_fixture::Observation,
) {
    let (bytes, publisher) = if matches!(mode, Mode::ToolEcho) {
        crate::durable_state::app_state::fixture_tool_package()
    } else {
        fixture_event_package()
    };
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let (process, observed) = fixture.spawn_blocking(mode, &verified).unwrap();
    let (starting, events) = AppWorkerOwner::start_blocking(
        process,
        &verified,
        catalog,
        delegate,
        PeerLimits::default(),
        runtime.handle().clone(),
    )
    .unwrap();
    (starting, events, observed)
}
fn activate(
    mut registered: RegisteredAppWorker,
    store: &DurableKernelStateStore,
) -> (AppWorkerOwner, ActivatedApp) {
    let catalog = registered.catalog().clone();
    let budget = registered.take_activation_budget().unwrap();
    let proof = store
        .confirm_app_activation("alice", catalog, budget)
        .unwrap();
    registered.activate_blocking(proof).unwrap()
}
fn event(runtime: &Runtime, events: &mut Receiver<ControlEvent>, expected: &str) {
    runtime
        .block_on(async {
            tokio::time::timeout(WAIT, async {
                loop {
                    if events.recv().await.unwrap().name == expected {
                        break;
                    }
                }
            })
            .await
        })
        .unwrap();
}

#[test]
fn native_sdk_ready_waits_for_actual_activation_and_handle_cannot_outlive_owner() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let control = crate::runtime::app_control::AppControlService::new(store.clone());
    assert!(control.active_app_lease("alice", "installed").is_none());
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = calls.clone();
    let delegate = broker(move |_| {
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Value::Null) })
    });
    let (starting, mut events, observed) =
        start(&fixture, Mode::Ready, &runtime, catalog, delegate);
    let registered = starting
        .await_registered_blocking(WAIT.min(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "provisional state.get must not reach the delegate"
    );
    assert!(!observed.ready_was_acknowledged());
    let (owner, handle) = activate(registered, &store);
    event(&runtime, &mut events, "fixture.ready_ack");
    assert!(observed.ready_was_acknowledged());
    assert_eq!(
        control
            .publish_app_worker("bob", handle.clone())
            .unwrap_err(),
        AppWorkerError::Unavailable
    );
    control.publish_app_worker("alice", handle.clone()).unwrap();
    assert_eq!(
        control
            .publish_app_worker("alice", handle.clone())
            .unwrap_err(),
        AppWorkerError::Busy
    );
    assert!(control.active_app_lease("bob", "installed").is_none());
    assert!(control.active_app_lease("alice", "other").is_none());
    assert_eq!(control.active_app_leases(None, 16).len(), 1);
    assert!(control
        .active_app_leases(Some(("alice", "installed")), 16)
        .is_empty());
    assert!(matches!(
        handle.lease("bob"),
        Err(AppWorkerError::Unavailable)
    ));
    let lease = handle.lease("alice").unwrap();
    assert_eq!(lease.catalog().generation(), 1);
    owner.stop();
    assert!(control.active_app_lease("alice", "installed").is_none());
    assert!(control.active_app_leases(None, 16).is_empty());
    runtime
        .block_on(async { tokio::time::timeout(WAIT, lease.cancelled()).await })
        .unwrap();
    assert!(matches!(
        handle.lease("alice"),
        Err(AppWorkerError::Unavailable)
    ));
    owner.shutdown_blocking();
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
}

#[test]
fn wrong_or_missing_native_report_never_creates_a_callable_handle() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    for mode in [Mode::WrongHandlers, Mode::NoReport] {
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let delegate = broker(move |_| {
            called.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(Value::Null) })
        });
        let (starting, _events, observed) =
            start(&fixture, mode, &runtime, catalog.clone(), delegate);
        let (timeout, expected) = match mode {
            Mode::WrongHandlers => (Duration::from_secs(2), AppWorkerError::Invalid),
            _ => (Duration::from_millis(150), AppWorkerError::Deadline),
        };
        match starting.await_registered_blocking(timeout) {
            Err(error) => assert_eq!(error, expected),
            Ok(_) => panic!("invalid or absent report became registered"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!observed.ready_was_acknowledged());
        assert!(observed.was_reaped());
        assert!(observed.lease_was_dropped());
    }
}

struct ShutdownGuard {
    release: Arc<Semaphore>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.release.add_permits(1);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn actual_process_and_prepared_leases_are_retained_until_broker_drain_finishes() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let release = Arc::new(Semaphore::new(0));
    let (started, received) = oneshot::channel();
    let started = Arc::new(Mutex::new(Some(started)));
    let (cancelled, cancellation_seen) = oneshot::channel();
    let cancelled = Arc::new(Mutex::new(Some(cancelled)));
    let handler_release = release.clone();
    let delegate = broker(move |mut request| {
        let started = started.lock().unwrap().take().unwrap();
        let cancelled = cancelled.lock().unwrap().take().unwrap();
        let release = handler_release.clone();
        Box::pin(async move {
            let _ = started.send(());
            request.cancellation.cancelled().await;
            let _ = cancelled.send(());
            let _permit = release.acquire().await.unwrap();
            Ok(Value::Null)
        })
    });
    let (starting, mut events, observed) =
        start(&fixture, Mode::BrokerCall, &runtime, catalog, delegate);
    let registered = starting
        .await_registered_blocking(Duration::from_secs(2))
        .unwrap();
    let (owner, handle) = activate(registered, &store);
    // This guard releases an intentionally retained callback even on assertion
    // failure; native process ownership is never detached during test cleanup.
    let (done, completion) = mpsc::channel();
    let shutdown = ShutdownGuard {
        release: release.clone(),
        thread: None,
    };
    event(&runtime, &mut events, "fixture.ready_ack");
    runtime
        .block_on(async { tokio::time::timeout(WAIT, received).await })
        .unwrap()
        .unwrap();
    let mut shutdown = shutdown;
    shutdown.thread = Some(thread::spawn(move || {
        owner.shutdown_blocking();
        let _ = done.send(());
    }));
    runtime
        .block_on(async { tokio::time::timeout(WAIT, cancellation_seen).await })
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + WAIT;
    while !observed.was_reaped() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let process_reaped = observed.was_reaped();
    let leases_retained = !observed.lease_was_dropped();
    let shutdown_pending = matches!(completion.try_recv(), Err(mpsc::TryRecvError::Empty));
    assert!(matches!(
        handle.lease("alice"),
        Err(AppWorkerError::Unavailable)
    ));
    release.add_permits(1);
    completion.recv_timeout(WAIT).unwrap();
    drop(shutdown);
    assert!(process_reaped);
    assert!(
        leases_retained,
        "native PreparedWorker pins must survive callback drain, even after process reap"
    );
    assert!(
        shutdown_pending,
        "shutdown must retain ownership through actual callback completion"
    );
    assert!(observed.lease_was_dropped());
}
