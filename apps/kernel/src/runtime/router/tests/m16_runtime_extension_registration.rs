use super::*;

static CAPABILITY_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestCapabilityEnv {
    root: std::path::PathBuf,
    previous_isolation_root: Option<std::ffi::OsString>,
    previous_arroba_home: Option<std::ffi::OsString>,
}

impl TestCapabilityEnv {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "arroba-m16-runtime-extension-{label}-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        std::fs::create_dir_all(&root).expect("test capability root should be created");
        let previous_isolation_root = std::env::var_os("ARROBA_CAPABILITY_ISOLATION_ROOT");
        let previous_arroba_home = std::env::var_os("ARROBA_HOME");
        std::env::set_var("ARROBA_CAPABILITY_ISOLATION_ROOT", &root);
        std::env::set_var("ARROBA_HOME", root.join("arroba-home"));
        Self {
            root,
            previous_isolation_root,
            previous_arroba_home,
        }
    }
}

impl Drop for TestCapabilityEnv {
    fn drop(&mut self) {
        match &self.previous_isolation_root {
            Some(value) => std::env::set_var("ARROBA_CAPABILITY_ISOLATION_ROOT", value),
            None => std::env::remove_var("ARROBA_CAPABILITY_ISOLATION_ROOT"),
        }
        match &self.previous_arroba_home {
            Some(value) => std::env::set_var("ARROBA_HOME", value),
            None => std::env::remove_var("ARROBA_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn yolo_agent_registers_global_mcp_and_can_grant_it_in_same_provider_session() {
    let _env_lock = CAPABILITY_ENV_LOCK.lock().await;
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
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
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
    let expected_path = env
        .root
        .join("user")
        .join("mcps")
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
}

#[tokio::test]
async fn required_permission_agent_gets_registration_approval_before_path_validation() {
    let _env_lock = CAPABILITY_ENV_LOCK.lock().await;
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
    let missing_source = "missing-skill";

    let registration = tokio::spawn(async move {
        runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &auth_token,
                crate::transport::runtime_tools::REGISTER_SKILL_PATH_TOOL,
                serde_json::json!({ "path": missing_source }),
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
        !env.root
            .join("user")
            .join("skills")
            .join(missing_source)
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
