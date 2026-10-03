use super::super::*;
use super::integration::{change, permits, receive, send, start, start_fenced, Fixture, WAIT};
use crate::durable_state::app_state::fixture_event_catalog;
use chariox_app_runtime::wire::{Message, WIRE_VERSION};
use std::time::Duration;
use tokio::time::timeout;

/// V1-INT-03: an eight-read burst from one worker must not refuse its neighbour.
/// A real SQLite writer lock pins the burst deterministically; both peers use
/// the same AppControl admission pool and distinct verified installations.
#[tokio::test]
async fn slow_consumer_burst_does_not_refuse_neighbour_storage() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let slow_catalog = fixture_event_catalog(&store);
    let neighbour_catalog = crate::durable_state::app_state::fixture_neighbour_catalog(&store);
    let admission = Arc::new(Semaphore::new(8));
    let (slow_peer, mut slow, slow_task) = start(&store, slow_catalog, "alice", admission.clone());
    let (neighbour_peer, mut neighbour, neighbour_task) =
        start(&store, neighbour_catalog, "alice", admission.clone());
    let blocker = rusqlite::Connection::open(store.path()).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    for n in 0..8 {
        send(
            &mut slow,
            &format!("burst-{n}"),
            "state.get",
            json!({"key":"status"}),
        )
        .await;
    }
    permits(&admission, 0).await;
    send(
        &mut neighbour,
        "neighbour",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    blocker.execute_batch("ROLLBACK").unwrap();
    let neighbour_result = receive(&mut neighbour).await;
    let mut slow_results = Vec::new();
    for _ in 0..8 {
        slow_results.push(receive(&mut slow).await);
    }
    permits(&admission, 8).await;
    slow_peer.close();
    neighbour_peer.close();
    timeout(WAIT, slow_task.join()).await.unwrap().unwrap();
    timeout(WAIT, neighbour_task.join()).await.unwrap().unwrap();
    assert_eq!(neighbour_result, Ok(Value::Null), "zero neighbour refusals");
    assert!(slow_results.iter().all(|r| r == &Ok(Value::Null)));
}

#[tokio::test]
async fn storage_admission_wait_expires_or_cancels_without_starting_a_write() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    let admission = Arc::new(Semaphore::new(8));
    let (peer, mut worker, task) = start(&store, catalog, "alice", admission.clone());
    let held = admission.clone().acquire_many_owned(8).await.unwrap();
    send(
        &mut worker,
        "cancel-write",
        "state.transaction",
        change(json!("cancelled")),
    )
    .await;
    worker
        .send(
            &Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "cancel-write".into(),
            },
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(receive(&mut worker).await.unwrap_err(), "CANCELLED");
    worker
        .send(
            &Message::Request {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "expire-write".into(),
                method: "state.transaction".into(),
                params: change(json!("expired")),
                deadline_ms: crate::session::unix_epoch_ms() + 50,
                context: None,
            },
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(receive(&mut worker).await.unwrap_err(), "DEADLINE_EXCEEDED");
    assert_eq!(admission.available_permits(), 0);
    drop(held);
    send(
        &mut worker,
        "not-written",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap(), Value::Null);
    permits(&admission, 8).await;
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}

/// A real 2 MiB reply burst fills the slow worker's duplex transport. Storage
/// admission must retire with the writer, rather than waiting for consumption.
#[tokio::test]
async fn slow_consumer_reply_drill_keeps_neighbour_responsive() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    let neighbour_catalog = crate::durable_state::app_state::fixture_neighbour_catalog(&store);
    let admission = Arc::new(Semaphore::new(8));
    let (slow_peer, mut slow, slow_task) = start(&store, catalog, "alice", admission.clone());
    let (neighbour_peer, mut neighbour, neighbour_task) =
        start(&store, neighbour_catalog, "alice", admission.clone());
    let value = json!("x".repeat(250 * 1024));
    send(
        &mut slow,
        "seed",
        "state.transaction",
        change(value.clone()),
    )
    .await;
    receive(&mut slow).await.unwrap();
    let blocker = rusqlite::Connection::open(store.path()).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    for n in 0..8 {
        send(
            &mut slow,
            &format!("large-{n}"),
            "state.get",
            json!({"key":"status"}),
        )
        .await;
    }
    permits(&admission, 0).await;
    send(
        &mut neighbour,
        "overlap",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    blocker.execute_batch("ROLLBACK").unwrap();
    let mut neighbour_results = vec![receive(&mut neighbour).await];
    // Consume no large replies while probing the neighbour. A duplex buffer of
    // 64 KiB cannot contain even one 250 KiB response, so this is backpressure.
    let mut max_latency = Duration::ZERO;
    for n in 0..20 {
        let started = tokio::time::Instant::now();
        send(
            &mut neighbour,
            &format!("probe-{n}"),
            "state.get",
            json!({"key":"status"}),
        )
        .await;
        neighbour_results.push(receive(&mut neighbour).await);
        max_latency = max_latency.max(started.elapsed());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    permits(&admission, 8).await;
    let mut bytes = 0;
    for _ in 0..8 {
        let reply = receive(&mut slow).await.unwrap();
        assert_eq!(reply["value"], value);
        bytes += reply["value"].as_str().unwrap().len();
    }
    slow_peer.close();
    neighbour_peer.close();
    timeout(WAIT, slow_task.join()).await.unwrap().unwrap();
    timeout(WAIT, neighbour_task.join()).await.unwrap().unwrap();
    assert!(
        neighbour_results.iter().all(|r| r == &Ok(Value::Null)),
        "zero neighbour refusals: {neighbour_results:?}"
    );
    assert!(max_latency < WAIT, "neighbour latency exceeded budget");
    println!(
        "slow-consumer drill: bytes={bytes}, neighbour_calls={}, refusals=0, max_latency_ms={}",
        neighbour_results.len(),
        max_latency.as_millis()
    );
}

#[tokio::test]
async fn storage_admission_wait_does_not_hold_the_snapshot_fence() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    let admission = Arc::new(Semaphore::new(8));
    let fence = Arc::new(tokio::sync::RwLock::new(()));
    let dispatched = Arc::new(AtomicBool::new(false));
    let observed = dispatched.clone();
    let (peer, mut worker, task) = start_fenced(
        &store,
        catalog,
        "alice",
        admission.clone(),
        Some(Arc::new(move || {
            observed.store(true, Ordering::Release);
        })),
        fence.clone(),
    );
    // Snapshots reserve admission before their exclusive fence. A queued state
    // request must follow that order, or it can block their completion.
    let held = admission.clone().acquire_many_owned(8).await.unwrap();
    send(&mut worker, "waiting", "state.get", json!({"key":"status"})).await;
    timeout(WAIT, async {
        while !dispatched.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let snapshot = fence.try_write();
    let snapshot_can_finish = snapshot.is_ok();
    drop(snapshot);
    worker
        .send(
            &Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "waiting".into(),
            },
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(receive(&mut worker).await.unwrap_err(), "CANCELLED");
    drop(held);
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
    assert!(
        snapshot_can_finish,
        "admission waiter blocked snapshot's exclusive fence"
    );
}

#[tokio::test]
async fn cancellation_while_waiting_for_snapshot_releases_storage_admission() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let catalog = fixture_event_catalog(&store);
    let admission = Arc::new(Semaphore::new(8));
    let fence = Arc::new(tokio::sync::RwLock::new(()));
    let snapshot = fence.write().await;
    let (peer, mut worker, task) = start_fenced(
        &store,
        catalog,
        "alice",
        admission.clone(),
        None,
        fence.clone(),
    );
    send(
        &mut worker,
        "blocked",
        "state.transaction",
        change(json!("cancelled")),
    )
    .await;
    permits(&admission, 7).await;
    worker
        .send(
            &Message::Cancel {
                version: WIRE_VERSION,
                generation: "1".into(),
                id: "blocked".into(),
            },
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(receive(&mut worker).await.unwrap_err(), "CANCELLED");
    permits(&admission, 8).await;
    drop(snapshot);
    send(
        &mut worker,
        "not-written",
        "state.get",
        json!({"key":"status"}),
    )
    .await;
    assert_eq!(receive(&mut worker).await.unwrap(), Value::Null);
    peer.close();
    timeout(WAIT, task.join()).await.unwrap().unwrap();
}
