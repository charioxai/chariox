use super::*;
use bytes::Bytes;
use std::{
    future::Future,
    sync::atomic::{AtomicUsize, Ordering},
    task::Poll,
};
use tokio::{sync::oneshot, time::timeout};
mod support;
use support::*;

#[test]
fn drain_before_first_task_poll_never_starts_an_exchange() {
    let fixture = Fixture::new();
    let idle = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let group = fixture.group_on(idle.handle().clone());
    let invoked = Arc::new(AtomicUsize::new(0));
    let observed = invoked.clone();
    let id = fixture
        .start(
            group.prepare(fixture.target(), false).unwrap(),
            &budget(),
            move |_, _, _, _, _, _| async move {
                observed.fetch_add(1, Ordering::AcqRel);
                Ok(())
            },
        )
        .unwrap();
    group.begin_draining();
    idle.block_on(async {
        timeout(WAIT, group.join()).await.unwrap();
    });
    assert_eq!(invoked.load(Ordering::Acquire), 0);
    assert!(matches!(group.entry(&id), Err(HttpError::Cancelled)));
    assert!(fixture.limits.acquire("alice", "installed").is_ok());
}

#[test]
fn pending_starts_are_bounded_and_cannot_rebind_another_catalog() {
    let mut fixture = Fixture::new();
    let group = fixture.group();
    let foreign = Fixture::new();
    assert!(matches!(
        group.prepare(foreign.target(), false),
        Err(HttpError::Provenance)
    ));
    let mut pending = (0..4)
        .map(|_| group.prepare(fixture.target(), false).unwrap())
        .collect::<Vec<_>>();
    assert!(matches!(
        group.prepare(fixture.target(), false),
        Err(HttpError::Busy)
    ));
    assert!(group.0.state.lock().unwrap().entries.is_empty());
    drop(pending.pop());
    let pending_again = group.prepare(fixture.target(), false).unwrap();
    fixture.shutdown();
    assert!(fixture.observed.was_reaped());
    assert!(!fixture.observed.lease_was_dropped());
    drop(pending_again);
    drop(pending);
    fixture.runtime.block_on(group.join());
    drop(group);
    assert!(fixture.observed.lease_was_dropped());
}

#[test]
fn writer_rechecks_expiry_cancellation_generation_and_unchanged_generation_revocation() {
    use crate::durable_state::{app_publishers::AppPublisherMutation, apps::AppRegistryMutation};
    use chariox_app_runtime::publisher_trust::TrustDecision;
    let fixture = Fixture::new();
    let group = fixture.group();
    let invoked = Arc::new(AtomicUsize::new(0));
    let mut expired = group.prepare(fixture.target(), false).unwrap();
    Arc::get_mut(&mut expired.entry).unwrap().deadline = Instant::now();
    let count = invoked.clone();
    assert!(matches!(
        fixture.start(expired, &budget(), move |_, _, _, _, _, _| async move {
            count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }),
        Err(HttpError::Deadline)
    ));
    for budget in [
        AppOperationBudget::fixture(Instant::now(), || false),
        AppOperationBudget::fixture(Instant::now() + WAIT, || true),
    ] {
        let count = invoked.clone();
        assert!(matches!(
            fixture.start(
                group.prepare(fixture.target(), false).unwrap(),
                &budget,
                move |_, _, _, _, _, _| async move {
                    count.fetch_add(1, Ordering::AcqRel);
                    Ok(())
                }
            ),
            Err(HttpError::Cancelled)
        ));
    }
    let pending = group.prepare(fixture.target(), false).unwrap();
    fixture
        .store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "http-revoke".into(),
                    authority_ref: "kernel-fixture".into(),
                },
                now_ms: 20,
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .store
            .get_app_installation("alice", "installed")
            .unwrap()
            .generation,
        1
    );
    let count = invoked.clone();
    assert!(matches!(
        fixture.start(pending, &budget(), move |_, _, _, _, _, _| async move {
            count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }),
        Err(HttpError::Provenance)
    ));
    fixture.runtime.block_on(group.join());
    drop(group);
    let fixture = Fixture::new();
    let group = fixture.group();
    let pending = group.prepare(fixture.target(), false).unwrap();
    fixture
        .store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Uninstall {
                installation_id: "installed".into(),
                expected_generation: 1,
                now_ms: 20,
            },
        )
        .unwrap();
    let count = invoked.clone();
    assert!(matches!(
        fixture.start(pending, &budget(), move |_, _, _, _, _, _| async move {
            count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }),
        Err(HttpError::Provenance)
    ));
    assert_eq!(invoked.load(Ordering::Acquire), 0);
    fixture.runtime.block_on(group.join());
}

#[test]
fn unread_full_response_retains_capacity_and_eof_cannot_hide_task_failure() {
    let fixture = Fixture::new();
    let group = fixture.group();
    let pending = group.prepare(fixture.target(), false).unwrap();
    let id = fixture
        .start(pending, &budget(), |_, _, exchange, _, _, _| async move {
            let (_, headers, chunks) = exchange.fixture_parts();
            headers.send(Ok(head())).unwrap();
            chunks.send(Ok(Bytes::from_static(b"first"))).await.unwrap();
            chunks
                .send(Ok(Bytes::from_static(b"second")))
                .await
                .unwrap();
            // A full queue cannot accept an extra error frame. EOF must consult the
            // actual retained task result, not report a successful body completion.
            assert!(chunks.try_send(Err(HttpError::Network)).is_err());
            Err(HttpError::Network)
        })
        .unwrap();
    let other = (0..3)
        .map(|_| group.prepare(fixture.target(), false).unwrap())
        .collect::<Vec<_>>();
    fixture.runtime.block_on(async {
        let cancellation = Cancellation::new().await;
        let mut complete = group.entry(&id).unwrap().completed.clone();
        timeout(WAIT, async {
            loop {
                if complete.borrow_and_update().is_some() {
                    break;
                }
                complete.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            group.prepare(fixture.target(), false),
            Err(HttpError::Busy)
        ));
        assert_eq!(
            group
                .headers(&id, Instant::now() + WAIT, cancellation.token.clone())
                .await
                .unwrap()
                .unwrap()
                .status,
            200
        );
        for expected in [b"first".as_slice(), b"second".as_slice()] {
            let ReadResult::Chunk(bytes) = group
                .read(&id, Instant::now() + WAIT, cancellation.token.clone())
                .await
                .unwrap()
            else {
                panic!("expected preserved response chunk");
            };
            assert_eq!(bytes.as_ref(), expected);
        }
        assert!(matches!(
            group
                .read(&id, Instant::now() + WAIT, cancellation.token.clone())
                .await,
            Err(HttpError::Network)
        ));
        assert!(matches!(group.entry(&id), Err(HttpError::Invalid)));
        timeout(WAIT, group.join()).await.unwrap();
        cancellation.close().await;
    });
    drop(other);
    assert!(fixture.limits.acquire("alice", "installed").is_ok());
}

#[test]
fn drain_cancels_immediately_but_retains_tasks_and_a_cancelled_join_can_resume() {
    let mut fixture = Fixture::new();
    let group = fixture.group();
    let (started, start) = oneshot::channel();
    let (stopped, stop) = oneshot::channel();
    let (release, cleanup) = oneshot::channel();
    let id = fixture
        .start(
            group.prepare(fixture.target(), false).unwrap(),
            &budget(),
            move |_, _, exchange, mut cancellation, lease, _| async move {
                let _ports = exchange;
                let _lease = lease;
                started.send(()).unwrap();
                super::super::cancelled(&mut cancellation).await;
                stopped.send(()).unwrap();
                cleanup.await.unwrap();
                Err(HttpError::Cancelled)
            },
        )
        .unwrap();
    let others = (0..3)
        .map(|_| fixture.limits.acquire("alice", "installed").unwrap())
        .collect::<Vec<_>>();
    fixture.runtime.block_on(async {
        timeout(WAIT, start).await.unwrap().unwrap();
        group.begin_draining();
        assert!(matches!(group.entry(&id), Err(HttpError::Cancelled)));
        timeout(WAIT, stop).await.unwrap().unwrap();
        assert!(matches!(
            fixture.limits.acquire("alice", "installed"),
            Err(HttpError::Busy)
        ));
        let mut joining = Box::pin(group.join());
        std::future::poll_fn(|cx| {
            assert!(joining.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(joining);
    });
    fixture.shutdown();
    assert!(fixture.observed.was_reaped());
    assert!(!fixture.observed.lease_was_dropped());
    release.send(()).unwrap();
    fixture.runtime.block_on(async {
        timeout(WAIT, group.join()).await.unwrap();
    });
    assert!(fixture.limits.acquire("alice", "installed").is_ok());
    drop(others);
    drop(group);
    assert!(fixture.observed.lease_was_dropped());
}

#[test]
fn concurrent_reads_and_writes_are_bounded_and_interrupted_upload_is_never_replayed() {
    let fixture = Fixture::new();
    let group = fixture.group();
    let id = fixture
        .start(
            group.prepare(fixture.target(), true).unwrap(),
            &budget(),
            |_, _, exchange, stopped, _, _| async move {
                let _ports = exchange; // Hold both real queues without consuming upload.
                wait_stopped(stopped).await
            },
        )
        .unwrap();
    fixture.runtime.block_on(async {
        let mut cancellation = Cancellation::new().await;
        for _ in 0..2 {
            group
                .write(
                    &id,
                    Bytes::from_static(b"accepted"),
                    false,
                    Instant::now() + WAIT,
                    cancellation.token.clone(),
                )
                .await
                .unwrap();
        }
        let mut reading =
            Box::pin(group.read(&id, Instant::now() + WAIT, cancellation.token.clone()));
        std::future::poll_fn(|cx| {
            assert!(reading.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert!(matches!(
            group
                .headers(&id, Instant::now() + WAIT, cancellation.token.clone())
                .await,
            Err(HttpError::Busy)
        ));
        drop(reading);
        let mut writing = Box::pin(group.write(
            &id,
            Bytes::from_static(b"blocked"),
            true,
            Instant::now() + WAIT,
            cancellation.token.clone(),
        ));
        std::future::poll_fn(|cx| {
            assert!(writing.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert!(matches!(
            group
                .write(
                    &id,
                    Bytes::from_static(b"parallel"),
                    false,
                    Instant::now() + WAIT,
                    cancellation.token.clone()
                )
                .await,
            Err(HttpError::Busy)
        ));
        cancellation.cancel().await;
        assert_eq!(writing.await, Err(HttpError::Cancelled));
        assert!(matches!(
            group
                .write(
                    &id,
                    Bytes::from_static(b"retry"),
                    false,
                    Instant::now() + WAIT,
                    cancellation.token.clone()
                )
                .await,
            Err(HttpError::Invalid)
        ));
        timeout(WAIT, group.join()).await.unwrap();
        cancellation.close().await;
    });
}
