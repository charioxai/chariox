use super::*;
use crate::local::RequestNativeProviderInteractionRequest;

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

#[tokio::test]
async fn get_session_state_keeps_activity_after_runtime_interaction_projection_refresh() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let provider_run = launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    router.active_turns.start(crate::app::ActiveTurnState::new(
        session_id.clone(),
        agent_id.clone(),
        "prompt-1".to_string(),
        provider_run.id().to_string(),
    ));
    let interaction = RuntimeInteraction::new(
        "interaction-1",
        &agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _resolution = router
        .runtime_state
        .create_runtime_interaction(&session_id, interaction)
        .await
        .expect("interaction should register");

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-state-interaction", None, None, &state_request);
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
        LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            ..
        } => {
            assert_eq!(session.focused_agent_id(), Some(agent_id.as_str()));
            assert_eq!(session.agents().len(), 1);
            assert_eq!(session.active_interactions().len(), 1);
            let activity = agent_activity
                .get(&agent_id)
                .expect("agent activity should include focused agent");
            assert!(
                activity.busy,
                "active turn must keep focused agent working during permission popup"
            );
            assert!(
                activity.active_turn.is_some(),
                "active turn projection must survive interaction projection refresh"
            );
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn runtime_interaction_registration_and_resolution_wake_terminal_subscribers() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let interaction = RuntimeInteraction::new(
        "interaction-terminal-wake",
        &agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );

    let before_projection_sequence = router.session_projection_change_sequence();
    let before_terminal_session_sequence = router.terminal_session_change_sequence(&session_id);
    let receiver = router
        .runtime_state
        .create_runtime_interaction(&session_id, interaction)
        .await
        .expect("interaction should register");

    assert!(
        router.session_projection_change_sequence() > before_projection_sequence,
        "runtime interaction registration should publish a projection change"
    );
    assert!(
        router.terminal_session_change_sequence(&session_id) > before_terminal_session_sequence,
        "runtime interaction registration should wake terminal subscription waiters"
    );

    let before_resolve_terminal_session_sequence =
        router.terminal_session_change_sequence(&session_id);
    router
        .runtime_state
        .resolve_runtime_interaction(&session_id, "interaction-terminal-wake", "allow_once", None)
        .await
        .expect("interaction should resolve");
    assert!(
        router.terminal_session_change_sequence(&session_id)
            > before_resolve_terminal_session_sequence,
        "runtime interaction resolution should wake terminal subscription waiters"
    );
    let resolution = receiver.await.expect("resolution should be delivered");
    assert_eq!(resolution.choice_id.as_deref(), Some("allow_once"));
    assert_eq!(resolution.reply.as_deref(), Some("allow"));
}

#[tokio::test]
async fn runtime_interaction_rejects_agent_outside_session() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (first_session, _first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session should be created");
    let (second_session, second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session should be created");
    let first_session_id = first_session.id().to_string();
    let second_agent_id = second_agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let interaction = RuntimeInteraction::new(
        "interaction-cross-session",
        &second_agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );

    let error = router
        .runtime_state
        .create_runtime_interaction(&first_session_id, interaction)
        .await
        .expect_err("cross-session runtime interactions should be rejected");

    assert!(matches!(
        error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == first_session_id && agent_id == second_agent_id
    ));
    let first_snapshot = router
        .runtime_state
        .session_snapshot_projection(first_session.id(), 0)
        .expect("first session projection should resolve");
    let second_snapshot = router
        .runtime_state
        .session_snapshot_projection(second_session.id(), 0)
        .expect("second session projection should resolve");
    assert!(first_snapshot.session.active_interactions().is_empty());
    assert!(second_snapshot.session.active_interactions().is_empty());
}

#[tokio::test]
async fn subscription_snapshot_includes_runtime_interaction_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "interaction-subscription",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);

    let initial = router
        .relay_watch_subscription_state(&session_id, &attachment_id, true, None, 0)
        .await;
    let initial_snapshot = match initial {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => snapshot
            .as_ref()
            .clone()
            .expect("initial subscription should include a snapshot"),
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    };
    let interaction = RuntimeInteraction::new(
        "interaction-subscription-1",
        &agent_id,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Info,
        Some("Approve file changes?".to_string()),
        "Approve file changes?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _resolution = router
        .runtime_state
        .create_runtime_interaction(&session_id, interaction)
        .await
        .expect("interaction should register");

    let update = router
        .relay_watch_subscription_state(
            &session_id,
            &attachment_id,
            true,
            Some(initial_snapshot),
            0,
        )
        .await;
    match update {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            let snapshot = snapshot
                .as_ref()
                .as_ref()
                .expect("runtime interaction should change subscription snapshot");
            assert_eq!(snapshot.session.active_interactions().len(), 1);
            assert_eq!(
                snapshot.session.active_interactions()[0].id(),
                "interaction-subscription-1"
            );
        }
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    }
}

#[tokio::test]
async fn dispatched_native_provider_interaction_updates_subscription_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "native-interaction-subscription",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);

    let initial = router
        .relay_watch_subscription_state(&session_id, &attachment_id, true, None, 0)
        .await;
    let initial_snapshot = match initial {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => snapshot
            .as_ref()
            .clone()
            .expect("initial subscription should include a snapshot"),
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    };

    let request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            &session_id,
            &agent_id,
            "native-interaction-dispatch",
            Some("Approve file changes?".to_string()),
            "Approve file changes?".to_string(),
            Some(30),
        ),
    );
    let command = KernelCommand::from_local_request(
        "cmd-native-interaction-subscription",
        None,
        None,
        &request,
    );
    let dispatch_router = router.clone();
    let dispatch_task =
        tokio::spawn(async move { dispatch_router.dispatch(command, request).await });

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let state = router
                .runtime_state
                .session_snapshot_projection(&session_id, 0)
                .expect("session projection should resolve")
                .session;
            if state
                .active_interactions()
                .iter()
                .any(|interaction| interaction.id() == "native-interaction-dispatch")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("native interaction should become active");

    let update = router
        .relay_watch_subscription_state(
            &session_id,
            &attachment_id,
            true,
            Some(initial_snapshot),
            0,
        )
        .await;
    match update {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            let snapshot = snapshot
                .as_ref()
                .as_ref()
                .expect("runtime interaction should change subscription snapshot");
            assert_eq!(snapshot.session.active_interactions().len(), 1);
            assert_eq!(
                snapshot.session.active_interactions()[0].id(),
                "native-interaction-dispatch"
            );
        }
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    }

    router
        .runtime_state
        .resolve_runtime_interaction(
            &session_id,
            "native-interaction-dispatch",
            "allow_once",
            None,
        )
        .await
        .expect("interaction should resolve");
    dispatch_task
        .await
        .expect("dispatch task should join")
        .expect("request should resolve");
}

#[tokio::test]
async fn native_provider_interaction_wakes_subscription_projection_across_routers() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "cross-router-native-interaction-subscription",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment_id = attachment.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let relay_router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let local_router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let initial = relay_router
        .relay_watch_subscription_state(&session_id, &attachment_id, true, None, 0)
        .await;
    let initial_snapshot = match initial {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => snapshot
            .as_ref()
            .clone()
            .expect("initial subscription should include a snapshot"),
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    };

    let before_relay_sequence = relay_router.session_projection_change_sequence();
    let request = LocalDaemonRequest::RequestNativeProviderInteraction(
        RequestNativeProviderInteractionRequest::allow_deny(
            &session_id,
            &agent_id,
            "cross-router-native-interaction",
            Some("Approve file changes?".to_string()),
            "Approve file changes?".to_string(),
            Some(30),
        ),
    );
    let command = KernelCommand::from_local_request(
        "cmd-cross-router-native-interaction",
        None,
        None,
        &request,
    );
    let dispatch_task = tokio::spawn(async move { local_router.dispatch(command, request).await });

    timeout(
        Duration::from_secs(1),
        relay_router.wait_for_session_projection_change_after(before_relay_sequence),
    )
    .await
    .expect("subscription router should wake for cross-router projection changes");
    assert!(
        relay_router.session_projection_change_sequence() > before_relay_sequence,
        "subscription router should observe projection changes published by the request router"
    );

    let update = relay_router
        .relay_watch_subscription_state(
            &session_id,
            &attachment_id,
            true,
            Some(initial_snapshot),
            0,
        )
        .await;
    match update {
        crate::runtime_transport::WatchResult::Ok { snapshot, .. } => {
            let snapshot = snapshot
                .as_ref()
                .as_ref()
                .expect("runtime interaction should change subscription snapshot");
            assert_eq!(snapshot.session.active_interactions().len(), 1);
            assert_eq!(
                snapshot.session.active_interactions()[0].id(),
                "cross-router-native-interaction"
            );
        }
        crate::runtime_transport::WatchResult::Unavailable(message) => {
            panic!("subscription unavailable: {message}")
        }
    }

    relay_router
        .runtime_state
        .resolve_runtime_interaction(
            &session_id,
            "cross-router-native-interaction",
            "allow_once",
            None,
        )
        .await
        .expect("interaction should resolve");
    dispatch_task
        .await
        .expect("dispatch task should join")
        .expect("request should resolve");
}

#[tokio::test]
async fn acknowledge_output_seen_uses_agent_store_membership_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let stored_session = router
        .runtime_state
        .session_snapshot_projection(&session_id, 0)
        .expect("projection should resolve")
        .session;
    assert!(
        stored_session
            .agents()
            .iter()
            .any(|candidate| candidate.id() == agent_id),
        "projection should include the created agent"
    );

    let acknowledged = router
        .runtime_state
        .acknowledge_agent_output_seen(&session_id, &agent_id, DEFAULT_LOCAL_USER_ID)
        .await
        .expect("output acknowledgement should use agent-store membership");

    assert_eq!(acknowledged.session.id(), session_id);
    assert!(
        !acknowledged.changed,
        "acknowledging output with no unread state should be a no-op"
    );
}

#[tokio::test]
async fn duplicate_output_seen_ack_does_not_publish_session_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    router
        .runtime_state
        .session_snapshot_projection(&session_id, 0)
        .expect("projection should resolve");
    let before_projection_sequence = router.session_projection_change_sequence();
    let ack_request =
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(AcknowledgeAgentOutputSeenRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        });
    let ack_command =
        KernelCommand::from_local_request("cmd-output-seen-noop", None, None, &ack_request);

    let response = router
        .dispatch(ack_command, ack_request)
        .await
        .expect("duplicate output acknowledgement should still return success");

    assert!(matches!(
        response,
        LocalDaemonResponse::AgentOutputSeenAcknowledged {
            session_id: ref acknowledged_session_id,
            agent_id: ref acknowledged_agent_id,
        } if acknowledged_session_id == &session_id && acknowledged_agent_id == &agent_id
    ));
    assert_eq!(
        router.session_projection_change_sequence(),
        before_projection_sequence,
        "no-op output acknowledgements must not publish session projections"
    );
}

#[test]
fn output_seen_ack_clears_unread_activity_for_same_user_attachments_only() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(async {
        tokio::spawn(async move {
            output_seen_ack_clears_unread_activity_for_same_user_attachments_only_inner().await
        })
        .await
        .expect("test task should complete");
    });
}

async fn output_seen_ack_clears_unread_activity_for_same_user_attachments_only_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    {
        let mut sessions = app.sessions_mut();
        let mut session = sessions
            .get_session(&session_id)
            .expect("session should be stored");
        session.add_member(
            "user-2",
            Some(DEFAULT_LOCAL_USER_ID.to_string()),
            crate::session::CollaborationLevel::Full,
        );
        sessions.restore_session(session);
    }
    let first_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            &session_id,
            "cli-same-user-a",
            ClientCapabilityLevel::FullTerminal,
            DEFAULT_LOCAL_USER_ID,
        ))
        .expect("first attachment should attach");
    let second_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            &session_id,
            "cli-same-user-b",
            ClientCapabilityLevel::FullTerminal,
            DEFAULT_LOCAL_USER_ID,
        ))
        .expect("second attachment should attach");
    let collaborator_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::for_user(
            &session_id,
            "cli-collaborator",
            ClientCapabilityLevel::FullTerminal,
            "user-2",
        ))
        .expect("collaborator attachment should attach");
    app.sessions_mut()
        .note_agent_output_sequence(&session_id, &agent_id, 7)
        .expect("agent output sequence should be noted");

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let before_first = router
        .session_snapshot_projection_for_attachment(&session_id, first_attachment.id(), 0)
        .expect("first attachment projection should resolve");
    let before_second = router
        .session_snapshot_projection_for_attachment(&session_id, second_attachment.id(), 0)
        .expect("second attachment projection should resolve");
    let before_collaborator = router
        .session_snapshot_projection_for_attachment(&session_id, collaborator_attachment.id(), 0)
        .expect("collaborator attachment projection should resolve");
    assert!(
        before_first.agent_activity[&agent_id].unread_idle_output,
        "first same-user attachment should see unread output"
    );
    assert!(
        before_second.agent_activity[&agent_id].unread_idle_output,
        "second same-user attachment should see unread output"
    );
    assert!(
        before_collaborator.agent_activity[&agent_id].unread_idle_output,
        "collaborator should have an independent unread receipt"
    );
    let public_snapshot_request = LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
        crate::local::GetWaitingRoomPublicSnapshotRequest,
    );
    let public_snapshot_command = KernelCommand::from_local_request(
        "cmd-output-unseen-waiting-room",
        None,
        None,
        &public_snapshot_request,
    );
    let response = router
        .dispatch(public_snapshot_command, public_snapshot_request)
        .await
        .expect("waiting room unread refresh should succeed");
    let LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } = response else {
        panic!("unexpected waiting room response before ack");
    };
    let unread_session = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == session_id)
        .expect("session should be in unread waiting room snapshot");
    let unread_agent = unread_session
        .agents
        .iter()
        .find(|candidate| candidate.id == agent_id)
        .expect("agent should be in unread waiting room snapshot");
    assert_eq!(unread_session.activity.unread_idle_agent_count, 1);
    assert!(unread_agent.activity.unread_idle_output);

    let ack_request =
        LocalDaemonRequest::AcknowledgeAgentOutputSeen(AcknowledgeAgentOutputSeenRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        });
    let ack_command = KernelCommand::from_local_request(
        "cmd-output-seen-cross-attachment",
        None,
        None,
        &ack_request,
    );
    router
        .dispatch(ack_command, ack_request)
        .await
        .expect("output acknowledgement should succeed");

    let after_first = router
        .session_snapshot_projection_for_attachment(&session_id, first_attachment.id(), 1)
        .expect("first attachment projection should resolve after ack");
    let after_second = router
        .session_snapshot_projection_for_attachment(&session_id, second_attachment.id(), 1)
        .expect("second attachment projection should resolve after ack");
    let after_collaborator = router
        .session_snapshot_projection_for_attachment(&session_id, collaborator_attachment.id(), 1)
        .expect("collaborator attachment projection should resolve after ack");
    assert!(
        !after_first.agent_activity[&agent_id].unread_idle_output,
        "ack should clear unread output for the acknowledging user"
    );
    assert!(
        !after_second.agent_activity[&agent_id].unread_idle_output,
        "ack should clear unread output in another same-user terminal"
    );
    assert!(
        after_collaborator.agent_activity[&agent_id].unread_idle_output,
        "ack should not clear another user's unread output"
    );
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-output-seen-refresh-state",
        None,
        None,
        &state_request,
    );
    let response = router
        .dispatch(state_command, state_request)
        .await
        .expect("session state refresh should succeed");
    assert!(matches!(
        response,
        LocalDaemonResponse::SessionState { agent_activity, .. }
            if !agent_activity[&agent_id].unread_idle_output
    ));

    let public_snapshot_request = LocalDaemonRequest::GetWaitingRoomPublicSnapshot(
        crate::local::GetWaitingRoomPublicSnapshotRequest,
    );
    let public_snapshot_command = KernelCommand::from_local_request(
        "cmd-output-seen-refresh-waiting-room",
        None,
        None,
        &public_snapshot_request,
    );
    let response = router
        .dispatch(public_snapshot_command, public_snapshot_request)
        .await
        .expect("waiting room refresh should succeed");
    let LocalDaemonResponse::WaitingRoomPublicSnapshot { snapshot } = response else {
        panic!("unexpected waiting room response");
    };
    let refreshed_session = snapshot
        .sessions
        .iter()
        .find(|candidate| candidate.id == session_id)
        .expect("session should be in waiting room snapshot");
    let refreshed_agent = refreshed_session
        .agents
        .iter()
        .find(|candidate| candidate.id == agent_id)
        .expect("agent should be in waiting room snapshot");
    assert_eq!(refreshed_session.activity.unread_idle_agent_count, 0);
    assert!(!refreshed_agent.activity.unread_idle_output);
}

#[tokio::test]
async fn update_session_config_uses_session_runtime_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-config-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let update_request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
        requires_idle: false,
    });
    let update_command =
        KernelCommand::from_local_request("cmd-session-config", None, None, &update_request);
    let update_response = router
        .dispatch(update_command, update_request)
        .await
        .expect("session config update should succeed");
    match update_response {
        LocalDaemonResponse::SessionConfigUpdated { config, session } => {
            assert_eq!(config.version(), 1);
            assert_eq!(session.config_state().version(), 1);
            assert_eq!(
                session.config_state().values().get("theme"),
                Some(&"compact".to_string())
            );
        }
        _ => panic!("unexpected config response"),
    }

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-session-config-state", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "session config update should publish a session projection for lock-free state reads"
    );

    drop(app_guard);
    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.config_state().version(), 1);
            assert_eq!(
                session.config_state().values().get("theme"),
                Some(&"compact".to_string())
            );
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn alias_session_uses_session_runtime_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let alias_request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
        session_id: session_id.clone(),
        alias: "review entry".to_string(),
    });
    let alias_command =
        KernelCommand::from_local_request("cmd-session-alias", None, None, &alias_request);
    let alias_response = router
        .dispatch(alias_command, alias_request)
        .await
        .expect("session alias should succeed");
    match alias_response {
        LocalDaemonResponse::SessionAliased { session } => {
            assert_eq!(session.alias(), Some("review_entry"));
        }
        _ => panic!("unexpected alias response"),
    }

    let app_guard = app.lock().await;
    let resolve_request = LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
        session_ref: "review_entry".to_string(),
        workspace_id: Some("workspace".to_string()),
    });
    let resolve_command = KernelCommand::from_local_request(
        "cmd-session-alias-resolve",
        None,
        None,
        &resolve_request,
    );
    let resolve_router = router.clone();
    let resolve_task = tokio::spawn(async move {
        resolve_router
            .dispatch(resolve_command, resolve_request)
            .await
    });

    tokio::task::yield_now().await;
    assert!(
        resolve_task.is_finished(),
        "session alias should publish a projection that resolves without app lock access"
    );

    drop(app_guard);
    let resolve_response = resolve_task
        .await
        .expect("resolve task should join")
        .expect("resolve should succeed");
    match resolve_response {
        LocalDaemonResponse::SessionResolved { session } => {
            assert_eq!(session.id(), session_id);
            assert_eq!(session.alias(), Some("review_entry"));
        }
        _ => panic!("unexpected resolve response"),
    }
}

#[tokio::test]
async fn poll_runtime_notices_routes_through_session_runtime() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let source = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-notice-source",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("source attachment should attach");
    let recipient = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-notice-recipient",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("recipient attachment should attach");
    app.record_notice(
        &session_id,
        None,
        vec![recipient.id().to_string()],
        format!(
            "Attachment `{}` updated configuration for session `{}`.",
            source.id(),
            session_id
        ),
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-runtime-notices-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm session projection");

    let app_guard = app.lock().await;
    let poll_request = LocalDaemonRequest::PollRuntimeNotices(PollRuntimeNoticesRequest {
        session_id: session_id.clone(),
        attachment_id: recipient.id().to_string(),
    });
    let poll_command =
        KernelCommand::from_local_request("cmd-runtime-notices", None, None, &poll_request);
    let poll_router = router.clone();
    let poll_task =
        tokio::spawn(async move { poll_router.dispatch(poll_command, poll_request).await });
    let poll_response = timeout(Duration::from_millis(100), poll_task)
        .await
        .expect("notice poll should not wait for the app lock")
        .expect("poll task should join")
        .expect("notice poll should succeed");
    drop(app_guard);

    assert!(
        router.session_runtime.has_lane(&session_id).await,
        "notice polling should be admitted through the per-session runtime lane"
    );
    match poll_response {
        LocalDaemonResponse::RuntimeNotices { notices } => {
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].session_id, session_id);
        }
        _ => panic!("unexpected notice response"),
    }
}

#[tokio::test]
async fn resize_without_active_run_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-resize-no-active-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm session projection");

    let app_guard = app.lock().await;
    let resize_request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
        session_id: session_id.clone(),
        cols: 120,
        rows: 40,
    });
    let resize_command = KernelCommand::from_local_request(
        "cmd-resize-no-active-projection",
        None,
        None,
        &resize_request,
    );
    let resize_router = router.clone();
    let resize_task =
        tokio::spawn(async move { resize_router.dispatch(resize_command, resize_request).await });

    let error = timeout(Duration::from_millis(100), resize_task)
        .await
        .expect("resize absence should not wait for the app lock")
        .expect("resize task should join")
        .expect_err("resize without active provider run should fail");
    drop(app_guard);

    match error {
        DaemonError::NoActiveProviderRun {
            session_id: error_session_id,
        } => assert_eq!(error_session_id, session_id),
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn get_session_state_projection_tracks_prompt_completion_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-complete-projection",
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
        prompt: "complete projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-complete-state", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent runtime projection should track prompt state after submit");
    assert!(prompt_projection.active_prompt.is_some());
    assert_eq!(prompt_projection.queued_prompt_count, 0);

    let before_complete_projection_sequence = router.session_projection_change_sequence();
    let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session_id.clone(),
    });
    let complete_command = KernelCommand::from_local_request(
        "cmd-complete-state-projection",
        None,
        None,
        &complete_request,
    );
    router
        .dispatch(complete_command, complete_request)
        .await
        .expect("prompt completion should publish session projection through agent runtime");
    assert!(
        router.session_projection_change_sequence() > before_complete_projection_sequence,
        "prompt completion should wake session projection subscribers"
    );
    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent runtime projection should retain prompt state after complete");
    assert!(prompt_projection.active_prompt.is_none());
    assert_eq!(prompt_projection.queued_prompt_count, 0);

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-state-complete-projection",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "completed prompt state should be served from projection without app lock access"
    );
    drop(app_guard);

    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert!(session.active_prompt_for_agent(&agent_id).is_none());
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn session_snapshot_refresh_tracks_agent_runtime_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-prompt-shadow-refresh",
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
        prompt: "shadow refresh".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-shadow-submit", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm agent runtime projection");
    assert!(router
        .agent_runtime_projection
        .get(&agent_id)
        .and_then(|projection| projection.active_prompt)
        .is_some());

    {
        let app = app.lock().await;
        app.sessions_mut()
            .complete_active_prompt_only(&session_id, &agent_id)
            .expect("compatibility state should be externally settled");
    }
    assert!(
        router
            .agent_runtime_projection
            .get(&agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some(),
        "prompt projection should stay stale until a session snapshot is observed"
    );

    let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
    });
    let pump_command =
        KernelCommand::from_local_request("cmd-shadow-refresh", None, None, &pump_request);
    router
        .dispatch(pump_command, pump_request)
        .await
        .expect("snapshot-producing pump should refresh projections");

    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent prompt projection should remain registered");
    assert!(prompt_projection.active_prompt.is_none());
    assert_eq!(prompt_projection.queued_prompt_count, 0);
}

#[tokio::test]
async fn prompt_complete_uses_agent_runtime_projection_when_session_projection_is_stale() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let default_agent_id = default_agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-complete-owner-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
    let spawned_agent_id = spawned_agent.id().to_string();
    launch_test_provider(
        &mut app,
        &session_id,
        &spawned_agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    focus_test_agent(&mut app, &session_id, &default_agent_id);
    let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("idle session snapshot should be available");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(spawned_agent_id.clone()),
        prompt: "complete owner projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-complete-owner", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    router.session_projection.update(idle_session_snapshot);

    let app_guard = app.lock().await;
    let complete_request = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session_id.clone(),
    });
    let complete_command = KernelCommand::from_local_request(
        "cmd-complete-owner-projection",
        None,
        None,
        &complete_request,
    );
    let complete_router = router.clone();
    let complete_task = tokio::spawn(async move {
        complete_router
            .dispatch(complete_command, complete_request)
            .await
    });

    let mut spawned_agent_lane_created = false;
    for _ in 0..50 {
        let projection = router.daemon_health_projection(0).await;
        if projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == spawned_agent_id)
        {
            spawned_agent_lane_created = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        spawned_agent_lane_created,
        "prompt complete should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
    );
    assert!(
        !complete_task.is_finished(),
        "agent worker should still wait on the deliberately held app lock"
    );

    drop(app_guard);
    let complete_response = complete_task
        .await
        .expect("complete task should join")
        .expect("prompt should complete");
    match complete_response {
        LocalDaemonResponse::PromptCompleted { .. } => {}
        _ => panic!("unexpected complete response"),
    }
}

#[tokio::test]
async fn get_session_state_projection_tracks_prompt_cancellation_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-cancel-projection",
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
        prompt: "cancel projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-cancel-state", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    assert!(router
        .agent_runtime_projection
        .get(&agent_id)
        .and_then(|projection| projection.active_prompt)
        .is_some());

    let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
    });
    let cancel_command = KernelCommand::from_local_request(
        "cmd-cancel-state-projection",
        None,
        None,
        &cancel_request,
    );
    router
        .dispatch(cancel_command, cancel_request)
        .await
        .expect("prompt cancellation should publish session projection");
    let prompt_projection = router
        .agent_runtime_projection
        .get(&agent_id)
        .expect("agent runtime projection should retain prompt state after cancel");
    assert_eq!(
        prompt_projection
            .active_prompt
            .as_ref()
            .map(|prompt| prompt.status()),
        Some(PromptStatus::Cancelling)
    );

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-state-cancel-projection",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "cancelled prompt state should be served from projection without app lock access"
    );
    drop(app_guard);

    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            let active_prompt = session
                .active_prompt_for_agent(&agent_id)
                .expect("prompt should still be settling");
            assert_eq!(active_prompt.status(), PromptStatus::Cancelling);
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn prompt_cancel_uses_agent_runtime_projection_when_session_projection_is_stale() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let default_agent_id = default_agent.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-cancel-owner-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let spawned_agent = spawn_test_agent(&mut app, &session_id, "worker", "claude-code");
    let spawned_agent_id = spawned_agent.id().to_string();
    launch_test_provider(
        &mut app,
        &session_id,
        &spawned_agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    focus_test_agent(&mut app, &session_id, &default_agent_id);
    let idle_session_snapshot = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("idle session snapshot should be available");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
        target_agent_id: Some(spawned_agent_id.clone()),
        prompt: "cancel owner projection".to_string(),
        attachments: Vec::new(),
    });
    let prompt_command =
        KernelCommand::from_local_request("cmd-prompt-cancel-owner", None, None, &prompt_request);
    router
        .dispatch(prompt_command, prompt_request)
        .await
        .expect("prompt submit should warm active prompt projection");
    router.session_projection.update(idle_session_snapshot);

    let app_guard = app.lock().await;
    let cancel_request = LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
    });
    let cancel_command = KernelCommand::from_local_request(
        "cmd-cancel-owner-projection",
        None,
        None,
        &cancel_request,
    );
    let cancel_router = router.clone();
    let cancel_task =
        tokio::spawn(async move { cancel_router.dispatch(cancel_command, cancel_request).await });

    let mut spawned_agent_lane_created = false;
    for _ in 0..50 {
        let projection = router.daemon_health_projection(0).await;
        if projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == spawned_agent_id)
        {
            spawned_agent_lane_created = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        spawned_agent_lane_created,
        "prompt cancel should resolve the active prompt owner from the agent-runtime projection before touching the app lock"
    );
    assert!(
        !cancel_task.is_finished(),
        "agent worker should still wait on the deliberately held app lock"
    );

    drop(app_guard);
    let cancel_response = cancel_task
        .await
        .expect("cancel task should join")
        .expect("prompt should cancel");
    match cancel_response {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(cancellation.prompt.status(), PromptStatus::Cancelling);
        }
        _ => panic!("unexpected cancel response"),
    }
}
