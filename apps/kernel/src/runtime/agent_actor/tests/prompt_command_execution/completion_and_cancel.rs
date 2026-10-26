use super::*;

#[tokio::test]
async fn prompt_complete_uses_owned_runtime_state_without_app_lock_for_simple_local_prompt() {
    let worktree = TestWorktree::new("owned-complete");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
    let worktree = TestWorktree::new("owned-cancel");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
        prompt_state_owner.clone(),
        crate::session::PromptIdAllocator::default(),
    );

    let request = CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: None,
        expected_prompt_id: Some(prompt.id().to_string()),
    };
    let local_request = LocalDaemonRequest::CancelActivePrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-prompt-cancel",
        None,
        None,
        &local_request,
    );
    let _locked_app = app.lock().await;
    // MP-08/MP-10/MP-11: a delayed display control for A must leave B running.
    let mut stale = request.clone();
    stale.expected_prompt_id = Some("settled-prompt-A".into());
    assert!(matches!(
        runtime.dispatch_prompt_cancel(&command, stale).await,
        Err(DaemonError::NoActivePrompt { .. })
    ));
    assert_eq!(
        prompt_state_owner
            .active_prompt_for_agent(&session_snapshot, &agent_id)
            .map(|active| (active.id().to_string(), active.status())),
        Some((prompt.id().to_string(), prompt.status()))
    );
    let mut foreign = request.clone();
    foreign.attachment_id = "foreign-attachment".into();
    assert!(
        runtime
            .dispatch_prompt_cancel(&command, foreign)
            .await
            .is_err(),
        "an expected prompt cannot bypass attachment admission"
    );
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
    let worktree = TestWorktree::new("owned-advance");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
    assert_eq!(started_next.id(), second.id());
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
    // MP-08 / MP-10 (panes R5, chariox#987 review): the echo names the
    // history row the prompt was stored as.
    let prompt_row = app
        .lock()
        .await
        .operational_history_store()
        .load_latest_user_prompt_events(&session_id, &agent_id, None, 8)
        .expect("prompt history should load")
        .into_iter()
        .find(|event| event.prompt_id.as_deref() == Some(started_next.id()))
        .map(|event| event.sequence);
    assert!(prompt_row.is_some(), "promoted prompt should be stored");
    assert!(
        source_records.iter().any(|record| {
            record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                && record.prompt_id.as_deref() == Some(started_next.id())
                && String::from_utf8_lossy(&record.bytes).contains(second.prompt())
                && record.sequence == prompt_row
        }),
        "promoted queued prompt should be echoed to the source attachment with its history sequence: {:?}",
        source_records
            .iter()
            .map(|record| (&record.kind, &record.prompt_id, record.sequence))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn prompt_cancel_uses_owned_runtime_state_for_pty_prompt_without_app_lock() {
    let worktree = TestWorktree::new("owned-cancel-pty");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
        expected_prompt_id: None,
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
#[tokio::test]
async fn conditional_prompt_cancel_preserves_successor_after_snapshot() {
    assert_conditional_prompt_cancel_preserves_successor(false).await;
}

#[tokio::test]
async fn conditional_remote_prompt_cancel_preserves_successor_after_snapshot() {
    assert_conditional_prompt_cancel_preserves_successor(true).await;
}

async fn assert_conditional_prompt_cancel_preserves_successor(remote: bool) {
    let worktree = TestWorktree::new("owned-cancel");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
    // MP-08/MP-10/MP-11: the native client observed A here. Queue B, then
    // settle A before the correlated cancellation reaches the agent owner.
    let expected_prompt_id = prompt.id().to_string();
    let successor = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "successor",
        PromptStatus::Queued,
    );
    app.prompt_owner_submit_prepared_prompt(session.id(), successor, false)
        .unwrap();
    let prompt_owner = app.prompt_state_owner();
    prompt_owner
        .complete_active_prompt_if_matches(&session, agent.id(), Some(&expected_prompt_id))
        .unwrap();
    let successor = prompt_owner
        .activate_next_queued_prompt_for_dispatch(&session, agent.id(), None)
        .unwrap()
        .unwrap();
    prompt_owner
        .mark_active_prompt_running(&session, agent.id())
        .unwrap();
    if remote {
        app.agents_mut()
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "conditional-worker".into(),
                    worker_machine_id: "conditional-machine".into(),
                    execution_lease_id: "conditional-lease".into(),
                    leased_agent_id: "conditional-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            successor.id(),
            crate::session::DurablePromptDeliveryPhase::Accepted,
            None,
            None,
        )
        .unwrap();
    }
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

    let request: CancelActivePromptRequest = serde_json::from_value(serde_json::json!({
        "session_id": session_id, "attachment_id": attachment_id,
        "target_agent_id": agent_id, "expected_prompt_id": expected_prompt_id,
    }))
    .unwrap();
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
    .expect("conditional prompt cancellation should not wait for the app lock");
    assert!(
        matches!(response, Err(DaemonError::NoActivePrompt { .. })),
        "stale native cancellation must be rejected: {response:?}"
    );
    let active = prompt_owner
        .active_prompt_for_agent(&session, &agent_id)
        .unwrap();
    assert_eq!(active.id(), successor.id());
    assert_eq!(active.status(), PromptStatus::Running);
    // A matching request still cancels B through the ordinary control path.
    let request: CancelActivePromptRequest = serde_json::from_value(serde_json::json!({
        "session_id": session_id, "attachment_id": attachment_id,
        "target_agent_id": agent_id, "expected_prompt_id": successor.id(),
    }))
    .unwrap();
    let local_request = LocalDaemonRequest::CancelActivePrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "conditional-cancel-successor",
        None,
        None,
        &local_request,
    );
    let result = runtime
        .dispatch_prompt_cancel(&command, request)
        .await
        .unwrap();
    let LocalDaemonResponse::PromptCancelled { cancellation } = result else {
        panic!("unexpected conditional response");
    };
    assert_eq!(cancellation.prompt.id(), successor.id());
    assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
}
