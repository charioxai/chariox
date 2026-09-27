use super::*;

#[tokio::test]
async fn different_rooms_cannot_share_a_physical_browser_even_after_stop() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let (first_room, second_room, terminal_stream) = {
        let mut app = app.lock().await;
        let mut sessions = crate::app::KernelSessionService::new(&mut app);
        let (first, _) = sessions
            .create_session(CreateSessionRequest::new(
                "first-workspace",
                "first-worktree",
            ))
            .expect("first Room");
        let (second, _) = sessions
            .create_session(CreateSessionRequest::new(
                "second-workspace",
                "second-worktree",
            ))
            .expect("second Room");
        (
            first.id().to_string(),
            second.id().to_string(),
            app.terminal_stream_store(),
        )
    };
    let tool = TestBrowserControllerTool::new();
    let mut state = owned_runtime_state(&app).await;
    state.set_browser_controller_process_store_for_test(
        crate::runtime::browser_controller_process::BrowserControllerProcessStore::new(
            &tool.path,
            Vec::new(),
            Duration::from_secs(5),
        ),
    );
    let runtime = SessionRuntime::with_queue_limit_and_focus_projection(
        state,
        1,
        FocusedAgentProjection::default(),
        SessionStateProjectionStore::default(),
        AgentRuntimeProjectionStore::default(),
        terminal_stream,
    );

    let initial = dispatch(&runtime, start(&first_room))
        .await
        .expect("first Room starts");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: initial,
    } = initial
    else {
        panic!("expected Environment projection");
    };
    assert_eq!(initial.tabs[0].url, "https://a.test");

    let denied = dispatch(&runtime, start(&second_room)).await;
    assert!(
        denied.is_err(),
        "second Room must not receive the first Room's tabs: {denied:?}"
    );
    assert!(denied
        .unwrap_err()
        .to_string()
        .contains("bound to another Room"));

    // Stopping the rejected Room must not stop the admitted Room's controller.
    dispatch(&runtime, stop(&second_room))
        .await
        .expect("stop rejected Room");
    let same_room = dispatch(&runtime, start(&first_room))
        .await
        .expect("owner remains usable");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: same_room,
    } = same_room
    else {
        panic!("expected owner Environment projection");
    };
    assert_eq!(same_room.environment_id, initial.environment_id);
    assert_eq!(same_room.tabs, initial.tabs);

    dispatch(&runtime, stop(&first_room))
        .await
        .expect("owner stops");
    let denied_after_stop = dispatch(&runtime, start(&second_room)).await;
    assert!(
        denied_after_stop.is_err(),
        "stopping a controller does not erase its browser profile"
    );
    assert!(denied_after_stop
        .unwrap_err()
        .to_string()
        .contains("bound to another Room"));
    dispatch(&runtime, start(&first_room))
        .await
        .expect("same Room can restart");
    dispatch(&runtime, stop(&first_room))
        .await
        .expect("clean up owner controller");
}

fn start(room: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::StartRoomEnvironment(crate::local::StartRoomEnvironmentRequest {
        session_id: room.to_string(),
        viewport: crate::local::RoomEnvironmentViewportRequest {
            css_width: 1280,
            css_height: 800,
            device_scale_factor: 1,
            desktop_pixel_width: 1280,
            desktop_pixel_height: 800,
        },
    })
}

fn stop(room: &str) -> LocalDaemonRequest {
    LocalDaemonRequest::StopRoomEnvironment(crate::local::StopRoomEnvironmentRequest {
        session_id: room.to_string(),
    })
}

async fn dispatch(
    runtime: &SessionRuntime,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    runtime
        .dispatch_session_command(
            KernelCommand::from_local_request("room-isolation", None, None, &request),
            request,
        )
        .await
}
