//! Real SQLite writer transactions. Health values here are explicitly test-only
//! fixtures; actual process/SDK health is covered by app_lifecycle tests.
use super::*;
use crate::durable_state::{
    app_installation_staging::AppVerifiedInstallationMutation,
    app_publishers::AppPublisherMutation,
    app_state::{fixture_event_catalog, fixture_event_package, fixture_tool_package},
    apps::AppRegistryMutation,
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::AppCatalog, app_outbox::EventCatalog, installation::CapabilityDecision,
    publisher_trust::TrustDecision,
};
use std::path::PathBuf;
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Fixture {
    store: DurableKernelStateStore,
    _scratch: Scratch,
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-first-install-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        let store = DurableKernelStateStore::open_owned(path.join("kernel.sqlite")).unwrap();
        fixture_event_catalog(&store);
        Self {
            store,
            _scratch: Scratch(path),
        }
    }
    fn candidate(&self, tool: bool) -> VerifiedInstallCandidate {
        let (bytes, publisher) = if tool {
            fixture_tool_package()
        } else {
            fixture_event_package()
        };
        let verified = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        let trust = self
            .store
            .trusted_app_publisher("alice", "com.example", "state-key")
            .unwrap();
        VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap()
    }
    fn begin(&self, request: &str) -> InstallOperation {
        self.store
            .begin_first_app_install("alice", request, self.candidate(false), budget())
            .unwrap()
    }
    fn approve(&self, operation: &InstallOperation) {
        self.store
            .mutate_app_installation(
                "alice",
                AppRegistryMutation::Decide {
                    token: operation.token.clone(),
                    decision: CapabilityDecision::Approved {
                        approval: CapabilityApproval {
                            decision_id: format!("approve_{}", operation.request_id),
                            authority_ref: "kernel_test_decision".into(),
                        },
                    },
                    now_ms: 2,
                },
            )
            .unwrap();
    }
    fn admitted(&self, request: &str, attempt: &str) -> Arc<ApprovedFirstInstall> {
        let operation = self.begin(request);
        self.approve(&operation);
        Arc::new(
            self.store
                .claim_first_app_install("alice", request, attempt, budget())
                .unwrap(),
        )
    }
    fn revoke(&self) {
        self.store
            .mutate_app_publisher(
                "alice",
                AppPublisherMutation::Revoke {
                    publisher_id: "com.example".into(),
                    key_id: "state-key".into(),
                    expected_revision: 1,
                    decision: TrustDecision {
                        decision_id: "revoke_first".into(),
                        authority_ref: "kernel_test".into(),
                    },
                    now_ms: 5,
                },
            )
            .unwrap();
    }
}
fn health(admission: &ApprovedFirstInstall) -> FirstInstallHealth {
    let (bytes, publisher) = fixture_event_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let catalog =
        Arc::new(AppCatalog::compile(&package, &admission.binding, &admission.trust).unwrap());
    FirstInstallHealth::fixture(
        Arc::new(EventCatalog::compile(&package, catalog).unwrap()),
        budget(),
    )
}

#[test]
fn begin_retry_and_cancel_receipts_survive_reopen_without_resurrection() {
    let fixture = Fixture::new();
    let initial = fixture.begin("request_one");
    assert_eq!(initial, fixture.begin("request_one"));
    assert!(matches!(
        fixture.store.begin_first_app_install(
            "alice",
            "request_one",
            fixture.candidate(true),
            budget()
        ),
        Err(InstallOperationError::Conflict)
    ));
    assert!(matches!(
        fixture.store.first_app_install_status("bob", "request_one"),
        Err(InstallOperationError::NotFound)
    ));
    let cancelled = fixture
        .store
        .cancel_first_app_install("alice", "request_one", budget())
        .unwrap();
    assert_eq!(cancelled.phase, InstallPhase::Cancelled);
    assert!(cancelled.cleanup_pending);
    assert_eq!(fixture.begin("request_one"), cancelled);
    let candidate = fixture.candidate(false);
    let path = fixture.store.path().to_owned();
    drop(fixture.store);
    let reopened = DurableKernelStateStore::open_owned(path).unwrap();
    assert_eq!(
        reopened
            .begin_first_app_install("alice", "request_one", candidate, budget())
            .unwrap(),
        cancelled
    );
    assert!(reopened
        .get_app_installation("alice", &initial.token.installation_id)
        .unwrap()
        .active
        .is_none());
    assert!(matches!(
        reopened.claim_first_app_install("alice", "request_one", "late", budget()),
        Err(InstallOperationError::Conflict) | Err(InstallOperationError::Stale)
    ));
}

#[test]
fn existing_capability_decline_can_terminalize_the_operation_without_resurrection() {
    let fixture = Fixture::new();
    let operation = fixture.begin("declined");
    fixture
        .store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::Decide {
                token: operation.token.clone(),
                decision: CapabilityDecision::Declined {
                    decision_id: "decline_first".into(),
                    authority_ref: "kernel_test".into(),
                },
                now_ms: 2,
            },
        )
        .unwrap();
    let cancelled = fixture
        .store
        .cancel_first_app_install("alice", "declined", budget())
        .unwrap();
    assert_eq!(cancelled.phase, InstallPhase::Cancelled);
    assert_eq!(fixture.begin("declined"), cancelled);
    assert!(fixture
        .store
        .get_app_installation("alice", &operation.token.installation_id)
        .unwrap()
        .active
        .is_none());
}

#[test]
fn approval_attempt_and_current_signer_fence_the_same_stage() {
    let fixture = Fixture::new();
    let operation = fixture.begin("needs_approval");
    assert!(matches!(
        fixture
            .store
            .claim_first_app_install("alice", "needs_approval", "early", budget()),
        Err(InstallOperationError::ApprovalRequired)
    ));
    assert!(fixture
        .store
        .first_app_install_recovery_candidates(None)
        .unwrap()
        .is_empty());
    fixture.approve(&operation);
    assert_eq!(
        fixture
            .store
            .first_app_install_recovery_candidates(None)
            .unwrap(),
        vec![("alice".into(), "needs_approval".into())]
    );
    let old = Arc::new(
        fixture
            .store
            .claim_first_app_install("alice", "needs_approval", "old", budget())
            .unwrap(),
    );
    let current = Arc::new(
        fixture
            .store
            .claim_first_app_install("alice", "needs_approval", "current", budget())
            .unwrap(),
    );
    assert!(matches!(
        fixture
            .store
            .commit_first_app_install(old.clone(), health(&old), budget()),
        Err(InstallOperationError::Conflict)
    ));
    fixture.revoke();
    assert!(matches!(
        fixture
            .store
            .commit_first_app_install(current.clone(), health(&current), budget()),
        Err(InstallOperationError::Stale)
    ));
    assert!(fixture
        .store
        .get_app_installation("alice", &operation.token.installation_id)
        .unwrap()
        .active
        .is_none());
    fixture
        .store
        .finish_first_app_install(current, false, "publisher_revoked", budget())
        .unwrap();
    assert_eq!(
        fixture
            .store
            .first_app_install_status("alice", "needs_approval")
            .unwrap()
            .phase,
        InstallPhase::Failed
    );
}

#[test]
fn foundation_commit_cannot_bypass_supervised_health_transaction() {
    let fixture = Fixture::new();
    let admission = fixture.admitted("supervised", "attempt");
    fixture
        .store
        .mutate_app_installation(
            "alice",
            AppRegistryMutation::MarkPrepared {
                token: admission.binding.token().clone(),
                now_ms: 4,
            },
        )
        .unwrap();
    assert!(fixture
        .store
        .mutate_verified_app_installation(
            "alice",
            AppVerifiedInstallationMutation::Commit {
                token: admission.binding.token().clone(),
                now_ms: 5
            }
        )
        .is_err());
    assert!(fixture
        .store
        .get_app_installation("alice", &admission.binding.token().installation_id)
        .unwrap()
        .active
        .is_none());
}

#[test]
fn commit_receipt_active_generation_and_worker_intent_are_atomic() {
    let fixture = Fixture::new();
    let admission = fixture.admitted("commit", "attempt");
    let committed = fixture
        .store
        .commit_first_app_install(admission.clone(), health(&admission), budget())
        .unwrap();
    assert_eq!(
        committed.activation.installation_id(),
        admission.binding.token().installation_id
    );
    assert_eq!(committed.active.attempt(), "attempt");
    let operation = fixture
        .store
        .first_app_install_status("alice", "commit")
        .unwrap();
    assert_eq!(operation.phase, InstallPhase::Committed);
    let worker = fixture
        .store
        .app_worker_status("alice", &operation.token.installation_id)
        .unwrap()
        .unwrap();
    assert!(worker.desired_running);
    assert_eq!(
        worker.phase,
        super::super::app_worker_lifecycle::WorkerPhase::Starting
    );
    assert!(fixture
        .store
        .cancel_first_app_install("alice", "commit", budget())
        .is_err());
    fixture
        .store
        .finish_first_app_install(admission, true, "late_cancel", budget())
        .unwrap();
    assert_eq!(
        fixture
            .store
            .first_app_install_status("alice", "commit")
            .unwrap(),
        operation
    );
    assert!(fixture
        .store
        .first_app_install_recovery_candidates(None)
        .unwrap()
        .is_empty());
}

#[test]
fn receipt_write_failure_rolls_back_active_release_and_worker_intent() {
    let fixture = Fixture::new();
    let admission = fixture.admitted("sql_failure", "attempt");
    let connection = Connection::open(fixture.store.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_first_commit BEFORE UPDATE OF phase ON app_installation_operations
        WHEN NEW.phase='committed' BEGIN SELECT RAISE(ABORT,'fixture receipt write failure'); END;",
        )
        .unwrap();
    assert!(matches!(
        fixture
            .store
            .commit_first_app_install(admission.clone(), health(&admission), budget()),
        Err(InstallOperationError::Storage)
    ));
    assert!(fixture
        .store
        .get_app_installation("alice", &admission.binding.token().installation_id)
        .unwrap()
        .active
        .is_none());
    assert!(fixture
        .store
        .app_worker_status("alice", &admission.binding.token().installation_id)
        .unwrap()
        .is_none());
    assert_eq!(
        fixture
            .store
            .first_app_install_status("alice", "sql_failure")
            .unwrap()
            .phase,
        InstallPhase::Starting
    );
    fixture.store.require_writer_healthy().unwrap();
    connection
        .execute_batch("DROP TRIGGER fail_first_commit;")
        .unwrap();
    fixture
        .store
        .commit_first_app_install(admission.clone(), health(&admission), budget())
        .unwrap();
}

#[test]
fn lost_commit_ack_reconciles_one_generation_without_replaying_initialization() {
    let fixture = Fixture::new();
    let admission = fixture.admitted("lost_ack", "attempt");
    let committed = fixture
        .store
        .fixture_commit_first_app_install(
            admission.clone(),
            health(&admission),
            budget(),
            CommitTestFault {
                fail_reconciliation: false,
            },
        )
        .unwrap();
    assert_eq!(committed.activation.generation(), 1);
    assert_eq!(fixture.begin("lost_ack").phase, InstallPhase::Committed);
    assert_eq!(
        fixture
            .store
            .app_installation_journal("alice", &admission.binding.token().installation_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn irreconcilable_commit_stops_writer_and_restart_recovers_committed_intent() {
    let fixture = Fixture::new();
    let admission = fixture.admitted("unknown", "attempt");
    assert!(matches!(
        fixture.store.fixture_commit_first_app_install(
            admission.clone(),
            health(&admission),
            budget(),
            CommitTestFault {
                fail_reconciliation: true
            }
        ),
        Err(InstallOperationError::CommitUnknown)
    ));
    fixture.store.fence_writer().unwrap();
    assert!(fixture.store.require_writer_healthy().is_err());
    assert!(fixture
        .store
        .cancel_first_app_install("alice", "unknown", budget())
        .is_err());
    let path = fixture.store.path().to_owned();
    drop(fixture.store);
    let reopened = DurableKernelStateStore::open_owned(path).unwrap();
    assert_eq!(
        reopened
            .first_app_install_status("alice", "unknown")
            .unwrap()
            .phase,
        InstallPhase::Committed
    );
    assert!(reopened
        .app_worker_recovery_candidates(None)
        .unwrap()
        .iter()
        .any(|(owner, id)| owner == "alice" && id == &admission.binding.token().installation_id));
}

#[test]
fn pending_limit_and_cancelled_budget_do_not_leave_an_extra_installation() {
    let fixture = Fixture::new();
    for index in 0..8 {
        fixture.begin(&format!("pending_{index}"));
    }
    assert!(matches!(
        fixture.store.begin_first_app_install(
            "alice",
            "overflow",
            fixture.candidate(false),
            budget()
        ),
        Err(InstallOperationError::Limit)
    ));
    assert!(matches!(
        fixture.store.begin_first_app_install(
            "alice",
            "cancelled",
            fixture.candidate(false),
            AppOperationBudget::from_supervisor(|| true)
        ),
        Err(InstallOperationError::Stopped)
    ));
    let connection = Connection::open(fixture.store.path()).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM app_installation_operations",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        8
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM app_installations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        9
    );
}
