//! MP-08 / MP-10: real durable admission and production provider steer seam.
use super::*;
use crate::durable_state::workflow_notifications::{NotificationOperation, PreparedNotification};
use crate::local::*;
use crate::session::*;

async fn fixture(
    app_event: bool,
    active_runs: usize,
) -> (
    crate::test_support::TestWorktree,
    KernelRuntimeState,
    String,
    String,
) {
    let (root, runtime, session_id, agent_id, _, _, _) =
        super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt().await;
    let owner = if app_event { "alice" } else { "local" };
    let (workflow, endpoint, publication) = {
        let mut sessions = runtime.owned.session_store.write();
        let mut session = sessions.get_session(&session_id).unwrap();
        session.set_agents(vec![runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()]);
        sessions.restore_session(session);
        let w = sessions
            .create_workflow(&session_id, Some("notification-target".into()))
            .unwrap();
        let n = sessions
            .add_workflow_node(&session_id, w.id(), &agent_id)
            .unwrap();
        let e = sessions
            .create_workflow_endpoint(&session_id, w.id(), n.id(), None)
            .unwrap();
        sessions
            .set_workflow_endpoint_owner(&session_id, w.id(), e.id(), owner.into())
            .unwrap();
        let p = sessions
            .create_workflow_publication(
                &session_id,
                w.id(),
                e.id(),
                Some("default".into()),
                Some("notifications".into()),
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
                owner.into(),
            )
            .unwrap();
        let mut session = sessions.get_session(&session_id).unwrap();
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap();
        for index in 0..active_runs {
            let mut run = WorkflowRun::new(
                format!("active-{index}"),
                w.id(),
                e.id(),
                n.id(),
                None,
                None,
                vec![WorkflowNodeRun::new(
                    format!("entry-{index}"),
                    n.id(),
                    &agent_id,
                    0,
                    WorkflowNodeRunStatus::Running,
                )],
                vec![],
            );
            run.set_status(WorkflowRunStatus::Running);
            session.create_workflow_run(run);
        }
        if active_runs > 0 {
            session.mirror_agent_prompt_state(
                &agent_id,
                Some(active.with_workflow_context("active-0", "entry-0")),
                Default::default(),
            );
            runtime
                .owned
                .prompt_state_owner
                .restore_session_state(&session);
        }
        sessions.restore_session(session.clone());
        runtime
            .owned
            .durable_state_store
            .persist_workflow_runtime_transition(&session, "inject fixture")
            .unwrap();
        (w.id().to_owned(), e.id().to_owned(), p.id().to_owned())
    };
    if app_event {
        use crate::durable_state::{
            app_automations::{AppAutomationMutation, WorkflowAutomationTarget},
            app_state::{fixture_event_catalog, AppStateOperation, AppStateOutcome},
        };
        use chariox_app_runtime::app_outbox::{occurrence_id, Invocation, Occurrence};
        let budget =
            || crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false);
        let catalog = fixture_event_catalog(&runtime.owned.durable_state_store);
        let target = WorkflowAutomationTarget::resolve(
            &runtime.owned.session_store.read(),
            owner,
            &session_id,
            &publication,
            None,
        )
        .unwrap();
        runtime
            .owned
            .durable_state_store
            .mutate_app_automation(
                owner,
                catalog.clone(),
                AppAutomationMutation::Configure {
                    automation_id: "inject-app".into(),
                    expected_revision: 0,
                    event_name: "changed".into(),
                    target,
                    scheduled: false,
                    delivery_mode: NotificationDeliveryMode::Inject,
                },
                budget(),
            )
            .unwrap();
        let now = unix_epoch_ms();
        let AppStateOutcome::Receipt(receipt) = runtime
            .owned
            .durable_state_store
            .execute_app_state(
                owner,
                catalog.clone(),
                AppStateOperation::Emit(Occurrence {
                    automation_id: "inject-app".into(),
                    occurrence_id: occurrence_id("inject-app", now).unwrap(),
                    event_version: 1,
                    occurred_at_ms: now,
                    schedule_revision: None,
                    payload: serde_json::json!({"text":"opaque"}),
                    invocation: Invocation {
                        prompt: "NOTIFICATION_INJECT_PROOF".into(),
                        artifacts: vec![],
                    },
                }),
                budget(),
            )
            .unwrap()
        else {
            panic!("receipt")
        };
        runtime
            .owned
            .queue_app_event(owner, catalog, &receipt.receipt_id, budget())
            .unwrap();
    } else {
        let snapshot = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let queue = snapshot
            .workflow_prompt_queues()
            .iter()
            .find(|q| q.workflow_id() == workflow)
            .unwrap()
            .id()
            .to_owned();
        let sub = WorkflowNotificationSubscription {
            subscription_id: "inject-workflow".into(),
            source_id: "remote-source".into(),
            owner_user_id: owner.into(),
            source_kernel_id: "remote-kernel".into(),
            target_kernel_id: snapshot.host_daemon_id().into(),
            target_kind: WorkflowNotificationTargetKind::WorkflowEndpoint,
            session_id: session_id.clone(),
            workflow_id: workflow,
            publication_id: publication,
            endpoint_id: endpoint,
            queue_id: queue,
            ttl_days: 7,
            source_available: true,
            events: WorkflowNotificationEvents::Both,
            filters: serde_json::Value::Null,
            delivery_mode: NotificationDeliveryMode::Inject,
        };
        runtime
            .owned
            .durable_state_store
            .notify(NotificationOperation::RemoteAttach {
                subscription: sub.clone(),
            })
            .unwrap();
        let env = WorkflowNotificationEnvelope {
            source_id: sub.source_id.clone(),
            occurrence_id: "completion".into(),
            status: WorkflowNotificationStatus::Success,
            output: Some(WorkflowOutputPayload::new(
                "NOTIFICATION_INJECT_PROOF",
                vec![],
            )),
            subject: Some("opaque:subject".into()),
            fields: serde_json::json!({"opaque":{"value":42}}),
            ancestry: vec!["source-identity".into()],
            deadline_ms: unix_epoch_ms() + 60_000,
        };
        runtime
            .owned
            .durable_state_store
            .notify(NotificationOperation::Accept {
                subscription: sub.clone(),
                envelope: env.clone(),
            })
            .unwrap();
        let mut sessions = runtime.owned.session_store.write();
        let prepared = PreparedNotification::prepare(&mut sessions, sub, env).unwrap();
        let after = prepared.after.clone();
        runtime
            .owned
            .durable_state_store
            .notify(NotificationOperation::Queue(Box::new(prepared)))
            .unwrap();
        sessions.restore_session(after);
    }
    let queued_id = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap()
        .workflow_queued_prompts()
        .back()
        .unwrap()
        .id()
        .to_owned();
    (root, runtime, session_id, queued_id)
}

#[tokio::test]
async fn notification_inject_is_observed_in_active_turn_for_both_source_kinds() {
    for app in [false, true] {
        let (_root, runtime, session, id) = fixture(app, 1).await;
        runtime.deliver_pending_notification_injections().await;
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        assert_eq!(
            snapshot
                .workflow_queued_prompts()
                .iter()
                .find(|q| q.id() == id)
                .unwrap()
                .status(),
            WorkflowQueuedPromptStatus::Completed
        );
        assert!(runtime
            .owned
            .terminal_stream
            .input_records()
            .iter()
            .any(|r| String::from_utf8_lossy(&r.bytes).contains("NOTIFICATION_INJECT_PROOF")));
        runtime.deliver_pending_notification_injections().await;
        assert_eq!(
            runtime
                .owned
                .terminal_stream
                .input_records()
                .iter()
                .filter(|r| String::from_utf8_lossy(&r.bytes).contains("NOTIFICATION_INJECT_PROOF"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn notification_inject_idle_and_multiple_runs_fall_back_to_durable_queue() {
    for count in [0, 2] {
        let (_root, runtime, session, id) = fixture(false, count).await;
        runtime.deliver_pending_notification_injections().await;
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        let item = snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .unwrap();
        assert_eq!(item.status(), WorkflowQueuedPromptStatus::Queued);
        assert!(!item.notification_injection_pending());
        assert!(runtime.owned.terminal_stream.input_records().is_empty());
        let db = rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        let expected = if count == 2 {
            "notification_inject_multiple_runs_queued"
        } else {
            "notification_inject_idle_or_ended_queued"
        };
        let reasons: i64 = db.query_row("SELECT count(*) FROM durable_state_events WHERE json_extract(payload_json,'$.reason')=?1", [expected], |r| r.get(0)).unwrap();
        assert_eq!(reasons, 1);
    }
}

#[tokio::test]
async fn notification_inject_turn_ends_after_admission_falls_back_without_loss() {
    let (_root, runtime, session, id) = fixture(false, 1).await;
    let injection = runtime
        .owned
        .prepare_notification_injection(&session, &id)
        .unwrap()
        .unwrap();
    let current = runtime.owned.session_store.get_session(&session).unwrap();
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_if_matches(
            &current,
            &injection.dispatch.agent_id,
            injection.dispatch.target_active_prompt_id.as_deref(),
        );
    runtime.deliver_pending_notification_injections().await;
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let item = snapshot
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == id)
        .unwrap();
    assert_eq!(item.status(), WorkflowQueuedPromptStatus::Queued);
    assert!(!item.notification_injection_pending());
}

#[tokio::test]
async fn notification_inject_restart_between_admission_and_steer_recovers_once() {
    let (_root, runtime, session, id) = fixture(false, 1).await;
    runtime
        .owned
        .prepare_notification_injection(&session, &id)
        .unwrap()
        .unwrap();
    // Restore only committed normalized state into the projection, as startup
    // does, then use the production injection pump and provider seam.
    let owner = runtime
        .owned
        .session_store
        .get_session(&session)
        .unwrap()
        .host_daemon_id()
        .to_owned();
    let (_, hot) = runtime
        .owned
        .durable_state_store
        .load_workflow_hot_states(&owner)
        .unwrap()
        .into_iter()
        .find(|(s, _)| s == &session)
        .unwrap();
    let mut restored = runtime.owned.session_store.get_session(&session).unwrap();
    for item in hot.workflow_queued_prompts {
        *restored.notification_prompt_mut(item.id()).unwrap() = item.clone();
    }
    runtime
        .owned
        .session_store
        .write()
        .restore_session(restored);
    runtime.deliver_pending_notification_injections().await;
    runtime.deliver_pending_notification_injections().await;
    assert_eq!(
        runtime
            .owned
            .terminal_stream
            .input_records()
            .iter()
            .filter(|r| String::from_utf8_lossy(&r.bytes).contains("NOTIFICATION_INJECT_PROOF"))
            .count(),
        1
    );
    let db = rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
    let lineage: String = db.query_row("SELECT json_extract(invocation_json,'$.injected_run_id') FROM app_outbox WHERE queued_prompt_id=?1", [&id], |r| r.get(0)).unwrap();
    assert_eq!(lineage, "active-0");
}

#[tokio::test]
async fn notification_inject_paused_queue_holds_then_delivers_after_resume() {
    let (_root, runtime, session, id) = fixture(false, 1).await;
    let mut snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let queue = snapshot
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == id)
        .unwrap()
        .queue_id()
        .to_owned();
    let workflow = snapshot
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == id)
        .unwrap()
        .workflow_id()
        .to_owned();
    snapshot
        .workflow_prompt_queue_mut(&workflow, &queue)
        .unwrap()
        .set_enabled(false);
    runtime
        .owned
        .durable_state_store
        .persist_workflow_runtime_transition(&snapshot, "pause inject fixture")
        .unwrap();
    runtime
        .owned
        .session_store
        .write()
        .restore_session(snapshot);
    runtime.deliver_pending_notification_injections().await;
    let mut snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    assert!(snapshot
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == id)
        .unwrap()
        .notification_injection_pending());
    assert!(runtime.owned.terminal_stream.input_records().is_empty());
    snapshot
        .workflow_prompt_queue_mut(&workflow, &queue)
        .unwrap()
        .set_enabled(true);
    runtime
        .owned
        .durable_state_store
        .persist_workflow_runtime_transition(&snapshot, "resume inject fixture")
        .unwrap();
    runtime
        .owned
        .session_store
        .write()
        .restore_session(snapshot);
    runtime.deliver_pending_notification_injections().await;
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    assert_eq!(
        snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .unwrap()
            .status(),
        WorkflowQueuedPromptStatus::Completed
    );
}

#[tokio::test]
async fn notification_inject_expiry_retires_hot_item_after_receipt_sweep() {
    let (_root, runtime, session, id) = fixture(false, 1).await;
    let mut snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    snapshot
        .notification_prompt_mut(&id)
        .unwrap()
        .notification_invocation_mut()
        .unwrap()
        .input["deadline_ms"] = serde_json::json!(0);
    runtime
        .owned
        .durable_state_store
        .persist_workflow_runtime_transition(&snapshot, "expired inject fixture")
        .unwrap();
    runtime
        .owned
        .session_store
        .write()
        .restore_session(snapshot);
    let db = rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
    db.execute(
        "UPDATE app_outbox SET state='expired' WHERE queued_prompt_id=?1",
        [&id],
    )
    .unwrap();
    drop(db);
    runtime.deliver_pending_notification_injections().await;
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    assert_eq!(
        snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .unwrap()
            .status(),
        WorkflowQueuedPromptStatus::Cancelled
    );
    assert!(runtime.owned.terminal_stream.input_records().is_empty());
}
