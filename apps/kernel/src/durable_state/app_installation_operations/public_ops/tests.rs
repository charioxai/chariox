use super::*;
use crate::durable_state::app_state::{fixture_event_catalog, fixture_event_package};
use chariox_app_package::{verify, VerificationPolicy};

struct Fixture {
    store: DurableKernelStateStore,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-public-install-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        let store = DurableKernelStateStore::open_owned(path.join("kernel.sqlite")).unwrap();
        fixture_event_catalog(&store);
        Self { store, path }
    }
    fn candidate(&self) -> VerifiedInstallCandidate {
        let (bytes, publisher) = fixture_event_package();
        let package = verify(
            &bytes,
            &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
        )
        .unwrap();
        let trust = self
            .store
            .trusted_app_publisher("alice", "com.example", "state-key")
            .unwrap();
        VerifiedInstallCandidate::from_verified(&package, &trust).unwrap()
    }
    fn reserve(&self, id: &str) -> InstallOperation {
        self.store
            .reserve_app_install(
                "alice",
                id,
                input(),
                &self.candidate().release_metadata().package_digest,
                budget(),
            )
            .unwrap()
    }
    fn stage(&self, id: &str) -> InstallOperation {
        self.reserve(id);
        self.store
            .complete_app_install_preparation("alice", id, self.candidate(), budget())
            .unwrap()
    }
    fn arm(&self, id: &str, nonce: &str) -> InstallApprovalChallenge {
        match self
            .store
            .arm_app_install_review("alice", id, nonce, budget())
            .unwrap()
        {
            InstallReviewDisposition::Prompt(value) => value,
            _ => panic!("expected exact pending decision"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.fence_writer();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
fn input() -> InstallInput {
    InstallInput {
        session_id: "session".into(),
        upload_handle: format!("upload_{}", "a".repeat(64)),
    }
}

#[test]
fn preparing_ack_replay_and_cancel_survive_reopen_without_creating_a_stage() {
    let f = Fixture::new();
    let initial = f.reserve("request");
    assert_eq!(initial.phase, InstallPhase::Preparing);
    assert!(initial.review.is_none());
    assert_eq!(initial, f.reserve("request"));
    let mut different = input();
    different.session_id = "other-session".into();
    assert!(matches!(
        f.store.reserve_app_install(
            "alice",
            "request",
            different,
            &initial.package_digest,
            budget()
        ),
        Err(InstallOperationError::Conflict)
    ));
    assert!(matches!(
        f.store.first_app_install_status("bob", "request"),
        Err(InstallOperationError::NotFound)
    ));
    let cancelled = f
        .store
        .cancel_first_app_install("alice", "request", budget())
        .unwrap();
    assert_eq!(cancelled.phase, InstallPhase::Cancelled);
    assert_eq!(
        cancelled,
        f.store
            .complete_app_install_preparation("alice", "request", f.candidate(), budget())
            .unwrap()
    );
    assert!(f
        .store
        .get_app_installation("alice", &initial.token.installation_id)
        .is_err());
    let reopened = DurableKernelStateStore::open_owned(f.store.path()).unwrap();
    assert_eq!(
        cancelled,
        reopened
            .replay_public_app_install("alice", "request", budget())
            .unwrap()
    );
}

#[test]
fn verified_review_nonce_and_original_deadline_fence_fresh_writer_budgets() {
    let f = Fixture::new();
    let operation = f.stage("review");
    let old = Arc::new(f.arm("review", "old-decision"));
    let mut current = f.arm("review", "current-decision");
    assert_eq!(current.review()["packageDigest"], operation.package_digest);
    assert_eq!(current.review()["informationSetConsent"], "not_granted");
    assert!(matches!(
        f.store.decide_app_install(old, true, budget()),
        Err(InstallOperationError::Conflict)
    ));
    current.deadline = std::time::Instant::now();
    assert!(matches!(
        f.store
            .decide_app_install(Arc::new(current), true, budget()),
        Err(InstallOperationError::Stopped)
    ));
    let pending = f
        .store
        .app_installation_journal("alice", &operation.token.installation_id)
        .unwrap();
    assert_eq!(pending[0].decision, CapabilityDecision::Pending);
    let accepted = Arc::new(f.arm("review", "new-human-decision"));
    f.store
        .decide_app_install(accepted, true, budget())
        .unwrap();
    assert!(matches!(
        f.store
            .arm_app_install_review("alice", "review", "restart", budget())
            .unwrap(),
        InstallReviewDisposition::Approved
    ));
    assert!(f
        .store
        .get_app_installation("alice", &operation.token.installation_id)
        .unwrap()
        .active
        .is_none());
}

#[test]
fn cancellation_before_commit_rolls_back_preparation_and_decline_is_terminal() {
    let f = Fixture::new();
    let preparing = f.reserve("cancelled-stage");
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let read = cancelled.clone();
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observe = checks.clone();
    let flip = cancelled.clone();
    let expired_budget = AppOperationBudget::from_supervisor(move || {
        read.load(std::sync::atomic::Ordering::Acquire)
    })
    .fixture_observe_checks(Arc::new(move || {
        if observe.fetch_add(1, std::sync::atomic::Ordering::AcqRel) == 0 {
            flip.store(true, std::sync::atomic::Ordering::Release);
        }
    }));
    assert!(matches!(
        f.store.complete_app_install_preparation(
            "alice",
            "cancelled-stage",
            f.candidate(),
            expired_budget
        ),
        Err(InstallOperationError::Stopped)
    ));
    assert_eq!(
        f.store
            .first_app_install_status("alice", "cancelled-stage")
            .unwrap(),
        preparing
    );
    assert!(f
        .store
        .get_app_installation("alice", &preparing.token.installation_id)
        .is_err());
    let staged = f.stage("decline");
    let challenge = Arc::new(f.arm("decline", "no"));
    assert_eq!(
        f.store
            .decide_app_install(challenge, false, budget())
            .unwrap()
            .phase,
        InstallPhase::Cancelled
    );
    assert_eq!(f.reserve("decline").phase, InstallPhase::Cancelled);
    assert!(f
        .store
        .get_app_installation("alice", &staged.token.installation_id)
        .unwrap()
        .active
        .is_none());
}

#[test]
fn old_operation_table_migrates_without_losing_receipts() {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE app_installation_operations(owner_id TEXT,request_id TEXT,installation_id TEXT,package_digest TEXT,phase TEXT CHECK(phase IN ('approval','starting','committed','cancelled','failed')),attempt TEXT,approval_json TEXT,failure TEXT,cleanup_pending INTEGER,created_ms INTEGER,updated_ms INTEGER,PRIMARY KEY(owner_id,request_id));
        INSERT INTO app_installation_operations VALUES('alice','old','old-install','sha256:old','cancelled',NULL,NULL,'declined',1,1,2);").unwrap();
    initialize(&connection).unwrap();
    initialize(&connection).unwrap();
    let receipt = load(&connection, "alice", "old").unwrap().unwrap();
    assert_eq!(receipt.phase, InstallPhase::Cancelled);
    assert!(receipt.input.is_none());
    assert_eq!(receipt.failure.as_deref(), Some("declined"));
}
