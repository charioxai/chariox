//! Local release replacement on the real writer/lifecycle with the libc worker
//! fixture: the old generation's worker is drained, the new one must pass
//! health before commit, and a failed start keeps the old generation.
use super::first_install::{approve, enrolled, prepare};
use super::*;
use crate::durable_state::{
    app_installation_operations::{InstallInput, InstallOperation, UpdateTarget},
    app_state::fixture_release_package,
};
use chariox_app_runtime::installation::VerifiedInstallCandidate;

/// Stages release `version` of the fixture App as an update of `installation`.
fn stage_update(
    store: &DurableKernelStateStore,
    installation: &str,
    request: &str,
    version: &str,
    schema: u32,
) -> InstallOperation {
    let (bytes, publisher) = fixture_release_package(version, schema, false);
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
    let trust = store
        .trusted_app_publisher("alice", "com.example", "state-key")
        .unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap();
    let budget = || AppOperationBudget::from_supervisor(|| false);
    store
        .reserve_app_install(
            "alice",
            request,
            InstallInput {
                session_id: "session".into(),
                upload_handle: format!("upload_{}", "b".repeat(64)),
                update: Some(UpdateTarget {
                    installation_id: installation.into(),
                    expected_generation: 1,
                }),
            },
            &candidate.release_metadata().package_digest.clone(),
            budget(),
        )
        .unwrap();
    let operation = store
        .complete_app_install_preparation("alice", request, candidate, budget())
        .unwrap();
    approve(store, &operation);
    operation
}

#[test]
fn an_update_drains_the_old_worker_and_commits_only_a_healthy_new_generation() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let bytes = enrolled(&store);
    let (control, observations) = make_control(&store, Arc::new(NativeFixture::compile().unwrap()));
    let (_, first) = prepare(&control, &runtime, &bytes);
    approve(&store, &first);
    let id = first.token.installation_id.clone();
    control
        .lifecycle()
        .start_first_blocking("alice", "first_request", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", &id).is_some());

    // A new release that fails its health check never replaces generation 1.
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fail_health = true;
    stage_update(&store, &id, "bad_update", "1.0.1", 0);
    control
        .lifecycle()
        .start_first_blocking("alice", "bad_update", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .first_app_install_status("alice", "bad_update")
            .unwrap()
            .phase
            == InstallPhase::Failed
    });
    assert_eq!(
        store
            .first_app_install_status("alice", "bad_update")
            .unwrap()
            .failure
            .as_deref(),
        Some("app_lifecycle_health")
    );
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, 1);
    assert_eq!(installation.pending_generation, None);
    assert!(!installation.admission_paused);
    // The old generation was drained, not user-stopped: use starts it again.
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fail_health = false;
    control
        .lifecycle()
        .start_on_demand_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|lease| lease.catalog().generation() == 1)
    });

    let update = stage_update(&store, &id, "good_update", "1.1.0", 0);
    assert_eq!(update.token.base_generation, 1);
    control
        .lifecycle()
        .start_first_blocking("alice", "good_update", runtime.handle().clone())
        .unwrap();
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|lease| lease.catalog().generation() == update.token.generation)
    });
    assert_eq!(
        store
            .first_app_install_status("alice", "good_update")
            .unwrap()
            .phase,
        InstallPhase::Committed
    );
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, update.token.generation);
    assert_eq!(installation.active.unwrap().release.version, "1.1.0");
    let worker = store.app_worker_status("alice", &id).unwrap().unwrap();
    assert_eq!(worker.generation, update.token.generation);
    assert_eq!(worker.phase, WorkerPhase::Running);
    // Every earlier worker process was reaped; only the new one is live.
    {
        let observations = observations.lock().unwrap();
        let live = observations.iter().filter(|v| !v.was_reaped()).count();
        assert_eq!(live, 1);
    }
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    assert!(all_reaped(&observations));
    control.lifecycle().shutdown_blocking().unwrap();
}

/// The installation's structured state as the kernel stored it.
fn state_value(store: &DurableKernelStateStore, key: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    rusqlite::Connection::open(store.path())
        .unwrap()
        .query_row(
            "SELECT value_json FROM app_state_values WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

fn installed(
    store: &DurableKernelStateStore,
    runtime: &tokio::runtime::Runtime,
) -> (AppControlService, Arc<Mutex<Vec<Observation>>>, String) {
    let bytes = enrolled(store);
    let (control, observations) = make_control(store, Arc::new(NativeFixture::compile().unwrap()));
    let (_, first) = prepare(&control, runtime, &bytes);
    approve(store, &first);
    let id = first.token.installation_id.clone();
    control
        .lifecycle()
        .start_first_blocking("alice", "first_request", runtime.handle().clone())
        .unwrap();
    wait(|| control.active_app_lease("alice", &id).is_some());
    (control, observations, id)
}

#[test]
fn an_update_with_a_newer_data_schema_migrates_before_it_commits() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = installed(&store, &runtime);
    let update = stage_update(&store, &id, "migrating_update", "2.0.0", 1);
    control
        .lifecycle()
        .start_first_blocking("alice", "migrating_update", runtime.handle().clone())
        .unwrap();
    wait(|| {
        let status = store
            .first_app_install_status("alice", "migrating_update")
            .unwrap();
        matches!(status.phase, InstallPhase::Committed | InstallPhase::Failed)
    });
    let status = store
        .first_app_install_status("alice", "migrating_update")
        .unwrap();
    assert_eq!(
        status.phase,
        InstallPhase::Committed,
        "{:?}",
        status.failure
    );
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|lease| lease.catalog().generation() == update.token.generation)
    });
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.active.unwrap().release.schema_version, 1);
    // The staged worker's migration write committed with the new generation.
    assert_eq!(state_value(&store, "migrated").as_deref(), Some("true"));
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    assert!(all_reaped(&observations));
    control.lifecycle().shutdown_blocking().unwrap();
}

#[test]
fn a_failed_migration_keeps_the_old_generation_and_restores_its_data() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = installed(&store, &runtime);
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fail_migration = true;
    stage_update(&store, &id, "failed_migration", "2.0.0", 1);
    control
        .lifecycle()
        .start_first_blocking("alice", "failed_migration", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .first_app_install_status("alice", "failed_migration")
            .unwrap()
            .phase
            == InstallPhase::Failed
    });
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, 1);
    assert_eq!(installation.pending_generation, None);
    assert!(!installation.admission_paused);
    assert_eq!(installation.active.unwrap().release.schema_version, 0);
    // The write the failed step made was rolled back with the snapshot.
    assert_eq!(state_value(&store, "migrated"), None);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
}

/// A kernel stop right after the update quiesced: the durable operation holds
/// admission paused and a migration begun, and no worker runs. Returns the
/// reopened store and a restarted lifecycle driven only by recovery.
fn crash_after_quiesce(
    scratch: &Scratch,
    runtime: &tokio::runtime::Runtime,
    request: &str,
    fail_migration: bool,
) -> (
    DurableKernelStateStore,
    AppControlService,
    Arc<Mutex<Vec<Observation>>>,
    String,
    InstallOperation,
) {
    let store = scratch.store();
    let (control, observations, id) = installed(&store, runtime);
    let update = stage_update(&store, &id, request, "2.0.0", 1);
    control.lifecycle().shutdown_blocking().unwrap();
    assert!(all_reaped(&observations));
    store
        .claim_first_app_install(
            "alice",
            request,
            &format!("{:032x}", 7u128),
            AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    let paused = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(paused.pending_generation, Some(update.token.generation));
    assert!(paused.admission_paused);
    drop(control);
    drop(store);

    let store = scratch.store();
    let (control, restarted) = make_control(&store, Arc::new(NativeFixture::compile().unwrap()));
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fail_migration = fail_migration;
    wait(|| {
        // Each pass runs now instead of waiting out the 5 s recovery interval,
        // so the drill's time goes to the recovery itself.
        control.lifecycle().0.maintenance.lock().unwrap().next = Instant::now();
        control
            .lifecycle()
            .schedule_recovery(runtime.handle().clone());
        let status = store.first_app_install_status("alice", request).unwrap();
        matches!(status.phase, InstallPhase::Committed | InstallPhase::Failed)
    });
    (store, control, restarted, id, update)
}

#[test]
fn an_update_interrupted_after_quiesce_finishes_on_restart() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let (store, control, observations, id, update) =
        crash_after_quiesce(&scratch, &runtime, "interrupted_update", false);
    let status = store
        .first_app_install_status("alice", "interrupted_update")
        .unwrap();
    assert_eq!(
        status.phase,
        InstallPhase::Committed,
        "{:?}",
        status.failure
    );
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|lease| lease.catalog().generation() == update.token.generation)
    });
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.pending_generation, None);
    assert!(!installation.admission_paused);
    assert_eq!(installation.active.unwrap().release.schema_version, 1);
    assert_eq!(state_value(&store, "migrated").as_deref(), Some("true"));
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    assert!(all_reaped(&observations));
    control.lifecycle().shutdown_blocking().unwrap();
}

#[test]
fn an_update_interrupted_after_quiesce_that_fails_on_restart_keeps_the_old_generation() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let (store, control, observations, id, _) =
        crash_after_quiesce(&scratch, &runtime, "interrupted_failure", true);
    let status = store
        .first_app_install_status("alice", "interrupted_failure")
        .unwrap();
    assert_eq!(status.phase, InstallPhase::Failed);
    let installation = store.get_app_installation("alice", &id).unwrap();
    assert_eq!(installation.generation, 1);
    assert_eq!(installation.pending_generation, None);
    assert!(!installation.admission_paused);
    assert_eq!(installation.active.unwrap().release.schema_version, 0);
    assert_eq!(state_value(&store, "migrated"), None);
    // The old generation is usable again after the failed recovery.
    control
        .lifecycle()
        .0
        .fixture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fail_migration = false;
    control
        .lifecycle()
        .start_on_demand_blocking("alice", &id, runtime.handle().clone())
        .unwrap();
    wait(|| {
        control
            .active_app_lease("alice", &id)
            .is_some_and(|lease| lease.catalog().generation() == 1)
    });
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    assert!(all_reaped(&observations));
    control.lifecycle().shutdown_blocking().unwrap();
}

#[test]
fn an_update_breaks_only_automations_whose_event_schema_changed_and_logs_why() {
    let scratch = Scratch::new();
    let runtime = runtime();
    let store = scratch.store();
    let (control, observations, id) = installed(&store, &runtime);
    let digest = control
        .active_app_lease("alice", &id)
        .unwrap()
        .catalog()
        .schema_digest("changed")
        .unwrap()
        .to_owned();
    // One automation matches the event the update still signs; the other was
    // configured against a schema the update no longer has, with one accepted
    // event it has not delivered.
    let db = rusqlite::Connection::open(store.path()).unwrap();
    for (automation, schema) in [("same", digest.as_str()), ("stale", &"0".repeat(64))] {
        db.execute(
            "INSERT INTO app_automations(owner_id,installation_id,automation_id,revision,event_name,
               event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status,scheduled)
             VALUES('alice',?1,?2,1,'changed',1,?3,'session','publication','endpoint','queue','active',0)",
            rusqlite::params![id, automation, schema],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,
           occurrence_id,occurred_at_ms,event_name,schema_digest,content_digest,automation_revision,
           accepted_generation,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms)
         VALUES('alice',?1,'receipt','stale',1,'occurrence',1,'changed',?2,?2,1,1,1,9007199254740991,
           'accepted',1,0,0)",
        rusqlite::params![id, "0".repeat(64)],
    )
    .unwrap();
    stage_update(&store, &id, "schema_update", "1.1.0", 0);
    control
        .lifecycle()
        .start_first_blocking("alice", "schema_update", runtime.handle().clone())
        .unwrap();
    wait(|| {
        store
            .first_app_install_status("alice", "schema_update")
            .unwrap()
            .phase
            == InstallPhase::Committed
    });
    let status = |automation: &str| -> (String, i64) {
        db.query_row(
            "SELECT status,revision FROM app_automations WHERE automation_id=?1",
            [automation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(status("same"), ("active".into(), 1));
    assert_eq!(status("stale"), ("broken".into(), 2));
    let (level, fields): (String, String) = db
        .query_row(
            "SELECT level,fields_json FROM app_logs WHERE installation_id=?1 AND message LIKE 'Automation stale stopped:%'",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(level, "warn");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fields).unwrap(),
        serde_json::json!({"automation_id": "stale", "undelivered": 1})
    );
    let logged: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM app_logs WHERE message LIKE 'Automation same %'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(logged, 0);
    control.lifecycle().stop_blocking("alice", &id).unwrap();
    assert!(all_reaped(&observations));
    control.lifecycle().shutdown_blocking().unwrap();
}
