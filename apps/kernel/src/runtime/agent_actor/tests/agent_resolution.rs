use super::*;

#[tokio::test]
async fn submit_agent_resolution_uses_single_agent_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session projection fixture should be available");
    let agent_runtime_projection = AgentRuntimeProjectionStore::default();
    agent_runtime_projection.update_session(&session_snapshot);
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        agent_runtime_projection,
        PromptStateOwner::default(),
        crate::session::PromptIdAllocator::default(),
    );

    let _locked_app = app.lock().await;
    let resolved = timeout(
        Duration::from_millis(100),
        runtime.resolve_submit_agent_id(session.id(), None),
    )
    .await
    .expect("projection-backed resolution should not wait for the app lock")
    .expect("single projected agent should resolve");

    assert_eq!(resolved, agent.id());
}

#[tokio::test]
async fn active_prompt_resolution_uses_warmed_projection_for_no_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session projection fixture should be available");
    let session_projection = SessionStateProjectionStore::default();
    session_projection.update(session_snapshot);
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        AgentRuntimeProjectionStore::default(),
        PromptStateOwner::default(),
        crate::session::PromptIdAllocator::default(),
    );

    let _locked_app = app.lock().await;
    let error = timeout(
        Duration::from_millis(100),
        runtime.resolve_active_prompt_agent_id(session.id()),
    )
    .await
    .expect("projection-backed no-active resolution should not wait for the app lock")
    .expect_err("session has no active prompt");

    match error {
        DaemonError::NoActivePrompt { session_id } => assert_eq!(session_id, session.id()),
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn active_prompt_resolution_uses_prompt_owner_without_app_lock_when_session_mirror_is_stale()
{
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owner-route",
            "worktree-owner-route",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owner-route",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "owner-backed routing",
        PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should submit through owner");
    app.sessions_mut()
        .complete_active_prompt_only(session.id(), agent.id())
        .expect("test should clear only the compatibility mirror");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should still be available");
    assert!(
        session_snapshot
            .active_prompt_for_agent(agent.id())
            .is_none(),
        "compatibility session snapshot is intentionally stale"
    );

    let session_projection = SessionStateProjectionStore::default();
    session_projection.update(session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        AgentRuntimeProjectionStore::default(),
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let _locked_app = app.lock().await;
    let resolved = timeout(
        Duration::from_millis(100),
        runtime.resolve_active_prompt_agent_id(session.id()),
    )
    .await
    .expect("owner-backed active prompt resolution should not wait for the app lock")
    .expect("prompt owner should still know the active agent");

    assert_eq!(resolved, agent.id());
}

#[tokio::test]
async fn submit_agent_resolution_uses_warmed_list_for_missing_session_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let session_projection = SessionStateProjectionStore::default();
    session_projection.update_list(Vec::new());
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        AgentRuntimeProjectionStore::default(),
        PromptStateOwner::default(),
        crate::session::PromptIdAllocator::default(),
    );

    let _locked_app = app.lock().await;
    let error = timeout(
        Duration::from_millis(100),
        runtime.resolve_submit_agent_id("missing-session", None),
    )
    .await
    .expect("warmed missing session should not wait for the app lock")
    .expect_err("missing session should fail");

    match error {
        DaemonError::SessionNotFound { session_id } => {
            assert_eq!(session_id, "missing-session");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn submit_agent_resolution_uses_session_projection_for_invalid_target_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session projection fixture should be available");
    let session_projection = SessionStateProjectionStore::default();
    session_projection.update(session_snapshot);
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        AgentRuntimeProjectionStore::default(),
        PromptStateOwner::default(),
        crate::session::PromptIdAllocator::default(),
    );

    let _locked_app = app.lock().await;
    let error = timeout(
        Duration::from_millis(100),
        runtime.resolve_submit_agent_id(session.id(), Some("missing-agent")),
    )
    .await
    .expect("projected invalid target resolution should not wait for the app lock")
    .expect_err("invalid target agent should fail");

    match error {
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } => {
            assert_eq!(session_id, session.id());
            assert_eq!(agent_id, "missing-agent");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn active_prompt_resolution_uses_warmed_list_for_missing_session_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let session_projection = SessionStateProjectionStore::default();
    session_projection.update_list(Vec::new());
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        AgentRuntimeProjectionStore::default(),
        PromptStateOwner::default(),
        crate::session::PromptIdAllocator::default(),
    );

    let _locked_app = app.lock().await;
    let error = timeout(
        Duration::from_millis(100),
        runtime.resolve_active_prompt_agent_id("missing-session"),
    )
    .await
    .expect("warmed missing active-prompt session should not wait for the app lock")
    .expect_err("missing session should fail");

    match error {
        DaemonError::SessionNotFound { session_id } => {
            assert_eq!(session_id, "missing-session");
        }
        error => panic!("unexpected error: {error}"),
    }
}
