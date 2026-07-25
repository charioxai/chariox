use super::*;

#[tokio::test]
async fn runtime_mcp_agents_can_message_and_queue_each_other_by_unique_alias() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-agent-messaging",
            "worktree-agent-messaging",
        ))
        .expect("session should be created");
    let sender = app
        .agents_mut()
        .alias_agent(sender.id(), Some("router".to_string()))
        .expect("sender alias should update");
    let reviewer = spawn_test_agent(&mut app, session.id(), "reviewer", "dev-stub");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    let reviewer_run = launch_test_provider(
        &mut app,
        session.id(),
        reviewer.id(),
        "dev-stub",
        "dev-stub",
        "reviewer-model",
    );
    let sender_token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender run should expose runtime MCP auth")
        .to_string();
    let reviewer_token = reviewer_run
        .runtime_mcp_auth_token()
        .expect("reviewer run should expose runtime MCP auth")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&sender_token);
    assert!(
        specs
            .iter()
            .any(|spec| { spec.name == crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL }),
        "regular agents should receive the agent messaging tool"
    );

    let first = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &sender_token,
            "mcp__arroba__send_agent_message",
            serde_json::json!({
                "agent": "@REVIEWER",
                "message": "Inspect package.json and report the package name."
            }),
        )
        .await
        .expect("agent message should dispatch");
    assert!(first.ok, "{:?}", first.payload);
    assert_eq!(first.payload["status"], "started");
    assert_eq!(first.payload["target_agent_id"], reviewer.id());

    let second = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &sender_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": reviewer.agent_ref(),
                "message": "Then report whether the package is private."
            }),
        )
        .await
        .expect("busy target message should queue");
    assert!(second.ok, "{:?}", second.payload);
    assert_eq!(second.payload["status"], "queued");

    let reply = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &reviewer_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": "router",
                "message": "The review has started."
            }),
        )
        .await
        .expect("reviewer should be able to message the original sender");
    assert!(reply.ok, "{:?}", reply.payload);
    assert_eq!(reply.payload["status"], "started");
    assert_eq!(reply.payload["target_agent_id"], sender.id());

    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain readable");
    let reviewer_prompt = snapshot
        .active_prompt_for_agent(reviewer.id())
        .expect("reviewer should have an active agent message");
    assert_eq!(
        reviewer_prompt.prompt(),
        "Message from @router:\n\nInspect package.json and report the package name."
    );
    assert!(reviewer_prompt
        .hidden_system_context()
        .contains("arroba.send_agent_message"));
    assert_eq!(
        snapshot
            .queued_prompts_for_agent(reviewer.id())
            .expect("reviewer queue should exist")
            .front()
            .map(|prompt| prompt.prompt()),
        Some("Message from @router:\n\nThen report whether the package is private.")
    );
    assert_eq!(
        snapshot
            .active_prompt_for_agent(sender.id())
            .map(|prompt| prompt.prompt()),
        Some("Message from @reviewer:\n\nThe review has started.")
    );
}

#[tokio::test]
async fn runtime_mcp_agent_message_rejects_unknown_and_self_targets_without_prompt_state() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, sender) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-agent-message-errors",
            "worktree-agent-message-errors",
        ))
        .expect("session should be created");
    let sender_run = launch_test_provider(
        &mut app,
        session.id(),
        sender.id(),
        "dev-stub",
        "dev-stub",
        "sender-model",
    );
    let auth_token = sender_run
        .runtime_mcp_auth_token()
        .expect("sender run should expose runtime MCP auth")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    for agent in ["missing", sender.id()] {
        let result = router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &auth_token,
                crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
                serde_json::json!({
                    "agent": agent,
                    "message": "Do not dispatch this."
                }),
            )
            .await
            .expect("invalid target should return a structured tool failure");
        assert!(!result.ok, "{:?}", result.payload);
    }
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .expect("session should remain readable");
    assert!(snapshot.active_prompt_for_agent(sender.id()).is_none());
    assert!(snapshot
        .queued_prompts_for_agent(sender.id())
        .into_iter()
        .flatten()
        .next()
        .is_none());
}

#[tokio::test]
async fn runtime_mcp_metaagent_can_message_an_existing_session_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, metaagent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-metaagent-messaging",
            "worktree-metaagent-messaging",
        ))
        .expect("session should be created");
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .expect("agent should enter Meta mode");
    let worker = spawn_test_agent(&mut app, session.id(), "worker", "dev-stub");
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    assert!(router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&auth_token)
        .iter()
        .any(|spec| { spec.name == crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL }));
    let result = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::SEND_AGENT_MESSAGE_TOOL,
            serde_json::json!({
                "agent": "worker",
                "message": "Perform the delegated check."
            }),
        )
        .await
        .expect("Meta agent message should dispatch");
    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(result.payload["status"], "started");
}
