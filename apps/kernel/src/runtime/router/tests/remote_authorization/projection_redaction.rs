use super::*;

#[tokio::test]
async fn remote_session_projection_redacts_other_users_private_agent_and_workflow_state() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-redaction",
            "worktree-redaction",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
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
    let local_agent = spawn_test_agent(&mut app, &session_id, "local-owned", "dev-stub");
    let extra_local_agent =
        spawn_test_agent(&mut app, &session_id, "local-owned-extra", "dev-stub");
    let user_two_agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("user-two-owned")
                .with_owner_user_id("user-2"),
        )
        .expect("user two agent should be created");
    let workflow = app
        .sessions_mut()
        .create_workflow(&session_id, Some("redaction-flow".to_string()))
        .expect("workflow should be created");
    let workflow_id = workflow.id().to_string();
    let local_node = app
        .sessions_mut()
        .add_workflow_node_owned(
            &session_id,
            &workflow_id,
            local_agent.id(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            "local public".to_string(),
        )
        .expect("local node should be created");
    app.sessions_mut()
        .update_workflow_node_instructions(
            &session_id,
            &workflow_id,
            local_node.id(),
            Some("local private prompt".to_string()),
        )
        .expect("local node instructions should update");
    let user_two_placed_local_node = app
        .sessions_mut()
        .add_workflow_node_owned(
            &session_id,
            &workflow_id,
            extra_local_agent.id(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            "user-2".to_string(),
            "user two placed local public".to_string(),
        )
        .expect("user two placed local node should be created");
    app.sessions_mut()
        .update_workflow_node_instructions(
            &session_id,
            &workflow_id,
            user_two_placed_local_node.id(),
            Some("user two graph prompt".to_string()),
        )
        .expect("user two placed node instructions should update");
    let user_two_node = app
        .sessions_mut()
        .add_workflow_node_owned(
            &session_id,
            &workflow_id,
            user_two_agent.id(),
            "user-2".to_string(),
            "user-2".to_string(),
            "user two public".to_string(),
        )
        .expect("user two node should be created");
    app.sessions_mut()
        .update_workflow_node_instructions(
            &session_id,
            &workflow_id,
            user_two_node.id(),
            Some("user two private prompt".to_string()),
        )
        .expect("user two node instructions should update");
    let provider_run = launch_test_provider(
        &mut app,
        &session_id,
        local_agent.id(),
        "dev-stub",
        "dev-stub",
        "redaction-model",
    );
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let state_request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let redacted_session = match router
        .dispatch(
            remote_command_for_request(&state_request, Some("user-2")),
            state_request,
        )
        .await
        .expect("member should read redacted session state")
    {
        LocalDaemonResponse::SessionState { session, .. } => session,
        other => panic!("unexpected session response: {other:?}"),
    };
    assert_eq!(redacted_session.agents().len(), 3);
    let redacted_local_agent = redacted_session
        .agents()
        .iter()
        .find(|agent| agent.id() == local_agent.id())
        .expect("other user's agent handle should remain workflow-selectable");
    assert_eq!(redacted_local_agent.provider(), "redacted");
    assert_eq!(redacted_local_agent.model(), None);
    assert_eq!(redacted_local_agent.visible_in_freeform(), false);
    assert!(redacted_session.agents().iter().any(|agent| {
        agent.id() == extra_local_agent.id()
            && agent.provider() == "redacted"
            && agent.model().is_none()
            && !agent.visible_in_freeform()
    }));
    let visible_user_two_agent = redacted_session
        .agents()
        .iter()
        .find(|agent| agent.id() == user_two_agent.id())
        .expect("own agent should remain visible");
    assert_eq!(visible_user_two_agent.visible_in_freeform(), true);
    let redacted_workflow = redacted_session
        .workflows()
        .iter()
        .find(|workflow| workflow.id() == workflow_id)
        .expect("workflow graph should remain visible");
    assert_eq!(redacted_workflow.nodes().len(), 3);
    let redacted_local_node = redacted_workflow
        .node(local_node.id())
        .expect("other user's node should remain visible");
    assert_eq!(redacted_local_node.public_label(), "local public");
    assert_eq!(redacted_local_node.instructions(), None);
    let visible_user_two_placed_local_node = redacted_workflow
        .node(user_two_placed_local_node.id())
        .expect("node inserted by user two should remain visible");
    assert_eq!(
        visible_user_two_placed_local_node.instructions(),
        Some("user two graph prompt")
    );
    let visible_user_two_node = redacted_workflow
        .node(user_two_node.id())
        .expect("own node should remain visible");
    assert_eq!(
        visible_user_two_node.instructions(),
        Some("user two private prompt")
    );

    let list_agents = LocalDaemonRequest::ListAgents(ListAgentsRequest {
        session_id: session_id.clone(),
    });
    match router
        .dispatch(
            remote_command_for_request(&list_agents, Some("user-2")),
            list_agents,
        )
        .await
        .expect("member should list workflow-selectable agent handles")
    {
        LocalDaemonResponse::AgentsListed { agents } => {
            assert_eq!(agents.len(), 3);
            let listed_local_agent = agents
                .iter()
                .find(|agent| agent.id() == local_agent.id())
                .expect("other user's redacted agent handle should be listed");
            assert_eq!(listed_local_agent.provider(), "redacted");
            assert_eq!(listed_local_agent.model(), None);
            assert_eq!(listed_local_agent.visible_in_freeform(), false);
            assert!(agents.iter().any(|agent| {
                agent.id() == extra_local_agent.id()
                    && agent.provider() == "redacted"
                    && agent.model().is_none()
                    && !agent.visible_in_freeform()
            }));
            assert!(agents
                .iter()
                .any(|agent| { agent.id() == user_two_agent.id() && agent.visible_in_freeform() }));
        }
        other => panic!("unexpected agents response: {other:?}"),
    }

    let provider_request = LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
        provider_run_id: provider_run.id().to_string(),
    });
    assert_ownership_denied(
        router
            .dispatch(
                remote_command_for_request(&provider_request, Some("user-2")),
                provider_request,
            )
            .await
            .expect_err("other user should not read provider run"),
        "user-2",
        DEFAULT_LOCAL_USER_ID,
    );
}
