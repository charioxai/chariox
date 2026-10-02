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
