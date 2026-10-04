//! Exercise the readiness handler's early rejection and the real owner's reap.
use super::*;
use crate::durable_state::app_state::{fixture_event_catalog, fixture_event_package};
use crate::runtime::app_worker::tests::{broker, runtime, start, Scratch};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    worker_peer::BrokerCancellation,
    worker_process::test_fixture::{Fixture, Mode},
};

fn readiness_owner(
    fixture: &Fixture,
    runtime: &tokio::runtime::Runtime,
) -> (
    Scratch,
    StartingAppWorker,
    Arc<StartupBroker>,
    chariox_app_runtime::worker_process::test_fixture::Observation,
    mpsc::Receiver<ControlEvent>,
) {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let delegate = broker(|_| panic!("rejected readiness must not reach the delegate"));
    let (mut starting, events, observed) = start(
        fixture,
        Mode::NoReport,
        runtime,
        catalog.clone(),
        delegate.clone(),
    );
    let (bytes, publisher) = fixture_event_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let contract =
        ReadinessContract::from_verified(&package, catalog.app_catalog().clone()).unwrap();
    let (report, registration) = oneshot::channel();
    starting.registration = registration;
    let readiness = Arc::new(StartupBroker {
        admission: starting.owner.admission.clone(),
        delegate,
        contract: Mutex::new(Some(contract)),
        report: Mutex::new(Some(report)),
    });
    (scratch, starting, readiness, observed, events)
}

fn rejected_readiness(cancelled: bool, expired: bool, expected: AppWorkerError, code: &str) {
    let runtime = runtime();
    let fixture = Fixture::compile().unwrap();
    let (_scratch, starting, readiness, observed, _native_events) =
        readiness_owner(&fixture, &runtime);

    let (signal, cancellation) = BrokerCancellation::fixture();
    let deadline = if expired {
        tokio::time::Instant::now()
    } else {
        tokio::time::Instant::now() + Duration::from_secs(30)
    };
    let request = BrokerRequest {
        id: "ready".into(),
        method: "worker.ready".into(),
        params: serde_json::json!({"tools": [], "events": [], "lifecycle": []}),
        deadline,
        cancellation,
        callers: vec![],
        open_calls: vec![],
    };
    // Cancel after dispatch constructed the future but before its first poll.
    let future = readiness.handle(request);
    signal.send_replace(cancelled);
    if !expired {
        assert!(tokio::time::Instant::now() < deadline);
    }
    let error = runtime.block_on(future).unwrap_err();
    assert_eq!(error.code, code);
    assert!(*readiness.admission.phase.lock().unwrap() == Phase::Stopped);
    assert!(
        matches!(starting.await_registered_blocking(Duration::from_secs(1)),
        Err(error) if error == expected)
    );
    assert!(!observed.ready_was_acknowledged());
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
}

#[test]
fn readiness_cancelled_before_deadline_reports_cancellation_and_reaps() {
    rejected_readiness(true, false, AppWorkerError::Cancelled, "APP_READY_EXPIRED");
}

#[test]
fn readiness_expired_without_cancellation_reports_deadline_and_reaps() {
    rejected_readiness(false, true, AppWorkerError::Deadline, "APP_READY_EXPIRED");
}

#[test]
fn cancelled_readiness_keeps_cancellation_when_deadline_also_expired() {
    rejected_readiness(true, true, AppWorkerError::Cancelled, "APP_READY_EXPIRED");
}

#[test]
fn readiness_timer_published_during_budget_check_retains_deadline_cause() {
    let (signal, cancellation) = BrokerCancellation::fixture();
    let observed = cancellation.clone();
    let budget = AppOperationBudget::fixture(
        tokio::time::Instant::now() + Duration::from_secs(30),
        move || {
            // Force publication between a premature cause read and the watch
            // observation. The cause must be read after observing cancellation.
            observed.fixture_deadline_cancel(&signal);
            observed.is_cancelled()
        },
    );
    assert!(!cancellation.cancelled_by_deadline());
    assert_eq!(
        check_readiness_budget(&budget, &cancellation),
        Err(AppWorkerError::Deadline)
    );
}

struct DispatchProbe {
    readiness: Arc<StartupBroker>,
    dispatched: std::sync::atomic::AtomicUsize,
    not_dispatched: std::sync::atomic::AtomicUsize,
    wait_for_timer: bool,
}
impl Broker for DispatchProbe {
    fn handle(&self, request: BrokerRequest) -> BrokerFuture {
        self.dispatched
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut cancellation = request.cancellation.clone();
        let future = self.readiness.handle(request);
        if self.wait_for_timer {
            Box::pin(async move {
                cancellation.cancelled().await;
                assert!(cancellation.cancelled_by_deadline());
                future.await
            })
        } else {
            future
        }
    }
    fn request_not_dispatched(&self, request: &BrokerRequest) {
        self.not_dispatched
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.readiness.request_not_dispatched(request);
    }
}

#[test]
fn peer_ready_then_cancel_before_dispatch_reports_cancellation_and_reaps() {
    peer_stopped_readiness(false);
}

#[test]
fn peer_readiness_timer_cancellation_reports_deadline_and_reaps() {
    peer_stopped_readiness(true);
}

fn peer_stopped_readiness(expire: bool) {
    use chariox_app_runtime::wire::{Channel, Message, Sender, WIRE_VERSION};
    use std::sync::atomic::Ordering;
    use tokio::io::AsyncWriteExt;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let fixture = Fixture::compile().unwrap();
    for _ in 0..10 {
        let (_scratch, starting, readiness, observed, _native_events) =
            readiness_owner(&fixture, &runtime);
        let probe = Arc::new(DispatchProbe {
            readiness,
            dispatched: 0.into(),
            not_dispatched: 0.into(),
            wait_for_timer: expire,
        });
        let (host, mut worker) = tokio::io::duplex(64 * 1024);
        let deadline_ms = crate::session::unix_epoch_ms() + 30_000;
        let ready = Message::Request {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: "ready".into(),
            method: "worker.ready".into(),
            params: serde_json::json!({"tools": [], "events": [], "lifecycle": []}),
            deadline_ms,
            context: None,
        };
        let cancel = Message::Cancel {
            version: WIRE_VERSION,
            generation: "1".into(),
            id: "ready".into(),
        };
        let mut frames = vec![];
        for message in std::iter::once(ready).chain((!expire).then_some(cancel)) {
            let bytes = serde_json::to_vec(&message).unwrap();
            frames.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            frames.extend_from_slice(&bytes);
        }
        // Preload both frames before starting the actor on a current-thread
        // executor. The reader queues both without yielding; receive processes
        // Cancel before the spawned request task can perform its dispatch check.
        runtime.block_on(worker.write_all(&frames)).unwrap();
        let (peer, _events, task) = {
            let _entered = runtime.enter();
            WorkerPeer::start(
                Channel::new(host, "1".into(), Sender::Worker).unwrap(),
                probe.clone(),
                PeerLimits {
                    max_deadline: if expire {
                        Duration::from_millis(50)
                    } else {
                        Duration::from_secs(30)
                    },
                    ..PeerLimits::default()
                },
            )
            .unwrap()
        };
        let began = std::time::Instant::now();
        let result = runtime.block_on(async {
            // Keep the current-thread I/O/timer driver running while the native
            // owner's blocking Handle::block_on waits for the readiness report.
            let result = tokio::task::spawn_blocking(move || {
                starting.await_registered_blocking(Duration::from_secs(1))
            })
            .await
            .unwrap();
            peer.close();
            tokio::time::timeout(Duration::from_secs(1), task.join())
                .await
                .unwrap()
                .unwrap();
            result
        });
        let expected = if expire {
            AppWorkerError::Deadline
        } else {
            AppWorkerError::Cancelled
        };
        assert!(matches!(result, Err(error) if error == expected));
        assert_eq!(
            probe.not_dispatched.load(Ordering::SeqCst),
            usize::from(!expire)
        );
        assert_eq!(probe.dispatched.load(Ordering::SeqCst), usize::from(expire));
        assert!(began.elapsed() < Duration::from_secs(1));
        assert!(observed.was_reaped());
        assert!(observed.lease_was_dropped());
        assert!(!observed.ready_was_acknowledged());
    }
}
