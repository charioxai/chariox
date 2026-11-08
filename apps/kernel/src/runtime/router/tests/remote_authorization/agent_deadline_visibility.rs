use super::*;

#[test]
fn a_private_member_cannot_read_another_users_agent_deadline() {
    run_remote_authorization_large_stack_test(
        "a-private-member-cannot-read-another-users-agent-deadline",
        a_private_member_cannot_read_another_users_agent_deadline_inner,
    );
}

async fn a_private_member_cannot_read_another_users_agent_deadline_inner() {
    let worktree = crate::test_support::TestWorktree::new("deadline-read-visibility");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(worktree.session_request())
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent_id = spawn_test_agent(&mut app, &session_id, "owned", "dev-stub")
        .id()
        .to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "invite-user-2".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session");
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let deadline_ms = 4_102_444_800_000_u64;
    let set = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        "SetAgentDeadline": {"session_id": session_id, "agent_id": agent_id, "deadline_ms": deadline_ms}
    }))
    .unwrap();
    router
        .dispatch(
            remote_command_for_request(&set, Some(DEFAULT_LOCAL_USER_ID)),
            set,
        )
        .await
        .expect("the owner arms the deadline");
    let get = serde_json::from_value::<LocalDaemonRequest>(serde_json::json!({
        "GetAgentDeadline": {"session_id": session_id, "agent_id": agent_id}
    }))
    .unwrap();

    let owner_read = router
        .dispatch(
            remote_command_for_request(&get, Some(DEFAULT_LOCAL_USER_ID)),
            get.clone(),
        )
        .await
        .expect("the owner reads the deadline");
    assert_eq!(
        serde_json::to_value(owner_read).unwrap()["AgentDeadline"]["deadline"]["deadline_ms"],
        deadline_ms
    );
    assert_ownership_denied(
        router
            .dispatch(remote_command_for_request(&get, Some("user-2")), get)
            .await
            .expect_err("a Private member must not read another user's agent deadline"),
        "user-2",
        DEFAULT_LOCAL_USER_ID,
    );
}
