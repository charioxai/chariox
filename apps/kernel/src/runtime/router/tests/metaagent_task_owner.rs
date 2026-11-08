use super::*;

// MP-08 / MP-11: a metaagent's task and plan steer its owner's work, so only someone who may
// act for that owner changes, pauses, resumes or aborts it; a Private member cannot.
#[tokio::test]
async fn a_private_member_cannot_change_another_users_metaagent_task() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub"))
        .unwrap();
    let metaagent = app
        .agents_mut()
        .activate_agent_meta_mode(metaagent.id(), None)
        .unwrap();
    let mut room = app.session_state_store().get_session(session.id()).unwrap();
    room.add_member(
        "private-member",
        None,
        crate::session::CollaborationLevel::Private,
    );
    app.sessions_mut().restore_session(room);
    assert_ne!(metaagent.owner_user_id(), "private-member");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let update = |label: &str, user: &str, task: &str| {
        let request =
            LocalDaemonRequest::UpdateMetaagentTask(crate::local::UpdateMetaagentTaskRequest {
                session_id: session.id().to_string(),
                metaagent_id: metaagent.id().to_string(),
                task_markdown: Some(task.to_string()),
                plan_markdown: None,
            });
        let mut command = KernelCommand::from_local_request(label, None, None, &request);
        command.caller.user_id = Some(user.to_string());
        (command, request)
    };

    let (command, request) = update("member-task", "private-member", "# Member task");
    let changed = router.dispatch(command, request).await;
    assert!(
        changed.is_err(),
        "a Private member changed the room owner's metaagent task: {changed:?}"
    );
    let (command, request) = update("owner-task", metaagent.owner_user_id(), "# Owner task");
    router
        .dispatch(command, request)
        .await
        .expect("the owner changes their own metaagent task");
}
