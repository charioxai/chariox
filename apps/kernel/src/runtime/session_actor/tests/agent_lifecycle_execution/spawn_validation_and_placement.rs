use super::*;

#[tokio::test]
async fn mixed_spawn_agents_preserves_response_order_and_final_focus() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream, local_kernel_ref) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (
            session.id().to_string(),
            app_locked.terminal_stream_store(),
            app_locked.config().daemon_id.clone(),
        )
    };
    let session_projection = SessionStateProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::SpawnAgents(crate::local::SpawnAgentsRequest {
        session_id: session_id.clone(),
        agents: vec![
            crate::local::SpawnAgentsRequestItem {
                alias: Some("mixed-owned".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: Some("worktree".to_string()),
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            },
            crate::local::SpawnAgentsRequestItem {
                alias: Some("mixed-local-kernel-ref".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: Some("worktree".to_string()),
                kernel_ref: Some(local_kernel_ref),
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            },
        ],
    });
    let command = KernelCommand::from_local_request("mixed-agent-batch", None, None, &request);
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("mixed agent batch spawn should succeed");

    let LocalDaemonResponse::AgentsSpawned { agents } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0].alias(), Some("mixed-owned"));
    assert_eq!(agents[1].alias(), Some("mixed-local-kernel-ref"));
    let projected = session_projection
        .get(&session_id)
        .expect("batch spawn should refresh session projection");
    assert_eq!(projected.focused_agent_id(), Some(agents[1].id()));
    assert_eq!(
        projected
            .agents()
            .iter()
            .filter(|agent| agent.state() == crate::agent::AgentState::Focused)
            .count(),
        1
    );
}

#[tokio::test]
async fn local_spawn_agents_batch_rejects_duplicate_aliases_without_partial_create() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream, initial_agent_count) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let initial_agent_count = app_locked.agents().get_session_agents(session.id()).len();
        (
            session.id().to_string(),
            app_locked.terminal_stream_store(),
            initial_agent_count,
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

    let request = LocalDaemonRequest::SpawnAgents(crate::local::SpawnAgentsRequest {
        session_id: session_id.clone(),
        agents: vec![
            crate::local::SpawnAgentsRequestItem {
                alias: Some("duplicate".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: Some("worktree".to_string()),
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            },
            crate::local::SpawnAgentsRequestItem {
                alias: Some(" duplicate ".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: Some("worktree".to_string()),
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            },
        ],
    });
    let command = KernelCommand::from_local_request("duplicate-agent-batch", None, None, &request);
    let error = runtime
        .dispatch_session_command(command, request)
        .await
        .expect_err("duplicate aliases should reject the whole batch");
    assert!(matches!(
        error,
        DaemonError::AgentAliasConflict {
            alias,
            ..
        } if alias == " duplicate "
    ));

    let app_locked = app.lock().await;
    assert_eq!(
        app_locked.agents().get_session_agents(&session_id).len(),
        initial_agent_count,
        "rejected batch must not create a partial prefix"
    );
}

#[tokio::test]
async fn local_spawn_agents_batch_recalculates_layout_and_focus_once() {
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
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::SpawnAgents(crate::local::SpawnAgentsRequest {
        session_id: session_id.clone(),
        agents: (0..50)
            .map(|index| crate::local::SpawnAgentsRequestItem {
                alias: Some(format!("bulk-{index}")),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: Some("worktree".to_string()),
                kernel_ref: None,
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            })
            .collect(),
    });
    let command = KernelCommand::from_local_request("layout-agent-batch", None, None, &request);
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("agent batch spawn should succeed");
    let LocalDaemonResponse::AgentsSpawned { agents } = response else {
        panic!("unexpected response");
    };
    let focused_agent = agents.last().expect("batch should create agents");
    let projected = session_projection
        .get(&session_id)
        .expect("batch spawn should refresh session projection");
    assert_eq!(projected.focused_agent_id(), Some(focused_agent.id()));
    assert_eq!(projected.agents().len(), 51);
    assert_eq!(
        projected
            .agents()
            .iter()
            .filter(|agent| agent.state() == crate::agent::AgentState::Focused)
            .count(),
        1
    );
}

#[tokio::test]
async fn local_spawn_agent_creates_requested_git_worktree_in_kernel() {
    let repo = temp_git_repo("agent-placement");
    let target = repo.with_file_name(format!(
        "{}-feature",
        repo.file_name().and_then(|name| name.to_str()).unwrap()
    ));
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new(
                "workspace",
                repo.display().to_string(),
            ))
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
        alias: Some("feature-worker".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: Some(crate::agent::GitWorktreePlacement {
            target_directory: Some(target.display().to_string()),
            branch: Some("feature/agent-placement".to_string()),
            from_ref: Some("HEAD".to_string()),
        }),
        metaagent: false,
    });
    let command = KernelCommand::from_local_request("local-agent-worktree", None, None, &request);
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("agent spawn should succeed");

    let LocalDaemonResponse::AgentSpawned { agent } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agent.session_id(), session_id);
    assert_eq!(
        agent.worktree_id(),
        Some(target.display().to_string().as_str())
    );
    assert!(target.is_dir());
    assert_eq!(
        git_output(&target, &["branch", "--show-current"]),
        "feature/agent-placement"
    );
    let _ = std::fs::remove_dir_all(&target);
    let _ = std::fs::remove_dir_all(&repo);
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
        metaagent: false,
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
async fn local_spawn_agent_rejects_deprecated_metaagent_creation() {
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
        alias: Some("meta".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some("worktree".to_string()),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: true,
    });
    let command = KernelCommand::from_local_request("spawn-meta-deprecated", None, None, &request);
    let error = runtime
        .dispatch_session_command(command, request)
        .await
        .expect_err("separate metaagent spawn should be rejected");
    assert!(
        error
            .to_string()
            .contains("send `/meta <task>` to a regular agent"),
        "unexpected error: {error}"
    );
}
