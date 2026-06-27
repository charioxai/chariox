use super::*;
use crate::local::LaunchProviderRunsRequest;

fn run_provider_projection_large_stack_test<Fut>(name: &str, test: fn() -> Fut)
where
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("provider projection test runtime should build")
                .block_on(test());
        })
        .expect("provider projection test thread should spawn")
        .join()
        .expect("provider projection test thread should not panic");
}

#[tokio::test]
async fn provider_launch_rejects_cross_session_agent_before_acceptance() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("first session should be created");
    let (second_session, _second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-2", "worktree-2"))
        .expect("second session should be created");
    let first_session_id = first_session.id().to_string();
    let first_agent_id = first_agent.id().to_string();
    let second_session_id = second_session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
        session_id: second_session_id.clone(),
        agent_id: Some(first_agent_id.clone()),
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
        "cmd-provider-launch-cross-session-agent",
        None,
        None,
        &launch_request,
    );

    let error = router
        .dispatch(launch_command, launch_request)
        .await
        .expect_err("provider launch should reject an agent outside the requested session");

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

#[test]
fn provider_batch_launch_accepts_multiple_agents_with_one_kernel_request() {
    run_provider_projection_large_stack_test(
        "provider-batch-launch-accepts-multiple-agents",
        provider_batch_launch_accepts_multiple_agents_with_one_kernel_request_inner,
    );
}

async fn provider_batch_launch_accepts_multiple_agents_with_one_kernel_request_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-batch",
            "worktree-batch",
        ))
        .expect("session should be created");
    let second_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("batch-launch-agent")
                .with_worktree("worktree-batch"),
        )
        .expect("second agent should be created");
    let session_id = session.id().to_string();
    let first_agent_id = first_agent.id().to_string();
    let second_agent_id = second_agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRuns(LaunchProviderRunsRequest {
        max_concurrency: Some(2),
        launches: vec![
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(first_agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
            LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(second_agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            },
        ],
    });
    let launch_command =
        KernelCommand::from_local_request("cmd-provider-batch-launch", None, None, &launch_request);

    let response = router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider batch launch should be accepted");
    let LocalDaemonResponse::ProviderRunsLaunchAccepted {
        provider_runs,
        failures,
    } = response
    else {
        panic!("unexpected launch response");
    };
    assert!(failures.is_empty());
    assert_eq!(provider_runs.len(), 2);
    assert_eq!(
        provider_runs[0].agent_id.as_deref(),
        Some(first_agent_id.as_str())
    );
    assert_eq!(
        provider_runs[1].agent_id.as_deref(),
        Some(second_agent_id.as_str())
    );
    assert_eq!(provider_runs[0].index, 0);
    assert_eq!(provider_runs[1].index, 1);
}

#[test]
fn get_provider_run_uses_warmed_projection_without_app_lock() {
    run_provider_projection_large_stack_test(
        "get-provider-run-uses-warmed-projection-without-app-lock",
        get_provider_run_uses_warmed_projection_without_app_lock_inner,
    );
}

async fn get_provider_run_uses_warmed_projection_without_app_lock_inner() {
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
    let provider_response = tokio::time::timeout(
        Duration::from_millis(100),
        router.dispatch_pre_lane(
            &provider_command,
            &provider_request,
            crate::session::DEFAULT_LOCAL_USER_ID,
        ),
    )
    .await
    .expect("warmed GetProviderRun should not wait for the app lock")
    .expect("provider run projection should not fail")
    .expect("warmed GetProviderRun should be served from projection");
    drop(app_guard);
    match provider_response {
        LocalDaemonResponse::ProviderRun { provider_run } => {
            assert_eq!(provider_run.id(), provider_run_id);
        }
        _ => panic!("unexpected provider response"),
    }
}

#[tokio::test]
async fn get_provider_run_projection_does_not_serve_opencode_arroba_runs() {
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
    let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
        provider_run_id: provider_run.id().to_string(),
    });
    let LocalDaemonRequest::GetProviderRun(provider_request) = provider_request else {
        unreachable!("request shape should stay GetProviderRun")
    };

    let projected = crate::runtime::provider_run_control::projected_provider_run_response(
        &router.provider_run_projection,
        &provider_request,
        crate::session::DEFAULT_LOCAL_USER_ID,
    )
    .expect("projection visibility check should not fail");

    assert!(
        projected.is_none(),
        "warmed opencode GetProviderRun must stay eligible for the refresh/sync handler"
    );
}

#[test]
fn provider_run_projection_tracks_async_launch_completion() {
    run_provider_projection_large_stack_test(
        "provider-run-projection-tracks-async-launch-completion",
        provider_run_projection_tracks_async_launch_completion_inner,
    );
}

async fn provider_run_projection_tracks_async_launch_completion_inner() {
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
    let provider_response = tokio::time::timeout(
        Duration::from_millis(100),
        router.dispatch_pre_lane(
            &provider_command,
            &provider_request,
            crate::session::DEFAULT_LOCAL_USER_ID,
        ),
    )
    .await
    .expect("running GetProviderRun should not wait for the app lock")
    .expect("running provider run projection should not fail")
    .expect("running GetProviderRun should be served from projection");
    drop(app_guard);
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

#[test]
fn list_provider_processes_uses_warmed_projection_without_app_lock() {
    run_provider_projection_large_stack_test(
        "list-provider-processes-uses-warmed-projection-without-app-lock",
        list_provider_processes_uses_warmed_projection_without_app_lock_inner,
    );
}

async fn list_provider_processes_uses_warmed_projection_without_app_lock_inner() {
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

    let canonical_processes = {
        let app = app.lock().await;
        app.list_provider_processes(None)
            .expect("provider process list should warm projection")
    };
    router
        .provider_process_projection
        .update_list(canonical_processes);

    let app_guard = app.lock().await;
    let projected_list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let projected_list_command = KernelCommand::from_local_request(
        "cmd-process-list-projection",
        None,
        None,
        &projected_list_request,
    );
    let list_response = tokio::time::timeout(
        Duration::from_millis(100),
        router.dispatch_pre_lane(
            &projected_list_command,
            &projected_list_request,
            crate::session::DEFAULT_LOCAL_USER_ID,
        ),
    )
    .await
    .expect("warmed ListProviderProcesses should not wait for the app lock")
    .expect("provider process projection should not fail")
    .expect("warmed ListProviderProcesses should be served from projection");
    drop(app_guard);
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
async fn provider_process_teardown_only_terminates_caller_owned_processes() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, local_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "provider-process-peer".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Full,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session");
    let peer_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer")
                .with_owner_user_id("user-2"),
        )
        .expect("peer agent should be created");
    let local_run = launch_test_provider(
        &mut app,
        &session_id,
        local_agent.id(),
        "dev-stub",
        "claude-code",
        "sonnet",
    );
    let peer_run = launch_test_provider(
        &mut app,
        &session_id,
        peer_agent.id(),
        "dev-stub",
        "codex",
        "gpt-5.4",
    );
    let process_before_teardown = app
        .list_provider_processes(None)
        .expect("processes should list");
    assert_eq!(process_before_teardown.len(), 1);
    assert_eq!(
        process_before_teardown[0].owner_provider_run_ids,
        vec![local_run.id().to_string(), peer_run.id().to_string()]
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request =
        LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest { provider: None });
    let list_command = remote_command_for_request(&list_request, Some("user-2"));
    let list_response = router
        .dispatch(list_command, list_request)
        .await
        .expect("list should complete");
    match list_response {
        LocalDaemonResponse::ProviderProcessesListed { processes } => {
            assert_eq!(processes.len(), 1);
            assert!(!processes[0].teardown_safe);
            assert_eq!(
                processes[0].teardown_blockers,
                vec!["shared with another user"]
            );
        }
        _ => panic!("unexpected list response"),
    }

    let teardown_request =
        LocalDaemonRequest::TeardownProviderProcesses(TeardownProviderProcessesRequest {
            provider: None,
            force: false,
        });
    let teardown_command = remote_command_for_request(&teardown_request, Some("user-2"));
    let teardown_response = router
        .dispatch(teardown_command, teardown_request)
        .await
        .expect("teardown should complete");

    match teardown_response {
        LocalDaemonResponse::ProviderProcessesTornDown { processes } => {
            assert!(
                processes.is_empty(),
                "caller must not tear down a shared process with another user's active run"
            );
        }
        _ => panic!("unexpected teardown response"),
    }
    let remaining = app
        .lock()
        .await
        .list_provider_processes(None)
        .expect("remaining processes should list");
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].owner_provider_run_ids,
        vec![local_run.id().to_string(), peer_run.id().to_string()]
    );
}

#[test]
fn teardown_provider_processes_refreshes_session_projection_without_app_lock() {
    run_provider_projection_large_stack_test(
        "teardown-provider-processes-refreshes-session-projection-without-app-lock",
        teardown_provider_processes_refreshes_session_projection_without_app_lock_inner,
    );
}

async fn teardown_provider_processes_refreshes_session_projection_without_app_lock_inner() {
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
