use super::*;

#[tokio::test]
async fn runtime_agent_skill_grant_survives_kernel_restart() {
    let config = DaemonConfig::for_tests();
    let (session_id, agent_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let granted = router
            .runtime_state
            .grant_agent_skill(agent.id(), "review".to_string(), DEFAULT_LOCAL_USER_ID)
            .await
            .expect("skill grant should persist");
        assert!(granted.skill_grants().contains(&"review".to_string()));
        (session.id().to_string(), agent.id().to_string())
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored_agent = app
        .agents
        .get_agent(&agent_id)
        .expect("agent should restore");
    assert_eq!(restored_agent.session_id(), session_id);
    assert!(restored_agent
        .skill_grants()
        .contains(&"review".to_string()));
}

#[tokio::test]
async fn runtime_destroy_agent_survives_kernel_restart() {
    let config = DaemonConfig::for_tests();
    let (session_id, default_agent_id, destroyed_agent_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, default_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let spawned = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(session.id(), "codex").with_alias("reviewer"))
            .expect("agent should spawn");
        let updated = app
            .agents_mut()
            .set_agent_runtime_profile(
                spawned.id(),
                "codex",
                None,
                None,
                crate::provider::ProviderResumeState::from_codex_thread_id("thread-deleted"),
            )
            .expect("agent resume state should update");
        app.durable_state_store()
            .append_event(
                "agent.updated",
                Some(updated.id().to_string()),
                serde_json::json!({ "agent": &updated }),
            )
            .expect("agent update should persist");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        router
            .runtime_state
            .destroy_agent(spawned.id(), DEFAULT_LOCAL_USER_ID)
            .await
            .expect("agent destroy should persist");
        (
            session.id().to_string(),
            default_agent.id().to_string(),
            spawned.id().to_string(),
        )
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    assert!(app.agents.get_agent(&destroyed_agent_id).is_err());
    assert_eq!(
        app.agents
            .get_agent(&default_agent_id)
            .expect("remaining default agent should restore")
            .session_id(),
        session_id
    );
    assert!(app
        .agents
        .get_session_agents(&session_id)
        .iter()
        .all(|agent| agent.id() != destroyed_agent_id));
    let external_sessions = app.external_provider_session_index_store();
    external_sessions.upsert(crate::local::ExternalProviderSessionRecord {
        external_session_id: "codex:thread-deleted".to_string(),
        provider: "codex".to_string(),
        provider_session_id: "thread-deleted".to_string(),
        title: Some("thread-deleted".to_string()),
        title_source: Some("test".to_string()),
        first_prompt_preview: None,
        created_at_ms: None,
        last_modified_at_ms: 30,
        worktree_path: None,
        account_profile: None,
        capabilities: crate::local::ExternalProviderSessionCapabilities {
            ..crate::local::ExternalProviderSessionCapabilities::default()
        },
        attached_to_arroba: false,
        attached_session_ids: Vec::new(),
        attached_agent_ids: Vec::new(),
    });
    let page = external_sessions.list(&crate::local::ListExternalProviderSessionsRequest {
        provider: Some("codex".to_string()),
        cursor: None,
        limit: None,
    });
    assert_eq!(page.sessions.len(), 1);
    assert!(
        page.sessions[0].is_attachable_to_arroba(),
        "deleted agents must not restore stale provider-session attachments after restart"
    );
}

#[tokio::test]
async fn runtime_agent_capability_grants_accept_agent_id_or_public_ref() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should be created");
    let agent_id = agent.id().to_string();
    let agent_ref = agent.agent_ref().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);

    for (agent_ref, skill_name) in [(agent_id, "by-id"), (agent_ref, "by-ref")] {
        let agent = router
            .runtime_state
            .grant_agent_skill(&agent_ref, skill_name.to_string(), DEFAULT_LOCAL_USER_ID)
            .await
            .expect("grant should succeed");
        assert_eq!(agent.session_id(), session.id());
        assert!(agent.skill_grants().contains(&skill_name.to_string()));
    }
}

#[tokio::test]
async fn workflow_definition_survives_kernel_restart() {
    let config = DaemonConfig::for_tests();
    let (session_id, agent_id, workflow_id) = {
        let mut app = DaemonApp::bootstrap(config.clone()).expect("first daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        let (created, _) = router
            .runtime_state
            .execute_workflow_request(
                LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                    session_id: session.id().to_string(),
                    alias: Some("review".to_string()),
                }),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
            )
            .await;
        let workflow_id = match created.expect("workflow should create") {
            LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let (added, _) = router
            .runtime_state
            .execute_workflow_request(
                LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    agent_id: agent.id().to_string(),
                    expected_workflow_revision: None,
                }),
                DEFAULT_LOCAL_USER_ID.to_string(),
                None,
            )
            .await;
        added.expect("workflow node should add");
        (
            session.id().to_string(),
            agent.id().to_string(),
            workflow_id,
        )
    };

    let app = DaemonApp::bootstrap(config).expect("second daemon should boot");
    let restored_session = app
        .sessions()
        .get_session(&session_id)
        .expect("session should restore");
    let workflow = restored_session
        .workflows()
        .iter()
        .find(|workflow| workflow.id() == workflow_id)
        .expect("workflow should restore");
    assert_eq!(workflow.alias(), Some("review"));
    assert_eq!(workflow.nodes().len(), 1);
    assert_eq!(workflow.nodes()[0].agent_id(), agent_id);
}

#[tokio::test]
async fn runtime_end_and_delete_session_survive_kernel_restart() {
    let end_config = DaemonConfig::for_tests();
    let ended_session_id = {
        let mut app = DaemonApp::bootstrap(end_config.clone()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        router
            .runtime_state
            .end_session(session.id())
            .await
            .expect("session should end");
        session.id().to_string()
    };
    let app = DaemonApp::bootstrap(end_config).expect("daemon should reboot");
    let restored = app
        .sessions()
        .get_session(&ended_session_id)
        .expect("ended session should restore");
    assert_eq!(restored.status(), SessionStatus::Ended);
    assert!(app.agents.get_session_agents(&ended_session_id).is_empty());

    let delete_config = DaemonConfig::for_tests();
    let deleted_session_id = {
        let mut app = DaemonApp::bootstrap(delete_config.clone()).expect("daemon should boot");
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
        router
            .runtime_state
            .delete_session_ref(session.id(), None)
            .await
            .expect("session should delete");
        session.id().to_string()
    };
    let app = DaemonApp::bootstrap(delete_config).expect("daemon should reboot");
    assert!(app.sessions().get_session(&deleted_session_id).is_err());
    assert!(app
        .agents
        .get_session_agents(&deleted_session_id)
        .is_empty());
}
