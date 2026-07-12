use super::*;

#[tokio::test]
async fn prompt_cancel_queued_uses_owned_runtime_state_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-cancel-queued",
            "worktree-owned-cancel-queued",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-cancel-queued",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
    let active_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "active prompt",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started {
        prompt: active_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active_prompt, false)
        .expect("active prompt should start")
    else {
        panic!("first prompt should start");
    };
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued prompt",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Queued {
        prompt: queued_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
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
    let attachment_id = attachment.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        agent_runtime_projection,
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = CancelQueuedPromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: agent_id.clone(),
        prompt_id: queued_prompt.id().to_string(),
    };
    let local_request = LocalDaemonRequest::CancelQueuedPrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-queued-prompt-cancel",
        None,
        None,
        &local_request,
    );
    let app_guard = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_cancel_queued(&command, request),
    )
    .await
    .expect("owned queued prompt cancellation should not wait for the app lock")
    .expect("queued prompt cancellation should succeed");
    drop(app_guard);

    let LocalDaemonResponse::QueuedPromptCancelled {
        prompt,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert_eq!(prompt.id(), queued_prompt.id());
    assert_eq!(prompt.status(), PromptStatus::Cancelled);
    assert!(agent_activity.contains_key(&agent_id));
    assert_eq!(
        session
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(active_prompt.id())
    );
    assert!(
        session
            .queued_prompts_for_agent(&agent_id)
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "cancelled queued prompt should be removed from the session queue"
    );
}

#[tokio::test]
async fn prompt_update_queued_changes_queue_entry_without_settling_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-update-queued",
            "worktree-owned-update-queued",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-update-queued",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
    let active_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "active prompt",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started {
        prompt: active_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active_prompt, false)
        .expect("active prompt should start")
    else {
        panic!("first prompt should start");
    };
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued prompt",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Queued {
        prompt: queued_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
        .expect("second prompt should queue")
    else {
        panic!("second prompt should queue");
    };
    let operational_history = app.operational_history_store();
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
        session_projection,
        agent_runtime_projection,
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = UpdateQueuedPromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: agent_id.clone(),
        prompt_id: queued_prompt.id().to_string(),
        prompt: "updated queued prompt".to_string(),
    };
    let local_request = LocalDaemonRequest::UpdateQueuedPrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-queued-prompt-update",
        None,
        None,
        &local_request,
    );
    let app_guard = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_update_queued(&command, request),
    )
    .await
    .expect("owned queued prompt update should not wait for the app lock")
    .expect("queued prompt update should succeed");
    drop(app_guard);

    let LocalDaemonResponse::QueuedPromptUpdated {
        prompt,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert_eq!(prompt.id(), queued_prompt.id());
    assert_eq!(prompt.prompt(), "updated queued prompt");
    assert!(agent_activity.contains_key(&agent_id));
    assert_eq!(
        session
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(active_prompt.id())
    );
    assert_eq!(
        session
            .queued_prompts_for_agent(&agent_id)
            .and_then(|queued| queued.front())
            .map(|prompt| prompt.prompt()),
        Some("updated queued prompt")
    );
    let history_events = operational_history
        .load_session_events(&session_id, Some(&agent_id))
        .expect("operational history should load");
    assert!(
        history_events
            .iter()
            .all(|event| event.prompt_id.as_deref() != Some(queued_prompt.id())),
        "editing a pending queued prompt must not replace history because no history exists yet"
    );
}

#[tokio::test]
async fn prompt_steer_queued_removes_queue_entry_without_settling_active_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-steer-queued",
            "worktree-owned-steer-queued",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-steer-queued",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let steering_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-steering-owned-queued-prompt",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("steering attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
    let active_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "active prompt",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started {
        prompt: active_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), active_prompt, false)
        .expect("active prompt should start")
    else {
        panic!("first prompt should start");
    };
    let queued_prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "queued prompt",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Queued {
        prompt: queued_prompt,
    } = app
        .prompt_owner_submit_prepared_prompt(session.id(), queued_prompt, false)
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
    let attachment_id = steering_attachment.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let runtime = AgentRuntime::new(
        owned_runtime_state(&app).await,
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection,
        agent_runtime_projection,
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = SteerQueuedPromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: agent_id.clone(),
        prompt_id: queued_prompt.id().to_string(),
    };
    let local_request = LocalDaemonRequest::SteerQueuedPrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-queued-prompt-steer",
        None,
        None,
        &local_request,
    );
    let app_guard = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_steer_queued(&command, request),
    )
    .await
    .expect("owned queued prompt steer should not wait for the app lock")
    .expect("queued prompt steer should succeed");
    drop(app_guard);

    let LocalDaemonResponse::QueuedPromptSteered {
        prompt, session, ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert_eq!(prompt.id(), queued_prompt.id());
    assert_eq!(
        session
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(active_prompt.id())
    );
    assert!(
        session
            .queued_prompts_for_agent(&agent_id)
            .map(|queued| queued.is_empty())
            .unwrap_or(true),
        "steered queued prompt should be removed from the session queue"
    );
    let source_records = app
        .lock()
        .await
        .terminal_stream_store()
        .drain_output_records(&session_id, attachment.id());
    let steering_merge_key = crate::history::steering_prompt_merge_key(queued_prompt.id());
    assert!(
        source_records.iter().any(|record| {
            record.kind == crate::terminal::TerminalOutputKind::PromptEcho
                && record.prompt_id.as_deref() == Some(queued_prompt.id())
                && record.merge_key.as_deref() == Some(steering_merge_key.as_str())
                && record.source_attachment_id.as_deref() == Some(attachment.id())
                && String::from_utf8_lossy(&record.bytes).contains(queued_prompt.prompt())
        }),
        "steered queued prompt should be echoed to the other attachment"
    );
    let steering_records = app
        .lock()
        .await
        .terminal_stream_store()
        .drain_output_records(&session_id, steering_attachment.id());
    assert!(
        steering_records
            .iter()
            .all(|record| { record.merge_key.as_deref() != Some(steering_merge_key.as_str()) }),
        "steering attachment already receives the prompt in its command response"
    );
}
