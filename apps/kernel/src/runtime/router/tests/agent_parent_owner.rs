use super::*;

// MP-08 / MP-11: a member moves only their own agents, and only under their own agents. A parent
// holds its children's decisions with its own authority, so a tree never spans two owners.
#[tokio::test]
async fn a_member_cannot_place_an_agent_under_another_users_agent() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, owner) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let foreign = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub").with_owner_user_id("foreign-owner"),
        )
        .unwrap();
    assert_ne!(owner.owner_user_id(), foreign.owner_user_id());
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "foreign-owner",
        None,
        crate::session::CollaborationLevel::Full,
    );
    app.sessions_mut().restore_session(room);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::SetAgentParent(crate::local::SetAgentParentRequest {
        session_id: session.id().into(),
        agent_id: foreign.id().into(),
        parent_agent_id: Some(owner.id().into()),
    });
    let mut command = KernelCommand::from_local_request("foreign-parent", None, None, &request);
    command.caller.user_id = Some("foreign-owner".into());

    let placed = router.dispatch(command, request).await;

    assert!(
        placed.is_err(),
        "the room owner's agent must not become the parent of another user's agent: {placed:?}"
    );
    let parent = app
        .lock()
        .await
        .agents()
        .get_agent(foreign.id())
        .unwrap()
        .parent_agent_id()
        .map(str::to_owned);
    assert_eq!(parent, None, "the foreign agent stays a room agent");
}
