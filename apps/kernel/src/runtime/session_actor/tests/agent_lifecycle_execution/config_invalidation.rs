use super::*;

#[tokio::test]
async fn update_agent_config_invalidates_only_that_agents_idle_provider_run() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, first_agent_id, second_agent_id, first_run_id, second_run_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let second_agent = crate::app::KernelSessionService::new(&mut app_locked)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("agent-b")
                    .with_worktree("worktree"),
            )
            .expect("second agent should be created");
        let first_run =
            launch_dev_stub_provider(&mut app_locked, session.id(), first_agent.id(), "sonnet");
        let second_run =
            launch_dev_stub_provider(&mut app_locked, session.id(), second_agent.id(), "opus");
        (
            session.id().to_string(),
            first_agent.id().to_string(),
            second_agent.id().to_string(),
            first_run.id().to_string(),
            second_run.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
        session_id: session_id.clone(),
        agent_id: first_agent_id.clone(),
        execution_mode: Some(AgentExecutionMode::Plan),
        clear_execution_mode: false,
        permission_level: None,
        clear_permission_level: false,
        workspace_id: None,
        clear_workspace_id: false,
        worktree_id: None,
        clear_worktree_id: false,
    });
    let command =
        KernelCommand::from_local_request("owned-agent-config-update", None, None, &request);
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("agent config update should succeed");

    let LocalDaemonResponse::AgentConfigUpdated { agent, session } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agent.id(), first_agent_id);
    assert_eq!(
        agent.execution_mode_override(),
        Some(AgentExecutionMode::Plan)
    );
    let second_agent = session
        .agents()
        .iter()
        .find(|agent| agent.id() == second_agent_id)
        .expect("second agent should remain in session");
    assert_eq!(second_agent.execution_mode_override(), None);

    let app_locked = app.lock().await;
    assert_eq!(
        app_locked
            .providers()
            .get_run(&first_run_id)
            .expect("first run should still be recorded")
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    assert_eq!(
        app_locked
            .providers()
            .get_run(&second_run_id)
            .expect("second run should still be recorded")
            .state(),
        crate::provider::ProviderRunState::Running
    );
}

#[tokio::test]
async fn update_agent_config_keeps_turn_scoped_provider_run_alive() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, agent_id, provider_run_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(
                CreateSessionRequest::new("workspace", "worktree")
                    .with_agent_defaults(SessionAgentDefaults::new("managed-dev-stub")),
            )
            .expect("session should be created");
        let run = launch_provider_for_adapter(
            &mut app_locked,
            session.id(),
            agent.id(),
            "managed-dev-stub",
            "sonnet",
        );
        (
            session.id().to_string(),
            agent.id().to_string(),
            run.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        execution_mode: Some(AgentExecutionMode::Plan),
        clear_execution_mode: false,
        permission_level: Some(AgentPermissionLevel::Required),
        clear_permission_level: false,
        workspace_id: None,
        clear_workspace_id: false,
        worktree_id: None,
        clear_worktree_id: false,
    });
    let command =
        KernelCommand::from_local_request("owned-turn-scoped-config-update", None, None, &request);
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("agent config update should succeed");

    let LocalDaemonResponse::AgentConfigUpdated { agent, .. } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agent.id(), agent_id);
    assert_eq!(
        agent.execution_mode_override(),
        Some(AgentExecutionMode::Plan)
    );

    let app_locked = app.lock().await;
    let provider_run = app_locked
        .providers()
        .get_run(&provider_run_id)
        .expect("provider run should still be recorded");
    assert_eq!(
        provider_run.state(),
        crate::provider::ProviderRunState::Running
    );
    assert_eq!(provider_run.execution_mode(), AgentExecutionMode::Plan);
    assert_eq!(
        provider_run.permission_level(),
        AgentPermissionLevel::Required
    );
}
