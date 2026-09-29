use super::*;

const CLAUDE_TOKEN: &str = "claude-print-permission-token";
const NATIVE_TUI_TOKEN: &str = "claude-native-tui-permission-token";

fn insert_claude_run(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    run_id: &str,
    token: &str,
    client_interface: crate::provider::ProviderClientInterface,
) {
    let request = crate::provider::LaunchProviderRequest::new(
        session_id, "claude", "claude", "default", "sonnet",
    )
    .with_agent_id(agent_id)
    .with_client_interface(client_interface)
    .with_execution_mode(crate::provider::AgentExecutionMode::Build)
    .with_permission_level(crate::provider::AgentPermissionLevel::Required)
    .with_runtime_mcp_binding(crate::provider::RuntimeMcpBinding::new(
        "http://127.0.0.1:1/mcp",
        token,
    ));
    let mut run = crate::provider::RuntimeProviderRun::new(
        run_id,
        &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::External,
            process_label: "claude:stream-json".to_string(),
            pty_target: None,
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: Default::default(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: Some("stdio://claude".to_string()),
        },
    );
    run.mark_running();
    app.providers_mut().insert_run_for_test(run);
}

async fn rpc(router: &Arc<CommandRouter>, token: &str, method: &str, params: Value) -> Value {
    let response = handle_json_rpc_value(
        router.clone(),
        token,
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
    )
    .await
    .expect("MCP request should return a JSON-RPC response");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("MCP body should collect")
        .to_bytes();
    serde_json::from_slice(&body).expect("MCP body should be JSON")
}

async fn tool_names(router: &Arc<CommandRouter>, token: &str) -> Vec<String> {
    rpc(router, token, "tools/list", serde_json::json!({})).await["result"]["tools"]
        .as_array()
        .expect("tools should be an array")
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect()
}

async fn prompt_and_answer(
    router: &Arc<CommandRouter>,
    session_id: &str,
    agent_id: &str,
    tool_use_id: &str,
    choice_id: &str,
) -> Value {
    let call_router = router.clone();
    let tool_use = tool_use_id.to_string();
    let call = tokio::spawn(async move {
        rpc(
            &call_router,
            CLAUDE_TOKEN,
            "tools/call",
            serde_json::json!({
                "name": "chariox.permission_prompt",
                "arguments": {
                    "tool_name": "Bash",
                    "input": {"command": "touch approved.txt", "description": "Create a file"},
                    "tool_use_id": tool_use,
                }
            }),
        )
        .await
    });
    let interaction = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let session = router
                .runtime_state()
                .session_snapshot_projection(session_id, 0)
                .expect("session projection should remain available")
                .session;
            if let Some(interaction) = session
                .active_interactions()
                .iter()
                .find(|interaction| interaction.id().ends_with(tool_use_id))
            {
                break interaction.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("permission prompt should create a runtime interaction");
    assert_eq!(
        interaction.kind(),
        crate::session::RuntimeInteractionKind::Permission
    );
    assert_eq!(interaction.agent_id(), Some(agent_id));
    assert_eq!(interaction.title(), Some("Approve Claude Code Bash?"));
    assert!(interaction.message().contains("touch approved.txt"));
    router
        .runtime_state()
        .resolve_runtime_interaction(session_id, interaction.id(), choice_id, None)
        .await
        .expect("permission interaction should resolve");
    let response = call.await.expect("permission prompt call should join");
    assert_eq!(response["result"]["isError"], false, "{response:#}");
    let content = response["result"]["content"]
        .as_array()
        .expect("tool result should carry content");
    assert_eq!(content.len(), 1, "Claude expects a single text block");
    assert_eq!(content[0]["type"], "text");
    serde_json::from_str(
        content[0]["text"]
            .as_str()
            .expect("permission decision should be text"),
    )
    .expect("permission decision text should be JSON")
}

#[tokio::test]
async fn claude_print_permission_prompt_asks_the_user_and_maps_the_answer() {
    let root = std::env::temp_dir().join(format!(
        "chariox-permission-prompt-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .expect("session should be created");
    // Launch before inserting the fixture runs so the launch cannot retire them.
    let dev_stub = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(agent.id()),
        )
        .expect("dev-stub provider should launch");
    let dev_stub_token = dev_stub
        .runtime_mcp_auth_token()
        .expect("dev-stub should expose runtime MCP auth")
        .to_string();
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "provider-run-claude-print",
        CLAUDE_TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "provider-run-claude-native",
        NATIVE_TUI_TOKEN,
        crate::provider::ProviderClientInterface::NativeTui,
    );
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));

    let claude_tools = tool_names(&router, CLAUDE_TOKEN).await;
    assert!(
        claude_tools.contains(&"chariox.permission_prompt".to_string()),
        "{claude_tools:?}"
    );
    for token in [NATIVE_TUI_TOKEN, dev_stub_token.as_str(), "unknown-token"] {
        assert!(
            !tool_names(&router, token)
                .await
                .contains(&"chariox.permission_prompt".to_string()),
            "only the Claude print-mode run owning the token may discover the prompt tool"
        );
    }

    let allowed = prompt_and_answer(
        &router,
        session.id(),
        agent.id(),
        "toolu_allow",
        "allow_once",
    )
    .await;
    assert_eq!(
        allowed,
        serde_json::json!({
            "behavior": "allow",
            "updatedInput": {"command": "touch approved.txt", "description": "Create a file"},
        })
    );
    let denied = prompt_and_answer(&router, session.id(), agent.id(), "toolu_deny", "deny").await;
    assert_eq!(denied["behavior"], "deny");
    assert_eq!(denied["message"], "Denied through Chariox.");

    let arguments = serde_json::json!({
        "name": "chariox.permission_prompt",
        "arguments": {"tool_name": "Bash", "input": {"command": "touch denied.txt"}}
    });
    for token in [NATIVE_TUI_TOKEN, dev_stub_token.as_str()] {
        let response = rpc(&router, token, "tools/call", arguments.clone()).await;
        assert_eq!(response["result"]["isError"], true, "{response:#}");
    }
    let unknown = rpc(&router, "unknown-token", "tools/call", arguments).await;
    assert_eq!(unknown["error"]["code"], -32000, "{unknown:#}");
    assert!(router
        .runtime_state()
        .session_snapshot_projection(session.id(), 0)
        .expect("session projection should remain available")
        .session
        .active_interactions()
        .is_empty());
    let _ = std::fs::remove_dir_all(root);
}
