use super::super::store::initialize;
use super::*;
use crate::durable_state::app_state::{fixture_event_catalog, fixture_event_package};
use chariox_app_package::{verify, VerificationPolicy};

struct Fixture {
    store: DurableKernelStateStore,
    _root: crate::test_support::TestWorktree,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::test_support::TestWorktree::new("public-install");
        let store = DurableKernelStateStore::open_owned(root.path().join("kernel.sqlite")).unwrap();
        fixture_event_catalog(&store);
        Self { store, _root: root }
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
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
fn input() -> InstallInput {
    InstallInput {
        session_id: "session".into(),
        upload_handle: format!("upload_{}", "a".repeat(64)),
        update: None,
        deployment: None,
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
    // Release this kernel's ownership, keeping the database, before reopening.
    let database = f.store.path().to_path_buf();
    let Fixture { store, _root } = f;
    store.fence_writer().unwrap();
    drop(store);
    let reopened = DurableKernelStateStore::open_owned(database).unwrap();
    assert_eq!(
        cancelled,
        reopened
            .replay_public_app_install("alice", "request", budget())
            .unwrap()
    );
    drop(reopened);
    drop(_root);
}

#[test]
fn verified_review_nonce_and_original_deadline_fence_fresh_writer_budgets() {
    let f = Fixture::new();
    let operation = f.stage("review");
    let old = Arc::new(f.arm("review", "old-decision"));
    let mut current = f.arm("review", "current-decision");
    assert_eq!(current.review()["packageDigest"], operation.package_digest);
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
    assert!(!operation.approved);
    let decided = f
        .store
        .decide_app_install(accepted, true, budget())
        .unwrap();
    // Still stored as approval until the start, but the decision is known.
    assert_eq!(decided.phase, InstallPhase::AwaitingApproval);
    assert!(decided.approved);
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

fn release(
    version: &str,
    schema: u32,
    with_network: bool,
    store: &DurableKernelStateStore,
) -> VerifiedInstallCandidate {
    let (bytes, publisher) =
        crate::durable_state::app_state::fixture_release_package(version, schema, with_network);
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let trust = store
        .trusted_app_publisher("alice", "com.example", "state-key")
        .unwrap();
    VerifiedInstallCandidate::from_verified(&package, &trust).unwrap()
}
fn update_input(expected_generation: u64) -> InstallInput {
    InstallInput {
        update: Some(UpdateTarget {
            installation_id: "installed".into(),
            expected_generation,
        }),
        ..input()
    }
}

#[test]
fn an_update_stages_onto_the_active_installation_and_a_failed_start_keeps_it() {
    let f = Fixture::new();
    let next = release("1.1.0", 0, false, &f.store);
    let digest = next.release_metadata().package_digest.clone();
    let reserve = |id: &str, owner: &str, input: InstallInput| {
        f.store
            .reserve_app_install(owner, id, input, &digest, budget())
    };
    assert_eq!(
        reserve("stale", "alice", update_input(2)).unwrap_err(),
        InstallOperationError::Conflict
    );
    assert_eq!(
        reserve("foreign", "bob", update_input(1)).unwrap_err(),
        InstallOperationError::NotFound
    );
    let preparing = reserve("update", "alice", update_input(1)).unwrap();
    assert_eq!(preparing.phase, InstallPhase::Preparing);
    assert_eq!(preparing.token.installation_id, "installed");
    assert_eq!(preparing.token.base_generation, 1);
    // One unfinished operation per installation.
    assert_eq!(
        reserve("second", "alice", update_input(1)).unwrap_err(),
        InstallOperationError::Conflict
    );
    let staged = f
        .store
        .complete_app_install_preparation("alice", "update", next, budget())
        .unwrap();
    assert_eq!(staged.phase, InstallPhase::AwaitingApproval);
    assert_eq!(staged.token.generation, 2);
    let installed = f.store.get_app_installation("alice", "installed").unwrap();
    assert_eq!(installed.generation, 1);
    assert_eq!(installed.pending_generation, Some(2));
    // Unchanged capabilities: the kernel approves by policy, no prompt.
    assert!(matches!(
        f.store
            .arm_app_install_review("alice", "update", "none", budget())
            .unwrap(),
        InstallReviewDisposition::Approved
    ));
    let admission = Arc::new(
        f.store
            .claim_first_app_install("alice", "update", "attempt", budget())
            .unwrap(),
    );
    assert!(
        f.store
            .get_app_installation("alice", "installed")
            .unwrap()
            .admission_paused
    );
    f.store
        .finish_first_app_install(admission, false, "app_worker_health", budget())
        .unwrap();
    let failed = f.store.first_app_install_status("alice", "update").unwrap();
    assert_eq!(failed.phase, InstallPhase::Failed);
    // The old generation stays active and admits work again; its data is kept.
    let installed = f.store.get_app_installation("alice", "installed").unwrap();
    assert_eq!(installed.generation, 1);
    assert_eq!(installed.pending_generation, None);
    assert!(!installed.admission_paused);
    assert!(installed.active.is_some());
    // A release with a newer data schema stages; its update migrates the data.
    let migrating = release("2.0.0", 1, false, &f.store);
    let digest = migrating.release_metadata().package_digest.clone();
    f.store
        .reserve_app_install("alice", "migrate", update_input(1), &digest, budget())
        .unwrap();
    f.store
        .complete_app_install_preparation("alice", "migrate", migrating, budget())
        .unwrap();
}

#[test]
fn an_update_that_changes_capabilities_asks_the_owner_again() {
    let f = Fixture::new();
    let before = f
        .store
        .get_app_installation("alice", "installed")
        .unwrap()
        .active;
    assert!(before.as_ref().is_some_and(|active| active.generation == 1));
    let next = release("1.1.0", 0, true, &f.store);
    let digest = next.release_metadata().package_digest.clone();
    f.store
        .reserve_app_install("alice", "network", update_input(1), &digest, budget())
        .unwrap();
    f.store
        .complete_app_install_preparation("alice", "network", next, budget())
        .unwrap();
    let challenge = Arc::new(f.arm("network", "yes"));
    assert!(challenge.is_update());
    // No early capability use (V1-INT-13): while the owner decides, the
    // running generation keeps its own release and approval.
    let waiting = f.store.get_app_installation("alice", "installed").unwrap();
    assert_eq!(waiting.active, before);
    assert_eq!(waiting.pending_generation, Some(2));
    assert_eq!(
        f.store
            .decide_app_install(challenge, false, budget())
            .unwrap()
            .phase,
        InstallPhase::Cancelled
    );
    // Declining keeps the old release active.
    let installed = f.store.get_app_installation("alice", "installed").unwrap();
    assert_eq!(installed.generation, 1);
    assert_eq!(installed.pending_generation, None);
    assert_eq!(installed.active, before);
}

#[test]
fn uninstalling_during_an_update_leaves_its_operation_cancellable() {
    let f = Fixture::new();
    let next = release("1.1.0", 0, false, &f.store);
    let digest = next.release_metadata().package_digest.clone();
    f.store
        .reserve_app_install("alice", "update", update_input(1), &digest, budget())
        .unwrap();
    f.store
        .complete_app_install_preparation("alice", "update", next, budget())
        .unwrap();
    f.store
        .mutate_app_installation(
            "alice",
            crate::durable_state::apps::AppRegistryMutation::Uninstall {
                installation_id: "installed".into(),
                expected_generation: 1,
                now_ms: 5,
            },
        )
        .unwrap();
    assert_eq!(
        f.store
            .cancel_first_app_install("alice", "update", budget())
            .unwrap()
            .phase,
        InstallPhase::Cancelled
    );
}

#[test]
fn protocol_348_operation_table_migrates_to_generations() {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE app_installation_operations (
        owner_id TEXT NOT NULL, request_id TEXT NOT NULL, installation_id TEXT NOT NULL UNIQUE,
        package_digest TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase IN ('preparing','approval','starting','committed','cancelled','failed')),
        attempt TEXT, approval_json TEXT, failure TEXT,
        cleanup_pending INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_pending IN (0,1)),
        created_ms INTEGER NOT NULL CHECK(created_ms>=0), updated_ms INTEGER NOT NULL CHECK(updated_ms>=0),
        session_id TEXT, upload_handle TEXT, review_json TEXT, interaction_id TEXT,
        PRIMARY KEY(owner_id,request_id));
        INSERT INTO app_installation_operations(owner_id,request_id,installation_id,package_digest,phase,created_ms,updated_ms,session_id,upload_handle,interaction_id)
        VALUES('alice','inflight','app_1','sha256:x','approval',1,2,'session','upload_x','nonce');").unwrap();
    initialize(&connection).unwrap();
    initialize(&connection).unwrap();
    let operation = load(&connection, "alice", "inflight").unwrap().unwrap();
    assert_eq!(operation.phase, InstallPhase::AwaitingApproval);
    assert_eq!(
        (operation.token.base_generation, operation.token.generation),
        (0, 1)
    );
    let input = operation.input.unwrap();
    assert_eq!(
        (input.session_id.as_str(), input.upload_handle.as_str()),
        ("session", "upload_x")
    );
    assert!(input.update.is_none());
    assert_eq!(operation.interaction_id.as_deref(), Some("nonce"));
}

#[test]
fn a_policy_approval_accepts_the_longest_request_id() {
    let f = Fixture::new();
    let next = release("1.1.0", 0, false, &f.store);
    let digest = next.release_metadata().package_digest.clone();
    let request = "r".repeat(128);
    f.store
        .reserve_app_install("alice", &request, update_input(1), &digest, budget())
        .unwrap();
    f.store
        .complete_app_install_preparation("alice", &request, next, budget())
        .unwrap();
    assert!(matches!(
        f.store
            .arm_app_install_review("alice", &request, "none", budget())
            .unwrap(),
        InstallReviewDisposition::Approved
    ));
}

#[test]
fn a_reinstall_updates_the_kept_installation_and_restores_no_configuration() {
    let f = Fixture::new();
    let sql = Connection::open(f.store.path()).unwrap();
    sql.execute_batch(
        "INSERT INTO app_automations(owner_id,installation_id,automation_id,revision,event_name,
            event_version,schema_digest,session_id,publication_id,endpoint_id,queue_id,status)
         VALUES('alice','installed','nightly',1,'ready',1,'sha256:x','s','p','e','q','active');
         INSERT INTO app_inbox_routes(owner_id,installation_id,route_id,event_name,
            source_event_type,source_event_version,active,created_at_ms)
         VALUES('alice','installed','mail','received','mail.received',1,1,1);
         INSERT INTO app_connection_grants(owner_id,installation_id,connection_id,generator_id,
            granted_at_ms) VALUES('alice','installed','c1','g1',1);",
    )
    .unwrap();
    f.store
        .mutate_app_installation(
            "alice",
            crate::durable_state::apps::AppRegistryMutation::Uninstall {
                installation_id: "installed".into(),
                expected_generation: 1,
                now_ms: 5,
            },
        )
        .unwrap();
    let kept = f.store.get_app_installation("alice", "installed").unwrap();
    assert!(kept.active.is_none());
    assert_eq!(kept.retained.as_ref().unwrap().generation, 1);
    let count = |table: &str, filter: &str| -> i64 {
        sql.query_row(
            &format!("SELECT count(*) FROM {table} WHERE installation_id='installed' {filter}"),
            [],
            |row| row.get(0),
        )
        .unwrap()
    };
    // Uninstall disables automations and drops routes and grants.
    assert_eq!(count("app_automations", "AND status='disabled'"), 1);
    assert_eq!(count("app_inbox_routes", ""), 0);
    assert_eq!(count("app_connection_grants", ""), 0);
    // The same release with unchanged capabilities still asks: a reinstall
    // inherits no approval.
    let next = release("1.1.0", 0, false, &f.store);
    let digest = next.release_metadata().package_digest.clone();
    f.store
        .reserve_app_install("alice", "reinstall", update_input(2), &digest, budget())
        .unwrap();
    f.store
        .complete_app_install_preparation("alice", "reinstall", next, budget())
        .unwrap();
    let challenge = Arc::new(f.arm("reinstall", "yes"));
    assert!(challenge.is_reinstall());
    let approved = f
        .store
        .decide_app_install(challenge, true, budget())
        .unwrap();
    assert_eq!(approved.phase, InstallPhase::AwaitingApproval);
    assert!(approved.approved);
    assert_eq!(count("app_automations", "AND status='active'"), 0);
}

#[test]
fn a_reinstall_needs_kept_data() {
    let f = Fixture::new();
    Connection::open(f.store.path())
        .unwrap()
        .execute_batch(
            "UPDATE app_installations SET active_json=NULL, retained_json=NULL,
             generation=2, allocated_generation=2 WHERE installation_id='installed'",
        )
        .unwrap();
    let next = release("1.1.0", 0, false, &f.store);
    let digest = next.release_metadata().package_digest.clone();
    assert_eq!(
        f.store
            .reserve_app_install("alice", "reinstall", update_input(2), &digest, budget())
            .unwrap_err(),
        InstallOperationError::Conflict
    );
}

#[test]
fn a_prepared_reinstall_ends_file_grants_without_nested_transactions() {
    use crate::durable_state::app_file_grants::{
        FileGrantCommand, FilePick, GrantedFile, PickState,
    };

    let f = Fixture::new();
    f.store
        .app_file_grant(FileGrantCommand::Create(FilePick {
            operation_id: "old-pick".into(),
            owner: "alice".into(),
            installation: "installed".into(),
            generation: 1,
            accept: vec![],
            multiple: false,
            state: PickState::Pending,
            expires_ms: 1_000,
            grants: vec![],
        }))
        .unwrap();
    let grant = f
        .store
        .app_file_grant(FileGrantCommand::Grant {
            owner: "alice".into(),
            operation_id: "old-pick".into(),
            files: vec![GrantedFile {
                name: "notes.md".into(),
                contents: b"private notes".to_vec(),
            }],
            now_ms: 2,
        })
        .unwrap()
        .unwrap()
        .grants[0]
        .clone();
    f.store
        .mutate_app_installation(
            "alice",
            crate::durable_state::apps::AppRegistryMutation::Uninstall {
                installation_id: "installed".into(),
                expected_generation: 1,
                now_ms: 5,
            },
        )
        .unwrap();
    let next = release("1.1.0", 0, false, &f.store);
    let digest = next.release_metadata().package_digest.clone();
    f.store
        .reserve_app_install("alice", "reinstall", update_input(2), &digest, budget())
        .unwrap();
    let prepared = f
        .store
        .complete_app_install_preparation("alice", "reinstall", next, budget())
        .unwrap();
    assert_eq!(prepared.token.installation_id, "installed");
    assert_eq!(prepared.phase, InstallPhase::AwaitingApproval);
    // Preparation's existing transaction must commit, without reviving a grant.
    assert_eq!(
        f.store.claim_app_file_grant(FileGrantCommand::Claim {
            owner: "alice".into(),
            installation: "installed".into(),
            grant_id: grant,
            now_ms: 6,
        }),
        Err("NOT_FOUND")
    );
    assert_eq!(
        f.store
            .app_file_pick("alice", "installed", "old-pick")
            .unwrap()
            .unwrap()
            .state,
        PickState::Expired
    );
    let contents: Option<Vec<u8>> = Connection::open(f.store.path())
        .unwrap()
        .query_row(
            "SELECT contents FROM app_file_grants WHERE operation_id='old-pick'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(contents.is_none());
}

/// Protocol 367: a deployment consent approves a copy's install only for an
/// exactly consented release whose capabilities the owner approved before.
mod deployment_consent {
    use super::*;
    use crate::durable_state::app_installation_operations::{
        ConsentStatus, ConsentedApp, DeploymentConsent, DeploymentInstall,
    };

    /// The owner approves the fixture App's capabilities interactively.
    fn approve_interactively(f: &Fixture) {
        f.stage("human");
        let challenge = Arc::new(f.arm("human", "human-prompt"));
        f.store
            .decide_app_install(challenge, true, budget())
            .unwrap();
    }

    fn consent(f: &Fixture, request: &str, interaction: &str) -> DeploymentConsent {
        let candidate = f.candidate();
        let release = candidate.release_metadata();
        f.store
            .begin_deployment_consent(
                "alice",
                DeploymentConsent {
                    request_id: request.into(),
                    interaction_id: interaction.into(),
                    session_id: "session".into(),
                    publication_id: "publication".into(),
                    deployment_id: "deployment".into(),
                    release_id: "release".into(),
                    package_digest: format!("sha256:{}", "d".repeat(64)),
                    apps: vec![ConsentedApp {
                        app_id: release.app_id.clone(),
                        publisher_id: release.publisher_id.clone(),
                        publisher_key_fingerprint: candidate.review_metadata()["keyFingerprint"]
                            .as_str()
                            .unwrap()
                            .into(),
                        package_digest: release.package_digest.clone(),
                        capabilities_digest: release.capabilities_digest.clone(),
                    }],
                    status: ConsentStatus::Pending,
                    expires_at_ms: crate::session::unix_epoch_ms() + 60_000,
                },
                budget(),
            )
            .unwrap()
    }

    /// Stages a deployment copy's install under `interaction` and arms it.
    fn arm_copy(f: &Fixture, request: &str, interaction: &str) -> InstallReviewDisposition {
        arm_copy_of(f, request, interaction, "deployment")
    }

    fn arm_copy_of(
        f: &Fixture,
        request: &str,
        interaction: &str,
        deployment: &str,
    ) -> InstallReviewDisposition {
        let candidate = f.candidate();
        f.store
            .reserve_app_install(
                "alice",
                request,
                InstallInput {
                    session_id: "session".into(),
                    upload_handle: String::new(),
                    update: None,
                    deployment: Some(DeploymentInstall {
                        consent: interaction.into(),
                        deployment_id: deployment.into(),
                    }),
                },
                &candidate.release_metadata().package_digest.clone(),
                budget(),
            )
            .unwrap();
        f.store
            .complete_app_install_preparation("alice", request, candidate, budget())
            .unwrap();
        f.store
            .arm_app_install_review("alice", request, "copy-prompt", budget())
            .unwrap()
    }

    fn prompts(disposition: InstallReviewDisposition) -> bool {
        matches!(disposition, InstallReviewDisposition::Prompt(_))
    }

    #[test]
    fn an_approved_consent_installs_the_copy_without_asking_and_tags_it() {
        let f = Fixture::new();
        approve_interactively(&f);
        let pending = consent(&f, "consent", "app_deploy_1");
        assert_eq!(pending.status, ConsentStatus::Pending);
        // A replay returns the same record; other facts under its id conflict.
        assert_eq!(consent(&f, "consent", "app_deploy_other"), pending);
        f.store
            .decide_deployment_consent("alice", "app_deploy_1", true, budget())
            .unwrap();
        assert!(matches!(
            arm_copy(&f, "copy", "app_deploy_1"),
            InstallReviewDisposition::Approved
        ));
        let operation = f.store.first_app_install_status("alice", "copy").unwrap();
        assert!(operation.approved);
        let journal = f
            .store
            .app_installation_journal("alice", &operation.token.installation_id)
            .unwrap();
        assert!(matches!(
            &journal.last().unwrap().decision,
            CapabilityDecision::Approved { approval }
                if approval.authority_ref == "kernel_deployment_consent:app_deploy_1"
        ));
        assert_eq!(
            f.store
                .app_installation_deployments("alice")
                .unwrap()
                .get(&operation.token.installation_id)
                .map(String::as_str),
            Some("deployment")
        );
        // The consent approves no other deployment's copy.
        assert!(prompts(arm_copy_of(
            &f,
            "copy-elsewhere",
            "app_deploy_1",
            "elsewhere"
        )));
        // Another owner cannot use Alice's consent.
        assert!(f
            .store
            .app_installation_deployments("bob")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_declined_consent_asks_the_owner() {
        let f = Fixture::new();
        approve_interactively(&f);
        consent(&f, "consent", "app_deploy_1");
        f.store
            .decide_deployment_consent("alice", "app_deploy_1", false, budget())
            .unwrap();
        assert_eq!(
            f.store
                .deployment_consent("alice", "consent")
                .unwrap()
                .unwrap()
                .status,
            ConsentStatus::Declined
        );
        // The answer is final.
        assert!(f
            .store
            .decide_deployment_consent("alice", "app_deploy_1", true, budget())
            .is_err());
        assert!(prompts(arm_copy(&f, "copy", "app_deploy_1")));
    }

    #[test]
    fn an_expired_consent_takes_no_answer_and_asks_the_owner() {
        let f = Fixture::new();
        approve_interactively(&f);
        consent(&f, "consent", "app_deploy_1");
        f.store
            .fixture_expire_deployment_consent("alice", "consent");
        assert_eq!(
            f.store
                .deployment_consent("alice", "consent")
                .unwrap()
                .unwrap()
                .status,
            ConsentStatus::Expired
        );
        assert!(f
            .store
            .decide_deployment_consent("alice", "app_deploy_1", true, budget())
            .is_err());
        assert!(prompts(arm_copy(&f, "copy", "app_deploy_1")));
    }

    #[test]
    fn an_approval_approves_installs_only_within_its_window() {
        let f = Fixture::new();
        approve_interactively(&f);
        consent(&f, "consent", "app_deploy_1");
        f.store
            .decide_deployment_consent("alice", "app_deploy_1", true, budget())
            .unwrap();
        f.store
            .fixture_expire_deployment_consent("alice", "consent");
        assert_eq!(
            f.store
                .deployment_consent("alice", "consent")
                .unwrap()
                .unwrap()
                .status,
            ConsentStatus::Expired
        );
        assert!(prompts(arm_copy(&f, "copy", "app_deploy_1")));
    }

    #[test]
    fn a_release_outside_the_consent_asks_the_owner() {
        let f = Fixture::new();
        approve_interactively(&f);
        let mut other = consent(&f, "consent", "app_deploy_1");
        other.request_id = "other".into();
        other.interaction_id = "app_deploy_2".into();
        other.apps[0].package_digest = format!("sha256:{}", "e".repeat(64));
        f.store
            .begin_deployment_consent("alice", other, budget())
            .unwrap();
        f.store
            .decide_deployment_consent("alice", "app_deploy_2", true, budget())
            .unwrap();
        assert!(prompts(arm_copy(&f, "copy", "app_deploy_2")));
        // Nor does a consent the owner has not answered approve anything.
        assert!(prompts(arm_copy(&f, "copy-2", "app_deploy_1")));
    }

    /// P1.20: the bind finds the approved consent by deployment, release and
    /// package; a deployment updates only an installation of its own copy.
    #[test]
    fn a_deployment_finds_its_consent_and_updates_only_its_own_copy() {
        let f = Fixture::new();
        let find = || {
            f.store
                .approved_deployment_consent(
                    "alice",
                    "deployment",
                    "release",
                    &format!("sha256:{}", "d".repeat(64)),
                )
                .unwrap()
        };
        consent(&f, "consent", "app_deploy_1");
        assert_eq!(find(), None, "not answered yet");
        f.store
            .decide_deployment_consent("alice", "app_deploy_1", true, budget())
            .unwrap();
        assert_eq!(find().as_deref(), Some("app_deploy_1"));
        assert_eq!(
            f.store
                .approved_deployment_consent("alice", "deployment", "release-2", "sha256:x")
                .unwrap(),
            None
        );
        let update = |request: &str| {
            f.store.reserve_app_install(
                "alice",
                request,
                InstallInput {
                    session_id: "session".into(),
                    upload_handle: String::new(),
                    update: Some(UpdateTarget {
                        installation_id: "installed".into(),
                        expected_generation: 1,
                    }),
                    deployment: Some(DeploymentInstall {
                        consent: "app_deploy_1".into(),
                        deployment_id: "deployment".into(),
                    }),
                },
                &f.candidate().release_metadata().package_digest.clone(),
                budget(),
            )
        };
        assert!(matches!(
            update("owner-install"),
            Err(InstallOperationError::Invalid)
        ));
        f.store
            .fixture_tag_app_installation("alice", "installed", "deployment");
        assert_eq!(
            update("copy-update").unwrap().phase,
            InstallPhase::Preparing
        );
    }

    #[test]
    fn capabilities_never_approved_interactively_ask_the_owner() {
        let f = Fixture::new();
        // The fixture installation was approved by a test policy, not a human.
        consent(&f, "consent", "app_deploy_1");
        f.store
            .decide_deployment_consent("alice", "app_deploy_1", true, budget())
            .unwrap();
        assert!(prompts(arm_copy(&f, "copy", "app_deploy_1")));
    }
}

#[test]
fn a_deployment_copy_prompt_names_its_deployment() {
    let f = Fixture::new();
    let copy = f.stage("copy-install");
    f.store
        .fixture_tag_app_installation("alice", &copy.token.installation_id, "deployment-7");
    assert_eq!(
        f.arm("copy-install", "nonce-copy").deployment_id(),
        Some("deployment-7")
    );
    f.stage("own-install");
    assert_eq!(f.arm("own-install", "nonce-own").deployment_id(), None);
}
