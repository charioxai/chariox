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

// b3, 2026-11-18: each history outline built two full session snapshots (~800 ms with the
// room's tasks, wakes and queued messages) on the kernel's async workers; 59 of them at once
// starved the runtime and the relay dropped the kernel. History reads need only the room and
// its agents.
#[tokio::test]
async fn history_reads_build_no_full_session_snapshot() {
    let (router, session_id, owner_agent_id) = room_with_private_member();
    let requests = [
        LocalDaemonRequest::GetSessionHistoryOutline(
            crate::local::GetSessionHistoryOutlineRequest {
                session_id: session_id.clone(),
                agent_ids: None,
                latest_prompt_count: Some(4),
                cursor: None,
                include_prefetched_content: false,
            },
        ),
        LocalDaemonRequest::PeekAgentTranscript(crate::local::PeekAgentTranscriptRequest {
            session_id: session_id.clone(),
            agent_id: owner_agent_id.clone(),
            limit: None,
            before: None,
        }),
    ];
    let _ = crate::runtime::state::KernelRuntimeState::take_session_snapshot_builds();
    for request in requests {
        let request = crate::runtime::history_executor::authorize_history_request(
            &router.runtime_state,
            crate::session::DEFAULT_LOCAL_USER_ID,
            request,
        )
        .await
        .expect("the owner may read the room's history");
        crate::runtime::history_executor::execute_history_request(
            router.history_store.clone(),
            router.operational_history_store.clone(),
            &router.runtime_state,
            &router.config_projection,
            request,
        )
        .await
        .expect("history read");
    }
    assert_eq!(
        crate::runtime::state::KernelRuntimeState::take_session_snapshot_builds(),
        0
    );
}

// b3, 2026-11-18: a reconnecting web view asked for the same outline once per pane and per
// attachment, 59 times in two minutes. Identical outline reads in flight read the history once.
#[tokio::test]
async fn identical_outline_reads_in_flight_read_the_history_once() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 16);
    let dispatch = |n: usize| {
        let request = LocalDaemonRequest::GetSessionHistoryOutline(
            crate::local::GetSessionHistoryOutlineRequest {
                session_id: session.id().to_string(),
                agent_ids: None,
                latest_prompt_count: Some(2),
                cursor: None,
                include_prefetched_content: true,
            },
        );
        let command =
            KernelCommand::from_local_request(format!("outline-{n}"), None, None, &request);
        router.dispatch(command, request)
    };
    dispatch(0).await.expect("first outline");
    let served = || router.operational_history_store.reads_served();
    let before = served();
    dispatch(1).await.expect("one outline");
    let one = served() - before;
    assert!(one > 0, "an outline reads the history");
    let before = served();
    for outline in futures_util::future::join_all((2..8).map(dispatch)).await {
        outline.expect("concurrent outline");
    }
    assert_eq!(
        served() - before,
        one,
        "six identical outline reads in flight each read the history"
    );
}

// Review of #1445: a wait row is rewritten in place when the wait ends, which appends nothing. An
// outline asked for after that rewrite must not join a read that started before it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_outline_asked_for_after_a_row_is_rewritten_does_not_join_an_older_read() {
    use crate::history::{HistoryEventTurnContext, SessionHistoryEntry};
    let mut config = DaemonConfig::for_tests();
    config.operational_history_read_delay_ms = 300;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 16);
    let (session_id, agent_id) = (session.id().to_string(), agent.id().to_string());
    let context = HistoryEventTurnContext {
        session_id: Some(session_id.clone()),
        agent_id: Some(agent_id.clone()),
        turn_id: Some("prompt-1".into()),
        prompt_id: Some("prompt-1".into()),
        provider_run_id: Some("run-1".into()),
        ..HistoryEventTurnContext::default()
    };
    let reply = |text: &str| {
        SessionHistoryEntry::provider_output(
            &session_id,
            "run-1",
            Some(&agent_id),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("wait-1".to_string()),
            text,
        )
    };
    let store = router.operational_history_store.clone();
    let prompt = SessionHistoryEntry::user_prompt(&session_id, "attachment-1", &agent_id, "wait");
    store
        .append_transcript(&prompt, context.clone())
        .expect("append the prompt");
    store
        .append_transcript(&reply("wait-row-running"), context.clone())
        .expect("append the wait row");
    let dispatch = |n: usize| {
        let request = LocalDaemonRequest::GetSessionHistoryOutline(
            crate::local::GetSessionHistoryOutlineRequest {
                session_id: session_id.clone(),
                agent_ids: None,
                latest_prompt_count: Some(2),
                cursor: None,
                include_prefetched_content: true,
            },
        );
        let command =
            KernelCommand::from_local_request(format!("outline-{n}"), None, None, &request);
        router.dispatch(command, request)
    };
    let served = || store.reads_served();
    let before = served();
    dispatch(0).await.expect("one outline");
    let one = served() - before;
    assert!(one > 0, "an outline reads the history");

    let before = served();
    let (older, newer) = tokio::join!(dispatch(1), async {
        // The older read is in flight once its first history read began; each waits 300 ms.
        while served() == before {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        store
            .replace_latest_transcript_by_merge_key(
                &session_id,
                Some(&agent_id),
                "wait-1",
                &reply("wait-row-completed"),
                context.clone(),
            )
            .expect("rewrite the wait row")
            .expect("the wait row exists");
        dispatch(2).await
    });
    older.expect("older outline");
    let newer = serde_json::to_string(&newer.expect("newer outline")).unwrap();
    assert_eq!(
        served() - before,
        2 * one,
        "the outline asked for after the rewrite joined the read that started before it"
    );
    assert!(newer.contains("wait-row-completed"), "{newer}");
    assert!(!newer.contains("wait-row-running"), "{newer}");
}
