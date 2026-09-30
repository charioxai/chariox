use super::*;

/// A slice restart's first reconcile can meet a browser that is still
/// settling. A transient controller failure is retried within the start; any
/// other failure still fails the Room at once.
async fn start_with_reconcile_failures(
    code: &str,
    failures: usize,
) -> (
    Result<LocalDaemonResponse, DaemonError>,
    TestBrowserControllerTool,
    KernelRuntimeState,
    String,
) {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (session_id, terminal_stream) = {
        let mut app = app.lock().await;
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("cold-workspace", "cold-worktree"))
            .expect("session should be created");
        (session.id().to_string(), app.terminal_stream_store())
    };
    let tool = TestBrowserControllerTool::with_reconcile_failures(code, failures);
    let mut state = owned_runtime_state(&app).await;
    state.set_browser_controller_process_store_for_test(
        crate::runtime::browser_controller_process::BrowserControllerProcessStore::new(
            &tool.path,
            Vec::new(),
            Duration::from_secs(5),
        ),
    );
    let observer = state.clone();
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        state,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );
    let request =
        LocalDaemonRequest::StartRoomEnvironment(crate::local::StartRoomEnvironmentRequest {
            session_id: session_id.clone(),
            viewport: crate::local::RoomEnvironmentViewportRequest {
                css_width: 1280,
                css_height: 800,
                device_scale_factor: 1,
                desktop_pixel_width: 1280,
                desktop_pixel_height: 800,
            },
        });
    let response = runtime
        .dispatch_session_command(
            KernelCommand::from_local_request("cold-browser-start", None, None, &request),
            request,
        )
        .await;
    (response, tool, observer, session_id)
}

fn reconcile_count(tool: &TestBrowserControllerTool) -> usize {
    std::fs::read_to_string(&tool.log)
        .expect("read controller commands")
        .lines()
        .filter(|line| *line == "reconcile")
        .count()
}

#[tokio::test]
async fn a_start_outlasts_a_browser_that_times_out_while_it_settles() {
    let (response, tool, _state, _session) =
        start_with_reconcile_failures("browser_cdp_timeout", 2).await;
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } =
        response.expect("the Room starts once the browser answers")
    else {
        panic!("expected an Environment projection");
    };
    assert_eq!(reconcile_count(&tool), 3);
    assert!(environment.health.iter().any(|health| {
        health.component == crate::session::EnvironmentComponent::Browser
            && health.state == crate::session::EnvironmentComponentHealthState::Ready
    }));
    assert_eq!(environment.tabs[0].url, "https://a.test");
}

#[tokio::test]
async fn a_start_fails_at_once_on_a_browser_error_that_waiting_cannot_fix() {
    let (response, tool, state, session_id) =
        start_with_reconcile_failures("browser_viewport_invalid", 1).await;
    let error = response.expect_err("the Room start fails");
    assert!(
        error.to_string().contains("browser_viewport_invalid"),
        "{error}"
    );
    assert_eq!(reconcile_count(&tool), 1);
    let environment = state
        .room_environment_snapshot(&session_id)
        .expect("Environment");
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Failed
    );
}
