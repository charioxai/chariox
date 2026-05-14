use super::*;

#[tokio::test]
async fn missing_terminal_output_session_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-pump-missing-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm empty session projection");

    let app_guard = app.lock().await;
    let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
        session_id: "missing-session".to_string(),
        attachment_id: "missing-attachment".to_string(),
    });
    let pump_command =
        KernelCommand::from_local_request("cmd-pump-missing-projection", None, None, &pump_request);
    let pump_router = router.clone();
    let pump_task =
        tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

    let error = timeout(Duration::from_millis(100), pump_task)
        .await
        .expect("missing terminal output session should not wait for the app lock")
        .expect("pump task should join")
        .expect_err("missing session should fail");
    drop(app_guard);

    match error {
        DaemonError::SessionNotFound { session_id } => {
            assert_eq!(session_id, "missing-session");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn missing_terminal_output_attachment_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-pump-projection",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-pump-attachment-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm session projection");

    let app_guard = app.lock().await;
    let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
        session_id: session_id.clone(),
        attachment_id: "missing-attachment".to_string(),
    });
    let pump_command = KernelCommand::from_local_request(
        "cmd-pump-attachment-projection",
        None,
        None,
        &pump_request,
    );
    let pump_router = router.clone();
    let pump_task =
        tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

    let error = timeout(Duration::from_millis(100), pump_task)
        .await
        .expect("missing terminal output attachment should not wait for the app lock")
        .expect("pump task should join")
        .expect_err("missing attachment should fail");
    drop(app_guard);

    match error {
        DaemonError::AttachmentNotInSession {
            session_id: error_session_id,
            attachment_id,
        } => {
            assert_eq!(error_session_id, session_id);
            assert_eq!(attachment_id, "missing-attachment");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn terminal_output_without_active_run_drains_store_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-pump-buffered",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    app.fan_out_output(
        &session_id,
        "provider-run-buffered",
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"buffered output",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-pump-drain-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm session projection");

    let app_guard = app.lock().await;
    let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
    });
    let pump_command =
        KernelCommand::from_local_request("cmd-pump-drain-projection", None, None, &pump_request);
    let pump_router = router.clone();
    let pump_task =
        tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

    let pump_response = timeout(Duration::from_millis(100), pump_task)
        .await
        .expect("buffered terminal output drain should not wait for the app lock")
        .expect("pump task should join")
        .expect("pump should succeed");
    drop(app_guard);

    match pump_response {
        LocalDaemonResponse::TerminalOutput { records } => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].session_id, session_id);
            assert_eq!(records[0].bytes, b"buffered output".to_vec());
        }
        _ => panic!("unexpected pump response"),
    }
}

#[tokio::test]
async fn terminal_output_with_active_run_enters_provider_runtime_lane() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-pump-active",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let provider_run_id = launch_test_provider(
        &mut app,
        &session_id,
        agent.id(),
        "dev-stub",
        "claude-code",
        "sonnet",
    )
    .id()
    .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-pump-active-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm active provider projection");

    let permit = router
        .provider_runtime_lanes
        .acquire(&provider_run_id)
        .await;
    let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
    });
    let pump_command =
        KernelCommand::from_local_request("cmd-pump-active-lane", None, None, &pump_request);
    let pump_router = router.clone();
    let pump_task =
        tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

    tokio::task::yield_now().await;
    assert!(
        !pump_task.is_finished(),
        "active terminal output pumping should wait behind the provider-run runtime lane"
    );

    drop(permit);
    let pump_response = pump_task
        .await
        .expect("pump task should join")
        .expect("pump should succeed");
    match pump_response {
        LocalDaemonResponse::TerminalOutput { records } => {
            assert!(records.is_empty());
        }
        _ => panic!("unexpected pump response"),
    }
}

#[tokio::test]
async fn terminal_output_with_projected_inactive_run_drains_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-pump-parked",
            ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    let mut projected_session = crate::app::KernelSessionReadService::new(&app)
        .session_snapshot(&session_id)
        .expect("session snapshot should be available");
    let mut provider_run = RuntimeProviderRun::from_control_capability_inference(
        "provider-run-parked",
        session_id.clone(),
        Some(agent.id().to_string()),
        "dev-stub".to_string(),
    );
    provider_run.mark_parked();
    projected_session.set_active_provider_run(Some(provider_run.id().to_string()));
    app.fan_out_output(
        &session_id,
        provider_run.id(),
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"parked buffered output",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    router.session_projection.update(projected_session);
    router.provider_run_projection.update(provider_run.clone());

    let app_guard = app.lock().await;
    let permit = router
        .provider_runtime_lanes
        .acquire(provider_run.id())
        .await;
    let pump_request = LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
        session_id: session_id.clone(),
        attachment_id: attachment.id().to_string(),
    });
    let pump_command =
        KernelCommand::from_local_request("cmd-pump-parked-projection", None, None, &pump_request);
    let pump_router = router.clone();
    let pump_task =
        tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

    let pump_response = timeout(Duration::from_millis(100), pump_task)
        .await
        .expect("inactive run drain should not wait for app lock or provider lane")
        .expect("pump task should join")
        .expect("pump should succeed");
    drop(permit);
    drop(app_guard);

    match pump_response {
        LocalDaemonResponse::TerminalOutput { records } => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].session_id, session_id);
            assert_eq!(records[0].bytes, b"parked buffered output".to_vec());
        }
        _ => panic!("unexpected pump response"),
    }
}
