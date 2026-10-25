use super::*;

const CLAUDE_TOKEN: &str = "claude-print-permission-token";
const NATIVE_TUI_TOKEN: &str = "claude-native-tui-permission-token";
const PLAN_TOKEN: &str = "claude-print-plan-permission-token";
const YOLO_TOKEN: &str = "claude-print-yolo-permission-token";
const BROKEN_TOKEN: &str = "claude-print-broken-permission-token";
/// Where the fixture Claude runs work, as the session rules are stated for that directory.
const RUN_DIRECTORY: &str = "/work/app";

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
            working_directory: Some(std::path::PathBuf::from(RUN_DIRECTORY)),
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

/// Calls the prompt tool as Claude does for a `touch` command and waits for
/// its interaction.
async fn start_prompt(
    router: &Arc<CommandRouter>,
    token: &str,
    session_id: &str,
    tool_use_id: &str,
) -> (
    tokio::task::JoinHandle<Value>,
    crate::session::RuntimeInteraction,
) {
    start_tool_prompt(
        router,
        token,
        session_id,
        tool_use_id,
        "Bash",
        serde_json::json!({"command": "touch approved.txt", "description": "Create a file"}),
    )
    .await
}

async fn start_tool_prompt(
    router: &Arc<CommandRouter>,
    token: &str,
    session_id: &str,
    tool_use_id: &str,
    tool_name: &str,
    input: Value,
) -> (
    tokio::task::JoinHandle<Value>,
    crate::session::RuntimeInteraction,
) {
    let call_router = router.clone();
    let (token, tool_use, tool_name) = (
        token.to_string(),
        tool_use_id.to_string(),
        tool_name.to_string(),
    );
    let mut call = tokio::spawn(async move {
        rpc(
            &call_router,
            &token,
            "tools/call",
            serde_json::json!({
                "name": "chariox.permission_prompt",
                "arguments": {
                    "tool_name": tool_name,
                    "input": input,
                    "tool_use_id": tool_use,
                }
            }),
        )
        .await
    });
    let interaction = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if call.is_finished() {
                panic!(
                    "permission {tool_use_id} returned before publishing: {:?}",
                    (&mut call).await
                );
            }
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
        .create_session(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_agent_defaults(crate::session::SessionAgentDefaults {
                    execution_mode: Some(crate::provider::AgentExecutionMode::Build),
                    permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                    ..Default::default()
                }),
        )
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
    // Only the current print run owns a live turn; other fixture runs model
    // discovery/refusal without replacing that turn.
    let active_turns = app.active_turn_store();
    active_turns.start(crate::app::ActiveTurnState::new(
        session.id().into(),
        agent.id().into(),
        "fixture-print-turn".into(),
        "provider-run-claude-print".into(),
    ));
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

    // Once the originating turn ends, the same still-running print provider
    // must deny a delayed request without publishing an approval prompt.
    active_turns.clear("provider-run-claude-print");
    let late = rpc(
        &router,
        CLAUDE_TOKEN,
        "tools/call",
        serde_json::json!({
            "name": "chariox.permission_prompt",
            "arguments": {
                "tool_name": "Bash",
                "input": {"command": "touch late.txt"},
                "tool_use_id": "toolu_late",
            }
        }),
    )
    .await;
    assert_eq!(decision(&late)["behavior"], "deny");
    assert!(active_interactions(&router, session.id()).is_empty());

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
        .create_session(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_agent_defaults(crate::session::SessionAgentDefaults {
                    execution_mode: Some(crate::provider::AgentExecutionMode::Build),
                    permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                    ..Default::default()
                }),
        )
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
    // Only the current print run owns a live turn; other fixture runs model
    // discovery/refusal without replacing that turn.
    let active_turns = app.active_turn_store();
    active_turns.start(crate::app::ActiveTurnState::new(
        session.id().into(),
        agent.id().into(),
        "fixture-print-turn".into(),
        "provider-run-claude-print".into(),
    ));
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
    active_turns.start(crate::app::ActiveTurnState::new(
        session.id().into(),
        agent.id().into(),
        "fixture-broken-turn".into(),
        format!("provider-run-{}", "x".repeat(120)),
    ));
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
        .create_session(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_agent_defaults(crate::session::SessionAgentDefaults {
                    execution_mode: Some(crate::provider::AgentExecutionMode::Build),
                    permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                    ..Default::default()
                }),
        )
        .expect("session should be created");
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "provider-run-claude-print",
        CLAUDE_TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
    // Only the current print run owns a live turn; other fixture runs model
    // discovery/refusal without replacing that turn.
    let active_turns = app.active_turn_store();
    active_turns.start(crate::app::ActiveTurnState::new(
        session.id().into(),
        agent.id().into(),
        "fixture-print-turn".into(),
        "provider-run-claude-print".into(),
    ));
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

/// Claude is silent while any runtime tool call waits for a person, not only
/// the permission prompt: a popup here. The run's turn stall watchdog sees the
/// wait for as long as the call runs, and no longer once it returns.
#[tokio::test]
async fn claude_waiting_on_a_runtime_popup_is_not_a_turn_stall() {
    let root = std::env::temp_dir().join(format!(
        "chariox-runtime-tool-wait-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_agent_defaults(crate::session::SessionAgentDefaults {
                    execution_mode: Some(crate::provider::AgentExecutionMode::Build),
                    permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                    ..Default::default()
                }),
        )
        .expect("session should be created");
    const RUN: &str = "provider-run-claude-print-popup-wait";
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        RUN,
        "claude-print-popup-wait-token",
        crate::provider::ProviderClientInterface::Chariox,
    );
    // Only the current print run owns a live turn; other fixture runs model
    // discovery/refusal without replacing that turn.
    let active_turns = app.active_turn_store();
    active_turns.start(crate::app::ActiveTurnState::new(
        session.id().into(),
        agent.id().into(),
        "fixture-print-turn".into(),
        RUN.into(),
    ));
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    assert!(!crate::provider::claude_runtime_tool_wait_pending(RUN));

    let call_router = router.clone();
    let call = tokio::spawn(async move {
        rpc(
            &call_router,
            "claude-print-popup-wait-token",
            "tools/call",
            serde_json::json!({
                "name": "chariox.request_popup",
                "arguments": {
                    "message": "Which channel?",
                    "timeout_sec": 120,
                    "choices": [
                        {"id": "stable", "label": "Stable", "reply": "stable"},
                        {"id": "beta", "label": "Beta", "reply": "beta"}
                    ]
                }
            }),
        )
        .await
    });
    let popup = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(interaction) = active_interactions(&router, session.id())
                .into_iter()
                .next()
            {
                break interaction;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the popup should open");
    assert!(
        crate::provider::claude_runtime_tool_wait_pending(RUN),
        "the run waits on the popup, so its watchdog must see activity"
    );

    router
        .runtime_state()
        .resolve_runtime_interaction(session.id(), popup.id(), "beta", None)
        .await
        .expect("popup should resolve");
    let response = call.await.expect("popup call should join");
    assert!(
        response["result"]["content"].is_array(),
        "popup call should answer: {response:#}"
    );
    assert!(!crate::provider::claude_runtime_tool_wait_pending(RUN));
    let _ = std::fs::remove_dir_all(root);
}

fn choice_ids(interaction: &crate::session::RuntimeInteraction) -> Vec<&str> {
    interaction
        .choices()
        .iter()
        .map(|choice| choice.id())
        .collect()
}

/// A router with one session whose agent runs the fixture Claude print run, and the directory
/// that holds the session's files.
fn claude_print_fixture() -> (
    Arc<CommandRouter>,
    crate::session::RuntimeSession,
    std::path::PathBuf,
) {
    let (router, session, _agent_id, root, _app) = claude_print_fixture_with_app();
    (router, session, root)
}

fn claude_print_fixture_with_app() -> (
    Arc<CommandRouter>,
    crate::session::RuntimeSession,
    String,
    std::path::PathBuf,
    Arc<Mutex<DaemonApp>>,
) {
    let root = std::env::temp_dir().join(format!(
        "chariox-permission-prompt-session-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(
            CreateSessionRequest::new(root.to_string_lossy(), root.to_string_lossy())
                .with_agent_defaults(crate::session::SessionAgentDefaults {
                    execution_mode: Some(crate::provider::AgentExecutionMode::Build),
                    permission_level: Some(crate::provider::AgentPermissionLevel::Required),
                    ..Default::default()
                }),
        )
        .expect("session should be created");
    // Creating the primary agent copies defaults into overrides; exercise inheritance here.
    app.agents_mut()
        .update_agent_config(agent.id(), Some(None), Some(None), None, None)
        .expect("the fixture agent should inherit session defaults");
    insert_claude_run(
        &mut app,
        session.id(),
        agent.id(),
        "provider-run-claude-print",
        CLAUDE_TOKEN,
        crate::provider::ProviderClientInterface::Chariox,
    );
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session.id().into(),
            agent.id().into(),
            "fixture-print-turn".into(),
            "provider-run-claude-print".into(),
        ));
    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 8));
    (router, session, agent.id().to_string(), root, app)
}

#[tokio::test]
async fn claude_print_permission_prompt_offers_allow_for_session_only_with_a_rule() {
    let (router, session, root) = claude_print_fixture();
    let answer = |call: tokio::task::JoinHandle<Value>| async move {
        decision(&call.await.expect("permission prompt call should join"))
    };
    let choose = |interaction: crate::session::RuntimeInteraction, choice: &'static str| {
        let router = router.clone();
        let session_id = session.id().to_string();
        async move {
            router
                .runtime_state()
                .resolve_runtime_interaction(&session_id, interaction.id(), choice, None)
                .await
        }
    };

    // A compound build command: the choice shows the exact rule and answers with that rule for
    // this run only.
    let compound = "cd /work/app/src && cargo test -p kernel --lib x 2>&1 | tail -40";
    let (call, interaction) = start_tool_prompt(
        &router,
        CLAUDE_TOKEN,
        session.id(),
        "toolu_cmd",
        "Bash",
        serde_json::json!({"command": compound}),
    )
    .await;
    assert_eq!(
        choice_ids(&interaction),
        ["allow_once", "allow_session", "deny"]
    );
    assert_eq!(
        interaction.choices()[1].label(),
        "Allow for this session: Bash(cargo test:*)"
    );
    assert!(
        interaction
            .message()
            .contains("Allow for this session grants: Bash(cargo test:*)"),
        "{}",
        interaction.message()
    );
    choose(interaction, "allow_session")
        .await
        .expect("allow_session should resolve");
    assert_eq!(
        answer(call).await,
        serde_json::json!({
            "behavior": "allow",
            "updatedInput": {"command": compound},
            "updatedPermissions": [{
                "type": "addRules",
                "behavior": "allow",
                "destination": "session",
                "rules": [{"toolName": "Bash", "ruleContent": "cargo test:*"}],
            }],
        })
    );

    // A file write grants the session's working directory and keeps credential names asking.
    let (call, interaction) = start_tool_prompt(
        &router,
        CLAUDE_TOKEN,
        session.id(),
        "toolu_write",
        "Write",
        serde_json::json!({"file_path": "/work/app/src/c.txt", "content": "x"}),
    )
    .await;
    assert_eq!(
        interaction.choices()[1].label(),
        "Allow for this session: Edit(//work/app/**)"
    );
    choose(interaction, "allow_session")
        .await
        .expect("allow_session should resolve");
    let granted = answer(call).await;
    let additions = granted["updatedPermissions"].as_array().expect("additions");
    assert_eq!(
        additions[0]["rules"],
        serde_json::json!([{"toolName": "Edit", "ruleContent": "//work/app/**"}])
    );
    assert_eq!(additions[1]["behavior"], "ask");
    assert!(additions
        .iter()
        .all(|addition| addition["destination"] == "session"));

    // A credential file is granted by itself, so the directory's credential asks stay as they are.
    let (call, interaction) = start_tool_prompt(
        &router,
        CLAUDE_TOKEN,
        session.id(),
        "toolu_env",
        "Write",
        serde_json::json!({"file_path": "/work/app/.env"}),
    )
    .await;
    assert_eq!(
        interaction.choices()[1].label(),
        "Allow for this session: Edit(//work/app/.env)"
    );
    choose(interaction, "allow_session")
        .await
        .expect("allow_session should resolve");
    let granted = answer(call).await;
    let additions = granted["updatedPermissions"].as_array().expect("additions");
    assert_eq!(additions.len(), 1);
    assert_eq!(
        additions[0]["rules"],
        serde_json::json!([{"toolName": "Edit", "ruleContent": "//work/app/.env"}])
    );

    // No rule is stated for a command outside the prefixes or a file outside the working
    // directory: two buttons, and the third cannot be chosen.
    for (id, tool, input) in [
        (
            "toolu_outside_prefix",
            "Bash",
            serde_json::json!({"command": "cargo test && git push"}),
        ),
        (
            "toolu_outside",
            "Write",
            serde_json::json!({"file_path": "/etc/hosts"}),
        ),
    ] {
        let (call, interaction) =
            start_tool_prompt(&router, CLAUDE_TOKEN, session.id(), id, tool, input).await;
        assert_eq!(choice_ids(&interaction), ["allow_once", "deny"], "{id}");
        assert!(
            choose(interaction.clone(), "allow_session").await.is_err(),
            "{id}"
        );
        choose(interaction, "allow_once")
            .await
            .expect("allow_once should resolve");
        let allowed = answer(call).await;
        assert_eq!(allowed["behavior"], "allow", "{id}");
        assert!(allowed.get("updatedPermissions").is_none(), "{allowed:#}");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn an_allow_for_session_grant_outlives_the_claude_process() {
    let (router, session, root) = claude_print_fixture();
    let call_now = |id: &str, tool: &'static str, input: Value| {
        let router = router.clone();
        let session_id = session.id().to_string();
        let id = id.to_string();
        async move {
            let response = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                rpc(
                    &router,
                    CLAUDE_TOKEN,
                    "tools/call",
                    serde_json::json!({
                        "name": "chariox.permission_prompt",
                        "arguments": {"tool_name": tool, "input": input, "tool_use_id": id}
                    }),
                ),
            )
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "{id} should be answered without a prompt: {:?}",
                    active_interactions(&router, &session_id)
                        .iter()
                        .map(|interaction| interaction.id().to_string())
                        .collect::<Vec<_>>()
                )
            });
            decision(&response)
        }
    };

    // Turn 1 of the first Claude process: the person grants `cargo test` and the work directory.
    let (call, interaction) = start_tool_prompt(
        &router,
        CLAUDE_TOKEN,
        session.id(),
        "toolu_first",
        "Bash",
        serde_json::json!({"command": "cd /work/app && cargo test 2>&1 | tail -5"}),
    )
    .await;
    router
        .runtime_state()
        .resolve_runtime_interaction(session.id(), interaction.id(), "allow_session", None)
        .await
        .expect("allow_session should resolve");
    decision(&call.await.expect("first prompt should join"));
    let (call, interaction) = start_tool_prompt(
        &router,
        CLAUDE_TOKEN,
        session.id(),
        "toolu_first_write",
        "Write",
        serde_json::json!({"file_path": "/work/app/a.rs", "content": "x"}),
    )
    .await;
    router
        .runtime_state()
        .resolve_runtime_interaction(session.id(), interaction.id(), "allow_session", None)
        .await
        .expect("allow_session should resolve");
    decision(&call.await.expect("first write should join"));

    // Claude was cancelled, restarted or replaced: the new process asks about the same kind of
    // call. The kernel answers at once and gives the new process every rule again.
    let again = call_now(
        "toolu_second",
        "Bash",
        serde_json::json!({"command": "cd /work/app/src && cargo test -p other"}),
    )
    .await;
    assert_eq!(again["behavior"], "allow");
    let additions = again["updatedPermissions"].as_array().expect("additions");
    let rules: Vec<&Value> = additions
        .iter()
        .filter(|addition| addition["behavior"] == "allow")
        .flat_map(|addition| addition["rules"].as_array().expect("rules"))
        .collect();
    assert_eq!(
        rules,
        [
            &serde_json::json!({"toolName": "Bash", "ruleContent": "cargo test:*"}),
            &serde_json::json!({"toolName": "Edit", "ruleContent": "//work/app/**"}),
        ]
    );
    assert!(additions
        .iter()
        .all(|addition| addition["destination"] == "session"));
    assert_eq!(
        call_now(
            "toolu_second_write",
            "Edit",
            serde_json::json!({"file_path": "/work/app/src/b.rs", "old_string": "a", "new_string": "b"}),
        )
        .await["behavior"],
        "allow"
    );
    // Files whose names only look like credentials are ordinary files in the granted directory.
    for round in 0..2 {
        for (id, file) in [
            ("credentials_rs", "/work/app/src/credentials.rs"),
            ("env_example", "/work/app/.env.example"),
            ("secrets_rs", "/work/app/src/secrets.rs"),
        ] {
            assert_eq!(
                call_now(
                    &format!("toolu_{id}_{round}"),
                    "Edit",
                    serde_json::json!({"file_path": file, "old_string": "a", "new_string": "b"}),
                )
                .await["behavior"],
                "allow",
                "{file} round {round}"
            );
        }
    }
    assert!(active_interactions(&router, session.id()).is_empty());

    // What the person did not grant still asks: another command, a credential file.
    for (id, tool, input) in [
        (
            "toolu_push",
            "Bash",
            serde_json::json!({"command": "cargo test && git push"}),
        ),
        (
            "toolu_check",
            "Bash",
            serde_json::json!({"command": "cargo check"}),
        ),
        (
            "toolu_env",
            "Write",
            serde_json::json!({"file_path": "/work/app/.env", "content": "x"}),
        ),
    ] {
        let (call, interaction) =
            start_tool_prompt(&router, CLAUDE_TOKEN, session.id(), id, tool, input).await;
        router
            .runtime_state()
            .resolve_runtime_interaction(session.id(), interaction.id(), "deny", None)
            .await
            .expect("deny should resolve");
        assert_eq!(
            decision(&call.await.expect("prompt joins"))["behavior"],
            "deny",
            "{id}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// Grants `cargo test` for the session through the real prompt and answer.
async fn grant_cargo_test(router: &Arc<CommandRouter>, session_id: &str, tool_use_id: &str) {
    let (call, interaction) = start_tool_prompt(
        router,
        CLAUDE_TOKEN,
        session_id,
        tool_use_id,
        "Bash",
        serde_json::json!({"command": "cargo test -p x"}),
    )
    .await;
    router
        .runtime_state()
        .resolve_runtime_interaction(session_id, interaction.id(), "allow_session", None)
        .await
        .expect("allow_session should resolve");
    decision(&call.await.expect("grant prompt should join"));
}

/// Whether `cargo test` is answered without asking the person.
async fn cargo_test_is_covered(
    router: &Arc<CommandRouter>,
    session_id: &str,
    tool_use_id: &str,
) -> bool {
    let call = rpc(
        router,
        CLAUDE_TOKEN,
        "tools/call",
        serde_json::json!({
            "name": "chariox.permission_prompt",
            "arguments": {
                "tool_name": "Bash",
                "input": {"command": "cargo test -p x"},
                "tool_use_id": tool_use_id,
            }
        }),
    );
    tokio::pin!(call);
    loop {
        tokio::select! {
            response = &mut call => {
                assert_eq!(decision(&response)["behavior"], "allow");
                return true;
            }
            _ = tokio::task::yield_now() => {
                if let Some(interaction) = active_interactions(router, session_id)
                    .into_iter()
                    .find(|interaction| interaction.id().ends_with(tool_use_id))
                {
                    router
                        .runtime_state()
                        .resolve_runtime_interaction(session_id, interaction.id(), "deny", None)
                        .await
                        .expect("deny should resolve");
                    call.await;
                    return false;
                }
            }
        }
    }
}

/// Starts the next turn, as the person's next prompt does. A config change ends the Claude
/// process of a mode it cannot change in place, so a new one stands in for it then.
async fn next_turn(app: &Arc<Mutex<DaemonApp>>, session_id: &str, agent_id: &str, turn: usize) {
    let mut app = app.lock().await;
    let live = app
        .providers()
        .get_run_for_agent(session_id, agent_id)
        .filter(|run| run.state() != crate::provider::ProviderRunState::Ended);
    let run_id = match live {
        Some(run) => run.id().to_string(),
        None => {
            let run_id = format!("provider-run-claude-print-{turn}");
            insert_claude_run(
                &mut app,
                session_id,
                agent_id,
                &run_id,
                CLAUDE_TOKEN,
                crate::provider::ProviderClientInterface::Chariox,
            );
            run_id
        }
    };
    app.active_turn_store()
        .start(crate::app::ActiveTurnState::new(
            session_id.into(),
            agent_id.into(),
            format!("fixture-print-turn-{turn}"),
            run_id,
        ));
}

/// Ends the turn of the fixture run, so the agent can be configured.
fn end_turns(app: &DaemonApp) {
    let session_runs = app.active_turn_store().snapshot();
    for run_id in session_runs.keys() {
        app.active_turn_store().clear(run_id);
    }
}

// MP-08 / MP-09 / MP-10 / MP-11: defaults never implicitly cancel inherited work.
#[tokio::test]
async fn session_defaults_change_preserves_active_inherited_claude_turn() {
    use crate::local::{LocalDaemonRequest, UpdateSessionConfigRequest};
    for (key, value) in [("agents.mode", "plan"), ("agents.permissions", "yolo")] {
        let (router, session, agent_id, root, app) = claude_print_fixture_with_app();
        let attachment = crate::app::KernelSessionService::new(&mut *app.lock().await)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "active-defaults",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        {
            let mut guard = app.lock().await;
            end_turns(&guard);
            let admitted = guard
                .prompt_owner_submit_prepared_prompt(
                    session.id(),
                    crate::session::PromptQueueItem::new(
                        "fixture-print-turn",
                        attachment.id(),
                        &agent_id,
                        "keep doing the active work",
                        crate::session::PromptStatus::Queued,
                    ),
                    false,
                )
                .unwrap();
            assert!(matches!(
                admitted,
                crate::session::PromptSubmissionOutcome::Started { .. }
            ));
            guard
                .active_turn_store()
                .start(crate::app::ActiveTurnState::new(
                    session.id().into(),
                    agent_id.clone(),
                    "fixture-print-turn".into(),
                    "provider-run-claude-print".into(),
                ));
        }
        grant_cargo_test(&router, session.id(), "toolu_grant").await;
        let before = app
            .lock()
            .await
            .sessions()
            .get_session(session.id())
            .unwrap();
        let request = LocalDaemonRequest::UpdateSessionConfig(UpdateSessionConfigRequest {
            session_id: session.id().into(),
            attachment_id: attachment.id().into(),
            values: std::collections::BTreeMap::from([(key.into(), value.into())]),
            requires_idle: false,
        });
        let command = crate::runtime::command::KernelCommand::from_local_request(
            "active-defaults",
            None,
            None,
            &request,
        );
        let result = router.dispatch(command, request).await;
        assert!(matches!(result, Err(crate::DaemonError::ConfigChangeRejectedWhilePromptRunning { .. })),
            "an affected active turn must refuse before defaults or provider state change: {result:?}");
        let guard = app.lock().await;
        assert_eq!(
            guard
                .sessions()
                .get_session(session.id())
                .unwrap()
                .config_state(),
            before.config_state()
        );
        assert_eq!(
            guard
                .providers()
                .get_run_for_agent(session.id(), &agent_id)
                .unwrap()
                .state(),
            crate::provider::ProviderRunState::Running
        );
        assert_eq!(guard.active_turn_store().snapshot().len(), 1);
        drop(guard);
        assert!(
            cargo_test_is_covered(&router, session.id(), "toolu_after_refusal").await,
            "a refused defaults change must keep the current standing grant"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[tokio::test]
async fn session_defaults_change_allows_unaffected_active_claude_turn() {
    let (router, session, agent_id, root, app) = claude_print_fixture_with_app();
    let attachment = crate::app::KernelSessionService::new(&mut *app.lock().await)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "unaffected-defaults",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    {
        let mut guard = app.lock().await;
        end_turns(&guard);
        let admitted = guard
            .prompt_owner_submit_prepared_prompt(
                session.id(),
                crate::session::PromptQueueItem::new(
                    "fixture-print-turn",
                    attachment.id(),
                    &agent_id,
                    "keep doing the active work",
                    crate::session::PromptStatus::Queued,
                ),
                false,
            )
            .unwrap();
        assert!(matches!(
            admitted,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ));
        guard
            .active_turn_store()
            .start(crate::app::ActiveTurnState::new(
                session.id().into(),
                agent_id.clone(),
                "fixture-print-turn".into(),
                "provider-run-claude-print".into(),
            ));
    }
    app.lock()
        .await
        .agents_mut()
        .update_agent_config(
            &agent_id,
            Some(Some(crate::provider::AgentExecutionMode::Build)),
            Some(Some(crate::provider::AgentPermissionLevel::Required)),
            None,
            None,
        )
        .unwrap();
    for values in [
        std::collections::BTreeMap::from([("theme".into(), "compact".into())]),
        std::collections::BTreeMap::from([("agents.mode".into(), "build".into())]),
        std::collections::BTreeMap::from([
            ("agents.mode".into(), "plan".into()),
            ("agents.permissions".into(), "yolo".into()),
        ]),
    ] {
        router
            .runtime_state()
            .update_session_config(session.id(), attachment.id(), values, false)
            .await
            .unwrap();
        assert_eq!(
            app.lock()
                .await
                .providers()
                .get_run_for_agent(session.id(), &agent_id)
                .unwrap()
                .state(),
            crate::provider::ProviderRunState::Running
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// A person who changes what the agent may do has ended what they granted, even when they change
/// it back: Plan and Yolo runs never ask Claude about a tool, so only the change itself shows it.
#[tokio::test]
async fn an_allow_for_session_grant_ends_when_the_agent_config_changes_and_changes_back() {
    use crate::provider::{AgentExecutionMode, AgentPermissionLevel};
    let (router, session, agent_id, root, app) = claude_print_fixture_with_app();
    grant_cargo_test(&router, session.id(), "toolu_grant").await;
    assert!(cargo_test_is_covered(&router, session.id(), "toolu_covered").await);

    for (round, (mode, level)) in [
        (AgentExecutionMode::Plan, AgentPermissionLevel::Required),
        (AgentExecutionMode::Build, AgentPermissionLevel::Yolo),
    ]
    .into_iter()
    .enumerate()
    {
        end_turns(&*app.lock().await);
        for (execution_mode, permission_level) in [
            (Some(Some(mode)), Some(Some(level))),
            (Some(None), Some(None)),
        ] {
            router
                .runtime_state()
                .update_agent_config(
                    session.id(),
                    &agent_id,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    execution_mode,
                    permission_level,
                    None,
                    None,
                )
                .await
                .expect("the config should update");
        }
        next_turn(&app, session.id(), &agent_id, round + 1).await;
        assert!(
            !cargo_test_is_covered(&router, session.id(), &format!("toolu_after_{round}")).await,
            "round {round}"
        );
        grant_cargo_test(&router, session.id(), &format!("toolu_regrant_{round}")).await;
        assert!(
            cargo_test_is_covered(&router, session.id(), &format!("toolu_again_{round}")).await
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// The same holds when the session's own configuration decides the agent's mode.
#[tokio::test]
async fn an_allow_for_session_grant_ends_when_the_session_config_changes_and_changes_back() {
    let (router, session, agent_id, root, app) = claude_print_fixture_with_app();
    let attachment = crate::app::KernelSessionService::new(&mut *app.lock().await)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "cli-permission-prompt",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("attachment should attach");
    grant_cargo_test(&router, session.id(), "toolu_grant").await;

    for (round, (key, intermediate, original)) in [
        ("agents.mode", "plan", "build"),
        ("agents.permissions", "yolo", "required"),
    ]
    .into_iter()
    .enumerate()
    {
        end_turns(&*app.lock().await);
        for value in [intermediate, original] {
            router
                .runtime_state()
                .update_session_config(
                    session.id(),
                    attachment.id(),
                    std::collections::BTreeMap::from([(key.to_string(), value.to_string())]),
                    false,
                )
                .await
                .expect("the session config should update");
        }
        assert!(
            app.lock()
                .await
                .providers()
                .get_run_for_agent(session.id(), &agent_id)
                .is_none_or(|run| run.state() == crate::provider::ProviderRunState::Ended),
            "session config changes must also end rules held by the provider process"
        );
        next_turn(&app, session.id(), &agent_id, round + 1).await;
        assert!(
            !cargo_test_is_covered(&router, session.id(), &format!("toolu_after_{round}")).await
        );
        grant_cargo_test(&router, session.id(), &format!("toolu_regrant_{round}")).await;
    }
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn an_allow_for_session_grant_ends_when_the_working_directory_changes_back() {
    let (router, session, agent_id, root, app) = claude_print_fixture_with_app();
    grant_cargo_test(&router, session.id(), "toolu_grant").await;
    end_turns(&*app.lock().await);
    let original = app
        .lock()
        .await
        .agents()
        .get_agent(&agent_id)
        .unwrap()
        .worktree_id()
        .map(str::to_string);
    let other = root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    for worktree in [Some(other.to_string_lossy().into_owned()), original] {
        router
            .runtime_state()
            .update_agent_config(
                session.id(),
                &agent_id,
                crate::session::DEFAULT_LOCAL_USER_ID,
                None,
                None,
                None,
                Some(worktree),
            )
            .await
            .expect("working directory should update");
    }
    assert!(
        app.lock()
            .await
            .providers()
            .get_run_for_agent(session.id(), &agent_id)
            .is_none_or(|run| run.state() == crate::provider::ProviderRunState::Ended),
        "directory changes must also end rules held by the provider process"
    );
    next_turn(&app, session.id(), &agent_id, 1).await;
    assert!(!cargo_test_is_covered(&router, session.id(), "toolu_after_directory_change").await);
    let _ = std::fs::remove_dir_all(root);
}
