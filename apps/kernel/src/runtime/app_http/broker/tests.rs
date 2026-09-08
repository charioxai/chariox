use super::*;
use crate::runtime::app_http::fixture::{FixedNetwork, NetworkFixture};
use std::sync::atomic::Ordering;
use tokio::time::timeout;
mod support;
use support::*;
#[test]
fn inherited_worker_channel_streams_binary_bytes_through_backend_and_writer() {
    let mut fixture = Fixture::new(false);
    fixture.event("worker.fixture.http_complete");
    assert_eq!(fixture.network.uploaded.load(Ordering::Acquire), 3);
    // The fixed shim consumed headers/body/EOF and then called cancel. Native
    // file ownership and actual broker drain remain the production mechanisms.
    fixture.shutdown();
    assert!(fixture.observed.was_reaped());
    assert!(fixture.observed.lease_was_dropped());
    assert_eq!(fixture.admission.available_permits(), 8);
    assert!(fixture.limits.acquire("alice", "installed").is_ok());
}
#[test]
fn full_native_upload_queue_retains_admission_and_worker_pins_until_actual_cleanup() {
    let mut fixture = Fixture::new(true);
    fixture.event("worker.fixture.http_blocked");
    fixture.runtime.block_on(async {
        timeout(WAIT, async {
            while fixture.admission.available_permits() != 7 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    });
    let reservations = (0..3)
        .map(|_| fixture.limits.acquire("alice", "installed").unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        fixture.limits.acquire("alice", "installed"),
        Err(HttpError::Busy)
    ));
    let owner = fixture.owner.take().unwrap();
    let joining = fixture
        .runtime
        .spawn_blocking(move || owner.shutdown_blocking());
    fixture.runtime.block_on(async {
        timeout(WAIT, fixture.network.cancelled.notified())
            .await
            .unwrap();
    });
    // begin_draining cancelled synchronously, but the fixture's actual task is
    // still inside cleanup. Neither lifetime capacity nor native pins are freed.
    assert!(!joining.is_finished());
    assert!(!fixture.observed.lease_was_dropped());
    assert!(matches!(
        fixture.limits.acquire("alice", "installed"),
        Err(HttpError::Busy)
    ));
    fixture.network.released.notify_one();
    fixture.runtime.block_on(async {
        timeout(WAIT, joining).await.unwrap().unwrap();
    });
    assert!(fixture.observed.was_reaped());
    assert!(fixture.observed.lease_was_dropped());
    assert_eq!(fixture.admission.available_permits(), 8);
    drop(reservations);
    assert!(fixture.limits.acquire("alice", "installed").is_ok());
}
