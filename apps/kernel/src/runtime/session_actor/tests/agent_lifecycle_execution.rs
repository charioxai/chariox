use super::*;

#[tokio::test]
async fn local_spawn_agent_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream, durable_state_store) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (
            session.id().to_string(),
            app_locked.terminal_stream_store(),
            app_locked.durable_state_store(),
        )
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
        metaagent: false,
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
    let durable_events = durable_state_store
        .load_events_after(0)
        .expect("durable state events should load");
    assert!(
        durable_events
            .iter()
            .all(|event| event.kind != "agents.created"),
        "single-agent spawn should preserve the agent.created durable event shape"
    );
    assert!(
        durable_events.iter().any(|event| {
            event.kind == "agent.created"
                && event.subject_id.as_deref() == Some(agent.id())
                && event
                    .payload
                    .get("agent")
                    .and_then(|agent| agent.get("id"))
                    .and_then(|id| id.as_str())
                    == Some(agent.id())
        }),
        "owned runtime spawn-agent path should persist the agent.created durable event"
    );
}

#[tokio::test]
async fn local_spawn_agents_batch_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream, durable_state_store) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (
            session.id().to_string(),
            app_locked.terminal_stream_store(),
            app_locked.durable_state_store(),
        )
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

    let request = LocalDaemonRequest::SpawnAgents(crate::local::SpawnAgentsRequest {
        session_id: session_id.clone(),
        agents: vec![
            crate::local::SpawnAgentsRequestItem {
                alias: Some("owned-agent-1".to_string()),
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
                alias: Some("owned-agent-2".to_string()),
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
    let command =
        KernelCommand::from_local_request("owned-local-agent-batch-spawn", None, None, &request);
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("owned local batch spawn should not wait for the app lock")
    .expect("agent batch spawn should succeed");

    let LocalDaemonResponse::AgentsSpawned { agents } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0].session_id(), session_id);
    assert_eq!(agents[0].alias(), Some("owned-agent-1"));
    assert_eq!(agents[1].alias(), Some("owned-agent-2"));
    let durable_events = durable_state_store
        .load_events_after(0)
        .expect("durable state events should load");
    let batch_events = durable_events
        .iter()
        .filter(|event| event.kind == "agents.created")
        .collect::<Vec<_>>();
    assert_eq!(
        batch_events.len(),
        1,
        "owned runtime batch spawn should persist one compact agents.created event"
    );
    assert_eq!(
        batch_events[0].subject_id.as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(
        batch_events[0]
            .payload
            .get("agents")
            .and_then(|agents| agents.as_array())
            .map(|agents| agents.len()),
        Some(2)
    );
    let projected = session_projection
        .get(&session_id)
        .expect("batch spawn should refresh session projection");
    assert_eq!(projected.focused_agent_id(), Some(agents[1].id()));
    for agent in &agents {
        assert!(
            agent_runtime_projection
                .get(agent.id())
                .filter(|projection| projection.session_id == session_id)
                .is_some(),
            "batch spawn should refresh agent-runtime projection"
        );
    }
}

#[tokio::test]
async fn local_spawn_agents_batch_emits_one_compact_metaagent_lifecycle_event() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, metaagent_id, terminal_stream, metaagent_events) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let metaagent = crate::app::KernelSessionService::new(&mut app_locked)
            .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
            .expect("metaagent should spawn");
        let metaagent = app_locked
            .agents_mut()
            .activate_agent_meta_mode(metaagent.id(), None)
            .expect("metaagent mode should activate");
        (
            session.id().to_string(),
            metaagent.id().to_string(),
            app_locked.terminal_stream_store(),
            app_locked.metaagent_event_store(),
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
        agents: (0..3)
            .map(|index| crate::local::SpawnAgentsRequestItem {
                alias: Some(format!("compact-batch-{index}")),
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
    let command =
        KernelCommand::from_local_request("compact-lifecycle-agent-batch", None, None, &request);
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("agent batch spawn should succeed");
    let LocalDaemonResponse::AgentsSpawned { agents } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agents.len(), 3);

    let compact_events = metaagent_events.list(&metaagent_id, Some("agents.spawned"), None, 10);
    assert_eq!(
        compact_events.len(),
        1,
        "batch spawn should emit one compact lifecycle event"
    );
    assert_eq!(
        compact_events[0]
            .detail
            .get("agent_count")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
    assert_eq!(
        compact_events[0]
            .detail
            .get("agents")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(3)
    );
    assert!(
        metaagent_events
            .list(&metaagent_id, Some("agent.spawned"), None, 10)
            .is_empty(),
        "batch spawn should not emit one lifecycle event per agent"
    );
}

#[tokio::test]
async fn local_spawn_agents_batch_restores_from_compact_durable_event() {
    let config = DaemonConfig::for_tests();
    let (session_id, first_agent_id, second_agent_id) = {
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(config.clone()).expect("daemon should boot"),
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
        let request = LocalDaemonRequest::SpawnAgents(crate::local::SpawnAgentsRequest {
            session_id: session_id.clone(),
            agents: vec![
                crate::local::SpawnAgentsRequestItem {
                    alias: Some("restored-bulk-1".to_string()),
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
                    alias: Some("restored-bulk-2".to_string()),
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
        let command =
            KernelCommand::from_local_request("restore-agent-batch", None, None, &request);
        let response = runtime
            .dispatch_session_command(command, request)
            .await
            .expect("agent batch spawn should succeed");
        let LocalDaemonResponse::AgentsSpawned { agents } = response else {
            panic!("unexpected response");
        };
        (
            session_id,
            agents[0].id().to_string(),
            agents[1].id().to_string(),
        )
    };

    let app = DaemonApp::bootstrap(config).expect("daemon should restore");
    let restored_session = app
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    assert_eq!(restored_session.agents().len(), 3);
    assert_eq!(
        app.agents()
            .get_agent(&first_agent_id)
            .expect("first batch agent should restore")
            .alias(),
        Some("restored-bulk-1")
    );
    assert_eq!(
        app.agents()
            .get_agent(&second_agent_id)
            .expect("second batch agent should restore")
            .alias(),
        Some("restored-bulk-2")
    );
}

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

fn temp_git_repo(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "arroba-{label}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "tests@example.invalid"]);
    run_git(&root, &["config", "user.name", "Arroba Tests"]);
    std::fs::write(root.join("README.md"), "worktree placement\n")
        .expect("fixture file should be written");
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-m", "initial"]);
    root
}

fn git_output(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
