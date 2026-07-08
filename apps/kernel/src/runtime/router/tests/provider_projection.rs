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

#[test]
fn provider_launch_rejects_cross_session_agent_before_acceptance() {
    run_provider_projection_large_stack_test(
        "provider-launch-rejects-cross-session-agent-before-acceptance",
        provider_launch_rejects_cross_session_agent_before_acceptance_inner,
    );
}

async fn provider_launch_rejects_cross_session_agent_before_acceptance_inner() {
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
fn provider_batch_launch_rejects_duplicate_targets_without_partial_launch() {
    run_provider_projection_large_stack_test(
        "provider-batch-launch-rejects-duplicate-targets",
        provider_batch_launch_rejects_duplicate_targets_without_partial_launch_inner,
    );
}

async fn provider_batch_launch_rejects_duplicate_targets_without_partial_launch_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-batch-duplicate",
            "worktree-batch-duplicate",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch = LaunchProviderRunRequest {
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
    };
    let launch_request = LocalDaemonRequest::LaunchProviderRuns(LaunchProviderRunsRequest {
        max_concurrency: Some(2),
        launches: vec![launch.clone(), launch],
    });
    let launch_command = KernelCommand::from_local_request(
        "cmd-provider-batch-launch-duplicate",
        None,
        None,
        &launch_request,
    );

    let response = router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider batch duplicate target should return indexed failures");
    let LocalDaemonResponse::ProviderRunsLaunchAccepted {
        provider_runs,
        failures,
    } = response
    else {
        panic!("unexpected launch response");
    };
    assert!(provider_runs.is_empty());
    assert_eq!(failures.len(), 2);
    assert!(failures
        .iter()
        .all(|failure| failure.message.contains("duplicate target agents")));
    let app = app.lock().await;
    assert!(app
        .providers()
        .get_latest_run_for_agent(&session_id, &agent_id)
        .is_none());
}

#[test]
fn provider_batch_launch_rejects_focused_and_explicit_duplicate_without_partial_launch() {
    run_provider_projection_large_stack_test(
        "provider-batch-launch-rejects-focused-explicit-duplicate",
        provider_batch_launch_rejects_focused_and_explicit_duplicate_without_partial_launch_inner,
    );
}

async fn provider_batch_launch_rejects_focused_and_explicit_duplicate_without_partial_launch_inner()
{
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-batch-focused-duplicate",
            "worktree-batch-focused-duplicate",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let focused_launch = LaunchProviderRunRequest {
        session_id: session_id.clone(),
        agent_id: None,
        adapter_key: "dev-stub".to_string(),
        provider: "claude-code".to_string(),
        account_profile: "default".to_string(),
        model: "sonnet".to_string(),
        variant: None,
        structured_endpoint: None,
        provider_session_id: None,
        native_tui: false,
    };
    let explicit_launch = LaunchProviderRunRequest {
        agent_id: Some(agent_id.clone()),
        ..focused_launch.clone()
    };
    let launch_request = LocalDaemonRequest::LaunchProviderRuns(LaunchProviderRunsRequest {
        max_concurrency: Some(2),
        launches: vec![focused_launch, explicit_launch],
    });
    let launch_command = KernelCommand::from_local_request(
        "cmd-provider-batch-launch-focused-explicit-duplicate",
        None,
        None,
        &launch_request,
    );

    let response = router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider batch focused duplicate should return indexed failures");
    let LocalDaemonResponse::ProviderRunsLaunchAccepted {
        provider_runs,
        failures,
    } = response
    else {
        panic!("unexpected launch response");
    };
    assert!(provider_runs.is_empty());
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0].agent_id.as_deref(), Some(agent_id.as_str()));
    assert_eq!(failures[1].agent_id.as_deref(), Some(agent_id.as_str()));
    assert!(failures
        .iter()
        .all(|failure| failure.message.contains("duplicate target agents")));
    let app = app.lock().await;
    assert!(app
        .providers()
        .get_latest_run_for_agent(&session_id, &agent_id)
        .is_none());
}

#[test]
fn provider_batch_launch_accepts_mixed_sessions_with_one_kernel_request() {
    run_provider_projection_large_stack_test(
        "provider-batch-launch-accepts-mixed-sessions",
        provider_batch_launch_accepts_mixed_sessions_with_one_kernel_request_inner,
    );
}

async fn provider_batch_launch_accepts_mixed_sessions_with_one_kernel_request_inner() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (first_session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-batch-mixed-1",
            "worktree-batch-mixed-1",
        ))
        .expect("first session should be created");
    let (second_session, second_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            "workspace-batch-mixed-2",
            "worktree-batch-mixed-2",
        ))
        .expect("second session should be created");
    let first_session_id = first_session.id().to_string();
    let second_session_id = second_session.id().to_string();
    let first_agent_id = first_agent.id().to_string();
    let second_agent_id = second_agent.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let launch_request = LocalDaemonRequest::LaunchProviderRuns(LaunchProviderRunsRequest {
        max_concurrency: Some(2),
        launches: vec![
            LaunchProviderRunRequest {
                session_id: first_session_id.clone(),
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
                session_id: second_session_id.clone(),
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
    let launch_command = KernelCommand::from_local_request(
        "cmd-provider-batch-launch-mixed-session",
        None,
        None,
        &launch_request,
    );

    let response = router
        .dispatch(launch_command, launch_request)
        .await
        .expect("provider batch mixed sessions should be accepted");
    let LocalDaemonResponse::ProviderRunsLaunchAccepted {
        provider_runs,
        failures,
    } = response
    else {
        panic!("unexpected launch response");
    };
    assert!(failures.is_empty());
    assert_eq!(provider_runs.len(), 2);
    assert_eq!(provider_runs[0].index, 0);
    assert_eq!(provider_runs[1].index, 1);
    assert_eq!(
        provider_runs[0].agent_id.as_deref(),
        Some(first_agent_id.as_str())
    );
    assert_eq!(
        provider_runs[1].agent_id.as_deref(),
        Some(second_agent_id.as_str())
    );
    assert_eq!(
        provider_runs[0].provider_run.session_id(),
        first_session_id.as_str()
    );
    assert_eq!(
        provider_runs[1].provider_run.session_id(),
        second_session_id.as_str()
    );
    let app = app.lock().await;
    assert!(app
        .providers()
        .get_latest_run_for_agent(&first_session_id, &first_agent_id)
        .is_some());
    assert!(app
        .providers()
        .get_latest_run_for_agent(&second_session_id, &second_agent_id)
        .is_some());
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

#[cfg(test)]
mod catalog_projection;
#[cfg(test)]
mod process_projection;
