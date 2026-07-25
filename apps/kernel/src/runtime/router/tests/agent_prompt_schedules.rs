use super::*;

#[tokio::test]
async fn agent_prompt_schedules_start_queue_recur_and_cancel_through_kernel_authority() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-agent-prompt-schedules",
            "worktree-agent-prompt-schedules",
        ))
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
    let once = router
        .dispatch(once_command, once_request)
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
    let recurring = router
        .dispatch(recurring_command, recurring_request)
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
    router
        .runtime_state
        .dispatch_due_agent_prompt_schedules(due_at_ms)
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

    let cancel_request = LocalDaemonRequest::CancelAgentPromptSchedule(
        crate::local::CancelAgentPromptScheduleRequest {
            session_id: session.id().to_string(),
            schedule_id: recurring_schedule.id().to_string(),
        },
    );
    let cancel_command =
        KernelCommand::from_local_request("cancel-wait-every", None, None, &cancel_request);
    let cancelled = router
        .dispatch(cancel_command, cancel_request)
        .await
        .expect("recurring schedule should cancel");
    match cancelled {
        LocalDaemonResponse::AgentPromptScheduleCancelled { schedule, session } => {
            assert_eq!(schedule.id(), recurring_schedule.id());
            assert!(session.agent_prompt_schedules().is_empty());
        }
        response => panic!("unexpected schedule cancellation response: {response:?}"),
    }
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
