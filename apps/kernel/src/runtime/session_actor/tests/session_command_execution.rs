use super::*;

#[tokio::test]
async fn create_session_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let session_projection = SessionStateProjectionStore::default();
    let agent_runtime_projection = AgentRuntimeProjectionStore::default();
    let terminal_stream = {
        let app_locked = app.lock().await;
        app_locked.terminal_stream_store()
    };
    let durable_state_store = {
        let app_locked = app.lock().await;
        app_locked.durable_state_store()
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        agent_runtime_projection.clone(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
        "owned-workspace",
        "owned-worktree",
    ));
    let command = KernelCommand::from_local_request("owned-session-create", None, None, &request);
    let locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("owned create-session path should not wait for the app lock")
    .expect("session creation should succeed");

    let LocalDaemonResponse::SessionCreated { session, agent } = response else {
        panic!("unexpected response");
    };
    assert_eq!(session.workspace_id(), "owned-workspace");
    assert_eq!(session.alias(), Some("owned-workspace-1"));
    assert_eq!(agent.session_id(), session.id());
    assert_eq!(session.focused_agent_id(), Some(agent.id()));
    drop(locked_app);
    let durable_events = durable_state_store
        .load_events_after(0)
        .expect("durable state events should load");
    assert!(
        durable_events.iter().any(|event| {
            event.kind == "session.created"
                && event.subject_id.as_deref() == Some(session.id())
                && event
                    .payload
                    .get("default_agent")
                    .and_then(|agent| agent.get("id"))
                    .and_then(|id| id.as_str())
                    == Some(agent.id())
        }),
        "owned runtime create-session path should persist the session.created durable event"
    );
    assert!(session_projection.get(session.id()).is_some());
    assert!(
        agent_runtime_projection
            .get(agent.id())
            .filter(|projection| projection.session_id == session.id())
            .is_some(),
        "session runtime should publish agent-runtime projection from the owned create response"
    );
}

#[tokio::test]
async fn create_session_rejects_deprecated_metaagent_default_agent() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let terminal_stream = {
        let app_locked = app.lock().await;
        app_locked.terminal_stream_store()
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("meta-workspace", "meta-worktree").with_metaagent(true),
    );
    let command = KernelCommand::from_local_request("meta-session-create", None, None, &request);
    let error = runtime
        .dispatch_session_command(command, request)
        .await
        .expect_err("metaagent session creation should be deprecated");
    assert!(
        error
            .to_string()
            .contains("send `/meta <task>` to enter meta mode"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn create_session_rejects_deprecated_metaagent_in_slice_before_slice_setup() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let terminal_stream = {
        let app_locked = app.lock().await;
        app_locked.terminal_stream_store()
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("slice-workspace", "slice-worktree")
            .with_slice_ref("linux-dev")
            .with_metaagent(true),
    );
    let command =
        KernelCommand::from_local_request("slice-meta-session-create", None, None, &request);
    let error = runtime
        .dispatch_session_command(command, request)
        .await
        .expect_err("slice-backed metaagent session creation should fail");

    assert!(
        error
            .to_string()
            .contains("send `/meta <task>` to enter meta mode"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn update_session_config_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, attachment_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_locked)
            .attach(AttachRequest::new(
                session.id(),
                "config-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        (
            session.id().to_string(),
            attachment.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let session_projection = SessionStateProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
        session_id: session_id.clone(),
        attachment_id,
        values: [("mode".to_string(), "owned".to_string())].into(),
        requires_idle: false,
    });
    let command = KernelCommand::from_local_request("owned-session-config", None, None, &request);
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("owned config-update path should not wait for the app lock")
    .expect("config update should succeed");

    let LocalDaemonResponse::SessionConfigUpdated { config, session } = response else {
        panic!("unexpected response");
    };
    assert_eq!(session.id(), session_id);
    assert_eq!(
        config.values().get("mode").map(String::as_str),
        Some("owned")
    );
    assert_eq!(
        session
            .config_state()
            .values()
            .get("mode")
            .map(String::as_str),
        Some("owned")
    );
    assert!(session_projection.get(&session_id).is_some());
}

#[tokio::test]
async fn idle_required_session_config_uses_prompt_owner_for_admission() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, agent_id, attachment_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_locked)
            .attach(AttachRequest::new(
                session.id(),
                "config-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app_locked
            .prompt_owner_sync_external_active_prompt(
                session.id(),
                agent.id(),
                Some(
                    PromptQueueItem::new(
                        "external-active-prompt",
                        attachment.id(),
                        agent.id(),
                        "external prompt",
                        PromptStatus::Running,
                    )
                    .with_prompt_origin(PromptOrigin::External),
                ),
            )
            .expect("active prompt should sync");
        (
            session.id().to_string(),
            agent.id().to_string(),
            attachment.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
        session_id: session_id.clone(),
        attachment_id,
        values: [("mode".to_string(), "owned".to_string())].into(),
        requires_idle: true,
    });
    let command =
        KernelCommand::from_local_request("owned-session-config-idle", None, None, &request);
    let error = runtime
        .dispatch_session_command(command, request)
        .await
        .expect_err("prompt owner active prompt should reject idle-required config");

    match error {
        DaemonError::ConfigChangeRejectedWhilePromptRunning {
            session_id: rejected,
        } => {
            assert_eq!(rejected, session_id);
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(
        app.lock()
            .await
            .prompt_owner_active_prompt_for_agent_snapshot(&session_id, &agent_id)
            .expect("prompt owner should read")
            .is_some(),
        "test should leave prompt owner active"
    );
}

#[tokio::test]
async fn idle_required_session_config_ignores_stale_session_prompt_mirror() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, agent_id, attachment_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_locked)
            .attach(AttachRequest::new(
                session.id(),
                "config-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = PromptQueueItem::new(
            "external-active-prompt",
            attachment.id(),
            agent.id(),
            "external prompt",
            PromptStatus::Running,
        )
        .with_prompt_origin(PromptOrigin::External);
        app_locked
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), Some(prompt))
            .expect("active prompt should sync");
        app_locked
            .prompt_owner_sync_external_active_prompt(session.id(), agent.id(), None)
            .expect("prompt owner should become idle");
        app_locked
            .sessions_mut()
            .mirror_agent_prompt_state(
                session.id(),
                agent.id(),
                Some(PromptQueueItem::new(
                    "stale-session-prompt",
                    attachment.id(),
                    agent.id(),
                    "stale prompt",
                    PromptStatus::Running,
                )),
                std::collections::VecDeque::new(),
            )
            .expect("legacy session mirror should be made stale");
        assert!(
            app_locked
                .sessions()
                .get_session(session.id())
                .expect("session should exist")
                .has_any_active_prompt(),
            "legacy session mirror should be stale-active"
        );
        assert!(
            !app_locked
                .prompt_owner_has_any_active_prompt(session.id())
                .expect("prompt owner should read"),
            "prompt owner should be authoritative and idle"
        );
        (
            session.id().to_string(),
            agent.id().to_string(),
            attachment.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
        session_id: session_id.clone(),
        attachment_id,
        values: [("mode".to_string(), "owned".to_string())].into(),
        requires_idle: true,
    });
    let command = KernelCommand::from_local_request(
        "owned-session-config-stale-mirror",
        None,
        None,
        &request,
    );
    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("stale session prompt mirror should not reject idle-required config");

    let LocalDaemonResponse::SessionConfigUpdated { config, session } = response else {
        panic!("unexpected response");
    };
    assert_eq!(session.id(), session_id);
    assert_eq!(
        config.values().get("mode").map(String::as_str),
        Some("owned")
    );
    assert!(
        app.lock()
            .await
            .prompt_owner_active_prompt_for_agent_snapshot(&session_id, &agent_id)
            .expect("prompt owner should read")
            .is_none(),
        "prompt owner should remain idle"
    );
}

#[tokio::test]
async fn alias_session_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (session.id().to_string(), app_locked.terminal_stream_store())
    };
    let session_projection = SessionStateProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
        session_id: session_id.clone(),
        alias: "owned-alias".to_string(),
    });
    let command = KernelCommand::from_local_request("owned-session-alias", None, None, &request);
    let _locked_app = app.lock().await;
    let response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("owned alias path should not wait for the app lock")
    .expect("alias update should succeed");

    let LocalDaemonResponse::SessionAliased { session } = response else {
        panic!("unexpected response");
    };
    assert_eq!(session.id(), session_id);
    assert_eq!(session.alias(), Some("owned-alias"));
    assert_eq!(
        session_projection
            .get(&session_id)
            .and_then(|projected| projected.alias().map(str::to_string)),
        Some("owned-alias".to_string())
    );
}

#[tokio::test]
async fn attach_and_detach_use_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (session.id().to_string(), app_locked.terminal_stream_store())
    };
    let session_projection = SessionStateProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let attach_request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
        session_id: session_id.clone(),
        client_id: "owned-client".to_string(),
        capability_level: ClientCapabilityLevel::FullTerminal,
    });
    let attach_command =
        KernelCommand::from_local_request("owned-attach", None, None, &attach_request);
    let _locked_app = app.lock().await;
    let attach_response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(attach_command, attach_request),
    )
    .await
    .expect("owned attach should not wait for the app lock")
    .expect("attach should succeed");
    let LocalDaemonResponse::SessionAttached { attachment } = attach_response else {
        panic!("unexpected attach response");
    };
    assert_eq!(attachment.session_id(), session_id);
    assert!(
        session_projection
            .get(&session_id)
            .is_some_and(|session| session.has_attachment(attachment.id())),
        "attach should refresh session projection"
    );

    let detach_request = LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest {
        attachment_id: attachment.id().to_string(),
    });
    let detach_command =
        KernelCommand::from_local_request("owned-detach", None, None, &detach_request);
    let detach_response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(detach_command, detach_request),
    )
    .await
    .expect("owned detach should not wait for the app lock")
    .expect("detach should succeed");
    assert!(matches!(
        detach_response,
        LocalDaemonResponse::SessionDetached { .. }
    ));
    assert!(
        session_projection
            .get(&session_id)
            .is_some_and(|session| !session.has_attachment(attachment.id())),
        "detach should refresh session projection"
    );
}

#[tokio::test]
async fn attach_remote_agent_does_not_resolve_worker_provider_run_locally() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, agent_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        app_locked
            .agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("provider-run-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("remote binding should succeed");
        app_locked
            .sessions
            .set_active_provider_run(
                session.id(),
                Some("leased:leased-agent-1:provider-run-1".to_string()),
            )
            .expect("opaque remote run should become active");
        (
            session.id().to_string(),
            agent.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );
    let request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
        session_id: session_id.clone(),
        client_id: "remote-client".to_string(),
        capability_level: ClientCapabilityLevel::FullTerminal,
    });
    let command = KernelCommand::from_local_request("remote-attach", None, None, &request);

    let response = runtime
        .dispatch_session_command(command, request)
        .await
        .expect("remote-backed session attach should succeed");

    assert!(matches!(
        response,
        LocalDaemonResponse::SessionAttached { .. }
    ));

    let replacement_request = LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
        session_id: session_id.clone(),
        client_id: "remote-client".to_string(),
        capability_level: ClientCapabilityLevel::FullTerminal,
    });
    let replacement_command = KernelCommand::from_local_request(
        "remote-attach-replacement",
        None,
        None,
        &replacement_request,
    );

    let replacement = runtime
        .dispatch_session_command(replacement_command, replacement_request)
        .await
        .expect("replacing a remote-backed session attachment should succeed");

    assert!(matches!(
        replacement,
        LocalDaemonResponse::SessionAttached { .. }
    ));

    let focus_request = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
        session_id,
        agent_id,
    });
    let focus_command = KernelCommand::from_local_request(
        "remote-focus-after-attach",
        None,
        None,
        &focus_request,
    );

    let focused = runtime
        .dispatch_session_command(focus_command, focus_request)
        .await
        .expect("focusing the remote agent should not resolve its worker run locally");

    assert!(matches!(focused, LocalDaemonResponse::AgentFocused { .. }));
}

#[tokio::test]
async fn focus_and_cycle_use_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, default_agent_id, extra_agent_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let extra_agent = crate::app::KernelSessionService::new(&mut app_locked)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("cycle-me")
                    .with_worktree("worktree"),
            )
            .expect("extra agent should be created");
        (
            session.id().to_string(),
            default_agent.id().to_string(),
            extra_agent.id().to_string(),
            app_locked.terminal_stream_store(),
        )
    };
    let session_projection = SessionStateProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let focus_request = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
        session_id: session_id.clone(),
        agent_id: default_agent_id.clone(),
    });
    let focus_command =
        KernelCommand::from_local_request("owned-focus", None, None, &focus_request);
    let _locked_app = app.lock().await;
    let focus_response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(focus_command, focus_request),
    )
    .await
    .expect("owned focus should not wait for the app lock")
    .expect("focus should succeed");
    assert!(matches!(
        focus_response,
        LocalDaemonResponse::AgentFocused { .. }
    ));
    assert_eq!(
        session_projection
            .get(&session_id)
            .and_then(|session| session.focused_agent_id().map(str::to_string)),
        Some(default_agent_id)
    );

    let cycle_request = LocalDaemonRequest::CycleAgentFocus(CycleAgentFocusRequest {
        session_id: session_id.clone(),
    });
    let cycle_command =
        KernelCommand::from_local_request("owned-cycle", None, None, &cycle_request);
    let cycle_response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(cycle_command, cycle_request),
    )
    .await
    .expect("owned focus cycle should not wait for the app lock")
    .expect("cycle should succeed");
    let LocalDaemonResponse::AgentFocusCycled { agent: Some(agent) } = cycle_response else {
        panic!("unexpected cycle response");
    };
    assert_eq!(agent.id(), extra_agent_id);
}

#[tokio::test]
async fn owned_multi_agent_reattach_resumes_focused_run_before_focus_cycle() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, attachment_id, default_agent_id, extra_agent_id, default_run_id, extra_run_id) = {
        let mut app_locked = app.lock().await;
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let extra_agent = crate::app::KernelSessionService::new(&mut app_locked)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("cycle-me")
                    .with_worktree("worktree"),
            )
            .expect("extra agent should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app_locked)
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let default_run =
            launch_dev_stub_provider(&mut app_locked, session.id(), default_agent.id(), "default");
        crate::app::KernelSessionService::new(&mut app_locked)
            .focus_agent(session.id(), extra_agent.id())
            .expect("extra agent should focus");
        let extra_run =
            launch_dev_stub_provider(&mut app_locked, session.id(), extra_agent.id(), "extra");
        crate::app::KernelSessionService::new(&mut app_locked)
            .focus_agent(session.id(), default_agent.id())
            .expect("default agent should refocus");
        (
            session.id().to_string(),
            attachment.id().to_string(),
            default_agent.id().to_string(),
            extra_agent.id().to_string(),
            default_run.id().to_string(),
            extra_run.id().to_string(),
        )
    };
    let state = owned_runtime_state(&app).await;

    state
        .detach(&attachment_id)
        .await
        .expect("last attachment should detach cleanly");
    {
        let app_locked = app.lock().await;
        assert_eq!(
            app_locked
                .providers()
                .get_run(&default_run_id)
                .expect("default run should remain")
                .state(),
            crate::provider::ProviderRunState::Parked
        );
        assert_eq!(
            app_locked
                .sessions()
                .get_session(&session_id)
                .expect("session should remain")
                .active_provider_run_id(),
            None
        );
    }

    state
        .attach(AttachRequest::new(
            &session_id,
            "client-b",
            ClientCapabilityLevel::FullTerminal,
        ))
        .await
        .expect("reattach should resume the focused provider run");
    {
        let app_locked = app.lock().await;
        assert_eq!(
            app_locked
                .sessions()
                .get_session(&session_id)
                .expect("session should remain")
                .active_provider_run_id(),
            Some(default_run_id.as_str())
        );
        assert_eq!(
            app_locked
                .providers()
                .get_run(&default_run_id)
                .expect("default run should remain")
                .state(),
            crate::provider::ProviderRunState::Running
        );
    }

    let cycled = state
        .cycle_agent_focus(&session_id, DEFAULT_LOCAL_USER_ID)
        .await
        .expect("cycling focus after reattach should not park an already parked run")
        .expect("another agent should be focused");
    assert_eq!(cycled.id(), extra_agent_id);
    let app_locked = app.lock().await;
    assert_eq!(
        app_locked
            .sessions()
            .get_session(&session_id)
            .expect("session should remain")
            .active_provider_run_id(),
        Some(extra_run_id.as_str())
    );
    assert_eq!(
        app_locked
            .providers()
            .get_run(&extra_run_id)
            .expect("extra run should remain")
            .state(),
        crate::provider::ProviderRunState::Running
    );
    assert_ne!(default_agent_id, extra_agent_id);
}

#[tokio::test]
async fn end_and_delete_use_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (
        end_session_id,
        end_agent_id,
        end_cursor_key,
        delete_session_id,
        delete_agent_id,
        delete_cursor_key,
        terminal_stream,
    ) = {
        let mut app_locked = app.lock().await;
        let (end_session, end_agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("end session should be created");
        let (delete_session, delete_agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(
                CreateSessionRequest::new("workspace", "worktree").with_alias("delete-owned"),
            )
            .expect("delete session should be created");
        let external_sessions = app_locked.external_provider_session_index_store();
        external_sessions.upsert(session_command_external_provider_session_record(
            "codex",
            "ended-session-thread",
            30,
        ));
        external_sessions.mark_attached(
            "codex:ended-session-thread",
            end_session.id(),
            end_agent.id(),
        );
        external_sessions.upsert(session_command_external_provider_session_record(
            "codex",
            "deleted-session-thread",
            40,
        ));
        external_sessions.mark_attached(
            "codex:deleted-session-thread",
            delete_session.id(),
            delete_agent.id(),
        );
        let delete_cursor_key = crate::app::AttachedProviderTranscriptCursorKey::new(
            delete_session.id(),
            delete_agent.id(),
            "codex",
            "deleted-session-thread",
        );
        let end_cursor_key = crate::app::AttachedProviderTranscriptCursorKey::new(
            end_session.id(),
            end_agent.id(),
            "codex",
            "ended-session-thread",
        );
        app_locked.attached_provider_transcript_cursor_store().set(
            end_cursor_key.clone(),
            crate::provider::ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-before-end".to_string()),
                ..crate::provider::ExternalProviderObservedCursor::default()
            },
        );
        app_locked.attached_provider_transcript_cursor_store().set(
            delete_cursor_key.clone(),
            crate::provider::ExternalProviderObservedCursor {
                last_observed_turn_id: Some("turn-before-delete".to_string()),
                ..crate::provider::ExternalProviderObservedCursor::default()
            },
        );
        (
            end_session.id().to_string(),
            end_agent.id().to_string(),
            end_cursor_key,
            delete_session.id().to_string(),
            delete_agent.id().to_string(),
            delete_cursor_key,
            app_locked.terminal_stream_store(),
        )
    };
    let session_projection = SessionStateProjectionStore::default();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        session_projection.clone(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: end_session_id.clone(),
    });
    let end_command = KernelCommand::from_local_request("owned-end", None, None, &end_request);
    let _locked_app = app.lock().await;
    let end_response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(end_command, end_request),
    )
    .await
    .expect("owned end should not wait for the app lock")
    .expect("end should succeed");
    assert!(matches!(
        end_response,
        LocalDaemonResponse::SessionEnded { .. }
    ));
    assert!(
        session_projection.get(&end_session_id).is_some(),
        "ended session should remain projected"
    );
    let end_page = _locked_app.external_provider_session_index_store().list(
        &ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        },
    );
    let ended_record = end_page
        .sessions
        .iter()
        .find(|session| session.external_session_id == "codex:ended-session-thread")
        .expect("ending a session with an attached external provider agent should return its provider thread to the unattached list");
    assert!(ended_record.is_attachable_to_arroba());
    assert_eq!(
        ended_record.attached_agent_ids,
        Vec::<String>::new(),
        "ended session agent `{end_agent_id}` should not remain attached to the external provider session"
    );
    assert_eq!(
        _locked_app
            .attached_provider_transcript_cursor_store()
            .get(&end_cursor_key),
        crate::provider::ExternalProviderObservedCursor::default(),
        "ending a session should prune its attached provider transcript cursor"
    );

    let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
        session_ref: "delete-owned".to_string(),
        workspace_id: Some("workspace".to_string()),
    });
    let delete_command =
        KernelCommand::from_local_request("owned-delete", None, None, &delete_request);
    let delete_response = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(delete_command, delete_request),
    )
    .await
    .expect("owned delete should not wait for the app lock")
    .expect("delete should succeed");
    assert!(matches!(
        delete_response,
        LocalDaemonResponse::SessionDeleted { .. }
    ));
    assert!(
        session_projection.get(&delete_session_id).is_none(),
        "deleted session should be removed from projection"
    );
    let page = _locked_app.external_provider_session_index_store().list(
        &ListExternalProviderSessionsRequest {
            provider: Some("codex".to_string()),
            cursor: None,
            limit: None,
        },
    );
    let listed_external_session_ids = page
        .sessions
        .iter()
        .map(|session| session.external_session_id.as_str())
        .collect::<Vec<_>>();
    assert!(
        listed_external_session_ids.contains(&"codex:deleted-session-thread"),
        "deleting a session with an attached external provider agent should return its provider thread to the unattached list"
    );
    assert!(
        listed_external_session_ids.contains(&"codex:ended-session-thread"),
        "ending a session should leave its returned provider thread in the unattached list"
    );
    let deleted_record = page
        .sessions
        .iter()
        .find(|session| session.external_session_id == "codex:deleted-session-thread")
        .expect("deleted provider thread should be listed as unattached");
    assert!(deleted_record.is_attachable_to_arroba());
    assert_eq!(
        deleted_record.attached_agent_ids,
        Vec::<String>::new(),
        "deleted session agent `{delete_agent_id}` should not remain attached to the external provider session"
    );
    assert_eq!(
        _locked_app
            .attached_provider_transcript_cursor_store()
            .get(&delete_cursor_key),
        crate::provider::ExternalProviderObservedCursor::default(),
        "deleting a session should prune its attached provider transcript cursor"
    );
}

#[tokio::test]
async fn resize_terminal_validates_owned_session_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app_locked = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app_locked)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        (session.id().to_string(), app_locked.terminal_stream_store())
    };
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let request = LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
        session_id: session_id.clone(),
        provider_run_id: None,
        cols: 120,
        rows: 40,
    });
    let command =
        KernelCommand::from_local_request("owned-resize-validation", None, None, &request);
    let _locked_app = app.lock().await;
    let error = timeout(
        Duration::from_millis(100),
        runtime.dispatch_session_command(command, request),
    )
    .await
    .expect("owned resize validation should not wait for the app lock")
    .expect_err("resize without an active provider run should fail");
    assert!(matches!(
        error,
        DaemonError::NoActiveProviderRun { session_id: id } if id == session_id
    ));
}

#[tokio::test]
async fn config_update_rejects_warmed_missing_attachment_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let session_projection = SessionStateProjectionStore::default();
    session_projection.update_list(Vec::new());
    let request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
        session_id: "missing-session".to_string(),
        attachment_id: "missing-attachment".to_string(),
        values: Default::default(),
        requires_idle: false,
    });

    let _locked_app = app.lock().await;
    let result = timeout(Duration::from_millis(100), async {
        projected_config_update_absence_response(&session_projection, &request)
    })
    .await
    .expect("projected config validation should not wait for the app lock")
    .expect("warmed projection should handle missing attachment");
    let error = result.expect_err("missing attachment should fail");

    match error {
        DaemonError::AttachmentNotFound { attachment_id } => {
            assert_eq!(attachment_id, "missing-attachment");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn session_end_clears_terminal_stream_records() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "cli-terminal-cleanup",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let terminal_stream = app.terminal_stream_store();
        terminal_stream.record_input(session.id(), "provider-run-1", attachment.id(), b"input");
        terminal_stream.fan_out_output(
            session.id(),
            "provider-run-1",
            None,
            TerminalOutputKind::ProviderOutput,
            None,
            vec![attachment.id().to_string()],
            b"output",
        );
        terminal_stream.record_notice(
            session.id(),
            None,
            None,
            vec![attachment.id().to_string()],
            "notice",
        );
        terminal_stream.record_assistant_message_completion(
            session.id(),
            "provider-run-1",
            None,
            vec![attachment.id().to_string()],
            "message-1",
            1,
        );
        (session.id().to_string(), terminal_stream)
    };
    assert_eq!(terminal_stream.health_snapshot().pending_output_records, 1);
    assert_eq!(terminal_stream.health_snapshot().pending_notice_records, 1);
    assert_eq!(
        terminal_stream.health_snapshot().pending_completion_records,
        1
    );

    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        owned_runtime_state(&app).await,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream.clone(),
    );
    let request = LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: session_id.clone(),
    });
    let command = crate::runtime::command::KernelCommand::from_local_request(
        "cmd-end-session-cleanup",
        None,
        None,
        &request,
    );
    runtime
        .dispatch_session_command(command, request)
        .await
        .expect("session end should succeed");

    assert!(terminal_stream.input_records().is_empty());
    assert!(terminal_stream.output_records().is_empty());
    assert!(terminal_stream.notice_records().is_empty());
    assert_eq!(terminal_stream.health_snapshot().pending_output_records, 0);
    assert_eq!(terminal_stream.health_snapshot().pending_notice_records, 0);
    assert_eq!(
        terminal_stream.health_snapshot().pending_completion_records,
        0
    );
}

#[test]
fn handles_attach_through_session_actor_surface() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "cli-1",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attach should succeed");
    let response = LocalDaemonResponse::SessionAttached { attachment };

    assert!(matches!(
        response,
        LocalDaemonResponse::SessionAttached { .. }
    ));
}

fn session_command_external_provider_session_record(
    provider: &str,
    provider_session_id: &str,
    last_modified_at_ms: u64,
) -> ExternalProviderSessionRecord {
    ExternalProviderSessionRecord {
        external_session_id: format!("{provider}:{provider_session_id}"),
        provider: provider.to_string(),
        provider_session_id: provider_session_id.to_string(),
        title: Some(provider_session_id.to_string()),
        title_source: Some("test".to_string()),
        first_prompt_preview: None,
        created_at_ms: None,
        last_modified_at_ms,
        worktree_path: None,
        account_profile: None,
        capabilities: ExternalProviderSessionCapabilities {
            ..ExternalProviderSessionCapabilities::default()
        },
        attached_to_arroba: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    }
}
