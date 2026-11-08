use super::*;

// MP-08 / MP-11: a native approval asks the room in the agent's name, so only someone who may act
// for the agent's owner raises one; a Private member cannot put a prompt in another user's agent.
#[tokio::test]
async fn a_private_member_cannot_raise_a_native_approval_for_another_users_agent() {
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
    let request = LocalDaemonRequest::RequestNativeProviderTurnInteraction(
        crate::local::RequestNativeProviderTurnInteractionRequest::allow_deny(
            session.id(),
            owner.id(),
            "member-native-interaction",
            Some("Approve file changes?".to_string()),
            "Approve file changes?".to_string(),
            Some(1),
            crate::session::NativeInteractionOrigin::ProviderStartup {
                provider_run_id: "owner-run".into(),
            },
        ),
    );
    let mut command = KernelCommand::from_local_request("member-native", None, None, &request);
    command.caller.user_id = Some("private-member".into());

    let raised = timeout(Duration::from_secs(10), router.dispatch(command, request)).await;

    assert!(
        matches!(raised, Ok(Err(DaemonError::OwnershipAccessDenied { .. }))),
        "a Private member raised a native approval for the room owner's agent: {raised:?}"
    );
}
