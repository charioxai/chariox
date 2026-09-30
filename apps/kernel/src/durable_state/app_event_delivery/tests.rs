use super::*;
#[path = "dispatch_tests.rs"]
mod dispatch_tests;
#[path = "fixtures.rs"]
mod fixtures;
#[path = "maintenance_tests.rs"]
mod maintenance_tests;
use crate::durable_state::{
    app_automations::{AppAutomationMutation, WorkflowAutomationTarget},
    app_state::{AppStateOperation, AppStateOutcome},
};
use crate::session::SessionService;
use chariox_app_runtime::app_outbox::{Artifact, Invocation, Occurrence};
use fixtures::{workflow, Fixture};

fn budget() -> AppOperationBudget {
    AppOperationBudget::fixture(
        tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        || false,
    )
}
fn setup(fixture: &Fixture) -> (DurableKernelStateStore, SessionService, String, Receipt) {
    let store = fixture.open();
    let (sessions, session, publication) = workflow();
    store
        .persist_workflow_runtime_transition(&sessions.get_session(&session).unwrap(), "fixture")
        .unwrap();
    let target =
        WorkflowAutomationTarget::resolve(&sessions, "local", &session, &publication, None)
            .unwrap();
    store
        .mutate_app_automation(
            "local",
            fixture.catalog.clone(),
            AppAutomationMutation::Configure {
                automation_id: "automation".into(),
                expected_revision: 0,
                event_name: "changed".into(),
                target,
                scheduled: false,
            },
            budget(),
        )
        .unwrap();
    let occurred_at = crate::session::unix_epoch_ms();
    let occurrence = Occurrence {
        automation_id: "automation".into(),
        occurrence_id: chariox_app_runtime::app_outbox::occurrence_id("first", occurred_at)
            .unwrap(),
        event_version: 1,
        occurred_at_ms: occurred_at,
        schedule_revision: None,
        payload: serde_json::json!({}),
        invocation: Invocation {
            prompt: "Review the change".into(),
            artifacts: vec![
                Artifact {
                    name: "host.txt".into(),
                    media_type: "text/plain".into(),
                    reference: "file:///private/host-secret".into(),
                    size_bytes: None,
                    digest: None,
                },
                Artifact {
                    name: "remote.txt".into(),
                    media_type: "text/plain".into(),
                    reference: "https://ambient.example/private".into(),
                    size_bytes: None,
                    digest: None,
                },
                Artifact {
                    name: "other.txt".into(),
                    media_type: "text/plain".into(),
                    reference: "artifact:other-installation".into(),
                    size_bytes: None,
                    digest: None,
                },
            ],
        },
    };
    let AppStateOutcome::Receipt(receipt) = store
        .execute_app_state(
            "local",
            fixture.catalog.clone(),
            AppStateOperation::Emit(occurrence),
            budget(),
        )
        .unwrap()
    else {
        panic!("expected receipt")
    };
    (store, sessions, session, receipt)
}
fn prepare(
    store: &DurableKernelStateStore,
    sessions: &mut SessionService,
    fixture: &Fixture,
    receipt: &Receipt,
) -> Arc<PreparedAppEvent> {
    let candidate = store
        .app_event_candidate("local", fixture.catalog.clone(), &receipt.receipt_id)
        .unwrap();
    Arc::new(PreparedAppEvent::prepare(sessions, candidate).unwrap())
}
fn queued_count(db: &Connection) -> i64 {
    db.query_row(
        "SELECT count(*) FROM durable_workflow_hot_entities WHERE entity_kind='queued_prompt'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn handoff_commits_workflow_queue_and_outbox_receipt_together_then_projects() {
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = setup(&fixture);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    assert!(sessions
        .get_session(&session)
        .unwrap()
        .workflow_queued_prompts()
        .is_empty());
    let queued = store
        .commit_app_event_queue(prepared.clone(), budget())
        .unwrap();
    assert_eq!(queued.state, ReceiptState::Queued);
    // The durable operation itself never publishes its tentative SessionService clone.
    assert!(sessions
        .get_session(&session)
        .unwrap()
        .workflow_queued_prompts()
        .is_empty());
    sessions.restore_session(prepared.session().clone());
    assert_eq!(
        sessions
            .get_session(&session)
            .unwrap()
            .workflow_queued_prompts()
            .len(),
        1
    );
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    assert_eq!(queued_count(&db), 1);
    assert_eq!(
        db.query_row(
            "SELECT queued_prompt_id FROM app_outbox WHERE receipt_id=?1",
            [&receipt.receipt_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        queued.queued_prompt_id.unwrap()
    );
    drop(db);
    drop(store);
    let reopened = fixture.open();
    let duplicate = reopened
        .app_event_candidate("local", fixture.catalog.clone(), &receipt.receipt_id)
        .unwrap();
    assert_eq!(duplicate.receipt().state, ReceiptState::Queued);
    assert!(PreparedAppEvent::prepare(&mut sessions, duplicate).is_err());
    assert_eq!(
        queued_count(&Connection::open(fixture.root.join("kernel.sqlite")).unwrap()),
        1
    );
}

#[test]
fn sql_failure_or_retarget_keeps_previous_session_and_receipt() {
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = setup(&fixture);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_app_queue BEFORE INSERT ON durable_state_events WHEN NEW.kind='workflow.runtime.updated' BEGIN SELECT RAISE(ABORT,'fixture queue failure'); END;").unwrap();
    assert!(store
        .commit_app_event_queue(prepared.clone(), budget())
        .is_err());
    assert_eq!(queued_count(&db), 0);
    assert_eq!(
        store
            .app_event_candidate("local", fixture.catalog.clone(), &receipt.receipt_id)
            .unwrap()
            .receipt(),
        &receipt
    );
    assert!(sessions
        .get_session(&session)
        .unwrap()
        .workflow_queued_prompts()
        .is_empty());
    db.execute_batch(
        "DROP TRIGGER fail_app_queue; UPDATE app_automations SET revision=revision+1;",
    )
    .unwrap();
    assert!(store.commit_app_event_queue(prepared, budget()).is_err());
    assert_eq!(queued_count(&db), 0);
}

#[test]
fn stale_hot_state_cannot_overwrite_another_queued_transition() {
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = setup(&fixture);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute("INSERT INTO durable_workflow_hot_entities(owner_id,session_id,entity_kind,entity_id,payload_json,updated_at_ms)
        SELECT owner_id,session_id,'queued_prompt','other','{}',updated_at_ms FROM durable_workflow_hot_entities WHERE session_id=?1 LIMIT 1",[&session]).unwrap();
    assert!(matches!(
        store.commit_app_event_queue(prepared, budget()),
        Err(AppEventDeliveryError::Conflict)
    ));
    assert_eq!(queued_count(&db), 1);
    assert_eq!(
        store
            .app_event_candidate("local", fixture.catalog.clone(), &receipt.receipt_id)
            .unwrap()
            .receipt(),
        &receipt
    );
}

#[test]
fn artifacts_remain_metadata_and_app_source_cannot_name_legacy_event_authority() {
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = setup(&fixture);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let queued = prepared
        .session()
        .workflow_queued_prompts()
        .front()
        .unwrap();
    let invocation = queued.publication_invocation().unwrap();
    assert_eq!(invocation.transport, "app_event");
    assert_eq!(invocation.hook_id.as_deref(), Some("automation"));
    // All existing legacy event context/action lookups require "event".
    assert_ne!(invocation.transport, "event");
    assert_eq!(
        invocation.artifacts[0]["reference"],
        "file:///private/host-secret"
    );
    assert_eq!(
        invocation.artifacts[1]["reference"],
        "https://ambient.example/private"
    );
    assert_eq!(
        invocation.artifacts[2]["reference"],
        "artifact:other-installation"
    );
    let run = sessions
        .invoke_queued_workflow_endpoint(&session, queued)
        .unwrap();
    assert_eq!(run.invocation_prompt(), Some("Review the change"));
    assert_eq!(run.publication_invocation().unwrap().artifacts.len(), 3);
    // The canonical workflow invocation creates text messages; artifact metadata
    // is not an inline attachment or host file injected into the prompt.
    for message in run.messages() {
        assert!(!message.handoff_payload().contains("host-secret"));
    }
}

#[test]
fn lost_commit_reply_is_reconciled_with_a_new_durable_write_without_a_second_queue_item() {
    let fixture = Fixture::new();
    let (store, mut sessions, _session, receipt) = setup(&fixture);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let committed = store
        .commit_app_event_queue(prepared.clone(), budget())
        .unwrap();
    let mut db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute_batch("PRAGMA synchronous=FULL;").unwrap();
    let before: i64 = db
        .query_row("SELECT count(*) FROM durable_state_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    let recovered = reconcile_commit(&mut db, &prepared, rusqlite::Error::InvalidQuery).unwrap();
    assert_eq!(recovered, committed);
    assert_eq!(queued_count(&db), 1);
    assert_eq!(
        db.query_row("SELECT count(*) FROM durable_state_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before + 1
    );
    // Failure of the recovery durability barrier cannot be acknowledged merely
    // because the first commit's queue item is still visible.
    db.execute_batch("CREATE TRIGGER fail_recovery BEFORE INSERT ON durable_state_events BEGIN SELECT RAISE(ABORT,'fixture recovery failure'); END;").unwrap();
    assert!(matches!(
        reconcile_commit(&mut db, &prepared, rusqlite::Error::InvalidQuery),
        Err(AppEventDeliveryError::CommitUnknown)
    ));
    assert_eq!(queued_count(&db), 1);
}

#[test]
fn admission_cancellation_and_revocation_do_not_publish_the_tentative_queue() {
    use chariox_app_runtime::publisher_trust::{PublisherTrustRegistry, TrustDecision};
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = setup(&fixture);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let cancelled = AppOperationBudget::fixture(
        tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        || true,
    );
    assert!(store
        .commit_app_event_queue(prepared.clone(), cancelled)
        .is_err());
    let mut db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    PublisherTrustRegistry::new(&mut db)
        .revoke(
            "local",
            "com.example",
            "automation-key",
            1,
            &TrustDecision {
                decision_id: "revoke".into(),
                authority_ref: "kernel-test".into(),
            },
            crate::session::unix_epoch_ms(),
        )
        .unwrap();
    assert!(store.commit_app_event_queue(prepared, budget()).is_err());
    assert_eq!(queued_count(&db), 0);
    assert!(sessions
        .get_session(&session)
        .unwrap()
        .workflow_queued_prompts()
        .is_empty());
}

#[test]
fn unknown_commit_stops_writer_and_rejects_already_queued_and_future_ordinary_writes() {
    use super::super::{DurableWriteOperation, DurableWriteRequest};
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = setup(&fixture);
    let mut prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    Arc::get_mut(&mut prepared).unwrap().fault_after_commit =
        Some(Arc::new(preparation::TestCommitFault {
            entered: entered_tx,
            release: std::sync::Mutex::new(release_rx),
        }));
    let task_store = store.clone();
    let task = std::thread::spawn(move || task_store.commit_app_event_queue(prepared, budget()));
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let (response, receiver) = mpsc::channel();
    let operation = || DurableWriteOperation::Event {
        event_id: "must-not-commit".into(),
        kind: "fixture.stale".into(),
        subject_id: None,
        timestamp_ms: crate::session::unix_epoch_ms(),
        payload_json: "{}".into(),
    };
    // This ordinary write is actually queued behind the uncertain handoff.
    store
        .writer
        .enqueue(DurableWriterRequest::Ordinary(DurableWriteRequest {
            operation: operation(),
            response,
        }))
        .unwrap();
    release_tx.send(()).unwrap();
    assert!(matches!(
        task.join().unwrap(),
        Err(AppEventDeliveryError::CommitUnknown)
    ));
    assert!(matches!(
        receiver.recv_timeout(std::time::Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
    assert!(store.writer.execute(operation()).is_err());
    assert!(store
        .execute_app_state(
            "local",
            fixture.catalog.clone(),
            AppStateOperation::Status {
                receipt_id: receipt.receipt_id.clone()
            },
            budget()
        )
        .is_err());
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    assert_eq!(queued_count(&db), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM durable_state_events WHERE event_id='must-not-commit'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(sessions
        .get_session(&session)
        .unwrap()
        .workflow_queued_prompts()
        .is_empty());
    drop(db);
    drop(store);
    let reopened = fixture.open();
    assert_eq!(
        reopened
            .app_event_candidate("local", fixture.catalog.clone(), &receipt.receipt_id)
            .unwrap()
            .receipt()
            .state,
        ReceiptState::Queued
    );
    // Restart reconstructs the authoritative queue from the ordinary hot store.
    let states = reopened
        .load_workflow_hot_states(sessions.get_session(&session).unwrap().host_daemon_id())
        .unwrap();
    assert_eq!(
        states
            .iter()
            .find(|(id, _)| id == &session)
            .unwrap()
            .1
            .workflow_queued_prompts
            .len(),
        1
    );
}

#[test]
fn an_app_with_undelivered_events_is_not_idle_until_they_are_queued() {
    let fixture = Fixture::new();
    let (store, mut sessions, _session, receipt) = setup(&fixture);
    let installation = fixture.catalog.installation_id().to_owned();
    assert!(store.has_deliverable_app_events("local", &installation));
    assert!(!store.has_deliverable_app_events("other", &installation));
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    store.commit_app_event_queue(prepared, budget()).unwrap();
    assert!(!store.has_deliverable_app_events("local", &installation));
}

#[test]
fn an_event_is_queued_while_its_target_publication_serves_a_deployment() {
    let fixture = Fixture::new();
    let (store, mut sessions, session, receipt) = setup(&fixture);
    // A deployed publication's runtime state changes in memory only.
    let publication = sessions
        .get_session(&session)
        .unwrap()
        .workflow_publications()[0]
        .id()
        .to_owned();
    sessions
        .register_workflow_publication_endpoint(
            &session,
            &publication,
            "running",
            "http://127.0.0.1:1/",
            serde_json::json!({"kind": "tunnel", "status": "running", "expires_at_ms": 1}),
        )
        .unwrap();
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let queued = store.commit_app_event_queue(prepared, budget()).unwrap();
    assert_eq!(queued.state, ReceiptState::Queued);
}

/// The typed handoff hit the bug: a full disk failed it with an unknown
/// outcome and fenced the writer. Now it is an ordinary failure, the writer
/// keeps serving, and the same handoff succeeds once space is back.
#[test]
fn a_full_disk_fails_the_handoff_without_stopping_the_writer() {
    use crate::durable_state::storage_full::{tests as full, DurableWriterCondition};
    let fixture = Fixture::new();
    let (mut store, mut sessions, _session, receipt) = setup(&fixture);
    full::limit_writer_pages(&mut store);
    full::fill(&store);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let failed = store
        .commit_app_event_queue(prepared, budget())
        .unwrap_err();
    assert!(
        !matches!(failed, AppEventDeliveryError::CommitUnknown),
        "{failed:?}"
    );
    store.require_writer_healthy().unwrap();
    full::wait_for(&store, DurableWriterCondition::StorageFull);

    full::free(&store);
    full::wait_for(&store, DurableWriterCondition::Writable);
    let prepared = prepare(&store, &mut sessions, &fixture, &receipt);
    let queued = store.commit_app_event_queue(prepared, budget()).unwrap();
    assert_eq!(queued.state, ReceiptState::Queued);
}

fn emit(
    store: &DurableKernelStateStore,
    fixture: &Fixture,
    name: &str,
) -> std::result::Result<AppStateOutcome, crate::durable_state::app_state::AppStateError> {
    let occurred_at = crate::session::unix_epoch_ms();
    store.execute_app_state(
        "local",
        fixture.catalog.clone(),
        AppStateOperation::Emit(Occurrence {
            automation_id: "automation".into(),
            occurrence_id: chariox_app_runtime::app_outbox::occurrence_id(name, occurred_at)
                .unwrap(),
            event_version: 1,
            occurred_at_ms: occurred_at,
            schedule_revision: None,
            payload: serde_json::json!({}),
            invocation: Invocation {
                prompt: "Review the change".into(),
                artifacts: Vec::new(),
            },
        }),
        budget(),
    )
}

/// Copies of an accepted event that wait for delivery, accepted at `at`.
fn fill_waiting(db: &Connection, receipt: &Receipt, prefix: &str, count: usize, at: u64) {
    db.execute(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
         INSERT INTO app_outbox(owner_id,installation_id,receipt_id,automation_id,event_version,occurrence_id,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,invocation_json,accepted_at_ms,expires_at_ms,state,revision,attempts,next_attempt_at_ms,occurred_at_ms,schedule_revision)
         SELECT owner_id,installation_id,?3||i,automation_id,event_version,?3||i,event_name,schema_digest,content_digest,automation_revision,accepted_generation,payload_json,invocation_json,?4,expires_at_ms,'accepted',1,0,next_attempt_at_ms,occurred_at_ms,schedule_revision
         FROM app_outbox,n WHERE receipt_id=?2",
        rusqlite::params![count as i64, receipt.receipt_id, prefix, at as i64],
    )
    .unwrap();
}

fn full_outbox_notices(store: &DurableKernelStateStore, installation: &str) -> usize {
    store
        .app_logs("local", installation, 0, 200)
        .unwrap()
        .iter()
        .filter(|entry| entry.fields["kernel"] == true && entry.fields["outbox_full"] == true)
        .count()
}

#[test]
fn a_full_outbox_refuses_new_events_as_backpressure_and_warns_the_owner_once_per_backlog() {
    use chariox_app_runtime::app_outbox::{OutboxError, MAX_PENDING};
    let fixture = Fixture::new();
    let (store, _sessions, _session, receipt) = setup(&fixture);
    let installation = fixture.catalog.installation_id().to_owned();
    let db = Connection::open(store.path()).unwrap();
    fill_waiting(
        &db,
        &receipt,
        "first-",
        MAX_PENDING - 1,
        receipt.accepted_at_ms,
    );
    let full = |result| {
        matches!(
            result,
            Err(crate::durable_state::app_state::AppStateError::Outbox(
                OutboxError::Full
            ))
        )
    };
    assert!(full(emit(&store, &fixture, "refused")));
    assert_eq!(full_outbox_notices(&store, &installation), 1);
    // A steady refusal loop on the same backlog warns once.
    for attempt in 0..5 {
        assert!(full(emit(&store, &fixture, &format!("again-{attempt}"))));
    }
    assert_eq!(full_outbox_notices(&store, &installation), 1);
    let warned_at = store
        .app_logs("local", &installation, 0, 200)
        .unwrap()
        .into_iter()
        .find(|entry| entry.fields["outbox_full"] == true)
        .unwrap()
        .at_ms;

    // Once that backlog has been delivered, the same event is accepted.
    db.execute(
        "UPDATE app_outbox SET state='delivered', revision=revision+1
         WHERE state IN ('accepted','retryable')",
        [],
    )
    .unwrap();
    assert!(emit(&store, &fixture, "refused").is_ok());

    // A new backlog, all accepted after the last warning, warns again.
    fill_waiting(&db, &receipt, "second-", MAX_PENDING, warned_at + 1);
    assert!(full(emit(&store, &fixture, "refused-again")));
    assert_eq!(full_outbox_notices(&store, &installation), 2);
}
