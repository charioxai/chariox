use super::*;

struct TestCapabilityEnv {
    root: std::path::PathBuf,
}

impl TestCapabilityEnv {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-m16-runtime-extension-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("test capability root should be created");
        Self { root }
    }
}

impl Drop for TestCapabilityEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn runtime_tools_reject_ambiguous_provider_run_tokens_for_run_scoped_tools() {
    let env = TestCapabilityEnv::new("ambiguous-token");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let provider_run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "m16-model",
    );
    let auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("provider run should expose runtime MCP auth token")
        .to_string();
    let server_url = provider_run
        .runtime_mcp_server_url()
        .expect("provider run should expose runtime MCP server URL")
        .to_string();
    let duplicate_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "m16-model-duplicate",
            )
            .with_agent_id(agent.id())
            .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                server_url,
                auth_token.clone(),
            )),
        )
        .expect("duplicate provider run should start")
        .into_run();
    app.update_provider_run_projection(duplicate_run.clone());
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let specs = router
        .runtime_state
        .runtime_tool_specs_for_auth_token(&auth_token);
    assert!(
        specs
            .iter()
            .any(|spec| spec.name == crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL),
        "workflow tools should remain visible for shared workflow auth"
    );
    assert!(
        specs
            .iter()
            .all(|spec| spec.name != crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL),
        "run-scoped extension tools should not be advertised for ambiguous provider tokens"
    );

    let result = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
            serde_json::json!({
                "kind": "mcp",
                "name": "filesystem"
            }),
        )
        .await
        .expect_err("run-scoped tool calls should reject ambiguous provider tokens");
    let message = result.to_string();
    assert!(
        message.contains("multiple active provider runs"),
        "{message}"
    );
    assert!(message.contains(provider_run.id()), "{message}");
    assert!(message.contains(duplicate_run.id()), "{message}");
    assert!(
        message.contains("run /kernel health and /provider processes"),
        "{message}"
    );
}

#[tokio::test]
async fn mcp_proxy_rejects_ambiguous_provider_run_tokens() {
    let env = TestCapabilityEnv::new("ambiguous-mcp-proxy-token");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let provider_run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "m16-model",
    );
    let auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("provider run should expose runtime MCP auth token")
        .to_string();
    let server_url = provider_run
        .runtime_mcp_server_url()
        .expect("provider run should expose runtime MCP server URL")
        .to_string();
    let duplicate_run = app
        .providers()
        .start_run_provider_only(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "m16-model-duplicate",
            )
            .with_agent_id(agent.id())
            .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
                server_url,
                auth_token.clone(),
            )),
        )
        .expect("duplicate provider run should start")
        .into_run();
    app.update_provider_run_projection(duplicate_run.clone());
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);

    let result = router
        .dispatch_authenticated_mcp_proxy_call(
            &auth_token,
            "status",
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list"
            }),
        )
        .await
        .expect_err("MCP proxy calls should reject ambiguous provider tokens");
    let message = result.to_string();
    assert!(
        message.contains("multiple active provider runs"),
        "{message}"
    );
    assert!(message.contains(provider_run.id()), "{message}");
    assert!(message.contains(duplicate_run.id()), "{message}");
    assert!(
        message.contains("run /kernel health and /provider processes"),
        "{message}"
    );
}

#[tokio::test]
async fn yolo_agent_registers_global_mcp_and_can_grant_it_in_same_provider_session() {
    let env = TestCapabilityEnv::new("yolo-mcp");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let provider_run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "m16-model",
    );
    let auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("provider run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let mcp_name = format!("m16-runtime-mcp-{}", crate::session::unix_epoch_ms());

    let registration = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::REGISTER_MCP_TOOL,
            serde_json::json!({
                "config": {
                    "name": mcp_name,
                    "transport": {
                        "type": "stdio",
                        "command": "/bin/echo",
                        "args": ["m16"]
                    }
                }
            }),
        )
        .await
        .expect("register_mcp should return a runtime tool result");

    assert!(registration.ok);
    assert_eq!(
        registration
            .payload
            .get("registered")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        registration
            .payload
            .get("granted")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    let expected_path = crate::mcp::CharioxMcpRegistry::user_root()
        .expect("HOME should resolve MCP registry root")
        .join(format!("{mcp_name}.json"));
    assert_eq!(
        registration
            .payload
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(std::path::PathBuf::from),
        Some(expected_path.clone())
    );
    assert!(expected_path.exists());

    let grant = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
            serde_json::json!({
                "kind": "mcp",
                "name": mcp_name
            }),
        )
        .await
        .expect("request_extension should grant the just-registered MCP");

    assert!(grant.ok);
    assert_eq!(
        grant
            .payload
            .get("granted")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        grant
            .payload
            .get("effective")
            .and_then(serde_json::Value::as_str),
        Some("after_provider_reload")
    );
    assert_eq!(
        grant
            .payload
            .get("requires_provider_restart")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let list = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::LIST_EXTENSIONS_TOOL,
            serde_json::json!({ "kind": "mcp" }),
        )
        .await
        .expect("list_extensions should return MCP readiness metadata");
    let listed_mcp = list
        .payload
        .pointer("/extensions/mcps")
        .and_then(serde_json::Value::as_array)
        .and_then(|mcps| {
            mcps.iter().find(|mcp| {
                mcp.get("name").and_then(serde_json::Value::as_str) == Some(mcp_name.as_str())
            })
        })
        .expect("registered MCP should be listed");
    assert_eq!(
        listed_mcp
            .get("effective_when_requested")
            .and_then(serde_json::Value::as_str),
        Some("after_provider_reload")
    );
    let audit_events = router
        .runtime_state
        .list_home_extension_audit_events(agent.id(), DEFAULT_LOCAL_USER_ID, 20)
        .expect("extension audit events should load");
    assert!(audit_events.iter().any(|event| {
        event.kind == "extension.registration.created"
            && event
                .payload
                .get("kind")
                .and_then(serde_json::Value::as_str)
                == Some("mcp")
            && event
                .payload
                .get("name")
                .and_then(serde_json::Value::as_str)
                == Some(mcp_name.as_str())
    }));
    assert!(audit_events.iter().any(|event| {
        event.kind == "home_extension.grant.created"
            && event
                .payload
                .pointer("/grant/kind")
                .and_then(serde_json::Value::as_str)
                == Some("mcp")
            && event
                .payload
                .pointer("/grant/name")
                .and_then(serde_json::Value::as_str)
                == Some(mcp_name.as_str())
    }));
    let _ = std::fs::remove_file(expected_path);
}

#[tokio::test]
async fn runtime_connector_request_rejects_missing_credential_before_grant() {
    let env = TestCapabilityEnv::new("connector-missing-credential");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let provider_run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "m16-model",
    );
    let auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("provider run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let connector_name = format!("m16-connector-{}", crate::session::unix_epoch_ms());
    let connector_root = crate::connector::CharioxConnectorRegistry::user_root()
        .expect("HOME should resolve connector registry root");
    std::fs::create_dir_all(&connector_root).expect("connector registry root should be created");
    let connector_path = connector_root.join(format!("{connector_name}.yaml"));
    std::fs::write(
        &connector_path,
        format!(
            r#"
kind: connector
name: {connector_name}
description: M16 connector
adapter: missing-adapter
operations:
  - name: lookup
    description: Lookup
    safety: read
    input_schema:
      type: object
    config: {{}}
"#
        ),
    )
    .expect("connector definition should be written");

    let result = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
            serde_json::json!({
                "kind": "connector",
                "name": connector_name,
                "credential": "missing-runtime-credential"
            }),
        )
        .await
        .expect_err("missing credential should fail before connector grant is persisted");

    assert!(result
        .to_string()
        .contains("credential `missing-runtime-credential` is not registered"));
    let agent_after = app
        .lock()
        .await
        .agents
        .get_agent(agent.id())
        .expect("agent should still exist");
    assert!(!agent_after
        .has_extension_grant(crate::extension::ExtensionKind::Connector, &connector_name));
    let _ = std::fs::remove_file(connector_path);
}

#[tokio::test]
async fn register_mcp_can_grant_to_current_agent_in_one_runtime_call() {
    let env = TestCapabilityEnv::new("register-grant-mcp");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let provider_run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "m16-model",
    );
    let auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("provider run should expose runtime MCP auth token")
        .to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let mcp_name = format!("m16-register-grant-mcp-{}", crate::session::unix_epoch_ms());

    let registration = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth_token,
            crate::transport::runtime_tools::REGISTER_MCP_TOOL,
            serde_json::json!({
                "grant_to_current_agent": true,
                "config": {
                    "name": mcp_name,
                    "transport": {
                        "type": "stdio",
                        "command": "/bin/echo",
                        "args": ["m16-register-grant"]
                    }
                }
            }),
        )
        .await
        .expect("register_mcp should support one-call registration and grant");

    assert!(registration.ok);
    assert_eq!(
        registration
            .payload
            .get("granted")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        registration
            .payload
            .pointer("/grant/effective")
            .and_then(serde_json::Value::as_str),
        Some("after_provider_reload")
    );
    let expected_path = crate::mcp::CharioxMcpRegistry::user_root()
        .expect("HOME should resolve MCP registry root")
        .join(format!("{mcp_name}.json"));
    let _ = std::fs::remove_file(expected_path);
}

#[tokio::test]
async fn required_permission_agent_gets_registration_approval_before_path_validation() {
    let env = TestCapabilityEnv::new("required-skill");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let session = app
        .sessions_mut()
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .expect("session should be created");
    let session_id = session.id().to_string();
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(&session_id, "dev-stub")
                .with_alias("m16-required")
                .with_permission_level_override(crate::provider::AgentPermissionLevel::Required),
        )
        .expect("required permission agent should be created");
    let provider_run = launch_test_provider(
        &mut app,
        &session_id,
        agent.id(),
        "dev-stub",
        "dev-stub",
        "m16-model",
    );
    let auth_token = provider_run
        .runtime_mcp_auth_token()
        .expect("provider run should expose runtime MCP auth token")
        .to_string();
    let app = Arc::new(Mutex::new(app));
    let router = CommandRouter::with_interactive_capacity(Arc::clone(&app), 4);
    let runtime_state = router.runtime_state.clone();
    let missing_source = format!("missing-skill-{}", crate::session::unix_epoch_ms());
    let missing_source_arg = missing_source.clone();

    let registration = tokio::spawn(async move {
        runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &auth_token,
                crate::transport::runtime_tools::REGISTER_SKILL_PATH_TOOL,
                serde_json::json!({ "path": missing_source_arg }),
            )
            .await
    });

    let interaction_id = wait_for_active_interaction(&app, &session_id, agent.id()).await;
    let response =
        LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
            session_id: session_id.clone(),
            interaction_id: interaction_id.clone(),
            choice_id: "deny".to_string(),
            custom_reply: None,
        });
    router
        .dispatch(
            KernelCommand::from_local_request("cmd-m16-deny-registration", None, None, &response),
            response,
        )
        .await
        .expect("interaction denial should be accepted");

    let result = timeout(Duration::from_secs(2), registration)
        .await
        .expect("registration task should finish after interaction denial")
        .expect("registration task should not panic")
        .expect("runtime tool dispatch should return a result");
    assert!(!result.ok);
    assert_eq!(
        result
            .payload
            .pointer("/reason/kind")
            .and_then(serde_json::Value::as_str),
        Some("permission_denied")
    );
    assert_eq!(
        result
            .payload
            .get("interaction_id")
            .and_then(serde_json::Value::as_str),
        Some(interaction_id.as_str())
    );
    assert!(
        !crate::skill::CharioxSkillRegistry::user_root()
            .expect("HOME should resolve skill registry root")
            .join(&missing_source)
            .exists(),
        "denied path-based registration must not install anything"
    );
}

async fn wait_for_active_interaction(
    app: &Arc<Mutex<DaemonApp>>,
    session_id: &str,
    agent_id: &str,
) -> String {
    timeout(Duration::from_secs(2), async {
        loop {
            if let Some(interaction_id) = {
                let app = app.lock().await;
                app.sessions()
                    .get_session(session_id)
                    .expect("session should exist")
                    .active_interaction_for_agent(agent_id)
                    .map(|interaction| interaction.id().to_string())
            } {
                return interaction_id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("registration approval interaction should be projected")
}
