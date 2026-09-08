use super::*;
use crate::{
    durable_state::{
        app_publishers::AppPublisherMutation,
        app_state::{fixture_event_catalog, fixture_event_package},
    },
    runtime::app_control::AppControlService,
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    publisher_trust::TrustDecision,
    release_store::{ReleaseStore, StageBudget},
    worker_process::test_fixture::{Fixture as NativeFixture, Observation},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tokio::runtime::{Builder, Runtime};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "chariox-app-lifecycle-{:016x}",
                rand::random::<u64>()
            ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn store(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            if fs::symlink_metadata(path).is_ok_and(|v| v.is_dir()) {
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        writable(&entry.path());
                    }
                }
            }
        }
        writable(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !predicate() {
        assert!(Instant::now() < deadline, "lifecycle condition timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn stage(store: &DurableKernelStateStore) {
    let (bytes, publisher) = fixture_event_package();
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(
            &verified,
            &bytes,
            StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 1024 * 1024,
            },
        )
        .unwrap();
}
fn make_control(
    store: &DurableKernelStateStore,
    native: Arc<NativeFixture>,
) -> (AppControlService, Arc<Mutex<Vec<Observation>>>) {
    let control = AppControlService::new(store.clone());
    let observations = Arc::new(Mutex::new(Vec::new()));
    *control.lifecycle().0.fixture.lock().unwrap() = Some(start::FixturePlatform {
        native,
        observations: observations.clone(),
    });
    (control, observations)
}
fn all_reaped(observations: &Mutex<Vec<Observation>>) -> bool {
    let observations = observations.lock().unwrap();
    !observations.is_empty()
        && observations
            .iter()
            .all(|v| v.was_reaped() && v.lease_was_dropped())
}

#[test]
fn recovery_starts_without_view_serializes_restart_and_preserves_manual_stop_after_reopen() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let native = Arc::new(NativeFixture::compile().unwrap());
    let store = scratch.store();
    fixture_event_catalog(&store);
    stage(&store);
    let (control, observations) = make_control(&store, native.clone());
    let service = control.lifecycle();
    // This is the daemon's real recovery entry, with no terminal/App view.
    service.schedule_recovery(runtime.handle().clone());
    wait(|| control.active_app_lease("alice", "installed").is_some());
    let running = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert_eq!(running.phase, WorkerPhase::Running);
    assert_eq!(
        service
            .start_active_blocking("alice", "installed", runtime.handle().clone())
            .unwrap(),
        StartDisposition::Existing {
            attempt: running.attempt
        }
    );
    assert_eq!(observations.lock().unwrap().len(), 1);
    let old = control.active_app_lease("alice", "installed").unwrap();
    let mut permits = None;
    wait(|| {
        permits = service.0.admission.clone().try_acquire_many_owned(8).ok();
        permits.is_some()
    });
    let mut locked = rusqlite::Connection::open(store.path()).unwrap();
    locked.busy_timeout(Duration::from_secs(2)).unwrap();
    let transaction = locked
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(matches!(
        service.stop_blocking("alice", "installed"),
        Err(LifecycleError::Busy)
    ));
    assert!(
        old.is_stopped(),
        "stop must withdraw callable handles before shared admission or SQLite waits"
    );
    drop(permits);
    transaction.rollback().unwrap();
    drop(locked);
    service.stop_blocking("alice", "installed").unwrap();
    assert!(old.is_stopped());
    assert!(all_reaped(&observations));
    assert!(control.active_app_lease("alice", "installed").is_none());
    let stopped = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert_eq!(stopped.phase, WorkerPhase::Stopped);
    assert!(!stopped.desired_running);
    service
        .start_active_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    assert_eq!(observations.lock().unwrap().len(), 2);
    // Kernel shutdown is distinct from a user stop: retain restart intent,
    // while every caller returns only after the same actual reap completes.
    let first = service.clone();
    let second = service.clone();
    std::thread::scope(|scope| {
        scope.spawn(move || first.shutdown_blocking());
        scope.spawn(move || second.shutdown_blocking());
    });
    assert!(all_reaped(&observations));
    assert!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .desired_running
    );
    drop(old);
    drop(control);
    drop(store);
    let store = scratch.store();
    let (control, restarted) = make_control(&store, native);
    control
        .lifecycle()
        .schedule_recovery(runtime.handle().clone());
    wait(|| control.active_app_lease("alice", "installed").is_some());
    assert_eq!(restarted.lock().unwrap().len(), 1);
    control
        .lifecycle()
        .stop_blocking("alice", "installed")
        .unwrap();
    control.lifecycle().shutdown_blocking();
    drop(control);
    drop(store);
    let store = scratch.store();
    assert!(store
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
}

#[test]
fn revocation_stops_actual_owner_and_failed_generation_does_not_autoregrant() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    fixture_event_catalog(&store);
    stage(&store);
    let (control, observations) = make_control(&store, Arc::new(NativeFixture::compile().unwrap()));
    control
        .lifecycle()
        .start_active_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    let lease = control.active_app_lease("alice", "installed").unwrap();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "revoke-live".into(),
                    authority_ref: "kernel-test".into(),
                },
                now_ms: 2,
            },
        )
        .unwrap();
    wait(|| {
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .is_some_and(|s| s.phase == WorkerPhase::Failed)
    });
    assert!(lease.is_stopped());
    assert!(all_reaped(&observations));
    assert!(store
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
    control.lifecycle().shutdown_blocking();
}

#[test]
fn per_installation_operation_guard_and_shared_admission_prevent_duplicate_preparation() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    fixture_event_catalog(&store);
    let control = AppControlService::new(store.clone());
    let service = control.lifecycle();
    let gate = service
        .0
        .operation(("alice".into(), "installed".into()))
        .unwrap();
    assert!(matches!(
        service.start_active_blocking("alice", "installed", runtime.handle().clone()),
        Err(LifecycleError::Busy)
    ));
    assert!(matches!(
        service.stop_blocking("alice", "installed"),
        Err(LifecycleError::Busy)
    ));
    drop(gate);
    let permits = service
        .0
        .admission
        .clone()
        .try_acquire_many_owned(8)
        .unwrap();
    assert!(matches!(
        service.start_active_blocking("alice", "installed", runtime.handle().clone()),
        Err(LifecycleError::Busy)
    ));
    drop(permits);
    // No verified stored archive exists. Failure must precede any worker
    // publication; the durable row explains why the generation is not running.
    service
        .start_active_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .is_some_and(|v| v.phase == WorkerPhase::Failed)
    });
    assert!(control.active_app_lease("alice", "installed").is_none());
    service.shutdown_blocking();
    assert_eq!(service.0.live.available_permits(), LIVE_LIMIT);
    assert_eq!(service.0.preparation.available_permits(), 1);
    assert_eq!(service.0.admission.available_permits(), 8);
}
