use super::*;

#[test]
fn focus_does_not_disturb_multi_agent_provider_liveness() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let default_run =
        launch_dev_stub_provider(&mut app, session.id(), default_agent.id(), "sonnet");

    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "claude-code").with_alias("agent-b"))
        .expect("second agent should spawn");

    let _second_run = launch_dev_stub_provider(&mut app, session.id(), second_agent.id(), "opus");

    crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), default_agent.id())
        .expect("focus should succeed");

    let started = LocalDaemonResponse::PromptSubmitted {
        outcome: crate::app::KernelAgentService::new(&mut app)
            .submit_prompt(session.id(), attachment.id(), None, "hello", Vec::new())
            .expect("prompt should start"),
        session: crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session snapshot should load"),
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 0,
    };

    match started {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), default_agent.id());
            }
            _ => panic!("expected prompt to start immediately"),
        },
        _ => panic!("unexpected local response"),
    }

    let agent = crate::app::KernelSessionService::new(&mut app)
        .focus_agent(session.id(), second_agent.id())
        .expect("focus should succeed");
    let response = LocalDaemonResponse::AgentFocused { agent };

    assert!(matches!(response, LocalDaemonResponse::AgentFocused { .. }));
    let session_state = app
        .sessions()
        .get_session(session.id())
        .expect("session should exist");
    assert_eq!(session_state.focused_agent_id(), Some(second_agent.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(default_run.id())
    );
    assert!(session_state
        .active_prompt_for_agent(default_agent.id())
        .is_some());
}
