use super::*;

#[tokio::test(flavor = "current_thread")]
async fn normal_project_update_reaches_session_lane_on_default_stack() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new("workspace", "worktree")
                .with_project_selection(crate::local::SessionProjectSelection::New),
        )
        .expect("session should be created");
    let project_id = session.project_id().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request =
        LocalDaemonRequest::UpdateProjectWorkspaces(crate::local::UpdateProjectWorkspacesRequest {
            project_id,
            workspace_ids: vec!["workspace".to_string()],
        });
    let command = KernelCommand::from_local_request("project-stack-budget", None, None, &request);
    assert_eq!(
        command.priority,
        crate::runtime::command::KernelCommandPriority::Normal
    );
    let response = router
        .dispatch(command, request)
        .await
        .expect("project update should run");
    assert!(matches!(
        response,
        LocalDaemonResponse::ProjectWorkspacesUpdated { .. }
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn normal_dispatch_keeps_the_request_match_out_of_its_callers_future() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::GetTerminalCommandCatalog(
        crate::local::GetTerminalCommandCatalogRequest,
    );
    let command = KernelCommand::from_local_request("stack-budget", None, None, &request);
    let future = router.dispatch_normal_or_background(command, request);
    assert!(
        std::mem::size_of_val(&future) <= 1_024,
        "normal/background dispatch must not embed the request-match future in its caller"
    );
    future
        .await
        .expect("the normal dispatch path should still execute");

    let request = LocalDaemonRequest::GetTerminalCommandCatalog(
        crate::local::GetTerminalCommandCatalogRequest,
    );
    let command = KernelCommand::from_local_request("refresh-stack-budget", None, None, &request);
    let future = router.dispatch_refresh_tracked(command, request);
    assert!(
        std::mem::size_of_val(&future) <= 1_024,
        "refresh dispatch must select its handler before constructing the caller's future"
    );
    future.await.expect("refresh dispatch should execute");

    // Exercise the surrounding public dispatch/refresh chain on the ordinary
    // test-thread stack too, without a custom runtime stack or RUST_MIN_STACK.
    let request = LocalDaemonRequest::GetTerminalCommandCatalog(
        crate::local::GetTerminalCommandCatalogRequest,
    );
    let command = KernelCommand::from_local_request("public-stack-budget", None, None, &request);
    router
        .dispatch(command, request)
        .await
        .expect("public dispatch should execute");
}
