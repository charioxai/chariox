use super::*;

#[test]
fn metaagent_run_command_submits_prompts_through_router_path() {
    run_large_stack_async_test(
        "metaagent-run-command-prompt",
        metaagent_run_command_submits_prompts_through_router_path_inner,
    );
}

async fn metaagent_run_command_submits_prompts_through_router_path_inner() {
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
    let metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_controlled_by_metaagent_id(metaagent.id()),
        )
        .expect("worker should spawn");
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
        result.payload.get("outcome").is_some(),
        "compact prompt outcome should be included"
    );
    assert_eq!(
        result
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("submitted")
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

#[test]
fn multiple_metaagents_in_one_session_are_isolated() {
    run_large_stack_async_test(
        "multiple-metaagents-isolated",
        multiple_metaagents_in_one_session_are_isolated_inner,
    );
}

async fn multiple_metaagents_in_one_session_are_isolated_inner() {
    let env = TestMetaRuntimeEnv::new("multi-metaagent-isolation");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let meta_a = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta-a"))
        .expect("first regular agent should spawn");
    let meta_b = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta-b"))
        .expect("second regular agent should spawn");
    let task_a_id = app
        .sessions_mut()
        .ensure_metaagent_task(session.id(), meta_a.id(), "Coordinate alpha work.")
        .expect("meta A task should start")
        .metaagent_task(meta_a.id())
        .expect("meta A task should be projected")
        .task_id()
        .to_string();
    app.sessions_mut()
        .update_metaagent_plan_markdown(session.id(), meta_a.id(), "- Spawn alpha worker.")
        .expect("meta A plan should be recorded");
    app.agents_mut()
        .activate_agent_meta_mode(meta_a.id(), Some(task_a_id))
        .expect("meta A should enter meta mode");
    let task_b_id = app
        .sessions_mut()
        .ensure_metaagent_task(session.id(), meta_b.id(), "Coordinate beta work.")
        .expect("meta B task should start")
        .metaagent_task(meta_b.id())
        .expect("meta B task should be projected")
        .task_id()
        .to_string();
    app.sessions_mut()
        .update_metaagent_plan_markdown(session.id(), meta_b.id(), "- Spawn beta worker.")
        .expect("meta B plan should be recorded");
    app.agents_mut()
        .activate_agent_meta_mode(meta_b.id(), Some(task_b_id))
        .expect("meta B should enter meta mode");
    let meta_a = app
        .agents()
        .get_agent(meta_a.id())
        .expect("meta A should exist after activation");
    let meta_b = app
        .agents()
        .get_agent(meta_b.id())
        .expect("meta B should exist after activation");
    assert!(meta_a.is_metaagent());
    assert!(meta_b.is_metaagent());
    assert_eq!(meta_a.role(), crate::agent::AgentRole::Standard);
    assert_eq!(meta_b.role(), crate::agent::AgentRole::Standard);
    let meta_a_run = launch_test_provider(
        &mut app,
        session.id(),
        meta_a.id(),
        "dev-stub",
        "dev-stub",
        "meta-a-model",
    );
    let meta_b_run = launch_test_provider(
        &mut app,
        session.id(),
        meta_b.id(),
        "dev-stub",
        "dev-stub",
        "meta-b-model",
    );
    let meta_a_auth = meta_a_run
        .runtime_mcp_auth_token()
        .expect("meta A run should expose runtime MCP auth token")
        .to_string();
    let meta_b_auth = meta_b_run
        .runtime_mcp_auth_token()
        .expect("meta B run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let spawn_a = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn alpha" }),
        )
        .await
        .expect("meta A spawn command should dispatch");
    assert!(spawn_a.ok, "{:?}", spawn_a.payload);
    let alpha_id = spawn_a
        .payload
        .pointer("/response/agent/id")
        .and_then(serde_json::Value::as_str)
        .expect("spawn A response should include agent id")
        .to_string();

    let spawn_b = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_b_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn beta" }),
        )
        .await
        .expect("meta B spawn command should dispatch");
    assert!(spawn_b.ok, "{:?}", spawn_b.payload);
    let beta_id = spawn_b
        .payload
        .pointer("/response/agent/id")
        .and_then(serde_json::Value::as_str)
        .expect("spawn B response should include agent id")
        .to_string();

    {
        let app = app.lock().await;
        let alpha = app
            .agents()
            .get_agent(&alpha_id)
            .expect("alpha worker should exist");
        let beta = app
            .agents()
            .get_agent(&beta_id)
            .expect("beta worker should exist");
        assert_eq!(alpha.controlled_by_metaagent_id(), Some(meta_a.id()));
        assert_eq!(beta.controlled_by_metaagent_id(), Some(meta_b.id()));
    }

    let list_a = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent list" }),
        )
        .await
        .expect("meta A list command should dispatch");
    assert!(list_a.ok, "{:?}", list_a.payload);
    let listed_agents = list_a
        .payload
        .pointer("/response/agents")
        .and_then(serde_json::Value::as_array)
        .expect("agent list should include agents");
    assert_eq!(listed_agents.len(), 1);
    assert_eq!(
        listed_agents[0]
            .get("alias")
            .and_then(serde_json::Value::as_str),
        Some("alpha")
    );

    let prompt_cross = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "prompt beta \"do not allow this\"" }),
        )
        .await
        .expect("cross prompt command should dispatch as a rejected tool result");
    assert!(!prompt_cross.ok, "{:?}", prompt_cross.payload);
    assert!(
        prompt_cross
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        prompt_cross.payload
    );
    let cross_prompt_error = prompt_cross
        .payload
        .get("error")
        .and_then(serde_json::Value::as_str)
        .expect("cross prompt should include an error");
    assert!(
        cross_prompt_error.contains("Available owned agents: alpha")
            && cross_prompt_error.contains("prompt alpha"),
        "{cross_prompt_error}"
    );
    assert!(
        !cross_prompt_error.contains("Available owned agents: beta"),
        "{cross_prompt_error}"
    );

    let create_workflow = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow new flow-a" }),
        )
        .await
        .expect("meta A workflow create should dispatch");
    assert!(create_workflow.ok, "{:?}", create_workflow.payload);

    let add_cross_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_a_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow node add flow-a beta" }),
        )
        .await
        .expect("cross workflow node command should dispatch as a rejected tool result");
    assert!(!add_cross_node.ok, "{:?}", add_cross_node.payload);

    let resolve_cross_workflow = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_b_auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow resolve flow-a" }),
        )
        .await
        .expect("cross workflow resolve should dispatch as a rejected tool result");
    assert!(
        !resolve_cross_workflow.ok,
        "{:?}",
        resolve_cross_workflow.payload
    );
    assert!(
        resolve_cross_workflow
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not controlled by metaagent")),
        "{:?}",
        resolve_cross_workflow.payload
    );
}

#[test]
fn metaagent_prompt_command_does_not_steer_active_workflow_turns() {
    run_large_stack_async_test(
        "metaagent-prompt-active-workflow-guard",
        metaagent_prompt_command_does_not_steer_active_workflow_turns_inner,
    );
}

async fn metaagent_prompt_command_does_not_steer_active_workflow_turns_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-workflow-prompt-guard");
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
        .spawn_agent(CreateAgentRequest::new(session.id(), "dev-stub").with_alias("meta"))
        .expect("metaagent should spawn");
    let metaagent = activate_test_agent_meta_mode(&mut app, metaagent);
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_controlled_by_metaagent_id(metaagent.id()),
        )
        .expect("worker should spawn");
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
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("guarded-flow".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), worker.id())
        .expect("workflow node should be created");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("start".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run guarded workflow".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "workflow node prompt".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    app.sessions_mut()
        .start_workflow_node_run(session.id(), workflow_run.id(), &node_run_id)
        .expect("workflow node run should start");
    let workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        worker.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);
    app.prompt_owner_submit_prepared_prompt(session.id(), workflow_prompt, false)
        .expect("workflow prompt should become active");

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
                "command": "prompt worker \"finish the active workflow turn\""
            }),
        )
        .await
        .expect("meta run_command should return a structured failure");

    assert!(!result.ok, "{:?}", result.payload);
    assert!(
        result
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| {
                message.contains("currently executing workflow run")
                    && message.contains("normal metaagent prompts cannot steer")
            }),
        "{:?}",
        result.payload
    );
    let session_state = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load");
    let active_prompt = session_state
        .active_prompt_for_agent(worker.id())
        .expect("workflow prompt should remain active");
    assert_eq!(active_prompt.workflow_run_id(), Some(workflow_run.id()));
    assert_eq!(
        session_state
            .queued_prompts_for_agent(worker.id())
            .map(|queued| queued.len())
            .unwrap_or_default(),
        0,
        "metaagent workflow steering failures must not queue detached prompts"
    );
}

#[test]
fn metaagent_prompt_command_does_not_queue_over_workflow_turns() {
    run_large_stack_async_test(
        "metaagent-prompt-queued-workflow-guard",
        metaagent_prompt_command_does_not_queue_over_workflow_turns_inner,
    );
}

async fn metaagent_prompt_command_does_not_queue_over_workflow_turns_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-queued-workflow-prompt-guard");
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
    let workflow_prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        crate::scheduler::runtime::workflow_prompt_source_attachment_id("workflow-run-queued"),
        worker.id(),
        "workflow node prompt".to_string(),
        crate::session::PromptStatus::Queued,
    )
    .with_workflow_context("workflow-run-queued", "workflow-node-run-queued");
    app.prompt_owner_submit_prepared_prompt(session.id(), workflow_prompt, true)
        .expect("workflow prompt should remain queued");

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
                "command": "prompt worker \"start this instead\""
            }),
        )
        .await
        .expect("meta run_command should return a structured failure");

    assert!(!result.ok, "{:?}", result.payload);
    assert!(
        result
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| {
                message.contains("already has queued workflow run")
                    && message.contains("normal metaagent prompts cannot be queued")
            }),
        "{:?}",
        result.payload
    );
    let session_state = app
        .lock()
        .await
        .sessions()
        .get_session(session.id())
        .expect("session should load");
    let queued_prompts = session_state
        .queued_prompts_for_agent(worker.id())
        .expect("workflow prompt should remain queued");
    assert_eq!(queued_prompts.len(), 1);
    assert_eq!(
        queued_prompts[0].workflow_run_id(),
        Some("workflow-run-queued")
    );
}

#[test]
fn metaagent_run_command_routes_core_workflow_commands() {
    run_large_stack_async_test(
        "metaagent-run-command-workflow",
        metaagent_run_command_routes_core_workflow_commands_inner,
    );
}

async fn metaagent_run_command_routes_core_workflow_commands_inner() {
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
    let worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("worker")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("worker should spawn");
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

    let help = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow new --help"
            }),
        )
        .await
        .expect("workflow help-like aliases should return a structured usage error");
    assert!(!help.ok);
    assert!(
        help.payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("usage: workflow new [alias]")),
        "{:?}",
        help.payload
    );

    let node_added = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow node add meta-flow worker"
            }),
        )
        .await
        .expect("workflow node add command should dispatch");
    assert!(node_added.ok, "{:?}", node_added.payload);
    assert!(
        serde_json::to_string(&node_added.payload)
            .expect("payload should serialize")
            .contains(worker.id()),
        "{:?}",
        node_added.payload
    );
    let node_id = node_added
        .payload
        .pointer("/response/node/id")
        .and_then(serde_json::Value::as_str)
        .expect("node add response should include the node id")
        .to_string();

    let endpoint_created = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow endpoint new meta-flow {node_id} default")
            }),
        )
        .await
        .expect("workflow endpoint new command should dispatch");
    assert!(endpoint_created.ok, "{:?}", endpoint_created.payload);
    assert!(
        endpoint_created
            .payload
            .pointer("/response/endpoint/alias")
            .and_then(serde_json::Value::as_str)
            == Some("default"),
        "{:?}",
        endpoint_created.payload
    );

    let meta_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow node add meta-flow meta"
            }),
        )
        .await
        .expect("metaagent node add should return structured denial");
    assert!(!meta_node.ok);
    assert!(
        meta_node
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        meta_node.payload
    );

    let peer_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow node add meta-flow {}", peer_worker.agent_ref())
            }),
        )
        .await
        .expect("peer node add should return structured denial");
    assert!(!peer_node.ok);
    assert!(
        peer_node
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_node.payload
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
