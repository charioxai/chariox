use super::*;

#[tokio::test]
async fn prompt_submit_rejects_cross_session_agent_before_admission() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session should be created");
    let (second_session, _second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            second_session.id(),
            "cross-session-router-prompt",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let first_session_id = first_session.id().to_string();
    let first_agent_id = first_agent.id().to_string();
    let second_session_id = second_session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: second_session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(first_agent_id.clone()),
        prompt: "must not cross session boundary".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command = KernelCommand::from_local_request(
        "cmd-cross-session-prompt-submit",
        None,
        None,
        &prompt_request,
    );

    let error = router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect_err("prompt submission should reject an agent outside the requested session");

    assert!(matches!(
        error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == second_session_id && agent_id == first_agent_id
    ));
    let app = app.lock().await;
    assert!(app
        .providers()
        .get_latest_run_for_agent(&first_session_id, &first_agent_id)
        .is_none());
    assert!(app
        .providers()
        .get_latest_run_for_agent(&second_session_id, &first_agent_id)
        .is_none());
}

#[tokio::test]
async fn agent_and_workflow_lanes_are_removed_when_session_ends() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-agent-lane-cleanup",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "create agent lane".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-agent-lane-create", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt should create an agent lane");
    assert!(router
        .daemon_health_projection(0)
        .await
        .agent_command_lanes
        .iter()
        .any(|lane| lane.lane_id == agent_id));
    let workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
        session_id: session_id.clone(),
        alias: Some("cleanup-workflow".to_string()),
    });
    let workflow_command = KernelCommand::from_local_request(
        "cmd-workflow-lane-create",
        None,
        None,
        &workflow_request,
    );
    router
        .dispatch(workflow_command, workflow_request)
        .await
        .expect("workflow command should create a workflow lane");
    assert!(router.workflow_runtime.has_lane(&session_id).await);

    let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: session_id.clone(),
    });
    let end_command =
        KernelCommand::from_local_request("cmd-agent-lane-end", None, None, &end_request);
    router
        .dispatch(end_command, end_request)
        .await
        .expect("ending session should clean up agent lane");

    assert!(!router
        .daemon_health_projection(0)
        .await
        .agent_command_lanes
        .iter()
        .any(|lane| lane.lane_id == agent_id));
    assert!(!router.workflow_runtime.has_lane(&session_id).await);
}

#[tokio::test]
async fn agent_lane_is_removed_when_agent_is_destroyed() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-agent-destroy-lane-cleanup",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "create agent lane".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command = KernelCommand::from_local_request(
        "cmd-agent-destroy-lane-create",
        None,
        None,
        &prompt_request,
    );
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt should create an agent lane");
    assert!(router
        .daemon_health_projection(0)
        .await
        .agent_command_lanes
        .iter()
        .any(|lane| lane.lane_id == agent_id));

    let destroy_request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
        session_id,
        agent_id: agent_id.clone(),
    });
    let destroy_command = KernelCommand::from_local_request(
        "cmd-agent-destroy-lane-cleanup",
        None,
        None,
        &destroy_request,
    );
    router
        .dispatch(destroy_command, destroy_request)
        .await
        .expect("destroying agent should clean up agent lane");

    assert!(!router
        .daemon_health_projection(0)
        .await
        .agent_command_lanes
        .iter()
        .any(|lane| lane.lane_id == agent_id));
}

#[tokio::test]
async fn prompt_submit_uses_agent_lane_without_generic_interactive_lane() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;

    let first_request = focus_request(&session_id, &agent_id);
    let first_command =
        KernelCommand::from_local_request("cmd-focus-1", None, None, &first_request);
    let first_router = router.clone();
    let first_task =
        tokio::spawn(async move { first_router.dispatch(first_command, first_request).await });

    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "hello from agent lane".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
    let prompt_router = router.clone();
    let prompt_task =
        tokio::spawn(async move { prompt_router.dispatch(prompt_command, prompt_request).await });

    let prompt_response = timeout(Duration::from_millis(100), prompt_task)
        .await
        .expect("owned prompt submit should not wait for the app lock")
        .expect("prompt task should join")
        .expect("prompt should submit");
    drop(app_guard);
    let _ = first_task.await.expect("first focus should join");
    match prompt_response {
        crate::local::LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), agent_id);
            }
            _ => panic!("expected prompt to start"),
        },
        _ => panic!("unexpected prompt response"),
    }
}

#[tokio::test]
async fn prompt_submit_uses_session_focus_projection_without_app_lock_for_routing() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let focused_agent = spawn_test_agent(&mut app, &session_id, "focused", "claude-code");
    launch_test_provider(
        &mut app,
        &session_id,
        focused_agent.id(),
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let focus_request = focus_request(&session_id, focused_agent.id());
    let focus_command =
        KernelCommand::from_local_request("cmd-focus-projection", None, None, &focus_request);
    router
        .dispatch(focus_command, focus_request)
        .await
        .expect("focus should populate the projection");

    let app_guard = app.lock().await;
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: None,
        prompt: "hello through projected focus".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-projection", None, None, &prompt_request);
    let prompt_router = router.clone();
    let prompt_task =
        tokio::spawn(async move { prompt_router.dispatch(prompt_command, prompt_request).await });

    let prompt_response = timeout(Duration::from_millis(100), prompt_task)
        .await
        .expect("owned prompt submit should not wait for the app lock")
        .expect("prompt task should join")
        .expect("prompt should submit");
    drop(app_guard);
    match prompt_response {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), focused_agent.id());
            }
            _ => panic!("expected prompt to start"),
        },
        _ => panic!("unexpected prompt response"),
    }
}

#[tokio::test]
async fn prompt_submit_uses_warmed_session_projection_without_app_lock_for_focus_fallback() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-session-projection-focus",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-focus-fallback-warm", None, None, &state_request);
    router
        .dispatch(state_command, state_request)
        .await
        .expect("state read should warm the session projection");

    let app_guard = app.lock().await;
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: None,
        prompt: "hello through warmed session projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command = KernelCommand::from_local_request(
        "cmd-prompt-session-projection-focus",
        None,
        None,
        &prompt_request,
    );
    let prompt_router = router.clone();
    let prompt_task =
        tokio::spawn(async move { prompt_router.dispatch(prompt_command, prompt_request).await });

    let prompt_response = timeout(Duration::from_millis(100), prompt_task)
        .await
        .expect("owned prompt submit should not wait for the app lock")
        .expect("prompt task should join")
        .expect("prompt should submit");
    drop(app_guard);
    match prompt_response {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), agent_id);
            }
            _ => panic!("expected prompt to start"),
        },
        _ => panic!("unexpected prompt response"),
    }
}

#[tokio::test]
async fn agent_spawn_refreshes_focus_projection_for_followup_prompt_routing() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        session_id: session_id.clone(),
        alias: Some("spawned".to_string()),
        provider: Some("claude-code".to_string()),
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let spawn_command =
        KernelCommand::from_local_request("cmd-spawn-projection", None, None, &spawn_request);
    let spawned_agent = match router
        .dispatch(spawn_command, spawn_request)
        .await
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected spawn response"),
    };

    {
        let mut app = app.lock().await;
        launch_test_provider(
            &mut app,
            &session_id,
            spawned_agent.id(),
            "dev-stub",
            "claude-code",
            "sonnet",
        );
    }

    let app_guard = app.lock().await;
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: None,
        prompt: "hello after spawn".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-after-spawn", None, None, &prompt_request);
    let prompt_router = router.clone();
    let prompt_task =
        tokio::spawn(async move { prompt_router.dispatch(prompt_command, prompt_request).await });

    let mut spawned_agent_lane_created = false;
    for _ in 0..50 {
        let projection = router.daemon_health_projection(0).await;
        if projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == spawned_agent.id())
        {
            spawned_agent_lane_created = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        spawned_agent_lane_created,
        "spawn should refresh focused-agent projection before followup prompt routing"
    );

    drop(app_guard);
    let prompt_response = prompt_task
        .await
        .expect("prompt task should join")
        .expect("prompt should submit");
    match prompt_response {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), spawned_agent.id());
            }
            _ => panic!("expected prompt to start"),
        },
        _ => panic!("unexpected prompt response"),
    }
}

#[tokio::test]
async fn get_session_state_uses_projection_after_prompt_submit_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(agent_id.clone()),
        prompt: "warm session projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-state", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm the session projection");

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-state-projection", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "warm GetSessionState should be served from the session projection without app lock access"
    );

    drop(app_guard);
    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert!(session.active_prompt_for_agent(&agent_id).is_some());
            assert_eq!(session.agents().len(), 1);
        }
        _ => panic!("unexpected state response"),
    }
}
