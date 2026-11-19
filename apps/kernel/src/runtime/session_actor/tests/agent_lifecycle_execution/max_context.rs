use super::*;

async fn runtime_for_new_room(
    name: &str,
) -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    SessionRuntime,
    String,
) {
    runtime_for_new_room_with_config(name, DaemonConfig::for_tests()).await
}

async fn runtime_for_new_room_with_config(
    name: &str,
    config: DaemonConfig,
) -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    SessionRuntime,
    String,
) {
    let worktree = crate::test_support::TestWorktree::new(name);
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(worktree.session_request())
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
    (worktree, app, runtime, session_id)
}

/// Lets go of a kernel so the next `bootstrap` can take over its durable state.
async fn release(runtime: SessionRuntime, app: Arc<Mutex<DaemonApp>>) {
    let released = Arc::downgrade(&app);
    drop(runtime);
    drop(app);
    tokio::time::timeout(Duration::from_secs(1), async {
        while released.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("session lane should release the kernel");
}

async fn spawn(
    runtime: &SessionRuntime,
    worktree: &crate::test_support::TestWorktree,
    session_id: &str,
    provider: &str,
    max_context_tokens: Option<u64>,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        initial_task: None,
        peer_spawn: None,
        max_context_tokens,
        no_subscribe: false,
        account_profile: None,
        session_id: session_id.to_string(),
        alias: None,
        provider: Some(provider.to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some(worktree.path().display().to_string()),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let command = KernelCommand::from_local_request("max-context-spawn", None, None, &request);
    match runtime.dispatch_session_command(command, request).await? {
        LocalDaemonResponse::AgentSpawned { agent, .. } => Ok(agent),
        other => panic!("unexpected response {other:?}"),
    }
}

async fn configure(
    runtime: &SessionRuntime,
    session_id: &str,
    agent_id: &str,
    max_context_tokens: Option<u64>,
    clear: bool,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    configure_with_mode(
        runtime,
        session_id,
        agent_id,
        max_context_tokens,
        clear,
        None,
    )
    .await
}

async fn configure_with_mode(
    runtime: &SessionRuntime,
    session_id: &str,
    agent_id: &str,
    max_context_tokens: Option<u64>,
    clear: bool,
    execution_mode: Option<crate::provider::AgentExecutionMode>,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
        max_context_tokens,
        clear_max_context_tokens: clear,
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        execution_mode,
        clear_execution_mode: false,
        permission_level: None,
        clear_permission_level: false,
        workspace_id: None,
        clear_workspace_id: false,
        worktree_id: None,
        clear_worktree_id: false,
        usage_limit: None,
        reason: None,
    });
    let command = KernelCommand::from_local_request("max-context-config", None, None, &request);
    match runtime.dispatch_session_command(command, request).await? {
        LocalDaemonResponse::AgentConfigUpdated { agent, .. } => Ok(agent),
        other => panic!("unexpected response {other:?}"),
    }
}

#[tokio::test]
async fn an_agent_keeps_the_max_context_it_was_spawned_with() {
    let (worktree, _app, runtime, session_id) = runtime_for_new_room("max-context-spawn").await;

    let with = spawn(&runtime, &worktree, &session_id, "claude", Some(200_000))
        .await
        .expect("a Claude agent may start with a max context");
    let without = spawn(&runtime, &worktree, &session_id, "claude", None)
        .await
        .expect("max context stays optional");

    assert_eq!(with.max_context_tokens(), Some(200_000));
    assert_eq!(without.max_context_tokens(), None);
}

#[tokio::test]
async fn a_spawn_with_a_max_context_the_provider_cannot_honour_is_refused() {
    let (worktree, _app, runtime, session_id) = runtime_for_new_room("max-context-refused").await;

    let too_small = spawn(&runtime, &worktree, &session_id, "claude", Some(50_000))
        .await
        .expect_err("Claude cannot compact below 100000 tokens");
    assert!(
        too_small.to_string().contains("100000 and 1000000"),
        "{too_small}"
    );
    let unsupported = spawn(&runtime, &worktree, &session_id, "dev-stub", Some(200_000))
        .await
        .expect_err("a provider without the setting must refuse it");
    assert!(
        unsupported.to_string().contains("not supported"),
        "{unsupported}"
    );
}

#[tokio::test]
async fn update_agent_config_sets_and_clears_the_max_context() {
    let (worktree, _app, runtime, session_id) = runtime_for_new_room("max-context-config").await;
    let agent = spawn(&runtime, &worktree, &session_id, "claude", None)
        .await
        .expect("agent should spawn");

    let set = configure(&runtime, &session_id, agent.id(), Some(300_000), false)
        .await
        .expect("an in-range value is accepted");
    assert_eq!(set.max_context_tokens(), Some(300_000));
    let refused = configure(&runtime, &session_id, agent.id(), Some(2_000_000), false)
        .await
        .expect_err("a value above Claude's range is refused");
    assert!(
        refused.to_string().contains("100000 and 1000000"),
        "{refused}"
    );
    let cleared = configure(&runtime, &session_id, agent.id(), None, true)
        .await
        .expect("clearing returns to the provider default");
    assert_eq!(cleared.max_context_tokens(), None);
}

#[tokio::test]
async fn a_set_or_cleared_max_context_survives_a_kernel_restart() {
    let config = DaemonConfig::for_tests();
    let (worktree, app, runtime, session_id) =
        runtime_for_new_room_with_config("max-context-restart", config.clone()).await;
    let agent = spawn(&runtime, &worktree, &session_id, "claude", None)
        .await
        .expect("agent should spawn");

    configure(&runtime, &session_id, agent.id(), Some(300_000), false)
        .await
        .expect("an in-range value is accepted");
    release(runtime, app).await;
    let restored = DaemonApp::bootstrap(config.clone()).expect("kernel should restore");
    assert_eq!(
        restored
            .agents()
            .get_agent(agent.id())
            .expect("agent should restore")
            .max_context_tokens(),
        Some(300_000)
    );

    let app = Arc::new(Mutex::new(restored));
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        app.lock().await.terminal_stream_store(),
    );
    configure(&runtime, &session_id, agent.id(), None, true)
        .await
        .expect("clearing returns to the provider default");
    release(runtime, app).await;
    let restored = DaemonApp::bootstrap(config).expect("kernel should restore again");
    assert_eq!(
        restored
            .agents()
            .get_agent(agent.id())
            .expect("agent should restore")
            .max_context_tokens(),
        None
    );
}

#[tokio::test]
async fn a_refused_max_context_leaves_the_rest_of_the_request_unapplied() {
    let (worktree, app, runtime, session_id) = runtime_for_new_room("max-context-atomic").await;
    let agent = spawn(&runtime, &worktree, &session_id, "claude", Some(200_000))
        .await
        .expect("agent should spawn");

    configure_with_mode(
        &runtime,
        &session_id,
        agent.id(),
        Some(50_000),
        false,
        Some(crate::provider::AgentExecutionMode::Plan),
    )
    .await
    .expect_err("Claude cannot compact below 100000 tokens");

    let kept = app
        .lock()
        .await
        .agents()
        .get_agent(agent.id())
        .expect("agent should exist");
    assert_eq!(kept.execution_mode_override(), None);
    assert_eq!(kept.max_context_tokens(), Some(200_000));
}

async fn switch_provider(
    runtime: &SessionRuntime,
    session_id: &str,
    agent_id: &str,
    provider: &str,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    let request = LocalDaemonRequest::UpdateAgentProfile(crate::local::UpdateAgentProfileRequest {
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        provider: Some(provider.to_string()),
        account_profile: None,
        model: None,
        effort: None,
        clear_effort: false,
        reason: None,
    });
    let command = KernelCommand::from_local_request("max-context-switch", None, None, &request);
    match runtime.dispatch_session_command(command, request).await? {
        LocalDaemonResponse::AgentProfileUpdated { agent, .. } => Ok(agent),
        other => panic!("unexpected response {other:?}"),
    }
}

#[tokio::test]
async fn a_provider_switch_refuses_a_max_context_the_new_provider_cannot_honour() {
    let (worktree, _app, runtime, session_id) = runtime_for_new_room("max-context-switch").await;

    let claude = spawn(&runtime, &worktree, &session_id, "claude", Some(200_000))
        .await
        .expect("a Claude agent may start with a max context");
    let to_opencode = switch_provider(&runtime, &session_id, claude.id(), "opencode")
        .await
        .expect_err("OpenCode ignores the setting, so the switch must be refused");
    assert!(
        to_opencode.to_string().contains("clear the setting"),
        "{to_opencode}"
    );

    let codex = spawn(&runtime, &worktree, &session_id, "codex", Some(2_000_000))
        .await
        .expect("Codex accepts a value outside Claude's range");
    let to_claude = switch_provider(&runtime, &session_id, codex.id(), "claude")
        .await
        .expect_err("Claude cannot honour 2000000 tokens");
    assert!(
        to_claude.to_string().contains("100000 and 1000000"),
        "{to_claude}"
    );
}
