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
        account_profile: None,
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
                account_profile: None,
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
                account_profile: None,
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
async fn local_spawn_agents_batch_normalizes_local_kernel_ref_to_bulk_owned_path() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, daemon_id, terminal_stream, durable_state_store) = {
        let mut app_locked = app.lock().await;
        let daemon_id = app_locked.config().daemon_id.clone();
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (
            session.id().to_string(),
            daemon_id,
            app_locked.terminal_stream_store(),
            app_locked.durable_state_store(),
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
                account_profile: None,
                alias: Some("local-kernel-ref-1".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: Some("worktree".to_string()),
                kernel_ref: Some(daemon_id.clone()),
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            },
            crate::local::SpawnAgentsRequestItem {
                account_profile: None,
                alias: Some("local-kernel-ref-2".to_string()),
                provider: Some("dev-stub".to_string()),
                model: Some("default".to_string()),
                effort: None,
                execution_mode: None,
                permission_level: None,
                worktree_id: Some("worktree".to_string()),
                kernel_ref: Some(daemon_id),
                slice_ref: None,
                worktree_placement: None,
                metaagent: false,
            },
        ],
    });
    let command = KernelCommand::from_local_request(
        "owned-local-kernel-ref-agent-batch",
        None,
        None,
        &request,
    );
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("local kernel-ref batch spawn should not wait for the app lock")
    .expect("local kernel-ref batch spawn should succeed");

    let LocalDaemonResponse::AgentsSpawned { agents } = response else {
        panic!("unexpected response");
    };
    assert_eq!(agents.len(), 2);
    let durable_events = durable_state_store
        .load_events_after(0)
        .expect("durable state events should load");
    assert_eq!(
        durable_events
            .iter()
            .filter(|event| event.kind == "agents.created")
            .count(),
        1,
        "local kernel-ref batch should still persist one compact agents.created event"
    );
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
                account_profile: None,
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
        let app_dropped = Arc::downgrade(&app);
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
                    account_profile: None,
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
                    account_profile: None,
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
        let restored_ids = (
            session_id,
            agents[0].id().to_string(),
            agents[1].id().to_string(),
        );
        drop(runtime);
        drop(app);
        tokio::time::timeout(Duration::from_secs(1), async {
            while app_dropped.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session lane should release the first daemon");
        restored_ids
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
