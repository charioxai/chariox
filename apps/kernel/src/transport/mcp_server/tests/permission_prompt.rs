use super::*;

const CLAUDE_TOKEN: &str = "claude-print-permission-token";
const NATIVE_TUI_TOKEN: &str = "claude-native-tui-permission-token";
const PLAN_TOKEN: &str = "claude-print-plan-permission-token";
const YOLO_TOKEN: &str = "claude-print-yolo-permission-token";
const BROKEN_TOKEN: &str = "claude-print-broken-permission-token";

fn insert_claude_run(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    run_id: &str,
    token: &str,
    client_interface: crate::provider::ProviderClientInterface,
) {
    insert_claude_run_in_mode(
        app,
        session_id,
        agent_id,
        run_id,
        token,
        client_interface,
        crate::provider::AgentExecutionMode::Build,
        crate::provider::AgentPermissionLevel::Required,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_claude_run_in_mode(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    run_id: &str,
    token: &str,
    client_interface: crate::provider::ProviderClientInterface,
    execution_mode: crate::provider::AgentExecutionMode,
    permission_level: crate::provider::AgentPermissionLevel,
) {
    let request = crate::provider::LaunchProviderRequest::new(
        session_id, "claude", "claude", "default", "sonnet",
    )
    .with_agent_id(agent_id)
    .with_client_interface(client_interface)
    .with_execution_mode(execution_mode)
    .with_permission_level(permission_level)
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

fn active_interactions(
    router: &Arc<CommandRouter>,
    session_id: &str,
) -> Vec<crate::session::RuntimeInteraction> {
    router
        .runtime_state()
        .session_snapshot_projection(session_id, 0)
        .expect("session projection should remain available")
        .session
        .active_interactions()
        .to_vec()
}

/// Calls the prompt tool as Claude does and waits for its interaction.
async fn start_prompt(
    router: &Arc<CommandRouter>,
    token: &str,
    session_id: &str,
    tool_use_id: &str,
) -> (
    tokio::task::JoinHandle<Value>,
    crate::session::RuntimeInteraction,
) {
    let call_router = router.clone();
    let (token, tool_use) = (token.to_string(), tool_use_id.to_string());
    let call = tokio::spawn(async move {
        rpc(
            &call_router,
            &token,
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
            if let Some(interaction) = active_interactions(router, session_id)
                .into_iter()
                .find(|interaction| interaction.id().ends_with(tool_use_id))
            {
                break interaction;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("permission prompt should create a runtime interaction");
    (call, interaction)
}

/// The documented `--permission-prompt-tool` result: exactly one text block
/// holding the JSON-stringified decision, and nothing else.
fn decision(response: &Value) -> Value {
    let result = response["result"]
        .as_object()
        .unwrap_or_else(|| panic!("tool call should succeed: {response:#}"));
    assert_eq!(
        result.keys().collect::<Vec<_>>(),
        vec!["content"],
        "{response:#}"
    );
    let content = result["content"]
        .as_array()
        .expect("tool result should carry content");
    assert_eq!(content.len(), 1, "Claude expects a single text block");
    assert_eq!(content[0].as_object().map(|block| block.len()), Some(2));
    assert_eq!(content[0]["type"], "text");
    serde_json::from_str(
        content[0]["text"]
            .as_str()
            .expect("permission decision should be text"),
    )
    .expect("permission decision text should be JSON")
}

async fn prompt_and_answer(
    router: &Arc<CommandRouter>,
    session_id: &str,
    agent_id: &str,
    tool_use_id: &str,
    choice_id: &str,
) -> Value {
    let (call, interaction) = start_prompt(router, CLAUDE_TOKEN, session_id, tool_use_id).await;
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
    decision(&call.await.expect("permission prompt call should join"))
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

#[tokio::test]
async fn claude_print_permission_prompt_fails_closed_and_closes_an_abandoned_prompt() {
    let root = std::env::temp_dir().join(format!(
        "chariox-permission-prompt-closed-{}-{}",
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
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "provider-run-claude-print",
        CLAUDE_TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
    // Print runs in Plan mode or at the Yolo level have no prompt tool flag.
    for (run_id, token, execution_mode, permission_level) in [
        (
            "provider-run-claude-plan",
            PLAN_TOKEN,
            crate::provider::AgentExecutionMode::Plan,
            crate::provider::AgentPermissionLevel::Required,
        ),
        (
            "provider-run-claude-yolo",
            YOLO_TOKEN,
            crate::provider::AgentExecutionMode::Build,
            crate::provider::AgentPermissionLevel::Yolo,
        ),
    ] {
        insert_claude_run_in_mode(
            &mut app,
            session.id(),
            agent.id(),
            run_id,
            token,
            crate::provider::ProviderClientInterface::Chariox,
            execution_mode,
            permission_level,
        );
    }
    // Its interaction id exceeds the store's limit, so the bridge fails.
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        &format!("provider-run-{}", "x".repeat(120)),
        BROKEN_TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let call = |tool_use_id: &str| {
        serde_json::json!({
            "name": "chariox.permission_prompt",
            "arguments": {
                "tool_name": "Bash",
                "input": {"command": "touch file.txt"},
                "tool_use_id": tool_use_id,
            }
        })
    };

    for token in [PLAN_TOKEN, YOLO_TOKEN] {
        assert!(!tool_names(&router, token)
            .await
            .contains(&"chariox.permission_prompt".to_string()));
        let response = rpc(&router, token, "tools/call", call("toolu_other_mode")).await;
        assert_eq!(response["result"]["isError"], true, "{response:#}");
    }
    // Without `input` the call is invalid, and Claude Code denies the tool.
    let missing = rpc(
        &router,
        CLAUDE_TOKEN,
        "tools/call",
        serde_json::json!({
            "name": "chariox.permission_prompt",
            "arguments": {"tool_name": "Bash"}
        }),
    )
    .await;
    assert_eq!(missing["error"]["code"], -32000, "{missing:#}");
    let null_input = rpc(
        &router,
        CLAUDE_TOKEN,
        "tools/call",
        serde_json::json!({
            "name": "chariox.permission_prompt",
            "arguments": {"tool_name": "Bash", "input": null}
        }),
    )
    .await;
    assert_eq!(null_input["error"]["code"], -32000, "{null_input:#}");

    // An unanswered prompt times out to its default Deny choice.
    let (pending, interaction) =
        start_prompt(&router, CLAUDE_TOKEN, session.id(), "toolu_timeout").await;
    router
        .runtime_state()
        .timeout_runtime_interaction(session.id(), interaction.id())
        .await
        .expect("permission interaction should time out");
    assert_eq!(
        decision(&pending.await.expect("permission prompt call should join")),
        serde_json::json!({"behavior": "deny", "message": "Denied through Chariox."})
    );

    // Claude abandons the call: dropping the handler future closes the prompt.
    let (abandoned, _interaction) =
        start_prompt(&router, CLAUDE_TOKEN, session.id(), "toolu_abandoned").await;
    abandoned.abort();
    let _ = abandoned.await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !active_interactions(&router, session.id()).is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("an abandoned permission prompt should close");

    // A bridge failure denies.
    let broken = rpc(&router, BROKEN_TOKEN, "tools/call", call("toolu_broken")).await;
    assert_eq!(
        decision(&broken),
        serde_json::json!({"behavior": "deny", "message": "Chariox permission bridge failed."})
    );
    assert!(active_interactions(&router, session.id()).is_empty());
    let _ = std::fs::remove_dir_all(root);
}

/// The production trigger of the abandoned-call cleanup: Claude closes its
/// HTTP connection while the prompt is open, and the runtime MCP server drops
/// the handler future.
#[tokio::test]
async fn claude_closing_the_connection_during_a_permission_prompt_closes_it() {
    use tokio::io::AsyncWriteExt;

    let root = std::env::temp_dir().join(format!(
        "chariox-permission-prompt-disconnect-{}-{}",
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
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "provider-run-claude-print",
        CLAUDE_TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("runtime MCP listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have an address");
    let server_router = router.clone();
    let server = tokio::spawn(async move {
        let _ =
            crate::transport::mcp_server::run_mcp_http_server_on_listener(server_router, listener)
                .await;
    });

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "chariox.permission_prompt",
            "arguments": {
                "tool_name": "Bash",
                "input": {"command": "touch file.txt"},
                "tool_use_id": "toolu_disconnect",
            }
        }
    })
    .to_string();
    let mut connection = tokio::net::TcpStream::connect(address)
        .await
        .expect("client should connect");
    connection
        .write_all(
            format!(
                "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {CLAUDE_TOKEN}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("request should be written");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !active_interactions(&router, session.id())
            .iter()
            .any(|interaction| interaction.id().ends_with("toolu_disconnect"))
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the prompt should open");

    drop(connection);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !active_interactions(&router, session.id()).is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("closing the connection should close the prompt");
    server.abort();
    let _ = std::fs::remove_dir_all(root);
}
