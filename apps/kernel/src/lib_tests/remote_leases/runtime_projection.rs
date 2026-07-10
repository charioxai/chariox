use super::*;

#[test]
fn remote_runtime_projection_records_output_and_completion_on_home_session() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter meta mode");
    let trace_subscription = app.metaagent_trace_subscription_store().subscribe(
        session.id(),
        metaagent.id(),
        agent.id(),
        crate::runtime::metaagent_trace::MetaagentTraceMode::Compact,
    );
    let prompt = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "remote prompt",
            Vec::new(),
        )
        .expect("prompt should start");
    assert!(matches!(prompt, PromptSubmissionOutcome::Started { .. }));

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            vec![RelayProjectedOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("assistant-1".to_string()),
                bytes: b"remote output".to_vec(),
            }],
            vec!["remote notice".to_string()],
            vec![RelayProjectedCompletion {
                message_id: "assistant-msg-1".to_string(),
                completed_at_ms: 1234,
                home_prompt_id: None,
            }],
        )
        .expect("projection should succeed");

    let outputs = app
        .terminal_mut()
        .drain_output_records(session.id(), attachment.id());
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(outputs[0].bytes, b"remote output".to_vec());

    let notices = app
        .terminal_mut()
        .drain_notice_records(session.id(), attachment.id());
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(notices[0].message, "remote notice");

    let completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id());
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(completions[0].message_id, "assistant-msg-1");

    let trace_outputs = app
        .terminal_mut()
        .drain_output_records(session.id(), &trace_subscription.recipient_attachment_id);
    assert_eq!(trace_outputs.len(), 1);
    assert_eq!(trace_outputs[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(trace_outputs[0].bytes, b"remote output".to_vec());

    let trace_notices = app
        .terminal_mut()
        .drain_notice_records(session.id(), &trace_subscription.recipient_attachment_id);
    assert_eq!(trace_notices.len(), 1);
    assert_eq!(trace_notices[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(trace_notices[0].message, "remote notice");

    let trace_completions = app
        .terminal_mut()
        .drain_completion_records(session.id(), &trace_subscription.recipient_attachment_id);
    assert_eq!(trace_completions.len(), 1);
    assert_eq!(trace_completions[0].agent_id.as_deref(), Some(agent.id()));
    assert_eq!(trace_completions[0].message_id, "assistant-msg-1");

    let projected = app
        .session_state_projection_store()
        .get(session.id())
        .expect("projection should refresh");
    assert!(projected
        .prompt_states()
        .get(agent.id())
        .and_then(|state| state.active_prompt())
        .is_none());
}

#[test]
fn stale_remote_completion_replay_does_not_complete_the_next_prompt() {
    let mut app =
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let first = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "first remote prompt",
            Vec::new(),
        )
        .expect("first prompt should start");
    let PromptSubmissionOutcome::Started { prompt: first } = first else {
        panic!("first prompt should be active");
    };
    let stale_completion = RelayProjectedCompletion {
        message_id: "assistant-msg-1".to_string(),
        completed_at_ms: 1234,
        home_prompt_id: Some(first.id().to_string()),
    };
    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![stale_completion.clone()],
        )
        .expect("first completion should project");
    let _ = app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id());

    let second = app
        .submit_prompt(
            session.id(),
            attachment.id(),
            Some(agent.id()),
            "second remote prompt",
            Vec::new(),
        )
        .expect("second prompt should start");
    let PromptSubmissionOutcome::Started { prompt: second } = second else {
        panic!("second prompt should be active");
    };

    RemoteLeaseRuntime::new(&mut app)
        .project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![stale_completion],
        )
        .expect("stale replay should be ignored");

    let active = app
        .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
        .expect("active prompt should load")
        .expect("second prompt must remain active");
    assert_eq!(active.id(), second.id());
    assert!(app
        .terminal_mut()
        .drain_completion_records(session.id(), attachment.id())
        .is_empty());
}
