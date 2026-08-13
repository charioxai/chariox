use super::*;

#[tokio::test]
async fn denied_home_extension_invocation_is_audited() {
    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            "workspace-home-extension-denied",
            "worktree-home-extension-denied",
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "remote-extension-denied", "dev-stub");
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should be remote-backed");

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-1",
        "home-only",
        None,
    );
    let hinted_tool = crate::extension::RemoteExtensionTool {
        kind: crate::extension::ExtensionKind::Script,
        name: "home-only".to_string(),
        tool_name: "home-only".to_string(),
        description: "forged script hint".to_string(),
        input_schema: serde_json::json!({}),
        authority: crate::extension::ExtensionAuthority::Home,
        definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
        execution_location: crate::extension::ExtensionExecutionLocation::Home,
        safety: None,
        timeout_sec: None,
        version_hash: Some("forged-hash".to_string()),
    };
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("forged-worker".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };

    let denied = router
        .runtime_state
        .dispatch_forwarded_home_extension_tool_call(
            context,
            metadata,
            hinted_tool,
            serde_json::json!({}),
        )
        .await
        .expect_err("forged worker identity should be denied");
    assert!(
        denied
            .to_string()
            .contains("worker kernel does not match home agent binding"),
        "unexpected denial: {denied}"
    );

    let events = router
        .runtime_state
        .list_home_extension_audit_events(agent.id(), DEFAULT_LOCAL_USER_ID, 10)
        .expect("audit events should load");
    let denied_event = events
        .iter()
        .find(|event| event.kind == "home_extension.invoke.denied")
        .expect("denied invocation should be audited");
    assert_eq!(
        denied_event
            .payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str),
        Some("denied")
    );
    assert!(denied_event
        .payload
        .pointer("/error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("worker kernel does not match home agent binding")));
    assert_eq!(
        denied_event
            .payload
            .pointer("/home_user_id")
            .and_then(serde_json::Value::as_str),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        denied_event
            .payload
            .pointer("/caller_user_id")
            .and_then(serde_json::Value::as_str),
        Some(DEFAULT_LOCAL_USER_ID)
    );
    assert_eq!(
        denied_event
            .payload
            .pointer("/lease_id")
            .and_then(serde_json::Value::as_str),
        Some("lease-1")
    );
    assert_eq!(
        denied_event
            .payload
            .pointer("/worker_provider_run_id")
            .and_then(serde_json::Value::as_str),
        Some("provider-run-1")
    );
    assert!(
        denied_event
            .payload
            .pointer("/duration_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        "denied invocation should include duration"
    );
}

#[tokio::test]
async fn forwarded_home_mcp_rejects_forged_dispatch_name() {
    let workspace = std::env::temp_dir().join(format!(
        "chariox-home-mcp-forged-name-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let mcp_registry =
        crate::mcp::CharioxMcpRegistry::new(vec![workspace.join(".chariox").join("mcps")]);
    mcp_registry
        .install(&crate::mcp::CharioxMcpServerConfig::streamable_http(
            "home-mcp-a",
            "http://127.0.0.1:9/a",
        ))
        .expect("authorized MCP should be installed");
    mcp_registry
        .install(&crate::mcp::CharioxMcpServerConfig::streamable_http(
            "home-mcp-b",
            "http://127.0.0.1:9/b",
        ))
        .expect("unguarded MCP should be installed");

    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "remote-mcp-forged-name", "dev-stub");
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should be remote-backed");
    let granted_agent = app
        .agents()
        .grant_mcp(agent.id(), "home-mcp-a".to_string())
        .expect("authorized MCP grant should be recorded");
    let hinted_tool = app
        .remote_extension_manifest_for_agent(&granted_agent)
        .expect("home manifest should be rebuilt from current state")
        .home_proxy_tool("home-mcp-a")
        .expect("authorized MCP should be projected")
        .clone();

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-1",
        "home-mcp-b",
        None,
    );
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };

    let denied = router
        .runtime_state
        .dispatch_forwarded_home_mcp_proxy_call(
            context,
            metadata,
            "home-mcp-b".to_string(),
            hinted_tool,
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .await
        .expect_err("worker-supplied MCP name must not select a different home MCP");
    assert!(
        denied
            .to_string()
            .contains("does not match authorized home-proxy tool"),
        "unexpected denial: {denied}"
    );

    let events = router
        .runtime_state
        .list_home_extension_audit_events(agent.id(), DEFAULT_LOCAL_USER_ID, 10)
        .expect("audit events should load");
    assert!(events
        .iter()
        .any(|event| event.kind == "home_extension.invoke.denied"));

    let _ = std::fs::remove_dir_all(&workspace);
}

#[tokio::test]
async fn forwarded_home_extension_runtime_rejects_mcp_tools() {
    let workspace = std::env::temp_dir().join(format!(
        "chariox-home-extension-mcp-wrong-dispatch-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos()
    ));
    let mcp_registry =
        crate::mcp::CharioxMcpRegistry::new(vec![workspace.join(".chariox").join("mcps")]);
    mcp_registry
        .install(&crate::mcp::CharioxMcpServerConfig::streamable_http(
            "home-mcp-runtime-misroute",
            "http://127.0.0.1:9/mcp",
        ))
        .expect("authorized MCP should be installed");

    let config = DaemonConfig::for_tests();
    let home_kernel_id = config.daemon_id.clone();
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = spawn_test_agent(&mut app, &session_id, "remote-mcp-misroute", "dev-stub");
    app.agents()
        .bind_remote_execution(
            agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel".to_string(),
                worker_machine_id: "worker-machine".to_string(),
                execution_lease_id: "lease-1".to_string(),
                leased_agent_id: "leased-agent-1".to_string(),
                active_worker_provider_run_id: Some("provider-run-1".to_string()),
                relay_url: None,
                relay_token: None,
            },
        )
        .expect("agent should be remote-backed");
    let granted_agent = app
        .agents()
        .grant_mcp(agent.id(), "home-mcp-runtime-misroute".to_string())
        .expect("authorized MCP grant should be recorded");
    let hinted_tool = app
        .remote_extension_manifest_for_agent(&granted_agent)
        .expect("home manifest should be rebuilt from current state")
        .home_proxy_tool("home-mcp-runtime-misroute")
        .expect("authorized MCP should be projected")
        .clone();

    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
        "provider-run-1",
        "home-mcp-runtime-misroute",
        None,
    );
    let context = crate::transport::relay_peer::RemoteExtensionInvocationContext {
        home_kernel_id,
        home_session_id: session_id,
        home_agent_id: agent.id().to_string(),
        leased_agent_id: "leased-agent-1".to_string(),
        worker_provider_run_id: "provider-run-1".to_string(),
        worker_kernel_id: Some("worker-kernel".to_string()),
        worker_machine_id: Some("worker-machine".to_string()),
    };

    let denied = router
        .runtime_state
        .dispatch_forwarded_home_extension_tool_call(
            context,
            metadata,
            hinted_tool,
            serde_json::json!({}),
        )
        .await
        .expect_err("MCP tools must use the dedicated home MCP proxy path");
    assert!(
        denied
            .to_string()
            .contains("must use its dedicated dispatch path"),
        "unexpected denial: {denied}"
    );

    let events = router
        .runtime_state
        .list_home_extension_audit_events(agent.id(), DEFAULT_LOCAL_USER_ID, 10)
        .expect("audit events should load");
    let denied_event = events
        .iter()
        .find(|event| event.kind == "home_extension.invoke.denied")
        .expect("misrouted MCP invocation should be audited as denied");
    assert_eq!(
        denied_event
            .payload
            .pointer("/tool/kind")
            .and_then(serde_json::Value::as_str),
        Some("mcp")
    );
    assert!(denied_event
        .payload
        .pointer("/error")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|error| error.contains("dedicated dispatch path")));
    assert!(
        !events
            .iter()
            .any(|event| event.kind == "home_extension.invoke.accepted"),
        "misrouted MCP invocation must not be audited as accepted"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
