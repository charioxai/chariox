use super::*;

#[tokio::test]
async fn prompt_complete_uses_owned_runtime_state_without_app_lock_for_simple_local_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-complete",
            "worktree-owned-complete",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-complete",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "owned complete",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should submit through owner")
    else {
        panic!("first prompt should start");
    };
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = CompletePromptRequest {
        session_id: session_id.clone(),
    };
    let local_request = LocalDaemonRequest::CompletePrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-prompt-complete",
        None,
        None,
        &local_request,
    );
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_complete(&command, request),
    )
    .await
    .expect("owned local prompt completion should not wait for the app lock")
    .expect("prompt completion should succeed");

    let LocalDaemonResponse::PromptCompleted { completion } = response else {
        panic!("unexpected response");
    };
    assert_eq!(completion.completed.id(), prompt.id());
    assert_eq!(completion.completed.status(), PromptStatus::Completed);
    assert!(completion.started_next.is_none());
    let projected = session_projection
        .get(&session_id)
        .expect("completion should refresh session projection");
    assert!(
        projected.active_prompt_for_agent(&agent_id).is_none(),
        "completed prompt should be removed from session projection"
    );
    assert!(
        agent_runtime_projection
            .get(&agent_id)
            .filter(|projection| projection.active_prompt.is_none())
            .is_some(),
        "completed prompt should be removed from agent-runtime projection"
    );
}

#[tokio::test]
async fn prompt_cancel_uses_owned_runtime_state_without_app_lock_for_structured_local_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-cancel",
            "worktree-owned-cancel",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-cancel",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let provider_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "slow-structured",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("structured provider should launch");
    app.update_provider_run_projection(provider_run.clone());
    let prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "owned cancel",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should submit through owner")
    else {
        panic!("first prompt should start");
    };
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: None,
    };
    let local_request = LocalDaemonRequest::CancelActivePrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-prompt-cancel",
        None,
        None,
        &local_request,
    );
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_cancel(&command, request),
    )
    .await
    .expect("owned local prompt cancellation should not wait for the app lock")
    .expect("prompt cancellation should succeed");

    let LocalDaemonResponse::PromptCancelled { cancellation } = response else {
        panic!("unexpected response");
    };
    assert_eq!(cancellation.prompt.id(), prompt.id());
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
    assert!(cancellation.started_next.is_none());
    let projected = session_projection
        .get(&session_id)
        .expect("cancellation should refresh session projection");
    assert_eq!(
        projected
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.status()),
        Some(PromptStatus::Cancelling)
    );
    assert_eq!(
        agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .map(|prompt| prompt.status()),
        Some(PromptStatus::Cancelling),
        "cancelling prompt should refresh agent-runtime projection"
    );
}

#[tokio::test]
async fn prompt_complete_advances_queued_prompt_with_owned_runtime_state_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-advance",
            "worktree-owned-advance",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-advance",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let provider_run = app
        .launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "slow-structured",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("structured provider should launch");
    app.update_provider_run_projection(provider_run.clone());
    let first = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "first",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started { prompt: first } = app
        .prompt_owner_submit_prepared_prompt(session.id(), first, false)
        .expect("first prompt should submit")
    else {
        panic!("first prompt should start");
    };
    let second = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "second",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Queued { prompt: second } = app
        .prompt_owner_submit_prepared_prompt(session.id(), second, false)
        .expect("second prompt should queue")
    else {
        panic!("second prompt should queue");
    };
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = CompletePromptRequest {
        session_id: session_id.clone(),
    };
    let local_request = LocalDaemonRequest::CompletePrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-prompt-advance",
        None,
        None,
        &local_request,
    );
    let response = {
        let _locked_app = app.lock().await;
        timeout(
            Duration::from_millis(100),
            runtime.dispatch_prompt_complete(&command, request),
        )
        .await
        .expect("owned queued advancement should not wait for the app lock")
        .expect("prompt completion should succeed")
    };

    let LocalDaemonResponse::PromptCompleted { completion } = response else {
        panic!("unexpected response");
    };
    assert_eq!(completion.completed.id(), first.id());
    let started_next = completion
        .started_next
        .as_ref()
        .expect("queued prompt should promote");
    assert_ne!(started_next.id(), second.id());
    assert!(started_next.pending_prompt_id().is_none());
    assert_eq!(started_next.prompt(), second.prompt());
    let projected_active = session_projection
        .get(&session_id)
        .and_then(|session| session.active_prompt_for_agent(&agent_id).cloned())
        .expect("projected active prompt should be updated");
    assert_eq!(projected_active.id(), started_next.id());
    assert_eq!(projected_active.prompt(), second.prompt());
    let source_records = app
        .lock()
        .await
        .terminal_stream_store()
        .drain_output_records(&session_id, attachment.id());
    assert!(
        source_records.iter().any(|record| {
            record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                && record.prompt_id.as_deref() == Some(started_next.id())
                && String::from_utf8_lossy(&record.bytes).contains(second.prompt())
        }),
        "promoted queued prompt should be echoed to the source attachment"
    );
}

#[tokio::test]
async fn prompt_cancel_uses_owned_runtime_state_for_pty_prompt_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-cancel-pty",
            "worktree-owned-cancel-pty",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-cancel-pty",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
    let prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "owned pty cancel",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .expect("prompt should submit through owner")
    else {
        panic!("first prompt should start");
    };
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let session_id = session.id().to_string();
    let attachment_id = attachment.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection,
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: None,
    };
    let local_request = LocalDaemonRequest::CancelActivePrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-pty-prompt-cancel",
        None,
        None,
        &local_request,
    );
    let app_guard = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_cancel(&command, request),
    )
    .await
    .expect("owned PTY prompt cancellation should not wait for the app lock")
    .expect("prompt cancellation should succeed");
    drop(app_guard);

    let LocalDaemonResponse::PromptCancelled { cancellation } = response else {
        panic!("unexpected response");
    };
    assert_eq!(cancellation.prompt.id(), prompt.id());
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
}
