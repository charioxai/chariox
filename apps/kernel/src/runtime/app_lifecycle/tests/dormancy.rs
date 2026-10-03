//! Restart residency with actual native owners and the durable wake writer.
use super::*;
use crate::durable_state::{
    app_state::AppStateOperation,
    app_wakes::{AppWakeOperation, AppWakeOutcome},
};
use chariox_app_runtime::{
    managed_state::{Wake, WakeChange},
    worker_process::test_fixture::Mode,
};

#[test]
fn twelve_idle_apps_survive_restart_and_only_the_due_wake_starts_a_worker() {
    let scratch = Scratch::new();
    let executor = runtime();
    let native = Arc::new(NativeFixture::compile().unwrap());
    let store = scratch.store();
    fixture_event_catalog(&store);
    stage(&store);
    let ids: Vec<_> = std::iter::once("installed".to_owned())
        .chain((1..12).map(|n| format!("idle-{n:02}")))
        .collect();
    for id in &ids[1..] {
        fixture_event_installation(&store, "alice", id);
    }
    let (control, observations) = make_control(&store, native.clone());
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::Lifecycle);
    let now = crate::session::unix_epoch_ms();
    for (n, id) in ids.iter().enumerate() {
        control
            .lifecycle()
            .start_active_blocking("alice", id, executor.handle().clone())
            .unwrap();
        wait(|| control.active_app_lease("alice", id).is_some());
        let catalog = control
            .active_app_lease("alice", id)
            .unwrap()
            .catalog()
            .clone();
        // One wake becomes overdue during downtime; the other eleven are future work.
        store
            .execute_app_state(
                "alice",
                catalog.clone(),
                AppStateOperation::Schedule {
                    wakes: vec![WakeChange::Set(Wake {
                        id: format!("wake-{n}"),
                        due_at_ms: if n == 7 {
                            now + 60_000
                        } else {
                            now + 3_600_000
                        },
                        revision: "r1".into(),
                    })],
                    wakes_count_as_use: true,
                },
                AppOperationBudget::from_supervisor(|| false),
            )
            .unwrap();
        control
            .lifecycle()
            .idle_stop_blocking("alice", catalog, || true)
            .unwrap();
        assert!(
            store
                .app_worker_status("alice", id)
                .unwrap()
                .unwrap()
                .dormant
        );
    }
    assert_eq!(observations.lock().unwrap().len(), ids.len());
    assert!(all_reaped(&observations));
    control.lifecycle().shutdown_blocking().unwrap();
    drop(control);
    drop(store);
    let store = scratch.store();
    let (control, restarted) = make_control(&store, native);
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::Lifecycle);
    assert!(store
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
    assert!(
        control.dormant_app_keys().is_empty(),
        "no in-memory dormancy survives"
    );
    // Drive both normal recovery lanes; neither may start an idle-stopped App.
    for _ in 0..2 {
        control.lifecycle().0.maintenance.lock().unwrap().next = Instant::now();
        control
            .lifecycle()
            .schedule_recovery(executor.handle().clone());
        wait(|| !control.lifecycle().0.maintenance.lock().unwrap().running);
    }
    assert!(restarted.lock().unwrap().is_empty());
    let AppWakeOutcome::Due(due) = store
        .app_wakes(AppWakeOperation::Due {
            now_ms: now + 180_000,
            limit: 8,
        })
        .unwrap()
    else {
        panic!("due wakes");
    };
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].installation_id, ids[7]);
    // The wake pump uses this exact on-demand entry, even with no dormant catalog.
    control
        .lifecycle()
        .start_on_demand_blocking("alice", &due[0].installation_id, executor.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", &ids[7]).is_some());
    let lease = control.active_app_lease("alice", &ids[7]).unwrap();
    executor
        .block_on(lease.deliver_wake(
            &due[0].wake,
            true,
            due[0].counts_as_use,
            Duration::from_secs(5),
        ))
        .unwrap();
    store
        .app_wakes(AppWakeOperation::Delivered(due[0].clone()))
        .unwrap();
    assert_eq!(restarted.lock().unwrap().len(), 1);
    let frames = restarted.lock().unwrap()[0].lifecycle_frames().unwrap();
    let wakes: Vec<_> = frames
        .iter()
        .filter(|f| f["method"] == "schedule.wake")
        .collect();
    assert_eq!(wakes.len(), 1);
    assert_eq!(wakes[0]["params"]["overdue"], true);
    for (n, id) in ids.iter().enumerate() {
        assert_eq!(control.active_app_lease("alice", id).is_some(), n == 7);
        assert_eq!(
            store
                .app_worker_status("alice", id)
                .unwrap()
                .unwrap()
                .dormant,
            n != 7
        );
    }
    assert_eq!(
        store
            .app_wakes(AppWakeOperation::Due {
                now_ms: now + 180_000,
                limit: 8
            })
            .unwrap(),
        AppWakeOutcome::Due(vec![])
    );
    drop(lease);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&restarted));
    drop(control);
    drop(store);
    let store = scratch.store();
    for (n, id) in ids.iter().enumerate() {
        assert_eq!(
            store.has_due_app_wakes("alice", id, now + 3_600_000),
            n != 7
        );
    }
    // A second reopen cannot redeliver the settled firing; future wakes remain.
    assert_eq!(
        store
            .app_wakes(AppWakeOperation::Due {
                now_ms: now + 180_000,
                limit: 8
            })
            .unwrap(),
        AppWakeOutcome::Due(vec![])
    );
    assert_eq!(
        store.app_worker_recovery_candidates(None).unwrap(),
        vec![("alice".into(), ids[7].clone())]
    );
}

#[test]
fn host_suspension_write_failure_keeps_dormant_use_without_spending_app_failures() {
    let (_scratch, executor, store, control, observations) = notifications::setup(Mode::Lifecycle);
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_dormancy BEFORE UPDATE OF dormant ON app_worker_lifecycle
        WHEN NEW.dormant=1 BEGIN SELECT RAISE(ABORT,'fixture storage unavailable'); END;",
        )
        .unwrap();
    for _ in 0..4 {
        let catalog = control
            .active_app_lease("alice", "installed")
            .unwrap()
            .catalog()
            .clone();
        control
            .lifecycle()
            .idle_stop_blocking("alice", catalog, || true)
            .unwrap();
        let status = store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap();
        assert_eq!(status.phase, WorkerPhase::Stopped);
        assert_eq!(status.failures, 0);
        assert!(status.desired_running && !status.dormant);
        assert!(control.is_app_dormant("alice", "installed"));
        assert_eq!(
            store.app_worker_start_gate("alice", "installed").unwrap(),
            crate::durable_state::app_worker_lifecycle::StartGate::Allowed
        );
        control
            .lifecycle()
            .start_on_demand_blocking("alice", "installed", executor.handle().clone())
            .unwrap();
        wait(|| control.active_app_lease("alice", "installed").is_some());
    }
    connection
        .execute_batch("DROP TRIGGER fail_dormancy;")
        .unwrap();
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}
