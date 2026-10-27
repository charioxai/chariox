use super::*;
use chariox_app_runtime::worker_peer::BrokerDrainFuture;
use std::sync::atomic::AtomicBool;

struct Probe {
    begins: AtomicUsize,
    outside_lock: AtomicBool,
    admission: Mutex<Option<Weak<Admission>>>,
    release: Arc<Semaphore>,
    draining: Mutex<Option<oneshot::Sender<()>>>,
}
impl Probe {
    fn new(release: Arc<Semaphore>) -> Arc<Self> {
        Arc::new(Self {
            begins: AtomicUsize::new(0),
            outside_lock: AtomicBool::new(true),
            admission: Mutex::new(None),
            release,
            draining: Mutex::new(None),
        })
    }
}
impl Broker for Probe {
    fn handle(&self, _: BrokerRequest) -> BrokerFuture {
        Box::pin(async { Ok(Value::Null) })
    }
    fn begin_draining(&self) {
        self.begins.fetch_add(1, Ordering::SeqCst);
        if let Some(admission) = self
            .admission
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
        {
            self.outside_lock
                .store(admission.phase.try_lock().is_ok(), Ordering::SeqCst);
        }
    }
    fn drain(&self) -> BrokerDrainFuture {
        let release = self.release.clone();
        let started = self.draining.lock().unwrap().take();
        Box::pin(async move {
            if let Some(started) = started {
                let _ = started.send(());
            }
            let _permit = release.acquire().await.unwrap();
        })
    }
}

#[test]
fn drain_hook_runs_once_outside_phase_lock_and_stale_leases_do_not_pin_broker() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let probe = Probe::new(Arc::new(Semaphore::new(1)));
    let weak_probe = Arc::downgrade(&probe);
    let (starting, mut events, observed) =
        start(&fixture, Mode::Ready, &runtime, catalog, probe.clone());
    let registered = starting
        .await_registered_blocking(Duration::from_secs(2))
        .unwrap();
    let (owner, handle) = activate(registered, &store);
    event(&runtime, &mut events, "worker.fixture.ready_ack");
    *probe.admission.lock().unwrap() = Some(Arc::downgrade(&owner.admission));
    let retained_lease = handle.lease("alice").unwrap();
    let drain = owner.drain_handle();
    drain.begin();
    drain.begin();
    assert!(retained_lease.is_stopped());
    assert_eq!(probe.begins.load(Ordering::SeqCst), 1);
    assert!(probe.outside_lock.load(Ordering::SeqCst));
    owner.stop();
    owner.shutdown_blocking();
    assert_eq!(probe.begins.load(Ordering::SeqCst), 1);
    drop(probe);
    assert!(
        weak_probe.upgrade().is_none(),
        "a stale callable lease must not retain the broker's runtime/data pins"
    );
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
    drop(retained_lease);
}

#[test]
fn actual_process_owner_waits_for_background_broker_drain_after_request_handlers_finish() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let release = Arc::new(Semaphore::new(0));
    let probe = Probe::new(release.clone());
    let (started, draining) = oneshot::channel();
    *probe.draining.lock().unwrap() = Some(started);
    let (starting, mut events, observed) =
        start(&fixture, Mode::Ready, &runtime, catalog, probe.clone());
    let registered = starting
        .await_registered_blocking(Duration::from_secs(2))
        .unwrap();
    let (owner, _) = activate(registered, &store);
    event(&runtime, &mut events, "worker.fixture.ready_ack");
    let (done, completion) = mpsc::channel();
    let mut shutdown = ShutdownGuard {
        release: release.clone(),
        thread: None,
    };
    shutdown.thread = Some(thread::spawn(move || {
        owner.shutdown_blocking();
        let _ = done.send(());
    }));
    runtime
        .block_on(async { tokio::time::timeout(WAIT, draining).await })
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + WAIT;
    while !observed.was_reaped() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(observed.was_reaped());
    assert!(!observed.lease_was_dropped());
    assert!(matches!(
        completion.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    release.add_permits(1);
    completion.recv_timeout(WAIT).unwrap();
    drop(shutdown);
    assert_eq!(probe.begins.load(Ordering::SeqCst), 1);
    assert!(observed.lease_was_dropped());
}

#[test]
fn rejected_startup_also_drains_host_resources_before_returning_failure() {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let runtime = runtime();
    let fixture = NativeFixture::compile().unwrap();
    let probe = Probe::new(Arc::new(Semaphore::new(1)));
    let (starting, _events, observed) = start(
        &fixture,
        Mode::WrongHandlers,
        &runtime,
        catalog,
        probe.clone(),
    );
    assert!(matches!(
        starting.await_registered_blocking(Duration::from_secs(2)),
        Err(AppWorkerError::Invalid)
    ));
    assert_eq!(probe.begins.load(Ordering::SeqCst), 1);
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
}
