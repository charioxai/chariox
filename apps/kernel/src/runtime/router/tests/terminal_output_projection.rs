use super::*;
use crate::local::AppendNativeProviderOutputRequest;
use crate::terminal::TerminalOutputKind;

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
    router
        .record_terminal_attachment_heartbeat(&session_id, attachment.id(), 0)
        .await
        .expect("test heartbeat should be recorded");

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
    router
        .runtime_state()
        .sweep_stale_terminal_attachments(30_000)
        .await;
    assert_eq!(
        router
            .runtime_state()
            .attachment_session_id(attachment.id())
            .await
            .expect("projected pump should refresh the attachment heartbeat"),
        session_id
    );
}

#[test]
fn append_native_provider_output_does_not_refresh_session_projection() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .build()
        .expect("test runtime should build");
    runtime.block_on(async {
        tokio::spawn(async move {
            append_native_provider_output_does_not_refresh_session_projection_inner().await
        })
        .await
        .expect("test task should complete");
    });
}

async fn append_native_provider_output_does_not_refresh_session_projection_inner() {
    let worktree = crate::test_support::TestWorktree::new("append-native-provider-output");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "browser-output",
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
        KernelCommand::from_local_request("cmd-append-output-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm session projection");
    let before_sequence = router.session_projection.change_sequence();

    let append_request =
        LocalDaemonRequest::AppendNativeProviderOutput(AppendNativeProviderOutputRequest {
            session_id: session_id.clone(),
            attachment_id: attachment.id().to_string(),
            provider_run_id,
            kind: TerminalOutputKind::ProviderOutput,
            merge_key: Some("native-output".to_string()),
            text: "native output\n".to_string(),
        });
    let append_command =
        KernelCommand::from_local_request("cmd-append-output", None, None, &append_request);
    let response = router
        .dispatch(append_command, append_request)
        .await
        .expect("native output append should succeed");

    match response {
        LocalDaemonResponse::TerminalOutput { records } => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].session_id, session_id);
            assert_eq!(records[0].bytes, b"native output\n".to_vec());
        }
        _ => panic!("unexpected append response"),
    }
    assert_eq!(
        router.session_projection.change_sequence(),
        before_sequence,
        "native output append should not wake session projection subscribers",
    );
}

#[tokio::test]
async fn terminal_output_with_active_run_does_not_wait_for_busy_provider_runtime_lane() {
    let worktree = crate::test_support::TestWorktree::new("terminal-output-with-active");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
    let before_projection_sequence = router.session_projection.change_sequence();

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

    let pump_response = timeout(Duration::from_millis(100), pump_task)
        .await
        .expect("terminal output polling should not wait behind a busy provider lane")
        .expect("pump task should join")
        .expect("pump should succeed");
    drop(permit);
    match pump_response {
        LocalDaemonResponse::TerminalOutput { records } => {
            assert!(records.is_empty());
        }
        _ => panic!("unexpected pump response"),
    }
    assert_eq!(
        router.session_projection.change_sequence(),
        before_projection_sequence,
        "empty active-run terminal output pumps must not wake session projection subscribers"
    );
}

#[tokio::test]
async fn background_output_pump_enters_provider_runtime_lane() {
    let worktree = crate::test_support::TestWorktree::new("background-output-pump-enters");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let provider_run_id = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "claude-code",
        "sonnet",
    )
    .id()
    .to_string();
    app.structured_output_record_store()
        .schedule_next_poll(provider_run_id.clone(), 0);

    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let permit = router
        .provider_runtime_lanes
        .acquire(&provider_run_id)
        .await;
    let probe = router
        .provider_runtime_lanes
        .notify_on_next_acquire_for_tests(&provider_run_id);
    let runtime = router.runtime_state().clone();
    let pump_task = tokio::spawn(async move { runtime.run_provider_output_pump().await });
    timeout(Duration::from_secs(1), probe.notified())
        .await
        .expect("independent scheduler should enter the provider lane");
    assert!(!pump_task.is_finished());
    drop(permit);
    let drained = timeout(
        Duration::from_secs(1),
        router.provider_runtime_lanes.acquire(&provider_run_id),
    )
    .await
    .expect("output pump should release the lane after draining");
    drop(drained);
    pump_task.abort();
    let _ = pump_task.await;
}

#[tokio::test]
async fn terminal_output_with_active_run_drains_buffer_before_provider_lane() {
    let worktree = crate::test_support::TestWorktree::new("terminal-output-with-active");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .expect("session should be created");
    let session_id = session.id().to_string();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            &session_id,
            "cli-pump-active-buffered",
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
    app.fan_out_output(
        &session_id,
        &provider_run_id,
        crate::terminal::TerminalOutputKind::ProviderOutput,
        None,
        vec![attachment.id().to_string()],
        b"active buffered output",
    );

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command = KernelCommand::from_local_request(
        "cmd-pump-active-buffered-warm",
        None,
        None,
        &list_request,
    );
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
    let pump_command = KernelCommand::from_local_request(
        "cmd-pump-active-buffered-lane",
        None,
        None,
        &pump_request,
    );
    let pump_router = router.clone();
    let pump_task =
        tokio::spawn(async move { pump_router.dispatch(pump_command, pump_request).await });

    let pump_response = timeout(Duration::from_millis(100), pump_task)
        .await
        .expect("buffered terminal output should drain before waiting on provider lane")
        .expect("pump task should join")
        .expect("pump should succeed");
    drop(permit);

    match pump_response {
        LocalDaemonResponse::TerminalOutput { records } => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].session_id, session_id);
            assert_eq!(records[0].bytes, b"active buffered output".to_vec());
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

// MP-08/MP-10/MP-11: the idle shortcut must consult the run and prompt projections.
#[tokio::test]
async fn projected_idle_terminal_shortcut_preserves_live_native_pump() {
    assert_projected_idle_shortcut_tracks_live_run(true).await;
}

#[tokio::test]
async fn projected_idle_terminal_shortcut_preserves_stale_mirror_prompt_pump() {
    assert_projected_idle_shortcut_tracks_live_run(false).await;
}

async fn assert_projected_idle_shortcut_tracks_live_run(native: bool) {
    let worktree = crate::test_support::TestWorktree::new("projected-idle-live-run");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "native-idle-check",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    router
        .dispatch(
            KernelCommand::from_local_request("warm-native-idle-check", None, None, &request),
            request,
        )
        .await
        .unwrap();
    let projected = router.session_projection.get(session.id()).unwrap();
    assert!(!projected.has_any_prompt_work());
    assert!(projected.active_provider_run_id().is_none());
    let mut launch = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "codex",
        "codex",
        "default",
        "gpt-test",
    )
    .with_agent_id(agent.id());
    if native {
        launch = launch.with_client_interface(crate::provider::ProviderClientInterface::NativeTui);
    }
    let mut run = crate::provider::RuntimeProviderRun::new(
        "run-idle-live-check",
        &launch,
        crate::provider::ProviderLaunchResult {
            process_label: "codex:test".into(),
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: std::collections::BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("codex:test".into()),
        },
    );
    run.mark_running();
    router.provider_run_projection.update(run);
    if !native {
        router.agent_runtime_projection.update_agent_prompt_state(
            session.id(),
            agent.id(),
            Some(crate::session::PromptQueueItem::external_observed_running(
                "codex",
                "thread",
                "turn",
                agent.id(),
                "running prompt",
            )),
            None,
            0,
        );
    }
    let request = PumpTerminalOutputRequest {
        session_id: session.id().into(),
        attachment_id: attachment.id().into(),
    };
    assert!(
        router
            .terminal_output_executor
            .projected_response(&request)
            .is_none(),
        "idle session mirror must not bypass its authoritative live provider output pump"
    );
}
