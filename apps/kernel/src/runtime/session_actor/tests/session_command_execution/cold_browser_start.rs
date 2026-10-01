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

#[tokio::test]
async fn refused_physical_viewport_does_not_publish_a_new_canonical_revision() {
    let (started, tool, state, session_id) = start_with_reconcile_failures("", 0).await;
    started.expect("initial viewport should start");
    state.transition_room_environment(&session_id, crate::session::EnvironmentLifecycle::Ready).expect("fixture reaches Ready after controller startup");
    state.reconcile_room_environment_actors(&session_id, Some(crate::session::DEFAULT_LOCAL_USER_ID)).expect("local actor participates");
    state.request_room_environment_takeover_as_actor(&session_id, crate::session::EnvironmentActor::new(crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID), crate::session::EnvironmentActorKind::Human, "local"), crate::session::InputTarget::Desktop).expect("local user owns Desktop input");
    assert!(state.browser_controller_enabled_for_room(&session_id));
    let before = state.room_environment_snapshot(&session_id).unwrap();
    assert_eq!(before.lifecycle, crate::session::EnvironmentLifecycle::Ready);
    assert!(before.input_ownership.iter().any(|ownership| ownership.target == crate::session::InputTarget::Desktop && ownership.actor_id == crate::session::human_environment_actor_id(crate::session::DEFAULT_LOCAL_USER_ID)));
    // The controller fixture keeps reporting 1280x800. A 1024x768 request
    // therefore fails the real reconciliation dimension check.
    let store = crate::runtime::session_actor::store::SessionRuntimeStore::new(state.clone());
    let (result, _) = store.update_room_environment_viewport(
        crate::local::UpdateRoomEnvironmentViewportRequest {
            session_id: session_id.clone(),
            expected_revision: before.viewport.revision,
            viewport: crate::local::RoomEnvironmentViewportRequest {
                css_width: 1024, css_height: 768, device_scale_factor: 1,
                desktop_pixel_width: 1024, desktop_pixel_height: 768,
            },
        }, crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
    ).await;
    assert!(matches!(before.lifecycle, crate::session::EnvironmentLifecycle::Ready | crate::session::EnvironmentLifecycle::Degraded));
    assert!(reconcile_count(&tool) > 1, "must reach physical reconcile, got {result:?}; owners {:?}", before.input_ownership);
    assert!(result.is_err(), "a refused display must not be an accepted resize: {result:?}");
    let after = state.room_environment_snapshot(&session_id).unwrap();
    assert_eq!(after.viewport, before.viewport);
    assert_eq!(after.actors, before.actors);
    let (unsupported, _) = store.update_room_environment_viewport(
        crate::local::UpdateRoomEnvironmentViewportRequest {
            session_id: session_id.clone(), expected_revision: before.viewport.revision,
            viewport: crate::local::RoomEnvironmentViewportRequest {
                css_width: 393, css_height: 844, device_scale_factor: 1,
                desktop_pixel_width: 391, desktop_pixel_height: 844,
            },
        }, crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
    ).await;
    assert!(unsupported.is_err(), "unverified physical sizes must be refused");
    assert_eq!(state.room_environment_snapshot(&session_id).unwrap().viewport, before.viewport);
}
