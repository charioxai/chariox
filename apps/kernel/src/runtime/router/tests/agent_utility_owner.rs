use super::*;

// MP-08 / MP-11: a hidden utility runs a turn on the agent's provider account, so only someone who
// may act for the agent's owner runs one; a Private member cannot use another user's agent.
#[tokio::test]
async fn a_private_member_cannot_run_a_utility_on_another_users_agent() {
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
    let request = LocalDaemonRequest::RunAgentUtility(crate::local::RunAgentUtilityRequest {
        session_id: session.id().into(),
        agent_id: owner.id().into(),
        kind: crate::local::AgentUtilityKind::SemanticRecallSearch,
        input: crate::local::AgentUtilityInput::SemanticRecallSearch(
            crate::local::SemanticRecallSearchUtilityInput {
                query: "owner's notes".into(),
                session_id: None,
                agent_id: None,
                provider: None,
                model: None,
                workflow_id: None,
                machine_id: None,
                repo_root: None,
                worktree_path: None,
                kind: None,
                limit: None,
            },
        ),
    });
    let mut command = KernelCommand::from_local_request("member-utility", None, None, &request);
    command.caller.user_id = Some("private-member".into());

    let ran = timeout(Duration::from_secs(10), router.dispatch(command, request)).await;

    assert!(
        matches!(ran, Ok(Err(DaemonError::OwnershipAccessDenied { .. }))),
        "a Private member's utility on the room owner's agent was not refused as not theirs: {ran:?}"
    );
}
