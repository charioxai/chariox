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
    let (store, id) = store();
    store.bind_environment("room", &id, 1, |_| Ok(())).unwrap();
    let error = store
        .queue_environment_use(&id, Some("other-room"), "browser_controller.route")
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("belongs to another Room"));
    let lifecycle = store.try_begin_operation(&id, "slice.stop").unwrap();
    let error = store
        .queue_environment_use(&id, Some("room"), "browser_controller.route")
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("slice.stop"));
    drop(lifecycle);
    store
        .queue_environment_use(&id, Some("room"), "browser_controller.route")
        .await
        .unwrap();
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
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        store.queue_environment_use(&id, None, "browser_controller.route"),
    )
    .await
    .unwrap();
    assert!(result.err().unwrap().to_string().contains("quarantined"));
    drop(route);
}
