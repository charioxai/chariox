use super::*;
use crate::session::CanonicalViewport;

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
