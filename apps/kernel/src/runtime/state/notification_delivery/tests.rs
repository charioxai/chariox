//! MP-08 / MP-10: real durable admission and production provider steer seam.
use super::*;
use crate::durable_state::workflow_notifications::{NotificationOperation, PreparedNotification};
use crate::local::*;
use crate::session::*;

#[cfg(unix)]
mod native_pty;

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

// MP-08 / MP-10: a real actor mailbox accepts; the unbound structured provider rejects.
#[tokio::test]
async fn notification_inject_structured_mailbox_acceptance_is_not_provider_ack() {
    for (app, accepted, legacy_reaper) in [
        (false, false, false),
        (true, false, true),
        (false, true, true),
        (true, true, false),
    ] {
        let (_root, runtime, session, id) = fixture(app, 1).await;
        let injection = runtime
            .owned
            .prepare_notification_injection(&session, &id)
            .unwrap()
            .unwrap();
        let request = crate::provider::LaunchProviderRequest::new(
            &session,
            if accepted { "dev-stub" } else { "codex" },
            if accepted { "slow-structured" } else { "codex" },
            "default",
            "model",
        )
        .with_agent_id(&injection.dispatch.agent_id);
        let mut run = crate::provider::RuntimeProviderRun::new(
            &injection.dispatch.provider_run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "notification-structured-failure".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some("test".into()),
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run);
        runtime.deliver_pending_notification_injections().await;
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        assert_eq!(
            snapshot
                .workflow_queued_prompts()
                .iter()
                .find(|q| q.id() == id)
                .unwrap()
                .status(),
            WorkflowQueuedPromptStatus::Running,
            "mailbox accepted, provider has not acknowledged"
        );
        let finished = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let jobs = runtime
                    .owned
                    .provider_store
                    .drain_finished_structured_prompt_submit_jobs();
                if let Some(job) = jobs.into_iter().next() {
                    break job;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(finished.result.is_ok(), accepted);
        // Actor completion alone is still not a durable injection settlement.
        runtime.deliver_pending_notification_injections().await;
        assert!(
            runtime
                .owned
                .provider_store
                .drain_finished_structured_prompt_submit_jobs()
                .is_empty(),
            "one mailbox command only"
        );
        runtime
            .owned
            .provider_store
            .push_finished_structured_prompt_submit_for_test(
                finished.session_id,
                finished.provider_run_id,
                finished.agent_id,
                finished.prompt_id,
                finished.result,
            );
        if legacy_reaper {
            runtime
                .with_app_side_effect(|app| app.reap_structured_prompt_jobs())
                .await;
        } else {
            runtime.owned.reap_structured_prompt_jobs();
        }
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        let item = snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .unwrap();
        assert_eq!(
            item.status(),
            if accepted {
                WorkflowQueuedPromptStatus::Completed
            } else {
                WorkflowQueuedPromptStatus::Queued
            }
        );
        assert!(!item.notification_injection_pending());
    }
}

// MP-08 / MP-10: bound production Codex RPC rejection is not an acceptance.
#[tokio::test]
async fn notification_inject_bound_codex_requires_steer_acknowledgement() {
    use crate::provider::{
        AgentEndpointMode, CodexRuntimeState, LaunchProviderRequest, ProviderLaunchResult,
        RuntimeProviderRun,
    };
    use serde_json::{json, Value};
    use std::{net::TcpListener, thread};
    use tokio_tungstenite::tungstenite::{accept, connect, Message};

    for (app, reject, lose_reply, restart) in [
        (false, Some("turn/steer"), false, false),
        (true, Some("turn/steer"), false, false),
        (false, Some("thread/inject_items"), false, false),
        (false, None, false, false),
        (false, None, true, false),
        (true, None, true, true),
        (false, None, false, true),
    ] {
        let (_root, runtime, session, id) = fixture(app, 1).await;
        let injection = runtime
            .owned
            .prepare_notification_injection(&session, &id)
            .unwrap()
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut socket = accept(stream).unwrap();
            let mut methods = Vec::new();
            loop {
                let request: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                let method = request["method"].as_str().unwrap();
                methods.push(method.to_owned());
                assert_eq!(request["params"]["threadId"], "notification-thread");
                assert!(matches!(method, "thread/inject_items" | "turn/steer"));
                if method == "turn/steer" {
                    assert_eq!(request["params"]["expectedTurnId"], "notification-turn");
                    assert!(request["params"]["input"]
                        .to_string()
                        .contains("NOTIFICATION_INJECT_PROOF"));
                }
                // Fixture applies the input, then loses the RPC response.
                if method == "turn/steer" && lose_reply {
                    break;
                }
                let response = if reject == Some(method) {
                    json!({"id":request["id"], "error":{"code":-32000,"message":"fixture rejects notification"}})
                } else {
                    json!({"id":request["id"], "result":{}})
                };
                socket
                    .send(Message::Text(response.to_string().into()))
                    .unwrap();
                if method == "turn/steer" || reject == Some(method) {
                    break;
                }
            }
            methods
        });
        let (socket, _) = connect(endpoint.as_str()).unwrap();
        let request = LaunchProviderRequest::new(&session, "codex", "codex", "default", "default")
            .with_agent_id(&injection.dispatch.agent_id);
        let mut run = RuntimeProviderRun::new(
            &injection.dispatch.provider_run_id,
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "bound-codex-notification-fixture".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: Some(endpoint.clone()),
            },
        );
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run);
        runtime
            .owned
            .provider_store
            .apply_runtime_binding(
                &injection.dispatch.provider_run_id,
                CodexRuntimeState::active_turn_binding_fixture(endpoint, socket),
            )
            .unwrap();
        runtime.deliver_pending_notification_injections().await;
        let finished = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(job) = runtime
                    .owned
                    .provider_store
                    .drain_finished_structured_prompt_submit_jobs()
                    .into_iter()
                    .next()
                {
                    break job;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let methods = server.join().unwrap();
        assert_eq!(
            methods.last().map(String::as_str),
            reject.or(Some("turn/steer"))
        );
        let acknowledged = finished.result.is_ok();
        assert_eq!(acknowledged, reject.is_none() && !lose_reply);
        let uncertain = lose_reply || restart;
        let runtime = if restart {
            end_original_notification_turn(&runtime, &session, &injection);
            let config = runtime.owned.config_projection.snapshot();
            runtime
                .with_app_side_effect(|app| app.save_durable_state_snapshot().unwrap())
                .await;
            drop(runtime);
            let restored =
                super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(
                    crate::app::DaemonApp::bootstrap(config).unwrap(),
                );
            restored.deliver_pending_notification_injections().await;
            restored
        } else {
            runtime
                .owned
                .provider_store
                .push_finished_structured_prompt_submit_for_test(
                    finished.session_id,
                    finished.provider_run_id,
                    finished.agent_id,
                    finished.prompt_id,
                    finished.result,
                );
            runtime.owned.reap_structured_prompt_jobs();
            runtime
        };
        if lose_reply && !restart {
            end_original_notification_turn(&runtime, &session, &injection);
        }
        runtime.deliver_pending_notification_injections().await;
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        let item = snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .unwrap();
        assert_eq!(
            item.status(),
            if uncertain {
                WorkflowQueuedPromptStatus::Running
            } else if reject.is_none() {
                WorkflowQueuedPromptStatus::Completed
            } else {
                WorkflowQueuedPromptStatus::Queued
            },
            "MP-08 / MP-10: only definite non-acceptance permits queue fallback"
        );
        assert_eq!(item.notification_injection_pending(), uncertain);
        if uncertain {
            let saved = &item.publication_invocation().unwrap().caller["notification_steer"];
            assert_eq!(saved["provider_run_id"], injection.dispatch.provider_run_id);
            assert_eq!(
                saved["prompt_id"],
                injection.dispatch.target_active_prompt_id.clone().unwrap()
            );
            assert!(runtime
                .owned
                .prepare_notification_injection(&session, &id)
                .unwrap()
                .is_none());
            // Bootstrap may prune the ended original run. No different run may appear.
            assert!(
                snapshot
                    .workflow_runs()
                    .iter()
                    .all(|run| run.id() == "active-0"),
                "never create another run"
            );
        }
        let db = rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        let receipt_state: String = db
            .query_row(
                "SELECT state FROM app_outbox WHERE queued_prompt_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            receipt_state,
            if !uncertain && acknowledged {
                "delivered"
            } else {
                "queued"
            }
        );
        drop(db);
        let hot = runtime
            .owned
            .durable_state_store
            .load_workflow_hot_states(snapshot.host_daemon_id())
            .unwrap();
        let (_, recovered) = hot.into_iter().find(|(s, _)| s == &session).unwrap();
        assert_eq!(
            recovered
                .workflow_queued_prompts
                .iter()
                .find(|q| q.id() == id)
                .unwrap()
                .status(),
            item.status()
        );
    }
}

// MP-08 / MP-10: an ended original turn is not proof of non-acceptance.
fn end_original_notification_turn(
    runtime: &KernelRuntimeState,
    session: &str,
    injection: &Injection,
) {
    let snapshot = runtime.owned.session_store.get_session(session).unwrap();
    let agent = &injection.dispatch.agent_id;
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_if_matches(
            &snapshot,
            agent,
            injection.dispatch.target_active_prompt_id.as_deref(),
        );
    let (active, queued) = runtime
        .owned
        .prompt_state_owner
        .state_parts(&snapshot, agent);
    assert!(active.is_none());
    runtime
        .owned
        .mirror_prompt_owner_agent_state(session, agent, active, queued)
        .unwrap();
    let mut ended = runtime.owned.session_store.get_session(session).unwrap();
    ended
        .workflow_run_mut("active-0")
        .unwrap()
        .set_status(WorkflowRunStatus::Completed);
    runtime
        .owned
        .durable_state_store
        .persist_workflow_runtime_transition(&ended, "local applied steer lost settlement")
        .unwrap();
    runtime.owned.session_store.write().restore_session(ended);
}
// MP-08 / MP-10: a lost ACK must hold the original identity after its turn ends.
#[tokio::test]
async fn notification_inject_remote_uncertainty_never_becomes_a_new_run() {
    let (_root, runtime, session, id) = fixture(false, 1).await;
    let injection = runtime
        .owned
        .prepare_notification_injection(&session, &id)
        .unwrap()
        .unwrap();
    let mut snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    snapshot
        .notification_prompt_mut(&id)
        .unwrap()
        .notification_invocation_mut()
        .unwrap()
        .caller["notification_steer"]["remote_uncertain"] = serde_json::json!({"worker_kernel_id":"worker","worker_machine_id":"machine","execution_lease_id":"lease","leased_agent_id":"leased","agent_id":injection.dispatch.agent_id,"target_home_prompt_id":injection.dispatch.target_active_prompt_id,"worker_provider_run_id":injection.dispatch.provider_run_id});
    runtime
        .owned
        .durable_state_store
        .persist_workflow_runtime_transition(&snapshot, "lost ACK fixture")
        .unwrap();
    runtime
        .owned
        .session_store
        .write()
        .restore_session(snapshot.clone());
    runtime
        .owned
        .prompt_state_owner
        .complete_active_prompt_if_matches(
            &snapshot,
            &injection.dispatch.agent_id,
            injection.dispatch.target_active_prompt_id.as_deref(),
        );
    // Production normalized hot-state recovery, including the uncertainty record.
    let owner = snapshot.host_daemon_id();
    let (_, hot) = runtime
        .owned
        .durable_state_store
        .load_workflow_hot_states(owner)
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
    let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
    let item = snapshot
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == id)
        .unwrap();
    assert_eq!(item.status(), WorkflowQueuedPromptStatus::Running);
    assert!(
        item.notification_injection_pending(),
        "cannot queue while the exact worker outcome is unknown"
    );
}

// MP-08 / MP-10 / MP-11: exact worker receipt is authoritative after the old turn ends.
#[tokio::test]
async fn notification_inject_remote_ack_loss_reconciles_after_real_bootstrap() {
    for accepted in [true, false] {
        let (_root, runtime, session, id) = fixture(false, 1).await;
        let snapshot = runtime.owned.session_store.get_session(&session).unwrap();
        let agent_id = snapshot.workflow_runs()[0].node_runs()[0]
            .agent_id()
            .to_owned();
        let binding = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker".into(),
            worker_machine_id: "machine".into(),
            leased_agent_id: "leased".into(),
            execution_lease_id: "lease".into(),
            active_worker_provider_run_id: Some("worker-run".into()),
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent_id, binding.clone())
            .unwrap();
        let injection = runtime
            .owned
            .prepare_notification_injection(&session, &id)
            .unwrap()
            .unwrap();
        runtime
            .owned
            .mark_notification_submit(&session, &injection, Some(&binding))
            .unwrap();
        // Worker has either applied it (ACK lost) or durably rejected it; home sees neither.
        let receipt = crate::transport::relay_peer::LeasedPromptReceipt {
            home_prompt_id: id.clone(),
            worker_provider_run_id: "worker-run".into(),
            target_home_prompt_id: injection.dispatch.target_active_prompt_id.clone(),
            execution_lease_id: Some("lease".into()),
            phase: if accepted {
                crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerAccepted
            } else {
                crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected
            },
        };
        runtime
            .owned
            .prompt_state_owner
            .complete_active_prompt_if_matches(
                &snapshot,
                &agent_id,
                injection.dispatch.target_active_prompt_id.as_deref(),
            );
        let ended = runtime.owned.session_store.get_session(&session).unwrap();
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&ended, &agent_id);
        assert!(active.is_none());
        runtime
            .owned
            .mirror_prompt_owner_agent_state(&session, &agent_id, active, queued)
            .unwrap();
        let mut ended = runtime.owned.session_store.get_session(&session).unwrap();
        ended
            .workflow_run_mut("active-0")
            .unwrap()
            .set_status(WorkflowRunStatus::Completed);
        runtime
            .owned
            .durable_state_store
            .persist_workflow_runtime_transition(&ended, "original notification target completed")
            .unwrap();
        runtime.owned.session_store.write().restore_session(ended);
        let config = runtime.owned.config_projection.snapshot();
        runtime
            .with_app_side_effect(|app| app.save_durable_state_snapshot().unwrap())
            .await;
        drop(runtime);
        let runtime =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(
                crate::app::DaemonApp::bootstrap(config).unwrap(),
            );
        let restored = runtime.owned.session_store.get_session(&session).unwrap();
        assert!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&restored, &agent_id)
                .is_none(),
            "the original turn is durably ended"
        );
        assert!(restored
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .unwrap()
            .notification_injection_pending());
        assert!(
            runtime
                .owned
                .prepare_notification_injection(&session, &id)
                .unwrap()
                .is_none(),
            "an ended original turn must not bypass uncertainty"
        );
        let mut conflicting = receipt.clone();
        conflicting.execution_lease_id = Some("other-lease".into());
        assert!(runtime
            .owned
            .settle_notification_remote_receipt(&session, &id, &conflicting)
            .is_err());
        let expected = receipt.clone();
        let target_prompt = injection
            .dispatch
            .target_active_prompt_id
            .as_deref()
            .unwrap();
        let queried = super::super::remote_prompt_worker_submission_runtime::query_remote_queued_steer_receipt_with_transport(
            &runtime, &agent_id, &id, "worker", "machine", "leased", target_prompt, "worker-run", "lease",
            |_, target, request| async move {
                assert_eq!(target.daemon_id.as_deref(), Some("worker"));
                let RelayPeerRequest::ReconcileLeasedPromptSteerReceipt { steer_id, target_home_prompt_id, worker_provider_run_id, execution_lease_id, leased_agent_id } = request else { panic!("exact reconciliation request required") };
                assert_eq!(steer_id, expected.home_prompt_id);
                assert_eq!(Some(target_home_prompt_id), expected.target_home_prompt_id);
                assert_eq!(worker_provider_run_id, expected.worker_provider_run_id);
                assert_eq!(Some(execution_lease_id), expected.execution_lease_id);
                assert_eq!(leased_agent_id, "leased");
                Ok(RelayPeerResponse::LeasedPromptReceiptQueried { receipt: Some(expected) })
            }).await.unwrap().unwrap();
        runtime
            .owned
            .settle_notification_remote_receipt(&session, &id, &queried)
            .unwrap();
        runtime.deliver_pending_notification_injections().await;
        let restored = runtime.owned.session_store.get_session(&session).unwrap();
        let item = restored
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .unwrap();
        assert_eq!(
            item.status(),
            if accepted {
                WorkflowQueuedPromptStatus::Completed
            } else {
                WorkflowQueuedPromptStatus::Queued
            }
        );
        assert!(!item.notification_injection_pending());
        assert!(
            runtime.owned.terminal_stream.input_records().is_empty(),
            "reconciliation never replays provider input"
        );
        assert_eq!(restored.workflow_queued_prompts().len(), 1);
        // Acceptance settles receipt/lineage; rejection retains the ordinary queue.
        let db = rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        let state: String = db
            .query_row(
                "SELECT state FROM app_outbox WHERE queued_prompt_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, if accepted { "delivered" } else { "queued" });
    }
}

// MP-08 / MP-10: a restart before actor dispatch must retain admitted work.
#[tokio::test]
async fn notification_inject_structured_pending_survives_restart_before_actor_write() {
    let (_root, runtime, session, id) = fixture(false, 1).await;
    let injection = runtime
        .owned
        .prepare_notification_injection(&session, &id)
        .unwrap()
        .unwrap();
    runtime
        .owned
        .mark_notification_submit(&session, &injection, None)
        .unwrap();
    let epoch = runtime.owned.provider_store.structured_submit_epoch();
    let config = runtime.owned.config_projection.snapshot();
    runtime
        .with_app_side_effect(|app| app.save_durable_state_snapshot().unwrap())
        .await;
    drop(runtime);
    let runtime = super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(
        crate::app::DaemonApp::bootstrap(config).unwrap(),
    );
    assert_ne!(
        epoch,
        runtime.owned.provider_store.structured_submit_epoch()
    );
    let restored = runtime.owned.session_store.get_session(&session).unwrap();
    let item = restored
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == id)
        .unwrap();
    assert_eq!(item.status(), WorkflowQueuedPromptStatus::Running);
    assert!(item.notification_injection_pending());
    // The old process may have written before it crashed: hold until known.
    runtime.deliver_pending_notification_injections().await;
    let restored = runtime.owned.session_store.get_session(&session).unwrap();
    let item = restored
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == id)
        .unwrap();
    assert_ne!(item.status(), WorkflowQueuedPromptStatus::Cancelled);
    assert!(matches!(item.status(), WorkflowQueuedPromptStatus::Running));
}
