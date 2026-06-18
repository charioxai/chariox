use arroba_kernel::attachment::ClientCapabilityLevel;
use arroba_kernel::local::{
    AddWorkflowNodeRequest, AttachToSessionRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, DeleteSessionRequest, GetDaemonHealthRequest, GetSessionStateRequest,
    InvokeWorkflowEndpointRequest, ListSessionsRequest, LocalDaemonRequest,
    RunShellCapabilityRequest, SpawnAgentRequest,
};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

mod support;

use support::kernel_websocket::*;

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
async fn kernel_websocket_streams_session_snapshot_and_unavailable_events() {
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
async fn kernel_websocket_streams_workflow_run_updates() {
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
        "create-session-workflow-run-events",
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
            "workspace-workflow-run-events",
            "worktree-workflow-run-events",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let spawn_response = send_request(
        &mut socket,
        "spawn-workflow-agent",
        LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("node-a".to_string()),
            provider: Some("dev-stub".to_string()),
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }),
    )
    .await;
    let agent_id = response_variant(&spawn_response, "AgentSpawned")["agent"]["id"]
        .as_str()
        .expect("spawned agent id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session-workflow-run-events",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-workflow-run-event-client".to_string(),
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
            "request_id": "subscribe-workflow-run-events",
            "session_id": session_id,
            "attachment_id": attachment_id,
        }),
    )
    .await;
    let _subscribe_response = wait_for_response(&mut socket, "subscribe-workflow-run-events").await;

    let workflow_response = send_request(
        &mut socket,
        "create-workflow",
        LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
            session_id: session_id.clone(),
            alias: Some("run-events".to_string()),
        }),
    )
    .await;
    let workflow_id = response_variant(&workflow_response, "WorkflowCreated")["workflow"]["id"]
        .as_str()
        .expect("workflow id should be present")
        .to_string();
    let node_response = send_request(
        &mut socket,
        "add-workflow-node",
        LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            agent_id,
            expected_workflow_revision: None,
        }),
    )
    .await;
    let node_id = response_variant(&node_response, "WorkflowNodeAdded")["node"]["id"]
        .as_str()
        .expect("node id should be present")
        .to_string();
    let endpoint_response = send_request(
        &mut socket,
        "create-workflow-endpoint",
        LocalDaemonRequest::CreateWorkflowEndpoint(CreateWorkflowEndpointRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            entry_node_id: node_id,
            alias: Some("entry".to_string()),
            expected_workflow_revision: None,
        }),
    )
    .await;
    let endpoint_id = response_variant(&endpoint_response, "WorkflowEndpointCreated")["endpoint"]
        ["id"]
        .as_str()
        .expect("endpoint id should be present")
        .to_string();
    let invoke_response = send_request(
        &mut socket,
        "invoke-workflow-endpoint",
        LocalDaemonRequest::InvokeWorkflowEndpoint(InvokeWorkflowEndpointRequest {
            session_id: session_id.clone(),
            workflow_ref: workflow_id.clone(),
            endpoint_ref: endpoint_id,
            prompt: Some("stream workflow run update".to_string()),
            queue_ref: None,
            publication_invocation: None,
        }),
    )
    .await;
    let expected_run_id = response_variant(&invoke_response, "WorkflowRunInvoked")["workflow_run"]
        ["id"]
        .as_str()
        .expect("workflow run id should be present")
        .to_string();

    let run_update_event = wait_for_event(&mut socket, "workflow_run_updated").await;
    assert_eq!(
        run_update_event["event"]["session_id"].as_str(),
        Some(session_id.as_str())
    );
    assert_eq!(
        run_update_event["event"]["workflow_run"]["id"].as_str(),
        Some(expected_run_id.as_str())
    );
    assert_eq!(
        run_update_event["event"]["workflow_run"]["workflow_id"].as_str(),
        Some(workflow_id.as_str())
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
    config.kernel_websocket_queue_capacity = 4;
    config.kernel_websocket_write_delay_ms = 200;
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

    for index in 0..12 {
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

    let replay_gap_event = wait_for_event(&mut socket, "replay_gap").await;
    assert_eq!(
        replay_gap_event["event"]["requested_from_event_id"].as_u64(),
        Some(1)
    );
    assert!(replay_gap_event["event"]["first_retained_event_id"].is_null());

    let subscribe_response = wait_for_response(&mut socket, "subscribe-session").await;
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
async fn kernel_websocket_reuses_completed_result_for_duplicate_command_id() {
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

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "create-session-first",
            "command_id": "duplicate-create-session",
            "request": LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-kernel-idempotent",
                "worktree-kernel-idempotent",
            )),
        }),
    )
    .await;
    let first_response = wait_for_response(&mut socket, "create-session-first").await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "create-session-retry",
            "command_id": "duplicate-create-session",
            "request": LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-kernel-idempotent",
                "worktree-kernel-idempotent",
            )),
        }),
    )
    .await;
    let retry_response = wait_for_response(&mut socket, "create-session-retry").await;
    let first_session_id = response_variant(&first_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("first session id should be present");
    let retry_session_id = response_variant(&retry_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("retry session id should be present");
    assert_eq!(first_session_id, retry_session_id);

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_fans_out_inflight_duplicate_command_id() {
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

    for request_id in ["create-session-first", "create-session-retry"] {
        send_frame(
            &mut socket,
            json!({
                "type": "request",
                "request_id": request_id,
                "command_id": "inflight-duplicate-create-session",
                "request": LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                    "workspace-kernel-inflight-idempotent",
                    "worktree-kernel-inflight-idempotent",
                )),
            }),
        )
        .await;
    }

    let mut responses = wait_for_responses(
        &mut socket,
        &["create-session-first", "create-session-retry"],
    )
    .await;
    let first_response = responses
        .remove("create-session-first")
        .expect("first response should be present");
    let retry_response = responses
        .remove("create-session-retry")
        .expect("retry response should be present");
    let first_session_id = response_variant(&first_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("first session id should be present");
    let retry_session_id = response_variant(&retry_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("retry session id should be present");
    assert_eq!(first_session_id, retry_session_id);

    let sessions_response = send_request(
        &mut socket,
        "list-sessions",
        LocalDaemonRequest::ListSessions(ListSessionsRequest),
    )
    .await;
    let matching_sessions = response_variant(&sessions_response, "SessionsListed")["sessions"]
        .as_array()
        .expect("sessions should be listed")
        .iter()
        .filter(|session| {
            session["workspace_id"].as_str() == Some("workspace-kernel-inflight-idempotent")
        })
        .count();
    assert_eq!(
        matching_sessions, 1,
        "in-flight duplicate create session should only apply once: {sessions_response}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_rejects_duplicate_command_id_for_different_request() {
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

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "create-session-first",
            "command_id": "conflicting-create-session",
            "request": LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-kernel-conflict-a",
                "worktree-kernel-conflict-a",
            )),
        }),
    )
    .await;
    let _first_response = wait_for_response(&mut socket, "create-session-first").await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "create-session-conflict",
            "command_id": "conflicting-create-session",
            "request": LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-kernel-conflict-b",
                "worktree-kernel-conflict-b",
            )),
        }),
    )
    .await;
    let conflict_response = wait_for_error_response(&mut socket, "create-session-conflict").await;
    assert_eq!(
        conflict_response["error"]["code"].as_str(),
        Some("duplicate_command_conflict")
    );
    assert_eq!(
        conflict_response["error"]["retryable"].as_bool(),
        Some(false)
    );

    let health = send_request(
        &mut socket,
        "daemon-health-after-duplicate-conflict",
        LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest),
    )
    .await;
    let transport = &response_variant(&health, "DaemonHealth")["projection"]["transport"];
    assert!(
        transport["duplicate_command_conflicts"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "transport health should report duplicate command conflicts: {health}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_rejects_requests_when_inbound_admission_is_full() {
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
        LocalDaemonRequest::CreateSession(CreateSessionRequest::new(cwd.as_str(), cwd.as_str())),
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
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    for index in 0..16 {
        send_frame(
            &mut socket,
            json!({
                "type": "request",
                "request_id": format!("slow-shell-{index}"),
                "request": LocalDaemonRequest::RunShellCommand(RunShellCapabilityRequest {
                    session_id: session_id.clone(),
                    attachment_id: attachment_id.clone(),
                    command: "sh".to_string(),
                    args: vec!["-c".to_string(), "sleep 0.2".to_string()],
                    working_directory: None,
                    timeout_ms: Some(1_000),
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
}
