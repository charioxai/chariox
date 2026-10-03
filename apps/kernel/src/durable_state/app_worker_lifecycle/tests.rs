use super::*;
use crate::durable_state::{
    app_publishers::AppPublisherMutation, app_state::fixture_event_catalog,
};
use chariox_app_runtime::publisher_trust::TrustDecision;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-worker-state-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.0.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
fn claim(store: &DurableKernelStateStore, attempt: &str) -> ActiveStartAdmission {
    store
        .claim_active_app_start("alice", "installed", attempt, false, budget())
        .unwrap()
}
fn revoke(store: &DurableKernelStateStore) {
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "state-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "revoke-worker".into(),
                    authority_ref: "kernel-test".into(),
                },
                now_ms: 2,
            },
        )
        .unwrap();
}

#[test]
fn writer_checks_owner_current_signer_and_attempt_without_using_query_connection() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    assert!(store
        .claim_active_app_start("bob", "installed", "wrong-owner", false, budget())
        .is_err());
    // The typed barrier owns the writer connection; a caller's reader guard
    // cannot block it or become an alternate persistence authority.
    let reader = store.connection.lock().unwrap();
    let old = claim(&store, "attempt-1");
    store
        .record_app_worker(&old, WorkerPhase::Running, true, None, budget())
        .unwrap();
    let replacement = claim(&store, "attempt-2");
    assert!(store
        .record_app_worker(
            &old,
            WorkerPhase::Failed,
            true,
            Some("old_failure"),
            budget()
        )
        .is_err());
    store
        .record_app_worker(&replacement, WorkerPhase::Running, true, None, budget())
        .unwrap();
    drop(reader);
    let current = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert_eq!(current.attempt, "attempt-2");
    assert_eq!(current.phase, WorkerPhase::Running);
    assert!(store
        .app_worker_status("bob", "installed")
        .unwrap()
        .is_none());
    revoke(&store);
    assert!(store.verify_app_start(&replacement, budget()).is_err());
    // Reaping a revoked worker still settles its exact attempt's health row.
    store
        .record_app_worker(
            &replacement,
            WorkerPhase::Failed,
            true,
            Some("app_lifecycle_authority"),
            budget(),
        )
        .unwrap();
    assert!(store
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
}

#[test]
fn revocation_between_claim_and_running_never_publishes_running_health() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    let admission = claim(&store, "attempt-1");
    revoke(&store);
    assert!(store
        .record_app_worker(&admission, WorkerPhase::Running, true, None, budget())
        .is_err());
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Starting
    );
    assert!(store
        .claim_active_app_start("alice", "installed", "attempt-2", false, budget())
        .is_err());
}

#[test]
fn stop_intent_survives_reopen_and_late_cleanup_cannot_reenable_recovery() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    let admission = claim(&store, "attempt-1");
    store
        .stop_app_worker_intent("alice", "installed", budget())
        .unwrap();
    assert!(store
        .record_app_worker(&admission, WorkerPhase::Running, true, None, budget())
        .is_err());
    store
        .record_app_worker(&admission, WorkerPhase::Stopped, true, None, budget())
        .unwrap();
    store
        .finish_app_worker_stop("alice", "installed", budget())
        .unwrap();
    drop(store);
    let store = fixture.open();
    let row = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert!(!row.desired_running);
    assert_eq!(row.phase, WorkerPhase::Stopped);
    assert!(store
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
    assert!(matches!(
        store.claim_active_app_start("alice", "installed", "recovery", true, budget()),
        Err(LifecycleStoreError::Stopped)
    ));
    let restarted = claim(&store, "explicit-restart");
    store
        .record_app_worker(&restarted, WorkerPhase::Stopped, true, None, budget())
        .unwrap();
    assert_eq!(
        store.app_worker_recovery_candidates(None).unwrap(),
        vec![("alice".into(), "installed".into())]
    );
    assert!(store
        .finish_app_worker_stop("alice", "installed", budget())
        .is_err());
}

#[test]
fn cancelled_claim_cannot_replace_a_durable_attempt() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    let _ = claim(&store, "original");
    assert!(matches!(
        store.claim_active_app_start(
            "alice",
            "installed",
            "cancelled",
            false,
            AppOperationBudget::from_supervisor(|| true)
        ),
        Err(LifecycleStoreError::Stopped)
    ));
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .attempt,
        "original"
    );
}

#[test]
fn start_gate_refuses_a_failed_current_generation_but_not_an_older_one() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    assert_eq!(
        store.app_worker_start_gate("alice", "installed").unwrap(),
        StartGate::Allowed
    );
    let admission = claim(&store, "attempt-1");
    store
        .record_app_worker(
            &admission,
            WorkerPhase::Failed,
            true,
            Some("crash"),
            budget(),
        )
        .unwrap();
    assert_eq!(
        store.app_worker_start_gate("alice", "installed").unwrap(),
        StartGate::Refused
    );
    // A row left by an older generation (the App was since updated) does not
    // fence the current one.
    let changed = rusqlite::Connection::open(fixture.0.join("kernel.sqlite"))
        .unwrap()
        .execute(
            "UPDATE app_worker_lifecycle SET generation=generation-1 WHERE generation>0",
            [],
        )
        .unwrap();
    assert_eq!(changed, 1);
    assert_eq!(
        store.app_worker_start_gate("alice", "installed").unwrap(),
        StartGate::Allowed
    );
}

#[test]
fn a_failed_worker_restarts_with_backoff_then_is_quarantined() {
    let failed = |failures: u32| WorkerStatus {
        generation: 1,
        attempt: "attempt".into(),
        phase: WorkerPhase::Failed,
        desired_running: true,
        dormant: false,
        failure: Some("app_worker_exited".into()),
        updated_ms: 10_000,
        failures,
    };
    // Restarts wait 1, 4 and 16 seconds after the failure.
    for (failures, wait) in [(1, 1_000), (2, 4_000), (3, 16_000)] {
        assert!(!failed(failures).is_quarantined());
        assert!(!store::restart_allowed(
            &failed(failures),
            10_000 + wait - 1
        ));
        assert!(store::restart_allowed(&failed(failures), 10_000 + wait));
    }
    // A fourth failure in a row quarantines it until an explicit start.
    assert!(failed(4).is_quarantined());
    assert!(!store::restart_allowed(&failed(4), u64::MAX / 2));
}

#[test]
fn a_kernel_restart_turns_a_crashed_running_worker_into_a_recoverable_stop() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    let admission = claim(&store, "attempt-1");
    store
        .record_app_worker(&admission, WorkerPhase::Running, true, None, budget())
        .unwrap();
    // kill -9: the worker ends with its kernel and records nothing.
    drop(store);
    let store = fixture.open();
    let row = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert_eq!(row.phase, WorkerPhase::Stopped);
    assert!(row.desired_running);
    assert_eq!(row.attempt, "attempt-1");
    // Recovery still starts it, and a new claim runs normally.
    assert_eq!(
        store.app_worker_recovery_candidates(None).unwrap(),
        vec![("alice".into(), "installed".into())]
    );
    let restarted = store
        .claim_active_app_start("alice", "installed", "attempt-2", true, budget())
        .unwrap();
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Starting
    );
    // A start still under way when the kernel died stays `starting`: a
    // pending first install resumes that exact claim after the restart.
    drop(restarted);
    drop(store);
    let store = fixture.open();
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Starting
    );
}

#[test]
fn opening_another_kernels_store_never_resets_its_live_workers() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    let admission = claim(&store, "attempt-1");
    store
        .record_app_worker(&admission, WorkerPhase::Running, true, None, budget())
        .unwrap();
    // A sibling kernel reads this store without its owner lock while the
    // owning kernel and its worker are alive.
    let sibling = DurableKernelStateStore::open(fixture.0.join("kernel.sqlite")).unwrap();
    drop(sibling);
    assert_eq!(
        store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .phase,
        WorkerPhase::Running
    );
    assert!(store.verify_app_start(&admission, budget()).is_ok());
}

#[test]
fn committed_suspension_survives_a_crash_before_owner_cleanup() {
    let fixture = Fixture::new();
    let store = fixture.open();
    fixture_event_catalog(&store);
    let admission = claim(&store, "suspended");
    store
        .record_app_worker(&admission, WorkerPhase::Running, true, None, budget())
        .unwrap();
    store.suspend_app_worker(&admission, budget()).unwrap();
    // No stopped transition: the kernel dies after the suspension commits.
    drop(store);
    let store = fixture.open();
    let status = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert_eq!(status.phase, WorkerPhase::Stopped);
    assert!(status.dormant && status.desired_running);
    assert!(store
        .app_worker_recovery_candidates(None)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.app_worker_start_gate("alice", "installed").unwrap(),
        StartGate::Allowed
    );
    let replacement = store
        .claim_active_app_start("alice", "installed", "due-wake", true, budget())
        .unwrap();
    assert!(
        !store
            .app_worker_status("alice", "installed")
            .unwrap()
            .unwrap()
            .dormant
    );
    // An old suspension cannot cross the new attempt's fence.
    assert_eq!(
        store.suspend_app_worker(&admission, budget()),
        Err(LifecycleStoreError::Stale)
    );
    store
        .record_app_worker(&replacement, WorkerPhase::Running, true, None, budget())
        .unwrap();
    store.suspend_app_worker(&replacement, budget()).unwrap();
    store
        .stop_app_worker_intent("alice", "installed", budget())
        .unwrap();
    assert_eq!(
        store.suspend_app_worker(&replacement, budget()),
        Err(LifecycleStoreError::Stopped)
    );
    store
        .record_app_worker(&replacement, WorkerPhase::Stopped, true, None, budget())
        .unwrap();
    let status = store
        .app_worker_status("alice", "installed")
        .unwrap()
        .unwrap();
    assert!(!status.dormant && !status.desired_running);
}

#[test]
fn pre_dormancy_schema_migrates_without_disabling_existing_restart_intent() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE app_worker_lifecycle (
        installation_id TEXT PRIMARY KEY,owner_id TEXT NOT NULL,generation INTEGER NOT NULL,
        attempt TEXT NOT NULL,phase TEXT NOT NULL,desired_running INTEGER NOT NULL,
        failure TEXT,updated_ms INTEGER NOT NULL,failures INTEGER NOT NULL DEFAULT 0);
        INSERT INTO app_worker_lifecycle VALUES('installed','alice',1,'old','running',1,NULL,1,0);",
        )
        .unwrap();
    store::initialize(&connection).unwrap();
    store::initialize(&connection).unwrap();
    store::reset_after_kernel_start(&connection).unwrap();
    let row: (String, bool, bool) = connection
        .query_row(
            "SELECT phase,desired_running,dormant FROM app_worker_lifecycle",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("stopped".into(), true, false));
    // The migrated column enforces the same two-state constraint as a fresh DB.
    assert!(connection
        .execute("UPDATE app_worker_lifecycle SET dormant=2", [])
        .is_err());
}
