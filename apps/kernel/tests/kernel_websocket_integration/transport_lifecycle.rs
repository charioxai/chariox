use crate::support::kernel_websocket::*;
use arroba_kernel::attachment::ClientCapabilityLevel;
use arroba_kernel::local::{
    AttachToSessionRequest, GetDaemonHealthRequest, GetSessionStateRequest, LocalDaemonRequest,
    RunShellCapabilityRequest,
};
use arroba_kernel::runtime_transport::{
    run_kernel_websocket_server_on_listener, CONNECTION_INBOUND_REQUEST_LIMIT,
    KERNEL_RUNTIME_THREAD_STACK_SIZE,
};
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_replies_to_client_ping() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.runtime_mcp_port = unused_tcp_port();
    let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            kernel_websocket_listener,
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
    socket
        .send(Message::Ping(Vec::new().into()))
        .await
        .expect("kernel websocket ping should send");

    let pong = timeout(Duration::from_millis(250), async {
        loop {
            let message = socket
                .next()
                .await
                .expect("kernel websocket should yield a frame")
                .expect("kernel websocket frame should decode");
            if matches!(message, Message::Pong(_)) {
                break;
            }
        }
    })
    .await;
    assert!(
        pong.is_ok(),
        "kernel websocket should reply to client pings"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_pongs_while_event_writer_is_delayed() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.runtime_mcp_port = unused_tcp_port();
    config.kernel_websocket_queue_capacity = 16;
    config.kernel_websocket_write_delay_ms = 800;
    let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            kernel_websocket_listener,
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
    let create_response = send_request(
        &mut socket,
        "create-session-before-delayed-ping",
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-delayed-pong",
            "worktree-delayed-pong",
        )),
    )
    .await;
    let session_id = response_variant(&create_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let attach_response = send_request(
        &mut socket,
        "attach-session-before-delayed-ping",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-delayed-pong-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    send_frame(
        &mut socket,
        json!({
            "type": "subscribe",
            "request_id": "subscribe-before-delayed-ping",
            "session_id": session_id,
            "attachment_id": attachment_id,
        }),
    )
    .await;
    let _subscribe_response = wait_for_response(&mut socket, "subscribe-before-delayed-ping").await;
    sleep(Duration::from_millis(100)).await;

    socket
        .send(Message::Ping(Vec::from("probe").into()))
        .await
        .expect("kernel websocket ping should send");
    let pong = timeout(Duration::from_millis(250), async {
        loop {
            let message = socket
                .next()
                .await
                .expect("kernel websocket should yield a frame")
                .expect("kernel websocket frame should decode");
            if matches!(message, Message::Pong(_)) {
                break;
            }
        }
    })
    .await;
    assert!(
        pong.is_ok(),
        "kernel websocket should answer pings while event output is delayed"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_detaches_terminal_attachment_on_connection_close() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.runtime_mcp_port = unused_tcp_port();
    let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            kernel_websocket_listener,
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
    let create_response = send_request(
        &mut socket,
        "create-session",
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-close-detach",
            "worktree-close-detach",
        )),
    )
    .await;
    let session_id = response_variant(&create_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-close-detach-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    send_frame(
        &mut socket,
        json!({
            "type": "subscribe",
            "request_id": "subscribe-session",
            "session_id": session_id.clone(),
            "attachment_id": attachment_id.clone(),
        }),
    )
    .await;
    let subscribe_response = wait_for_response(&mut socket, "subscribe-session").await;
    assert_eq!(subscribe_response["response"]["ok"].as_bool(), Some(true));

    socket
        .close(None)
        .await
        .expect("kernel websocket should close");
    sleep(Duration::from_millis(100)).await;

    let mut probe = connect_with_retry(&config.kernel_websocket_url()).await;
    let state_response = send_request(
        &mut probe,
        "session-state-after-close",
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        }),
    )
    .await;
    let attachment_ids =
        &response_variant(&state_response, "SessionState")["session"]["attachment_ids"];
    assert_eq!(
        attachment_ids.as_array().map(Vec::len),
        Some(0),
        "closed websocket attachment should not remain in session state: {state_response}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_closes_slow_consumers_when_the_outgoing_queue_overflows() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.runtime_mcp_port = unused_tcp_port();
    config.kernel_websocket_queue_capacity = 2;
    config.kernel_websocket_write_delay_ms = 400;
    let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server_on_listener(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            kernel_websocket_listener,
            async {
                let _ = shutdown_rx.await;
            },
        )
        .await
    });

    let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;

    let create_response = send_request(
        &mut socket,
        "create-session",
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-kernel-overflow",
            "worktree-kernel-overflow",
        )),
    )
    .await;
    let session_id = response_variant(&create_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-overflow-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    send_frame(
        &mut socket,
        json!({
            "type": "subscribe",
            "request_id": "subscribe-session",
            "session_id": session_id,
            "attachment_id": attachment_id,
        }),
    )
    .await;
    let _subscribe_response = wait_for_response(&mut socket, "subscribe-session").await;

    for index in 0..64 {
        send_frame(
            &mut socket,
            json!({
                "type": "request",
                "request_id": format!("state-{index}"),
                "request": LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                    session_id: session_id.clone(),
                }),
            }),
        )
        .await;
    }

    let close_frame = wait_for_close(&mut socket).await;
    assert_eq!(close_frame.0, Some(1008));
    assert_eq!(
        close_frame.1.as_deref(),
        Some("kernel transport overloaded; reconnecting")
    );

    let mut health_socket = connect_with_retry(&config.kernel_websocket_url()).await;
    let health = send_request(
        &mut health_socket,
        "daemon-health-after-slow-consumer",
        LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest),
    )
    .await;
    let transport = &response_variant(&health, "DaemonHealth")["projection"]["transport"];
    assert!(
        transport["outgoing_queue_overflows"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "transport health should report outgoing queue pressure: {health}"
    );
    assert!(
        transport["slow_consumer_closes"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "transport health should report slow-consumer closes: {health}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[test]
fn kernel_websocket_rejects_requests_when_inbound_admission_is_full() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .enable_all()
        .build()
        .expect("production-equivalent kernel runtime should build");
    runtime.block_on(async {
        let mut config = DaemonConfig::for_tests();
        let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
        config.kernel_websocket_port = kernel_websocket_port;
        config.runtime_mcp_port = unused_tcp_port();
        config.kernel_websocket_queue_capacity = 64;
        let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            run_kernel_websocket_server_on_listener(
                std::sync::Arc::new(tokio::sync::Mutex::new(app)),
                kernel_websocket_listener,
                async {
                    let _ = shutdown_rx.await;
                },
            )
            .await
        });

        let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let cwd = std::env::current_dir()
            .expect("current directory should be available")
            .to_string_lossy()
            .to_string();

        let create_response = send_request(
            &mut socket,
            "create-session",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                cwd.as_str(),
                cwd.as_str(),
            )),
        )
        .await;
        let session_id = response_variant(&create_response, "SessionCreated")["session"]["id"]
            .as_str()
            .expect("session id should be present")
            .to_string();

        let attach_response = send_request(
            &mut socket,
            "attach-session",
            LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: session_id.clone(),
                client_id: "ws-inbound-limit-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            }),
        )
        .await;
        let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]
            ["id"]
            .as_str()
            .expect("attachment id should be present")
            .to_string();

        for index in 0..=CONNECTION_INBOUND_REQUEST_LIMIT {
            send_frame(
                &mut socket,
                json!({
                    "type": "request",
                    "request_id": format!("slow-shell-{index}"),
                    "request": LocalDaemonRequest::RunShellCommand(RunShellCapabilityRequest {
                        session_id: session_id.clone(),
                        attachment_id: attachment_id.clone(),
                        command: "sh".to_string(),
                        args: vec!["-c".to_string(), "sleep 1".to_string()],
                        working_directory: None,
                        timeout_ms: Some(3_000),
                    }),
                }),
            )
            .await;
        }

        let overload_response = wait_for_error_code(&mut socket, "kernel_request_overloaded").await;
        assert_eq!(
            overload_response["error"]["retryable"].as_bool(),
            Some(true)
        );

        let mut health_socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let health = send_request(
            &mut health_socket,
            "daemon-health-after-inbound-overload",
            LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest),
        )
        .await;
        let transport = &response_variant(&health, "DaemonHealth")["projection"]["transport"];
        assert!(
            transport["inbound_overload_rejections"]
                .as_u64()
                .unwrap_or_default()
                >= 1,
            "transport health should report inbound overload rejections: {health}"
        );

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}
