use super::*;

#[test]
fn metaagent_runtime_mcp_returns_session_overview_and_command_docs() {
    run_large_stack_async_test(
        "metaagent-runtime-mcp-returns-session-overview-and-command-docs",
        assert_metaagent_runtime_mcp_returns_session_overview_and_command_docs,
    );
}

async fn assert_metaagent_runtime_mcp_returns_session_overview_and_command_docs() {
    let env = TestMetaRuntimeEnv::new("overview");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker"))
        .expect("worker should spawn");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2"),
        )
        .expect("peer worker should spawn");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let owned_interaction = RuntimeInteraction::new(
        "overview-owned-interaction",
        worker.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow owned command?".to_string()),
        "Allow owned command?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _owned_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), owned_interaction)
        .await
        .expect("owned interaction should register");
    let peer_interaction = RuntimeInteraction::new(
        "overview-peer-interaction",
        peer_worker.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow peer command?".to_string()),
        "Allow peer command?",
        vec![RuntimeInteractionChoice::new(
            "allow_once",
            "Allow once",
            "allow",
            Some(RuntimeInteractionChoiceStyle::Primary),
        )],
        None,
        None,
        None,
    );
    let _peer_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), peer_interaction)
        .await
        .expect("peer interaction should register");

    let overview = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            "chariox_meta_session_overview",
            serde_json::json!({
                "include_workflows": false,
                "include_events": true
            }),
        )
        .await
        .expect("meta session overview should dispatch");
    assert!(overview.ok);
    assert_eq!(
        overview
            .payload
            .pointer("/metaagent/id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );
    assert_eq!(
        overview
            .payload
            .pointer("/agents/owned_total")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        overview
            .payload
            .pointer("/agents/total")
            .and_then(serde_json::Value::as_u64),
        Some(4)
    );
    let owned_agents = overview
        .payload
        .pointer("/agents/owned")
        .and_then(serde_json::Value::as_array)
        .expect("owned agents should be included");
    assert!(owned_agents
        .iter()
        .any(|agent| { agent.get("id").and_then(serde_json::Value::as_str) == Some(worker.id()) }));
    let owned_worker = owned_agents
        .iter()
        .find(|agent| agent.get("id").and_then(serde_json::Value::as_str) == Some(worker.id()))
        .expect("owned worker should be present");
    assert_eq!(
        owned_worker
            .get("prompt_ref")
            .and_then(serde_json::Value::as_str),
        Some("worker")
    );
    assert!(
        owned_worker
            .get("example_prompt_command")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|command| command.starts_with("prompt worker ")),
        "{:?}",
        owned_worker
    );
    assert_eq!(
        overview
            .payload
            .pointer("/completion_recommendation/kind")
            .and_then(serde_json::Value::as_str),
        Some("should_wait"),
        "{:?}",
        overview.payload
    );
    assert_eq!(
        overview
            .payload
            .pointer("/completion_recommendation/pending_interaction_count")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "{:?}",
        overview.payload
    );
    assert_eq!(
        overview.payload.get("workflows"),
        Some(&serde_json::Value::Null)
    );
    let pending_interactions = overview
        .payload
        .get("pending_interactions")
        .and_then(serde_json::Value::as_array)
        .expect("pending interactions should be included");
    assert_eq!(pending_interactions.len(), 1);
    assert_eq!(
        pending_interactions
            .first()
            .and_then(|interaction| interaction.get("id"))
            .and_then(serde_json::Value::as_str),
        Some("overview-owned-interaction")
    );

    let search = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SEARCH_COMMANDS_TOOL,
            serde_json::json!({
                "query": "workflow",
                "mutates": true
            }),
        )
        .await
        .expect("meta command search should dispatch");
    assert!(search.ok);
    let commands = search
        .payload
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .expect("commands should be returned");
    assert!(commands.iter().any(|command| {
        command
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| name.starts_with("workflow "))
    }));

    let listed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_COMMANDS_TOOL,
            serde_json::json!({
                "tag": "agent",
                "scope": "session",
                "policy": "allow",
                "limit": 20
            }),
        )
        .await
        .expect("meta command list should dispatch");
    assert!(listed.ok);
    let listed_commands = listed
        .payload
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .expect("listed commands should be returned");
    assert!(listed_commands.iter().any(|command| {
        command.get("name").and_then(serde_json::Value::as_str) == Some("agent spawn")
    }));
    assert!(listed_commands.iter().all(|command| {
        command
            .get("tags")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some("agent")))
            && command.get("scope").and_then(serde_json::Value::as_str) == Some("session")
            && command
                .get("metaagent_policy")
                .and_then(serde_json::Value::as_str)
                == Some("allow")
    }));

    let docs = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_COMMAND_DOCS_TOOL,
            serde_json::json!({
                "command": "session create"
            }),
        )
        .await
        .expect("meta command docs should dispatch");
    assert!(docs.ok);
    assert_eq!(
        docs.payload
            .get("metaagent_policy")
            .and_then(serde_json::Value::as_str),
        Some("deny")
    );

    let guide_search = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SEARCH_GUIDES_TOOL,
            serde_json::json!({
                "query": "create endpoint run workflow",
                "tag": "workflow",
                "limit": 5
            }),
        )
        .await
        .expect("meta guide search should dispatch");
    assert!(guide_search.ok);
    let guides = guide_search
        .payload
        .get("guides")
        .and_then(serde_json::Value::as_array)
        .expect("guide search should return guides");
    assert!(guides.iter().any(|guide| {
        guide.get("id").and_then(serde_json::Value::as_str) == Some("workflows/basic-components")
    }));
    assert!(guides.iter().all(|guide| guide.get("body").is_none()));

    let listed_guides = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_GUIDES_TOOL,
            serde_json::json!({
                "command": "workflow run",
                "limit": 10
            }),
        )
        .await
        .expect("meta guide list should dispatch");
    assert!(listed_guides.ok);
    assert!(listed_guides
        .payload
        .get("guides")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|guides| guides.iter().any(|guide| {
            guide
                .get("commands")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|commands| {
                    commands
                        .iter()
                        .any(|command| command.as_str() == Some("workflow run"))
                })
        })));

    let guide = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_READ_GUIDE_TOOL,
            serde_json::json!({
                "guide": "agent-apps/generate-app"
            }),
        )
        .await
        .expect("meta guide read should dispatch");
    assert!(guide.ok);
    assert!(guide
        .payload
        .get("body")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|body| body.contains("Do not implement directly")));
}
