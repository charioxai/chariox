use super::*;

// MP-08 / MP-11: a deadline ends the agent's work when it is reached, so only someone who may
// act for the agent's owner arms one; a Private member cannot stop another user's agent.
#[tokio::test]
async fn a_private_member_cannot_arm_a_deadline_on_another_users_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, owner) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    app.sessions_mut().restore_session(room);
    assert_ne!(owner.owner_user_id(), "private-member");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::SetAgentDeadline(crate::local::SetAgentDeadlineRequest {
        session_id: session.id().into(),
        agent_id: owner.id().into(),
        deadline_ms: Some(crate::session::unix_epoch_ms() + 60_000),
    });
    let mut command = KernelCommand::from_local_request("member-deadline", None, None, &request);
    command.caller.user_id = Some("private-member".into());

    let armed = router.dispatch(command, request).await;

    assert!(
        armed.is_err(),
        "a Private member armed a deadline on the room owner's agent: {armed:?}"
    );
}
