use super::*;

#[test]
fn handles_prompt_submit_through_agent_request_surface() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attach should succeed");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");

    let outcome = crate::app::KernelAgentService::new(&mut app)
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "hello",
            Vec::new(),
        )
        .expect("prompt submit should succeed");
    let response = LocalDaemonResponse::PromptSubmitted {
        outcome,
        session: crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(session.id())
            .expect("session snapshot should load"),
        agent_activity: std::collections::BTreeMap::new(),
        agent_activity_revision: 0,
    };

    match response {
        LocalDaemonResponse::PromptSubmitted {
            outcome,
            session: projected_session,
            ..
        } => {
            match outcome {
                PromptSubmissionOutcome::Started { prompt } => {
                    assert_eq!(prompt.target_agent_id(), agent.id());
                }
                _ => panic!("expected prompt to start immediately"),
            }
            assert_eq!(projected_session.id(), session.id());
        }
        _ => panic!("unexpected local response"),
    }
}

#[test]
fn handles_prompt_cancel_through_agent_request_surface() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attach should succeed");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
    crate::app::KernelAgentService::new(&mut app)
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "hello",
            Vec::new(),
        )
        .expect("prompt submit should succeed");

    let response = LocalDaemonResponse::PromptCancelled {
        cancellation: crate::app::KernelAgentService::new(&mut app)
            .cancel_active_prompt(session.id(), attachment.id())
            .expect("prompt cancel should succeed"),
    };

    match response {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(cancellation.prompt.target_agent_id(), agent.id());
            assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
        }
        _ => panic!("unexpected local response"),
    }
}
