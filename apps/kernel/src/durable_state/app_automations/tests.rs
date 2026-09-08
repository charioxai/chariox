use super::*;
use chariox_app_package::{pack, verify, Limits, Manifest, TrustedPublisher, VerificationPolicy};
use chariox_app_runtime::{
    app_catalog::AppCatalog,
    installation::{
        CapabilityApproval, CapabilityDecision, InstallationRegistry, VerifiedInstallCandidate,
    },
    publisher_trust::{PublisherTrustRegistry, TrustDecision},
};
use ed25519_dalek::SigningKey;
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

struct Fixture {
    root: PathBuf,
    catalog: Arc<EventCatalog>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-app-automation-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        // Test-only enrolled publisher/verified installation. Production obtains
        // these through the existing installer/permission authority.
        let mut db = Connection::open(root.join("kernel.sqlite")).unwrap();
        InstallationRegistry::new(&mut db).initialize().unwrap();
        PublisherTrustRegistry::new(&mut db).initialize().unwrap();
        let key = SigningKey::from_bytes(&[74; 32]);
        let publisher = TrustedPublisher {
            publisher_id: "com.example".into(),
            key_id: "automation-key".into(),
            public_key: key.verifying_key(),
        };
        PublisherTrustRegistry::new(&mut db)
            .enroll(
                "local",
                &publisher,
                0,
                &TrustDecision {
                    decision_id: "enroll".into(),
                    authority_ref: "kernel-test".into(),
                },
                1,
            )
            .unwrap();
        let trust = PublisherTrustRegistry::new(&mut db)
            .trusted_publisher("local", "com.example", "automation-key")
            .unwrap();
        let manifest:Manifest=serde_json::from_value(json!({
            "schema":"chariox.app.v1","appId":"com.example.automation","version":"1.0.0",
            "publisher":{"id":"com.example","keyId":"automation-key","name":"Developer"},
            "sdkVersion":"0.3.0","appContractVersion":1,"minKernelProtocol":291,
            "resourcePolicy":"chariox.app.resources.v1","runtime":{"engine":"node","entry":"runtime/main.js"},
            "ui":{"entry":"ui/index.html"},"events":"schemas/events.json","capabilities":{}
        })).unwrap();
        let files=BTreeMap::from([
            ("runtime/main.js".into(),b"export default function register() {}".to_vec()),
            ("ui/index.html".into(),b"<!doctype html><title>Automation</title>".to_vec()),
            ("schemas/events.json".into(),serde_json::to_vec(&json!({"events":[{"name":"changed","schemaVersion":1,"direction":"outgoing","payloadSchema":{"type":"object","additionalProperties":false,"properties":{}}}]})).unwrap()),
        ]);
        let bytes = pack(&manifest, &files, &key, &Limits::default()).unwrap();
        let verified = verify(&bytes, &VerificationPolicy::new(291, vec![publisher])).unwrap();
        let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap();
        let mut registry = InstallationRegistry::new(&mut db);
        let token = registry
            .create_and_stage_verified("installed", "local", &candidate, 2)
            .unwrap()
            .token;
        registry
            .decide(
                &token,
                CapabilityDecision::Approved {
                    approval: CapabilityApproval {
                        decision_id: "approve".into(),
                        authority_ref: "kernel-test".into(),
                    },
                },
                3,
            )
            .unwrap();
        registry.quiesce(&token, 4).unwrap();
        registry.mark_prepared(&token, 5).unwrap();
        registry
            .commit_verified(&token, "local", &trust, 6)
            .unwrap();
        let binding = registry.staged_trust("local", &token).unwrap();
        let catalog = Arc::new(
            EventCatalog::compile(
                &verified,
                Arc::new(AppCatalog::compile(&verified, &binding, &trust).unwrap()),
            )
            .unwrap(),
        );
        Self { root, catalog }
    }
    fn open(&self) -> DurableKernelStateStore {
        DurableKernelStateStore::open_owned(self.root.join("kernel.sqlite")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn workflow() -> (SessionService, String, String) {
    use crate::{
        agent::{AgentInstance, GridPosition},
        session::CreateSessionRequest,
    };
    let mut sessions = SessionService::new(&crate::config::DaemonConfig::for_tests());
    let mut session = sessions
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    session.set_agents(vec![AgentInstance::new(
        "agent",
        "agent-ref",
        session.id(),
        None,
        "dev-stub",
        None,
        None,
        None,
        GridPosition::new(0, 0, 1, 1),
    )]);
    let id = session.id().to_owned();
    sessions.restore_session(session);
    let workflow = sessions
        .create_workflow(&id, Some("automation-test".into()))
        .unwrap();
    let node = sessions
        .add_workflow_node(&id, workflow.id(), "agent")
        .unwrap();
    let endpoint = sessions
        .create_workflow_endpoint(&id, workflow.id(), node.id(), Some("main".into()))
        .unwrap();
    let publication = sessions
        .create_workflow_publication(
            &id,
            workflow.id(),
            endpoint.id(),
            Some("default".into()),
            Some("events".into()),
            Some("event_based".into()),
            None,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            "local".into(),
        )
        .unwrap();
    (sessions, id, publication.id().into())
}
fn mutation(target: WorkflowAutomationTarget, expected: u64) -> AppAutomationMutation {
    AppAutomationMutation::Configure {
        automation_id: "automation".into(),
        expected_revision: expected,
        event_name: "changed".into(),
        target,
        scheduled: false,
    }
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::fixture(
        tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        || false,
    )
}
fn revision(result: AppAutomationOutcome) -> u64 {
    match result {
        AppAutomationOutcome::Configured(c) => c.revision,
        _ => panic!("expected config"),
    }
}

#[test]
fn normalized_target_fence_and_config_cas_use_existing_writer_and_survive_reopen() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let (sessions, session, publication) = workflow();
    let resolve = || {
        WorkflowAutomationTarget::resolve(&sessions, "local", &session, &publication, None).unwrap()
    };
    // In-memory target alone is insufficient until the existing workflow writer
    // has committed that exact normalized publication/workflow/queue state.
    assert!(matches!(
        store.mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            mutation(resolve(), 0),
            budget()
        ),
        Err(AppAutomationError::TargetChanged)
    ));
    store
        .persist_workflow_runtime_transition(&sessions.get_session(&session).unwrap(), "fixture")
        .unwrap();
    // The query-only connection is held: the mutation must use the writer's own.
    let reader = store.connection.lock().unwrap();
    let task_store = store.clone();
    let task_catalog = fixture.catalog.clone();
    let target = resolve();
    let (response, receive) = mpsc::channel();
    let task = std::thread::spawn(move || {
        let _ = response.send(task_store.mutate_app_automation(
            "local",
            task_catalog,
            mutation(target, 0),
            budget(),
        ));
    });
    let result = receive.recv_timeout(std::time::Duration::from_secs(5));
    drop(reader);
    task.join().unwrap();
    assert_eq!(revision(result.unwrap().unwrap()), 1);
    assert!(matches!(
        store.mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            mutation(resolve(), 0),
            budget()
        ),
        Err(AppAutomationError::Outbox(OutboxError::Conflict))
    ));
    assert!(matches!(
        store.mutate_app_automation(
            "other",
            fixture.catalog.clone(),
            mutation(resolve(), 1),
            budget()
        ),
        Err(AppAutomationError::NotOwner)
    ));
    assert_eq!(
        revision(
            store
                .mutate_app_automation(
                    "local",
                    fixture.catalog.clone(),
                    AppAutomationMutation::Deactivate {
                        automation_id: "automation".into(),
                        expected_revision: 1,
                        status: AutomationStatus::Paused
                    },
                    budget()
                )
                .unwrap()
        ),
        2
    );
    drop(store);
    let store = fixture.open();
    let AppAutomationOutcome::Listed(items) = store
        .mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            AppAutomationMutation::List,
            budget(),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].revision, 2);
    assert_eq!(items[0].status, AutomationStatus::Paused);
    assert_eq!(
        revision(
            store
                .mutate_app_automation(
                    "local",
                    fixture.catalog.clone(),
                    mutation(resolve(), 2),
                    budget()
                )
                .unwrap()
        ),
        3
    );
}

#[test]
fn stale_normalized_target_and_sql_failure_preserve_previous_configuration() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let (sessions, session, publication) = workflow();
    let resolve = || {
        WorkflowAutomationTarget::resolve(&sessions, "local", &session, &publication, None).unwrap()
    };
    store
        .persist_workflow_runtime_transition(&sessions.get_session(&session).unwrap(), "fixture")
        .unwrap();
    store
        .mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            mutation(resolve(), 0),
            budget(),
        )
        .unwrap();
    // A separate test-only connection simulates a target transaction racing the
    // queued request. Production holds the session guard and transition mutex.
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute("UPDATE durable_workflow_hot_entities SET payload_json='{}' WHERE entity_kind='publication'",[]).unwrap();
    assert!(matches!(
        store.mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            mutation(resolve(), 1),
            budget()
        ),
        Err(AppAutomationError::TargetChanged)
    ));
    store
        .persist_workflow_runtime_transition(
            &sessions.get_session(&session).unwrap(),
            "fixture-restored",
        )
        .unwrap();
    db.execute_batch("CREATE TRIGGER reject_config BEFORE UPDATE ON app_automations BEGIN SELECT RAISE(ABORT,'fixture config failure'); END;").unwrap();
    assert!(store
        .mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            mutation(resolve(), 1),
            budget()
        )
        .is_err());
    db.execute_batch("DROP TRIGGER reject_config;").unwrap();
    let AppAutomationOutcome::Listed(items) = store
        .mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            AppAutomationMutation::List,
            budget(),
        )
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(items[0].revision, 1);
    assert_eq!(
        revision(
            store
                .mutate_app_automation(
                    "local",
                    fixture.catalog.clone(),
                    mutation(resolve(), 1),
                    budget()
                )
                .unwrap()
        ),
        2
    );
}

#[test]
fn resolver_reuses_publication_and_endpoint_ownership_and_revocation_is_fenced() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let (mut sessions, session, publication) = workflow();
    assert!(matches!(
        WorkflowAutomationTarget::resolve(&sessions, "other", &session, &publication, None),
        Err(AppAutomationError::NotOwner)
    ));
    assert!(WorkflowAutomationTarget::resolve(
        &sessions,
        "local",
        &session,
        &publication,
        Some("missing")
    )
    .is_err());
    let publication_record = sessions
        .resolve_workflow_publication_ref(&session, &publication)
        .unwrap();
    sessions
        .set_workflow_endpoint_owner(
            &session,
            publication_record.workflow_id(),
            publication_record.endpoint_id(),
            "other".into(),
        )
        .unwrap();
    assert!(matches!(
        WorkflowAutomationTarget::resolve(&sessions, "local", &session, &publication, None),
        Err(AppAutomationError::NotOwner)
    ));
    sessions
        .set_workflow_endpoint_owner(
            &session,
            publication_record.workflow_id(),
            publication_record.endpoint_id(),
            "local".into(),
        )
        .unwrap();
    let target =
        WorkflowAutomationTarget::resolve(&sessions, "local", &session, &publication, None)
            .unwrap();
    store
        .persist_workflow_runtime_transition(&sessions.get_session(&session).unwrap(), "fixture")
        .unwrap();
    store
        .mutate_app_publisher(
            "local",
            crate::durable_state::app_publishers::AppPublisherMutation::Revoke {
                publisher_id: "com.example".into(),
                key_id: "automation-key".into(),
                expected_revision: 1,
                decision: TrustDecision {
                    decision_id: "revoke".into(),
                    authority_ref: "kernel-test".into(),
                },
                now_ms: 7,
            },
        )
        .unwrap();
    assert!(store
        .mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            mutation(target, 0),
            budget()
        )
        .is_err());
}

#[test]
fn cancellation_during_sqlite_wait_prevents_configuration_and_keeps_writer_usable() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;
    let fixture = Fixture::new();
    let store = fixture.open();
    let (sessions, session, publication) = workflow();
    let target =
        WorkflowAutomationTarget::resolve(&sessions, "local", &session, &publication, None)
            .unwrap();
    store
        .persist_workflow_runtime_transition(&sessions.get_session(&session).unwrap(), "fixture")
        .unwrap();
    let mut blocker = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    let held = blocker
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let (started, receive_started) = mpsc::channel();
    let checks = AtomicUsize::new(0);
    let budget = AppOperationBudget::fixture(
        tokio::time::Instant::now() + Duration::from_secs(30),
        move || {
            let result = flag.load(Ordering::Acquire);
            if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                let _ = started.send(());
            }
            result
        },
    );
    let task_store = store.clone();
    let catalog = fixture.catalog.clone();
    let task = std::thread::spawn(move || {
        task_store.mutate_app_automation("local", catalog, mutation(target, 0), budget)
    });
    let started = receive_started.recv_timeout(Duration::from_secs(5));
    cancelled.store(true, Ordering::Release);
    drop(held);
    let outcome = task.join().unwrap();
    assert!(started.is_ok());
    assert!(matches!(
        outcome,
        Err(AppAutomationError::Stopped(AppOperationStopped::Cancelled))
    ));
    let AppAutomationOutcome::Listed(items) = store
        .mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            AppAutomationMutation::List,
            self::budget(),
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(items.is_empty());
}

#[test]
fn target_snapshot_encoding_checks_encoded_bytes_before_growing_output() {
    const LIMIT: usize = 1024 * 1024;
    assert_eq!(encode(&"x".repeat(LIMIT - 2)).unwrap().len(), LIMIT);
    assert!(matches!(
        encode(&"x".repeat(LIMIT - 1)),
        Err(AppAutomationError::Outbox(OutboxError::Limit))
    ));
    assert!(matches!(
        encode(&"\n".repeat(LIMIT / 2)),
        Err(AppAutomationError::Outbox(OutboxError::Limit))
    ));
}
