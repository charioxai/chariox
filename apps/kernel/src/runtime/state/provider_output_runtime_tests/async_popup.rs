//! MP-08 / MP-09 / MP-10 / MP-11 AM-ASK: authenticated non-blocking questions.
use super::*;

fn browser_region() -> crate::session::RuntimeInteractionBrowserRegion {
    crate::session::RuntimeInteractionBrowserRegion {
        tab_id: "tab-one".into(),
        reference: "backend:42".into(),
        generation: 1,
        document_id: "doc-one".into(),
        origin: "https://en.wikipedia.org".into(),
        rect: crate::session::RuntimeInteractionBrowserRect {
            x: 10,
            y: 20,
            width: 200,
            height: 100,
        },
        viewport_width: 1280,
        viewport_height: 800,
        verification: None,
        verification_result: None,
    }
}

#[tokio::test]
async fn async_popup_browser_region_preserves_equal_timestamp_card_order() {
    let f = fixture("async-popup-region-equal-time-order", true).await;
    f.begin(&f.prompt_id);
    let first = ask(&f, serde_json::json!({})).await;
    let second = ask(&f, serde_json::json!({})).await;
    let first_id = first["interaction_id"].as_str().unwrap();
    let second_id = second["interaction_id"].as_str().unwrap();
    let region = browser_region();
    {
        let mut sessions = f.runtime.owned.session_store.write();
        let mut session = sessions.get_session(&f.room).unwrap();
        let first = session.remove_active_interaction(first_id).unwrap();
        let second = session.remove_active_interaction(second_id).unwrap();
        let mut second = serde_json::to_value(second).unwrap();
        second["requested_at_ms"] = first.requested_at_ms().into();
        session.add_active_interaction(first.with_view(crate::session::RuntimeInteractionView {
            browser_region: region.clone(),
        }));
        session.add_active_interaction(serde_json::from_value(second).unwrap());
        sessions.restore_session(session);
    }
    let mut moved = region.clone();
    moved.rect.y = 2;
    f.runtime
        .owned
        .update_browser_region_card(&f.room, first_id, &region, moved)
        .unwrap();
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert_eq!(
        session
            .active_interactions()
            .iter()
            .map(|card| card.id())
            .collect::<Vec<_>>(),
        vec![first_id, second_id]
    );
}

#[tokio::test]
async fn async_popup_browser_region_refuses_without_browser_authority_before_registering() {
    let f = fixture("async-popup-region-admission", true).await;
    f.begin(&f.prompt_id);
    let result = try_ask(
        &f,
        serde_json::json!({"view":{"browser_region":{"tab_id":"foreign-tab","ref":"backend:42"}}}),
    )
    .await;
    assert!(result.is_err(), "a popup cannot grant browser access");
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
    assert!(f
        .store()
        .agent_wakes(Some(&f.room), Some(&f.agent))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn async_popup_browser_region_source_failure_is_unavailable_without_claiming_condition_met() {
    let f = fixture("async-popup-region-source-unavailable", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    f.runtime
        .close_unavailable_browser_region_card(&f.room, id, "Browser access was revoked".into())
        .unwrap();
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let told = answers(&f, id);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["status"], "unavailable");
    assert!(told[0]["choice_id"].is_null());
    assert_eq!(told[0]["reply"], "Browser access was revoked");
}

#[tokio::test]
async fn async_popup_browser_region_projects_movement_and_closes_exactly_once_on_condition_met() {
    let f = fixture("async-popup-region-projection", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let initial = browser_region();
    // Browser resolution is covered through the production controller entry
    // separately; this seeds its result to exercise the real card projection,
    // durable question settlement and attributed answer inbox.
    {
        let mut sessions = f.runtime.owned.session_store.write();
        let mut session = sessions.get_session(&f.room).unwrap();
        let card = session.remove_active_interaction(id).unwrap();
        session.add_active_interaction(card.with_view(crate::session::RuntimeInteractionView {
            browser_region: initial.clone(),
        }));
        sessions.restore_session(session);
    }
    let mut moved = initial.clone();
    moved.rect.y = 4;
    moved.rect.height = 60;
    moved.viewport_height = 600;
    f.runtime
        .owned
        .update_browser_region_card(&f.room, id, &initial, moved.clone())
        .unwrap();
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert_eq!(session.active_interactions().len(), 1);
    let card = &session.active_interactions()[0];
    assert_eq!(card.view().unwrap().browser_region, moved);
    assert!(card.choice("continue").is_some());
    assert!(card.choice("cancel").is_some());
    let mut replacement = moved.clone();
    replacement.document_id = "replacement-document".into();
    assert!(
        f.runtime
            .owned
            .update_browser_region_card(&f.room, id, &moved, replacement)
            .is_err(),
        "a browser read must not rebind the card to a new document"
    );
    f.runtime
        .close_browser_region_card(&f.room, id, "Browser document changed".into())
        .unwrap();
    f.runtime
        .close_browser_region_card(&f.room, id, "Browser document changed".into())
        .unwrap();
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
    let told = answers(&f, id);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["choice_id"], "condition_met");
    assert_eq!(told[0]["reply"], "Browser document changed");
}

async fn ask(f: &Fixture, extra: serde_json::Value) -> serde_json::Value {
    try_ask(f, extra).await.unwrap().payload
}

async fn try_ask(
    f: &Fixture,
    extra: serde_json::Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
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
}

#[tokio::test]
async fn blocked_worker_can_ask_for_its_missing_owner_decision() {
    let f = fixture("blocked-popup", true).await;
    let task = f.begin(&f.prompt_id);
    f.store()
        .agent_lifecycle(Operation::Block {
            task: task.task_id.clone(),
            prompt: task.prompt_id.clone(),
            reason: "Choose the next step".into(),
        })
        .unwrap();
    let blocked = f
        .store()
        .agent_tasks(Some(&f.room), None)
        .unwrap()
        .remove(0);
    let result = ask(&f, serde_json::json!({"timeout_sec":300})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    assert!(session.active_interactions().iter().any(|i| i.id() == id));
    answer_through_router(&f, id, "local", "owner-client")
        .await
        .unwrap();
    assert_eq!(answers(&f, id)[0]["reply"], "Exact reply: continue.");
    let after = f
        .store()
        .agent_tasks(Some(&f.room), None)
        .unwrap()
        .remove(0);
    assert_eq!(after.state, ExecutionState::Blocked);
    assert_eq!(after.blocked_revision, blocked.blocked_revision);
    assert_eq!(after.reason, blocked.reason);
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
    assert!(
        inbox["events"].is_array(),
        "the same turn can keep using tools"
    );
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
    answer_choice_through_router(f, id, "continue", user, client).await
}

async fn answer_choice_through_router(
    f: &Fixture,
    id: &str,
    choice: &str,
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
            choice_id: choice.into(),
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

// VUI-1: a card with only "Done" must still let the person decline, and the agent reads a definitive outcome.
#[tokio::test]
async fn async_popup_always_offers_cancel_and_cancel_is_a_definitive_outcome() {
    let f = fixture("async-popup-cancel-choice", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let card = session
        .active_interactions()
        .iter()
        .find(|card| card.id() == id)
        .expect("the question is pending");
    assert_eq!(
        card.choices()
            .iter()
            .map(|choice| choice.id())
            .collect::<Vec<_>>(),
        vec!["continue", "cancel"]
    );
    answer_choice_through_router(&f, id, "cancel", "local", "owner-web")
        .await
        .unwrap();
    let answers = answers(&f, id);
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["status"], "cancelled");
    assert_eq!(answers[0]["choice_id"], "cancel");
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
async fn async_popup_keeps_an_agents_own_cancel_choice_and_adds_none() {
    let f = fixture("async-popup-own-cancel", true).await;
    f.begin(&f.prompt_id);
    let result = ask(
        &f,
        serde_json::json!({"choices":[
            {"id":"continue","label":"Continue","reply":"go"},
            {"id":"cancel","label":"Stop here","reply":"stopped"}
        ]}),
    )
    .await;
    let id = result["interaction_id"].as_str().unwrap();
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let card = session
        .active_interactions()
        .iter()
        .find(|card| card.id() == id)
        .unwrap();
    assert_eq!(card.choices().len(), 2);
    assert_eq!(card.choice("cancel").unwrap().label(), "Stop here");
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
        let now = crate::session::unix_epoch_ms();
        let Outcome::Settled { task, .. } = f
            .store()
            .agent_lifecycle(Operation::Settle {
                room: f.room.clone(),
                agent: f.agent.clone(),
                prompt: task.prompt_id,
                run: f.run.clone(),
                has_answer: true,
                cancelled: false,
                now,
            })
            .unwrap()
        else {
            panic!("settlement outcome")
        };
        // A question without a timeout waits the default wait timeout, like every wait.
        assert_eq!(
            task.wait.unwrap().deadline_ms,
            Some(
                wake.next_due_ms.unwrap_or(
                    now + crate::durable_state::agent_lifecycle::DEFAULT_WAIT_TIMEOUT_MS
                )
            )
        );
    }
}

// PCD-03: a card names a wake source the agent armed; the source occurring closes the card.
#[tokio::test]
async fn async_popup_follows_a_wake_source_and_closes_when_it_occurs() {
    let f = fixture("async-popup-until", true).await;
    let task = f.begin(&f.prompt_id);
    let wake = f.arm(&task, "wake-a").unwrap();
    let result = ask(&f, serde_json::json!({"until":{"wake_id":wake.id}})).await;
    let id = result["interaction_id"].as_str().unwrap();
    assert_eq!(
        f.runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .iter()
            .find(|card| card.id() == id)
            .and_then(|card| card.closes_with_wake().map(str::to_owned)),
        Some(wake.id.clone()),
        "the card carries the wake it follows"
    );
    f.driver
        .sink
        .occurred(f.occurrence(&wake.id, "o-1", None))
        .unwrap();
    for _ in 0..40 {
        if f.runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        f.runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .is_empty(),
        "the card closes within 2 s of the event"
    );
    let told = answers(&f, id);
    assert_eq!(told.len(), 1, "the agent is told once");
    assert_eq!(told[0]["status"], "answered");
    assert_eq!(told[0]["choice_id"], "condition_met");
    assert!(
        told[0]["reply"].as_str().unwrap().contains("occurred"),
        "{}",
        told[0]
    );
}

#[tokio::test]
async fn async_popup_refuses_a_condition_it_cannot_follow() {
    let f = fixture("async-popup-until-refused", true).await;
    let task = f.begin(&f.prompt_id);
    let ended = f.arm(&task, "wake-ended").unwrap();
    f.driver
        .sink
        .occurred(f.occurrence(&ended.id, "o-1", Some(WakeEnd::Succeeded)))
        .unwrap();
    let live = f.arm(&task, "wake-live").unwrap();
    for (until, wait, why) in [
        ("no-such-wake", None, "not a wake source you armed"),
        (ended.id.as_str(), None, "already ended"),
        (live.id.as_str(), Some(true), "needs wait:false"),
    ] {
        let mut extra = serde_json::json!({"until":{"wake_id":until}});
        if let Some(wait) = wait {
            extra["wait"] = wait.into();
        }
        let error = try_ask(&f, extra).await.unwrap_err().to_string();
        assert!(error.contains(why), "{until}: {error}");
    }
    assert!(f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .is_empty());
}

// PCD-03: the kernel clock is a wake source too; its firing closes the card.
#[tokio::test]
async fn async_popup_reconciles_cancelled_timer_condition_once() {
    let f = fixture("async-popup-cancelled-until-timer", true).await;
    let task = f.begin(&f.prompt_id);
    let token = f
        .runtime
        .owned
        .provider_store
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    let timer = call(
        &f,
        &token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"completion timer","delay_ms":60_000}),
    )
    .await
    .unwrap()
    .payload;
    let source = timer["wake_id"].as_str().unwrap();
    let result = ask(&f, serde_json::json!({"until":{"wake_id":source}})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let cancelled = call(
        &f,
        &token,
        "chariox.events.cancel_wake",
        &task,
        serde_json::json!({"wake_id":source}),
    )
    .await
    .unwrap()
    .payload;
    assert_eq!(cancelled["settled"], true);
    assert!(
        !f.runtime
            .owned
            .session_store
            .get_session(&f.room)
            .unwrap()
            .active_interactions()
            .iter()
            .any(|card| card.id() == id),
        "cancelling a source must close its dependent card without ending the asking task"
    );
    for _ in 0..2 {
        f.runtime
            .tick_interaction_wakes(crate::session::unix_epoch_ms())
            .unwrap();
    }
    let told = answers(&f, id);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["choice_id"], "condition_met");
    assert!(told[0]["reply"].as_str().unwrap().contains("cancelled"));
}

#[tokio::test]
async fn async_popup_reconciles_source_lost_before_first_occurrence_from_current_ledger() {
    let f = fixture("async-popup-lost-before-first-until", true).await;
    let task = f.begin(&f.prompt_id);
    let source = f.arm(&task, "wake-lost-before-first").unwrap();
    assert!(source.last_sequence.is_none());
    let result = ask(&f, serde_json::json!({"until":{"wake_id":source.id}})).await;
    let id = result["interaction_id"].as_str().unwrap();
    f.runtime
        .report_lost_wake(
            &source,
            crate::session::unix_epoch_ms(),
            "source disappeared before its first occurrence",
        )
        .unwrap();
    // Lost-source and restart paths can queue delivery with their pre-transition
    // snapshot. The completion condition must read the authoritative ledger.
    f.runtime.close_cards_following(&source);
    assert!(!f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .any(|card| card.id() == id));
    for _ in 0..2 {
        f.runtime
            .tick_interaction_wakes(crate::session::unix_epoch_ms())
            .unwrap();
        f.runtime.close_cards_following(&source);
    }
    let told = answers(&f, id);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["choice_id"], "condition_met");
    assert!(told[0]["reply"].as_str().unwrap().contains("lost"));
}

#[tokio::test]
async fn async_popup_reconciles_source_ended_during_card_registration() {
    let f = fixture("async-popup-until-registration-race", true).await;
    let task = f.begin(&f.prompt_id);
    let source = f.arm(&task, "wake-ended-before-register").unwrap();
    // Model the source ending after request_popup's initial live-source check,
    // but before the durable question is registered.
    f.runtime
        .cancel_wake_source(&source, &task.task_id, Some(&task.prompt_id))
        .unwrap();
    let card = crate::session::RuntimeInteraction::new(
        "registration-race-card",
        &f.agent,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Wait for completion",
        vec![crate::session::RuntimeInteractionChoice::new(
            "cancel", "Cancel", "cancel", None,
        )],
        None,
        None,
        None,
    )
    .closing_with_wake(source.id);
    f.runtime
        .arm_interaction_wake(&f.room, &f.agent, card)
        .unwrap();
    assert!(!f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .any(|card| card.id() == "registration-race-card"));
    for _ in 0..2 {
        f.runtime
            .tick_interaction_wakes(crate::session::unix_epoch_ms())
            .unwrap();
    }
    let told = answers(&f, "registration-race-card");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["choice_id"], "condition_met");
}

#[tokio::test]
async fn async_popup_reconciles_recovered_card_with_already_cancelled_source() {
    let f = fixture("async-popup-until-recovered-cancel", true).await;
    let task = f.begin(&f.prompt_id);
    let source = f.arm(&task, "wake-cancelled-offline").unwrap();
    let result = ask(&f, serde_json::json!({"until":{"wake_id":source.id}})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let question = f.store().agent_wake(id).unwrap().unwrap();
    f.runtime.owned.pending_interactions.write().remove(id);
    // A durable cancellation while no process-local question source is attached.
    f.store()
        .agent_lifecycle(Operation::CancelWake {
            id: source.id,
            task: task.task_id,
            prompt: Some(task.prompt_id),
            pending: false,
        })
        .unwrap();
    let driver = f
        .runtime
        .owned
        .agent_wakes
        .drivers
        .get("interaction")
        .unwrap();
    assert_eq!(driver.recover(&question), Recovery::Reattached);
    for _ in 0..2 {
        f.runtime
            .tick_interaction_wakes(crate::session::unix_epoch_ms())
            .unwrap();
    }
    assert!(!f
        .runtime
        .owned
        .session_store
        .get_session(&f.room)
        .unwrap()
        .active_interactions()
        .iter()
        .any(|card| card.id() == id));
    let told = answers(&f, id);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["choice_id"], "condition_met");
}

// PCD-03: the kernel clock is a wake source too; its firing closes the card.
#[tokio::test]
async fn async_popup_follows_a_timer_the_scheduler_fires() {
    let f = fixture("async-popup-until-timer", true).await;
    let task = f.begin(&f.prompt_id);
    let token = f
        .runtime
        .owned
        .provider_store
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    let timer = call(
        &f,
        &token,
        "chariox.events.timer",
        &task,
        serde_json::json!({"label":"card timer","delay_ms":1_000}),
    )
    .await
    .unwrap()
    .payload;
    let wake_id = timer["wake_id"].as_str().unwrap().to_owned();
    let result = ask(&f, serde_json::json!({"until":{"wake_id":wake_id}})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let scheduler = {
        let runtime = f.runtime.clone();
        tokio::spawn(async move { runtime.run_agent_wake_scheduler().await })
    };
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if f.runtime
                .owned
                .session_store
                .get_session(&f.room)
                .unwrap()
                .active_interactions()
                .is_empty()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    scheduler.abort();
    assert!(closed.is_ok(), "the card closes when its timer fires");
    let told = answers(&f, id);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["choice_id"], "condition_met");
}

/// MP-09: a managed kernel counts the unresolved cards as work. Answering one records that change after the
/// Room is updated; recording it while the Room store is still locked waited for the answerer itself, so the
/// kernel stopped heartbeating (found creating the Vault on a remote machine).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn answering_a_card_on_a_managed_kernel_does_not_wait_for_itself() {
    let f = fixture("managed-answer", true).await;
    f.runtime
        .ensure_managed_activity_tracking("kernel-managed")
        .unwrap();
    let card = crate::session::RuntimeInteraction::new(
        "blocking-card",
        &f.agent,
        crate::session::RuntimeInteractionKind::Choice,
        crate::session::RuntimeInteractionLevel::Info,
        None,
        "Work needs this answer",
        vec![crate::session::RuntimeInteractionChoice::new(
            "continue", "Continue", "continue", None,
        )],
        None,
        None,
        None,
    );
    let receiver = f
        .runtime
        .create_runtime_interaction(&f.room, card)
        .await
        .unwrap();
    let runtime = f.runtime.clone();
    let room = f.room.clone();
    let (done, answered) = std::sync::mpsc::channel();
    // A wedge blocks its thread for good, so the answer runs on its own thread and the test waits with a deadline.
    std::thread::spawn(move || {
        let outcome = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(runtime.resolve_runtime_interaction(
                &room,
                "blocking-card",
                "continue",
                None,
            ));
        let _ = done.send(outcome.is_ok());
    });
    assert_eq!(
        answered.recv_timeout(Duration::from_secs(20)),
        Ok(true),
        "answering the card never finished"
    );
    assert_eq!(receiver.await.unwrap().reply.as_deref(), Some("continue"));
}

// MP-08 / MP-09 / MP-10 / MP-11: a page disappearing is not a verification result.
#[tokio::test]
async fn async_popup_browser_verification_requires_this_cards_positive_receipt() {
    let f = fixture("async-popup-verification-receipt", true).await;
    f.begin(&f.prompt_id);
    let result = ask(&f, serde_json::json!({})).await;
    let id = result["interaction_id"].as_str().unwrap();
    let mut region = browser_region();
    region.verification = Some(crate::session::RuntimeInteractionBrowserVerification {
        challenge_id: "challenge-one".into(),
        url: "https://en.wikipedia.org/verify".into(),
        accepted_json_pointer: "/accepted".into(),
    });
    {
        let mut sessions = f.runtime.owned.session_store.write();
        let mut session = sessions.get_session(&f.room).unwrap();
        let card = session.remove_active_interaction(id).unwrap();
        session.add_active_interaction(card.with_view(crate::session::RuntimeInteractionView {
            browser_region: region.clone(),
        }));
        sessions.restore_session(session);
    }
    assert!(f
        .runtime
        .close_browser_region_card(&f.room, id, "Browser element disappeared".into())
        .is_err());
    let proof = serde_json::json!({
        "card_id":id,"tab_id":region.tab_id,"document_id":region.document_id,"generation":region.generation,
        "verification_result":{"challenge_id":"challenge-one","request_id":"request-one","input_delivered":true,"http_status":200,"result":"accepted"}
    });
    for pointer in [
        "/card_id",
        "/tab_id",
        "/document_id",
        "/verification_result/challenge_id",
    ] {
        let mut foreign = proof.clone();
        *foreign.pointer_mut(pointer).unwrap() = "foreign".into();
        assert!(f
            .runtime
            .close_browser_region_card(&f.room, id, foreign.to_string())
            .is_err());
    }
    for replacement in [
        serde_json::json!({"challenge_id":"challenge-one","request_id":"request-one","input_delivered":true,"http_status":422,"result":"rejected"}),
        serde_json::json!({"challenge_id":"challenge-one","request_id":"request-one","input_delivered":false,"http_status":200,"result":"accepted"}),
    ] {
        let mut rejected = proof.clone();
        rejected["verification_result"] = replacement;
        assert!(f
            .runtime
            .close_browser_region_card(&f.room, id, rejected.to_string())
            .is_err());
    }
    assert!(answers(&f, id).is_empty());
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
    f.runtime
        .close_browser_region_card(&f.room, id, proof.to_string())
        .unwrap();
    f.runtime
        .close_browser_region_card(&f.room, id, proof.to_string())
        .unwrap();
    let told = answers(&f, id);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0]["choice_id"], "condition_met");
}

#[tokio::test]
async fn async_popup_browser_verification_refuses_confirm_and_blocking_calls() {
    let f = fixture("async-popup-verification-shape", true).await;
    f.begin(&f.prompt_id);
    let view = serde_json::json!({"browser_region":{"tab_id":"tab","ref":"backend:42","verification":{"challenge_id":"one","url":"https://example.com/verify","accepted_json_pointer":"/accepted"}}});
    for extra in [
        serde_json::json!({"view":view}),
        serde_json::json!({"view":view,"wait":true,"choices":[{"id":"cancel","label":"Cancel","reply":"cancel"}]}),
    ] {
        let error = try_ask(&f, extra).await.unwrap_err().to_string();
        assert!(
            error.contains("verification cards require wait:false and only Cancel"),
            "{error}"
        );
    }
}
