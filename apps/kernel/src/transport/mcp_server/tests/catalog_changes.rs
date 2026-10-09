//! MP-08/MP-10: real HTTP stream, real registry and agent grants.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Scratch(pub std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn receive(socket: &mut tokio::net::TcpStream, expected: &str) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut text = String::new();
        loop {
            let mut buffer = [0; 8192];
            let size = socket.read(&mut buffer).await.unwrap();
            assert!(
                size > 0,
                "MP-08/MP-10 stream closed before {expected}: {text}"
            );
            text.push_str(std::str::from_utf8(&buffer[..size]).unwrap());
            if text.contains(expected) {
                return text;
            }
        }
    })
    .await
    .expect("MP-08/MP-10 catalog notification must arrive promptly")
}

#[tokio::test]
async fn mcp_catalog_stream_tracks_grant_revoke_register_remove_and_scope() {
    let root = std::env::temp_dir().join(format!(
        "chariox-extfix-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = Scratch(root.clone());
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    let run = app
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
        .unwrap();
    let token = run.runtime_mcp_auth_token().unwrap().to_string();
    let outsider = app
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("outsider")
                .with_worktree(root.to_string_lossy()),
        )
        .unwrap();
    let agents = app.agents().clone();
    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(super::super::run_mcp_http_server_on_listener(
        router.clone(),
        listener,
    ));
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(format!("GET /mcp HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nAuthorization: Bearer {token}\r\n\r\n").as_bytes()).await.unwrap();
    let headers = receive(&mut socket, "connected").await;
    assert!(
        headers.starts_with("HTTP/1.1 200"),
        "MP-08/MP-10 MCP GET must stream: {headers}"
    );
    assert!(headers.contains("text/event-stream"));
    let environment = crate::script::CharioxEnvironmentConfig {
        name: "extfix_python".into(),
        runtime: crate::script::CharioxEnvironmentRuntime::Python {
            python: "/usr/bin/python3".into(),
        },
    };
    let registry = crate::script::CharioxScriptRegistry::new(vec![
        crate::script::CharioxScriptRegistry::project_root(&root),
    ]);
    let source = root.join("fixture.py");
    std::fs::write(&source, "def run() -> dict:\n    \"\"\"Return the extension result.\"\"\"\n    return {'marker': 'MP-08-MP-10-live'}\n\ndef test_run():\n    \"\"\"Validate the fixture.\"\"\"\n    assert run()['marker'] == 'MP-08-MP-10-live'\n").unwrap();
    registry
        .install(&source, Some("extfix_tool"), &environment)
        .unwrap();
    // Registration alone is not effective until this agent has a grant.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), socket.read_u8())
            .await
            .is_err()
    );
    agents
        .grant_extension(
            outsider.id(),
            crate::extension::ExtensionGrant::script("extfix_tool", "extfix_python"),
        )
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), socket.read_u8())
            .await
            .is_err()
    );
    agents
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script("extfix_tool", "extfix_python"),
        )
        .unwrap();
    let notification = receive(&mut socket, "notifications/tools/list_changed").await;
    assert!(
        !notification.contains("extfix_tool"),
        "notification must not disclose catalog contents"
    );
    assert!(router
        .runtime_tool_specs_for_auth_token(&token)
        .iter()
        .any(|tool| tool.name == "extfix_tool"));
    // Identical grants are not catalog changes.
    agents
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script("extfix_tool", "extfix_python"),
        )
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), socket.read_u8())
            .await
            .is_err()
    );
    agents
        .revoke_extension(
            agent.id(),
            crate::extension::ExtensionKind::Script,
            "extfix_tool",
        )
        .unwrap();
    receive(&mut socket, "notifications/tools/list_changed").await;
    agents
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script("extfix_tool", "extfix_python"),
        )
        .unwrap();
    receive(&mut socket, "notifications/tools/list_changed").await;
    registry.uninstall("extfix_tool").unwrap();
    receive(&mut socket, "notifications/tools/list_changed").await;
    registry
        .install(&source, Some("extfix_tool"), &environment)
        .unwrap();
    receive(&mut socket, "notifications/tools/list_changed").await;
    // Same-name schema/description replacement is an effective catalog change.
    registry.uninstall("extfix_tool").unwrap();
    receive(&mut socket, "notifications/tools/list_changed").await;
    std::fs::write(&source, "def run(value: str) -> str:\n    \"\"\"Return changed input.\"\"\"\n    return value\n\ndef test_run():\n    \"\"\"Validate changed fixture.\"\"\"\n    assert run('ok') == 'ok'\n").unwrap();
    registry
        .install(&source, Some("extfix_tool"), &environment)
        .unwrap();
    receive(&mut socket, "notifications/tools/list_changed").await;
    drop(socket);
    server.abort();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn mcp_catalog_post_notifies_before_result_and_negotiates_supported_version() {
    let response = super::super::catalog::tool_response(
        true,
        serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"isError":false}}),
    );
    assert_eq!(
        response.headers()[hyper::header::CONTENT_TYPE],
        "text/event-stream"
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(
        body.find("notifications/tools/list_changed").unwrap() < body.find("isError").unwrap(),
        "MP-08/MP-10 catalog notification precedes tool result"
    );
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(
            DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap(),
        )),
        8,
    ));
    let response = super::super::handle_json_rpc_value(router, "unused-token", serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28"}})).await.unwrap();
    let value: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(
        value["result"]["protocolVersion"], "2025-03-26",
        "MP-08/MP-10 unsupported protocol must negotiate an implemented version"
    );
    assert_eq!(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, 486);
}

// MP-08/MP-10: fallback discovery does not depend on a provider GET stream.
#[tokio::test]
async fn mcp_catalog_monitor_tracks_changes_without_get_stream() {
    let root = std::env::temp_dir().join(format!(
        "chariox-extfix-monitor-{}-{}",
        std::process::id(),
        crate::session::unix_epoch_ms()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = Scratch(root.clone());
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            root.to_string_lossy(),
            root.to_string_lossy(),
        ))
        .unwrap();
    app.launch_provider(
        crate::provider::LaunchProviderRequest::new(
            session.id(),
            "dev-stub",
            "dev-stub",
            "default",
            "default",
        )
        .with_agent_id(agent.id()),
    )
    .unwrap();
    let agents = app.agents().clone();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 8);
    let mut monitor = super::super::catalog::CatalogMonitor::new(&router);
    let environment = crate::script::CharioxEnvironmentConfig {
        name: "extfix_python".into(),
        runtime: crate::script::CharioxEnvironmentRuntime::Python {
            python: "/usr/bin/python3".into(),
        },
    };
    let registry = crate::script::CharioxScriptRegistry::new(vec![
        crate::script::CharioxScriptRegistry::project_root(&root),
    ]);
    let source = root.join("fixture.py");
    std::fs::write(&source, "def run() -> str:\n    \"\"\"Return a fixture result.\"\"\"\n    return 'ok'\n\ndef test_run():\n    \"\"\"Validate fixture.\"\"\"\n    assert run() == 'ok'\n").unwrap();
    registry
        .install(&source, Some("extfix_tool"), &environment)
        .unwrap();
    assert_eq!(monitor.refresh(&router), 0);
    agents
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::script("extfix_tool", "extfix_python"),
        )
        .unwrap();
    assert_eq!(
        monitor.refresh(&router),
        1,
        "MP-08/MP-10 grant must be detected without GET"
    );
    assert_eq!(monitor.refresh(&router), 0);
    registry.uninstall("extfix_tool").unwrap();
    assert_eq!(
        monitor.refresh(&router),
        1,
        "MP-08/MP-10 removal must be detected without GET"
    );
    registry
        .install(&source, Some("extfix_tool"), &environment)
        .unwrap();
    assert_eq!(monitor.refresh(&router), 1);
    agents
        .revoke_extension(
            agent.id(),
            crate::extension::ExtensionKind::Script,
            "extfix_tool",
        )
        .unwrap();
    assert_eq!(monitor.refresh(&router), 1);
}
