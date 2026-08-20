use super::*;

#[tokio::test]
async fn list_sessions_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-list-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm the projection");

    let app_guard = app.lock().await;
    let projected_list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let projected_list_command = KernelCommand::from_local_request(
        "cmd-list-projection",
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
        "warmed ListSessions should be served from the session list projection without app lock access"
    );

    drop(app_guard);
    let list_response = list_task
        .await
        .expect("list task should join")
        .expect("list should resolve");
    match list_response {
        LocalDaemonResponse::SessionsListed { sessions } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id(), session_id);
        }
        _ => panic!("unexpected list response"),
    }
}

#[tokio::test]
async fn get_session_state_uses_list_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-list-state-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should hydrate per-session projection entries");

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let state_command =
        KernelCommand::from_local_request("cmd-list-state-projection", None, None, &state_request);
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    tokio::task::yield_now().await;
    assert!(
        state_task.is_finished(),
        "ListSessions warm-up should hydrate GetSessionState projection entries without app lock access"
    );

    drop(app_guard);
    let state_response = state_task
        .await
        .expect("state task should join")
        .expect("state should resolve");
    match state_response {
        LocalDaemonResponse::SessionState { session, .. } => {
            assert_eq!(session.id(), session_id);
        }
        _ => panic!("unexpected state response"),
    }
}

#[tokio::test]
async fn missing_session_state_uses_list_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-list-missing-state-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm empty session projection");

    let app_guard = app.lock().await;
    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: "missing-session".to_string(),
    });
    let state_command = KernelCommand::from_local_request(
        "cmd-missing-state-projection",
        None,
        None,
        &state_request,
    );
    let state_router = router.clone();
    let state_task =
        tokio::spawn(async move { state_router.dispatch(state_command, state_request).await });

    let error = timeout(Duration::from_millis(100), state_task)
        .await
        .expect("missing state should not wait for the app lock")
        .expect("state task should join")
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
async fn resolve_session_uses_warmed_projection_without_app_lock() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let session_prefix = session_id[..8].to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-resolve-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm visible session projection entries");

    let app_guard = app.lock().await;
    let resolve_request = LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
        session_ref: session_prefix,
        workspace_id: Some("workspace".to_string()),
    });
    let resolve_command =
        KernelCommand::from_local_request("cmd-resolve-projection", None, None, &resolve_request);
    let resolve_router = router.clone();
    let resolve_task = tokio::spawn(async move {
        resolve_router
            .dispatch(resolve_command, resolve_request)
            .await
    });

    tokio::task::yield_now().await;
    assert!(
        resolve_task.is_finished(),
        "warmed ResolveSession should return from session projection without app lock access"
    );

    drop(app_guard);
    let resolve_response = resolve_task
        .await
        .expect("resolve task should join")
        .expect("resolve should succeed");
    match resolve_response {
        LocalDaemonResponse::SessionResolved { session } => {
            assert_eq!(session.id(), session_id);
        }
        _ => panic!("unexpected resolve response"),
    }
}

#[tokio::test]
async fn missing_resolve_session_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-resolve-missing-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm empty session projection");

    let app_guard = app.lock().await;
    let resolve_request = LocalDaemonRequest::ResolveSession(ResolveSessionRequest {
        session_ref: "missing-session".to_string(),
        workspace_id: None,
    });
    let resolve_command = KernelCommand::from_local_request(
        "cmd-resolve-missing-projection",
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

    let error = timeout(Duration::from_millis(100), resolve_task)
        .await
        .expect("missing resolve should not wait for the app lock")
        .expect("resolve task should join")
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
async fn missing_session_inspection_uses_warmed_projection_without_app_lock() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-inspection-missing-warm", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm empty session projection");

    let app_guard = app.lock().await;
    let inspection_request = LocalDaemonRequest::ListAgents(ListAgentsRequest {
        session_id: "missing-session".to_string(),
    });
    let inspection_command = KernelCommand::from_local_request(
        "cmd-inspection-missing-projection",
        None,
        None,
        &inspection_request,
    );
    let inspection_router = router.clone();
    let inspection_task = tokio::spawn(async move {
        inspection_router
            .dispatch(inspection_command, inspection_request)
            .await
    });

    let error = timeout(Duration::from_millis(100), inspection_task)
        .await
        .expect("missing inspection should not wait for the app lock")
        .expect("inspection task should join")
        .expect_err("missing session should fail");
    drop(app_guard);

    match error {
        DaemonError::SessionNotFound { session_id } => {
            assert_eq!(session_id, "missing-session");
        }
        error => panic!("unexpected error: {error}"),
    }
}

fn session_inspection_projection_setup() -> (Arc<Mutex<DaemonApp>>, Box<CommandRouter>, String) {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = Box::new(CommandRouter::with_interactive_capacity(
        Arc::clone(&app),
        1,
    ));
    (app, router, session_id)
}

fn spawn_session_inspection_request(
    router: &CommandRouter,
    command_id: &'static str,
    request: LocalDaemonRequest,
) -> tokio::task::JoinHandle<Result<LocalDaemonResponse, DaemonError>> {
    let command = KernelCommand::from_local_request(command_id, None, None, &request);
    let router = router.clone();
    tokio::spawn(async move { router.dispatch(command, request).await })
}

#[tokio::test]
async fn session_inspection_reads_use_warmed_projection_without_app_lock() {
    let (app, router, session_id) = session_inspection_projection_setup();

    let spawn_request = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        account_profile: None,
        session_id: session_id.clone(),
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
    });
    let spawn_command =
        KernelCommand::from_local_request("cmd-inspection-spawn", None, None, &spawn_request);
    router
        .dispatch(spawn_command, spawn_request)
        .await
        .expect("spawn should refresh the session projection");

    let create_workflow_request = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
        session_id: session_id.clone(),
        alias: Some("inspection".to_string()),
    });
    let create_workflow_command = KernelCommand::from_local_request(
        "cmd-inspection-workflow",
        None,
        None,
        &create_workflow_request,
    );
    let workflow_id = match router
        .dispatch(create_workflow_command, create_workflow_request)
        .await
        .expect("workflow should create")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
        _ => panic!("unexpected workflow response"),
    };

    let app_guard = app.lock().await;
    let list_agents_task = spawn_session_inspection_request(
        &router,
        "cmd-inspection-agents",
        LocalDaemonRequest::ListAgents(ListAgentsRequest {
            session_id: session_id.clone(),
        }),
    );
    let list_workflows_task = spawn_session_inspection_request(
        &router,
        "cmd-inspection-workflows",
        LocalDaemonRequest::ListWorkflows(ListWorkflowsRequest {
            session_id: session_id.clone(),
        }),
    );
    let resolve_workflow_task = spawn_session_inspection_request(
        &router,
        "cmd-inspection-resolve-workflow",
        LocalDaemonRequest::ResolveWorkflow(ResolveWorkflowRequest {
            session_id: session_id.clone(),
            workflow_ref: "inspection".to_string(),
        }),
    );
    let list_runs_task = spawn_session_inspection_request(
        &router,
        "cmd-inspection-runs",
        LocalDaemonRequest::ListWorkflowRuns(ListWorkflowRunsRequest {
            session_id: session_id.clone(),
            workflow_ref: Some("inspection".to_string()),
            cursor: None,
            limit: None,
        }),
    );
    let list_watchdogs_task = spawn_session_inspection_request(
        &router,
        "cmd-inspection-watchdogs",
        LocalDaemonRequest::ListWorkflowWatchdogs(ListWorkflowWatchdogsRequest {
            session_id: session_id.clone(),
            workflow_ref: Some("inspection".to_string()),
        }),
    );

    tokio::task::yield_now().await;
    assert!(list_agents_task.is_finished());
    assert!(list_workflows_task.is_finished());
    assert!(resolve_workflow_task.is_finished());
    assert!(list_runs_task.is_finished());
    assert!(list_watchdogs_task.is_finished());
    drop(app_guard);

    match list_agents_task
        .await
        .expect("list agents task should join")
        .expect("agents should list")
    {
        LocalDaemonResponse::AgentsListed { agents } => {
            assert_eq!(agents.len(), 2);
        }
        _ => panic!("unexpected agents response"),
    }
    match list_workflows_task
        .await
        .expect("list workflows task should join")
        .expect("workflows should list")
    {
        LocalDaemonResponse::WorkflowsListed { workflows } => {
            assert_eq!(workflows.len(), 1);
            assert_eq!(workflows[0].id(), workflow_id);
        }
        _ => panic!("unexpected workflows response"),
    }
    match resolve_workflow_task
        .await
        .expect("resolve workflow task should join")
        .expect("workflow should resolve")
    {
        LocalDaemonResponse::WorkflowResolved { workflow } => {
            assert_eq!(workflow.id(), workflow_id);
        }
        _ => panic!("unexpected workflow resolve response"),
    }
    match list_runs_task
        .await
        .expect("list runs task should join")
        .expect("workflow runs should list")
    {
        LocalDaemonResponse::WorkflowRunsListed { workflow_runs, .. } => {
            assert!(workflow_runs.is_empty());
        }
        _ => panic!("unexpected workflow runs response"),
    }
    match list_watchdogs_task
        .await
        .expect("list watchdogs task should join")
        .expect("workflow watchdogs should list")
    {
        LocalDaemonResponse::WorkflowWatchdogsListed { watchdogs } => {
            assert!(watchdogs.is_empty());
        }
        _ => panic!("unexpected workflow watchdogs response"),
    }
}

#[tokio::test]
async fn warmed_session_list_projection_tracks_create_and_delete_responses() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_command =
        KernelCommand::from_local_request("cmd-list-empty", None, None, &list_request);
    router
        .dispatch(list_command, list_request)
        .await
        .expect("initial list should warm an empty projection");

    let create_request = LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
        "workspace-list-projection",
        "worktree-list-projection",
    ));
    let create_command =
        KernelCommand::from_local_request("cmd-create-for-list", None, None, &create_request);
    let created_session_id = match router
        .dispatch(create_command, create_request)
        .await
        .expect("create should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session.id().to_string(),
        _ => panic!("unexpected create response"),
    };

    let app_guard = app.lock().await;
    let projected_list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let projected_list_command = KernelCommand::from_local_request(
        "cmd-list-after-create",
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
    assert!(list_task.is_finished());
    drop(app_guard);
    let list_response = list_task
        .await
        .expect("list task should join")
        .expect("list should resolve");
    match list_response {
        LocalDaemonResponse::SessionsListed { sessions } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id(), created_session_id);
        }
        _ => panic!("unexpected list response"),
    }

    let delete_request = LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
        session_ref: created_session_id.clone(),
        workspace_id: None,
    });
    let delete_command =
        KernelCommand::from_local_request("cmd-delete-for-list", None, None, &delete_request);
    router
        .dispatch(delete_command, delete_request)
        .await
        .expect("delete should succeed");

    let app_guard = app.lock().await;
    let projected_list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let projected_list_command = KernelCommand::from_local_request(
        "cmd-list-after-delete",
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
    assert!(list_task.is_finished());
    drop(app_guard);
    let list_response = list_task
        .await
        .expect("list task should join")
        .expect("list should resolve");
    match list_response {
        LocalDaemonResponse::SessionsListed { sessions } => {
            assert!(sessions.is_empty());
        }
        _ => panic!("unexpected list response"),
    }
}

#[tokio::test]
async fn delete_kernel_removes_current_kernel_sessions_from_projection() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-kernel-delete",
            "worktree-kernel-delete",
        ))
        .expect("session should create");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_response = router
        .dispatch(
            KernelCommand::from_local_request(
                "cmd-list-before-kernel-delete",
                None,
                None,
                &list_request,
            ),
            list_request,
        )
        .await
        .expect("list before kernel delete should resolve");
    match list_response {
        LocalDaemonResponse::SessionsListed { sessions } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id(), session_id);
        }
        _ => panic!("unexpected list response"),
    }

    let delete_request = LocalDaemonRequest::DeleteKernel(DeleteKernelRequest);
    let delete_response = router
        .dispatch(
            KernelCommand::from_local_request("cmd-delete-kernel", None, None, &delete_request),
            delete_request,
        )
        .await
        .expect("kernel delete should resolve");
    match delete_response {
        LocalDaemonResponse::KernelDeleted {
            deleted_sessions, ..
        } => assert_eq!(deleted_sessions.len(), 1),
        _ => panic!("unexpected kernel delete response"),
    }

    let list_request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let list_response = router
        .dispatch(
            KernelCommand::from_local_request(
                "cmd-list-after-kernel-delete",
                None,
                None,
                &list_request,
            ),
            list_request,
        )
        .await
        .expect("list after kernel delete should resolve");
    match list_response {
        LocalDaemonResponse::SessionsListed { sessions } => assert!(sessions.is_empty()),
        _ => panic!("unexpected list response"),
    }
}
