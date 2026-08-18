use super::*;

#[tokio::test]
async fn local_destroy_agent_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, agent_id, provider_run_id, terminal_stream, cursor_key) = {
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
        let external_sessions = app_locked.external_provider_session_index_store();
        external_sessions.upsert(external_provider_session_record(
            "codex",
            "destroyed-agent-thread",
            30,
        ));
        external_sessions.mark_attached(
            "codex:destroyed-agent-thread",
            session.id(),
            extra_agent.id(),
        );
        let cursor_key = crate::app::AttachedProviderTranscriptCursorKey::new(
            session.id(),
            extra_agent.id(),
            "codex",
            "default",
            "destroyed-agent-thread",
        );
        app_locked.attached_provider_transcript_cursor_store().set(
            cursor_key.clone(),
            crate::provider::ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-before-destroy".to_string()),
                ..crate::provider::ExternalProviderObservedCursor::default()
            },
        );
        assert_ne!(default_agent.id(), extra_agent.id());
        (
            session.id().to_string(),
            extra_agent.id().to_string(),
            provider_run.id().to_string(),
            app_locked.terminal_stream_store(),
            cursor_key,
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
    let page = _locked_app.external_provider_session_index_store().list(
        &ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        },
    );
    assert_eq!(
        page.sessions
            .iter()
            .map(|session| session.external_session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["codex:default:destroyed-agent-thread"],
        "destroying an attached agent should return its provider thread to the unattached list"
    );
    assert!(page.sessions[0].is_attachable_to_chariox());
    assert_eq!(
        _locked_app
            .attached_provider_transcript_cursor_store()
            .get(&cursor_key),
        crate::provider::ExternalProviderObservedCursor::default(),
        "destroying an attached agent should prune its Chariox-owned provider transcript cursor"
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
