use super::*;

#[tokio::test]
async fn prompt_submit_batch_starts_multiple_agents_with_one_kernel_request() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit-batch",
            "worktree-owned-submit-batch",
        ))
        .expect("session should be created");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("batch-agent")
                .with_worktree("worktree-owned-submit-batch"),
        )
        .expect("second agent should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-submit-batch",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), default_agent.id(), "sonnet");
    launch_dev_stub_provider(&mut app, session.id(), second_agent.id(), "sonnet");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let session_id = session.id().to_string();
    let first_agent_id = default_agent.id().to_string();
    let second_agent_id = second_agent.id().to_string();
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

    let request = crate::local::SubmitPromptsRequest {
        session_id: session_id.clone(),
        attachment_id,
        max_concurrency: Some(2),
        prompts: vec![
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: first_agent_id.clone(),
                prompt: "batch prompt one".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: second_agent_id.clone(),
                prompt: "batch prompt two".to_string(),
                attachments: Vec::new(),
            },
        ],
    };
    let local_request = LocalDaemonRequest::SubmitPrompts(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-batch-prompt-submit",
        None,
        None,
        &local_request,
    );
    let response = runtime
        .dispatch_prompt_submit_batch(&command, request)
        .await
        .expect("batch prompt submit should succeed");

    let LocalDaemonResponse::PromptsSubmitted {
        results,
        failures,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert!(failures.is_empty());
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].agent_id, first_agent_id);
    assert_eq!(results[1].agent_id, second_agent_id);
    assert!(matches!(
        results[0].outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    assert!(matches!(
        results[1].outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    assert!(session.active_prompt_for_agent(&first_agent_id).is_some());
    assert!(session.active_prompt_for_agent(&second_agent_id).is_some());
    assert!(
        agent_activity
            .get(&first_agent_id)
            .is_some_and(|activity| activity.busy)
    );
    assert!(
        agent_activity
            .get(&second_agent_id)
            .is_some_and(|activity| activity.busy)
    );
}

#[tokio::test]
async fn prompt_submit_batch_rejects_duplicate_targets_without_partial_submit() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit-batch-duplicate",
            "worktree-owned-submit-batch-duplicate",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-submit-batch-duplicate",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let session_id = session.id().to_string();
    let agent_id = default_agent.id().to_string();
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

    let request = crate::local::SubmitPromptsRequest {
        session_id: session_id.clone(),
        attachment_id,
        max_concurrency: Some(2),
        prompts: vec![
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: agent_id.clone(),
                prompt: "duplicate batch prompt one".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: agent_id.clone(),
                prompt: "duplicate batch prompt two".to_string(),
                attachments: Vec::new(),
            },
        ],
    };
    let local_request = LocalDaemonRequest::SubmitPrompts(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-batch-prompt-submit-duplicate",
        None,
        None,
        &local_request,
    );
    let response = runtime
        .dispatch_prompt_submit_batch(&command, request)
        .await
        .expect("duplicate prompt batch should return indexed failures");

    let LocalDaemonResponse::PromptsSubmitted {
        results,
        failures,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert!(results.is_empty());
    assert_eq!(failures.len(), 2);
    assert!(
        failures
            .iter()
            .all(|failure| failure.message.contains("duplicate target agents"))
    );
    assert!(session.active_prompt_for_agent(&agent_id).is_none());
    assert!(
        agent_activity
            .get(&agent_id)
            .is_some_and(|activity| !activity.busy)
    );
    let app = app.lock().await;
    let session_after = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should remain available");
    assert!(session_after.active_prompt_for_agent(&agent_id).is_none());
}

#[tokio::test]
async fn prompt_submit_batch_rejects_invalid_targets_without_partial_submit() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit-batch-invalid",
            "worktree-owned-submit-batch-invalid",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-submit-batch-invalid",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(session.id())
        .expect("session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(session_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&session_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let session_id = session.id().to_string();
    let agent_id = default_agent.id().to_string();
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

    let request = crate::local::SubmitPromptsRequest {
        session_id: session_id.clone(),
        attachment_id,
        max_concurrency: Some(2),
        prompts: vec![
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: agent_id.clone(),
                prompt: "valid prompt must not partially start".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: "missing-agent".to_string(),
                prompt: "invalid prompt".to_string(),
                attachments: Vec::new(),
            },
        ],
    };
    let local_request = LocalDaemonRequest::SubmitPrompts(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-batch-prompt-submit-invalid",
        None,
        None,
        &local_request,
    );
    let response = runtime
        .dispatch_prompt_submit_batch(&command, request)
        .await
        .expect("invalid prompt batch should return indexed failures");

    let LocalDaemonResponse::PromptsSubmitted {
        results,
        failures,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert!(results.is_empty());
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].index, 1);
    assert_eq!(failures[0].agent_id.as_deref(), Some("missing-agent"));
    assert!(failures[0].message.contains("missing-agent"));
    assert!(session.active_prompt_for_agent(&agent_id).is_none());
    assert!(
        agent_activity
            .get(&agent_id)
            .is_some_and(|activity| !activity.busy)
    );
    let app = app.lock().await;
    let session_after = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should remain available");
    assert!(session_after.active_prompt_for_agent(&agent_id).is_none());
}

#[tokio::test]
async fn prompt_submit_batch_accepts_explicit_mixed_sessions() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit-batch-mixed-1",
            "worktree-owned-submit-batch-mixed-1",
        ))
        .expect("first session should be created");
    let (second_session, second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit-batch-mixed-2",
            "worktree-owned-submit-batch-mixed-2",
        ))
        .expect("second session should be created");
    let first_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            first_session.id(),
            "client-owned-submit-batch-mixed",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("first attachment should attach");
    let second_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            second_session.id(),
            "client-owned-submit-batch-mixed-second",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("second attachment should attach");
    launch_dev_stub_provider(&mut app, first_session.id(), first_agent.id(), "sonnet");
    launch_dev_stub_provider(&mut app, second_session.id(), second_agent.id(), "sonnet");
    let first_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(first_session.id())
        .expect("first session snapshot should be available");
    let second_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(second_session.id())
        .expect("second session snapshot should be available");
    let session_projection = app.session_state_projection_store();
    session_projection.update(first_snapshot.clone());
    session_projection.update(second_snapshot.clone());
    let agent_runtime_projection = app.agent_runtime_projection_store();
    agent_runtime_projection.update_session(&first_snapshot);
    agent_runtime_projection.update_session(&second_snapshot);
    let prompt_state_owner = app.prompt_state_owner();
    let first_session_id = first_session.id().to_string();
    let second_session_id = second_session.id().to_string();
    let first_agent_id = first_agent.id().to_string();
    let second_agent_id = second_agent.id().to_string();
    let first_attachment_id = first_attachment.id().to_string();
    let second_attachment_id = second_attachment.id().to_string();
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

    let request = crate::local::SubmitPromptsRequest {
        session_id: first_session_id.clone(),
        attachment_id: first_attachment_id,
        max_concurrency: Some(2),
        prompts: vec![
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: first_agent_id.clone(),
                prompt: "valid prompt must not partially start".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
                session_id: Some(second_session_id.clone()),
                attachment_id: Some(second_attachment_id),
                target_agent_id: second_agent_id.clone(),
                prompt: "mixed session prompt".to_string(),
                attachments: Vec::new(),
            },
        ],
    };
    let local_request = LocalDaemonRequest::SubmitPrompts(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-batch-prompt-submit-mixed-session",
        None,
        None,
        &local_request,
    );
    let response = runtime
        .dispatch_prompt_submit_batch(&command, request)
        .await
        .expect("mixed-session prompt batch should succeed");

    let LocalDaemonResponse::PromptsSubmitted {
        results,
        failures,
        session,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert!(failures.is_empty());
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].index, 0);
    assert_eq!(results[1].index, 1);
    assert_eq!(results[0].agent_id, first_agent_id);
    assert_eq!(results[1].agent_id, second_agent_id);
    assert!(session.active_prompt_for_agent(&first_agent_id).is_some());
    let app = app.lock().await;
    let first_after = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&first_session_id)
        .expect("first session should remain available");
    let second_after = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&second_session_id)
        .expect("second session should remain available");
    assert!(
        first_after
            .active_prompt_for_agent(&first_agent_id)
            .is_some()
    );
    assert!(
        second_after
            .active_prompt_for_agent(&second_agent_id)
            .is_some()
    );
}

#[tokio::test]
async fn prompt_submit_batch_projects_final_queued_prompt_state() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit-batch-queued",
            "worktree-owned-submit-batch-queued",
        ))
        .expect("session should be created");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("batch-queued-agent")
                .with_worktree("worktree-owned-submit-batch-queued"),
        )
        .expect("second agent should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-submit-batch-queued",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let first_active = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        default_agent.id(),
        "active prompt one",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), first_active, false)
        .expect("first active prompt should start")
    else {
        panic!("first prompt should start");
    };
    let second_active = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        second_agent.id(),
        "active prompt two",
        PromptStatus::Queued,
    );
    let PromptSubmissionOutcome::Started { .. } = app
        .prompt_owner_submit_prepared_prompt(session.id(), second_active, false)
        .expect("second active prompt should start")
    else {
        panic!("second prompt should start");
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
    let first_agent_id = default_agent.id().to_string();
    let second_agent_id = second_agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let projection_sequence_before_batch = session_projection.change_sequence();
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

    let request = crate::local::SubmitPromptsRequest {
        session_id: session_id.clone(),
        attachment_id,
        max_concurrency: Some(2),
        prompts: vec![
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: first_agent_id.clone(),
                prompt: "queued batch prompt one".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
                session_id: None,
                attachment_id: None,
                target_agent_id: second_agent_id.clone(),
                prompt: "queued batch prompt two".to_string(),
                attachments: Vec::new(),
            },
        ],
    };
    let local_request = LocalDaemonRequest::SubmitPrompts(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-batch-prompt-submit-queued",
        None,
        None,
        &local_request,
    );
    let response = runtime
        .dispatch_prompt_submit_batch(&command, request)
        .await
        .expect("batch prompt submit should succeed");

    let LocalDaemonResponse::PromptsSubmitted {
        results,
        failures,
        session,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    assert!(failures.is_empty());
    assert_eq!(results.len(), 2);
    assert!(matches!(
        results[0].outcome,
        PromptSubmissionOutcome::Queued { .. }
    ));
    assert!(matches!(
        results[1].outcome,
        PromptSubmissionOutcome::Queued { .. }
    ));
    assert_eq!(
        session
            .queued_prompts_for_agent(&first_agent_id)
            .map(|prompts| prompts.len())
            .unwrap_or_default(),
        1,
        "first agent should have exactly one queued batch prompt"
    );
    assert_eq!(
        session
            .queued_prompts_for_agent(&second_agent_id)
            .map(|prompts| prompts.len())
            .unwrap_or_default(),
        1,
        "second agent should have exactly one queued batch prompt"
    );
    let projected = session_projection
        .get(&session_id)
        .expect("batch prompt submit should publish final session projection");
    assert!(
        session_projection.change_sequence() > projection_sequence_before_batch,
        "batch prompt submit should publish a final session projection revision"
    );
    assert_eq!(
        projected
            .queued_prompts_for_agent(&first_agent_id)
            .map(|prompts| prompts.len())
            .unwrap_or_default(),
        1,
        "first projected agent should have exactly one queued batch prompt"
    );
    assert_eq!(
        projected
            .queued_prompts_for_agent(&second_agent_id)
            .map(|prompts| prompts.len())
            .unwrap_or_default(),
        1,
        "second projected agent should have exactly one queued batch prompt"
    );
}
