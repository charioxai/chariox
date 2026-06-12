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
async fn forwarded_remote_metaagent_runtime_tools_use_home_scope() {
    let env = TestMetaRuntimeEnv::new("forwarded-remote");
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
                .with_alias("remote-meta")
                .with_role(crate::agent::AgentRole::Meta),
        )
        .expect("metaagent should spawn");
    let metaagent = app
        .agents()
        .bind_remote_execution(
            metaagent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("worker-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("metaagent should be remote-backed");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: "home-kernel".to_string(),
        home_session_id: session.id().to_string(),
        home_agent_id: metaagent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_kernel_id: "worker-kernel".to_string(),
        worker_machine_id: "worker-machine".to_string(),
        worker_provider_run_id: "worker-run-1".to_string(),
        worker_worktree_path: workspace.to_string_lossy().to_string(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(
            workspace.to_string_lossy().to_string(),
        ),
    };

    let overview = router
        .dispatch_forwarded_meta_runtime_tool_call(
            context.clone(),
            crate::transport::runtime_tools::META_SESSION_OVERVIEW_TOOL.to_string(),
            serde_json::json!({}),
        )
        .await
        .expect("forwarded overview should dispatch home-side");
    assert!(overview.ok, "{overview:?}");
    assert_eq!(
        overview
            .payload
            .pointer("/metaagent/id")
            .and_then(serde_json::Value::as_str),
        Some(metaagent.id())
    );

    let command = router
        .dispatch_forwarded_meta_runtime_tool_call(
            context,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL.to_string(),
            serde_json::json!({ "command": "agent list" }),
        )
        .await
        .expect("forwarded run_command should dispatch through the router");
    assert!(command.ok, "{command:?}");
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
    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let command_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.command.executed"
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["command"] == "prompt worker \"please inspect the failing test\""
                && event.payload["status"] == "succeeded"
        })
        .expect("metaagent command audit should include durable provenance");
    assert_eq!(command_audit.payload["provider_run_id"], meta_run.id());
    assert_eq!(command_audit.payload["causation_id"], meta_run.id());
    let command_correlation_id = command_audit
        .payload
        .get("correlation_id")
        .and_then(serde_json::Value::as_str)
        .expect("command audit should include a correlation id");
    assert!(
        command_correlation_id.starts_with(&format!("metaagent:{}:command:", metaagent.id())),
        "{command_correlation_id}"
    );
    let prompt_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.prompt.submitted"
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["target_agent_id"] == worker.id()
                && event.payload["status"] == "steered"
        })
        .expect("metaagent prompt audit should include durable provenance");
    let prompt_id = prompt_audit
        .payload
        .get("prompt_id")
        .and_then(serde_json::Value::as_str)
        .expect("prompt audit should include a prompt id");
    assert_eq!(prompt_audit.payload["causation_id"], prompt_id);
    assert_eq!(
        prompt_audit.payload["correlation_id"],
        format!("metaagent:{}:prompt:{prompt_id}", metaagent.id())
    );
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
async fn user_agent_lifecycle_events_notify_metaagent_but_meta_commands_do_not() {
    let env = TestMetaRuntimeEnv::new("agent-lifecycle-events");
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

    let human_spawn = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        session_id: session.id().to_string(),
        alias: Some("human-worker".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some(workspace.to_string_lossy().to_string()),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let spawned = router
        .dispatch(
            KernelCommand::from_local_request("human-spawn-worker", None, None, &human_spawn),
            human_spawn,
        )
        .await
        .expect("human spawn should dispatch");
    let LocalDaemonResponse::AgentSpawned {
        agent: human_worker,
    } = spawned
    else {
        panic!("unexpected human spawn response");
    };

    let events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert!(events.ok, "{events:?}");
    assert_eq!(
        events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events.payload
    );

    let meta_spawn = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn quiet-worker" }),
        )
        .await
        .expect("metaagent spawn command should dispatch");
    assert!(meta_spawn.ok, "{meta_spawn:?}");
    let events_after_meta_spawn = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert_eq!(
        events_after_meta_spawn
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events_after_meta_spawn.payload
    );

    let human_delete = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: session.id().to_string(),
        agent_id: human_worker.id().to_string(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("human-delete-worker", None, None, &human_delete),
            human_delete,
        )
        .await
        .expect("human delete should dispatch");
    let deleted_events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.deleted" }),
        )
        .await
        .expect("metaagent should list delete lifecycle events");
    assert_eq!(
        deleted_events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        deleted_events.payload
    );
}

#[tokio::test]
async fn forged_metaagent_caller_id_does_not_suppress_lifecycle_events() {
    let env = TestMetaRuntimeEnv::new("forged-metaagent-caller");
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
    let metaagent_id = metaagent.id().to_string();
    let session_id = session.id().to_string();
    let worktree_id = workspace.to_string_lossy().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);

    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        session_id,
        alias: Some("forged-worker".to_string()),
        provider: Some("dev-stub".to_string()),
        model: Some("default".to_string()),
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: Some(worktree_id),
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let mut command =
        KernelCommand::from_local_request("forged-metaagent-spawn-worker", None, None, &request);
    command.caller.caller_id = format!("metaagent:{metaagent_id}");
    router
        .dispatch(command, request)
        .await
        .expect("forged caller id should dispatch as a normal user command");

    let events = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_LIST_EVENTS_TOOL,
            serde_json::json!({ "kind": "agent.spawned" }),
        )
        .await
        .expect("metaagent should list lifecycle events");
    assert_eq!(
        events
            .payload
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1),
        "{:?}",
        events.payload
    );
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
        Some(true)
    );

    for command in ["mcp list", "skill list", "credential list", "slice list"] {
        let routed = router
            .dispatch_authenticated_runtime_tool_call(
                &meta_auth_token,
                crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
                serde_json::json!({
                    "command": command
                }),
            )
            .await
            .expect("safe registered commands should dispatch");
        assert!(routed.ok, "{command}: {:?}", routed.payload);
    }

    let not_routed = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "mcp install test --command node"
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
            .is_some_and(|message| message.contains("only `mcp list`")),
        "{:?}",
        not_routed.payload
    );

    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    let denied_audit = audit_events
        .iter()
        .find(|event| {
            event.kind == "metaagent.command.executed"
                && event.payload["metaagent_id"] == metaagent.id()
                && event.payload["command"] == "session new"
                && event.payload["status"] == "denied"
        })
        .expect("denied metaagent commands should be audited");
    assert_eq!(denied_audit.payload["provider_run_id"], meta_run.id());
    assert_eq!(denied_audit.payload["causation_id"], meta_run.id());
    let denied_correlation_id = denied_audit
        .payload
        .get("correlation_id")
        .and_then(serde_json::Value::as_str)
        .expect("denied command audit should include a correlation id");
    assert!(
        denied_correlation_id.starts_with(&format!("metaagent:{}:command:", metaagent.id())),
        "{denied_correlation_id}"
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
    assert!(
        event
            .get("prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| matches!(status, "submitted" | "delivered")),
        "event should expose visible prompt delivery status: {event:?}"
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
    assert!(
        read.payload
            .pointer("/event/prompt_delivery_status")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "{:?}",
        read.payload
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
async fn local_metaagent_command_search_request_enforces_owner_scope() {
    Box::pin(local_metaagent_command_search_request_enforces_owner_scope_impl()).await
}

async fn local_metaagent_command_search_request_enforces_owner_scope_impl() {
    let env = TestMetaRuntimeEnv::new("local-command-search");
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
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());

    let search_request =
        LocalDaemonRequest::SearchMetaagentCommands(SearchMetaagentCommandsRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            query: Some("agent".to_string()),
            tag: Some("agent".to_string()),
            scope: Some("session".to_string()),
            mutates: None,
            policy: Some("allow".to_string()),
            limit: Some(10),
        });
    let searched = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "search-metaagent-commands",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &search_request,
            ),
            search_request.clone(),
        )
        .await
        .expect("owner should search metaagent commands");
    let LocalDaemonResponse::MetaagentCommandsSearched { commands } = searched else {
        panic!("unexpected metaagent command search response: {searched:?}");
    };
    assert!(
        commands.iter().any(|command| {
            command
                .get("name")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|name| name.contains("agent"))
        }),
        "command search should return agent command descriptors: {commands:?}"
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "foreign-search-metaagent-commands",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &search_request,
            ),
            search_request,
        )
        .await
        .expect_err("another user must not search a metaagent command registry");
    assert!(
        denied
            .to_string()
            .contains("requires an owned session metaagent"),
        "{denied:?}"
    );
}

#[tokio::test]
async fn local_metaagent_turn_inspection_requests_enforce_owner_scope() {
    Box::pin(local_metaagent_turn_inspection_requests_enforce_owner_scope_impl()).await
}

async fn local_metaagent_turn_inspection_requests_enforce_owner_scope_impl() {
    let env = TestMetaRuntimeEnv::new("local-turn-inspection");
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
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "attachment-local-turn",
        worker.id(),
        "inspect this turn",
    );
    router
        .operational_history_store
        .append_transcript(
            &prompt_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(worker.id().to_string()),
                provider: Some(worker_run.provider().to_string()),
                model: Some(worker_run.model().to_string()),
                provider_run_id: Some(worker_run.id().to_string()),
                turn_id: Some("local-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("user prompt should append to operational history");
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
                turn_id: Some("local-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("provider tool output should append to operational history");

    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());
    let overview_request =
        LocalDaemonRequest::GetMetaagentTurnOverview(GetMetaagentTurnOverviewRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("local-turn".to_string()),
            turns_back: None,
            limit: Some(20),
        });
    let overview = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "local-metaagent-turn-overview",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &overview_request,
            ),
            overview_request,
        )
        .await
        .expect("owner should inspect metaagent turn overview");
    let LocalDaemonResponse::MetaagentTurnOverview { overview } = overview else {
        panic!("unexpected metaagent turn overview response: {overview:?}");
    };
    assert_eq!(
        overview
            .pointer("/agent/id")
            .and_then(serde_json::Value::as_str),
        Some(worker.id())
    );
    let blob_id = overview
        .pointer("/turns/0/items")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.iter().find_map(|item| item.get("blob_id")))
        .and_then(serde_json::Value::as_str)
        .expect("overview should expose provider tool blob id")
        .to_string();

    let blob_request = LocalDaemonRequest::GetMetaagentTurnBlob(GetMetaagentTurnBlobRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        blob_id: blob_id.clone(),
    });
    let blob = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "local-metaagent-turn-blob",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &blob_request,
            ),
            blob_request,
        )
        .await
        .expect("owner should inspect metaagent turn blob");
    let LocalDaemonResponse::MetaagentTurnBlob { blob } = blob else {
        panic!("unexpected metaagent turn blob response: {blob:?}");
    };
    assert_eq!(
        blob.get("blob_id").and_then(serde_json::Value::as_str),
        Some(blob_id.as_str())
    );
    assert!(
        blob.pointer("/entries/0/entry/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("cargo test")),
        "{blob:?}"
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let forged_request =
        LocalDaemonRequest::GetMetaagentTurnOverview(GetMetaagentTurnOverviewRequest {
            session_id: session.id().to_string(),
            metaagent_id: metaagent.id().to_string(),
            agent_ref: Some("worker".to_string()),
            turn_ref: Some("local-turn".to_string()),
            turns_back: None,
            limit: Some(20),
        });
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "forged-local-metaagent-turn-overview",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &forged_request,
            ),
            forged_request,
        )
        .await
        .expect_err("foreign users must not inspect owned metaagent turns");
    assert!(
        denied.to_string().contains("owned session metaagent"),
        "{denied}"
    );
}

#[tokio::test]
async fn local_metaagent_event_requests_enforce_owner_and_mutate_inbox() {
    Box::pin(local_metaagent_event_requests_enforce_owner_and_mutate_inbox_impl()).await
}

async fn local_metaagent_event_requests_enforce_owner_and_mutate_inbox_impl() {
    let env = TestMetaRuntimeEnv::new("local-event-requests");
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
    let event =
        app.metaagent_event_store()
            .record(crate::runtime::metaagent_event::NewMetaagentEvent {
                session_id: session.id().to_string(),
                metaagent_id: metaagent.id().to_string(),
                owner_user_id: metaagent.owner_user_id().to_string(),
                kind: "agent.turn.completed".to_string(),
                source_agent_id: Some("agent-1".to_string()),
                title: "Worker completed".to_string(),
                summary: "Worker completed a turn".to_string(),
                detail: serde_json::json!({ "prompt_id": "prompt-1" }),
                injected_prompt_id: None,
            });
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mut owner_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    owner_caller.user_id = Some(metaagent.owner_user_id().to_string());

    let list_request = LocalDaemonRequest::ListMetaagentEvents(ListMetaagentEventsRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        limit: Some(10),
        status: None,
        kind: Some("agent.turn.completed".to_string()),
    });
    let listed = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "list-metaagent-events",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &list_request,
            ),
            list_request.clone(),
        )
        .await
        .expect("owner should list metaagent events");
    let LocalDaemonResponse::MetaagentEventsListed { events } = listed else {
        panic!("unexpected metaagent event list response: {listed:?}");
    };
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]
            .get("event_id")
            .and_then(serde_json::Value::as_str),
        Some(event.event_id.as_str())
    );

    let mut forged_caller = KernelCaller::for_source(&KernelCommandSource::LocalCli);
    forged_caller.user_id = Some("user-2".to_string());
    let denied = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "foreign-list-metaagent-events",
                KernelCommandSource::LocalCli,
                forged_caller,
                None,
                None,
                &list_request,
            ),
            list_request,
        )
        .await
        .expect_err("another user must not list a metaagent inbox");
    assert!(
        denied
            .to_string()
            .contains("requires an owned session metaagent"),
        "{denied:?}"
    );

    let read_request = LocalDaemonRequest::ReadMetaagentEvent(ReadMetaagentEventRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        event_id: event.event_id.clone(),
    });
    let read = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "read-metaagent-event",
                KernelCommandSource::LocalCli,
                owner_caller.clone(),
                None,
                None,
                &read_request,
            ),
            read_request,
        )
        .await
        .expect("owner should read metaagent event");
    let LocalDaemonResponse::MetaagentEventRead { event: read_event } = read else {
        panic!("unexpected metaagent event read response: {read:?}");
    };
    assert!(
        read_event
            .get("read_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{read_event:?}"
    );

    let ack_request = LocalDaemonRequest::AckMetaagentEvents(AckMetaagentEventsRequest {
        session_id: session.id().to_string(),
        metaagent_id: metaagent.id().to_string(),
        event_id: Some(event.event_id.clone()),
        event_ids: None,
        up_to_sequence: None,
    });
    let acked = router
        .dispatch(
            KernelCommand::from_local_request_with_caller(
                "ack-metaagent-event",
                KernelCommandSource::LocalCli,
                owner_caller,
                None,
                None,
                &ack_request,
            ),
            ack_request,
        )
        .await
        .expect("owner should ack metaagent event");
    let LocalDaemonResponse::MetaagentEventsAcked { acked } = acked else {
        panic!("unexpected metaagent event ack response: {acked:?}");
    };
    assert_eq!(acked.len(), 1);
    assert!(
        acked[0]
            .get("ack_at_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "{acked:?}"
    );
}

#[tokio::test]
async fn metaagent_event_prompts_retry_after_provider_launch() {
    let env = TestMetaRuntimeEnv::new("event-retry-after-launch");
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
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    router
        .runtime_state
        .inject_metaagent_agent_lifecycle_event_for_agent(session.id(), &worker, "agent.spawned")
        .await
        .expect("metaagent event should record even before provider launch");
    let failed_event = app
        .lock()
        .await
        .metaagent_event_store()
        .list(metaagent.id(), Some("agent.spawned"), Some("failed"), 10)
        .into_iter()
        .next()
        .expect("event prompt should fail while no metaagent provider route exists");
    let failed_prompt_id = failed_event.injected_prompt_id.clone();

    let started = app
        .lock()
        .await
        .start_provider_launch(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "meta-model",
            )
            .with_agent_id(metaagent.id()),
        )
        .expect("metaagent provider launch should start");
    router
        .runtime_state
        .finish_provider_launch(&started, None)
        .await;

    let retried_event = app
        .lock()
        .await
        .metaagent_event_store()
        .read(metaagent.id(), &failed_event.event_id)
        .expect("event should still be readable after retry");
    assert_ne!(
        retried_event.prompt_delivery_status, "failed",
        "provider launch should retry failed metaagent event prompts: {retried_event:?}"
    );
    assert_ne!(
        retried_event.injected_prompt_id, failed_prompt_id,
        "retry should use a fresh prompt id for the replayed visible event prompt"
    );
    assert!(
        matches!(
            retried_event.prompt_delivery_status.as_str(),
            "submitted" | "queued" | "delivered"
        ),
        "retried event should be re-admitted through normal prompt delivery: {retried_event:?}"
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
    let worker_run = launch_test_provider(
        &mut app,
        session.id(),
        worker.id(),
        "dev-stub",
        "dev-stub",
        "worker-model",
    );
    let peer_worker_run = launch_test_provider(
        &mut app,
        session.id(),
        peer_worker.id(),
        "dev-stub",
        "dev-stub",
        "peer-worker-model",
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
    let peer_prompt_entry = crate::history::SessionHistoryEntry::user_prompt(
        session.id(),
        "peer-attachment",
        peer_worker.id(),
        "peer private prompt",
    );
    router
        .operational_history_store
        .append_transcript(
            &peer_prompt_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(peer_worker.id().to_string()),
                provider: Some(peer_worker_run.provider().to_string()),
                model: Some(peer_worker_run.model().to_string()),
                provider_run_id: Some(peer_worker_run.id().to_string()),
                turn_id: Some("peer-trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("peer prompt should append to operational history");
    let peer_tool_entry = crate::history::SessionHistoryEntry::provider_output(
        session.id(),
        peer_worker_run.id(),
        Some(peer_worker.id()),
        crate::terminal::TerminalOutputKind::ProviderTool,
        None,
        serde_json::json!({
            "tool": "shell",
            "status": "completed",
            "input": {"command": "cat secret-peer-file"}
        })
        .to_string(),
    );
    router
        .operational_history_store
        .append_transcript(
            &peer_tool_entry,
            crate::history::HistoryEventTurnContext {
                session_id: Some(session.id().to_string()),
                agent_id: Some(peer_worker.id().to_string()),
                provider: Some(peer_worker_run.provider().to_string()),
                model: Some(peer_worker_run.model().to_string()),
                provider_run_id: Some(peer_worker_run.id().to_string()),
                turn_id: Some("peer-trace-turn".to_string()),
                ..crate::history::HistoryEventTurnContext::default()
            },
        )
        .expect("peer provider tool output should append to operational history");

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

    let peer_overview_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_OVERVIEW_TOOL,
            serde_json::json!({ "agent_ref": "peer-worker" }),
        )
        .await
        .expect("peer turn overview denial should dispatch");
    assert!(!peer_overview_denied.ok);
    assert!(
        peer_overview_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_overview_denied.payload
    );

    let peer_history = crate::runtime::history_requests::execute_session_history_outline_request(
        router.operational_history_store.clone(),
        crate::local::GetSessionHistoryOutlineRequest {
            session_id: session.id().to_string(),
            agent_ids: Some(vec![peer_worker.id().to_string()]),
            latest_prompt_count: Some(1),
        },
    )
    .await
    .expect("peer history outline should load");
    let crate::local::LocalDaemonResponse::SessionHistoryOutline { agents } = peer_history else {
        panic!("unexpected peer history response");
    };
    let peer_blob_id = agents
        .first()
        .and_then(|agent| agent.turns.first())
        .and_then(|turn| turn.blobs.first())
        .map(|blob| blob.blob_id.clone())
        .expect("peer history should include a blob");
    let peer_blob_denied = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_TURN_BLOB_TOOL,
            serde_json::json!({ "blob_id": peer_blob_id }),
        )
        .await
        .expect("peer blob denial should dispatch");
    assert!(!peer_blob_denied.ok);
    assert!(
        peer_blob_denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("owned regular agent")),
        "{:?}",
        peer_blob_denied.payload
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
async fn subscribed_collaborator_workflow_output_records_and_injects_metaagent_event() {
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
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_owner_user_id("user-2"),
        )
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
    let audit_events = app
        .lock()
        .await
        .durable_state_store()
        .load_events_after(0)
        .expect("durable audit events should load");
    assert!(audit_events.iter().any(|event| {
        event.kind == "metaagent.interaction.resolved"
            && event.payload["session_id"] == session.id()
            && event.payload["metaagent_id"] == metaagent.id()
            && event.payload["target_agent_id"] == worker.id()
            && event.payload["interaction_id"] == "interaction-worker"
            && event.payload["choice_id"] == "allow_once"
            && event.payload["causation_id"] == "interaction-worker"
            && event.payload["correlation_id"]
                == format!(
                    "metaagent:{}:runtime-interaction:interaction-worker",
                    metaagent.id()
                )
    }));

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
