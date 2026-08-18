use super::*;

#[tokio::test]
async fn session_runtime_publishes_attach_and_focus_projection_without_router_snapshot() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let second_agent = spawn_test_agent(&mut app, &session_id, "reviewer", "claude-code");
    assert_ne!(first_agent.id(), second_agent.id());

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let attach_request = attach_request(&session_id, "cli-session-projection");
    let attach_command = KernelCommand::from_local_request(
        "cmd-session-projection-attach",
        None,
        None,
        &attach_request,
    );
    let attachment_id = match router
        .dispatch(attach_command, attach_request)
        .await
        .expect("attach should succeed")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        _ => panic!("unexpected attach response"),
    };

    let focus_request = focus_request(&session_id, second_agent.id());
    let focus_command = KernelCommand::from_local_request(
        "cmd-session-projection-focus",
        None,
        None,
        &focus_request,
    );
    router
        .dispatch(focus_command, focus_request)
        .await
        .expect("focus should succeed");

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-session-projection-state",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "session state should come from the SessionRuntime-published projection without taking the app lock"
    );
    drop(app_guard);

    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState {
            session,
            agent_activity,
            ..
        } => {
            assert!(session.has_attachment(&attachment_id));
            assert_eq!(session.focused_agent_id(), Some(second_agent.id()));
            assert!(agent_activity.contains_key(second_agent.id()));
        }
        _ => panic!("unexpected session state response"),
    }
}

#[tokio::test]
async fn agent_lifecycle_refresh_uses_published_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        account_profile: None,
        session_id: session_id.clone(),
        alias: Some("projected-agent".to_string()),
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
    });
    let spawn_command =
        KernelCommand::from_local_request("cmd-agent-lifecycle-spawn", None, None, &spawn_request);
    let spawned_agent_id = match router
        .dispatch(spawn_command, spawn_request)
        .await
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
        _ => panic!("unexpected spawn response"),
    };
    assert!(
        router.session_runtime.has_lane(&session_id).await,
        "agent lifecycle should run through the session runtime lane"
    );

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-agent-lifecycle-spawn-state",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
    let state_response = timeout(Duration::from_millis(100), state_task)
        .await
        .expect("spawn-projected state should not wait for the app lock")
        .expect("state task should join")
        .expect("state should resolve");
    drop(app_guard);
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert!(session
                .agents()
                .iter()
                .any(|agent| agent.id() == spawned_agent_id));
        }
        _ => panic!("unexpected state response"),
    }

    let destroy_request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
        session_id: session_id.clone(),
        agent_id: spawned_agent_id.clone(),
    });
    let destroy_command = KernelCommand::from_local_request(
        "cmd-agent-lifecycle-destroy",
        None,
        None,
        &destroy_request,
    );
    router
        .dispatch(destroy_command, destroy_request)
        .await
        .expect("destroy should succeed");
    assert!(
        router.session_runtime.has_lane(&session_id).await,
        "destroying an agent should not bypass the session runtime lane"
    );

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-agent-lifecycle-destroy-state",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });
    let state_response = timeout(Duration::from_millis(100), state_task)
        .await
        .expect("destroy-projected state should not wait for the app lock")
        .expect("state task should join")
        .expect("state should resolve");
    drop(app_guard);
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert!(!session
                .agents()
                .iter()
                .any(|agent| agent.id() == spawned_agent_id));
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn end_session_uses_session_lane_and_removes_lane_registration() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let attach_request = attach_request(&session_id, "cli-1");
    let attach_command =
        KernelCommand::from_local_request("cmd-attach", None, None, &attach_request);
    router
        .dispatch(attach_command, attach_request)
        .await
        .expect("attach should create a session lane");
    assert!(router.session_runtime.has_lane(&session_id).await);

    let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: session_id.clone(),
    });
    let end_command = KernelCommand::from_local_request("cmd-end", None, None, &end_request);
    let response = router
        .dispatch(end_command, end_request)
        .await
        .expect("end session should run through the session lane");

    assert!(matches!(
        response,
        crate::local::LocalDaemonResponse::SessionEnded { .. }
    ));
    assert!(
        !router.session_runtime.has_lane(&session_id).await,
        "ending a session should remove its mailbox registration"
    );
}

#[tokio::test]
async fn end_session_detaches_reusable_slice_agents() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace", "worktree")
                .with_agent_defaults(crate::session::SessionAgentDefaults::new("dev-stub")),
        )
        .expect("session should be created");
    let session_id = session.id().to_string();
    let second_agent = spawn_test_agent(&mut app, &session_id, "second", "dev-stub");
    let slice = app
        .slices()
        .create(
            app.config().daemon_id.as_str(),
            app.config().host_machine_id.as_str(),
            crate::slice::CreateSliceInput {
                name: "test-slice".to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                workspace_id: Some("workspace".to_string()),
                worktree_id: Some("worktree".to_string()),
                workspace_mount: Some("worktree".to_string()),
                worker_kernel_ref: Some("slice:test-slice".to_string()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )
        .expect("slice should be created");
    app.slices()
        .attach_agent(
            &slice.id,
            &session_id,
            first_agent.id(),
            crate::session::unix_epoch_ms(),
        )
        .expect("first agent should attach");
    app.slices()
        .attach_agent(
            &slice.id,
            &session_id,
            second_agent.id(),
            crate::session::unix_epoch_ms(),
        )
        .expect("second agent should attach");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: session_id.clone(),
    });
    let end_command = KernelCommand::from_local_request("cmd-end-slice", None, None, &end_request);

    router
        .dispatch(end_command, end_request)
        .await
        .expect("end session should detach slice agents");

    let app_guard = app.lock().await;
    let detached = app_guard
        .slices()
        .resolve(&slice.id)
        .expect("slice should remain reusable");
    assert!(detached.agent_ids.is_empty());
    assert!(detached.session_ids.is_empty());
}

#[tokio::test]
async fn create_slice_ignores_client_supplied_provider_auth() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::CreateSlice(crate::local::CreateSliceRequest {
        name: "forged-auth".to_string(),
        backend: crate::slice::SliceBackendKind::LocalDocker,
        os: "linux".to_string(),
        display_mode: crate::slice::SliceDisplayMode::Headless,
        workspace_id: Some("workspace".to_string()),
        worktree_id: Some("worktree".to_string()),
        workspace_mount: Some("worktree".to_string()),
        worker_kernel_ref: None,
        display_url: None,
        provider_auth: vec![crate::slice_provider_auth::SliceProviderAuthSummary {
            provider: "codex".to_string(),
            account_profile: "default".to_string(),
            state: crate::slice_provider_auth::SliceProviderAuthState::Authenticated,
            auth_type: Some("forged".to_string()),
            account_id: Some("forged-account".to_string()),
            email: Some("forged@example.com".to_string()),
            organization_id: None,
            organization_name: None,
            subscription_type: None,
            source: "client".to_string(),
        }],
        from_saved_state: None,
        base: None,
    });
    let command =
        KernelCommand::from_local_request("cmd-create-slice-forged-auth", None, None, &request);

    let response = router
        .dispatch(command, request)
        .await
        .expect("slice create should succeed");

    let created_slice = match response {
        LocalDaemonResponse::SliceCreated { slice } => {
            assert!(
                slice.provider_auth.is_empty(),
                "provider auth summaries are kernel-owned and must not be accepted from clients"
            );
            slice
        }
        _ => panic!("unexpected create slice response"),
    };

    let app_guard = app.lock().await;
    let events = app_guard
        .durable_state_store()
        .load_events_after(0)
        .expect("durable events should load");
    let audit = events
        .iter()
        .find(|event| {
            event.kind == "slice.audit"
                && event.subject_id.as_deref() == Some(created_slice.id.as_str())
                && event.payload["action"] == "create"
        })
        .expect("slice create should write an audit event");
    assert_eq!(audit.payload["result"], "completed");
    assert_eq!(audit.payload["actor"], "kernel");
    assert_eq!(audit.payload["client_type"], "local_daemon");
    assert_eq!(audit.payload["workspace_id"], "workspace");
    assert_eq!(audit.payload["worktree_id"], "worktree");
    assert_eq!(audit.payload["workspace_mount"], "worktree");
    assert_eq!(audit.payload["provider"], serde_json::Value::Null);
    assert_eq!(audit.payload["redacted_error"], serde_json::Value::Null);
}

#[tokio::test]
async fn unsupported_slice_auth_mutations_fail_loudly_and_audit() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let create_request = LocalDaemonRequest::CreateSlice(crate::local::CreateSliceRequest {
        name: "ssh-slice".to_string(),
        backend: crate::slice::SliceBackendKind::SshDocker,
        os: "linux".to_string(),
        display_mode: crate::slice::SliceDisplayMode::Headless,
        workspace_id: Some("workspace".to_string()),
        worktree_id: Some("worktree".to_string()),
        workspace_mount: Some("worktree".to_string()),
        worker_kernel_ref: None,
        display_url: None,
        provider_auth: Vec::new(),
        from_saved_state: None,
        base: None,
    });
    let create_command =
        KernelCommand::from_local_request("cmd-create-ssh-slice", None, None, &create_request);
    let slice = match router
        .dispatch(create_command, create_request)
        .await
        .expect("slice create should succeed")
    {
        LocalDaemonResponse::SliceCreated { slice } => slice,
        _ => panic!("unexpected create slice response"),
    };

    let import_request =
        LocalDaemonRequest::ImportSliceProviderAuth(crate::local::ImportSliceProviderAuthRequest {
            slice_ref: slice.id.clone(),
            provider: "codex".to_string(),
            account_profile: "default".to_string(),
        });
    let import_command =
        KernelCommand::from_local_request("cmd-import-ssh-slice-auth", None, None, &import_request);
    let error = router
        .dispatch(import_command, import_request)
        .await
        .expect_err("unsupported slice auth import should fail");

    assert!(
        error
            .to_string()
            .contains("only implemented for local Docker slices"),
        "unexpected error: {error}"
    );
    let app_guard = app.lock().await;
    let events = app_guard
        .durable_state_store()
        .load_events_after(0)
        .expect("durable events should load");
    let audit = events
        .iter()
        .find(|event| {
            event.kind == "slice.audit"
                && event.subject_id.as_deref() == Some(slice.id.as_str())
                && event.payload["action"] == "auth.import"
                && event.payload["result"] == "failed"
        })
        .expect("failed auth import should write an audit event");
    assert_eq!(audit.payload["provider"], "codex");
    assert!(audit.payload["redacted_error"]
        .as_str()
        .unwrap_or_default()
        .contains("only implemented for local Docker slices"));
    drop(app_guard);

    let remove_request =
        LocalDaemonRequest::RemoveSliceProviderAuth(crate::local::RemoveSliceProviderAuthRequest {
            slice_ref: slice.id.clone(),
            provider: "codex".to_string(),
            account_profile: "default".to_string(),
        });
    let remove_command =
        KernelCommand::from_local_request("cmd-remove-ssh-slice-auth", None, None, &remove_request);
    let error = router
        .dispatch(remove_command, remove_request)
        .await
        .expect_err("unsupported slice auth removal should fail");
    assert!(
        error
            .to_string()
            .contains("only implemented for local Docker slices"),
        "unexpected error: {error}"
    );
    let app_guard = app.lock().await;
    let events = app_guard
        .durable_state_store()
        .load_events_after(0)
        .expect("durable events should load");
    assert!(events.iter().any(|event| {
        event.kind == "slice.audit"
            && event.subject_id.as_deref() == Some(slice.id.as_str())
            && event.payload["action"] == "auth.remove"
            && event.payload["result"] == "failed"
    }));
}

#[tokio::test]
async fn create_session_rejects_slice_from_another_worktree() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    create_router_test_slice(&mut app, "dev", "workspace", "other-worktree");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace", "worktree").with_slice_ref("dev"),
    );
    let command =
        KernelCommand::from_local_request("cmd-create-session-wrong-slice", None, None, &request);

    let error = router
        .dispatch(command, request)
        .await
        .expect_err("wrong-worktree slice should be rejected by the kernel");

    assert!(
        error
            .to_string()
            .contains("slice `dev` belongs to worktree `other-worktree`, not `worktree`"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn spawn_agent_rejects_slice_from_another_worktree() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    create_router_test_slice(&mut app, "dev", "workspace", "other-worktree");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        account_profile: None,
        session_id,
        alias: Some("slice-agent".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some("worktree".to_string()),
        kernel_ref: None,
        slice_ref: Some("dev".to_string()),
        worktree_placement: None,
        metaagent: false,
    });
    let command =
        KernelCommand::from_local_request("cmd-spawn-agent-wrong-slice", None, None, &request);

    let error = router
        .dispatch(command, request)
        .await
        .expect_err("wrong-worktree slice should be rejected by the kernel");

    assert!(
        error
            .to_string()
            .contains("slice `dev` belongs to worktree `other-worktree`, not `worktree`"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn delete_session_uses_owned_runtime_state_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let create_request = LocalDaemonRequest::CreateSession(
        CreateSessionRequest::new("workspace-delete-projection", "worktree").with_alias("doomed"),
    );
    let create_command = KernelCommand::from_local_request(
        "cmd-delete-projection-create",
        None,
        None,
        &create_request,
    );
    let session_id = match router
        .dispatch(create_command, create_request)
        .await
        .expect("create should warm session projection")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
        _ => panic!("unexpected create response"),
    };

    let app_guard = app.lock().await;
    let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
        session_ref: "doomed".to_string(),
        workspace_id: Some("workspace-delete-projection".to_string()),
    });
    let delete_command =
        KernelCommand::from_local_request("cmd-delete-projection", None, None, &delete_request);
    let delete_router = router.clone();
    let delete_task =
        tokio::spawn(async move { delete_router.dispatch(delete_command, delete_request).await });

    let delete_response = timeout(Duration::from_millis(100), delete_task)
        .await
        .expect("owned delete should not wait for the app lock")
        .expect("delete task should join")
        .expect("delete should succeed");
    drop(app_guard);
    assert!(matches!(
        delete_response,
        LocalDaemonResponse::SessionDeleted { .. }
    ));
    assert!(
        !router.session_runtime.has_lane(&session_id).await,
        "deleting a session should remove its mailbox registration"
    );
}

#[tokio::test]
async fn missing_delete_session_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-list-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("list should warm the session projection");

    let app_guard = app.lock().await;
    let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
        session_ref: "missing-session".to_string(),
        workspace_id: None,
    });
    let delete_command =
        KernelCommand::from_local_request("cmd-delete-missing", None, None, &delete_request);
    let delete_router = router.clone();
    let delete_task =
        tokio::spawn(async move { delete_router.dispatch(delete_command, delete_request).await });

    let error = timeout(Duration::from_millis(100), delete_task)
        .await
        .expect("missing delete should not wait for the app lock")
        .expect("delete task should join")
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
async fn missing_detach_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-list-warm-detach", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("list should warm the session projection");

    let app_guard = app.lock().await;
    let detach_request = LocalDaemonRequest::DetachFromSession(DetachFromSessionRequest {
        attachment_id: "missing-attachment".to_string(),
    });
    let detach_command =
        KernelCommand::from_local_request("cmd-detach-missing", None, None, &detach_request);
    let detach_router = router.clone();
    let detach_task =
        tokio::spawn(async move { detach_router.dispatch(detach_command, detach_request).await });

    let error = timeout(Duration::from_millis(100), detach_task)
        .await
        .expect("missing detach should not wait for the app lock")
        .expect("detach task should join")
        .expect_err("missing attachment should fail");
    drop(app_guard);

    match error {
        DaemonError::AttachmentNotFound { attachment_id } => {
            assert_eq!(attachment_id, "missing-attachment");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn missing_attach_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-attach-missing-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("list should warm the session projection");

    let app_guard = app.lock().await;
    let attach_request = attach_request("missing-session", "cli-missing-session");
    let attach_command =
        KernelCommand::from_local_request("cmd-attach-missing", None, None, &attach_request);
    let attach_router = router.clone();
    let attach_task =
        tokio::spawn(async move { attach_router.dispatch(attach_command, attach_request).await });

    let error = timeout(Duration::from_millis(100), attach_task)
        .await
        .expect("missing attach should not wait for the app lock")
        .expect("attach task should join")
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
async fn missing_alias_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-alias-missing-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("list should warm the session projection");

    let app_guard = app.lock().await;
    let alias_request = LocalDaemonRequest::AliasSession(AliasSessionRequest {
        session_id: "missing-session".to_string(),
        alias: "review".to_string(),
    });
    let alias_command =
        KernelCommand::from_local_request("cmd-alias-missing", None, None, &alias_request);
    let alias_router = router.clone();
    let alias_task =
        tokio::spawn(async move { alias_router.dispatch(alias_command, alias_request).await });

    let error = timeout(Duration::from_millis(100), alias_task)
        .await
        .expect("missing alias should not wait for the app lock")
        .expect("alias task should join")
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
async fn missing_end_session_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-end-missing-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("list should warm the session projection");

    let app_guard = app.lock().await;
    let end_request = LocalDaemonRequest::EndSession(EndSessionRequest {
        session_id: "missing-session".to_string(),
    });
    let end_command =
        KernelCommand::from_local_request("cmd-end-missing", None, None, &end_request);
    let end_router = router.clone();
    let end_task = tokio::spawn(async move { end_router.dispatch(end_command, end_request).await });

    let error = timeout(Duration::from_millis(100), end_task)
        .await
        .expect("missing end should not wait for the app lock")
        .expect("end task should join")
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
async fn invalid_focus_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-focus-invalid-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("list should warm the session projection");

    let app_guard = app.lock().await;
    let focus_request = focus_request(&session_id, "missing-agent");
    let focus_command =
        KernelCommand::from_local_request("cmd-focus-invalid", None, None, &focus_request);
    let focus_router = router.clone();
    let focus_task =
        tokio::spawn(async move { focus_router.dispatch(focus_command, focus_request).await });

    let error = timeout(Duration::from_millis(100), focus_task)
        .await
        .expect("invalid focus should not wait for the app lock")
        .expect("focus task should join")
        .expect_err("missing agent should fail");
    drop(app_guard);

    match error {
        DaemonError::AgentNotInSession {
            session_id: error_session_id,
            agent_id,
        } => {
            assert_eq!(error_session_id, session_id);
            assert_eq!(agent_id, "missing-agent");
        }
        error => panic!("unexpected error: {error}"),
    }
}

#[tokio::test]
async fn missing_cycle_focus_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-cycle-missing-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("list should warm the session projection");

    let app_guard = app.lock().await;
    let cycle_request = LocalDaemonRequest::CycleAgentFocus(CycleAgentFocusRequest {
        session_id: "missing-session".to_string(),
    });
    let cycle_command =
        KernelCommand::from_local_request("cmd-cycle-missing", None, None, &cycle_request);
    let cycle_router = router.clone();
    let cycle_task =
        tokio::spawn(async move { cycle_router.dispatch(cycle_command, cycle_request).await });

    let error = timeout(Duration::from_millis(100), cycle_task)
        .await
        .expect("missing cycle focus should not wait for the app lock")
        .expect("cycle task should join")
        .expect_err("missing session should fail");
    drop(app_guard);

    match error {
        DaemonError::SessionNotFound { session_id } => {
            assert_eq!(session_id, "missing-session");
        }
        error => panic!("unexpected error: {error}"),
    }
}

fn create_router_test_slice(
    app: &mut DaemonApp,
    name: &str,
    workspace_id: &str,
    worktree_id: &str,
) -> crate::slice::SliceRecord {
    app.slices()
        .create(
            app.config().daemon_id.as_str(),
            app.config().host_machine_id.as_str(),
            crate::slice::CreateSliceInput {
                name: name.to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                workspace_id: Some(workspace_id.to_string()),
                worktree_id: Some(worktree_id.to_string()),
                workspace_mount: Some(worktree_id.to_string()),
                worker_kernel_ref: Some(format!("slice:{name}")),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )
        .expect("slice should be created")
}
