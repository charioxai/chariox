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
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_complete(&command, request),
    )
    .await
    .expect("owned queued advancement should not wait for the app lock")
    .expect("prompt completion should succeed");

    let LocalDaemonResponse::PromptCompleted { completion } = response else {
        panic!("unexpected response");
    };
    assert_eq!(completion.completed.id(), first.id());
    assert_eq!(
        completion.started_next.as_ref().map(|prompt| prompt.id()),
        Some(second.id())
    );
    assert_eq!(
        session_projection
            .get(&session_id)
            .and_then(|session| session.active_prompt_for_agent(&agent_id).cloned())
            .map(|prompt| prompt.id().to_string()),
        Some(second.id().to_string())
    );
}

#[tokio::test]
async fn prompt_submit_uses_owned_runtime_state_without_app_lock_for_local_prompt() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit",
            "worktree-owned-submit",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-submit",
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

    let request = SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: Some(agent_id.clone()),
        prompt: "owned submit".to_string(),
        attachments: Vec::new(),
    };
    let local_request = LocalDaemonRequest::SubmitPrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-prompt-submit",
        None,
        None,
        &local_request,
    );
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_submit(&command, request),
    )
    .await
    .expect("owned local prompt submit should not wait for the app lock")
    .expect("prompt submit should succeed");

    let LocalDaemonResponse::PromptSubmitted {
        outcome,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    let PromptSubmissionOutcome::Started { prompt } = outcome else {
        panic!("prompt should start");
    };
    assert_eq!(prompt.target_agent_id(), agent_id);
    assert_eq!(
        session
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(prompt.id())
    );
    assert_eq!(
        agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .map(|prompt| prompt.id().to_string()),
        Some(prompt.id().to_string())
    );
    let activity = agent_activity
        .get(&agent_id)
        .expect("submitted response should carry agent activity");
    assert!(activity.busy);
    assert_eq!(
        activity
            .active_turn
            .as_ref()
            .and_then(|turn| turn.provider_run_id.as_deref()),
        Some(provider_run.id())
    );
}

#[tokio::test]
async fn prompt_submit_meta_slash_activates_meta_mode_and_strips_command() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-meta-slash-submit",
            "worktree-meta-slash-submit",
        ))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-meta-slash-submit",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
    app.agents()
        .update_agent_config(
            agent.id(),
            Some(Some(crate::provider::AgentExecutionMode::Build)),
            Some(Some(crate::provider::AgentPermissionLevel::Yolo)),
            None,
            None,
        )
        .expect("baseline agent profile should update");
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
    let runtime_state = owned_runtime_state(&app).await;
    let runtime = AgentRuntime::new(
        runtime_state.clone(),
        ProviderRunOperationLanes::default(),
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        prompt_state_owner,
        crate::session::PromptIdAllocator::default(),
    );

    let request = SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "/meta Inspect the repo by delegation.".to_string(),
        attachments: Vec::new(),
    };
    let local_request = LocalDaemonRequest::SubmitPrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "meta-slash-prompt-submit",
        None,
        None,
        &local_request,
    );
    let response = timeout(
        Duration::from_secs(5),
        runtime.dispatch_prompt_submit(&command, request),
    )
    .await
    .expect("meta slash prompt submit should not hang")
    .expect("meta slash prompt should submit");

    let LocalDaemonResponse::PromptSubmitted {
        outcome, session, ..
    } = response
    else {
        panic!("unexpected response");
    };
    let PromptSubmissionOutcome::Started { prompt } = outcome else {
        panic!("meta slash prompt should start");
    };
    assert_eq!(prompt.prompt(), "Inspect the repo by delegation.");
    assert!(
        prompt
            .hidden_system_context()
            .to_ascii_lowercase()
            .contains("now operating in arroba meta mode"),
        "meta mode boundary context should be hidden on first meta turn"
    );
    assert_eq!(
        session
            .metaagent_task(&agent_id)
            .map(|task| task.task_markdown()),
        Some("Inspect the repo by delegation.")
    );
    let replacement_request = SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: Some(agent_id.clone()),
        prompt: "/meta Expand the delegation plan.".to_string(),
        attachments: Vec::new(),
    };
    let replacement_local_request = LocalDaemonRequest::SubmitPrompt(replacement_request.clone());
    let replacement_command = crate::runtime::command::KernelCommand::from_local_request(
        "meta-slash-prompt-replace",
        None,
        None,
        &replacement_local_request,
    );
    let replacement_response = timeout(
        Duration::from_secs(5),
        runtime.dispatch_prompt_submit(&replacement_command, replacement_request),
    )
    .await
    .expect("replacement meta slash prompt submit should not hang")
    .expect("replacement meta slash prompt should submit");
    let LocalDaemonResponse::PromptSubmitted {
        outcome,
        session: replacement_session,
        ..
    } = replacement_response
    else {
        panic!("unexpected replacement response");
    };
    let PromptSubmissionOutcome::Queued { prompt } = outcome else {
        panic!("replacement meta slash prompt should queue behind active meta turn");
    };
    assert_eq!(prompt.prompt(), "Expand the delegation plan.");
    assert_eq!(
        replacement_session
            .metaagent_task(&agent_id)
            .map(|task| task.task_markdown()),
        Some("Expand the delegation plan.")
    );
    let agent = runtime_state
        .list_agents()
        .into_iter()
        .find(|agent| agent.id() == agent_id)
        .expect("agent should exist");
    assert!(agent.is_metaagent());
    assert_eq!(agent.role(), crate::agent::AgentRole::Standard);
    let run = app
        .lock()
        .await
        .providers()
        .get_run_for_agent(&session_id, &agent_id)
        .expect("provider run should be launched for meta prompt");
    let auth_token = run
        .runtime_mcp_auth_token()
        .expect("provider run should have runtime MCP auth token")
        .to_string();
    let specs = runtime_state.runtime_tool_specs_for_auth_token(&auth_token);
    assert!(specs
        .iter()
        .any(|spec| spec.name == crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL));
    let completion = runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::META_COMPLETE_TASK_TOOL,
            serde_json::json!({ "summary": "delegation complete" }),
        )
        .await
        .expect("complete_task should dispatch");
    assert!(completion.ok);
    let completed_agent = runtime_state
        .list_agents()
        .into_iter()
        .find(|agent| agent.id() == agent_id)
        .expect("agent should still exist");
    assert!(!completed_agent.is_metaagent());
    assert_eq!(
        completed_agent.operating_mode(),
        crate::agent::AgentOperatingMode::Regular
    );
    assert_eq!(
        completed_agent.execution_mode_override(),
        Some(crate::provider::AgentExecutionMode::Build)
    );
    assert_eq!(
        completed_agent.permission_level_override(),
        Some(crate::provider::AgentPermissionLevel::Yolo)
    );
    let stale_specs = runtime_state.runtime_tool_specs_for_auth_token(&auth_token);
    assert!(
        stale_specs.iter().all(|spec| {
            spec.name != crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL
        }),
        "stale provider auth token must not retain meta tools after mode exit"
    );
    let stale_call = runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect_err("stale provider auth token must not dispatch meta tools after mode exit");
    assert!(
        stale_call
            .to_string()
            .contains("agents currently in Meta mode")
            || stale_call
                .to_string()
                .contains("exactly one active provider run for an agent in Meta mode"),
        "{stale_call:?}"
    );
    let session_after_completion = runtime_state
        .session_snapshot(&session_id)
        .await
        .expect("session should still exist");
    assert_eq!(
        session_after_completion
            .metaagent_task(&agent_id)
            .map(|task| task.status()),
        Some(crate::session::MetaagentTaskStatus::Completed)
    );
    assert!(
        session_after_completion
            .queued_prompts_for_agent(&agent_id)
            .into_iter()
            .flatten()
            .any(|prompt| {
                prompt
                    .hidden_system_context()
                    .to_ascii_lowercase()
                    .contains("has left arroba meta mode")
            }),
        "mode exit should queue a kernel continuation prompt"
    );
}

#[tokio::test]
async fn prompt_submit_uses_owned_runtime_state_for_multi_agent_pty_prompt_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-owned-submit-pty",
            "worktree-owned-submit-pty",
        ))
        .expect("session should be created");
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("pty-agent")
                .with_worktree("worktree-owned-submit-pty"),
        )
        .expect("second agent should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-owned-submit-pty",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_dev_stub_provider(&mut app, session.id(), agent.id(), "sonnet");
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

    let request = SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id,
        target_agent_id: Some(agent_id.clone()),
        prompt: "owned pty submit".to_string(),
        attachments: Vec::new(),
    };
    let local_request = LocalDaemonRequest::SubmitPrompt(request.clone());
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "owned-local-pty-prompt-submit",
        None,
        None,
        &local_request,
    );
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_prompt_submit(&command, request),
    )
    .await
    .expect("owned multi-agent PTY prompt submit should not wait for the app lock")
    .expect("prompt submit should succeed");

    let LocalDaemonResponse::PromptSubmitted {
        outcome,
        session,
        agent_activity,
        ..
    } = response
    else {
        panic!("unexpected response");
    };
    let PromptSubmissionOutcome::Started { prompt } = outcome else {
        panic!("prompt should start");
    };
    assert_eq!(prompt.target_agent_id(), agent_id);
    assert_eq!(
        session
            .active_prompt_for_agent(&agent_id)
            .map(|prompt| prompt.id()),
        Some(prompt.id())
    );
    assert!(agent_activity
        .get(&agent_id)
        .map(|activity| activity.busy)
        .unwrap_or(false));
}

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
                target_agent_id: first_agent_id.clone(),
                prompt: "batch prompt one".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
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
    assert!(agent_activity
        .get(&first_agent_id)
        .is_some_and(|activity| activity.busy));
    assert!(agent_activity
        .get(&second_agent_id)
        .is_some_and(|activity| activity.busy));
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
                target_agent_id: agent_id.clone(),
                prompt: "duplicate batch prompt one".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
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
    assert!(failures
        .iter()
        .all(|failure| failure.message.contains("duplicate target agents")));
    assert!(session.active_prompt_for_agent(&agent_id).is_none());
    assert!(agent_activity
        .get(&agent_id)
        .is_some_and(|activity| !activity.busy));
    let app = app.lock().await;
    let session_after = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should remain available");
    assert!(session_after.active_prompt_for_agent(&agent_id).is_none());
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
                target_agent_id: first_agent_id.clone(),
                prompt: "queued batch prompt one".to_string(),
                attachments: Vec::new(),
            },
            crate::local::SubmitPromptsRequestItem {
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

    let LocalDaemonResponse::QueuedPromptCancelled { prompt, session } = response else {
        panic!("unexpected response");
    };
    assert_eq!(prompt.id(), queued_prompt.id());
    assert_eq!(prompt.status(), PromptStatus::Cancelled);
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
}
