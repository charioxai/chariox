use crate::support::kernel_websocket::*;
use chariox_kernel::attachment::ClientCapabilityLevel;
use chariox_kernel::local::{AttachToSessionRequest, GetDaemonHealthRequest, LocalDaemonRequest};
use chariox_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use chariox_kernel::session::CreateSessionRequest;
use chariox_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_reports_replay_gap_when_resume_cursor_is_not_retained() {
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
            "workspace-kernel-replay-gap",
            "worktree-kernel-replay-gap",
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
            client_id: "ws-replay-gap-client".to_string(),
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
            "resume_from_event_id": 1,
        }),
    )
    .await;

    let (replay_gap_event, subscribe_response) = timeout(Duration::from_secs(5), async {
        let mut replay_gap_event = None;
        let mut subscribe_response = None;
        loop {
            let frame = next_json_frame(&mut socket).await;
            if frame["type"] == "event" && frame["event"]["event"] == "replay_gap" {
                replay_gap_event = Some(frame.clone());
            }
            if frame["type"] == "response" && frame["request_id"] == "subscribe-session" {
                subscribe_response = Some(frame);
            }
            if let (Some(event), Some(response)) =
                (replay_gap_event.clone(), subscribe_response.clone())
            {
                return (event, response);
            }
        }
    })
    .await
    .expect("replay gap and subscribe response should arrive");
    assert_eq!(
        replay_gap_event["event"]["requested_from_event_id"].as_u64(),
        Some(1)
    );
    assert!(replay_gap_event["event"]["first_retained_event_id"].is_null());

    assert_eq!(subscribe_response["response"]["ok"].as_bool(), Some(true));
    assert_eq!(
        subscribe_response["response"]["replay_gap"]["requested_from_event_id"].as_u64(),
        Some(1)
    );

    let snapshot_event = wait_for_event(&mut socket, "session_snapshot").await;
    assert_eq!(
        snapshot_event["event"]["session"]["id"].as_str(),
        Some(session_id.as_str())
    );

    let health = send_request(
        &mut socket,
        "daemon-health-after-replay-gap",
        LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest),
    )
    .await;
    let transport = &response_variant(&health, "DaemonHealth")["projection"]["transport"];
    assert!(
        transport["replay_gaps"].as_u64().unwrap_or_default() >= 1,
        "transport health should report replay gaps: {health}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_replays_persisted_events_after_server_restart() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.runtime_mcp_port = unused_tcp_port();
    let app = std::sync::Arc::new(tokio::sync::Mutex::new(
        DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed"),
    ));

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let server = {
        let app = std::sync::Arc::clone(&app);
        tokio::spawn(async move {
            run_kernel_websocket_server_on_listener(app, kernel_websocket_listener, async {
                let _ = shutdown_rx.await;
            })
            .await
        })
    };

    let mut socket = connect_with_retry(&config.kernel_websocket_url()).await;

    let create_response = send_request(
        &mut socket,
        "create-session-before-event-replay-restart",
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-event-replay-restart",
            "worktree-event-replay-restart",
        )),
    )
    .await;
    let session_id = response_variant(&create_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session-before-event-replay-restart",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-event-replay-restart-client".to_string(),
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
            "request_id": "subscribe-before-event-replay-restart",
            "session_id": session_id,
            "attachment_id": attachment_id,
        }),
    )
    .await;
    let subscribe_response =
        wait_for_response(&mut socket, "subscribe-before-event-replay-restart").await;
    assert_eq!(subscribe_response["response"]["ok"].as_bool(), Some(true));

    let snapshot_event = wait_for_event(&mut socket, "session_snapshot").await;
    let resume_from_event_id = snapshot_event["event_id"]
        .as_u64()
        .expect("snapshot event id should be present");
    let heartbeat_event = wait_for_event(&mut socket, "heartbeat").await;
    let expected_replay_event_id = heartbeat_event["event_id"]
        .as_u64()
        .expect("heartbeat event id should be present");
    assert!(
        expected_replay_event_id > resume_from_event_id,
        "heartbeat should be replayed from the snapshot cursor"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
    drop(socket);

    let (restart_port, restart_listener) = reserved_kernel_listener();
    let (restart_shutdown_tx, restart_shutdown_rx) = oneshot::channel::<()>();
    let restart_server = {
        let app = std::sync::Arc::clone(&app);
        tokio::spawn(async move {
            run_kernel_websocket_server_on_listener(app, restart_listener, async {
                let _ = restart_shutdown_rx.await;
            })
            .await
        })
    };

    let restart_endpoint = format!("ws://127.0.0.1:{restart_port}/kernel");
    let mut restart_socket = connect_with_retry(&restart_endpoint).await;
    send_frame(
        &mut restart_socket,
        json!({
            "type": "subscribe",
            "request_id": "subscribe-after-event-replay-restart",
            "session_id": session_id,
            "attachment_id": attachment_id,
            "resume_from_event_id": resume_from_event_id,
        }),
    )
    .await;

    let (replayed_event, replay_response) = timeout(Duration::from_secs(5), async {
        let mut replayed_event = None;
        let mut replay_response = None;
        loop {
            let frame = next_json_frame(&mut restart_socket).await;
            if frame["type"] == "event"
                && frame["event_id"].as_u64() == Some(expected_replay_event_id)
            {
                replayed_event = Some(frame.clone());
            }
            if frame["type"] == "response"
                && frame["request_id"] == "subscribe-after-event-replay-restart"
            {
                replay_response = Some(frame);
            }
            if let (Some(event), Some(response)) = (replayed_event.clone(), replay_response.clone())
            {
                return (event, response);
            }
        }
    })
    .await
    .expect("persisted event should replay after websocket restart");
    assert_eq!(replay_response["response"]["ok"].as_bool(), Some(true));
    assert_eq!(
        replay_response["response"]["resumed_from_event_id"].as_u64(),
        Some(resume_from_event_id)
    );
    assert_eq!(replayed_event["event"]["event"].as_str(), Some("heartbeat"));
    assert_eq!(
        replayed_event["event"]["session_id"].as_str(),
        Some(session_id.as_str())
    );

    let _ = restart_shutdown_tx.send(());
    restart_server
        .await
        .expect("kernel websocket restart task should join")
        .expect("kernel websocket restart server should shut down cleanly");
}
