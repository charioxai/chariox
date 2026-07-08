use super::*;

#[test]
fn metaagent_workflow_run_commands_expose_execution_visibility() {
    run_large_stack_async_test(
        "metaagent-workflow-run-visibility",
        metaagent_workflow_run_commands_expose_execution_visibility_inner,
    );
}

async fn metaagent_workflow_run_commands_expose_execution_visibility_inner() {
    let env = TestMetaRuntimeEnv::new("workflow-run-visibility");
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
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("worker should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, worker.id(), metaagent.id());
    let reviewer = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("reviewer")
                .with_owner_user_id(metaagent.owner_user_id()),
        )
        .expect("reviewer should spawn");
    mark_test_agent_controlled_by_metaagent(&mut app, reviewer.id(), metaagent.id());
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
            serde_json::json!({ "command": "workflow new visible-flow" }),
        )
        .await
        .expect("workflow create command should dispatch");
    assert!(created.ok);

    let worker_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow node add visible-flow worker" }),
        )
        .await
        .expect("worker node add command should dispatch");
    assert!(worker_node.ok);
    let worker_node_id = worker_node
        .payload
        .pointer("/response/node/id")
        .and_then(serde_json::Value::as_str)
        .expect("worker node add response should include node id")
        .to_string();

    let reviewer_node = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "workflow node add visible-flow reviewer" }),
        )
        .await
        .expect("reviewer node add command should dispatch");
    assert!(reviewer_node.ok);
    let reviewer_node_id = reviewer_node
        .payload
        .pointer("/response/node/id")
        .and_then(serde_json::Value::as_str)
        .expect("reviewer node add response should include node id")
        .to_string();

    let edge_added = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow edge add visible-flow {worker_node_id} {reviewer_node_id}")
            }),
        )
        .await
        .expect("workflow edge add command should dispatch");
    assert!(edge_added.ok);
    assert_eq!(
        edge_added
            .payload
            .pointer("/response/type")
            .and_then(serde_json::Value::as_str),
        Some("WorkflowEdgeAdded")
    );
    assert_eq!(
        edge_added
            .payload
            .pointer("/response/workflow/edges/0/from_node_id")
            .and_then(serde_json::Value::as_str),
        Some(worker_node_id.as_str())
    );

    let endpoint = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": format!("workflow endpoint new visible-flow {worker_node_id} default")
            }),
        )
        .await
        .expect("workflow endpoint new command should dispatch");
    assert!(endpoint.ok);

    let invoked = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "workflow run visible-flow default implement the requested change"
            }),
        )
        .await
        .expect("workflow run command should dispatch");
    assert!(invoked.ok);
    assert_eq!(
        invoked
            .payload
            .pointer("/response/type")
            .and_then(serde_json::Value::as_str),
        Some("WorkflowRunInvoked")
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/node_runs/0/node_id")
            .and_then(serde_json::Value::as_str),
        Some(worker_node_id.as_str())
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/active_node_run/node_id")
            .and_then(serde_json::Value::as_str),
        Some(worker_node_id.as_str())
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/active_node_run/turn/state")
            .and_then(serde_json::Value::as_str),
        Some("dispatched")
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/message_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/unconsumed_message_count")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
    assert_eq!(
        invoked
            .payload
            .pointer("/response/workflow_run/final_output_present")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    let run_id = invoked
        .payload
        .pointer("/response/workflow_run/id")
        .and_then(serde_json::Value::as_str)
        .expect("workflow run response should include id")
        .to_string();
    drop(invoked);

    let run_status = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": format!("workflow get-run {run_id}") }),
        )
        .await
        .expect("workflow get-run command should dispatch");
    assert!(run_status.ok);
    assert_eq!(
        run_status
            .payload
            .pointer("/response/type")
            .and_then(serde_json::Value::as_str),
        Some("WorkflowRun")
    );
    assert!(run_status
        .payload
        .pointer("/response/workflow_run/node_run_counts_by_status")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|counts| !counts.is_empty()));
}

#[test]
fn metaagent_run_command_routes_owned_agent_lifecycle_commands() {
    run_large_stack_async_test(
        "metaagent-run-command-agent-lifecycle",
        metaagent_run_command_routes_owned_agent_lifecycle_commands_inner,
    );
}

#[test]
fn metaagent_run_command_requires_plan_before_delegation() {
    run_large_stack_async_test(
        "metaagent-run-command-requires-plan",
        metaagent_run_command_requires_plan_before_delegation_inner,
    );
}

async fn metaagent_run_command_requires_plan_before_delegation_inner() {
    let env = TestMetaRuntimeEnv::new("run-command-requires-plan");
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
    let task_id = app
        .sessions_mut()
        .ensure_metaagent_task(session.id(), metaagent.id(), "Fix the bug by delegation.")
        .expect("metaagent task should start")
        .metaagent_task(metaagent.id())
        .expect("metaagent task should be projected")
        .task_id()
        .to_string();
    app.agents_mut()
        .activate_agent_meta_mode(metaagent.id(), Some(task_id))
        .expect("agent should enter meta mode");
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
            serde_json::json!({ "command": "agent spawn fixer" }),
        )
        .await
        .expect("planless delegation should return structured denial");
    assert!(!denied.ok);
    assert!(
        denied
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("call `arroba.meta.update_plan`")),
        "{:?}",
        denied.payload
    );

    let plan = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_UPDATE_PLAN_TOOL,
            serde_json::json!({
                "markdown": "- Spawn a fixer worker.\n- Review the fix and verification."
            }),
        )
        .await
        .expect("update_plan should dispatch");
    assert!(plan.ok, "{:?}", plan.payload);

    let spawned = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn fixer" }),
        )
        .await
        .expect("planned delegation should dispatch");
    assert!(spawned.ok, "{:?}", spawned.payload);
}

async fn metaagent_run_command_routes_owned_agent_lifecycle_commands_inner() {
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

#[test]
fn metaagent_run_command_allows_agent_slice_placement_but_denies_slice_management_policy() {
    run_large_stack_async_test(
        "metaagent-slice-placement-policy",
        metaagent_run_command_allows_agent_slice_placement_but_denies_slice_management_policy_inner,
    );
}

async fn metaagent_run_command_allows_agent_slice_placement_but_denies_slice_management_policy_inner(
) {
    let env = TestMetaRuntimeEnv::new("run-command-slice-policy");
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
    let daemon_id = app.config().daemon_id.clone();
    let host_machine_id = app.config().host_machine_id.clone();
    app.slices()
        .create(
            &daemon_id,
            &host_machine_id,
            crate::slice::CreateSliceInput {
                name: "linux-dev".to_string(),
                backend: crate::slice::SliceBackendKind::LocalDocker,
                os: "linux".to_string(),
                display_mode: crate::slice::SliceDisplayMode::Headless,
                workspace_id: Some(session.workspace_id().to_string()),
                worktree_id: Some(session.worktree_id().to_string()),
                workspace_mount: Some(session.worktree_id().to_string()),
                worker_kernel_ref: Some(daemon_id.clone()),
                display_url: None,
                provider_auth: Vec::new(),
                from_saved_state: None,
                now_ms: crate::session::unix_epoch_ms(),
            },
        )
        .expect("test slice should be seeded");
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let slice_placement = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({
                "command": "agent spawn helper --slice linux-dev"
            }),
        )
        .await
        .expect("slice-backed helper spawn should dispatch");
    assert!(slice_placement.ok, "{:?}", slice_placement.payload);

    let slice_list = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "slice list" }),
        )
        .await
        .expect("slice list should return a structured denial");
    assert!(!slice_list.ok, "{:?}", slice_list.payload);
    assert!(
        slice_list
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| {
                message.contains("cannot manage slices") && message.contains("regular agents")
            }),
        "{:?}",
        slice_list.payload
    );

    let reset_state = router
        .dispatch_authenticated_runtime_tool_call(
            &meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "slice reset-state linux-dev" }),
        )
        .await
        .expect("unrouted slice command should return a structured denial");
    assert!(!reset_state.ok);
    assert!(
        reset_state
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("cannot manage slices")),
        "{:?}",
        reset_state.payload
    );
}

#[test]
fn collaborator_metaagents_are_allowed_and_controller_scoped() {
    run_large_stack_async_test(
        "collaborator-metaagent-scope",
        collaborator_metaagents_are_allowed_and_controller_scoped_inner,
    );
}

async fn collaborator_metaagents_are_allowed_and_controller_scoped_inner() {
    let env = TestMetaRuntimeEnv::new("collaborator-metaagent-scope");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let (_, invite) = app
        .sessions_mut()
        .create_session_invite(
            &session_id,
            "invite-metaagent-collaborator".to_string(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
            Some(1),
            crate::session::CollaborationLevel::Private,
        )
        .expect("invite should be created");
    app.sessions_mut()
        .join_session_invite(&session_id, invite.invite_id(), "user-2".to_string(), 1)
        .expect("collaborator should join session");
    assert!(app
        .sessions()
        .get_session(&session_id)
        .expect("session should remain")
        .has_member("user-2"));

    let owner_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(&session_id, "dev-stub").with_alias("owner-meta"))
        .expect("owner metaagent should spawn");
    let owner_metaagent = activate_test_agent_meta_mode(&mut app, owner_metaagent);
    let peer_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer-meta")
                .with_owner_user_id("user-2"),
        )
        .expect("peer metaagent should spawn");
    let peer_metaagent = activate_test_agent_meta_mode(&mut app, peer_metaagent);
    let owner_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("owner-worker")
                .with_controlled_by_metaagent_id(owner_metaagent.id()),
        )
        .expect("owner worker should spawn");
    let peer_worker = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer-worker")
                .with_owner_user_id("user-2")
                .with_controlled_by_metaagent_id(peer_metaagent.id()),
        )
        .expect("peer worker should spawn");

    let owner_second_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(&session_id, "dev-stub").with_alias("owner-meta-2"))
        .expect("owner should be allowed to create a second metaagent");
    let owner_second_metaagent = activate_test_agent_meta_mode(&mut app, owner_second_metaagent);
    let peer_second_metaagent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("peer-meta-2")
                .with_owner_user_id("user-2"),
        )
        .expect("collaborator should be allowed to create a second metaagent");
    let peer_second_metaagent = activate_test_agent_meta_mode(&mut app, peer_second_metaagent);
    assert!(owner_second_metaagent.is_metaagent());
    assert!(peer_second_metaagent.is_metaagent());

    let owner_meta_run = launch_test_provider(
        &mut app,
        &session_id,
        owner_metaagent.id(),
        "dev-stub",
        "dev-stub",
        "owner-meta-model",
    );
    let peer_meta_run = launch_test_provider(
        &mut app,
        &session_id,
        peer_metaagent.id(),
        "dev-stub",
        "dev-stub",
        "peer-meta-model",
    );
    let owner_meta_auth_token = owner_meta_run
        .runtime_mcp_auth_token()
        .expect("owner meta run should expose runtime MCP auth token")
        .to_string();
    let peer_meta_auth_token = peer_meta_run
        .runtime_mcp_auth_token()
        .expect("peer meta run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let owner_alias = router
        .dispatch_authenticated_runtime_tool_call(
            &owner_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias owner-worker owner-renamed" }),
        )
        .await
        .expect("owner metaagent should dispatch owned alias command");
    assert!(owner_alias.ok, "{:?}", owner_alias.payload);
    let owner_peer_denial = router
        .dispatch_authenticated_runtime_tool_call(
            &owner_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias peer-worker owner-takeover" }),
        )
        .await
        .expect("owner metaagent peer alias should return structured denial");
    assert!(!owner_peer_denial.ok);
    assert!(
        owner_peer_denial
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        owner_peer_denial.payload
    );

    let peer_alias = router
        .dispatch_authenticated_runtime_tool_call(
            &peer_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias peer-worker peer-renamed" }),
        )
        .await
        .expect("peer metaagent should dispatch owned alias command");
    assert!(peer_alias.ok, "{:?}", peer_alias.payload);
    let peer_owner_denial = router
        .dispatch_authenticated_runtime_tool_call(
            &peer_meta_auth_token,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent alias owner-renamed peer-takeover" }),
        )
        .await
        .expect("peer metaagent owner alias should return structured denial");
    assert!(!peer_owner_denial.ok);
    assert!(
        peer_owner_denial
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("not an owned regular agent")),
        "{:?}",
        peer_owner_denial.payload
    );

    let app = app.lock().await;
    let owner_worker = app
        .agents()
        .get_agent(owner_worker.id())
        .expect("owner worker should remain");
    let peer_worker = app
        .agents()
        .get_agent(peer_worker.id())
        .expect("peer worker should remain");
    assert_eq!(owner_worker.alias(), Some("owner-renamed"));
    assert_eq!(peer_worker.alias(), Some("peer-renamed"));
}
