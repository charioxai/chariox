use super::*;

#[test]
fn local_request_api_rejects_config_updates_for_native_tui_provider_agents() {
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };

    harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "codex", "default", "gpt-5.4")
                .with_agent_id(agent.id())
                .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("native TUI provider launch should succeed");
    });

    let profile_error = harness
        .dispatch(LocalDaemonRequest::UpdateAgentProfile(
            UpdateAgentProfileRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                provider: Some("codex".to_string()),
                model: Some("gpt-5.5".to_string()),
                effort: Some("high".to_string()),
                clear_effort: false,
            },
        ))
        .expect_err("native TUI provider profile should be read-only from Arroba");
    assert_native_tui_config_error(profile_error, "update agent profile");

    let config_error = harness
        .dispatch(LocalDaemonRequest::UpdateAgentConfig(
            UpdateAgentConfigRequest {
                session_id: session.id().to_string(),
                agent_id: agent.id().to_string(),
                execution_mode: Some(AgentExecutionMode::Plan),
                clear_execution_mode: false,
                permission_level: Some(AgentPermissionLevel::Required),
                clear_permission_level: false,
                workspace_id: None,
                clear_workspace_id: false,
                worktree_id: None,
                clear_worktree_id: false,
            },
        ))
        .expect_err("native TUI provider config should be read-only from Arroba");
    assert_native_tui_config_error(config_error, "update agent config");
}

fn assert_native_tui_config_error(error: DaemonError, operation: &'static str) {
    match error {
        DaemonError::LocalTransport {
            operation: actual,
            message,
        } => {
            assert_eq!(actual, operation);
            assert!(message.contains("provider-native TUI"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn app_submit_prompt_rejects_agent_from_another_session() {
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
            "cross-session-prompt",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");

    let error = app
        .submit_prompt(
            second_session.id(),
            attachment.id(),
            Some(first_agent.id()),
            "must not cross session boundary",
            Vec::new(),
        )
        .expect_err("prompt submission should reject an agent outside the requested session");

    assert!(matches!(
        error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == second_session.id() && agent_id == first_agent.id()
    ));
    assert!(app
        .providers()
        .get_latest_run_for_agent(first_session.id(), first_agent.id())
        .is_none());
    assert!(app
        .providers()
        .get_latest_run_for_agent(second_session.id(), first_agent.id())
        .is_none());
}

#[test]
fn app_prompt_settlement_rejects_agent_from_another_session() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (_first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session should be created");
    let (second_session, _second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session should be created");

    let complete_error = app
        .complete_active_prompt(second_session.id(), first_agent.id(), None)
        .expect_err("prompt completion should reject an agent outside the requested session");
    assert!(matches!(
        complete_error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == second_session.id() && agent_id == first_agent.id()
    ));

    let cancel_error = crate::app::KernelAgentService::new(&mut app)
        .cancel_active_prompt_internal(second_session.id(), first_agent.id(), None)
        .expect_err("prompt cancellation should reject an agent outside the requested session");
    assert!(matches!(
        cancel_error,
        DaemonError::AgentNotInSession {
            session_id,
            agent_id,
        } if session_id == second_session.id() && agent_id == first_agent.id()
    ));
}

#[test]
fn focusing_another_agent_during_a_prompt_keeps_the_working_run_active() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let _default_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("default provider launch should succeed")
    });

    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("reviewer".to_string()),
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
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let _focused_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(session.id(), "dev-stub", "claude-code", "default", "opus")
                .with_agent_id(spawned.id()),
        )
        .expect("spawned provider launch should succeed")
    });

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focusing default agent should succeed");

    let started = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: None,
            prompt: "keep streaming while focus changes\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt should start");

    match started {
        LocalDaemonResponse::PromptSubmitted { outcome, .. } => match outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.target_agent_id(), default_agent.id());
            }
            _ => panic!("expected prompt to start immediately"),
        },
        _ => panic!("unexpected local response"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: spawned.id().to_string(),
        }))
        .expect("focusing spawned agent should succeed");

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(_default_run.id())
    );

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_output = false;
    while Instant::now() < deadline {
        let records = harness.with_app_mut(|app| {
            crate::app::provider_output::pump_terminal_output_for_attachment(
                app,
                session.id(),
                attachment.id(),
            )
            .expect("terminal output should keep pumping")
        });
        if !records.is_empty() {
            saw_output = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        saw_output,
        "expected background agent output to continue while unfocused"
    );

    harness.with_app_mut(|app| {
        pump_active_prompt_outputs(app);
    });
    harness.with_app(|app| {
        let session_state = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
        assert_eq!(
            session_state.active_provider_run_id(),
            Some(_default_run.id())
        );
        assert!(
            session_state
                .active_prompt_for_agent(default_agent.id())
                .is_some(),
            "background prompt should remain owned by the original agent while unfocused"
        );
    });
}

#[test]
fn spawning_agent_during_active_prompt_keeps_snapshot_on_working_run() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let default_run = harness.with_app_mut(|app| {
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            )
            .with_agent_id(default_agent.id()),
        )
        .expect("provider run should launch")
    });

    harness
        .with_app_mut(|app| {
            app.submit_prompt(
                session.id(),
                attachment.id(),
                Some(default_agent.id()),
                "keep working\n",
                Vec::new(),
            )
        })
        .expect("prompt should start");
    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("observer".to_string()),
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
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let session_state = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        _ => panic!("unexpected local response"),
    };

    assert_eq!(session_state.focused_agent_id(), Some(spawned.id()));
    assert_eq!(
        session_state.active_provider_run_id(),
        Some(default_run.id()),
        "snapshots must keep the still-running provider visible for recovery and stream routing"
    );
}

#[test]
fn local_request_api_auto_launches_provider_run_for_prompt() {
    let harness = LocalRouterTestHarness::new();
    let (session, _default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let response = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should auto-launch a provider run");

    match response {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            session,
            ..
        } => {
            assert_eq!(prompt.status(), crate::session::PromptStatus::Running);
            assert!(session.active_provider_run_id().is_some());
            assert!(session.active_prompt_for_agent(prompt_agent.id()).is_some());
        }
        other => panic!("unexpected local response: {other:?}"),
    }
}

#[test]
fn direct_prompt_completion_resolves_unfocused_single_active_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => assert_eq!(prompt.target_agent_id(), prompt_agent.id()),
        other => panic!("unexpected local response: {other:?}"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should move to the idle default agent");

    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("completion should resolve the single active agent")
    {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert_eq!(completion.completed.target_agent_id(), prompt_agent.id());
            assert!(completion.started_next.is_none());
        }
        other => panic!("unexpected local response: {other:?}"),
    }

    let session_state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .clone()
    });
    assert_eq!(session_state.focused_agent_id(), Some(default_agent.id()));
    assert!(session_state
        .active_prompt_for_agent(prompt_agent.id())
        .is_none());
}

#[test]
fn completed_native_tui_turn_projects_undo_action_for_tracked_session() {
    let root = temp_git_repo("native-tui-turn-actions");
    let proof_path = root.join("web-terminal-undo-proof.txt");
    let harness = LocalRouterTestHarness::new();
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "turn-actions-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let provider_run = match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: Some("default".to_string()),
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: true,
            },
        ))
        .expect("provider launch should succeed")
    {
        LocalDaemonResponse::ProviderRunLaunched { provider_run }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => provider_run,
        other => panic!("unexpected provider launch response: {other:?}"),
    };
    assert!(
        provider_run.tracks_workspace_live_sync(),
        "tracked session should launch a tracked provider run"
    );
    harness.with_app_mut(|app| {
        if app
            .providers()
            .get_run(provider_run.id())
            .is_ok_and(|run| run.state() != crate::provider::ProviderRunState::Running)
        {
            app.providers_mut()
                .mark_run_running(provider_run.id())
                .expect("provider run should be marked running");
        }
    });

    let prompt = match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent.id().to_string()),
            prompt: "turn actions prompt".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => prompt,
        other => panic!("unexpected prompt submit response: {other:?}"),
    };
    fs::write(&proof_path, "created by native tui turn\n").expect("proof file should be written");
    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("prompt completion should succeed")
    {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert_eq!(completion.completed.id(), prompt.id());
        }
        other => panic!("unexpected completion response: {other:?}"),
    }

    let agent_activity = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity,
        other => panic!("unexpected session state response: {other:?}"),
    };
    let completed_turn = agent_activity
        .get(agent.id())
        .and_then(|activity| activity.last_completed_turn.as_ref())
        .expect("completed tracked turn should project a turn action");
    assert_eq!(completed_turn.agent_id, agent.id());
    assert_eq!(completed_turn.prompt_id, prompt.id());
    assert!(completed_turn.undo_available);
    assert_eq!(
        completed_turn.changed_paths,
        vec!["web-terminal-undo-proof.txt".to_string()]
    );
    let event_projection = harness.with_app_mut(|app| {
        crate::runtime::projection::SessionSnapshotProjection::from_daemon_app(app, session.id(), 0)
            .expect("subscription projection should load")
    });
    let event_completed_turn = event_projection
        .agent_activity
        .get(agent.id())
        .and_then(|activity| activity.last_completed_turn.as_ref())
        .expect("subscription projection should include completed tracked turn action");
    assert_eq!(event_completed_turn.prompt_id, prompt.id());
    assert!(event_completed_turn.undo_available);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn queued_native_tui_turn_projects_undo_action_after_provider_launch() {
    let root = temp_git_repo("queued-native-tui-turn-actions");
    let proof_path = root.join("web-terminal-undo-proof.txt");
    let mut config = DaemonConfig::for_tests();
    config.provider_runtime_init_delay_ms = 50;
    let harness = LocalRouterTestHarness::with_config(config);
    let (session, agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_workspace_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected local response: {other:?}"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "queued-turn-actions-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        other => panic!("unexpected local response: {other:?}"),
    };
    let provider_run = match harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: Some(agent.id().to_string()),
                adapter_key: "dev-stub".to_string(),
                provider: "dev-stub".to_string(),
                account_profile: "default".to_string(),
                model: "default".to_string(),
                variant: Some("default".to_string()),
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: true,
            },
        ))
        .expect("provider launch should be accepted")
    {
        LocalDaemonResponse::ProviderRunLaunched { provider_run }
        | LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => provider_run,
        other => panic!("unexpected provider launch response: {other:?}"),
    };

    let prompt = match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent.id().to_string()),
            prompt: "queued turn actions prompt".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should queue while native TUI provider starts")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Queued { prompt },
            ..
        } => prompt,
        other => panic!("unexpected prompt submit response: {other:?}"),
    };

    harness.wait_for_session_where(
        session.id(),
        "queued native TUI prompt should become active after provider launch",
        |session| {
            session
                .active_prompt_for_agent(agent.id())
                .is_some_and(|active| active.id() == prompt.id())
        },
    );
    fs::write(&proof_path, "created by queued native tui turn\n")
        .expect("proof file should be written");
    match harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("queued prompt completion should succeed")
    {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert_eq!(completion.completed.id(), prompt.id());
        }
        other => panic!("unexpected completion response: {other:?}"),
    }

    let agent_activity = match harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("session state should load")
    {
        LocalDaemonResponse::SessionState { agent_activity, .. } => agent_activity,
        other => panic!("unexpected session state response: {other:?}"),
    };
    let completed_turn = agent_activity
        .get(agent.id())
        .and_then(|activity| activity.last_completed_turn.as_ref())
        .expect("completed queued tracked turn should project a turn action");
    assert_eq!(completed_turn.agent_id, agent.id());
    assert_eq!(completed_turn.prompt_id, prompt.id());
    assert_eq!(completed_turn.provider_run_id, provider_run.id());
    assert!(completed_turn.undo_available);
    assert_eq!(
        completed_turn.changed_paths,
        vec!["web-terminal-undo-proof.txt".to_string()]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn direct_prompt_cancel_resolves_unfocused_single_active_agent() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    match harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "whoami".to_string(),
            attachments: Vec::new(),
        }))
        .expect("prompt submit should start")
    {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { prompt },
            ..
        } => assert_eq!(prompt.target_agent_id(), prompt_agent.id()),
        other => panic!("unexpected local response: {other:?}"),
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should move to the idle default agent");

    match harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("cancel should resolve the single active agent")
    {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(cancellation.prompt.target_agent_id(), prompt_agent.id());
            assert!(cancellation.started_next.is_none());
        }
        other => panic!("unexpected local response: {other:?}"),
    }

    let session_state = harness.with_app(|app| {
        app.sessions()
            .get_session(session.id())
            .expect("session should still exist")
            .clone()
    });
    assert_eq!(session_state.focused_agent_id(), Some(default_agent.id()));
    assert_eq!(
        session_state
            .active_prompt_for_agent(prompt_agent.id())
            .map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Cancelling)
    );
}

#[test]
fn local_request_api_rejects_invalid_provider_adapter() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::LaunchProviderRun(
            LaunchProviderRunRequest {
                session_id: session.id().to_string(),
                agent_id: None,
                adapter_key: "missing-adapter".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ))
        .expect_err("unknown adapters should be rejected");

    match error {
        DaemonError::ProviderAdapterNotFound { adapter_key } => {
            assert_eq!(adapter_key, "missing-adapter")
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn local_request_api_exposes_queue_config_and_notices() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };
    let a = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let b = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-b".to_string(),
                capability_level: ClientCapabilityLevel::InteractiveStructured,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    harness.with_app_mut(|app| {
        app.launch_provider(LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "claude-code",
            "default",
            "sonnet",
        ))
        .expect("provider launch should succeed");
    });

    let first = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: a.id().to_string(),
            target_agent_id: None,
            prompt: "first".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start");
    let second = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: b.id().to_string(),
            target_agent_id: None,
            prompt: "second".to_string(),
            attachments: Vec::new(),
        }))
        .expect("second prompt should queue");
    let config = harness
        .dispatch(LocalDaemonRequest::UpdateSessionConfig(
            UpdateSessionConfigRequest {
                session_id: session.id().to_string(),
                attachment_id: a.id().to_string(),
                values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                requires_idle: false,
            },
        ))
        .expect("config update should succeed");

    match first {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            session,
            ..
        } => {
            assert!(session.active_prompt().is_some());
        }
        _ => panic!("unexpected first prompt response"),
    }
    match second {
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Queued { .. },
            session,
            ..
        } => {
            assert_eq!(session.queued_prompts().len(), 1);
        }
        _ => panic!("unexpected second prompt response"),
    }
    match config {
        LocalDaemonResponse::SessionConfigUpdated { config, session } => {
            assert_eq!(config.version(), 1);
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected config response"),
    }

    let notices = harness
        .dispatch(LocalDaemonRequest::PollRuntimeNotices(
            PollRuntimeNoticesRequest {
                session_id: session.id().to_string(),
                attachment_id: b.id().to_string(),
            },
        ))
        .expect("notice polling should succeed");
    match notices {
        LocalDaemonResponse::RuntimeNotices { notices } => assert!(!notices.is_empty()),
        _ => panic!("unexpected notices response"),
    }

    let state = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("state request should succeed");
    match state {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.queued_prompts().len(), 1);
            assert_eq!(session.config_state().version(), 1);
        }
        _ => panic!("unexpected state response"),
    }

    let completed = harness
        .dispatch(LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
            session_id: session.id().to_string(),
        }))
        .expect("complete prompt should succeed");
    match completed {
        LocalDaemonResponse::PromptCompleted { completion } => {
            assert!(completion.started_next.is_some())
        }
        _ => panic!("unexpected completion response"),
    }
}

#[test]
fn local_request_api_can_cancel_an_active_prompt() {
    let harness = LocalRouterTestHarness::new();

    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent: _ } => session,
        _ => panic!("unexpected local response"),
    };

    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-a".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };

    let prompt_agent = harness.spawn_workflow_test_agent(session.id(), "prompt-agent");

    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "first prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("first prompt should start");
    let _ = harness
        .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session.id().to_string(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(prompt_agent.id().to_string()),
            prompt: "second prompt\n".to_string(),
            attachments: Vec::new(),
        }))
        .expect("second prompt should queue");

    let response = harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
            },
        ))
        .expect("cancel should succeed");

    match response {
        LocalDaemonResponse::PromptCancelled { cancellation } => {
            assert_eq!(
                cancellation.prompt.status(),
                crate::session::PromptStatus::Cancelling
            );
            assert!(cancellation.started_next.is_none());
        }
        _ => panic!("unexpected local response"),
    }
}

fn temp_git_repo(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "arroba-{label}-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    fs::create_dir_all(&root).expect("temp repo should be created");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "tests@example.invalid"]);
    run_git(&root, &["config", "user.name", "Arroba Tests"]);
    fs::write(root.join("README.md"), "turn actions seed\n").expect("seed file should be written");
    run_git(&root, &["add", "README.md"]);
    run_git(&root, &["commit", "-m", "initial"]);
    root
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
