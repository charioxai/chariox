use super::*;

#[tokio::test]
async fn mcp_tools_list_exposes_slice_tools_only_for_slice_provider_tokens() {
    let mut config = DaemonConfig::for_tests();
    config.host_machine_id = "slice:slice-test".to_string();
    config.user_config.providers.workspace_live_sync.mode =
        crate::config::WorkspaceLiveSyncMode::Tracked;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should exist");
    let agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree("worktree-1"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    app.invoke_workflow_endpoint_and_schedule(
        session.id(),
        &workflow_id,
        "entry",
        Some("start".to_string()),
    )
    .expect("workflow should invoke");
    let auth_token = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("provider run should exist")
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let response = handle_json_rpc_value(
        router,
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await
    .expect("tools/list should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("tools list body should collect")
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("tools list body json");
    let tools = value["result"]["tools"]
        .as_array()
        .expect("tools should be an array");
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_screenshot"));
    assert!(tools.iter().any(|tool| tool["name"] == "slice_screenshot"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_find_text"));
    assert!(tools.iter().any(|tool| tool["name"] == "slice_mouse"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_browser_status"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "slice_browser_status"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_browser_wait_for_text"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "slice_browser_wait_for_idle"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "chariox.slice_browser_dialog"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "slice_browser_dialog"));
}

#[cfg(unix)]
#[tokio::test]
async fn mcp_tools_call_dispatches_slice_screen_status_inside_slice_kernel() {
    use std::os::unix::fs::PermissionsExt;

    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-slice-mcp-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let tool = root.join("slice-screen.sh");
    std::fs::write(
        &tool,
        "#!/usr/bin/env bash\nset -euo pipefail\nif [[ \"${1:-}\" == status ]]; then printf 'display=:99\\nscreen=1280x800\\nviewer=http://127.0.0.1:6080/vnc.html\\n'; exit 0; fi\nexit 2\n",
    )
    .expect("fake screen tool should be written");
    let mut permissions = std::fs::metadata(&tool)
        .expect("fake tool metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tool, permissions).expect("fake tool should be executable");
    std::env::set_var("CHARIOX_SLICE_SCREEN_TOOL", &tool);

    let mut config = DaemonConfig::for_tests();
    config.host_machine_id = "slice:slice-test".to_string();
    config.user_config.providers.workspace_live_sync.mode =
        crate::config::WorkspaceLiveSyncMode::Tracked;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should exist");
    let agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree("worktree-1"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    app.invoke_workflow_endpoint_and_schedule(
        session.id(),
        &workflow_id,
        "entry",
        Some("start".to_string()),
    )
    .expect("workflow should invoke");
    let auth_token = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("provider run should exist")
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let response = handle_json_rpc_value(
        router,
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "slice_screen_status",
                "arguments": {}
            }
        }),
    )
    .await
    .expect("slice status call should succeed");
    std::env::remove_var("CHARIOX_SLICE_SCREEN_TOOL");
    let _ = std::fs::remove_dir_all(root);
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("slice status body should collect")
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("slice status body json");
    assert_eq!(value["result"]["isError"], false);
    assert_eq!(
        value["result"]["structuredContent"]["slice_id"],
        "slice-test"
    );
    assert_eq!(value["result"]["structuredContent"]["display"], ":99");
    assert_eq!(value["result"]["structuredContent"]["screen"], "1280x800");
}
