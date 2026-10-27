use super::*;

async fn archive(
    router: &CommandRouter,
    room: &str,
    agent: &str,
    archived: bool,
) -> Result<LocalDaemonResponse, DaemonError> {
    let request = LocalDaemonRequest::ArchiveAgent(crate::local::ArchiveAgentRequest {
        session_id: room.into(),
        agent_id: agent.into(),
        archived,
    });
    let command = KernelCommand::from_local_request("agent-archive", None, None, &request);
    router.dispatch(command, request).await
}

#[tokio::test]
async fn reversible_agent_archive_stops_running_and_queued_work_only_for_target() {
    let worktree = crate::test_support::TestWorktree::new("agent-archive-running");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let other = spawn_test_agent(&mut app, session.id(), "other", "dev-stub");
    let first_run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "archive-test",
    );
    let other_run = launch_test_provider(
        &mut app,
        session.id(),
        other.id(),
        "dev-stub",
        "dev-stub",
        "archive-test",
    );
    // Admit through the same prompt owner used by runtime dispatch, leaving the
    // provider asynchronous so the archive request races a genuinely active lane.
    crate::test_support::admit_room_test_turn(&mut app, session.id(), agent.id());
    let active = app
        .sessions()
        .get_session(session.id())
        .unwrap()
        .active_prompt_for_agent(agent.id())
        .unwrap()
        .clone();
    app.submit_prompt(
        session.id(),
        active.source_attachment_id(),
        Some(agent.id()),
        "queued archive marker",
        vec![],
    )
    .unwrap();
    let attachments = app.attachments().clone();
    attachments
        .detach(&mut app.sessions_mut(), active.source_attachment_id())
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    archive(&router, session.id(), agent.id(), true)
        .await
        .expect("MP-08 / MP-09 / MP-10 / MP-11: archive must stop running target");
    let snapshot = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .unwrap();
    assert!(snapshot
        .agents()
        .iter()
        .find(|a| a.id() == agent.id())
        .unwrap()
        .archived());
    assert!(!snapshot
        .agents()
        .iter()
        .find(|a| a.id() == other.id())
        .unwrap()
        .archived());
    assert_ne!(snapshot.status(), crate::session::SessionStatus::Ended);
    assert!(snapshot.active_prompt_for_agent(agent.id()).is_none());
    assert!(snapshot
        .queued_prompts_for_agent(agent.id())
        .is_none_or(|queue| queue.is_empty()));
    assert_eq!(
        app.lock()
            .await
            .providers()
            .get_run(first_run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    router
        .runtime_state
        .owned
        .provider_run_projection
        .update(first_run.clone());
    assert_eq!(
        router
            .runtime_state
            .owned
            .provider_run_projection
            .get(first_run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Ended,
        "a late launch callback cannot revive an archived run projection"
    );
    assert_ne!(
        app.lock()
            .await
            .providers()
            .get_run(other_run.id())
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    archive(&router, session.id(), agent.id(), false)
        .await
        .unwrap();
    let restored = router
        .runtime_state
        .session_snapshot(session.id())
        .await
        .unwrap();
    assert!(!restored
        .agents()
        .iter()
        .find(|a| a.id() == agent.id())
        .unwrap()
        .archived());
}

#[tokio::test]
async fn reversible_agent_archive_has_an_explicit_list_and_keeps_snapshot() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    archive(&router, session.id(), agent.id(), true)
        .await
        .unwrap();
    for (kind, expected) in [("ListAgents", 0), ("ListArchivedAgents", 1)] {
        let request: LocalDaemonRequest =
            serde_json::from_value(serde_json::json!({kind:{"session_id":session.id()}}))
                .expect("shared archive discovery request");
        let command = KernelCommand::from_local_request("archive-list", None, None, &request);
        let LocalDaemonResponse::AgentsListed { agents } =
            router.dispatch(command, request).await.unwrap()
        else {
            panic!("agent list")
        };
        assert_eq!(agents.len(), expected);
    }
    assert_eq!(
        router
            .runtime_state
            .session_snapshot(session.id())
            .await
            .unwrap()
            .agents()[0]
            .id(),
        agent.id()
    );
}

#[tokio::test]
async fn reversible_agent_archive_is_owner_only_even_for_room_agents() {
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    for archived in [true, false] {
        let request = LocalDaemonRequest::ArchiveAgent(crate::local::ArchiveAgentRequest {
            session_id: session.id().into(),
            agent_id: agent.id().into(),
            archived,
        });
        assert!(router
            .runtime_state
            .authorize_room_agent_request(agent.id(), &request)
            .is_err());
        let command = remote_command_for_request(&request, Some("not-the-owner"));
        assert!(router.dispatch(command, request).await.is_err());
    }
}
