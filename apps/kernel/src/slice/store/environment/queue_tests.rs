use super::*;
use futures_util::poll;
use std::time::Duration;
use tokio::time::Instant;

fn store() -> (SliceStore, String) {
    let store = SliceStore::default();
    let slice = store
        .create(
            "home",
            "machine",
            super::super::super::tests::create_input("desktop"),
        )
        .unwrap();
    (store, slice.id)
}

#[tokio::test]
async fn mp08_mp11_display_readers_share_admission_and_exclude_lifecycle() {
    let (store, id) = store();
    let first = store
        .queue_environment_use(&id, None, SHARED_DISPLAY_READ_OPERATION)
        .await
        .unwrap();
    let second = tokio::time::timeout(
        Duration::from_millis(100),
        store.queue_environment_use(&id, None, SHARED_DISPLAY_READ_OPERATION),
    )
    .await
    .expect("MP-08: another viewer must not wait for a network-bound frame read")
    .unwrap();
    assert!(store.try_begin_operation(&id, "slice.stop").is_err());
    drop((first, second));
    store.try_begin_operation(&id, "slice.stop").unwrap();
}

#[tokio::test]
async fn mp10_display_resume_does_not_wait_behind_controller_routes() {
    for exclusive in ["browser_controller.route", "environment.display.open"] {
        let (store, id) = store();
        let route = store
            .queue_environment_use(&id, None, exclusive)
            .await
            .unwrap();
        let queued = store.queue_environment_use(&id, None, "browser_controller.route");
        tokio::pin!(queued);
        assert!(poll!(&mut queued).is_pending());
        let resume = tokio::time::timeout(
            Duration::from_millis(100),
            store.queue_environment_use(&id, None, SHARED_DISPLAY_READ_OPERATION),
        )
        .await
        .expect("MP-10: recovery must not wait for a controller route backlog")
        .unwrap();
        assert!(store.try_begin_operation(&id, "slice.stop").is_err());
        drop(resume);
        drop(route);
        drop(queued.await.unwrap());
        store.try_begin_operation(&id, "slice.stop").unwrap();
    }
}

#[tokio::test]
async fn environment_queue_is_fifo_across_routes_and_viewer_reads() {
    let (store, id) = store();
    let blocker = store
        .queue_environment_use(&id, None, "browser_controller.route")
        .await
        .unwrap();
    let first = store.queue_environment_use(&id, None, "environment.screenshot.capture");
    let second = store.queue_environment_use(&id, None, "browser_controller.route");
    tokio::pin!(first, second);
    assert!(poll!(&mut first).is_pending());
    assert!(poll!(&mut second).is_pending());
    drop(blocker);
    let first = first.await.unwrap();
    assert!(
        poll!(&mut second).is_pending(),
        "background route must not overtake an earlier viewer read"
    );
    assert!(store.try_begin_operation(&id, "slice.stop").is_err());
    drop(first);
    drop(second.await.unwrap());
    store.try_begin_operation(&id, "slice.stop").unwrap();
}

#[tokio::test]
async fn environment_queue_deadline_leaves_no_operation_or_waiter() {
    let (store, id) = store();
    for queued_blocker in [false, true] {
        let raw;
        let queued;
        if queued_blocker {
            queued = Some(
                store
                    .queue_environment_use(&id, None, "browser_controller.route")
                    .await
                    .unwrap(),
            );
            raw = None;
        } else {
            raw = Some(
                store
                    .guard_environment_use(&id, None, "browser_controller.route")
                    .unwrap(),
            );
            queued = None;
        }
        let error = store
            .queue_environment_use_until(
                &id,
                None,
                "browser_controller.route",
                Instant::now() + Duration::from_millis(30),
            )
            .await
            .err()
            .expect("occupied route must have a bounded wait");
        assert!(error
            .to_string()
            .contains(ENVIRONMENT_USE_ADMISSION_EXPIRED));
        drop(raw);
        drop(queued);
        drop(
            store
                .queue_environment_use(&id, None, "browser_controller.route")
                .await
                .unwrap(),
        );
        store.try_begin_operation(&id, "slice.stop").unwrap();
    }
}

#[tokio::test]
async fn environment_queue_dropped_waiter_does_not_block_following_route() {
    let (store, id) = store();
    let blocker = store
        .queue_environment_use(&id, None, "browser_controller.route")
        .await
        .unwrap();
    let mut abandoned =
        Box::pin(store.queue_environment_use(&id, None, "browser_controller.route"));
    assert!(poll!(&mut abandoned).is_pending());
    drop(abandoned);
    drop(blocker);
    tokio::time::timeout(
        Duration::from_millis(100),
        store.queue_environment_use(&id, None, "browser_controller.route"),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn environment_queue_preserves_lifecycle_and_room_scope_refusals() {
    for operation in ["browser_controller.route", SHARED_DISPLAY_READ_OPERATION] {
        let (store, id) = store();
        store.bind_environment("room", &id, 1, |_| Ok(())).unwrap();
        let error = store
            .queue_environment_use(&id, Some("other-room"), operation)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("belongs to another Room"));
        let lifecycle = store.try_begin_operation(&id, "slice.stop").unwrap();
        let error = store
            .queue_environment_use(&id, Some("room"), operation)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("slice.stop"));
        drop(lifecycle);
        store
            .queue_environment_use(&id, Some("room"), operation)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn environment_queue_refuses_quarantine_even_with_a_controller_route_active() {
    let (store, id) = store();
    let route = store
        .guard_environment_use(&id, None, "browser_controller.route")
        .unwrap();
    store.replay_backup_restore_started(super::super::super::tests::restore_transaction(
        "restore", &id,
    ));
    for operation in ["browser_controller.route", SHARED_DISPLAY_READ_OPERATION] {
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            store.queue_environment_use(&id, None, operation),
        )
        .await
        .unwrap();
        assert!(result.err().unwrap().to_string().contains("quarantined"));
    }
    drop(route);
}

// MP-08/MP-10/MP-11: source regression for marker wakeup, not a wall-clock gate.
#[tokio::test(start_paused = true)]
async fn mp10_environment_marker_release_wakes_without_advancing_a_poll_timer() {
    let (store, id) = store();
    let route = store
        .guard_environment_use(&id, None, "browser_controller.route")
        .unwrap();
    let waiting = store.queue_environment_use(&id, None, "environment.computer.observe");
    tokio::pin!(waiting);
    assert!(poll!(&mut waiting).is_pending());
    drop(route);
    assert!(
        poll!(&mut waiting).is_ready(),
        "MP-10: a settled route must notify its admitted waiter without a 10ms timer"
    );
}

#[tokio::test]
async fn mp11_queue_release_retains_route_ordering_and_lifecycle_exclusion() {
    let (store, id) = store();
    let mut route = store
        .queue_environment_use(&id, None, "browser_controller.route")
        .await
        .unwrap();
    route.release_queue();
    let queue = store.inner.lock().unwrap().environment_use_queues[&id].clone();
    drop(
        queue
            .admission
            .try_read()
            .expect("MP-10: peer I/O must not retain the slice write lock"),
    );
    let read = store.queue_environment_use(&id, None, "environment.computer.observe");
    tokio::pin!(read);
    assert!(
        poll!(&mut read).is_pending(),
        "MP-11: input marker still orders later observation"
    );
    assert!(store.try_begin_operation(&id, "slice.stop").is_err());
    drop(route);
    let read = read.await.unwrap();
    assert!(store.try_begin_operation(&id, "slice.stop").is_err());
    drop(read);
    store.try_begin_operation(&id, "slice.stop").unwrap();
}
