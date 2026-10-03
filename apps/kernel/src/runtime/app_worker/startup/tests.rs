//! Exercise the readiness handler's early rejection and the real owner's reap.
use super::*;
use crate::durable_state::app_state::{fixture_event_catalog, fixture_event_package};
use crate::runtime::app_worker::tests::{broker, runtime, start, Scratch};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    worker_peer::BrokerCancellation,
    worker_process::test_fixture::{Fixture, Mode},
};

fn rejected_readiness(cancelled: bool, expired: bool, expected: AppWorkerError, code: &str) {
    let scratch = Scratch::new();
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    let runtime = runtime();
    let fixture = Fixture::compile().unwrap();
    let delegate = broker(|_| panic!("rejected readiness must not reach the delegate"));
    let (mut starting, _events, observed) = start(
        &fixture,
        Mode::NoReport,
        &runtime,
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
    let readiness = StartupBroker {
        admission: starting.owner.admission.clone(),
        delegate,
        contract: Mutex::new(Some(contract)),
        report: Mutex::new(Some(report)),
    };
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
    if cancelled {
        assert!(error.message.contains("cancelled"));
    }
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
    rejected_readiness(true, false, AppWorkerError::Cancelled, "CANCELLED");
}

#[test]
fn readiness_expired_without_cancellation_reports_deadline_and_reaps() {
    rejected_readiness(false, true, AppWorkerError::Deadline, "APP_READY_EXPIRED");
}

#[test]
fn cancelled_readiness_keeps_cancellation_when_deadline_also_expired() {
    rejected_readiness(true, true, AppWorkerError::Cancelled, "CANCELLED");
}
