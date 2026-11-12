//! MP-08 / MP-09 / MP-10 / MP-11 G02: Stop uses the parked task ledger.
use super::goal_tests::blocked_task;
use super::tests::{fixture_with_options, Fixture};
use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::durable_state::agent_lifecycle::{
    AgentTaskExecution, AgentWake, ExecutionState, Operation, Outcome,
};

fn waiting_task(f: &Fixture, prompt: &str) -> AgentTaskExecution {
    let room = &f.request.session_id;
    let agent = f.request.target_agent_id.as_deref().unwrap();
    f.state
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: room.clone(),
            agent: agent.into(),
            prompt: prompt.into(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    park_task(f, prompt)
}

/// Parks a begun task on a timer and settles its turn: the state a task is in while it waits.
pub(super) fn park_task(f: &Fixture, prompt: &str) -> AgentTaskExecution {
    let store = &f.state.owned.durable_state_store;
    let now = crate::session::unix_epoch_ms();
    let room = &f.request.session_id;
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let wake_id = format!("timer-{prompt}");
    let registration_id = format!("completion-{wake_id}");
    let wake: AgentWake = serde_json::from_value(serde_json::json!({
        "id": wake_id, "task_id": prompt, "room_id": room, "agent_id": agent,
        "registration_id": registration_id, "kind": "timer", "label": "wait for review",
        "state": "scheduled", "created_at_ms": now, "verified_at_ms": null,
        "next_due_ms": now + 3_600_000, "interval_ms": null, "command": [],
        "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
        "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null,
        "last_sequence": null, "last_delivery": null, "last_delivered_at_ms": null,
        "last_acknowledged_at_ms": null, "alerted_sequence": null
    }))
    .unwrap();
    store
        .agent_lifecycle(Operation::CreateWake {
            task: prompt.into(),
            prompt: prompt.into(),
            wake,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Yield {
            task: prompt.into(),
            prompt: prompt.into(),
            registrations: vec![registration_id],
            cursor: 0,
            deadline: Some(now + 3_600_000),
            reason: "Waiting for review".into(),
            now,
        })
        .unwrap();
    let Outcome::Settled { task, .. } = store
        .agent_lifecycle(Operation::Settle {
            room: room.clone(),
            agent: agent.into(),
            prompt: prompt.into(),
            run: f.run.id().into(),
            has_answer: true,
            cancelled: false,
            now,
        })
        .unwrap()
    else {
        panic!("settled task")
    };
    assert_eq!(task.state, ExecutionState::Waiting);
    task
}

#[tokio::test]
async fn targetless_owner_stop_cancels_waiting_task_and_timer_without_active_prompt() {
    let f = fixture_with_options(None, true);
    let original = waiting_task(&f, "waiting-turn");
    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        target_agent_id: None,
        expected_prompt_id: None,
    });
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "stop-waiting",
        None,
        None,
        &request,
    );
    let response = f.router.dispatch(command, request).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!("expected task cancellation")
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(task.state, ExecutionState::Cancelled);
    assert_eq!(task.reason, "Stopped by you");
    assert!(task.wait.is_none());
    assert!(task.obligations.iter().all(|o| o.status == "cancelled"));
    let wake = f
        .state
        .owned
        .durable_state_store
        .agent_wake("timer-waiting-turn")
        .unwrap()
        .unwrap();
    assert_eq!(wake.state, "cancelled");
    f.state.sweep_agent_lifecycle().await.unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    assert!(f
        .state
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &original.agent_id)
        .is_none());
}

/// An orchestrator that wakes on events leaves one parked task per wait. Stopping it is one
/// request that ends them all, not an error that asks for the intended task.
#[tokio::test]
async fn stop_of_an_idle_agent_with_several_parked_tasks_cancels_them_all() {
    let f = fixture_with_options(None, true);
    let first = waiting_task(&f, "waiting-turn-1");
    let second = waiting_task(&f, "waiting-turn-2");
    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        target_agent_id: None,
        expected_prompt_id: None,
    });
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "stop-several-waiting",
        None,
        None,
        &request,
    );
    let response = f.router.dispatch(command, request).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!("expected task cancellation")
    };
    assert_eq!(task.state, ExecutionState::Cancelled);
    let store = &f.state.owned.durable_state_store;
    for original in [first, second] {
        let stopped = store.agent_task(&original.task_id).unwrap().unwrap();
        assert_eq!(stopped.state, ExecutionState::Cancelled, "{}", original.task_id);
        let wake = store
            .agent_wake(&format!("timer-{}", original.task_id))
            .unwrap()
            .unwrap();
        assert_eq!(wake.state, "cancelled", "{}", original.task_id);
    }
}

#[tokio::test]
async fn stop_cancels_parked_task_through_prompt_route() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, false);
    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        target_agent_id: f.request.target_agent_id.clone(),
        expected_prompt_id: None,
    });
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "stop-parked",
        None,
        None,
        &request,
    );
    let result = f.router.dispatch(command, request).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = result else {
        panic!("expected task cancellation")
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(task.state, ExecutionState::Cancelled);
    let store = &f.state.owned.durable_state_store;
    assert!(store
        .agent_delivery_front(&task.room_id, &task.agent_id)
        .unwrap()
        .is_none());
    assert!(!store
        .agent_pending_inbox_recipients()
        .unwrap()
        .contains(&(task.room_id.clone(), task.agent_id.clone())));
    let Outcome::Event(message) = store
        .agent_lifecycle(Operation::Occur(
            crate::durable_state::agent_lifecycle::occurrence(
                &task.room_id,
                &task.agent_id,
                "peer",
                "after-stop",
                "message",
                serde_json::json!({"message":"New work for a later turn"}),
            ),
        ))
        .unwrap()
    else {
        panic!("message occurrence")
    };
    assert_eq!(
        store
            .agent_delivery_front(&task.room_id, &task.agent_id)
            .unwrap()
            .unwrap()
            .sequence,
        message.sequence
    );
    let Outcome::Event(attempt) = store
        .agent_lifecycle(Operation::Attempt {
            room: task.room_id.clone(),
            agent: task.agent_id.clone(),
            sequence: message.sequence,
            prompt: "later-delivery".into(),
            target: None,
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
            work: None,
            through: 0,
        })
        .unwrap()
    else {
        panic!("later delivery admission")
    };
    assert_eq!(attempt.state, "submitting");
    assert_eq!(
        store
            .agent_delivery_front(&task.room_id, &task.agent_id)
            .unwrap()
            .unwrap()
            .sequence,
        message.sequence
    );
}

#[tokio::test]
async fn prompt_specific_stop_does_not_cancel_parked_task() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, false);
    for target_agent_id in [None, f.request.target_agent_id.clone()] {
        let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: f.request.session_id.clone(),
            attachment_id: f.request.attachment_id.clone(),
            target_agent_id: target_agent_id.clone(),
            expected_prompt_id: Some("stale-prompt".into()),
        });
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "stop-stale",
            None,
            None,
            &request,
        );
        assert!(f.router.dispatch(command, request).await.is_err());
    }
    assert_eq!(
        f.state
            .owned
            .durable_state_store
            .agent_tasks(Some(&f.request.session_id), None)
            .unwrap()[0]
            .revision,
        original.revision
    );
}

#[tokio::test]
async fn owner_stop_keeps_existing_attachment_membership_authority() {
    let f = fixture_with_options(None, true);
    let original = blocked_task(&f, false);
    let attachment = {
        let mut app = f.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::for_user(
                &f.request.session_id,
                "member-terminal",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
                "room-member",
            ))
            .unwrap()
    };
    let request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: attachment.id().into(),
        target_agent_id: f.request.target_agent_id.clone(),
        expected_prompt_id: None,
    });
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let agent = f.request.target_agent_id.as_deref().unwrap();
    f.state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt_with_queue_policy(
            &session,
            PromptQueueItem::new(
                "active-parity",
                attachment.id(),
                agent,
                "Current ordinary work",
                PromptStatus::Queued,
            ),
            false,
            false,
        )
        .unwrap();
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owner-stop-active-member-terminal",
        None,
        None,
        &request,
    );
    assert!(matches!(
        f.router.dispatch(command, request.clone()).await.unwrap(),
        LocalDaemonResponse::PromptCancelled { .. }
    ));
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, agent);
    // The normal prompt route authorizes the caller against the agent owner;
    // membership of the attachment is checked separately in that same room.
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owner-stop-member-terminal",
        None,
        None,
        &request,
    );
    let response = f.router.dispatch(command, request).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!()
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(task.state, ExecutionState::Cancelled);
}

async fn cancel_request(
    f: &Fixture,
    value: serde_json::Value,
) -> Result<LocalDaemonResponse, DaemonError> {
    let request: LocalDaemonRequest = serde_json::from_value(value).unwrap();
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "owner-cancel",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    f.router.dispatch(command, request).await
}

#[tokio::test]
async fn exact_owner_task_cancellation_preserves_other_waits_and_active_prompt() {
    let f = fixture_with_options(None, true);
    let original = waiting_task(&f, "first-wait");
    let other = waiting_task(&f, "second-wait");
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    f.state
        .owned
        .prompt_state_owner
        .sync_external_active_prompt(
            &session,
            &original.agent_id,
            Some(PromptQueueItem::new(
                "unrelated-turn",
                &f.request.attachment_id,
                &original.agent_id,
                "Keep working",
                PromptStatus::Running,
            )),
        );
    let response = cancel_request(
        &f,
        serde_json::json!({"CancelAgentTask":{
            "session_id": f.request.session_id, "attachment_id": f.request.attachment_id,
            "task_id": original.task_id, "revision": original.revision,
        }}),
    )
    .await
    .unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!("expected task cancellation")
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(task.state, ExecutionState::Cancelled);
    assert_eq!(task.reason, "Stopped by you");
    let store = &f.state.owned.durable_state_store;
    let tasks = store
        .agent_tasks(Some(&original.room_id), Some(&original.agent_id))
        .unwrap();
    assert_eq!(
        tasks.iter().find(|t| t.task_id == other.task_id).unwrap(),
        &other
    );
    assert_eq!(
        store.agent_wake("timer-first-wait").unwrap().unwrap().state,
        "cancelled"
    );
    assert_eq!(
        store
            .agent_wake("timer-second-wait")
            .unwrap()
            .unwrap()
            .state,
        "scheduled"
    );
    assert_eq!(
        f.state
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &original.agent_id)
            .unwrap()
            .id(),
        "unrelated-turn"
    );
    let notices = store
        .agent_inbox(&original.room_id, &original.agent_id, 0)
        .unwrap();
    let notice = notices.iter().find(|e| e.kind == "task_cancelled").unwrap();
    assert_eq!(notice.state, "pending");
    assert_eq!(notice.payload["cancelled_task_id"], original.task_id);
    assert_eq!(notice.payload["reason"], "Stopped by you");
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: original.room_id.clone(),
            agent: original.agent_id.clone(),
            prompt: "unrelated-turn".into(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let hint = f
        .state
        .owned
        .agent_task_context_hint(&original.room_id, &original.agent_id, "unrelated-turn")
        .unwrap()
        .unwrap();
    assert!(
        hint.contains("chariox-owner-cancellations") && hint.contains(&original.task_id),
        "{hint}"
    );
    f.router.dispatch_authenticated_runtime_tool_call("sudo-fixture-bearer", "chariox.events.ack", serde_json::json!({
        "task_id":"unrelated-turn", "origin_prompt_id":"unrelated-turn", "sequence":notice.sequence, "handled":true,
    })).await.unwrap();
    let hint = f
        .state
        .owned
        .agent_task_context_hint(&original.room_id, &original.agent_id, "unrelated-turn")
        .unwrap()
        .unwrap();
    assert!(
        !hint.contains("chariox-owner-cancellations"),
        "handled cancellations must not repeat: {hint}"
    );
}

#[tokio::test]
async fn owner_timer_cancellation_preserves_waiting_task_and_delivers_source_receipt_once() {
    let f = fixture_with_options(None, true);
    let original = waiting_task(&f, "timer-wait");
    let request = serde_json::json!({"CancelAgentWake":{
        "session_id": f.request.session_id, "attachment_id": f.request.attachment_id,
        "wake_id": "timer-timer-wait",
    }});
    let response = cancel_request(&f, request.clone()).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!("expected timer cancellation")
    };
    assert_eq!(task.task_id, original.task_id);
    assert_eq!(task.state, ExecutionState::Waiting);
    assert_eq!(task.reason, original.reason);
    assert!(task.obligations.iter().all(|o| o.status == "cancelled"));
    let store = &f.state.owned.durable_state_store;
    assert_eq!(
        store.agent_wake("timer-timer-wait").unwrap().unwrap().state,
        "cancelled"
    );
    let receipt = || {
        store
            .agent_inbox(&original.room_id, &original.agent_id, 0)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "source_lost")
            .collect::<Vec<_>>()
    };
    let receipts = receipt();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].payload["public_answer"]["cancelled"], true);
    assert_eq!(receipts[0].payload["task_id"], original.task_id);
    assert_eq!(receipts[0].state, "pending");
    cancel_request(&f, request).await.unwrap();
    assert_eq!(
        receipt(),
        receipts,
        "repeated cancellation must not repeat the wake receipt"
    );
    waiting_task(&f, "already-fired");
    store
        .agent_lifecycle(Operation::FireWakes {
            now: crate::session::unix_epoch_ms() + 3_600_001,
        })
        .unwrap();
    let error = cancel_request(
        &f,
        serde_json::json!({"CancelAgentWake":{
            "session_id": f.request.session_id, "attachment_id": f.request.attachment_id,
            "wake_id": "timer-already-fired",
        }}),
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("already ended as fired"),
        "{error}"
    );
    assert_eq!(
        store
            .agent_wake("timer-already-fired")
            .unwrap()
            .unwrap()
            .state,
        "fired"
    );
}

#[tokio::test]
async fn exact_owner_task_cancellation_rejects_stale_revision_and_foreign_owner() {
    let f = fixture_with_options(None, true);
    let original = waiting_task(&f, "protected-wait");
    assert!(cancel_request(
        &f,
        serde_json::json!({"CancelAgentTask":{
            "session_id": f.request.session_id, "attachment_id": f.request.attachment_id,
            "task_id": original.task_id, "revision": original.revision - 1,
        }})
    )
    .await
    .is_err());
    let request = LocalDaemonRequest::CancelAgentTask(crate::local::CancelAgentTaskRequest {
        session_id: f.request.session_id.clone(),
        attachment_id: f.request.attachment_id.clone(),
        task_id: original.task_id.clone(),
        revision: original.revision,
    });
    let mut command = crate::runtime::command::KernelCommand::from_local_request(
        "foreign-owner-cancel",
        None,
        None,
        &request,
    );
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::Terminal);
    command.caller.user_id = Some("another-owner".into());
    assert!(f.router.dispatch(command, request).await.is_err());
    let store = &f.state.owned.durable_state_store;
    assert_eq!(
        store
            .agent_tasks(Some(&original.room_id), Some(&original.agent_id))
            .unwrap(),
        vec![original]
    );
    assert_eq!(
        store
            .agent_wake("timer-protected-wait")
            .unwrap()
            .unwrap()
            .state,
        "scheduled"
    );
}

#[tokio::test]
async fn owner_cancellation_rejects_foreign_room_and_attachment() {
    let f = fixture_with_options(None, true);
    let original = waiting_task(&f, "room-bound-wait");
    let (other_room, other_attachment) = {
        let mut app = f.app.lock().await;
        let (room, _) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "other-workspace",
                "other-worktree",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                room.id(),
                "other-terminal",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        (room, attachment)
    };
    for (room, attachment) in [
        (original.room_id.as_str(), other_attachment.id()),
        (other_room.id(), other_attachment.id()),
    ] {
        assert!(cancel_request(
            &f,
            serde_json::json!({"CancelAgentWake":{
                "session_id": room, "attachment_id": attachment, "wake_id": "timer-room-bound-wait",
            }})
        )
        .await
        .is_err());
        assert!(cancel_request(
            &f,
            serde_json::json!({"CancelAgentTask":{
                "session_id": room, "attachment_id": attachment, "task_id": original.task_id,
                "revision": original.revision,
            }})
        )
        .await
        .is_err());
    }
    assert_eq!(
        f.state
            .owned
            .durable_state_store
            .agent_tasks(Some(&original.room_id), Some(&original.agent_id))
            .unwrap(),
        vec![original]
    );
    assert_eq!(
        f.state
            .owned
            .durable_state_store
            .agent_wake("timer-room-bound-wait")
            .unwrap()
            .unwrap()
            .state,
        "scheduled"
    );
}

#[tokio::test]
async fn owner_timer_cancellation_after_unsubscribe_reaches_next_turn_once() {
    let f = fixture_with_options(None, true);
    let room = &f.request.session_id;
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let store = &f.state.owned.durable_state_store;
    let session = f.state.owned.session_store.get_session(room).unwrap();
    let original_prompt = "unsubscribed-timer-turn";
    f.state
        .owned
        .prompt_state_owner
        .sync_external_active_prompt(
            &session,
            agent,
            Some(PromptQueueItem::new(
                original_prompt,
                &f.request.attachment_id,
                agent,
                "Start a timer and withdraw its subscription",
                PromptStatus::Running,
            )),
        );
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: room.clone(),
            agent: agent.into(),
            prompt: original_prompt.into(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let timer = f
        .router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox.events.timer",
            serde_json::json!({
                "task_id":original_prompt, "origin_prompt_id":original_prompt,
                "label":"Unsubscribed review timer", "delay_ms":3_600_000,
            }),
        )
        .await
        .unwrap()
        .payload;
    let wake_id = timer["wake_id"].as_str().unwrap();
    f.router
        .dispatch_authenticated_runtime_tool_call(
            "sudo-fixture-bearer",
            "chariox.events.unsubscribe",
            serde_json::json!({
                "task_id":original_prompt, "origin_prompt_id":original_prompt,
                "registration_id":timer["registration_id"],
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        store.agent_wake(wake_id).unwrap().unwrap().state,
        "scheduled"
    );
    assert!(!store.agent_registrations(original_prompt).unwrap()[0].live);
    let request = serde_json::json!({"CancelAgentWake":{
        "session_id":room, "attachment_id":f.request.attachment_id, "wake_id":wake_id,
    }});
    let response = cancel_request(&f, request.clone()).await.unwrap();
    let LocalDaemonResponse::AgentTaskUpdated { task, .. } = response else {
        panic!("expected timer cancellation")
    };
    assert_eq!(task.task_id, original_prompt);
    assert_eq!(
        task.state,
        ExecutionState::Working,
        "timer cancellation preserves the task"
    );
    assert_eq!(
        store.agent_wake(wake_id).unwrap().unwrap().state,
        "cancelled"
    );
    store
        .agent_lifecycle(Operation::Settle {
            room: room.clone(),
            agent: agent.into(),
            prompt: original_prompt.into(),
            run: f.run.id().into(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let next_prompt = "after-unsubscribed-timer-cancellation";
    assert_eq!(
        f.state
            .owned
            .prompt_state_owner
            .complete_active_prompt_if_matches(&session, agent, Some(original_prompt))
            .unwrap()
            .id(),
        original_prompt
    );
    f.state
        .owned
        .prompt_state_owner
        .submit_prepared_prompt_with_queue_policy(
            &session,
            PromptQueueItem::new(
                next_prompt,
                &f.request.attachment_id,
                agent,
                "Read the timer cancellation outcome",
                PromptStatus::Queued,
            ),
            false,
            false,
        )
        .unwrap();
    assert_eq!(
        f.state
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent)
            .unwrap()
            .id(),
        next_prompt
    );
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: room.clone(),
            agent: agent.into(),
            prompt: next_prompt.into(),
            run: Some(f.run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    for _ in 0..2 {
        let inbox = f
            .router
            .dispatch_authenticated_runtime_tool_call(
                "sudo-fixture-bearer",
                "chariox.events.inbox",
                serde_json::json!({
                    "task_id":next_prompt, "origin_prompt_id":next_prompt, "after":0,
                }),
            )
            .await
            .unwrap()
            .payload;
        let receipts = inbox["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["source"]["id"] == wake_id && event["kind"] == "source_lost")
            .collect::<Vec<_>>();
        assert_eq!(
            receipts.len(),
            1,
            "next turn must receive exactly one cancellation receipt"
        );
        assert_eq!(receipts[0]["payload"]["task_id"], original_prompt);
        assert_eq!(receipts[0]["payload"]["public_answer"]["cancelled"], true);
        cancel_request(&f, request.clone()).await.unwrap();
    }
}
