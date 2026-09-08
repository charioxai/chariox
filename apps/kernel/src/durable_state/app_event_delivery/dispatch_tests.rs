//! Real SQLite writer/reopen checks. No provider processes are started.
use super::*;
use crate::durable_prompt_state::{
    DurablePromptStateEventPayload, DURABLE_PROMPT_STATE_EVENT_KIND,
};
use crate::session::{PreparedWorkflowQueueRun, PromptQueueItem, PromptStatus};

fn ready_fixture(
    fixture: &Fixture,
) -> (
    DurableKernelStateStore,
    SessionService,
    String,
    Receipt,
    Arc<PreparedWorkflowQueueRun>,
) {
    let (store, mut sessions, session, receipt) = setup(fixture);
    let queued = prepare(&store, &mut sessions, fixture, &receipt);
    let receipt = store
        .commit_app_event_queue(queued.clone(), budget())
        .unwrap();
    sessions.restore_session(queued.session().clone());
    sessions
        .ensure_primary_workflow_runtime_instance(&session)
        .unwrap()
        .unwrap();
    store
        .persist_workflow_runtime_transition(
            &sessions.get_session(&session).unwrap(),
            "fixture_instance",
        )
        .unwrap();
    let ready = Arc::new(
        sessions
            .prepare_durable_workflow_queue_run(&session)
            .unwrap(),
    );
    assert!(ready.next().is_some());
    (store, sessions, session, receipt, ready)
}
fn intent(
    store: &DurableKernelStateStore,
    ready: &PreparedWorkflowQueueRun,
) -> crate::durable_state::workflow_dispatch_intents::WorkflowDispatchIntent {
    store
        .workflow_dispatch_intent(
            ready.after().host_daemon_id(),
            ready.after().id(),
            ready.next().unwrap().1.id(),
        )
        .unwrap()
        .unwrap()
}
fn event(
    ready: &PreparedWorkflowQueueRun,
    intent: &crate::durable_state::workflow_dispatch_intents::WorkflowDispatchIntent,
) -> DurablePromptStateEventPayload {
    let mut session = ready.after().clone();
    let mut prompt = PromptQueueItem::new(
        "fixture-prompt",
        "workflow:entry",
        &intent.agent_id,
        "review",
        PromptStatus::Running,
    )
    .with_workflow_context(&intent.run_id, &intent.node_id)
    .with_durable_operation(&intent.operation_id, &intent.fingerprint);
    prompt.set_durable_delivery(
        crate::session::DurablePromptDeliveryPhase::Accepted,
        None,
        None,
    );
    session.mirror_agent_prompt_state(&intent.agent_id, Some(prompt), Default::default());
    DurablePromptStateEventPayload::capture(&session, &intent.agent_id)
}
fn append(
    store: &DurableKernelStateStore,
    event: DurablePromptStateEventPayload,
) -> Result<u64, crate::error::DaemonError> {
    store.append_event(
        DURABLE_PROMPT_STATE_EVENT_KIND,
        Some(event.session_id.clone()),
        serde_json::to_value(event).unwrap(),
    )
}

#[test]
fn durable_queue_run_failure_keeps_original_queue_receipt_and_hot_projection() {
    let fixture = Fixture::new();
    let (store, sessions, session, receipt, ready) = ready_fixture(&fixture);
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_entry BEFORE INSERT ON durable_workflow_dispatch_intents BEGIN SELECT RAISE(ABORT,'entry fault'); END;").unwrap();
    assert!(store.commit_workflow_queue_start(ready.clone()).is_err());
    assert_eq!(queued_count(&db), 1);
    assert_eq!(
        db.query_row("SELECT count(*) FROM durable_workflow_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        sessions
            .get_session(&session)
            .unwrap()
            .workflow_queued_prompts()
            .len(),
        1
    );
    assert!(sessions
        .get_session(&session)
        .unwrap()
        .workflow_runs()
        .is_empty());
    let AppStateOutcome::Receipt(current) = store
        .execute_app_state(
            "local",
            fixture.catalog.clone(),
            AppStateOperation::Status {
                receipt_id: receipt.receipt_id,
            },
            budget(),
        )
        .unwrap()
    else {
        panic!("receipt")
    };
    assert_eq!(current.state, ReceiptState::Queued);
    db.execute_batch("DROP TRIGGER reject_entry").unwrap();
    store.commit_workflow_queue_start(ready).unwrap();
}

#[test]
fn crash_after_ready_commit_recovers_same_run_and_original_queue_envelope() {
    let fixture = Fixture::new();
    let (store, sessions, session, receipt, ready) = ready_fixture(&fixture);
    store.commit_workflow_queue_start(ready.clone()).unwrap();
    // Simulate the caller disappearing before publishing its tentative session.
    assert_eq!(
        sessions
            .get_session(&session)
            .unwrap()
            .workflow_queued_prompts()
            .len(),
        1
    );
    let original = intent(&store, &ready);
    drop(store);
    let store = fixture.open();
    let pending = store
        .pending_workflow_dispatch_intent(ready.after().host_daemon_id(), &session)
        .unwrap()
        .unwrap();
    assert_eq!(pending.run_id, ready.next().unwrap().1.id());
    assert_eq!(pending.operation_id, original.operation_id);
    assert_eq!(
        pending.queued_prompt.id(),
        receipt.queued_prompt_id.as_deref().unwrap()
    );
    assert_eq!(
        pending.queued_prompt.prompt(),
        ready.next().unwrap().0.prompt()
    );
    let runs = store
        .load_active_workflow_runs(ready.after().host_daemon_id())
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].1.id(), pending.run_id);
    assert_eq!(
        store
            .pending_workflow_dispatch_sessions(ready.after().host_daemon_id(), None, 8)
            .unwrap(),
        vec![session]
    );
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    assert_eq!(queued_count(&db), 0);
    assert_eq!(
        db.query_row(
            "SELECT state FROM app_outbox WHERE receipt_id=?1",
            [receipt.receipt_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "delivered"
    );
}

#[test]
fn prompt_admission_and_entry_receipt_are_atomic_and_survive_completion_before_reply() {
    let fixture = Fixture::new();
    let (store, _, session, _, ready) = ready_fixture(&fixture);
    store.commit_workflow_queue_start(ready.clone()).unwrap();
    let original = intent(&store, &ready);
    let db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_submission BEFORE UPDATE OF submitted ON durable_workflow_dispatch_intents WHEN NEW.submitted=1 BEGIN SELECT RAISE(ABORT,'submission fault'); END;").unwrap();
    assert!(append(&store, event(&ready, &original)).is_err());
    assert!(!intent(&store, &ready).submitted);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM durable_state_events WHERE kind='session.prompt_state.updated'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER reject_submission").unwrap();
    append(&store, event(&ready, &original)).unwrap();
    assert!(intent(&store, &ready).submitted);
    let mut completed = event(&ready, &original);
    completed.active_prompt = None;
    completed.private_states.clear();
    append(&store, completed).unwrap();
    drop(db);
    drop(store);
    let store = fixture.open();
    assert!(intent(&store, &ready).submitted);
    assert!(store
        .pending_workflow_dispatch_intent(ready.after().host_daemon_id(), &session)
        .unwrap()
        .is_none());
    assert!(store
        .pending_workflow_dispatch_sessions(ready.after().host_daemon_id(), None, 8)
        .unwrap()
        .is_empty());
}

#[test]
fn unrelated_or_missing_private_context_cannot_acknowledge_entry_intent() {
    let fixture = Fixture::new();
    let (store, _, _, _, ready) = ready_fixture(&fixture);
    store.commit_workflow_queue_start(ready.clone()).unwrap();
    let original = intent(&store, &ready);
    let mut missing = event(&ready, &original);
    missing.private_states.clear();
    append(&store, missing).unwrap();
    assert!(!intent(&store, &ready).submitted);
    for (run, node, fingerprint) in [
        (
            "other",
            original.node_id.as_str(),
            original.fingerprint.as_str(),
        ),
        (
            original.run_id.as_str(),
            "other",
            original.fingerprint.as_str(),
        ),
        (original.run_id.as_str(), original.node_id.as_str(), "wrong"),
    ] {
        let mut wrong = original.clone();
        wrong.run_id = run.into();
        wrong.node_id = node.into();
        wrong.fingerprint = fingerprint.into();
        append(&store, event(&ready, &wrong)).unwrap();
        assert!(!intent(&store, &ready).submitted);
    }
    let mut external = event(&ready, &original);
    external.active_prompt = Some(
        PromptQueueItem::new(
            "caller-prompt",
            "attachment",
            &original.agent_id,
            "caller",
            PromptStatus::Running,
        )
        .with_durable_operation(&original.operation_id, &original.fingerprint),
    );
    external.private_states.clear();
    append(&store, external).unwrap();
    assert!(!intent(&store, &ready).submitted);
    append(&store, event(&ready, &original)).unwrap();
    assert!(intent(&store, &ready).submitted);
}

#[test]
fn submission_receipt_retention_follows_actual_run_deletion() {
    let fixture = Fixture::new();
    let (store, _, _, _, ready) = ready_fixture(&fixture);
    store.commit_workflow_queue_start(ready.clone()).unwrap();
    let original = intent(&store, &ready);
    append(&store, event(&ready, &original)).unwrap();
    // Ordinary upserts must keep the tombstone even though prompts have settled.
    store
        .persist_workflow_runtime_transition(ready.after(), "ordinary_update")
        .unwrap();
    assert!(intent(&store, &ready).submitted);
    let mut db = Connection::open(fixture.root.join("kernel.sqlite")).unwrap();
    let tx = db.transaction().unwrap();
    tx.execute(
        "DELETE FROM durable_workflow_runs WHERE run_id=?1",
        [&original.run_id],
    )
    .unwrap();
    tx.rollback().unwrap();
    assert!(intent(&store, &ready).submitted);
    db.execute(
        "DELETE FROM durable_workflow_runs WHERE run_id=?1",
        [&original.run_id],
    )
    .unwrap();
    assert!(store
        .workflow_dispatch_intent(
            ready.after().host_daemon_id(),
            ready.after().id(),
            &original.run_id
        )
        .unwrap()
        .is_none());
}
