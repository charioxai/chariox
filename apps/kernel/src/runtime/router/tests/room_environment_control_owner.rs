use super::*;

// MP-08 / MP-11: starting, stopping or retrying the Room's environment controls the Room, like
// its processes: the owner or a member who may prompt agents directly, never a Private member.
#[tokio::test]
async fn a_private_member_cannot_stop_or_retry_the_room_environment() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    app.sessions_mut().restore_session(room);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let session_id = session.id();
    let controls: [LocalDaemonRequest; 2] = [
        serde_json::from_value(
            serde_json::json!({ "StopRoomEnvironment": { "session_id": session_id } }),
        )
        .unwrap(),
        serde_json::from_value(
            serde_json::json!({ "RetryRoomEnvironment": { "session_id": session_id } }),
        )
        .unwrap(),
    ];
    for request in controls {
        let mut command = KernelCommand::from_local_request("member-env", None, None, &request);
        command.caller.user_id = Some("private-member".into());
        let controlled = router.dispatch(command, request.clone()).await;
        assert!(
            matches!(controlled, Err(DaemonError::SessionAccessDenied { .. })),
            "a Private member controlled the Room environment with {request:?}: {controlled:?}"
        );
    }
}
