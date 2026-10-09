use super::*;
use chariox_app_runtime::worker_process::test_fixture::Mode;
use serde_json::{json, Value};

pub(super) fn setup(
    mode: Mode,
) -> (
    Scratch,
    Runtime,
    DurableKernelStateStore,
    AppControlService,
    Arc<Mutex<Vec<Observation>>>,
) {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    fixture_event_catalog(&store);
    stage(&store);
    let (control, observations) = make_control(&store, Arc::new(NativeFixture::compile().unwrap()));
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(mode);
    control
        .lifecycle()
        .start_active_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    (scratch, runtime, store, control, observations)
}
fn frames(observations: &Mutex<Vec<Observation>>, worker: usize) -> Vec<Value> {
    observations.lock().unwrap()[worker]
        .lifecycle_frames()
        .unwrap()
}
fn names(frames: &[Value]) -> Vec<&str> {
    frames
        .iter()
        .map(|f| f["params"]["event"].as_str().unwrap())
        .collect()
}
fn contains(observations: &Mutex<Vec<Observation>>, worker: usize, event: &str) -> bool {
    frames(observations, worker)
        .iter()
        .any(|f| f["params"]["event"] == event)
}
fn wait_long(mut predicate: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(40);
    while !predicate() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn grant(store: &DurableKernelStateStore, connection: &str) {
    use crate::durable_state::app_connections::ConnectionGrantCommand;
    store
        .app_connection_grant(ConnectionGrantCommand::Grant {
            owner: "alice".into(),
            installation: "installed".into(),
            generator_id: "dev.chariox.slack".into(),
            connection_id: connection.into(),
            now_ms: 1,
        })
        .unwrap();
}

#[test]
fn seeded_catalog_first_start_does_not_emit_idle_resume() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    fixture_event_catalog(&store);
    stage(&store);
    let (control, observations) = make_control(&store, Arc::new(NativeFixture::compile().unwrap()));
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::Lifecycle);
    control.seed_dormant("alice", "installed");
    assert!(control.is_app_dormant("alice", "installed"));
    control
        .lifecycle()
        .start_on_demand_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    assert_eq!(names(&frames(&observations, 0)), ["startup"]);
    control
        .lifecycle()
        .stop_blocking("alice", "installed")
        .unwrap();
    assert_eq!(names(&frames(&observations, 0)), ["startup", "shutdown"]);
    assert!(all_reaped(&observations));
}

#[test]
fn idle_suspends_before_dormancy_and_reactivation_resumes_before_publication() {
    let (_scratch, runtime, _store, control, observations) = setup(Mode::Lifecycle);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    control
        .lifecycle()
        .idle_stop_blocking("alice", catalog, || true)
        .unwrap();
    assert!(control.is_app_dormant("alice", "installed"));
    assert!(observations.lock().unwrap()[0].was_reaped());
    let old = frames(&observations, 0);
    assert_eq!(names(&old), ["startup", "suspend"]);
    assert_eq!(old[1]["params"]["data"], json!({"reason":"idle"}));
    // A slow resume is tested below; publication here requires its successful reply.
    control
        .lifecycle()
        .start_on_demand_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    assert_eq!(names(&frames(&observations, 1)), ["startup", "resume"]);
    assert!(!control.is_app_dormant("alice", "installed"));
    control
        .lifecycle()
        .stop_blocking("alice", "installed")
        .unwrap();
    assert_eq!(
        names(&frames(&observations, 1)),
        ["startup", "resume", "shutdown"]
    );
    assert!(all_reaped(&observations));
}

#[test]
fn dormant_configuration_change_precedes_resume_and_does_not_start_an_idle_worker() {
    let (_scratch, runtime, store, control, observations) = setup(Mode::Lifecycle);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    control
        .lifecycle()
        .idle_stop_blocking("alice", catalog, || true)
        .unwrap();
    grant(&store, "while-dormant");
    assert_eq!(observations.lock().unwrap().len(), 1);
    assert!(control.active_app_lease("alice", "installed").is_none());
    control
        .lifecycle()
        .start_on_demand_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", "installed").is_some());
    let received = frames(&observations, 1);
    assert_eq!(
        names(&received),
        ["startup", "configuration_change", "resume"]
    );
    assert_eq!(
        received[1]["params"]["data"]["previous"],
        json!({"connections":[]})
    );
    assert_eq!(
        received[1]["params"]["data"]["current"],
        received[2]["params"]["data"]["configuration"]
    );
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}

#[test]
fn prepare_update_flushes_old_state_before_fence_drain_and_snapshot() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = local_update::installed(&store, &runtime);
    // Restart the old generation with the fixed lifecycle observer.
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::Lifecycle);
    control
        .lifecycle()
        .start_active_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", &id).is_some());
    let update = local_update::stage_update(&store, &id, "prepared_update", "1.1.0", 0);
    control
        .lifecycle()
        .start_first_blocking("alice", "prepared_update", runtime.handle().clone())
        .unwrap();
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|lease| lease.catalog().generation() == update.token.generation)
    });
    let old = frames(&observations, 1);
    assert_eq!(names(&old), ["startup", "prepare_update", "shutdown"]);
    assert_eq!(
        old[1]["params"]["data"],
        json!({"request_id":"prepared_update"})
    );
    assert_eq!(
        local_update::state_value(&store, "prepared").as_deref(),
        Some("true")
    );
    assert!(observations.lock().unwrap()[1].was_reaped());
    assert_eq!(
        store.get_app_installation("alice", &id).unwrap().generation,
        update.token.generation
    );
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}

fn refused_preparation(mode: Mode, timeout: bool) {
    crate::test_support::isolated_env_test!();
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = local_update::installed(&store, &runtime);
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(mode);
    control
        .lifecycle()
        .start_active_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", &id).is_some());
    local_update::stage_update(&store, &id, "refused_update", "1.1.0", 0);
    let before = Instant::now();
    assert_eq!(
        control.lifecycle().start_first_blocking(
            "alice",
            "refused_update",
            runtime.handle().clone()
        ),
        Err(LifecycleError::Notification)
    );
    let elapsed = before.elapsed();
    if timeout {
        assert!(
            elapsed >= Duration::from_secs(29) && elapsed < Duration::from_secs(34),
            "{elapsed:?}"
        );
    }
    wait(|| observations.lock().unwrap()[1].was_reaped());
    let old = frames(&observations, 1);
    assert_eq!(names(&old), ["startup", "prepare_update"]);
    assert_eq!(
        store
            .first_app_install_status("alice", "refused_update")
            .unwrap()
            .phase,
        InstallPhase::Failed
    );
    assert_eq!(
        store
            .first_app_install_status("alice", "refused_update")
            .unwrap()
            .failure
            .as_deref(),
        Some("app_lifecycle_notification")
    );
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, 1);
    assert!(!installation.admission_paused);
    assert_eq!(installation.pending_generation, None);
    // Worker reaping precedes completion of the retained owner's teardown.
    // A start during that gap correctly returns Existing; wait before requesting a fresh owner.
    wait(|| !control.lifecycle().has_pending_owner("alice", &id));
    // The old generation can be used again after its normal failed-worker backoff.
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::Lifecycle);
    control
        .lifecycle()
        .start_active_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", &id).is_some());
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}
#[test]
fn prepare_update_failure_refuses_update_without_fencing_old_data() {
    refused_preparation(Mode::LifecycleFailPrepare, false);
}
#[test]
fn prepare_update_deadline_refuses_update_and_never_overlaps_shutdown() {
    refused_preparation(Mode::LifecycleHangPrepare, true);
}

#[test]
fn suspend_deadline_terminates_worker_without_dormancy_or_shutdown_overlap() {
    let (_scratch, _runtime, store, control, observations) = setup(Mode::LifecycleHangSuspend);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    let before = Instant::now();
    assert_eq!(
        control
            .lifecycle()
            .idle_stop_blocking("alice", catalog, || true),
        Err(LifecycleError::Notification)
    );
    assert!(
        before.elapsed() >= Duration::from_secs(29) && before.elapsed() < Duration::from_secs(34)
    );
    wait(|| all_reaped(&observations));
    assert!(!control.is_app_dormant("alice", "installed"));
    assert_eq!(names(&frames(&observations, 0)), ["startup", "suspend"]);
    wait(|| !control.lifecycle().has_pending_owner("alice", "installed"));
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Failed
    );
}
#[test]
fn resume_deadline_keeps_worker_unpublished_and_never_overlaps_shutdown() {
    let (_scratch, runtime, store, control, observations) = setup(Mode::Lifecycle);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    control
        .lifecycle()
        .idle_stop_blocking("alice", catalog, || true)
        .unwrap();
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::LifecycleHangResume);
    let before = Instant::now();
    control
        .lifecycle()
        .start_on_demand_blocking("alice", "installed", runtime.handle().clone())
        .unwrap();
    wait(|| {
        let count = observations.lock().unwrap().len();
        count == 2 && contains(&observations, 1, "resume")
    });
    assert!(control.active_app_lease("alice", "installed").is_none());
    // Hold the final write across native reap to pin the cleanup window: the
    // process observation precedes the owner's durable Failed transition.
    let mut writer = rusqlite::Connection::open(store.path()).unwrap();
    writer.busy_timeout(Duration::from_secs(5)).unwrap();
    let delayed_failure = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    wait_long(|| observations.lock().unwrap()[1].was_reaped());
    assert!(
        before.elapsed() >= Duration::from_secs(29) && before.elapsed() < Duration::from_secs(35)
    );
    // The blocked durable write retains the owner; completion cannot precede it.
    assert!(control.lifecycle().has_pending_owner("alice", "installed"));
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Starting
    );
    // Release the writer before waiting for completion: the owner needs it
    // to persist Failed before it can publish that it is finished.
    delayed_failure.rollback().unwrap();
    wait(|| !control.lifecycle().has_pending_owner("alice", "installed"));
    wait(|| {
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .is_some_and(|worker| worker.phase == WorkerPhase::Failed)
    });
    let failed = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert_eq!(failed.phase, WorkerPhase::Failed);
    assert_eq!(
        failed.failure.as_deref(),
        Some("app_lifecycle_notification")
    );
    assert!(control.active_app_lease("alice", "installed").is_none());
    control.lifecycle().shutdown_blocking().unwrap();
    assert_eq!(names(&frames(&observations, 1)), ["startup", "resume"]);
    assert_eq!(observations.lock().unwrap().len(), 2);
    assert!(all_reaped(&observations));
}
#[test]
fn configuration_change_carries_effective_grants_and_skips_idempotent_writes() {
    let (_scratch, _runtime, store, control, observations) = setup(Mode::Lifecycle);
    grant(&store, "connection-1");
    wait(|| contains(&observations, 0, "configuration_change"));
    let received = frames(&observations, 0);
    assert_eq!(
        received[1]["params"]["data"],
        json!({"previous":{"connections":[]},"current":{"connections":[{"generatorId":"dev.chariox.slack","connectionId":"connection-1"}]}})
    );
    let remaining = received[1]["deadline_ms"]
        .as_u64()
        .unwrap()
        .saturating_sub(crate::session::unix_epoch_ms());
    assert!((27000..=30000).contains(&remaining));
    grant(&store, "connection-1");
    std::thread::sleep(Duration::from_millis(2200));
    assert_eq!(
        names(&frames(&observations, 0)),
        ["startup", "configuration_change"]
    );
    use crate::durable_state::app_connections::ConnectionGrantCommand;
    store
        .app_connection_grant(ConnectionGrantCommand::Revoke {
            owner: "alice".into(),
            installation: "installed".into(),
            connection_id: "connection-1".into(),
        })
        .unwrap();
    wait(|| frames(&observations, 0).len() == 3);
    assert_eq!(
        frames(&observations, 0)[2]["params"]["data"]["current"],
        json!({"connections":[]})
    );
    // Receipt of the revocation frame precedes settlement of its callback.
    // Wait for a subsequent acknowledged request on the same owner thread so
    // normal shutdown does not race cancellation of the revocation callback.
    let entry = control
        .lifecycle()
        .0
        .entries
        .lock()
        .unwrap()
        .get(&("alice".to_string(), "installed".to_string()))
        .unwrap()
        .clone();
    entry
        .control
        .notify(
            "prepare_update",
            json!({"request_id":"configuration-settled"}),
        )
        .unwrap();
    control.lifecycle().shutdown_blocking().unwrap();
    assert_eq!(
        names(&frames(&observations, 0)),
        [
            "startup",
            "configuration_change",
            "configuration_change",
            "prepare_update",
            "shutdown"
        ]
    );
}
#[test]
fn configuration_deadline_terminates_without_overlapping_lifecycle() {
    let (_scratch, _runtime, store, _control, observations) =
        setup(Mode::LifecycleHangConfiguration);
    grant(&store, "connection-1");
    wait(|| contains(&observations, 0, "configuration_change"));
    let before = Instant::now();
    wait_long(|| all_reaped(&observations));
    assert!(
        before.elapsed() >= Duration::from_secs(29) && before.elapsed() < Duration::from_secs(35)
    );
    assert_eq!(
        names(&frames(&observations, 0)),
        ["startup", "configuration_change"]
    );
}
#[test]
fn shutdown_wins_over_a_hung_callback_without_waiting_for_its_deadline() {
    let (_scratch, _runtime, store, control, observations) =
        setup(Mode::LifecycleHangConfiguration);
    grant(&store, "connection-1");
    wait(|| contains(&observations, 0, "configuration_change"));
    let before = Instant::now();
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(before.elapsed() < Duration::from_secs(3));
    assert_eq!(
        names(&frames(&observations, 0)),
        ["startup", "configuration_change"]
    );
    assert!(all_reaped(&observations));
}

#[test]
fn configuration_callback_releases_every_shared_admission_permit() {
    let (_scratch, _runtime, store, control, observations) =
        setup(Mode::LifecycleHangConfiguration);
    grant(&store, "connection-1");
    wait(|| contains(&observations, 0, "configuration_change"));
    let permits = control
        .lifecycle()
        .0
        .admission
        .clone()
        .try_acquire_many_owned(8)
        .expect("App callback must not pin shared admission");
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
    drop(permits);
}

#[test]
fn idle_request_returns_without_waiting_for_suspend_and_keeps_one_owner() {
    let (_scratch, runtime, _store, control, observations) = setup(Mode::LifecycleHangSuspend);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    let before = Instant::now();
    control
        .lifecycle()
        .request_idle_stop_blocking("alice", catalog.clone(), || true)
        .unwrap();
    assert!(before.elapsed() < Duration::from_secs(1));
    wait(|| contains(&observations, 0, "suspend"));
    assert_eq!(
        control
            .lifecycle()
            .request_idle_stop_blocking("alice", catalog, || true),
        Err(LifecycleError::Busy)
    );
    assert_eq!(
        control.lifecycle().start_on_demand_blocking(
            "alice",
            "installed",
            runtime.handle().clone()
        ),
        Err(LifecycleError::Busy)
    );
    control.lifecycle().shutdown_blocking().unwrap();
    assert_eq!(names(&frames(&observations, 0)), ["startup", "suspend"]);
    assert!(all_reaped(&observations));
}

#[test]
fn full_dormant_capacity_keeps_the_worker_live_without_dispatch_or_failure() {
    let (_scratch, _runtime, store, control, observations) = setup(Mode::Lifecycle);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    let publisher = &control.lifecycle().0.publisher;
    for index in 0..64 {
        assert!(publisher
            .reserve_dormant(
                &format!("other-{index}"),
                catalog.clone(),
                json!({"connections":[]})
            )
            .unwrap()
            .commit());
    }
    assert_eq!(
        control
            .lifecycle()
            .idle_stop_blocking("alice", catalog, || true),
        Err(LifecycleError::Busy)
    );
    assert!(control.active_app_lease("alice", "installed").is_some());
    assert!(!control.is_app_dormant("alice", "installed"));
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Running
    );
    assert_eq!(names(&frames(&observations, 0)), ["startup"]);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}

#[test]
fn completed_owner_rejects_new_requests_and_returns_non_refusal_for_queued_work() {
    let control = Control::new();
    let receipt = control.enqueue("prepare_update", json!(null)).unwrap();
    control.complete();
    assert_eq!(
        control.wait_notification(receipt),
        Err(LifecycleError::NotificationNotDispatched)
    );
    assert!(matches!(
        control.enqueue("prepare_update", json!(null)),
        Err(LifecycleError::NotificationNotDispatched)
    ));
}

#[test]
fn on_demand_caller_waits_for_a_valid_resume_longer_than_twenty_seconds() {
    let (_scratch, runtime, store, control, observations) = setup(Mode::LifecycleSlowResume);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    control
        .lifecycle()
        .idle_stop_blocking("alice", catalog, || true)
        .unwrap();
    let before = Instant::now();
    // This is the complete production caller path delegated by
    // KernelRuntimeState::app_lease_on_demand, including its actual wait cap.
    let lease = runtime
        .block_on(crate::runtime::app_on_demand::app_lease_on_demand(
            &control,
            &store,
            "alice",
            "installed",
        ))
        .unwrap();
    assert_eq!(lease.catalog().installation_id(), "installed");
    assert!(
        before.elapsed() >= Duration::from_secs(20) && before.elapsed() < Duration::from_secs(30)
    );
    assert_eq!(names(&frames(&observations, 1)), ["startup", "resume"]);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}

#[test]
fn an_old_owner_exit_before_dispatch_does_not_cancel_an_approved_update() {
    use crate::durable_state::app_connections::ConnectionGrantCommand;
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = local_update::installed(&store, &runtime);
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::LifecycleFailConfiguration);
    control
        .lifecycle()
        .start_active_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", &id).is_some());
    let old = control
        .lifecycle()
        .0
        .entries
        .lock()
        .unwrap()
        .get(&("alice".into(), id.clone()))
        .unwrap()
        .control
        .clone();
    let (paused, ready) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel(1);
    let resume = Mutex::new(resume);
    *old.completion_checkpoint.lock().unwrap() = Some(Arc::new(move || {
        let _ = paused.send(());
        let _ = resume.lock().unwrap().recv();
    }));
    store
        .app_connection_grant(ConnectionGrantCommand::Grant {
            owner: "alice".into(),
            installation: id.clone(),
            generator_id: "dev.chariox.slack".into(),
            connection_id: "end-owner".into(),
            now_ms: 1,
        })
        .unwrap();
    ready.recv_timeout(Duration::from_secs(6)).unwrap();
    assert!(observations.lock().unwrap()[1].was_reaped());
    assert!(!old.finished());
    let update = local_update::stage_update(&store, &id, "owner_gone_update", "1.1.0", 0);
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::Lifecycle);
    let service = control.lifecycle().clone();
    let handle = runtime.handle().clone();
    let updating = std::thread::spawn(move || {
        service.start_first_blocking("alice", "owner_gone_update", handle)
    });
    wait(|| old.notification.lock().unwrap().is_some());
    release.send(()).unwrap();
    updating.join().unwrap().unwrap();
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|lease| lease.catalog().generation() == update.token.generation)
    });
    assert_eq!(
        store
            .first_app_install_status("alice", "owner_gone_update")
            .unwrap()
            .phase,
        InstallPhase::Committed
    );
    assert_eq!(
        names(&frames(&observations, 1)),
        ["startup", "configuration_change"]
    );
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}

#[test]
fn explicit_start_after_failure_waits_for_the_retiring_owner() {
    let (_scratch, runtime, store, control, observations) = setup(Mode::LifecycleFailConfiguration);
    let entry = control
        .lifecycle()
        .0
        .entries
        .lock()
        .unwrap()
        .get(&("alice".into(), "installed".into()))
        .unwrap()
        .clone();
    let old = entry.control.clone();
    let (paused, ready) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel(1);
    let resume = Mutex::new(resume);
    *old.completion_checkpoint.lock().unwrap() = Some(Arc::new(move || {
        let _ = paused.send(());
        let _ = resume.lock().unwrap().recv();
    }));
    grant(&store, "fail-before-restart");
    ready.recv_timeout(Duration::from_secs(6)).unwrap();
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Failed
    );
    assert!(observations.lock().unwrap()[0].was_reaped());
    assert!(!old.finished());
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .lifecycle = Some(Mode::Lifecycle);
    let service = control.lifecycle().clone();
    let handle = runtime.handle().clone();
    let (sent, received) = std::sync::mpsc::sync_channel(1);
    let restarting = std::thread::spawn(move || {
        sent.send(service.start_active_blocking("alice", "installed", handle))
            .unwrap();
    });
    // Wait for either the buggy Existing reply or the replacement joining
    // the retained thread. Completion stays blocked until this observation.
    let until = Instant::now() + Duration::from_secs(6);
    let mut early = None;
    let mut reached = false;
    while Instant::now() < until {
        early = received.try_recv().ok();
        let joining = match entry.thread.try_lock() {
            Ok(thread) => thread.is_none(),
            Err(std::sync::TryLockError::WouldBlock) => true,
            Err(error) => panic!("{error}"),
        };
        if early.is_some() || joining {
            reached = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    release.send(()).unwrap();
    restarting.join().unwrap();
    assert!(
        reached,
        "start neither returned nor joined the retiring owner"
    );
    let disposition = early.unwrap_or_else(|| received.recv().unwrap()).unwrap();
    assert!(
        matches!(disposition, StartDisposition::Starting { .. }),
        "{disposition:?}"
    );
    wait(|| control.active_app_lease("alice", "installed").is_some());
    assert_eq!(observations.lock().unwrap().len(), 2);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}

#[test]
fn on_demand_call_during_suspend_keeps_discovery_and_waits_for_the_outcome() {
    let (_scratch, runtime, store, control, observations) = setup(Mode::LifecycleHangSuspend);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    assert!(control
        .lifecycle()
        .request_idle_stop_blocking("alice", catalog, || true)
        .unwrap());
    wait(|| contains(&observations, 0, "suspend"));
    assert!(control.active_app_lease("alice", "installed").is_none());
    assert!(control.is_app_dormant("alice", "installed"));
    assert_eq!(control.dormant_app_catalogs("alice").len(), 1);
    assert!(control.dormant_app_keys().is_empty());
    assert!(control
        .lifecycle()
        .0
        .publisher
        .dormant_configuration("alice", "installed")
        .is_none());
    runtime.block_on(async {
        let lease = crate::runtime::app_on_demand::app_lease_on_demand(
            &control,
            &store,
            "alice",
            "installed",
        );
        tokio::pin!(lease);
        assert!(
            tokio::time::timeout(Duration::from_millis(400), &mut lease)
                .await
                .is_err(),
            "the call must wait for the retained suspension owner"
        );
        let lifecycle = control.lifecycle().clone();
        tokio::task::spawn_blocking(move || lifecycle.shutdown_blocking())
            .await
            .unwrap()
            .unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), lease)
            .await
            .unwrap()
            .is_err());
    });
    assert!(!control.is_app_dormant("alice", "installed"));
    assert!(control.dormant_app_catalogs("alice").is_empty());
    assert_eq!(names(&frames(&observations, 0)), ["startup", "suspend"]);
    assert!(all_reaped(&observations));
}

#[test]
fn on_demand_full_live_limit_without_an_evictable_victim_fails_promptly() {
    let (_scratch, runtime, store, control, observations) = setup(Mode::Lifecycle);
    let catalog = control
        .active_app_lease("alice", "installed")
        .unwrap()
        .catalog()
        .clone();
    control
        .lifecycle()
        .idle_stop_blocking("alice", catalog, || true)
        .unwrap();
    // Saturate the actual admission semaphore: no idle worker can be evicted.
    let live = control
        .lifecycle()
        .0
        .live
        .clone()
        .try_acquire_many_owned(LIVE_LIMIT as u32)
        .unwrap();
    let before = Instant::now();
    runtime.block_on(async {
        assert!(tokio::time::timeout(
            Duration::from_secs(2),
            crate::runtime::app_on_demand::app_lease_on_demand(
                &control,
                &store,
                "alice",
                "installed",
            ),
        )
        .await
        .unwrap()
        .is_err());
    });
    assert!(before.elapsed() < Duration::from_secs(2));
    assert!(control.is_app_dormant("alice", "installed"));
    assert_eq!(observations.lock().unwrap().len(), 1);
    drop(live);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}
