use super::*;

// MP-08 / MP-11: ending, deleting, renaming or reconfiguring a Room acts on every member's agents,
// so it takes the room-level control rule (acts for the room owner, or Full collaboration); a
// Private member may not do it to the owner's Room.
fn room_with_private_member() -> (CommandRouter, String, String) {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _owner) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "client-private-member",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    assert_ne!(room.owner_user_id(), "private-member");
    app.sessions_mut().restore_session(room);
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(app, 1);
    (
        router,
        session.id().to_string(),
        attachment.id().to_string(),
    )
}

async fn dispatch_as_private_member(
    router: &CommandRouter,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut command =
        KernelCommand::from_local_request("member-room-control", None, None, &request);
    command.caller.user_id = Some("private-member".into());
    router.dispatch(command, request).await
}

#[tokio::test]
async fn a_private_member_cannot_end_the_owners_room() {
    let (router, session_id, _) = room_with_private_member();
    let ended = dispatch_as_private_member(
        &router,
        LocalDaemonRequest::EndSession(crate::local::EndSessionRequest { session_id }),
    )
    .await;

    assert!(
        matches!(ended, Err(DaemonError::SessionAccessDenied { .. })),
        "a Private member ended the room owner's Room: {ended:?}"
    );
}

#[tokio::test]
async fn a_private_member_cannot_delete_the_owners_room() {
    let (router, session_id, _) = room_with_private_member();
    let deleted = dispatch_as_private_member(
        &router,
        LocalDaemonRequest::DeleteSession(crate::local::DeleteSessionRequest {
            session_ref: session_id,
            workspace_id: None,
        }),
    )
    .await;

    assert!(
        matches!(deleted, Err(DaemonError::SessionAccessDenied { .. })),
        "a Private member deleted the room owner's Room: {deleted:?}"
    );
}

#[tokio::test]
async fn a_private_member_cannot_rename_the_owners_room() {
    let (router, session_id, _) = room_with_private_member();
    let aliased = dispatch_as_private_member(
        &router,
        LocalDaemonRequest::AliasSession(crate::local::AliasSessionRequest {
            session_id,
            alias: "member-name".into(),
        }),
    )
    .await;

    assert!(
        matches!(aliased, Err(DaemonError::SessionAccessDenied { .. })),
        "a Private member renamed the room owner's Room: {aliased:?}"
    );
}

#[tokio::test]
async fn a_private_member_cannot_reconfigure_the_owners_room() {
    let (router, session_id, attachment_id) = room_with_private_member();
    let updated = dispatch_as_private_member(
        &router,
        LocalDaemonRequest::UpdateSessionConfig(crate::local::UpdateSessionConfigRequest {
            session_id,
            attachment_id,
            values: BTreeMap::from([("theme".to_string(), "compact".to_string())]),
            requires_idle: false,
        }),
    )
    .await;

    assert!(
        matches!(updated, Err(DaemonError::SessionAccessDenied { .. })),
        "a Private member reconfigured the room owner's Room: {updated:?}"
    );
}
