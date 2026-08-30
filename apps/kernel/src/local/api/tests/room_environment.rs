use super::*;
use crate::session::CanonicalViewport;

#[test]
fn room_environment_start_rejects_invalid_initial_viewport_with_stable_code() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new(
                "workspace-environment-invalid-viewport",
                "worktree-environment-invalid-viewport",
            ),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 0,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect_err("an initial zero-width viewport must be rejected");
    match error {
        DaemonError::LocalTransport { operation, message } => {
            assert_eq!(operation, "environment.start");
            assert!(message.starts_with("environment_invalid_viewport:"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn room_environment_start_crosses_the_router_boundary_without_duplication() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-start", "worktree-environment-start"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let request = StartRoomEnvironmentRequest {
        session_id: session.id().to_string(),
        viewport: RoomEnvironmentViewportRequest {
            css_width: 1280,
            css_height: 800,
            device_scale_factor: 2,
            desktop_pixel_width: 2560,
            desktop_pixel_height: 1600,
        },
    };

    let first = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(request.clone()))
        .expect("Room Environment should start through the router");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = first else {
        panic!("unexpected local response: {first:?}");
    };
    assert_eq!(environment.session_id, session.id());
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
    assert_eq!(environment.runtime_generation, 1);
    assert_eq!(environment.viewport.css_width, 1280);

    let second = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(request))
        .expect("repeating start should be idempotent");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated,
    } = second
    else {
        panic!("unexpected local response: {second:?}");
    };
    assert_eq!(repeated.environment_id, environment.environment_id);
    assert_eq!(repeated.runtime_generation, environment.runtime_generation);
    assert_eq!(repeated.event_cursor, environment.event_cursor);

    let repeated_without_viewport = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 0,
                    css_height: 0,
                    device_scale_factor: 0,
                    desktop_pixel_width: 0,
                    desktop_pixel_height: 0,
                },
            },
        ))
        .expect("an existing Environment should keep its canonical viewport");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated_without_viewport,
    } = repeated_without_viewport
    else {
        panic!("unexpected local response: {repeated_without_viewport:?}");
    };
    assert_eq!(repeated_without_viewport, repeated);
}

#[test]
fn room_environment_stop_preserves_identity_and_is_idempotent() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-stop", "worktree-environment-stop"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: started,
    } = started
    else {
        panic!("unexpected local response: {started:?}");
    };

    let first = harness
        .dispatch(LocalDaemonRequest::StopRoomEnvironment(
            StopRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("Room Environment should stop");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = first else {
        panic!("unexpected local response: {first:?}");
    };
    assert_eq!(environment.environment_id, started.environment_id);
    assert_eq!(environment.runtime_generation, started.runtime_generation);
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Stopped
    );

    let second = harness
        .dispatch(LocalDaemonRequest::StopRoomEnvironment(
            StopRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("repeating stop should be idempotent");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: repeated,
    } = second
    else {
        panic!("unexpected local response: {second:?}");
    };
    assert_eq!(repeated, environment);

    let restarted = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("a stopped Room Environment should restart");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: restarted,
    } = restarted
    else {
        panic!("unexpected local response: {restarted:?}");
    };
    assert_eq!(restarted.environment_id, environment.environment_id);
    assert_eq!(
        restarted.runtime_generation,
        environment.runtime_generation + 1
    );
    assert_eq!(
        restarted.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
}

#[test]
fn room_environment_retry_invalidates_failed_runtime_without_replacing_environment() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment-retry", "worktree-environment-retry"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    let started = harness
        .dispatch(LocalDaemonRequest::StartRoomEnvironment(
            StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            },
        ))
        .expect("Room Environment should start");
    let LocalDaemonResponse::RoomEnvironmentUpdated {
        environment: started,
    } = started
    else {
        panic!("unexpected local response: {started:?}");
    };
    harness.with_app_mut(|app| {
        app.session_state_store()
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Failed)
            .expect("managed runtime failure should be recorded");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::RetryRoomEnvironment(
            RetryRoomEnvironmentRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("failed Room Environment should retry");
    let LocalDaemonResponse::RoomEnvironmentUpdated { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(environment.environment_id, started.environment_id);
    assert_eq!(
        environment.runtime_generation,
        started.runtime_generation + 1
    );
    assert_eq!(
        environment.lifecycle,
        crate::session::EnvironmentLifecycle::Starting
    );
}

#[test]
fn room_environment_state_crosses_the_router_boundary() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-environment", "worktree-environment"),
        ))
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };
    harness.with_app_mut(|app| {
        app.session_state_store()
            .create_room_environment(
                session.id(),
                "environment-1",
                CanonicalViewport::new(1280, 800, 1, 1280, 800).expect("viewport should be valid"),
            )
            .expect("Room should acquire an Environment");
    });

    let response = harness
        .dispatch(LocalDaemonRequest::GetRoomEnvironmentState(
            GetRoomEnvironmentStateRequest {
                session_id: session.id().to_string(),
            },
        ))
        .expect("Room Environment should be projected through the router");
    let LocalDaemonResponse::RoomEnvironmentState { environment } = response else {
        panic!("unexpected local response: {response:?}");
    };
    assert_eq!(environment.session_id, session.id());
    assert_eq!(environment.environment_id, "environment-1");
}

#[test]
fn room_environment_state_requires_room_membership() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-auth",
                "worktree-environment-auth",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::GetRoomEnvironmentState(GetRoomEnvironmentStateRequest {
                session_id: session.id().to_string(),
            }),
        )
        .expect_err("an outsider must not read the Room Environment");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));
}

#[test]
fn room_environment_lifecycle_requires_room_membership() {
    let harness = LocalRouterTestHarness::new();
    let session = match harness
        .dispatch_as_user(
            "owner-1",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-environment-start-auth",
                "worktree-environment-start-auth",
            )),
        )
        .expect("Room should be created")
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected local response: {other:?}"),
    };

    let error = harness
        .dispatch_as_user(
            "outsider-1",
            LocalDaemonRequest::StartRoomEnvironment(StartRoomEnvironmentRequest {
                session_id: session.id().to_string(),
                viewport: RoomEnvironmentViewportRequest {
                    css_width: 1280,
                    css_height: 800,
                    device_scale_factor: 1,
                    desktop_pixel_width: 1280,
                    desktop_pixel_height: 800,
                },
            }),
        )
        .expect_err("an outsider must not start the Room Environment");
    assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));

    for request in [
        LocalDaemonRequest::StopRoomEnvironment(StopRoomEnvironmentRequest {
            session_id: session.id().to_string(),
        }),
        LocalDaemonRequest::RetryRoomEnvironment(RetryRoomEnvironmentRequest {
            session_id: session.id().to_string(),
        }),
    ] {
        let error = harness
            .dispatch_as_user("outsider-1", request)
            .expect_err("an outsider must not control the Room Environment lifecycle");
        assert!(matches!(error, DaemonError::SessionAccessDenied { .. }));
    }
}
