//! MP-08 / MP-09 / MP-10 / MP-11 AM-ASK: authenticated non-blocking questions.
use super::*;

async fn ask(f: &Fixture, extra: serde_json::Value) -> serde_json::Value {
    let token = f
        .runtime
        .owned
        .provider_store
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    let mut args = serde_json::json!({"wait":false,"message":"Choose the next step", "choices":[
        {"id":"continue","label":"Continue","reply":"Exact reply: continue."}
    ]});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    tokio::time::timeout(
        Duration::from_secs(2),
        f.runtime.dispatch_authenticated_runtime_tool_call(
            &token,
            crate::transport::runtime_tools::REQUEST_POPUP_TOOL,
            args,
        ),
    )
    .await
    .expect("AM-ASK-01: wait:false must return before the question is answered")
    .unwrap()
    .payload
}

#[tokio::test]
async fn async_popup_returns_while_pending_and_uses_the_existing_wake_registry() {
    let f = fixture("async-popup", true).await;
    let task = f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert_eq!(session.active_interactions().len(), 1);
    let wake = f
        .store()
        .agent_wake(id)
        .unwrap()
        .expect("question is a wake source");
    assert_eq!(wake.task_id, task.task_id);
    assert_eq!(result["registration_id"], wake.registration_id);
    assert_eq!(wake.kind, "interaction");
    let token = f
        .runtime
        .owned
        .provider_store
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    let inbox = f
        .runtime
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.events.inbox",
            serde_json::json!({
                "task_id": task.task_id, "origin_prompt_id": f.prompt_id
            }),
        )
        .await
        .unwrap()
        .payload;
    assert!(inbox.is_array(), "the same turn can keep using tools");
}

fn answers(f: &Fixture, id: &str) -> Vec<serde_json::Value> {
    f.store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.source_id == id)
        .map(|e| e.payload["public_answer"].clone())
        .collect()
}

#[tokio::test]
async fn async_popup_cancellation_intent_does_not_fire_its_deadline() {
    let f = fixture("async-popup-cancelling-deadline", true).await;
    let task = f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({"timeout_sec":1})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let wake = f.store().agent_wake(id).unwrap().unwrap();
    // Model a crash after cancellation intent commits but before its driver
    // finishes removing the card. A deadline cannot answer a cancelled source.
    f.store()
        .agent_lifecycle(Operation::CancelWake {
            id: id.into(),
            task: task.task_id.clone(),
            prompt: Some(task.prompt_id.clone()),
            pending: true,
        })
        .unwrap();
    f.runtime
        .tick_interaction_wakes(wake.next_due_ms.unwrap())
        .unwrap();
    assert!(answers(&f, id).is_empty());
    assert_eq!(
        f.store().agent_wake(id).unwrap().unwrap().state,
        "cancelling"
    );
    f.runtime
        .cancel_wake_source(&wake, &task.task_id, Some(&task.prompt_id))
        .unwrap();
    assert_eq!(
        f.store().agent_wake(id).unwrap().unwrap().state,
        "cancelled"
    );
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
}

async fn answer_through_router(
    f: &Fixture,
    id: &str,
    user: &str,
    client: &str,
) -> Result<crate::local::LocalDaemonResponse, DaemonError> {
    use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(f.app.clone(), 8);
    let mut caller = KernelCaller::for_source(&KernelCommandSource::LocalCli)
        .with_connection_class(crate::local::KernelConnectionClass::Terminal);
    caller.caller_id = client.into();
    caller.client_id = Some(client.into());
    caller.user_id = Some(user.into());
    let request = crate::local::LocalDaemonRequest::RespondToInteraction(
        crate::local::RespondToInteractionRequest {
            session_id: f.room.clone(),
            interaction_id: id.into(),
            choice_id: "continue".into(),
            custom_reply: None,
            passkey: None,
            passkey_remember_minutes: None,
        },
    );
    let command = KernelCommand::from_local_request_with_caller(
        "async-answer",
        KernelCommandSource::LocalCli,
        caller,
        None,
        None,
        &request,
    );
    router.dispatch(command, request).await
}

#[tokio::test]
async fn async_popup_returns_first_authorized_answer_wins_with_exact_reply_and_client() {
    let f = fixture("async-popup-answer", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    assert!(
        answer_through_router(&f, id, "stranger", "unauthorized-client")
            .await
            .is_err()
    );
    assert!(answers(&f, id).is_empty());
    answer_through_router(&f, id, "local", "owner-web-dpr2")
        .await
        .unwrap();
    assert!(answer_through_router(&f, id, "local", "owner-tui")
        .await
        .is_err());
    let answers = answers(&f, id);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["status"], "answered");
    assert_eq!(answers[0]["choice_id"], "continue");
    assert_eq!(answers[0]["reply"], "Exact reply: continue.");
    assert_eq!(answers[0]["answered_by"], "owner-web-dpr2");
    assert_eq!(answers[0]["client_id"], "owner-web-dpr2");
    assert_eq!(answers[0]["user_id"], "local");
}

#[tokio::test]
async fn async_popup_returns_plain_turn_end_keeps_the_question_without_a_correction() {
    let f = fixture("async-popup-turn-end", true).await;
    let task = f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let Outcome::Settled { task, correction } = f
        .store()
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: task.prompt_id,
            run: f.run.clone(),
            has_answer: true,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("settlement outcome");
    };
    assert!(
        !correction,
        "a question must not force the emitting turn to stay open"
    );
    assert_eq!(task.state, ExecutionState::Waiting);
    f.runtime.owned.withdraw_stale_agent_interactions();
    answer_through_router(&f, id, "local", "owner-tui")
        .await
        .unwrap();
    assert_eq!(answers(&f, id).len(), 1);
}

#[tokio::test]
async fn async_popup_returns_recovery_reattaches_the_same_pending_card_and_delivers_once() {
    let f = fixture("async-popup-recovery", true).await;
    let task = f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let wake = f.store().agent_wake(id).unwrap().unwrap();
    f.wait_on(&task, &wake.registration_id);
    // Lose only process-local responders, as a restart does. The source and
    // projected interaction stay in their durable stores.
    f.runtime.owned.pending_interactions.write().remove(id);
    let driver = f
        .runtime
        .owned
        .agent_wakes
        .drivers
        .get("interaction")
        .unwrap();
    assert_eq!(driver.recover(&wake), Recovery::Reattached);
    assert_eq!(driver.recover(&wake), Recovery::Reattached);
    assert_eq!(
        f.runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .len(),
        1
    );
    answer_through_router(&f, id, "local", "owner-after-restart")
        .await
        .unwrap();
    assert_eq!(answers(&f, id).len(), 1);
    assert_eq!(f.store().agent_wake(id).unwrap().unwrap().fire_count, 1);
}

#[tokio::test]
async fn async_popup_returns_timeouts_deliver_default_or_no_reply_once_and_close_the_card() {
    for default in [None, Some("continue")] {
        let f = fixture("async-popup-timeout", true).await;
        f.begin(&f.prompt_id);
        let result = ask(
            &f,
            serde_json::json!({"timeout_sec":1, "default_on_timeout":default}),
        )
        .await;
        let id = result["interaction_id"].as_str().unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        f.runtime
            .timeout_runtime_interaction(&f.room, id)
            .await
            .unwrap();
        f.runtime
            .timeout_runtime_interaction(&f.room, id)
            .await
            .unwrap();
        assert!(answer_through_router(&f, id, "local", "late-client")
            .await
            .is_err());
        let answers = answers(&f, id);
        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0]["status"], "timed_out");
        assert_eq!(answers[0]["choice_id"].as_str(), default);
        assert_eq!(
            answers[0]["reply"].as_str(),
            default.map(|_| "Exact reply: continue.")
        );
        assert!(answers[0]["answered_by"].is_null());
        assert!(f
            .runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .is_empty());
    }
}

#[tokio::test]
async fn async_popup_returns_stop_withdraws_without_applying_a_timeout_default() {
    let f = fixture("async-popup-stop", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({"default_on_timeout":"continue"})).await;
    let id = result["interaction_id"].as_str().unwrap();
    f.runtime
        .owned
        .withdraw_agent_interactions(&f.room, Some(&f.agent))
        .unwrap();
    assert_eq!(f.wake_state(id), "lost");
    let withdrawn = answers(&f, id);
    assert_eq!(withdrawn.len(), 1);
    assert_eq!(withdrawn[0]["status"], "withdrawn");
    assert!(withdrawn[0]["reply"].is_null());
    assert!(answer_through_router(&f, id, "local", "late-client")
        .await
        .is_err());
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
}

#[tokio::test]
async fn async_popup_keeps_multiple_questions_and_prioritizes_a_blocking_card() {
    let f = fixture("async-popup-coexist", true).await;
    f.begin(&f.prompt_id);
    let first = ask(&f, serde_json::json!({"title":"first"})).await;
    let second = ask(&f, serde_json::json!({"title":"second"})).await;
    let blocking = crate::session::RuntimeInteraction::new(
        "blocking-card",
        &f.agent,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Work needs this answer first",
        vec![crate::session::RuntimeInteractionChoice::new(
            "continue", "Continue", "continue", None,
        )],
        None,
        None,
        None,
    );
    let receiver = f
        .runtime
        .create_runtime_interaction(&f.room, blocking)
        .await
        .unwrap();
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert_eq!(session.active_interactions().len(), 3);
    assert_eq!(session.active_interactions()[0].id(), "blocking-card");
    assert!(session
        .active_interactions()
        .iter()
        .any(|q| Some(q.id()) == first["interaction_id"].as_str()));
    assert!(session
        .active_interactions()
        .iter()
        .any(|q| Some(q.id()) == second["interaction_id"].as_str()));
    f.runtime
        .resolve_runtime_interaction(&f.room, "blocking-card", "continue", None)
        .await
        .unwrap();
    assert_eq!(receiver.await.unwrap().reply.as_deref(), Some("continue"));
    assert_eq!(
        f.runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .len(),
        2
    );
}

#[tokio::test]
async fn async_popup_task_cancel_withdraws_and_refuses_recovery_and_late_answers() {
    let f = fixture("async-popup-cancel-task", true).await;
    let task = f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({"default_on_timeout":"continue"})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let current = f.task(&task.task_id);
    f.store()
        .agent_lifecycle(Operation::CancelTask {
            task: task.task_id.clone(),
            owner: "local".into(),
            revision: current.revision,
        })
        .unwrap();
    f.runtime.owned.withdraw_stale_agent_interactions();
    assert!(answer_through_router(&f, id, "local", "late-client")
        .await
        .is_err());
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let wake = f.store().agent_wake(id).unwrap().unwrap();
    let recovered = f
        .runtime
        .owned
        .agent_wakes
        .drivers
        .get("interaction")
        .unwrap()
        .recover(&wake);
    assert!(matches!(recovered, Recovery::Lost(_)));
    assert!(answers(&f, id)
        .iter()
        .all(|answer| answer["reply"].is_null()));
}

#[tokio::test]
async fn async_popup_kernel_shutdown_preserves_the_task_question_for_restart() {
    let f = fixture("async-popup-shutdown", true).await;
    let task = f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({"timeout_sec":300})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let wake = f.store().agent_wake(id).unwrap().unwrap();
    f.wait_on(&task, &wake.registration_id);
    f.runtime.shutdown_cleanup().await.unwrap();
    assert_eq!(
        f.wake_state(id),
        "running",
        "kernel shutdown is not an owner stop of the asking agent"
    );
    assert!(
        answers(&f, id).is_empty(),
        "shutdown must not manufacture a lost answer"
    );
    // Discard the process-local responder, then reattach through the same driver.
    f.runtime.owned.pending_interactions.write().remove(id);
    let driver = f
        .runtime
        .owned
        .agent_wakes
        .drivers
        .get("interaction")
        .unwrap();
    assert_eq!(driver.recover(&wake), Recovery::Reattached);
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .any(|q| q.id() == id));
    f.runtime.owned.pending_interactions.write().remove(id);
}

#[tokio::test]
async fn async_popup_review_failed_turn_stays_blocked_with_question_pending() {
    let f = fixture("async-review-failure", true).await;
    let task = f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let reason =
        "Provider run failed: connection closed. Owner must reconcile and resume or cancel";
    crate::durable_state::agent_lifecycle::end_failed_turn(f.store(), &task, reason.into())
        .unwrap();
    let current = f
        .store()
        .agent_tasks(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == task.task_id)
        .unwrap();
    assert_eq!(current.state, ExecutionState::Blocked);
    assert_eq!(current.reason, reason);
    assert!(current.wait.is_none());
    assert_eq!(f.wake_state(id), "running");
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .any(|q| q.id() == id));
    f.runtime
        .owned
        .complete_local_prompt_without_advance(&f.room, &f.agent, Some(&f.run))
        .unwrap();
    answer_through_router(&f, id, "local", "owner-after-failure")
        .await
        .unwrap();
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    let receipt = f
        .store()
        .agent_inbox(&f.room, &f.agent, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.source_id == id)
        .unwrap();
    assert_eq!(
        receipt.state, "expired",
        "blocked tasks do not automatically restart a failed provider"
    );
    assert_eq!(
        receipt.payload["public_answer"]["reply"],
        "Exact reply: continue."
    );
    let blocked = f
        .store()
        .agent_tasks(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == task.task_id)
        .unwrap();
    assert_eq!(blocked.state, ExecutionState::Blocked);
    assert_eq!(blocked.reason, reason);
    f.store()
        .agent_lifecycle(Operation::OwnerResponse {
            task: task.task_id.clone(),
            revision: blocked.blocked_revision,
            resume: true,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    f.store()
        .agent_lifecycle(Operation::Ack {
            room: f.room.clone(),
            agent: f.agent.clone(),
            sequence: receipt.sequence,
            handled: true,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    let resumed = f
        .store()
        .agent_tasks(Some(&f.room), Some(&f.agent))
        .unwrap()
        .into_iter()
        .find(|t| t.task_id == task.task_id)
        .unwrap();
    assert!(resumed.obligations.iter().all(|o| o.status == "satisfied"));
}

#[tokio::test]
async fn async_popup_review_finished_ticks_do_not_take_the_mutation_lock() {
    let f = fixture("async-review-finished", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    answer_through_router(&f, id, "local", "owner-tui")
        .await
        .unwrap();
    let sequence = f
        .store()
        .agent_wake(id)
        .unwrap()
        .unwrap()
        .last_sequence
        .unwrap();
    f.store()
        .agent_lifecycle(Operation::Expire {
            room: f.room.clone(),
            agent: f.agent.clone(),
            sequence,
        })
        .unwrap();
    let guard = f
        .runtime
        .owned
        .pending_interactions
        .mutation
        .lock()
        .unwrap();
    let runtime = f.runtime.clone();
    let handle = tokio::runtime::Handle::current();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _entered = handle.enter();
        for _ in 0..65 {
            runtime
                .tick_interaction_wakes(crate::session::unix_epoch_ms())
                .unwrap();
        }
        send.send(()).unwrap();
    });
    let completed = receive.recv_timeout(Duration::from_secs(2)).is_ok();
    drop(guard);
    worker.join().unwrap();
    assert!(
        completed,
        "finished questions must not reacquire the interaction mutation lock each second"
    );
    assert_eq!(
        f.store()
            .agent_wake(id)
            .unwrap()
            .unwrap()
            .last_delivery
            .as_deref(),
        Some("expired")
    );
}

#[tokio::test]
async fn async_popup_review_finished_recovery_cleans_the_remaining_card_once() {
    let f = fixture("async-review-finished-recovery", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    f.store()
        .agent_lifecycle(Operation::WakeOccurred {
            id: id.into(),
            occurrence: "resolved-before-crash".into(),
            kind: "interaction_resolved".into(),
            summary: None,
            payload: serde_json::json!({"status":"answered"}),
            end: Some(WakeEnd::Succeeded),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    assert!(f
        .runtime
        .owned
        .pending_interactions
        .write()
        .contains_key(id));
    f.runtime
        .tick_interaction_wakes(crate::session::unix_epoch_ms())
        .unwrap();
    assert!(!f
        .runtime
        .owned
        .pending_interactions
        .write()
        .contains_key(id));
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
    f.runtime
        .tick_interaction_wakes(crate::session::unix_epoch_ms())
        .unwrap();
    assert_eq!(f.store().agent_wake(id).unwrap().unwrap().fire_count, 1);
}

#[tokio::test]
async fn async_popup_review_bad_question_does_not_stall_other_questions_or_timers() {
    let f = fixture("async-review-isolation", true).await;
    let task = f.begin(&f.prompt_id);
    let bad = ask(&f, serde_json::json!({"timeout_sec":1})).await;
    let good = ask(&f, serde_json::json!({"timeout_sec":1})).await;
    let bad_id = bad["interaction_id"].as_str().unwrap();
    let good_id = good["interaction_id"].as_str().unwrap();
    let mut session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    session.remove_active_interaction(bad_id);
    f.runtime
        .owned
        .session_store
        .write()
        .restore_session(session);
    let now = crate::session::unix_epoch_ms();
    let mut timer = f.wake(&task, "review-independent-timer");
    timer.kind = "timer".into();
    timer.state = "scheduled".into();
    timer.next_due_ms = Some(now + 1_000);
    f.store()
        .agent_lifecycle(Operation::ArmWake {
            task: task.task_id,
            prompt: task.prompt_id,
            wake: Box::new(timer),
        })
        .unwrap();
    f.runtime.tick_interaction_wakes(now + 20_000).unwrap();
    assert_eq!(
        answers(&f, good_id).len(),
        1,
        "one bad question must not block another timeout"
    );
    f.runtime.sweep_agent_wakes(now + 20_000).await.unwrap();
    assert_eq!(
        f.store()
            .agent_wake("review-independent-timer")
            .unwrap()
            .unwrap()
            .fire_count,
        1
    );
}

#[tokio::test]
async fn async_popup_review_wait_deadline_follows_the_latest_question() {
    for seconds in [Some(60), Some(10 * 86_400), None] {
        let f = fixture("async-review-deadline", true).await;
        let task = f.begin(&f.prompt_id);
        let result = ask(&f, serde_json::json!({"timeout_sec":seconds})).await;
        ask(&f, serde_json::json!({"timeout_sec":1})).await;
        let wake = f
            .store()
            .agent_wake(result["interaction_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        let Outcome::Settled { task, .. } = f
            .store()
            .agent_lifecycle(Operation::Settle {
                room: f.room.clone(),
                agent: f.agent.clone(),
                prompt: task.prompt_id,
                run: f.run.clone(),
                has_answer: true,
                cancelled: false,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("settlement outcome")
        };
        assert_eq!(
            task.wait.unwrap().deadline_ms,
            wake.next_due_ms.unwrap_or(8_639_999_999_999_999)
        );
    }
}
