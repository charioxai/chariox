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
                target_agent_id: None,
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
fn direct_prompt_cancel_uses_explicit_target_agent_when_multiple_agents_are_active() {
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

    for (agent, prompt_text) in [
        (&default_agent, "default running"),
        (&prompt_agent, "prompt-agent running"),
    ] {
        match harness
            .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent.id().to_string()),
                prompt: prompt_text.to_string(),
                attachments: Vec::new(),
            }))
            .expect("prompt submit should start")
        {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { prompt },
                ..
            } => assert_eq!(prompt.target_agent_id(), agent.id()),
            other => panic!("unexpected local response: {other:?}"),
        }
    }

    let _ = harness
        .dispatch(LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session.id().to_string(),
            agent_id: default_agent.id().to_string(),
        }))
        .expect("focus should stay on the default agent");

    match harness
        .dispatch(LocalDaemonRequest::CancelActivePrompt(
            CancelActivePromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(prompt_agent.id().to_string()),
            },
        ))
        .expect("cancel should use the explicit target agent")
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
    assert_eq!(
        session_state
            .active_prompt_for_agent(default_agent.id())
            .map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Running)
    );
    assert_eq!(
        session_state
            .active_prompt_for_agent(prompt_agent.id())
            .map(|prompt| prompt.status()),
        Some(crate::session::PromptStatus::Cancelling)
    );
}

#[test]
fn local_request_api_rejects_invalid_provider_adapter() {
    run_with_large_test_stack(
        "invalid-provider-adapter",
        local_request_api_rejects_invalid_provider_adapter_inner,
    );
}

fn local_request_api_rejects_invalid_provider_adapter_inner() {
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
