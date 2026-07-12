use super::*;

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
        provider_run_id: None,
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
