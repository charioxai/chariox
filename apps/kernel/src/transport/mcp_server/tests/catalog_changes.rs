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

#[test]
fn authenticated_mcp_initialize_does_not_wait_for_catalog_workers() {
    let worktree = crate::test_support::TestWorktree::new("mcp-initialize-with-busy-catalog");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
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
    let token = run.runtime_mcp_auth_token().unwrap().to_owned();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            started_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(std::time::Duration::from_secs(5));
        });
        started_rx.await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(super::super::run_mcp_http_server_on_listener(router, listener));

        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#;
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket.write_all(format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        ).as_bytes()).await.unwrap();
        let mut response = String::new();
        let received = tokio::time::timeout(
            std::time::Duration::from_millis(750), socket.read_to_string(&mut response),
        ).await;
        release_tx.send(()).unwrap();
        worker.await.unwrap();
        server.abort();
        received.expect("initialize should not wait for catalog workers").unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("chariox-runtime"));
    });
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
    assert_eq!(
        crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
        crate::test_support::expected_local_protocol_version()
    );
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
    let monitor = super::super::catalog::CatalogMonitor::new();
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

async fn unauthenticated_status(address: std::net::SocketAddr) -> String {
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket
        .write_all(b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
        .await
        .unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).await.unwrap();
    response.lines().next().unwrap_or_default().to_string()
}

// A provider's first connection is answered while the catalog monitor waits on
// a busy provider store: the accept loop never queues behind a catalog refresh.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_connection_is_answered_while_a_catalog_refresh_waits() {
    let root = std::env::temp_dir().join(format!(
        "chariox-mcpstart-{}-{}",
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
    let providers = app.providers().clone();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(super::super::run_mcp_http_server_on_listener(
        router.clone(),
        listener,
    ));
    let ready = std::time::Duration::from_secs(5);
    let status = tokio::time::timeout(ready, unauthenticated_status(address)).await;
    assert_eq!(status.as_deref(), Ok("HTTP/1.1 401 Unauthorized"));

    let (held, holding) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let _busy = providers.write();
        held.send(()).unwrap();
        let _ = released.recv_timeout(std::time::Duration::from_secs(10));
    });
    holding.recv().unwrap();
    super::super::catalog_changed();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let started = std::time::Instant::now();
    let status = tokio::time::timeout(ready, unauthenticated_status(address)).await;
    let waited = started.elapsed();
    release.send(()).unwrap();
    holder.join().unwrap();
    server.abort();
    assert_eq!(
        status.as_deref(),
        Ok("HTTP/1.1 401 Unauthorized"),
        "a new MCP connection must be answered while a catalog refresh waits (waited {waited:?})"
    );
    assert!(
        waited < std::time::Duration::from_secs(1),
        "a new MCP connection waited {waited:?} behind a catalog refresh"
    );
}

async fn open_catalog_stream(address: std::net::SocketAddr, token: &str) -> tokio::net::TcpStream {
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(format!("GET /mcp HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nAuthorization: Bearer {token}\r\n\r\n").as_bytes()).await.unwrap();
    let headers = receive(&mut socket, "connected").await;
    assert!(headers.starts_with("HTTP/1.1 200"), "{headers}");
    socket
}

// Open notification streams (one per running agent on a busy kernel) never
// compute catalogs on the async runtime: a change hint while the provider
// store is busy must not stall the kernel's other connections.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_catalog_streams_do_not_stall_the_kernel_on_a_catalog_change() {
    let root = std::env::temp_dir().join(format!(
        "chariox-mcpcatalog-{}-{}",
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
    let providers = app.providers().clone();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        8,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(super::super::run_mcp_http_server_on_listener(
        router.clone(),
        listener,
    ));
    let _streams = [
        open_catalog_stream(address, &token).await,
        open_catalog_stream(address, &token).await,
        open_catalog_stream(address, &token).await,
    ];
    // Let the monitor's first refresh finish before the store turns busy.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let (held, holding) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let _busy = providers.write();
        held.send(()).unwrap();
        let _ = released.recv_timeout(std::time::Duration::from_secs(10));
    });
    holding.recv().unwrap();
    super::super::catalog_changed();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let started = std::time::Instant::now();
    let status = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        unauthenticated_status(address),
    )
    .await;
    let waited = started.elapsed();
    // A stalled runtime outlives the holder's own 10 s bound.
    let _ = release.send(());
    holder.join().unwrap();
    server.abort();
    assert_eq!(
        status.as_deref(),
        Ok("HTTP/1.1 401 Unauthorized"),
        "the kernel must answer while open streams see a catalog change (waited {waited:?})"
    );
    assert!(
        waited < std::time::Duration::from_secs(1),
        "a new MCP connection waited {waited:?} behind the open streams' catalog work"
    );
}
