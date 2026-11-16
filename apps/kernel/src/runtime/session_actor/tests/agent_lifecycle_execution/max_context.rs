use super::*;

async fn runtime_for_new_room(
    name: &str,
) -> (
    crate::test_support::TestWorktree,
    Arc<Mutex<DaemonApp>>,
    SessionRuntime,
    String,
) {
    let worktree = crate::test_support::TestWorktree::new(name);
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
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

async fn spawn(
    runtime: &SessionRuntime,
    worktree: &crate::test_support::TestWorktree,
    session_id: &str,
    provider: &str,
    max_context_tokens: Option<u64>,
) -> Result<crate::agent::AgentInstance, DaemonError> {
    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
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
    let request = LocalDaemonRequest::UpdateAgentConfig(UpdateAgentConfigRequest {
        max_context_tokens,
        clear_max_context_tokens: clear,
        session_id: session_id.to_string(),
        agent_id: agent_id.to_string(),
        execution_mode: None,
        clear_execution_mode: false,
        permission_level: None,
        clear_permission_level: false,
        workspace_id: None,
        clear_workspace_id: false,
        worktree_id: None,
        clear_worktree_id: false,
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
