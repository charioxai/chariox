use super::*;
use chariox_app_runtime::worker_process::test_fixture::Mode;
use serde_json::{json, Value};

fn setup(
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
        InstallPhase::Cancelled
    );
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, 1);
    assert!(!installation.admission_paused);
    assert_eq!(installation.pending_generation, None);
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
    wait_long(|| observations.lock().unwrap()[1].was_reaped());
    assert!(
        before.elapsed() >= Duration::from_secs(29) && before.elapsed() < Duration::from_secs(35)
    );
    assert_eq!(names(&frames(&observations, 1)), ["startup", "resume"]);
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
    control.lifecycle().shutdown_blocking().unwrap();
    assert_eq!(
        names(&frames(&observations, 0)),
        [
            "startup",
            "configuration_change",
            "configuration_change",
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
