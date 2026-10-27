//! Registration expiry uses the real native owner and the production 15 s wait.
use super::*;
use chariox_app_runtime::worker_process::test_fixture::Mode;

fn mode(control: &AppControlService, value: Mode) {
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(value);
}

#[test]
fn registration_deadline_after_migration_rollback_is_retryable_without_restart() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = local_update::installed(&store, &runtime);
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fail_migration = true;
    local_update::stage_update(&store, &id, "migration", "2.0.0", 1);
    control
        .lifecycle()
        .start_first_blocking("alice", "migration", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .first_app_install_status("alice", "migration")
            .unwrap()
            .phase
            == InstallPhase::Failed
    });
    assert_eq!(local_update::state_value(&store, "migrated"), None);
    let previous = store.get_app_installation("alice", &id).unwrap();
    mode(&control, Mode::NoReport);
    control
        .lifecycle()
        .start_active_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .app_worker_status("alice", &id)
            .unwrap()
            .unwrap()
            .phase
            == WorkerPhase::Failed
    });
    let failed = store.app_worker_status("alice", &id).unwrap().unwrap();
    assert!(failed.desired_running);
    assert!(control.active_app_lease("alice", &id).is_none());
    assert!(observations.lock().unwrap().iter().all(|v| v.was_reaped()));
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, previous.generation);
    assert_eq!(installation.active, previous.active);
    assert_eq!(installation.pending_generation, None);
    assert!(!installation.admission_paused);
    mode(&control, Mode::Lifecycle);
    let retry = control
        .lifecycle()
        .start_active_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    assert!(matches!(retry, StartDisposition::Starting { .. }));
    wait(|| control.active_app_lease("alice", &id).is_some());
    let restored = store.app_worker_status("alice", &id).unwrap().unwrap();
    assert_ne!(restored.attempt, failed.attempt);
    assert_eq!(restored.failures, 0);
    assert_eq!(restored.generation, 1);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
    assert_eq!(
        failed.failure.as_deref(),
        Some("app_lifecycle_registration_deadline")
    );
}

#[test]
fn registration_deadline_aborts_pending_update_and_reaps_before_retry() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = local_update::installed(&store, &runtime);
    let previous = store.get_app_installation("alice", &id).unwrap();
    mode(&control, Mode::NoReport);
    local_update::stage_update(&store, &id, "late_update", "1.1.0", 0);
    control
        .lifecycle()
        .start_first_blocking("alice", "late_update", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .first_app_install_status("alice", "late_update")
            .unwrap()
            .phase
            == InstallPhase::Failed
    });
    let failed = store
        .first_app_install_status("alice", "late_update")
        .unwrap();
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, previous.generation);
    assert_eq!(installation.active, previous.active);
    assert_eq!(installation.pending_generation, None);
    assert!(!installation.admission_paused);
    assert!(control.active_app_lease("alice", &id).is_none());
    assert!(observations.lock().unwrap().iter().all(|v| v.was_reaped()));
    mode(&control, Mode::Lifecycle);
    control
        .lifecycle()
        .start_on_demand_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|v| v.catalog().generation() == 1)
    });
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
    assert_eq!(
        failed.failure.as_deref(),
        Some("app_lifecycle_registration_deadline")
    );
}
