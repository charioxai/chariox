use super::*;

// Keep large router/scheduler futures out of the async tests' poll frames,
// including their construction before they are moved onto the heap.
#[inline(never)]
fn boxed_schedule_future<F: std::future::Future>(
    make_future: impl FnOnce() -> F,
) -> std::pin::Pin<Box<F>> {
    Box::pin(make_future())
}

#[tokio::test]
async fn agent_prompt_schedules_start_queue_recur_and_cancel_through_kernel_authority() {
    let worktree = crate::test_support::TestWorktree::new("agent-prompt-schedules");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let once_request = LocalDaemonRequest::CreateAgentPromptSchedule(
        crate::local::CreateAgentPromptScheduleRequest {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            kind: crate::session::AgentPromptScheduleKind::Once,
            interval_seconds: 60,
            prompt: None,
        },
    );
    let once_command =
        KernelCommand::from_local_request("create-wait-in", None, None, &once_request);
    let once = boxed_schedule_future(|| router.dispatch(once_command, once_request))
        .await
        .expect("one-shot schedule should create");
    let once_schedule = match once {
        LocalDaemonResponse::AgentPromptScheduleCreated { schedule, session } => {
            assert_eq!(schedule.prompt(), "Continue from where you left off.");
            assert_eq!(session.agent_prompt_schedules().len(), 1);
            schedule
        }
        response => panic!("unexpected one-shot schedule response: {response:?}"),
    };

    let recurring_request = LocalDaemonRequest::CreateAgentPromptSchedule(
        crate::local::CreateAgentPromptScheduleRequest {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            kind: crate::session::AgentPromptScheduleKind::Recurring,
            interval_seconds: 60,
            prompt: Some("Check whether more work remains.".to_string()),
        },
    );
    let recurring_command =
        KernelCommand::from_local_request("create-wait-every", None, None, &recurring_request);
    let recurring = boxed_schedule_future(|| router.dispatch(recurring_command, recurring_request))
        .await
        .expect("recurring schedule should create");
    let recurring_schedule = match recurring {
        LocalDaemonResponse::AgentPromptScheduleCreated { schedule, session } => {
            assert_eq!(session.agent_prompt_schedules().len(), 2);
            schedule
        }
        response => panic!("unexpected recurring schedule response: {response:?}"),
    };

    let due_at_ms = once_schedule
        .next_run_at_ms()
        .max(recurring_schedule.next_run_at_ms());
    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_prompt_schedules(due_at_ms)
    })
    .await;

    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("scheduled prompt snapshot should remain readable");
    assert_eq!(
        snapshot
            .active_prompt_for_agent(agent.id())
            .map(|prompt| prompt.prompt()),
        Some("Continue from where you left off.")
    );
    assert_eq!(
        snapshot
            .queued_prompts_for_agent(agent.id())
            .and_then(|prompts| prompts.front())
            .map(|prompt| prompt.prompt()),
        Some("Check whether more work remains.")
    );
    assert!(
        snapshot
            .agent_prompt_schedules()
            .iter()
            .all(|schedule| schedule.id() != once_schedule.id()),
        "one-shot schedule should disappear after normal prompt admission"
    );
    let projected_recurring = snapshot
        .agent_prompt_schedules()
        .iter()
        .find(|schedule| schedule.id() == recurring_schedule.id())
        .expect("recurring schedule should remain");
    assert_eq!(projected_recurring.runs_dispatched(), 1);
    assert!(projected_recurring.next_run_at_ms() > due_at_ms);

    for tick in 1..=43 {
        let now_ms = due_at_ms + tick * 60_000;
        boxed_schedule_future(|| {
            router
                .runtime_state
                .dispatch_due_agent_prompt_schedules(now_ms)
        })
        .await;
        let snapshot = router
            .runtime_state
            .session_snapshot(session.id())
            .await
            .expect("busy schedule snapshot should remain readable");
        assert_eq!(
            snapshot.queued_prompts_for_agent(agent.id()).unwrap().len(),
            1,
            "a busy agent must retain only one occurrence of the recurrence"
        );
        let schedule = &snapshot.agent_prompt_schedules()[0];
        assert_eq!(schedule.runs_dispatched(), 1);
        assert_eq!(schedule.last_triggered_at_ms(), Some(due_at_ms));
        assert_eq!(schedule.next_run_at_ms(), now_ms + 60_000);
    }

    let cancel_request = LocalDaemonRequest::CancelAgentPromptSchedule(
        crate::local::CancelAgentPromptScheduleRequest {
            session_id: session.id().to_string(),
            schedule_id: recurring_schedule.id().to_string(),
        },
    );
    let cancel_command =
        KernelCommand::from_local_request("cancel-wait-every", None, None, &cancel_request);
    let cancelled = boxed_schedule_future(|| router.dispatch(cancel_command, cancel_request))
        .await
        .expect("recurring schedule should cancel");
    match cancelled {
        LocalDaemonResponse::AgentPromptScheduleCancelled { schedule, session } => {
            assert_eq!(schedule.id(), recurring_schedule.id());
            assert!(session.agent_prompt_schedules().is_empty());
            assert_eq!(
                session.queued_prompts_for_agent(agent.id()).unwrap().len(),
                1
            );
            assert_eq!(
                session
                    .active_prompt_for_agent(agent.id())
                    .unwrap()
                    .prompt(),
                "Continue from where you left off."
            );
        }
        response => panic!("unexpected schedule cancellation response: {response:?}"),
    }
}

#[tokio::test]
async fn recurring_agent_prompt_schedules_coalesce_active_and_restored_occurrences() {
    let worktree = crate::test_support::TestWorktree::new("schedule-coalescing");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should create");
    launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "schedule-model",
    );
    let prompt_owner = app.prompt_state_owner();
    let session_store = app.session_state_store();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let mut schedules = Vec::new();
    for _ in 0..2 {
        let response = router
            .runtime_state
            .create_agent_prompt_schedule(crate::local::CreateAgentPromptScheduleRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                kind: crate::session::AgentPromptScheduleKind::Recurring,
                interval_seconds: 60,
                prompt: Some("The same prompt text".to_string()),
            })
            .await
            .expect("independent schedule should create");
        let LocalDaemonResponse::AgentPromptScheduleCreated { schedule, .. } = response else {
            panic!("unexpected schedule response");
        };
        schedules.push(schedule);
    }
    let due_at_ms = schedules
        .iter()
        .map(|schedule| schedule.next_run_at_ms())
        .max()
        .unwrap();
    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_prompt_schedules(due_at_ms)
    })
    .await;
    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    assert_eq!(
        active.as_ref().unwrap().agent_prompt_schedule_id(),
        Some(schedules[0].id())
    );
    assert_eq!(
        queued.len(),
        1,
        "a different schedule with identical text must still queue"
    );
    assert_eq!(
        queued[0].agent_prompt_schedule_id(),
        Some(schedules[1].id())
    );

    let manual = crate::session::PromptQueueItem::new(
        "manual-same-text",
        active.as_ref().unwrap().source_attachment_id(),
        agent.id(),
        "The same prompt text",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Queued { prompt: manual } = prompt_owner
        .submit_prepared_prompt(&session, manual, false)
        .expect("same-text manual prompt should queue")
    else {
        panic!("manual prompt should remain queued");
    };
    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    let snapshot = session_store
        .mirror_agent_prompt_state(session.id(), agent.id(), active, queued)
        .expect("prompt state should project");

    // Replay the same public session and private prompt records used by restart recovery.
    let mut durable: crate::durable_prompt_state::DurablePromptStateEventPayload =
        serde_json::from_value(
            serde_json::to_value(
                crate::durable_prompt_state::DurablePromptStateEventPayload::capture(
                    &snapshot,
                    agent.id(),
                ),
            )
            .unwrap(),
        )
        .unwrap();
    durable.restore_private_states();
    let mut restored: crate::session::RuntimeSession =
        serde_json::from_value(serde_json::to_value(&snapshot).unwrap()).unwrap();
    restored.mirror_agent_prompt_state(agent.id(), durable.active_prompt, durable.queued_prompts);
    prompt_owner.restore_session_state(&restored);
    session_store.restore_session(restored);

    for tick in 1..=43 {
        let now_ms = due_at_ms + tick * 60_000;
        boxed_schedule_future(|| {
            router
                .runtime_state
                .dispatch_due_agent_prompt_schedules(now_ms)
        })
        .await;
        let (active, queued) = prompt_owner.state_parts(&session, agent.id());
        assert_eq!(
            active.as_ref().unwrap().agent_prompt_schedule_id(),
            Some(schedules[0].id())
        );
        assert_eq!(
            queued.len(),
            2,
            "active and queued occurrences must both coalesce after restart"
        );
        assert_eq!(
            queued[0].agent_prompt_schedule_id(),
            Some(schedules[1].id())
        );
        assert_eq!(
            queued[1].id(),
            manual.id(),
            "coalescing must preserve the manual prompt"
        );
        let snapshot = session_store.get_session(session.id()).unwrap();
        for schedule in snapshot.agent_prompt_schedules() {
            assert_eq!(schedule.runs_dispatched(), 1);
            assert_eq!(schedule.next_run_at_ms(), now_ms + 60_000);
        }
    }

    prompt_owner
        .complete_active_prompt_only(&session, agent.id())
        .expect("first occurrence should finish");
    prompt_owner
        .activate_next_queued_prompt_with_prompt_id(
            &session,
            agent.id(),
            None,
            "active-second-schedule".to_string(),
        )
        .unwrap()
        .expect("second schedule should start");
    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_prompt_schedules(due_at_ms + 44 * 60_000)
    })
    .await;
    let (active, queued) = prompt_owner.state_parts(&session, agent.id());
    assert_eq!(
        active.as_ref().unwrap().agent_prompt_schedule_id(),
        Some(schedules[1].id())
    );
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].id(), manual.id());
    assert_eq!(
        queued[1].agent_prompt_schedule_id(),
        Some(schedules[0].id())
    );
    let snapshot = session_store.get_session(session.id()).unwrap();
    assert_eq!(snapshot.agent_prompt_schedules()[0].runs_dispatched(), 2);
    assert_eq!(snapshot.agent_prompt_schedules()[1].runs_dispatched(), 1);

    for schedule in schedules {
        router
            .runtime_state
            .cancel_agent_prompt_schedule(crate::local::CancelAgentPromptScheduleRequest {
                session_id: session.id().to_string(),
                schedule_id: schedule.id().to_string(),
            })
            .await
            .expect("schedule should cancel future ticks");
    }
    boxed_schedule_future(|| {
        router
            .runtime_state
            .dispatch_due_agent_prompt_schedules(due_at_ms + 100 * 60_000)
    })
    .await;
    assert_eq!(
        prompt_owner.state_parts(&session, agent.id()),
        (active, queued),
        "cancellation must preserve already-admitted scheduled and manual prompts"
    );
}

#[test]
fn agent_prompt_schedule_state_survives_session_serialization() {
    let mut session = crate::session::RuntimeSession::new(
        "session-wait-reload",
        None,
        "workspace",
        "worktree",
        "machine",
        "kernel",
    );
    session.add_agent_prompt_schedule(crate::session::AgentPromptSchedule::new(
        "wait-1",
        "agent-1",
        crate::session::AgentPromptScheduleKind::Recurring,
        300,
        "Continue the audit.",
        1_000,
    ));
    let restored: crate::session::RuntimeSession =
        serde_json::from_value(serde_json::to_value(&session).expect("session should serialize"))
            .expect("session should deserialize");
    assert_eq!(
        restored.agent_prompt_schedules(),
        session.agent_prompt_schedules()
    );
}
