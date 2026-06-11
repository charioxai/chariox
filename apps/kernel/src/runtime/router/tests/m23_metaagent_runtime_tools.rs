use super::*;

struct TestMetaRuntimeEnv {
    root: std::path::PathBuf,
}

impl TestMetaRuntimeEnv {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "arroba-m23-metaagent-runtime-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("test meta runtime root should be created");
        Self { root }
    }
}

impl Drop for TestMetaRuntimeEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn runtime_mcp_advertises_meta_tools_only_to_metaagent_provider_runs() {
    let env = TestMetaRuntimeEnv::new("tool-visibility");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, standard_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let standard_run = launch_test_provider(
        &mut app,
        session.id(),
        standard_agent.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let standard_auth_token = standard_run
        .runtime_mcp_auth_token()
        .expect("standard run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let standard_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&standard_auth_token);
    assert!(
        standard_specs.iter().all(|spec| {
            spec.name != crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL
        }),
        "standard agents must not see metaagent runtime tools"
    );

    let meta_specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&meta_auth_token);
    assert!(
        meta_specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL),
        "metaagents should see the metaagent runtime MCP tools"
    );

    let denied = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &standard_auth_token,
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect_err("standard agents should not be able to guess-call meta tools");
    assert!(
        denied
            .to_string()
            .contains("only available to session metaagents"),
        "{denied:?}"
    );
}

#[tokio::test]
async fn metaagent_runtime_mcp_returns_session_overview_and_command_docs() {
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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
            "arroba_meta_session_overview",
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
        Some(3)
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
        command.get("name").and_then(serde_json::Value::as_str) == Some("workflow")
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
}

#[tokio::test]
async fn metaagent_run_command_submits_prompts_through_router_path() {
    let env = TestMetaRuntimeEnv::new("run-command-prompt");
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
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let _worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
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

    let result = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "prompt worker \"please inspect the failing test\""
            }),
        )
        .await
        .expect("meta run_command should dispatch through the router");

    assert!(result.ok, "{:?}", result.payload);
    assert_eq!(
        result
            .payload
            .get("command")
            .and_then(serde_json::Value::as_str),
        Some("prompt worker \"please inspect the failing test\"")
    );
    assert!(
        result.payload.get("response").is_some(),
        "router response should be included"
    );
    let steered = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "prompt worker \"add this to the active investigation\""
            }),
        )
        .await
        .expect("meta run_command should steer active worker prompts");
    assert!(steered.ok, "{:?}", steered.payload);
    assert_eq!(
        steered
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("steered")
    );
    let worker_queued_prompts = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load")
        .queued_prompts_for_agent(worker.id())
        .map(|queued| queued.len())
        .unwrap_or_default();
    assert_eq!(
        worker_queued_prompts, 0,
        "metaagent prompt commands should steer active local agents instead of queueing"
    );
    let attachments = app
        .lock()
        .await
        .attachments()
        .list_client_attachments(&format!("metaagent:{}:commands", metaagent.id()));
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].session_id(), session.id());
}

#[tokio::test]
async fn metaagent_run_command_routes_core_workflow_commands() {
    let env = TestMetaRuntimeEnv::new("run-command-workflow");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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

    let created = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow new meta-flow"
            }),
        )
        .await
        .expect("workflow create command should dispatch");
    assert!(created.ok, "{:?}", created.payload);
    assert!(
        serde_json::to_string(&created.payload)
            .expect("payload should serialize")
            .contains("meta-flow"),
        "{:?}",
        created.payload
    );

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow list"
            }),
        )
        .await
        .expect("workflow list command should dispatch");
    assert!(listed.ok, "{:?}", listed.payload);
    assert!(
        serde_json::to_string(&listed.payload)
            .expect("payload should serialize")
            .contains("meta-flow"),
        "{:?}",
        listed.payload
    );

    let invalid_run = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow run endpoint-only"
            }),
        )
        .await
        .expect("workflow run usage errors should be structured");
    assert!(!invalid_run.ok);
    assert!(
        invalid_run
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("workflow run <workflow-ref>")),
        "{:?}",
        invalid_run.payload
    );
}

#[tokio::test]
async fn metaagent_run_command_routes_owned_agent_lifecycle_commands() {
    let env = TestMetaRuntimeEnv::new("run-command-agent-lifecycle");
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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

    let aliased = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "agent alias worker renamed-worker"
            }),
        )
        .await
        .expect("agent alias command should dispatch");
    assert!(aliased.ok, "{:?}", aliased.payload);
    assert!(
        serde_json::to_string(&aliased.payload)
            .expect("payload should serialize")
            .contains("renamed-worker"),
        "{:?}",
        aliased.payload
    );

    let peer_delete = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("agent delete {}", peer_worker.agent_ref())
            }),
        )
        .await
        .expect("peer delete should return structured denial");
    assert!(!peer_delete.ok);
    assert!(
        peer_delete
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_delete.payload
    );

    let deleted = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "agent delete renamed-worker"
            }),
        )
        .await
        .expect("owned agent delete command should dispatch");
    assert!(deleted.ok, "{:?}", deleted.payload);
    let app = app.lock().await;
    let session = app
        .sessions()
        .get_session(session.id())
        .expect("session should remain");
    assert!(session
        .agents()
        .iter()
        .all(|agent| agent.id() != worker.id()));
}

#[tokio::test]
async fn metaagent_run_command_returns_structured_denials_for_forbidden_commands() {
    let env = TestMetaRuntimeEnv::new("run-command-deny");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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

    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "session new"
            }),
        )
        .await
        .expect("meta run_command denials should be structured tool results");

    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot create")),
        "{:?}",
        denied.payload
    );

    let docs = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_COMMAND_DOCS_TOOL,
            serde_json::json!({
                "command": "mcp list"
            }),
        )
        .await
        .expect("meta command docs should dispatch");
    assert!(docs.ok);
    assert_eq!(
        docs.payload
            .get("metaagent_policy")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
    assert_eq!(
        docs.payload
            .get("routed")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );

    let not_routed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "mcp list"
            }),
        )
        .await
        .expect("registry-backed not-routed commands should return structured tool results");
    assert!(!not_routed.ok);
    assert!(
        not_routed
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("command registry")),
        "{:?}",
        not_routed.payload
    );
}

#[tokio::test]
async fn regular_agent_turn_completion_injects_metaagent_event_and_inbox_entry() {
    let env = TestMetaRuntimeEnv::new("turn-event");
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
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let _worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
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
    let attach = attach_request(session.id(), "client-1");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(worker.id().to_string()),
        prompt: "finish this test turn".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit", None, None, &submit),
            submit,
        )
        .await
        .expect("worker prompt should submit");
    let complete = LocalDaemonRequest::CompletePrompt(CompletePromptRequest {
        session_id: session.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("complete", None, None, &complete),
            complete,
        )
        .await
        .expect("worker prompt should complete and notify metaagent");

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.turn.completed" }),
        )
        .await
        .expect("meta list_events should dispatch");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("turn completion event should be listed");
    let event_id = event
        .get("event_id")
        .and_then(serde_json::Value::as_str)
        .expect("event should have id")
        .to_string();
    assert_eq!(
        event
            .get("source_agent_id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        event
            .get("injected_prompt_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "event should record injected prompt id"
    );

    let read = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_READ_EVENT_TOOL,
            serde_json::json!({ "event_id": event_id }),
        )
        .await
        .expect("meta read_event should dispatch");
    assert!(read.ok);
    assert_eq!(
        read.payload
            .pointer("/event/read_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        true
    );
    let acked = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_ACK_EVENT_TOOL,
            serde_json::json!({ "event_id": event_id }),
        )
        .await
        .expect("meta ack_event should dispatch");
    assert!(acked.ok);
    assert_eq!(
        acked
            .payload
            .get("acked")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[tokio::test]
async fn metaagent_turn_overview_and_blob_are_scoped_to_owned_regular_agents() {
    let env = TestMetaRuntimeEnv::new("turn-trace");
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
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
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
    let attach = attach_request(session.id(), "client-trace");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-trace", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let submit = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id: attachment_id.clone(),
        target_agent_id: Some(worker.id().to_string()),
        prompt: "summarize the trace".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-trace", None, None, &submit),
            submit,
        )
        .await
        .expect("worker prompt should submit");
    let tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        worker_run.id(),
        Some(worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cargo test"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("provider tool output should append to operational history");

    let overview = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "worker" }),
        )
        .await
        .expect("turn overview should dispatch");
    assert!(overview.ok, "{:?}", overview.payload);
    let blob_id = overview
        .payload
        .pointer("/turns/0/items")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.iter().find_map(|item| item.get("blob_id")))
        .and_then(serde_json::Value::as_str)
        .expect("overview should include provider tool blob id")
        .to_string();

    let blob = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_BLOB_TOOL,
            serde_json::json!({ "blob_id": blob_id }),
        )
        .await
        .expect("turn blob should dispatch");
    assert!(blob.ok, "{:?}", blob.payload);
    assert_eq!(
        blob.payload
            .pointer("/agent/id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        blob.payload
            .pointer("/entries/0/entry/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("cargo test")),
        "{:?}",
        blob.payload
    );

    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "meta" }),
        )
        .await
        .expect("turn overview denial should dispatch");
    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        denied.payload
    );
}

#[tokio::test]
async fn metaagent_event_subscriptions_persist_and_can_be_removed() {
    let env = TestMetaRuntimeEnv::new("event-subscriptions");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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

    let subscribed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("subscribe should dispatch");
    assert!(subscribed.ok);
    let subscription_id = subscribed
        .payload
        .pointer("/subscription/subscription_id")
        .and_then(serde_json::Value::as_str)
        .expect("subscription id should be returned")
        .to_string();

    let listed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_SUBSCRIPTIONS_TOOL,
            serde_json::json!({}),
        )
        .await
        .expect("list subscriptions should dispatch");
    assert!(listed.ok);
    let subscriptions = listed
        .payload
        .get("subscriptions")
        .and_then(serde_json::Value::as_array)
        .expect("subscriptions should be listed");
    assert!(subscriptions.iter().any(|subscription| {
        subscription.get("kind").and_then(serde_json::Value::as_str) == Some("agent.turn.completed")
            && subscription
                .get("required")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    assert!(subscriptions.iter().any(|subscription| {
        subscription
            .get("subscription_id")
            .and_then(serde_json::Value::as_str)
            == Some(subscription_id.as_str())
            && subscription.get("kind").and_then(serde_json::Value::as_str)
                == Some("workflow.output.final")
    }));

    let removed = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UNSUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "subscription_id": subscription_id }),
        )
        .await
        .expect("unsubscribe should dispatch");
    assert!(removed.ok);
    assert_eq!(
        removed
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("removed")
    );
}

#[tokio::test]
async fn subscribed_workflow_output_records_and_injects_metaagent_event() {
    let env = TestMetaRuntimeEnv::new("workflow-output-event");
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
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let meta_run = launch_test_provider(
        &mut app,
        session.id(),
        metaagent.id(),
        "dev-stub",
        "dev-stub",
        "meta-model",
    );
    let worker_auth_token = worker_run
        .runtime_mcp_auth_token()
        .expect("worker run should expose runtime MCP auth token")
        .to_string();
    let meta_auth_token = meta_run
        .runtime_mcp_auth_token()
        .expect("meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_SUBSCRIBE_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("metaagent should subscribe to workflow final outputs");

    let create_workflow = LocalDaemonRequest::CreateWorkflow(crate::local::CreateWorkflowRequest {
        session_id: session.id().to_string(),
        alias: Some("review".to_string()),
    });
    let workflow = match router
        .dispatch(
            KernelCommand::from_local_request("create-workflow", None, None, &create_workflow),
            create_workflow,
        )
        .await
        .expect("workflow should be created")
    {
        LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow,
        other => panic!("unexpected workflow create response: {other:?}"),
    };
    let add_node = LocalDaemonRequest::AddWorkflowNode(crate::local::AddWorkflowNodeRequest {
        session_id: session.id().to_string(),
        workflow_ref: workflow.id().to_string(),
        agent_id: worker.id().to_string(),
        expected_workflow_revision: None,
    });
    let node = match router
        .dispatch(
            KernelCommand::from_local_request("add-workflow-node", None, None, &add_node),
            add_node,
        )
        .await
        .expect("workflow node should be added")
    {
        LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node,
        other => panic!("unexpected workflow node response: {other:?}"),
    };
    let set_completion = LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(
        crate::local::SetWorkflowNodeCanCompleteRunRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            node_id: node.id().to_string(),
            can_complete_workflow_run: true,
            expected_workflow_revision: None,
        },
    );
    router
        .dispatch(
            KernelCommand::from_local_request("set-workflow-complete", None, None, &set_completion),
            set_completion,
        )
        .await
        .expect("workflow node should be allowed to complete run");
    let create_endpoint =
        LocalDaemonRequest::CreateWorkflowEndpoint(crate::local::CreateWorkflowEndpointRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            entry_node_id: node.id().to_string(),
            alias: Some("entry".to_string()),
            expected_workflow_revision: None,
        });
    let endpoint = match router
        .dispatch(
            KernelCommand::from_local_request(
                "create-workflow-endpoint",
                None,
                None,
                &create_endpoint,
            ),
            create_endpoint,
        )
        .await
        .expect("workflow endpoint should be created")
    {
        LocalDaemonResponse::WorkflowEndpointCreated { endpoint, .. } => endpoint,
        other => panic!("unexpected workflow endpoint response: {other:?}"),
    };
    let invoke =
        LocalDaemonRequest::InvokeWorkflowEndpoint(crate::local::InvokeWorkflowEndpointRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow.id().to_string(),
            endpoint_ref: endpoint.id().to_string(),
            prompt: Some("produce final output".to_string()),
            queue_ref: None,
            publication_invocation: None,
        });
    let workflow_run = match router
        .dispatch(
            KernelCommand::from_local_request("invoke-workflow", None, None, &invoke),
            invoke,
        )
        .await
        .expect("workflow should be invoked")
    {
        LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
        other => panic!("unexpected workflow invoke response: {other:?}"),
    };

    let submitted = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &worker_auth_token,
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL,
            serde_json::json!({
                "workflow_output_json": "{\"summary\":\"done\"}"
            }),
        )
        .await
        .expect("workflow final output tool should dispatch");
    assert!(submitted.ok, "{:?}", submitted.payload);
    assert_eq!(
        submitted
            .payload
            .get("workflow_run_id")
            .and_then(serde_json::Value::as_str),
        Some(workflow_run.id())
    );

    let listed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "workflow.output.final" }),
        )
        .await
        .expect("workflow final output event should list");
    assert!(listed.ok);
    let event = listed
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .and_then(|events| events.first())
        .expect("workflow output event should be recorded");
    assert_eq!(
        event
            .get("source_agent_id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    assert!(
        event
            .get("injected_prompt_id")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "workflow output event should record prompt injection id"
    );
}

#[tokio::test]
async fn metaagent_can_resolve_owned_regular_agent_interactions_but_not_its_own() {
    let env = TestMetaRuntimeEnv::new("interaction");
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
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
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
    let attach = attach_request(session.id(), "client-meta-busy");
    let attachment_id = match router
        .dispatch(
            KernelCommand::from_local_request("attach-meta-busy", None, None, &attach),
            attach,
        )
        .await
        .expect("client should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment.id().to_string(),
        other => panic!("unexpected attach response: {other:?}"),
    };
    let meta_prompt = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
        session_id: session.id().to_string(),
        attachment_id,
        target_agent_id: Some(metaagent.id().to_string()),
        prompt: "stay busy while a worker asks for permission".to_string(),
        attachments: Vec::new(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("submit-meta-busy", None, None, &meta_prompt),
            meta_prompt,
        )
        .await
        .expect("metaagent prompt should start");
    let worker_interaction = RuntimeInteraction::new(
        "interaction-worker",
        worker.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow command?".to_string()),
        "Allow command?",
        vec![
            RuntimeInteractionChoice::new(
                "allow_once",
                "Allow once",
                "allow",
                Some(RuntimeInteractionChoiceStyle::Primary),
            ),
            RuntimeInteractionChoice::new(
                "deny",
                "Deny",
                "deny",
                Some(RuntimeInteractionChoiceStyle::Danger),
            ),
        ],
        None,
        None,
        None,
    );
    let worker_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), worker_interaction)
        .await
        .expect("worker interaction should register");
    let meta_queued_prompts = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load")
        .queued_prompts_for_agent(metaagent.id())
        .map(|queued| queued.len())
        .unwrap_or_default();
    assert_eq!(
        meta_queued_prompts, 0,
        "runtime interaction event prompts should steer an active metaagent instead of queueing"
    );
    let listed_events = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "runtime.interaction" }),
        )
        .await
        .expect("required interaction event should be listed");
    assert!(listed_events.ok);
    let events = listed_events
        .payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .expect("interaction events should be returned");
    assert_eq!(events.len(), 1);
    assert_eq!(
        events
            .first()
            .and_then(|event| event.get("source_agent_id"))
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );

    let resolved = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-worker",
                "choice_id": "allow_once"
            }),
        )
        .await
        .expect("meta interaction resolution should dispatch");
    assert!(resolved.ok, "{:?}", resolved.payload);
    let resolution = tokio::time::timeout(std::time::Duration::from_secs(1), worker_resolution)
        .await
        .expect("resolution should be delivered")
        .expect("interaction responder should receive resolution");
    assert_eq!(resolution.choice_id.as_deref(), Some("allow_once"));
    assert_eq!(resolution.reply.as_deref(), Some("allow"));

    let self_interaction = RuntimeInteraction::new(
        "interaction-meta",
        metaagent.id(),
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow self?".to_string()),
        "Allow self?",
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
    let _self_resolution = router
        .runtime_state
        .create_runtime_interaction(session.id(), self_interaction)
        .await
        .expect("self interaction should register");
    let denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RESOLVE_RUNTIME_INTERACTION_TOOL,
            serde_json::json!({
                "interaction_id": "interaction-meta",
                "choice_id": "allow_once"
            }),
        )
        .await
        .expect("self resolution denial should dispatch");
    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot resolve their own")),
        "{:?}",
        denied.payload
    );
}
