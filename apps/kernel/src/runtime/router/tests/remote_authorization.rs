use super::*;
use crate::local::UpdateWorkflowNodeInstructionsRequest;

#[tokio::test]
async fn remote_session_requests_require_membership() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-membership",
            "worktree-a",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);

    let request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
        session_id: session_id.clone(),
    });
    let denied = router
        .dispatch(
            remote_command_for_request(&request, Some("user-2")),
            request,
        )
        .await
        .expect_err("non-member should be rejected");
    assert!(matches!(
        denied,
        DaemonError::SessionAccessDenied {
            session_id: denied_session,
            user_id
        } if denied_session == session_id && user_id == "user-2"
    ));

    let request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest { session_id });
    let missing = router
        .dispatch(remote_command_for_request(&request, None), request)
        .await
        .expect_err("remote session request without user id should be rejected");
    assert!(matches!(
        missing,
        DaemonError::MissingSessionCallerIdentity { .. }
    ));
}

#[tokio::test]
async fn remote_session_list_is_filtered_to_memberships() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session_a = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-membership",
            "worktree-a",
        ))
        .expect("session a should be created");
    let session_b = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-membership",
            "worktree-b",
        ))
        .expect("session b should be created");
    let session_a_id = session_a.id().to_string();
    let session_b_id = session_b.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_b_id,
            "invite-user-2".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_b_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session b");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 1);
    let request = LocalDaemonRequest::ListSessions(ListSessionsRequest);
    let response = router
        .dispatch(
            remote_command_for_request(&request, Some("user-2")),
            request,
        )
        .await
        .expect("member list should succeed");
    match response {
        LocalDaemonResponse::SessionsListed { sessions } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].id(), session_b_id);
            assert_ne!(sessions[0].id(), session_a_id);
        }
        _ => panic!("unexpected list response"),
    }
}

#[tokio::test]
async fn remote_owned_session_objects_record_caller_user() {
    let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-ownership",
            "worktree-ownership",
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

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let spawn_one = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        session_id: session_id.clone(),
        alias: Some("owned-a".to_string()),
        provider: Some("dev-stub".to_string()),
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
    });
    let agent_one = match router
        .dispatch(
            remote_command_for_request(&spawn_one, Some("user-2")),
            spawn_one,
        )
        .await
        .expect("agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        other => panic!("unexpected spawn response: {other:?}"),
    };
    assert_eq!(agent_one.owner_user_id(), "user-2");

    let spawn_two = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        session_id: session_id.clone(),
        alias: Some("owned-b".to_string()),
        provider: Some("dev-stub".to_string()),
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
    });
    let agent_two = match router
        .dispatch(
            remote_command_for_request(&spawn_two, Some("user-2")),
            spawn_two,
        )
        .await
        .expect("second agent should spawn")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        other => panic!("unexpected spawn response: {other:?}"),
    };
    assert_eq!(agent_two.owner_user_id(), "user-2");

    let create_workflow = LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
        session_id: session_id.clone(),
        alias: Some("owned-flow".to_string()),
    });
    let workflow_id = match router
        .dispatch(
            remote_command_for_request(&create_workflow, Some("user-2")),
            create_workflow,
        )
        .await
        .expect("workflow should create")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
        other => panic!("unexpected workflow response: {other:?}"),
    };

    let add_first_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow_id.clone(),
        agent_id: agent_one.id().to_string(),
        expected_workflow_revision: None,
    });
    let first_node = match router
        .dispatch(
            remote_command_for_request(&add_first_node, Some("user-2")),
            add_first_node,
        )
        .await
        .expect("first node should add")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected node response: {other:?}"),
    };
    assert_eq!(first_node.owner_user_id(), "user-2");
    assert_eq!(first_node.public_label(), agent_one.id());

    let add_second_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow_id.clone(),
        agent_id: agent_two.id().to_string(),
        expected_workflow_revision: None,
    });
    let second_node = match router
        .dispatch(
            remote_command_for_request(&add_second_node, Some("user-2")),
            add_second_node,
        )
        .await
        .expect("second node should add")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected node response: {other:?}"),
    };
    assert_eq!(second_node.owner_user_id(), "user-2");

    let create_endpoint =
        LocalDaemonRequest::CreateWorkflowEndpoint(CreateWorkflowEndpointRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            entry_node_id: first_node.id().to_string(),
            alias: Some("owned-entry".to_string()),
            expected_workflow_revision: None,
        });
    let endpoint = match router
        .dispatch(
            remote_command_for_request(&create_endpoint, Some("user-2")),
            create_endpoint,
        )
        .await
        .expect("endpoint should create")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        other => panic!("unexpected endpoint response: {other:?}"),
    };
    assert_eq!(endpoint.owner_user_id(), "user-2");

    let add_edge = LocalDaemonRequest::AddWorkflowEdge(AddWorkflowEdgeRequest {
        session_id,
        workflow_ref: workflow_id,
        from_node_id: first_node.id().to_string(),
        to_node_id: second_node.id().to_string(),
        handoff_schema_ref: None,
        validation_policy: None,
        expected_workflow_revision: None,
    });
    let edge = match router
        .dispatch(
            remote_command_for_request(&add_edge, Some("user-2")),
            add_edge,
        )
        .await
        .expect("edge should add")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
        other => panic!("unexpected edge response: {other:?}"),
    };
    assert_eq!(edge.created_by_user_id(), "user-2");
}

#[tokio::test]
async fn remote_created_session_records_caller_as_owner_and_default_agent_owner() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let create_session = LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
        "workspace-remote-session-owner",
        "worktree-remote-session-owner",
    ));

    let (session, agent) = match router
        .dispatch(
            remote_command_for_request(&create_session, Some("user-2")),
            create_session,
        )
        .await
        .expect("remote create session should succeed")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        other => panic!("unexpected create session response: {other:?}"),
    };

    assert_eq!(session.owner_user_id(), "user-2");
    assert!(session.has_member("user-2"));
    assert!(!session.has_member(DEFAULT_LOCAL_USER_ID));
    assert_eq!(agent.owner_user_id(), "user-2");
}

#[tokio::test]
async fn remote_user_cannot_control_other_users_agents_or_endpoint() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-authz",
            "worktree-authz",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let local_agent = spawn_test_agent(&mut app, &session_id, "local-owned", "dev-stub");
    let local_agent_id = local_agent.id().to_string();
    let extra_local_agent =
        spawn_test_agent(&mut app, &session_id, "local-owned-extra", "dev-stub");
    let extra_local_agent_id = extra_local_agent.id().to_string();
    let workflow = app
        .sessions_mut()
        .create_workflow(&session_id, Some("authz-flow".to_string()))
        .expect("workflow should be created");
    let workflow_id = workflow.id().to_string();
    let local_node = app
        .sessions_mut()
        .add_workflow_node_owned(
            &session_id,
            &workflow_id,
            &local_agent_id,
            DEFAULT_LOCAL_USER_ID.to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            local_agent_id.clone(),
        )
        .expect("node should be created");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            &session_id,
            &workflow_id,
            local_node.id(),
            Some("local-entry".to_string()),
        )
        .expect("endpoint should be created");
    app.sessions_mut()
        .set_workflow_endpoint_owner(
            &session_id,
            &workflow_id,
            endpoint.id(),
            DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("endpoint owner should be set");
    for (invite_id, user_id) in [("invite-user-2", "user-2"), ("invite-user-3", "user-3")] {
        let (_, invite) = app
            .sessions_mut()
            .create_session_invite(
                &session_id,
                invite_id.to_string(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
                Some(1),
                crate::session::CollaborationLevel::Private,
            )
            .expect("invite should be created");
        app.sessions_mut()
            .join_session_invite(&session_id, invite.invite_id(), user_id.to_string(), 1)
            .expect("user should join session");
    }

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let focus = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
        session_id: session_id.clone(),
        agent_id: local_agent_id.clone(),
    });
    assert_ownership_denied(
        router
            .dispatch(remote_command_for_request(&focus, Some("user-2")), focus)
            .await
            .expect_err("other user should not focus local agent"),
        "user-2",
        DEFAULT_LOCAL_USER_ID,
    );

    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session_id.clone(),
        attachment_id: "remote-attachment".to_string(),
        target_agent_id: Some(local_agent_id.clone()),
        prompt: "should be denied".to_string(),
        attachments: Vec::new(),
    });
    assert_ownership_denied(
        router
            .dispatch(remote_command_for_request(&submit, Some("user-2")), submit)
            .await
            .expect_err("other user should not submit to local agent"),
        "user-2",
        DEFAULT_LOCAL_USER_ID,
    );

    let add_local_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow_id.clone(),
        agent_id: extra_local_agent_id.clone(),
        expected_workflow_revision: None,
    });
    let collaborator_placed_local_node = match router
        .dispatch(
            remote_command_for_request(&add_local_node, Some("user-2")),
            add_local_node,
        )
        .await
        .expect("collaborator should be able to add another user's agent as a workflow node")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected add node response: {other:?}"),
    };
    assert_eq!(
        collaborator_placed_local_node.owner_user_id(),
        DEFAULT_LOCAL_USER_ID
    );
    assert_eq!(
        collaborator_placed_local_node.created_by_user_id(),
        "user-2"
    );

    let update_collaborator_node =
        LocalDaemonRequest::UpdateWorkflowNodeInstructions(UpdateWorkflowNodeInstructionsRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            node_id: collaborator_placed_local_node.id().to_string(),
            instructions: Some("user-2 graph instructions".to_string()),
            expected_workflow_revision: None,
        });
    router
        .dispatch(
            remote_command_for_request(&update_collaborator_node, Some("user-2")),
            update_collaborator_node,
        )
        .await
        .expect("collaborator should edit the node they inserted");

    let create_collaborator_endpoint =
        LocalDaemonRequest::CreateWorkflowEndpoint(CreateWorkflowEndpointRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            entry_node_id: collaborator_placed_local_node.id().to_string(),
            alias: Some("collaborator-entry".to_string()),
            expected_workflow_revision: None,
        });
    router
        .dispatch(
            remote_command_for_request(&create_collaborator_endpoint, Some("user-2")),
            create_collaborator_endpoint,
        )
        .await
        .expect("collaborator should create an endpoint for the node they inserted");

    let invoke = LocalDaemonRequest::InvokeWorkflowEndpoint(InvokeWorkflowEndpointRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow_id.clone(),
        endpoint_ref: endpoint.id().to_string(),
        prompt: Some("should be denied".to_string()),
        queue_ref: None,
    });
    assert_ownership_denied(
        router
            .dispatch(remote_command_for_request(&invoke, Some("user-2")), invoke)
            .await
            .expect_err("other user should not invoke local endpoint"),
        "user-2",
        DEFAULT_LOCAL_USER_ID,
    );

    let spawn_user_two = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
        session_id: session_id.clone(),
        alias: Some("user-two-owned".to_string()),
        provider: Some("dev-stub".to_string()),
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
    });
    let user_two_agent = match router
        .dispatch(
            remote_command_for_request(&spawn_user_two, Some("user-2")),
            spawn_user_two,
        )
        .await
        .expect("user two should spawn own agent")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        other => panic!("unexpected spawn response: {other:?}"),
    };
    let add_user_two_node = LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow_id.clone(),
        agent_id: user_two_agent.id().to_string(),
        expected_workflow_revision: None,
    });
    let user_two_node = match router
        .dispatch(
            remote_command_for_request(&add_user_two_node, Some("user-2")),
            add_user_two_node,
        )
        .await
        .expect("user two should add own node")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected node response: {other:?}"),
    };

    let add_cross_owner_edge = LocalDaemonRequest::AddWorkflowEdge(AddWorkflowEdgeRequest {
        session_id: session_id.clone(),
        workflow_ref: workflow_id.clone(),
        from_node_id: collaborator_placed_local_node.id().to_string(),
        to_node_id: user_two_node.id().to_string(),
        handoff_schema_ref: None,
        validation_policy: None,
        expected_workflow_revision: None,
    });
    let edge = match router
        .dispatch(
            remote_command_for_request(&add_cross_owner_edge, Some("user-2")),
            add_cross_owner_edge,
        )
        .await
        .expect("edge touching caller node should be allowed")
    {
        LocalDaemonResponse::WorkflowEdgeAdded { edge, .. } => edge,
        other => panic!("unexpected edge response: {other:?}"),
    };
    assert_eq!(edge.created_by_user_id(), "user-2");

    let remove_edge = LocalDaemonRequest::RemoveWorkflowEdge(RemoveWorkflowEdgeRequest {
        session_id,
        workflow_ref: workflow_id,
        edge_id: edge.id().to_string(),
        expected_workflow_revision: None,
    });
    assert_ownership_denied(
        router
            .dispatch(
                remote_command_for_request(&remove_edge, Some("user-3")),
                remove_edge,
            )
            .await
            .expect_err("unrelated user should not remove edge"),
        "user-3",
        "user-2",
    );
}

#[tokio::test]
async fn full_collaboration_invite_allows_prompting_other_users_agents() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-full-collab",
            "worktree-full-collab",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let local_agent = spawn_test_agent(&mut app, &session_id, "local-owned", "dev-stub");
    let local_agent_id = local_agent.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "invite-user-2-full".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Full,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("user should join session");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id,
        attachment_id: "remote-attachment".to_string(),
        target_agent_id: Some(local_agent_id),
        prompt: "allowed by full collaboration".to_string(),
        attachments: Vec::new(),
    });
    let error = router
        .dispatch(remote_command_for_request(&submit, Some("user-2")), submit)
        .await
        .expect_err("prompt should reach runtime and fail only because no provider run exists");
    assert!(
        !matches!(error, DaemonError::OwnershipAccessDenied { .. }),
        "full collaboration should pass prompt ownership checks"
    );
}

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
            assert!(
                agents.iter().any(|agent| {
                    agent.id() == user_two_agent.id() && agent.visible_in_freeform()
                })
            );
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
