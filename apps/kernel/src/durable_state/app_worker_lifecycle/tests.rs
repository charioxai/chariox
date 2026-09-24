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
