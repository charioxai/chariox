use super::*;

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
    assert!(prompt.id().starts_with("session-task:"));
    assert_eq!(
        replacement_session
            .metaagent_task(&agent_id)
            .map(|task| task.task_markdown()),
        Some("Inspect the repo by delegation.")
    );
    assert_eq!(replacement_session.queued_metaagent_tasks().len(), 1);
    assert_eq!(
        replacement_session
            .queued_metaagent_tasks()
            .front()
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
    let completion = timeout(
        Duration::from_secs(5),
        runtime_state.dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::META_COMPLETE_TASK_TOOL,
            serde_json::json!({ "summary": "delegation complete" }),
        ),
    )
    .await
    .expect("meta task completion should not hang")
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
