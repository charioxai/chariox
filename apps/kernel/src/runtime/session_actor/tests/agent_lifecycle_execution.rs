use super::*;

#[tokio::test]
async fn local_spawn_agent_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (session.id().to_string(), app_locked.terminal_stream_store())
    };
    let session_projection = SessionStateProjectionStore::default();
    let agent_runtime_projection = AgentRuntimeProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        session_id: session_id.clone(),
        alias: Some("owned-agent".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some("worktree".to_string()),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
    });
    let command =
        KernelCommand::from_local_request("owned-local-agent-spawn", None, None, &request);
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("owned local agent spawn should not wait for the app lock")
    .expect("agent spawn should succeed");

    let LocalDaemonResponse::AgentSpawned { agent } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agent.session_id(), session_id);
    assert_eq!(agent.alias(), Some("owned-agent"));
    let projected = session_projection
        .get(&session_id)
        .expect("spawn should refresh session projection");
    assert_eq!(projected.focused_agent_id(), Some(agent.id()));
    assert!(
        agent_runtime_projection
            .get(agent.id())
            .filter(|projection| projection.session_id == session_id)
            .is_some(),
        "spawn should refresh agent-runtime projection"
    );
}

#[tokio::test]
async fn local_spawn_agent_inherits_session_agent_defaults_when_omitted() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let defaults = SessionAgentDefaults::new("opencode")
            .with_model("moonshotai/kimi-k2")
            .with_effort("high")
            .with_execution_mode(AgentExecutionMode::Plan)
            .with_permission_level(AgentPermissionLevel::Required);
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(
                CreateSessionRequest::new("workspace", "worktree").with_agent_defaults(defaults),
            )
            .expect("session should be created");
        (session.id().to_string(), app_locked.terminal_stream_store())
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        session_id: session_id.clone(),
        alias: Some("inherited-agent".to_string()),
        provider: None,
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some("worktree".to_string()),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
    });
    let command =
        KernelCommand::from_local_request("owned-default-agent-spawn", None, None, &request);
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("agent spawn should succeed");

    let LocalDaemonResponse::AgentSpawned { agent } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agent.session_id(), session_id);
    assert_eq!(agent.alias(), Some("inherited-agent"));
    assert_eq!(agent.provider(), "opencode");
    assert_eq!(agent.model(), Some("moonshotai/kimi-k2"));
    assert_eq!(agent.effort(), Some("high"));
    assert_eq!(
        agent.execution_mode_override(),
        Some(AgentExecutionMode::Plan)
    );
    assert_eq!(
        agent.permission_level_override(),
        Some(AgentPermissionLevel::Required)
    );
}

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

#[tokio::test]
async fn local_destroy_agent_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, agent_id, provider_run_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let extra_agent = crate::app::KernelSessionService::new(&mut app_locked)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("destroy-me")
                    .with_worktree("worktree"),
            )
            .expect("extra agent should be created");
        let provider_run =
            launch_dev_stub_provider(&mut app_locked, session.id(), extra_agent.id(), "opus");
        assert_ne!(default_agent.id(), extra_agent.id());
        (
            session.id().to_string(),
            extra_agent.id().to_string(),
            provider_run.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let session_projection = SessionStateProjectionStore::default();
    let agent_runtime_projection = AgentRuntimeProjectionStore::default();
    let state = owned_runtime_state(&app).await;
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        state.clone(),
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        terminal_stream,
    );
    let session = state
        .session_snapshot(&session_id)
        .await
        .expect("session snapshot should be available");
    session_projection.update(session.clone());
    agent_runtime_projection.update_session(&session);
    assert!(
        agent_runtime_projection.get(&agent_id).is_some(),
        "agent projection should be warmed before destroy"
    );

    let request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
    });
    let command =
        KernelCommand::from_local_request("owned-local-agent-destroy", None, None, &request);
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("owned local agent destroy should not wait for the app lock")
    .expect("agent destroy should succeed");

    let LocalDaemonResponse::AgentDestroyed { agent } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agent.id(), agent_id);
    let projected = session_projection
        .get(&session_id)
        .expect("destroy should refresh session projection");
    assert!(
        projected
            .agents()
            .iter()
            .all(|agent| agent.id() != agent_id),
        "destroyed agent should be removed from session projection"
    );
    assert!(
        agent_runtime_projection.get(&agent_id).is_none(),
        "destroyed agent should be removed from agent-runtime projection"
    );
    let provider_run = _locked_app
        .providers()
        .get_run(&provider_run_id)
        .expect("destroyed agent provider run should still be addressable");
    assert_eq!(
        provider_run.state(),
        crate::provider::ProviderRunState::Ended,
        "destroying an agent should end its provider run"
    );
}

#[tokio::test]
async fn local_destroy_agent_repairs_canonical_stale_focus() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, destroyed_agent_id, remaining_agent_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let extra_agent = crate::app::KernelSessionService::new(&mut app_locked)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("agent-b")
                    .with_worktree("worktree"),
            )
            .expect("extra agent should be created");
        app_locked
            .focus_agent(session.id(), default_agent.id())
            .expect("default agent should become canonical focus");
        app_locked
            .agents_mut()
            .set_agent_state(default_agent.id(), AgentState::Idle)
            .expect("test should be able to force stale agent state");
        assert_eq!(
            app_locked
                .sessions()
                .get_session(session.id())
                .expect("session should remain")
                .focused_agent_id(),
            Some(default_agent.id()),
            "session focus remains canonical even when agent state diverges"
        );
        (
            session.id().to_string(),
            default_agent.id().to_string(),
            extra_agent.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let session_projection = SessionStateProjectionStore::default();
    let agent_runtime_projection = AgentRuntimeProjectionStore::default();
    let state = owned_runtime_state(&app).await;
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        state.clone(),
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection,
        terminal_stream,
    );
    let session = state
        .session_snapshot(&session_id)
        .await
        .expect("session snapshot should be available");
    session_projection.update(session);

    let request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
        session_id: session_id.clone(),
        agent_id: destroyed_agent_id.clone(),
    });
    let command = KernelCommand::from_local_request(
        "owned-local-agent-destroy-stale-focus",
        None,
        None,
        &request,
    );

    runtime
        .dispatch_session_command(command, request)
        .await
        .expect("agent destroy should succeed");

    let projected = session_projection
        .get(&session_id)
        .expect("destroy should refresh session projection");
    assert!(
        projected
            .agents()
            .iter()
            .all(|agent| agent.id() != destroyed_agent_id),
        "destroyed agent should be removed from session projection"
    );
    assert_eq!(
        projected.focused_agent_id(),
        Some(remaining_agent_id.as_str()),
        "destroying the canonical focused agent should focus the first remaining agent"
    );
}
