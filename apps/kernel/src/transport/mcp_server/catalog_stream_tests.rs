//! Real HTTP/SSE fixtures; no provider process, external service or browser.
use super::*;
use crate::{
    app::DaemonApp,
    provider::{LaunchProviderRequest, RuntimeMcpBinding},
};
use std::time::Duration;

#[test]
fn runtime_origin_rejects_lookalike_hosts_and_opaque_origins() {
    for origin in [
        "http://localhost.evil.test",
        "https://127.0.0.1.evil.test",
        "http://localhost@evil.test",
        "null",
        "file:///local",
        "http://localhost/?redirect=1",
    ] {
        assert!(!valid_runtime_origin(origin), "accepted {origin}");
    }
    for origin in [
        "http://localhost:4812",
        "https://127.0.0.1",
        "http://[::1]:4812",
    ] {
        assert!(valid_runtime_origin(origin), "rejected {origin}");
    }
}

async fn fixture() -> (
    Arc<CommandRouter>,
    Arc<tokio::sync::Mutex<DaemonApp>>,
    std::net::SocketAddr,
    tokio::task::JoinHandle<()>,
) {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    for token in ["catalog-token-a", "catalog-token-b"] {
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "catalog-workspace",
                "catalog-worktree",
            ))
            .unwrap();
        let run = app
            .providers_mut()
            .launch_run_detached(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "default",
                )
                .with_agent_id(agent.id())
                .with_runtime_mcp_binding(RuntimeMcpBinding::new("http://127.0.0.1/mcp", token)),
            )
            .unwrap();
        app.update_provider_run_projection(run);
    }
    let app = Arc::new(tokio::sync::Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 2));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_router = router.clone();
    let server = tokio::spawn(async move {
        let _ = run_mcp_http_server_on_listener(server_router, listener).await;
    });
    (router, app, address, server)
}

async fn request(
    address: std::net::SocketAddr,
    method: Method,
    token: &str,
    body: &'static str,
) -> (Response<Incoming>, tokio::task::JoinHandle<()>) {
    let io = TokioIo::new(tokio::net::TcpStream::connect(address).await.unwrap());
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io).await.unwrap();
    let task = tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender
        .send_request(
            Request::builder()
                .method(method)
                .uri("/mcp")
                .header("Host", "127.0.0.1")
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header("Accept", "application/json, text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from_static(body.as_bytes())))
                .unwrap(),
        )
        .await
        .unwrap();
    (response, task)
}
async fn notification(body: &mut Incoming) -> Bytes {
    tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap()
}

#[tokio::test]
async fn authenticated_stream_is_scoped_and_fresh_tools_list_observes_only_its_generation() {
    let (router, app, address, server) = fixture().await;
    let (unauthorized, bad_connection) = request(address, Method::GET, "wrong-token", "").await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    bad_connection.abort();
    let (mut first, first_connection) = request(address, Method::GET, "catalog-token-a", "").await;
    let (mut second, second_connection) =
        request(address, Method::GET, "catalog-token-b", "").await;
    assert_eq!(first.headers()[CONTENT_TYPE], "text/event-stream");
    assert!(
        String::from_utf8_lossy(&notification(first.body_mut()).await)
            .contains("notifications/tools/list_changed")
    );
    notification(second.body_mut()).await;
    let run = router.runtime_mcp_catalog_run("catalog-token-a").unwrap();
    let mut refresh = router
        .runtime_mcp_catalog_changes()
        .begin_refresh(run.id())
        .unwrap();
    notification(first.body_mut()).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(25), second.body_mut().frame())
            .await
            .is_err()
    );
    let (listed, listed_connection) = request(
        address,
        Method::POST,
        "catalog-token-a",
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
    )
    .await;
    assert_eq!(listed.status(), StatusCode::OK);
    let list: Value =
        serde_json::from_slice(&listed.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(list.pointer("/result/tools").unwrap().is_array());
    tokio::time::timeout(Duration::from_secs(2), refresh.wait_until_observed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        router
            .runtime_mcp_catalog_run("catalog-token-a")
            .unwrap()
            .id(),
        run.id()
    );
    listed_connection.abort();
    {
        let mut app = app.lock().await;
        let ended = app
            .providers_mut()
            .terminate_run_provider_only(run.session_id(), run.id())
            .unwrap()
            .into_run();
        app.update_provider_run_projection(ended);
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(2), first.body_mut().frame())
            .await
            .unwrap()
            .is_none()
    );
    assert!(refresh.wait_until_observed().await.is_err());
    server.abort();
    let _ = server.await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), second.body_mut().frame())
            .await
            .unwrap()
            .is_none()
    );
    first_connection.abort();
    second_connection.abort();
}

#[tokio::test]
async fn real_http_stream_limit_releases_capacity_after_disconnect() {
    let (_router, _app, address, server) = fixture().await;
    let mut connections = Vec::new();
    for _ in 0..4 {
        let (mut response, task) = request(address, Method::GET, "catalog-token-a", "").await;
        assert_eq!(response.status(), StatusCode::OK);
        notification(response.body_mut()).await;
        connections.push((response, task));
    }
    let (overflow, task) = request(address, Method::GET, "catalog-token-a", "").await;
    assert_eq!(overflow.status(), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
    let (response, task) = connections.pop().unwrap();
    drop(response);
    task.abort();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let (response, task) = request(address, Method::GET, "catalog-token-a", "").await;
        let admitted = response.status() == StatusCode::OK;
        drop(response);
        task.abort();
        if admitted {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "closed stream kept its reservation"
        );
        tokio::task::yield_now().await;
    }
    for (response, task) in connections {
        drop(response);
        task.abort();
    }
    server.abort();
}
