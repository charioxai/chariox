use super::*;

#[tokio::test]
async fn get_provider_run_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id.clone()),
        adapter_key: "dev-stub".to_string(),
        provider: "claude-code".to_string(),
        account_profile: "default".to_string(),
        model: "sonnet".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: false,
    });
    let launch_command =
        KernelCommand::from_local_request("cmd-provider-launch", None, None, &launch_request);
    let provider_run_id = match router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider launch should be accepted")
    {
        LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => {
            provider_run.id().to_string()
        }
        _ => panic!("unexpected launch response"),
    };

    let app_guard = app.lock().await;
    let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
        provider_run_id: provider_run_id.clone(),
    });
    let provider_command =
        KernelCommand::from_local_request("cmd-provider-projection", None, None, &provider_request);
    let provider_router = router.clone();
    let provider_task = tokio::spawn(async move {
        provider_router
            .dispatch(provider_command, provider_request)
            .await
    });

    tokio::task::yield_now().await;
    assert!(
            provider_task.is_finished(),
            "warmed GetProviderRun should be served from the provider-run projection without app lock access"
        );
    drop(app_guard);

    let provider_response = provider_task
        .await
        .expect("provider task should join")
        .expect("provider run should resolve");
    match provider_response {
        LocalDaemonResponse::ProviderRun { provider_run } => {
            assert_eq!(provider_run.id(), provider_run_id);
        }
        _ => panic!("unexpected provider response"),
    }
}

#[tokio::test]
async fn get_provider_run_does_not_bypass_opencode_selection_sync_path() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let provider_run = RuntimeProviderRun::from_control_capability_inference(
        "projected-opencode-run",
        session.id().to_string(),
        Some(agent.id().to_string()),
        "opencode".to_string(),
    );
    app.update_provider_run_projection(provider_run.clone());

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;
    let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
        provider_run_id: provider_run.id().to_string(),
    });
    let provider_command = KernelCommand::from_local_request(
        "cmd-opencode-provider-run-refresh",
        None,
        None,
        &provider_request,
    );
    let provider_router = router.clone();
    let provider_task = tokio::spawn(async move {
        provider_router
            .dispatch(provider_command, provider_request)
            .await
    });

    tokio::task::yield_now().await;
    assert!(
        !provider_task.is_finished(),
        "warmed opencode GetProviderRun must not bypass the refresh/sync handler"
    );
    drop(app_guard);
    let _ = provider_task
        .await
        .expect("provider task should join after app lock is released");
}

#[tokio::test]
async fn provider_run_projection_tracks_async_launch_completion() {
    let mut config = DaemonConfig::for_tests();
    config.provider_runtime_init_delay_ms = 25;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id.clone()),
        adapter_key: "dev-stub".to_string(),
        provider: "claude-code".to_string(),
        account_profile: "default".to_string(),
        model: "sonnet".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: false,
    });
    let launch_command =
        KernelCommand::from_local_request("cmd-provider-launch-async", None, None, &launch_request);
    let provider_run_id = match router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider launch should be accepted")
    {
        LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => {
            assert_eq!(
                provider_run.state(),
                crate::provider::ProviderRunState::Starting
            );
            provider_run.id().to_string()
        }
        _ => panic!("unexpected launch response"),
    };

    let mut running_seen = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
            provider_run_id: provider_run_id.clone(),
        });
        let provider_command = KernelCommand::from_local_request(
            "cmd-provider-running-poll",
            None,
            None,
            &provider_request,
        );
        let response = router
            .dispatch(provider_command, provider_request)
            .await
            .expect("provider run should resolve");
        if let LocalDaemonResponse::ProviderRun { provider_run } = response {
            if provider_run.state() == crate::provider::ProviderRunState::Running {
                running_seen = true;
                break;
            }
        }
    }
    assert!(
        running_seen,
        "provider projection should observe async launch completion"
    );

    let app_guard = app.lock().await;
    let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
        provider_run_id: provider_run_id.clone(),
    });
    let provider_command = KernelCommand::from_local_request(
        "cmd-provider-running-projection",
        None,
        None,
        &provider_request,
    );
    let provider_router = router.clone();
    let provider_task = tokio::spawn(async move {
        provider_router
            .dispatch(provider_command, provider_request)
            .await
    });
    tokio::task::yield_now().await;
    assert!(provider_task.is_finished());
    drop(app_guard);

    let provider_response = provider_task
        .await
        .expect("provider task should join")
        .expect("provider run should resolve");
    match provider_response {
        LocalDaemonResponse::ProviderRun { provider_run } => {
            assert_eq!(
                provider_run.state(),
                crate::provider::ProviderRunState::Running
            );
        }
        _ => panic!("unexpected provider response"),
    }

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-provider-running-session-projection",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
    let state_response = timeout(Duration::from_millis(100), state_task)
        .await
        .expect("async launch completion should publish session projection without app lock")
        .expect("state task should join")
        .expect("state should resolve");
    drop(app_guard);

    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(
                session.active_provider_run_id(),
                Some(provider_run_id.as_str())
            );
        }
        _ => panic!("unexpected session state response"),
    }
}

#[tokio::test]
async fn settled_provider_launch_pending_state_uses_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (mut session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let mut provider_run = RuntimeProviderRun::from_control_capability_inference(
        "projected-run",
        session_id.clone(),
        Some(agent_id),
        "dev-stub".to_string(),
    );
    provider_run.mark_running();
    session.set_active_provider_run(Some(provider_run.id().to_string()));
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    router.session_projection.update(session);
    router.provider_run_projection.update(provider_run);
    router
        .provider_launch_pending
        .insert_for_tests(session_id.clone())
        .await;

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-settled-launch-state-projection",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    let response = timeout(Duration::from_millis(100), state_task)
        .await
        .expect("settled provider launch state should not wait for the app lock")
        .expect("state task should join")
        .expect("state should resolve");
    drop(app_guard);

    match response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.id(), session_id);
            assert_eq!(session.active_provider_run_id(), Some("projected-run"));
        }
        _ => panic!("unexpected state response"),
    }
    assert!(
        !router
            .provider_launch_pending
            .contains_for_tests(&session_id)
            .await,
        "projection-settled launch should clear pending launch guard"
    );
}

#[tokio::test]
async fn list_provider_processes_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id.clone()),
        adapter_key: "dev-stub".to_string(),
        provider: "claude-code".to_string(),
        account_profile: "default".to_string(),
        model: "sonnet".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: false,
    });
    let launch_command = KernelCommand::from_local_request(
        "cmd-process-provider-launch",
        None,
        None,
        &launch_request,
    );
    let provider_run_id = match router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider launch should be accepted")
    {
        LocalDaemonResponse::ProviderRunLaunchAccepted { provider_run } => {
            provider_run.id().to_string()
        }
        _ => panic!("unexpected launch response"),
    };

    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command =
        KernelCommand::from_local_request("cmd-process-list-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial provider process list should warm projection");

    let app_guard = app.lock().await;
    let projected_list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let projected_list_command = KernelCommand::from_local_request(
        "cmd-process-list-projection",
        None,
        None,
        &projected_list_request,
    );
    let list_router = router.clone();
    let list_task = tokio::spawn(async move {
        list_router
            .dispatch(projected_list_command, projected_list_request)
            .await
    });

    tokio::task::yield_now().await;
    assert!(
        list_task.is_finished(),
        "warmed ListProviderProcesses should be served from projection without app lock access"
    );
    drop(app_guard);

    let list_response = list_task
        .await
        .expect("process list task should join")
        .expect("process list should resolve");
    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert_eq!(processes.len(), 1);
            assert_eq!(processes[0].owner_provider_run_ids, vec![provider_run_id]);
        }
        _ => panic!("unexpected provider process list response"),
    }
}

#[tokio::test]
async fn provider_process_projection_stores_canonical_unfiltered_snapshot() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    for (idx, provider, model) in [(1, "claude-code", "sonnet"), (2, "codex", "gpt-5.4")] {
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new(
                format!("workspace-{idx}"),
                format!("worktree-{idx}"),
            ))
            .expect("session should be created");
        launch_test_provider(
            &mut app,
            session.id(),
            agent.id(),
            "dev-stub",
            provider,
            model,
        );
    }

    let filtered = app
        .list_provider_processes(Some("claude-code"))
        .expect("filtered process list should warm projection");
    assert_eq!(filtered.len(), 1);

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;
    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command = KernelCommand::from_local_request(
        "cmd-process-canonical-projection",
        None,
        None,
        &list_request,
    );
    let list_router = router.clone();
    let list_task =
        tokio::spawn(async move { list_router.dispatch(list_command, list_request).await });

    tokio::task::yield_now().await;
    assert!(list_task.is_finished());
    drop(app_guard);

    let list_response = list_task
        .await
        .expect("list task should join")
        .expect("list should resolve");
    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert_eq!(processes.len(), 2);
        }
        _ => panic!("unexpected provider process list response"),
    }
}

#[tokio::test]
async fn provider_process_projection_updates_after_teardown() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    launch_test_provider(
        &mut app,
        &session_id,
        &agent_id,
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    app.list_provider_processes(None)
        .expect("process list should warm projection");
    app.teardown_provider_processes(None, false)
        .expect("teardown should update projection");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;
    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command = KernelCommand::from_local_request(
        "cmd-process-post-teardown-projection",
        None,
        None,
        &list_request,
    );
    let list_router = router.clone();
    let list_task =
        tokio::spawn(async move { list_router.dispatch(list_command, list_request).await });

    tokio::task::yield_now().await;
    assert!(list_task.is_finished());
    drop(app_guard);

    let list_response = list_task
        .await
        .expect("list task should join")
        .expect("list should resolve");
    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert!(processes.is_empty());
        }
        _ => panic!("unexpected provider process list response"),
    }
}

#[tokio::test]
async fn teardown_provider_processes_refreshes_session_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: session_id.clone(),
        agent_id: Some(agent_id),
        adapter_key: "dev-stub".to_string(),
        provider: "claude-code".to_string(),
        account_profile: "default".to_string(),
        model: "sonnet".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: false,
    });
    let launch_command = KernelCommand::from_local_request(
        "cmd-teardown-refresh-launch",
        None,
        None,
        &launch_request,
    );
    router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider launch should be accepted");

    let teardown_request =
        LocalDaemonRequest::TeardownProviderProcesses(TeardownProviderProcessesRequest {
            provider: None,
            force: false,
        });
    let teardown_command =
        KernelCommand::from_local_request("cmd-teardown-refresh", None, None, &teardown_request);
    let teardown_response = router
        .dispatch(teardown_command, teardown_request)
        .await
        .expect("safe process teardown should succeed");
    match teardown_response {
        LocalDaemonResponse::ProviderProcessesTornDown { processes } => {
            assert_eq!(processes.len(), 1);
        }
        _ => panic!("unexpected teardown response"),
    }

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-teardown-refresh-state", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    let state_response = timeout(Duration::from_millis(100), state_task)
        .await
        .expect("post-teardown session state should not wait for the app lock")
        .expect("state task should join")
        .expect("state should resolve");
    drop(app_guard);

    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.id(), session_id);
            assert_eq!(session.active_provider_run_id(), None);
        }
        _ => panic!("unexpected session state response"),
    }
}

#[tokio::test]
async fn get_provider_catalog_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    app.cache_provider_catalog(OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        }],
        default: Default::default(),
        connected: vec!["codex".to_string()],
    });
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let app_guard = app.lock().await;
    let catalog_request = LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest);
    let catalog_command = KernelCommand::from_local_request(
        "cmd-provider-catalog-projection",
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

    tokio::task::yield_now().await;
    assert!(
        catalog_task.is_finished(),
        "warmed GetProviderCatalog should be served from projection without app lock access"
    );
    drop(app_guard);

    let catalog_response = catalog_task
        .await
        .expect("catalog task should join")
        .expect("catalog should resolve");
    match catalog_response {
        LocalDaemonResponse::ProviderCatalog { catalog } => {
            assert_eq!(catalog.connected, vec!["codex"]);
        }
        _ => panic!("unexpected provider catalog response"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_configure_invalidates_provider_catalog_projection() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    app.cache_provider_catalog(OpenCodeProviderCatalog {
        all: vec![OpenCodeProviderInfo {
            id: "codex".to_string(),
            name: "Codex".to_string(),
            remote_machine_aliases: Vec::new(),
            models: Default::default(),
        }],
        default: Default::default(),
        connected: vec!["codex".to_string()],
    });
    app.configure_relay(None, None)
        .expect("relay configure should invalidate provider catalog projection");
    app.invalidate_provider_catalog_projection();

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let app_guard = app.lock().await;
    let catalog_request = LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest);
    let catalog_command = KernelCommand::from_local_request(
        "cmd-provider-catalog-invalidated",
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

    tokio::task::yield_now().await;
    assert!(
        !catalog_task.is_finished(),
        "relay configuration should invalidate warmed provider catalog projection"
    );
    drop(app_guard);
    let _ = catalog_task
        .await
        .expect("catalog task should join after app lock is released");
}
