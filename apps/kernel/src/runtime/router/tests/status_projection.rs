use super::*;

#[tokio::test]
async fn daemon_health_projection_reports_session_and_agent_mailboxes() {
    Box::pin(async {
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

        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let focus_request = focus_request(&session_id, &agent_id);
        let focus_command =
            KernelCommand::from_local_request("cmd-focus", None, None, &focus_request);
        router
            .dispatch(focus_command, focus_request)
            .await
            .expect("focus should create a session lane");

        let prompt_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "hello from health projection test".to_string(),
            attachments: Vec::new(),
        });
        let prompt_command =
            KernelCommand::from_local_request("cmd-prompt", None, None, &prompt_request);
        router
            .dispatch(prompt_command, prompt_request)
            .await
            .expect("prompt should create an agent lane");

        let workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("health-workflow".to_string()),
        });
        let workflow_command =
            KernelCommand::from_local_request("cmd-workflow", None, None, &workflow_request);
        router
            .dispatch(workflow_command, workflow_request)
            .await
            .expect("workflow command should create a workflow lane");

        let shell_request = LocalDaemonRequest::RunShellCommand(RunShellCapabilityRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            command: "/bin/true".to_string(),
            args: Vec::new(),
            working_directory: None,
            timeout_ms: Some(1_000),
        });
        let shell_command =
            KernelCommand::from_local_request("cmd-capability", None, None, &shell_request);
        router
            .dispatch(shell_command, shell_request)
            .await
            .expect_err(
                "capability command should report executor failure for missing test worktree",
            );

        let projection = router.daemon_health_projection(0).await;
        assert!(projection
            .session_command_lanes
            .iter()
            .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128));
        assert!(projection
            .agent_command_lanes
            .iter()
            .any(|lane| lane.lane_id == agent_id && lane.queue_limit == 128));
        assert!(projection
            .workflow_command_lanes
            .iter()
            .any(|lane| lane.lane_id == session_id && lane.queue_limit == 128));
        assert_eq!(projection.session_projection.projected_sessions, 1);
        assert_eq!(projection.session_projection.active_prompts, 1);
        assert_eq!(projection.session_projection.queued_prompts, 0);
        assert_eq!(projection.agent_runtime_projection.projected_agents, 1);
        assert_eq!(projection.agent_runtime_projection.active_prompts, 1);
        assert_eq!(projection.agent_runtime_projection.queued_prompts, 0);
        assert_eq!(projection.provider_runs.projected_runs, 1);
        assert_eq!(projection.provider_runs.active_runs, 1);
        assert_eq!(projection.provider_runs.arroba_active_runs, 1);
        assert!(projection
            .provider_runs
            .duplicate_arroba_agent_bindings
            .is_empty());
        assert!(projection.provider_runs.orphaned_active_runs.is_empty());
        assert!(projection
            .provider_runs
            .session_active_run_mismatches
            .is_empty());
        assert_eq!(projection.capability_executor.max_concurrent_jobs, 64);
        assert_eq!(projection.capability_executor.available_permits, 64);
        assert_eq!(projection.capability_executor.submitted_jobs, 1);
        assert_eq!(projection.capability_executor.completed_jobs, 0);
        assert_eq!(projection.capability_executor.failed_jobs, 1);
        assert_eq!(projection.capability_executor.rejected_jobs, 0);
        assert!(!projection.provider_catalog.cached);
    })
    .await;
}

#[tokio::test]
async fn daemon_health_reports_duplicate_active_arroba_provider_runs_per_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let first_run = launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let duplicate_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(&agent_id),
        )
        .expect("duplicate provider run should start")
        .into_run();
    app.update_provider_run_projection(duplicate_run.clone());

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
    let health_command = KernelCommand::from_local_request(
        "cmd-health-duplicate-provider",
        None,
        None,
        &health_request,
    );
    let health_response = router
        .dispatch(health_command, health_request)
        .await
        .expect("health projection should be returned");
    let projection = match health_response {
        LocalDaemonResponse::DaemonHealth { projection } => projection,
        _ => panic!("unexpected health response"),
    };

    assert_eq!(projection.provider_runs.projected_runs, 2);
    assert_eq!(projection.provider_runs.active_runs, 2);
    assert_eq!(projection.provider_runs.arroba_active_runs, 2);
    assert_eq!(
        projection.provider_runs.duplicate_arroba_agent_bindings,
        vec![
            crate::runtime::projection::ProviderRunAgentBindingConflict {
                session_id,
                agent_id,
                provider_run_ids: {
                    let mut ids = vec![first_run.id().to_string(), duplicate_run.id().to_string()];
                    ids.sort();
                    ids
                },
            }
        ]
    );
    assert!(projection
        .provider_runs
        .duplicate_native_tui_agent_bindings
        .is_empty());
}

#[tokio::test]
async fn provider_run_projection_lookup_prefers_deterministic_latest_highest_state_run() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let first_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(&agent_id),
        )
        .expect("first provider run should start")
        .into_run();
    app.update_provider_run_projection(first_run.clone());
    let second_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "opus")
                .with_agent_id(&agent_id),
        )
        .expect("second provider run should start")
        .into_run();
    app.update_provider_run_projection(second_run.clone());

    assert_eq!(
        app.provider_run_projection_store()
            .get_for_agent(&session_id, &agent_id)
            .map(|run| run.id().to_string()),
        Some(second_run.id().to_string())
    );
    assert!(second_run.active_selection_cmp(&first_run).is_gt());
}

#[tokio::test]
async fn daemon_health_reports_multi_interface_provider_runs_per_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let arroba_run = launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let native_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(&agent_id)
                .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("native provider run should start")
        .into_run();
    app.update_provider_run_projection(native_run.clone());

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
    let health_command = KernelCommand::from_local_request(
        "cmd-health-multi-interface-provider",
        None,
        None,
        &health_request,
    );
    let health_response = router
        .dispatch(health_command, health_request)
        .await
        .expect("health projection should be returned");
    let projection = match health_response {
        LocalDaemonResponse::DaemonHealth { projection } => projection,
        _ => panic!("unexpected health response"),
    };

    assert_eq!(projection.provider_runs.projected_runs, 2);
    assert_eq!(projection.provider_runs.active_runs, 2);
    assert_eq!(projection.provider_runs.arroba_active_runs, 1);
    assert_eq!(projection.provider_runs.native_tui_active_runs, 1);
    assert!(projection
        .provider_runs
        .duplicate_arroba_agent_bindings
        .is_empty());
    assert!(projection
        .provider_runs
        .duplicate_native_tui_agent_bindings
        .is_empty());
    assert_eq!(
        projection.provider_runs.multi_interface_agent_bindings,
        vec![
            crate::runtime::projection::ProviderRunAgentBindingConflict {
                session_id,
                agent_id,
                provider_run_ids: {
                    let mut ids = vec![
                        format!("{}:arroba", arroba_run.id()),
                        format!("{}:native_tui", native_run.id()),
                    ];
                    ids.sort();
                    ids
                },
            }
        ]
    );
}

#[tokio::test]
async fn daemon_health_reports_duplicate_active_native_tui_provider_runs_per_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let first_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(&agent_id)
                .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("first native provider run should start")
        .into_run();
    app.update_provider_run_projection(first_run.clone());
    let duplicate_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(&session_id, "dev-stub", "claude-code", "default", "opus")
                .with_agent_id(&agent_id)
                .with_client_interface(ProviderClientInterface::NativeTui),
        )
        .expect("duplicate native provider run should start")
        .into_run();
    app.update_provider_run_projection(duplicate_run.clone());

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
    let health_command = KernelCommand::from_local_request(
        "cmd-health-duplicate-native-provider",
        None,
        None,
        &health_request,
    );
    let health_response = router
        .dispatch(health_command, health_request)
        .await
        .expect("health projection should be returned");
    let projection = match health_response {
        LocalDaemonResponse::DaemonHealth { projection } => projection,
        _ => panic!("unexpected health response"),
    };

    assert_eq!(projection.provider_runs.projected_runs, 2);
    assert_eq!(projection.provider_runs.active_runs, 2);
    assert_eq!(projection.provider_runs.arroba_active_runs, 0);
    assert_eq!(projection.provider_runs.native_tui_active_runs, 2);
    assert!(projection
        .provider_runs
        .duplicate_arroba_agent_bindings
        .is_empty());
    assert_eq!(
        projection.provider_runs.duplicate_native_tui_agent_bindings,
        vec![
            crate::runtime::projection::ProviderRunAgentBindingConflict {
                session_id,
                agent_id,
                provider_run_ids: {
                    let mut ids = vec![first_run.id().to_string(), duplicate_run.id().to_string()];
                    ids.sort();
                    ids
                },
            }
        ]
    );
    assert!(projection
        .provider_runs
        .multi_interface_agent_bindings
        .is_empty());
}

#[tokio::test]
async fn daemon_health_reports_active_provider_run_for_nonfocused_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, focused_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let focused_agent_id = focused_agent.id().to_string();
    let background_agent = spawn_test_agent(&mut app, &session_id, "background", "dev-stub");
    let background_agent_id = background_agent.id().to_string();
    let background_run = launch_test_provider(
        &mut app,
        &session_id,
        &background_agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let projected_session = {
        let session_store = app.session_state_store();
        let mut sessions = session_store.write();
        sessions
            .set_focused_agent(&session_id, Some(focused_agent_id.clone()))
            .expect("focus should be forced for drift test");
        sessions
            .set_active_provider_run(&session_id, Some(background_run.id().to_string()))
            .expect("active run should be forced for drift test")
    };
    app.update_session_projection(projected_session);

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
    let health_command = KernelCommand::from_local_request(
        "cmd-health-active-run-agent-drift",
        None,
        None,
        &health_request,
    );
    let health_response = router
        .dispatch(health_command, health_request)
        .await
        .expect("health projection should be returned");
    let projection = match health_response {
        LocalDaemonResponse::DaemonHealth { projection } => projection,
        _ => panic!("unexpected health response"),
    };

    assert_eq!(
        projection.provider_runs.session_active_run_mismatches,
        vec![crate::runtime::projection::ProviderRunSessionPointerIssue {
            session_id,
            active_provider_run_id: Some(background_run.id().to_string()),
            details: format!(
                "active provider run points at agent {background_agent_id}, focused agent is {focused_agent_id}"
            ),
        }]
    );
}

#[tokio::test]
async fn daemon_health_reads_terminal_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let app_guard = app.lock().await;
    let health_request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
    let health_command =
        KernelCommand::from_local_request("cmd-health-no-lock", None, None, &health_request);
    let health_router = router.clone();
    let health_task =
        tokio::spawn(async move { health_router.dispatch(health_command, health_request).await });

    let response = timeout(Duration::from_millis(100), health_task)
        .await
        .expect("daemon health should not wait for the app lock")
        .expect("health task should join")
        .expect("health should resolve");
    drop(app_guard);

    match response {
        LocalDaemonResponse::DaemonHealth { projection } => {
            assert_eq!(projection.terminal_stream.pending_output_records, 0);
        }
        _ => panic!("unexpected health response"),
    }
}

#[tokio::test]
async fn relay_status_uses_config_projection_without_app_lock() {
    let mut config = DaemonConfig::for_tests();
    config.relay_url = Some("ws://127.0.0.1:9".to_string());
    config.relay_token = Some("secret".to_string());
    config.host_machine_id = "machine-projected".to_string();
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(config).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let app_guard = app.lock().await;
    let relay_request = LocalDaemonRequest::RelayStatus(RelayStatusRequest);
    let relay_command = KernelCommand::from_local_request(
        "cmd-relay-status-projection",
        None,
        None,
        &relay_request,
    );
    let relay_router = router.clone();
    let relay_task =
        tokio::spawn(async move { relay_router.dispatch(relay_command, relay_request).await });

    let response = timeout(Duration::from_millis(100), relay_task)
        .await
        .expect("relay status should not wait for the app lock")
        .expect("relay task should join")
        .expect("relay status should resolve");
    drop(app_guard);

    match response {
        LocalDaemonResponse::RelayStatus { status } => {
            assert!(status.configured);
            assert_eq!(status.relay_url.as_deref(), Some("ws://127.0.0.1:9"));
            assert!(status.relay_token_configured);
            assert_eq!(status.machine_id, "machine-projected");
        }
        _ => panic!("unexpected relay response"),
    }
}

#[tokio::test]
async fn provider_command_catalogs_do_not_wait_for_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let app_guard = app.lock().await;
    let catalog_request =
        LocalDaemonRequest::GetProviderCommandCatalogs(GetProviderCommandCatalogsRequest);
    let catalog_command = KernelCommand::from_local_request(
        "cmd-provider-command-catalog-projection",
        None,
        None,
        &catalog_request,
    );
    let catalog_router = router.clone();
    let catalog_task = tokio::spawn(async move {
        catalog_router
            .dispatch(catalog_command, catalog_request)
            .await
    });

    let response = timeout(Duration::from_millis(100), catalog_task)
        .await
        .expect("provider command catalogs should not wait for the app lock")
        .expect("catalog task should join")
        .expect("provider command catalogs should resolve");
    drop(app_guard);

    match response {
        LocalDaemonResponse::ProviderCommandCatalogs { catalogs } => {
            assert!(!catalogs.is_empty());
        }
        _ => panic!("unexpected provider command catalog response"),
    }
}

#[tokio::test]
async fn provider_auth_status_does_not_use_generic_app_fallback() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let app_guard = app.lock().await;
    let auth_request = LocalDaemonRequest::GetProviderAuthStatus(GetProviderAuthStatusRequest {
        provider: "unsupported-provider".to_string(),
    });
    let auth_command = KernelCommand::from_local_request(
        "cmd-provider-auth-no-fallback",
        None,
        None,
        &auth_request,
    );
    let auth_router = router.clone();
    let auth_task =
        tokio::spawn(async move { auth_router.dispatch(auth_command, auth_request).await });

    let error = timeout(Duration::from_millis(100), auth_task)
        .await
        .expect("provider auth status should not wait for the app lock")
        .expect("auth task should join")
        .expect_err("unsupported provider should be rejected");
    drop(app_guard);

    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "get_provider_auth_status");
            assert!(message.contains("unsupported-provider"));
        }
        error => panic!("unexpected error: {error}"),
    }
}
