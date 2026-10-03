use super::*;
use crate::{
    local::{AppRequestErrorCode as Error, RestoreAppDataSnapshotRequest},
    runtime::app_snapshot_broker::AppSnapshotBroker,
};

#[test]
fn saved_snapshot_restore_drains_worker_blocks_concurrent_start_and_leaves_it_stopped() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let native = Arc::new(NativeFixture::compile().unwrap());
    let store = scratch.store();
    let catalog = fixture_event_catalog(&store);
    stage(&store);
    let (control, observations) = make_control(&store, native);
    let service = control.lifecycle().clone();
    service
        .start_active_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    let data = service
        .0
        .entries
        .lock()
        .unwrap()
        .get(&("alice".into(), "installed".into()))
        .unwrap()
        .control
        .restore_data
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    data.prepare_replace("fixture-file", b"saved")
        .unwrap()
        .publish()
        .unwrap();
    let broker = AppSnapshotBroker::new(
        store.clone(),
        "alice".into(),
        catalog,
        Arc::new(Semaphore::new(8)),
        data.clone(),
        Arc::new(tokio::sync::RwLock::new(())),
    );
    let snapshot = broker.fixture_take().unwrap()["snapshotId"]
        .as_str()
        .unwrap()
        .to_owned();
    data.prepare_replace("fixture-file", b"prior")
        .unwrap()
        .publish()
        .unwrap();
    assert!(matches!(
        crate::runtime::app_snapshot_restore::fixture_with_free_bytes(0, || service
            .restore_snapshot_blocking(
                "alice",
                &RestoreAppDataSnapshotRequest {
                    installation_id: "installed".into(),
                    expected_generation: "1".into(),
                    snapshot_id: snapshot.clone(),
                }
            )),
        Err(Error::LimitExceeded)
    ));
    assert!(!observations.lock().unwrap()[0].was_reaped());
    assert!(control.active_app_lease("alice", "installed").is_some());
    assert!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .desired_running
    );
    assert_eq!(data.read_file("fixture-file", 64).unwrap(), b"prior");
    let mut locked = rusqlite::Connection::open(store.path()).unwrap();
    let transaction = locked
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let worker_service = service.clone();
    let thread = std::thread::spawn(move || {
        worker_service.restore_snapshot_blocking(
            "alice",
            &RestoreAppDataSnapshotRequest {
                installation_id: "installed".into(),
                expected_generation: "1".into(),
                snapshot_id: snapshot,
            },
        )
    });
    wait(|| {
        service
            .0
            .operations
            .lock()
            .unwrap()
            .contains(&("alice".into(), "installed".into()))
    });
    assert_eq!(
        service.start_active_blocking("alice", "installed", runtime.handle().clone()),
        Err(LifecycleError::Busy)
    );
    assert!(matches!(
        service.restore_snapshot_blocking(
            "alice",
            &RestoreAppDataSnapshotRequest {
                installation_id: "installed".into(),
                expected_generation: "1".into(),
                snapshot_id: "snapshot-missing".into(),
            }
        ),
        Err(Error::Busy)
    ));
    transaction.commit().unwrap();
    thread.join().unwrap().unwrap();
    assert!(observations.lock().unwrap()[0].was_reaped());
    assert!(control.active_app_lease("alice", "installed").is_none());
    assert_eq!(data.read_file("fixture-file", 64).unwrap(), b"saved");
    let status = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert_eq!(status.phase, WorkerPhase::Stopped);
    assert!(!status.desired_running);
    assert!(matches!(
        service.start_on_demand_blocking("alice", "installed", runtime.handle().clone()),
        Err(LifecycleError::Stopped)
    ));
    assert_eq!(
        observations.lock().unwrap().len(),
        1,
        "no competing App process started"
    );
}

#[test]
fn saved_snapshot_restore_uninstall_fences_reinstall_and_retires_owned_journal_and_receipt() {
    use crate::{
        durable_state::apps::{AppRegistryMutation, AppRegistryOutcome},
        runtime::app_snapshot_restore as snapshots,
    };
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let scratch = Scratch::new();
    let runtime = runtime();
    let native = Arc::new(NativeFixture::compile().unwrap());
    let store = scratch.store();
    fixture_event_catalog(&store);
    stage(&store);
    let (control, observations) = make_control(&store, native);
    let service = control.lifecycle().clone();
    service
        .start_active_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    let parent = store.path().parent().unwrap().join("app-restores");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&parent)
        .unwrap();
    let journal = parent.join("installed");
    std::fs::create_dir(&journal).unwrap();
    std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(journal.join("journal.json"), b"corrupt interruption").unwrap();
    rusqlite::Connection::open(store.path())
        .unwrap()
        .execute(
            "INSERT INTO app_restore_receipts VALUES('alice','installed','previous')",
            [],
        )
        .unwrap();
    assert!(matches!(
        service.begin_uninstall_blocking("bob", "installed", 1, true),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        service.begin_uninstall_blocking("alice", "installed", 1, false),
        Err(Error::Conflict)
    ));
    assert!(!observations.lock().unwrap()[0].was_reaped());
    assert!(control.active_app_lease("alice", "installed").is_some());
    let gate = service
        .begin_uninstall_blocking("alice", "installed", 1, true)
        .unwrap();
    assert!(observations.lock().unwrap()[0].was_reaped());
    assert!(matches!(
        service.start_active_blocking("alice", "installed", runtime.handle().clone()),
        Err(LifecycleError::Busy)
    ));
    let AppRegistryOutcome::Installation(uninstalled) = store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Uninstall {
                installation_id: "installed".into(),
                expected_generation: 1,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )
        .unwrap()
    else {
        panic!("uninstall outcome");
    };
    assert_eq!(
        rusqlite::Connection::open(store.path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM app_restore_receipts WHERE installation_id='installed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    // Stand in for successful platform erasure by forgetting its private DB data.
    // No signed-volume deletion is claimed by this development fixture.
    store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::ForgetData {
                installation_id: "installed".into(),
                expected_generation: uninstalled.generation,
            },
        )
        .unwrap();
    assert!(matches!(
        snapshots::retire_uninstalled(&store, "bob", "installed", uninstalled.generation),
        Err(Error::NotFound)
    ));
    assert!(journal.exists());
    snapshots::retire_uninstalled(&store, "alice", "installed", uninstalled.generation).unwrap();
    snapshots::retire_uninstalled(&store, "alice", "installed", uninstalled.generation).unwrap();
    assert!(!journal.exists());
    drop(gate);
    snapshots::require_recovery_generation(
        &store,
        "alice",
        "installed",
        uninstalled.generation + 1,
    )
    .unwrap();
}
