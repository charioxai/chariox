use super::*;

// MP-08 / MP-11: the room hides another user's agent trace from a Private member (session
// projection, terminal output); its history outline and blob content must stay hidden as well.
fn room_with_private_member() -> (CommandRouter, String, String) {
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
    assert_ne!(owner.owner_user_id(), "private-member");
    app.sessions_mut().restore_session(room);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    (router, session.id().to_string(), owner.id().to_string())
}

async fn dispatch_as_private_member(
    router: &CommandRouter,
    request: LocalDaemonRequest,
) -> Result<LocalDaemonResponse, DaemonError> {
    let mut command = KernelCommand::from_local_request("member-history", None, None, &request);
    command.caller.user_id = Some("private-member".into());
    router.dispatch(command, request).await
}

#[tokio::test]
async fn a_private_member_cannot_read_another_users_agent_history() {
    let (router, session_id, owner_agent_id) = room_with_private_member();

    for request in [
        LocalDaemonRequest::GetSessionHistoryOutline(crate::local::GetSessionHistoryOutlineRequest {
            session_id: session_id.clone(),
            agent_ids: Some(vec![owner_agent_id.clone()]),
            latest_prompt_count: None,
            cursor: None,
            include_prefetched_content: true,
        }),
        LocalDaemonRequest::GetSessionHistoryBlobContent(
            crate::local::GetSessionHistoryBlobContentRequest {
                session_id: session_id.clone(),
                agent_id: owner_agent_id.clone(),
                blob_id: "blob".into(),
            },
        ),
        LocalDaemonRequest::GetSessionHistoryBlobContents(
            crate::local::GetSessionHistoryBlobContentsRequest {
                session_id: session_id.clone(),
                agent_id: owner_agent_id.clone(),
                blob_ids: Vec::new(),
            },
        ),
    ] {
        let read = dispatch_as_private_member(&router, request.clone()).await;
        assert!(
            matches!(read, Err(DaemonError::OwnershipAccessDenied { .. })),
            "a Private member read the room owner's agent history with {request:?}: {read:?}"
        );
    }
}
