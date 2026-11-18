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

    let outline = dispatch_as_private_member(
        &router,
        LocalDaemonRequest::GetSessionHistoryOutline(
            crate::local::GetSessionHistoryOutlineRequest {
                session_id: session_id.clone(),
                agent_ids: Some(vec![owner_agent_id.clone()]),
                latest_prompt_count: None,
                cursor: None,
                include_prefetched_content: true,
            },
        ),
    )
    .await;
    // A room view asks for every listed agent, so the outline keeps only what the member may see.
    assert!(
        matches!(&outline, Ok(LocalDaemonResponse::SessionHistoryOutline { agents })
            if agents.iter().all(|agent| agent.agent_id != owner_agent_id)),
        "a Private member read the room owner's agent history outline: {outline:?}"
    );

    for request in [
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

    let request = LocalDaemonRequest::GetSessionHistoryOutline(
        crate::local::GetSessionHistoryOutlineRequest {
            session_id,
            agent_ids: Some(vec![owner_agent_id.clone()]),
            latest_prompt_count: None,
            cursor: None,
            include_prefetched_content: false,
        },
    );
    let command = KernelCommand::from_local_request("owner-history", None, None, &request);
    let owner_outline = router.dispatch(command, request).await;
    assert!(
        matches!(&owner_outline, Ok(LocalDaemonResponse::SessionHistoryOutline { agents })
            if agents.iter().any(|agent| agent.agent_id == owner_agent_id)),
        "the room owner must still read their agent's history outline: {owner_outline:?}"
    );
}

// A sub-agent's pane reads its history with the Room's outline: the owner reads every
// agent of their tree, at any depth, though the Room's agent list leaves sub-agents out.
#[tokio::test]
async fn the_owner_reads_the_history_outline_of_sub_agents_at_any_depth() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, top) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let child = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub").with_parent_agent_id(top.id()),
        )
        .unwrap();
    let grandchild = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub").with_parent_agent_id(child.id()),
        )
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let asked = vec![child.id().to_string(), grandchild.id().to_string()];
    let request = LocalDaemonRequest::GetSessionHistoryOutline(
        crate::local::GetSessionHistoryOutlineRequest {
            session_id: session.id().to_string(),
            agent_ids: Some(asked.clone()),
            latest_prompt_count: Some(2),
            cursor: None,
            include_prefetched_content: true,
        },
    );
    let command = KernelCommand::from_local_request("sub-agent-history", None, None, &request);
    let outline = router.dispatch(command, request).await;
    let Ok(LocalDaemonResponse::SessionHistoryOutline { agents }) = &outline else {
        panic!("the owner's outline of their sub-agents failed: {outline:?}");
    };
    let mut read = agents
        .iter()
        .map(|agent| agent.agent_id.clone())
        .collect::<Vec<_>>();
    read.sort();
    let mut expected = asked;
    expected.sort();
    assert_eq!(read, expected, "every sub-agent the pane asked for is read");
}
