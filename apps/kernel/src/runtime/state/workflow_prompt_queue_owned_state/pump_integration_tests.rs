//! One real AppControl pass through the writer and ordinary workflow prompt path.
//! The fixed libc worker supplies readiness only; no Node/provider/App view runs.
use super::*;
use crate::durable_state::{
    app_automations::{AppAutomationMutation, WorkflowAutomationTarget},
    app_state::{fixture_event_catalog, fixture_event_package, AppStateOperation, AppStateOutcome},
    DurableKernelStateStore,
};
use crate::runtime::{app_operation_budget::AppOperationBudget, app_worker::AppWorkerOwner};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    app_outbox::{
        occurrence_id, Artifact, EventCatalog, Invocation, Occurrence, Receipt, ReceiptState,
    },
    worker_peer::{Broker, BrokerFuture, BrokerRequest, PeerLimits},
    worker_process::test_fixture::{Fixture as NativeFixture, Mode},
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

struct NoEffects(Arc<AtomicUsize>);
impl Broker for NoEffects {
    fn handle(&self, _: BrokerRequest) -> BrokerFuture {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
fn receipt(store: &DurableKernelStateStore, catalog: &Arc<EventCatalog>, id: &str) -> Receipt {
    let AppStateOutcome::Receipt(receipt) = store
        .execute_app_state(
            "alice",
            catalog.clone(),
            AppStateOperation::Status {
                receipt_id: id.into(),
            },
            budget(),
        )
        .unwrap()
    else {
        panic!("receipt")
    };
    receipt
}

#[test]
fn one_pass_without_a_view_queues_the_original_event_and_recovers_its_prompt_receipt() {
    let (runtime, session, workflow, endpoint, _root) = runtime_with_idle_workflow();
    let store = runtime.owned.durable_state_store.clone();
    let catalog = fixture_event_catalog(&store);
    {
        let mut sessions = runtime.owned.session_store.write();
        sessions
            .set_workflow_endpoint_owner(&session, &workflow, &endpoint, "alice".into())
            .unwrap();
        let publication = sessions
            .create_workflow_publication(
                &session,
                &workflow,
                &endpoint,
                Some("default".into()),
                Some("app-pump".into()),
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
                "alice".into(),
            )
            .unwrap();
        store
            .persist_workflow_runtime_transition(
                &sessions.get_session(&session).unwrap(),
                "pump_fixture",
            )
            .unwrap();
        let target =
            WorkflowAutomationTarget::resolve(&sessions, "alice", &session, publication.id(), None)
                .unwrap();
        store
            .mutate_app_automation(
                "alice",
                catalog.clone(),
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
    }
    let original_time = crate::session::unix_epoch_ms();
    let occurrence = Occurrence {
        automation_id: "automation".into(),
        occurrence_id: occurrence_id("original-service-event", original_time).unwrap(),
        event_version: 1,
        occurred_at_ms: original_time,
        schedule_revision: None,
        payload: serde_json::json!({"text":"original service event"}),
        invocation: Invocation {
            prompt: "Review exactly this original service event".into(),
            artifacts: vec![Artifact {
                name: "metadata-only".into(),
                media_type: "text/plain".into(),
                reference: "file:///private/never-read-by-this-test".into(),
                size_bytes: None,
                digest: None,
            }],
        },
    };
    let AppStateOutcome::Receipt(accepted) = store
        .execute_app_state(
            "alice",
            catalog.clone(),
            AppStateOperation::Emit(occurrence.clone()),
            budget(),
        )
        .unwrap()
    else {
        panic!("accepted receipt")
    };
    assert_eq!(accepted.state, ReceiptState::Accepted);
    assert!(runtime
        .owned
        .session_store
        .get_session(&session)
        .unwrap()
        .workflow_runs()
        .is_empty());

    let async_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_io()
        .enable_time()
        .build()
        .unwrap();
    let native = NativeFixture::compile().unwrap();
    let (bytes, publisher) = fixture_event_package();
    let verified = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    let (process, observed) = native.spawn_blocking(Mode::Ready, &verified).unwrap();
    let unexpected = Arc::new(AtomicUsize::new(0));
    let (starting, mut controls) = AppWorkerOwner::start_blocking(
        process,
        &verified,
        catalog.clone(),
        Arc::new(NoEffects(unexpected.clone())),
        PeerLimits::default(),
        async_runtime.handle().clone(),
    )
    .unwrap();
    let mut registered = starting
        .await_registered_blocking(Duration::from_secs(2))
        .unwrap();
    let proof = store
        .confirm_app_activation(
            "alice",
            catalog.clone(),
            registered.take_activation_budget().unwrap(),
        )
        .unwrap();
    let (owner, worker) = registered.activate_blocking(proof).unwrap();
    async_runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if controls.recv().await.unwrap().name == "fixture.ready_ack" {
                    break;
                }
            }
        })
        .await
        .unwrap();
    });
    assert!(observed.ready_was_acknowledged());
    runtime
        .app_control()
        .publish_app_worker("alice", worker)
        .unwrap();
    assert_eq!(runtime.app_control().active_app_leases(None, 8).len(), 1);

    // No terminal attachment or browser view is created. The real pass owns
    // discovery, queue admission, Ready intent creation and normal prompt staging.
    assert!(runtime
        .owned
        .attachment_store
        .list_session_attachment_ids(&session)
        .is_empty());
    let dispatches = runtime.fixture_app_event_pass();
    assert!(dispatches.admitted_workflow_prompt);
    assert_eq!(dispatches.starting_provider_runs.len(), 1);
    let delivered = receipt(&store, &catalog, &accepted.receipt_id);
    assert_eq!(delivered.state, ReceiptState::Delivered);
    assert_eq!(delivered.occurrence_id, occurrence.occurrence_id);
    assert_eq!(
        delivered.queued_session_id.as_deref(),
        Some(session.as_str())
    );
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    assert_eq!(snapshot.workflow_runs().len(), 1);
    assert!(snapshot.workflow_queued_prompts().is_empty());
    let run = &snapshot.workflow_runs()[0];
    assert_eq!(
        run.invocation_prompt(),
        Some(occurrence.invocation.prompt.as_str())
    );
    assert_eq!(run.queue_item_id(), delivered.queued_prompt_id.as_deref());
    let intent = runtime
        .owned
        .workflow_entry_intent(&session, run.id())
        .unwrap()
        .unwrap();
    assert!(intent.submitted);
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&snapshot, &intent.agent_id);
    let prompts: Vec<_> = active.iter().chain(queued.iter()).collect();
    assert_eq!(prompts.len(), 1);
    let prompt = prompts[0];
    assert_eq!(prompt.workflow_run_id(), Some(run.id()));
    assert!(
        prompt.prompt().contains(&occurrence.invocation.prompt),
        "the actual admitted provider prompt must retain the original invocation body"
    );
    assert_eq!(
        prompt.durable_operation_id(),
        Some(intent.operation_id.as_str())
    );
    assert!(
        prompt.attachments().is_empty(),
        "untrusted artifact metadata must not become an attachment"
    );
    let prompt_id = prompt.id().to_owned();
    let run_id = run.id().to_owned();
    let operation_id = intent.operation_id;
    assert!(
        runtime.fixture_app_event_pass().is_empty(),
        "a repeated pass must not dispatch the entry again"
    );
    assert_eq!(unexpected.load(Ordering::SeqCst), 0);

    // Drop every process/channel/store owner before reopening the same DB. The
    // returned provider dispatch is intentionally never executed in this fixture.
    let path = store.path().to_path_buf();
    let durable_owner = runtime.owned.config_projection.snapshot().daemon_id;
    owner.shutdown_blocking();
    assert!(observed.was_reaped());
    assert!(observed.lease_was_dropped());
    drop(dispatches);
    drop(controls);
    drop(async_runtime);
    drop(runtime);
    drop(store);
    let reopened = DurableKernelStateStore::open_owned(path.clone()).unwrap();
    assert_eq!(
        receipt(&reopened, &catalog, &accepted.receipt_id).state,
        ReceiptState::Delivered
    );
    let retained = reopened
        .workflow_dispatch_intent(&durable_owner, &session, &run_id)
        .unwrap()
        .unwrap();
    assert!(retained.submitted);
    assert_eq!(retained.operation_id, operation_id);
    assert!(reopened
        .pending_workflow_dispatch_sessions(&durable_owner, None, 8)
        .unwrap()
        .is_empty());
    let db = rusqlite::Connection::open(path).unwrap();
    let (retained_prompt, count): (String, i64) = db.query_row(
        "SELECT prompt_id, (SELECT count(*) FROM durable_workflow_dispatch_intents) FROM durable_workflow_dispatch_intents WHERE run_id=?1",
        [&run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(retained_prompt, prompt_id);
    assert_eq!(count, 1);
    let json: String = db.query_row("SELECT payload_json FROM durable_state_events WHERE kind='session.prompt_state.updated' ORDER BY sequence DESC LIMIT 1", [], |row| row.get(0)).unwrap();
    let mut event: crate::durable_prompt_state::DurablePromptStateEventPayload =
        serde_json::from_str(&json).unwrap();
    event.restore_private_states();
    assert!(event
        .active_prompt
        .iter()
        .chain(event.queued_prompts.iter())
        .any(|prompt| prompt.id() == prompt_id
            && prompt.durable_operation_id() == Some(operation_id.as_str())
            && prompt.prompt().contains(&occurrence.invocation.prompt)));
}
