use std::net::TcpListener;
use std::time::Duration;

use arroba_daemon::attachment::ClientCapabilityLevel;
use arroba_daemon::kernel_transport::run_kernel_websocket_server;
use arroba_daemon::local::{
    AttachToSessionRequest, DeleteSessionRequest, GetSessionStateRequest, LocalDaemonRequest,
};
use arroba_daemon::session::CreateSessionRequest;
use arroba_daemon::{DaemonApp, DaemonConfig};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_streams_session_snapshot_and_unavailable_events() {
    let mut config = DaemonConfig::for_tests();
    config.kernel_websocket_port = free_port();
    let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server(std::sync::Arc::new(tokio::sync::Mutex::new(app)), async {
            let _ = shutdown_rx.await;
        })
        .await
    });

    let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;

    let create_response = send_request(
        &mut socket,
        "create-session",
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-kernel-ws",
            "worktree-kernel-ws",
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
            client_id: "ws-test-client".to_string(),
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
    let subscribe_response = wait_for_response(&mut socket, "subscribe-session").await;
    assert_eq!(subscribe_response["response"]["ok"].as_bool(), Some(true));
    assert!(subscribe_response["response"]["resumed_from_event_id"].is_null());

    let snapshot_event = wait_for_event(&mut socket, "session_snapshot").await;
    assert_eq!(
        snapshot_event["event"]["session"]["id"].as_str(),
        Some(session_id.as_str())
    );
    let heartbeat_event = wait_for_event(&mut socket, "heartbeat").await;
    assert_eq!(
        heartbeat_event["event"]["session_id"].as_str(),
        Some(session_id.as_str())
    );

    let _delete_response = send_request(
        &mut socket,
        "delete-session",
        LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: session_id,
            workspace_id: Some("workspace-kernel-ws".to_string()),
        }),
    )
    .await;

    let unavailable_event = wait_for_event(&mut socket, "session_unavailable").await;
    assert_eq!(
        unavailable_event["event"]["message"].as_str(),
        Some("Current session is no longer available.")
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
    config.kernel_websocket_port = free_port();
    config.kernel_websocket_queue_capacity = 2;
    config.kernel_websocket_write_delay_ms = 200;
    let app = DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        run_kernel_websocket_server(std::sync::Arc::new(tokio::sync::Mutex::new(app)), async {
            let _ = shutdown_rx.await;
        })
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
    let _snapshot_event = wait_for_event(&mut socket, "session_snapshot").await;

    for index in 0..8 {
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

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("should bind an ephemeral port");
    listener
        .local_addr()
        .expect("listener address should exist")
        .port()
}

async fn connect_with_retry(url: &str) -> WebSocketStream<MaybeTlsStream<TcpStream>> {
    for _ in 0..20 {
        match connect_async(url).await {
            Ok((socket, _)) => return socket,
            Err(_) => sleep(Duration::from_millis(25)).await,
        }
    }

    let (socket, _) = connect_async(url)
        .await
        .expect("kernel websocket should accept connections");
    socket
}

async fn send_request(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    request: LocalDaemonRequest,
) -> Value {
    send_frame(
        socket,
        json!({
            "type": "request",
            "request_id": request_id,
            "request": request,
        }),
    )
    .await;
    wait_for_response(socket, request_id).await
}

async fn send_frame(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>, frame: Value) {
    socket
        .send(Message::Text(frame.to_string().into()))
        .await
        .expect("kernel websocket frame should send");
}

async fn wait_for_response(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                assert!(
                    frame["error"].is_null(),
                    "kernel websocket response should not contain an error: {frame}"
                );
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket response")
}

async fn wait_for_event(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    event_name: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "event" && frame["event"]["event"] == event_name {
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket event")
}

async fn next_json_frame(socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>) -> Value {
    loop {
        let message = socket
            .next()
            .await
            .expect("kernel websocket should yield a frame")
            .expect("kernel websocket frame should decode");

        match message {
            Message::Text(text) => {
                return serde_json::from_str(&text)
                    .expect("kernel websocket text frame should be valid json");
            }
            Message::Binary(bytes) => {
                return serde_json::from_slice(&bytes)
                    .expect("kernel websocket binary frame should be valid json");
            }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(frame) => panic!("kernel websocket closed unexpectedly: {frame:?}"),
            Message::Frame(_) => {}
        }
    }
}

async fn wait_for_close(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> (Option<u16>, Option<String>) {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let message = socket
                .next()
                .await
                .expect("kernel websocket should yield a frame")
                .expect("kernel websocket frame should decode");

            if let Message::Close(frame) = message {
                return (
                    frame.as_ref().map(|frame| u16::from(frame.code)),
                    frame.as_ref().map(|frame| frame.reason.to_string()),
                );
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket close")
}

fn response_variant<'a>(frame: &'a Value, variant: &str) -> &'a Value {
    frame["response"][variant]
        .as_object()
        .map(|_| &frame["response"][variant])
        .unwrap_or_else(|| panic!("expected response variant `{variant}`, got: {frame}"))
}
