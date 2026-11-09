use super::*;

// MP-08 / MP-11: an attachment is one member's client in the Room; another member, even a Private
// one, must not detach the owner's client.
#[tokio::test]
async fn a_private_member_cannot_detach_the_owners_attachment() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _owner) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let owner_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "owner-client",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    assert_ne!(owner_attachment.owner_user_id(), "private-member");
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    app.sessions_mut().restore_session(room);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::DetachFromSession(crate::local::DetachFromSessionRequest {
        attachment_id: owner_attachment.id().into(),
    });
    let mut command = KernelCommand::from_local_request("member-detach", None, None, &request);
    command.caller.user_id = Some("private-member".into());

    let detached = router.dispatch(command, request).await;

    assert!(
        matches!(detached, Err(DaemonError::OwnershipAccessDenied { .. })),
        "a Private member detached the room owner's client: {detached:?}"
    );
}
