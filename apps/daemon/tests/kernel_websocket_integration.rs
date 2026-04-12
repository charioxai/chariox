use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use arroba_daemon::attachment::ClientCapabilityLevel;
use arroba_daemon::kernel_transport::run_kernel_websocket_server_on_listener;
use arroba_daemon::local::{
    AttachToSessionRequest, CancelActivePromptRequest, DeleteSessionRequest, FocusAgentRequest,
    GetDaemonHealthRequest, GetProviderCatalogRequest, GetProviderRunRequest,
    GetSessionHistoryRequest, GetSessionStateRequest, LaunchProviderRunRequest,
    ListProviderProcessesRequest, ListSessionsRequest, LocalDaemonRequest,
    PumpTerminalOutputRequest, ResizeTerminalRequest, RunShellCapabilityRequest, SpawnAgentRequest,
    SubmitPromptRequest,
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

const UX_RESPONSE_BUDGET: Duration = Duration::from_millis(250);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_streams_session_snapshot_and_unavailable_events() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
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
async fn kernel_websocket_closes_slow_consumers_when_the_outgoing_queue_overflows() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
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
    let _snapshot_event = wait_for_event(&mut socket, "session_snapshot").await;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_history_read_is_slow() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.session_history_read_delay_ms = 500;
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
            "workspace-history-responsive",
            "worktree-history-responsive",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-history-responsive-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    let provider_response = send_request(
        &mut socket,
        "launch-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let provider_run_id = provider_run_id_from_launch_response(&provider_response);
    wait_for_provider_run_state(&mut socket, &provider_run_id, "Running").await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "slow-history",
            "request": LocalDaemonRequest::GetSessionHistory(GetSessionHistoryRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                round_count: Some(1),
                max_chars: Some(4096),
                before_entry_index: None,
                before_entry_char_offset: None,
            }),
        }),
    )
    .await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "submit-prompt",
            "request": LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session_id.clone(),
                attachment_id: attachment_id.clone(),
                target_agent_id: Some(agent_id.clone()),
                prompt: "prompt should ack while history is still loading".to_string(),
                attachments: Vec::new(),
            }),
        }),
    )
    .await;
    let submit_response =
        wait_for_response_with_timeout(&mut socket, "submit-prompt", Duration::from_millis(250))
            .await;
    assert!(
        response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
        "prompt should start before the delayed history read completes: {submit_response}"
    );

    let history_response = wait_for_response(&mut socket, "slow-history").await;
    assert!(
        response_variant(&history_response, "SessionHistory")["entries"].is_array(),
        "history response should still complete: {history_response}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_provider_catalog_is_slow() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.provider_catalog_read_delay_ms = 500;
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
            "workspace-catalog-responsive",
            "worktree-catalog-responsive",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-catalog-responsive-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    let provider_response = send_request(
        &mut socket,
        "launch-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let provider_run_id = provider_run_id_from_launch_response(&provider_response);
    wait_for_provider_run_state(&mut socket, &provider_run_id, "Running").await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "slow-provider-catalog",
            "request": LocalDaemonRequest::GetProviderCatalog(GetProviderCatalogRequest),
        }),
    )
    .await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "submit-prompt",
            "request": LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session_id.clone(),
                attachment_id: attachment_id.clone(),
                target_agent_id: Some(agent_id.clone()),
                prompt: "prompt should ack while provider catalog is still loading".to_string(),
                attachments: Vec::new(),
            }),
        }),
    )
    .await;
    let submit_response =
        wait_for_response_with_timeout(&mut socket, "submit-prompt", Duration::from_millis(250))
            .await;
    assert!(
        response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
        "prompt should start before the delayed provider catalog completes: {submit_response}"
    );

    let _catalog_response = wait_for_request_completion(&mut socket, "slow-provider-catalog").await;

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_provider_process_list_is_slow() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.provider_process_list_delay_ms = 500;
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
            "workspace-provider-process-responsive",
            "worktree-provider-process-responsive",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-provider-process-responsive-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    let provider_response = send_request(
        &mut socket,
        "launch-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let provider_run_id = provider_run_id_from_launch_response(&provider_response);
    wait_for_provider_run_state(&mut socket, &provider_run_id, "Running").await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "slow-provider-process-list",
            "request": LocalDaemonRequest::ListProviderProcesses(ListProviderProcessesRequest {
                provider: None,
            }),
        }),
    )
    .await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "submit-prompt",
            "request": LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session_id.clone(),
                attachment_id: attachment_id.clone(),
                target_agent_id: Some(agent_id.clone()),
                prompt: "prompt should ack while provider process list is delayed".to_string(),
                attachments: Vec::new(),
            }),
        }),
    )
    .await;
    let submit_response =
        wait_for_response_with_timeout(&mut socket, "submit-prompt", Duration::from_millis(250))
            .await;
    assert!(
        response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
        "prompt should start before the delayed provider process list completes: {submit_response}"
    );

    let _process_response =
        wait_for_request_completion(&mut socket, "slow-provider-process-list").await;

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_provider_launch_is_initializing() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.provider_runtime_init_delay_ms = 500;
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
            "workspace-provider-launch-responsive",
            "worktree-provider-launch-responsive",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-provider-launch-responsive-client".to_string(),
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
            "type": "request",
            "request_id": "slow-provider-launch",
            "request": LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
                session_id: session_id.clone(),
                agent_id: Some(agent_id.clone()),
                adapter_key: "dev-stub".to_string(),
                provider: "claude-code".to_string(),
                account_profile: "default".to_string(),
                model: "sonnet".to_string(),
                variant: None,
            }),
        }),
    )
    .await;
    let launch_response = wait_for_response_with_timeout(
        &mut socket,
        "slow-provider-launch",
        Duration::from_millis(250),
    )
    .await;
    let accepted_run =
        &response_variant(&launch_response, "ProviderRunLaunchAccepted")["provider_run"];
    assert_eq!(
        accepted_run["state"], "Starting",
        "launch should ack with a starting provider run before runtime initialization completes: {launch_response}"
    );

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "submit-prompt",
            "request": LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session_id.clone(),
                attachment_id: attachment_id.clone(),
                target_agent_id: Some(agent_id.clone()),
                prompt: "prompt should ack while provider launch is initializing".to_string(),
                attachments: Vec::new(),
            }),
        }),
    )
    .await;
    let submit_response =
        wait_for_response_with_timeout(&mut socket, "submit-prompt", Duration::from_millis(250))
            .await;
    assert!(
        response_variant(&submit_response, "PromptSubmitted")["outcome"]["Queued"].is_object(),
        "prompt should queue while the accepted provider launch is still starting: {submit_response}"
    );

    sleep(Duration::from_millis(600)).await;
    let state_response = send_request(
        &mut socket,
        "session-state-after-launch",
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        }),
    )
    .await;
    assert!(
        response_variant(&state_response, "SessionState")["session"]["prompt_states"][&agent_id]
            ["active_prompt"]
            .is_object(),
        "queued prompt should start after provider launch finishes: {state_response}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_state_and_cancel_ack_while_structured_provider_io_is_slow() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
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
            "workspace-structured-io-responsive",
            "worktree-structured-io-responsive",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-structured-io-responsive-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    let provider_response = send_request(
        &mut socket,
        "launch-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "slow-structured".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let provider_run_id = provider_run_id_from_launch_response(&provider_response);
    wait_for_provider_run_state(&mut socket, &provider_run_id, "Running").await;

    let spawn_response = send_request(
        &mut socket,
        "spawn-second-agent",
        LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session_id.clone(),
            alias: Some("reviewer".to_string()),
            provider: "dev-stub".to_string(),
            model: Some("sonnet".to_string()),
            effort: None,
            worktree_id: None,
            machine_ref: None,
        }),
    )
    .await;
    let second_agent_id = response_variant(&spawn_response, "AgentSpawned")["agent"]["id"]
        .as_str()
        .expect("second agent id should be present")
        .to_string();
    let second_provider_response = send_request(
        &mut socket,
        "launch-second-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(second_agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "slow-structured".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let second_provider_run_id = provider_run_id_from_launch_response(&second_provider_response);
    wait_for_provider_run_state(&mut socket, &second_provider_run_id, "Running").await;
    let _focus_first_response = send_request(
        &mut socket,
        "focus-first-agent-before-slow-submit",
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        }),
    )
    .await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "submit-slow-structured-prompt",
            "request": LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session_id.clone(),
                attachment_id: attachment_id.clone(),
                target_agent_id: Some(agent_id.clone()),
                prompt: "slow structured provider submit should not block kernel".to_string(),
                attachments: Vec::new(),
            }),
        }),
    )
    .await;
    let submit_response = wait_for_response_with_timeout(
        &mut socket,
        "submit-slow-structured-prompt",
        UX_RESPONSE_BUDGET,
    )
    .await;
    assert!(
        response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
        "prompt should ack before slow structured submit finishes: {submit_response}"
    );

    let state_response = send_request_with_ux_budget(
        &mut socket,
        "state-during-slow-submit",
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        }),
    )
    .await;
    assert!(
        response_variant(&state_response, "SessionState")["session"]["prompt_states"][&agent_id]
            ["active_prompt"]
            .is_object(),
        "session state should remain readable while structured submit is slow: {state_response}"
    );

    let focus_response = send_request_with_ux_budget(
        &mut socket,
        "focus-during-slow-submit",
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.clone(),
            agent_id: second_agent_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        response_variant(&focus_response, "AgentFocused")["agent"]["id"].as_str(),
        Some(second_agent_id.as_str()),
        "focus should ack while structured submit is slow: {focus_response}"
    );

    let resize_response = send_request_with_ux_budget(
        &mut socket,
        "resize-during-slow-submit",
        LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols: 132,
            rows: 43,
        }),
    )
    .await;
    assert_eq!(
        response_variant(&resize_response, "TerminalResized")["cols"].as_u64(),
        Some(132),
        "resize should ack while structured submit is slow: {resize_response}"
    );

    let second_submit_response = send_request_with_ux_budget(
        &mut socket,
        "second-agent-submit-during-first-slow-submit",
        LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment_id.clone(),
            target_agent_id: Some(second_agent_id.clone()),
            prompt: "second agent prompt should ack during another run's provider I/O".to_string(),
            attachments: Vec::new(),
        }),
    )
    .await;
    assert!(
        response_variant(&second_submit_response, "PromptSubmitted")["outcome"]["Started"]
            .is_object(),
        "another agent's prompt should ack while the first provider submit is slow: {second_submit_response}"
    );

    let cancel_response = send_request_with_ux_budget(
        &mut socket,
        "cancel-during-slow-submit",
        LocalDaemonRequest::CancelActivePrompt(CancelActivePromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment_id.clone(),
        }),
    )
    .await;
    assert!(
        response_variant(&cancel_response, "PromptCancelled")["cancellation"]["prompt"]["status"]
            == "Cancelling",
        "cancel should ack while structured provider abort is slow: {cancel_response}"
    );

    sleep(Duration::from_millis(800)).await;
    let state_during_abort_response = send_request_with_ux_budget(
        &mut socket,
        "state-during-slow-abort",
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        }),
    )
    .await;
    assert!(
        response_variant(&state_during_abort_response, "SessionState")["session"].is_object(),
        "session state should remain readable while structured abort is slow: {state_during_abort_response}"
    );

    let focus_during_abort_response = send_request_with_ux_budget(
        &mut socket,
        "focus-during-slow-abort",
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        response_variant(&focus_during_abort_response, "AgentFocused")["agent"]["id"].as_str(),
        Some(agent_id.as_str()),
        "focus should ack while structured abort is slow: {focus_during_abort_response}"
    );

    let resize_during_abort_response = send_request_with_ux_budget(
        &mut socket,
        "resize-during-slow-abort",
        LocalDaemonRequest::ResizeTerminal(ResizeTerminalRequest {
            session_id: session_id.clone(),
            cols: 120,
            rows: 38,
        }),
    )
    .await;
    assert_eq!(
        response_variant(&resize_during_abort_response, "TerminalResized")["rows"].as_u64(),
        Some(38),
        "resize should ack while structured abort is slow: {resize_during_abort_response}"
    );

    let poll_response = send_request_with_ux_budget(
        &mut socket,
        "start-slow-output-poll",
        LocalDaemonRequest::PumpTerminalOutput(PumpTerminalOutputRequest {
            session_id: session_id.clone(),
            attachment_id: attachment_id.clone(),
        }),
    )
    .await;
    assert!(
        response_variant(&poll_response, "TerminalOutput")["records"].is_array(),
        "terminal output polling should ack before structured output poll finishes: {poll_response}"
    );
    sleep(Duration::from_millis(50)).await;

    let state_during_poll_response = send_request_with_ux_budget(
        &mut socket,
        "state-during-slow-output-poll",
        LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
            session_id: session_id.clone(),
        }),
    )
    .await;
    assert!(
        response_variant(&state_during_poll_response, "SessionState")["session"].is_object(),
        "session state should remain readable while structured output poll is slow: {state_during_poll_response}"
    );

    let focus_during_poll_response = send_request_with_ux_budget(
        &mut socket,
        "focus-during-slow-output-poll",
        LocalDaemonRequest::FocusAgent(FocusAgentRequest {
            session_id: session_id.clone(),
            agent_id: second_agent_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        response_variant(&focus_during_poll_response, "AgentFocused")["agent"]["id"].as_str(),
        Some(second_agent_id.as_str()),
        "focus should ack while structured output poll is slow: {focus_during_poll_response}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_reports_async_provider_launch_failure() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
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
            "workspace-provider-launch-failure",
            "worktree-provider-launch-failure",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"].as_str().expect("session id").to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-provider-launch-failure-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id")
        .to_string();

    send_frame(
        &mut socket,
        json!({
            "type": "subscribe",
            "request_id": "subscribe-session",
            "session_id": session_id.clone(),
            "attachment_id": attachment_id.clone(),
            "resume_from_event_id": null,
        }),
    )
    .await;
    let _subscribe_response = wait_for_response(&mut socket, "subscribe-session").await;

    let launch_response = send_request(
        &mut socket,
        "launch-failing-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id),
            adapter_key: "dev-stub".to_string(),
            provider: "runtime-init-fail".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let provider_run_id = provider_run_id_from_launch_response(&launch_response);
    assert_eq!(
        response_variant(&launch_response, "ProviderRunLaunchAccepted")["provider_run"]["state"],
        "Starting"
    );

    let notice_event = wait_for_event(&mut socket, "runtime_notices").await;
    let notices = notice_event["event"]["notices"]
        .as_array()
        .expect("runtime notices should be present");
    assert!(
        notices.iter().any(|notice| {
            notice["provider_run_id"].as_str() == Some(provider_run_id.as_str())
                && notice["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("failed before it became ready"))
        }),
        "launch failure should be visible as a runtime notice: {notice_event}"
    );
    wait_for_provider_run_state(&mut socket, &provider_run_id, "Ended").await;

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_replaces_starting_provider_launch() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
    config.provider_runtime_init_delay_ms = 500;
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
            "workspace-provider-launch-replace",
            "worktree-provider-launch-replace",
        )),
    )
    .await;
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"].as_str().expect("session id").to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id")
        .to_string();

    let first_launch = send_request(
        &mut socket,
        "launch-provider-first",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let first_run_id = provider_run_id_from_launch_response(&first_launch);
    assert_eq!(
        response_variant(&first_launch, "ProviderRunLaunchAccepted")["provider_run"]["state"],
        "Starting"
    );

    let second_launch = send_request(
        &mut socket,
        "launch-provider-second",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "opus".to_string(),
            variant: None,
        }),
    )
    .await;
    let second_run_id = provider_run_id_from_launch_response(&second_launch);
    assert_ne!(first_run_id, second_run_id);
    wait_for_provider_run_state(&mut socket, &first_run_id, "Ended").await;
    wait_for_provider_run_state(&mut socket, &second_run_id, "Running").await;

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_shell_capability_is_slow() {
    let mut config = DaemonConfig::for_tests();
    let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
    config.kernel_websocket_port = kernel_websocket_port;
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
    let session = &response_variant(&create_response, "SessionCreated")["session"];
    let session_id = session["id"]
        .as_str()
        .expect("session id should be present")
        .to_string();
    let agent_id = session["agents"][0]["id"]
        .as_str()
        .expect("agent id should be present")
        .to_string();

    let attach_response = send_request(
        &mut socket,
        "attach-session",
        LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
            session_id: session_id.clone(),
            client_id: "ws-shell-responsive-client".to_string(),
            capability_level: ClientCapabilityLevel::FullTerminal,
        }),
    )
    .await;
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
        .as_str()
        .expect("attachment id should be present")
        .to_string();

    let provider_response = send_request(
        &mut socket,
        "launch-provider",
        LocalDaemonRequest::LaunchProviderRun(LaunchProviderRunRequest {
            session_id: session_id.clone(),
            agent_id: Some(agent_id.clone()),
            adapter_key: "dev-stub".to_string(),
            provider: "claude-code".to_string(),
            account_profile: "default".to_string(),
            model: "sonnet".to_string(),
            variant: None,
        }),
    )
    .await;
    let provider_run_id = provider_run_id_from_launch_response(&provider_response);
    wait_for_provider_run_state(&mut socket, &provider_run_id, "Running").await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "slow-shell",
            "request": LocalDaemonRequest::RunShellCommand(RunShellCapabilityRequest {
                session_id: session_id.clone(),
                attachment_id: attachment_id.clone(),
                command: "sh".to_string(),
                args: vec!["-c".to_string(), "sleep 0.5".to_string()],
                working_directory: None,
                timeout_ms: Some(1_000),
            }),
        }),
    )
    .await;
    sleep(Duration::from_millis(50)).await;

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "submit-prompt",
            "request": LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session_id.clone(),
                attachment_id: attachment_id.clone(),
                target_agent_id: Some(agent_id.clone()),
                prompt: "prompt should ack while shell command is still running".to_string(),
                attachments: Vec::new(),
            }),
        }),
    )
    .await;
    let submit_response =
        wait_for_response_with_timeout(&mut socket, "submit-prompt", Duration::from_millis(250))
            .await;
    assert!(
        response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
        "prompt should start before the shell capability completes: {submit_response}"
    );

    let shell_response = wait_for_response(&mut socket, "slow-shell").await;
    assert!(
        response_variant(&shell_response, "ShellCommandCompleted")["result"].is_object(),
        "shell capability should still complete: {shell_response}"
    );

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");
}

fn reserved_kernel_listener() -> (u16, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("should bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("listener address should exist")
        .port();
    (port, listener)
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

async fn send_request_with_ux_budget(
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
    wait_for_response_with_timeout(socket, request_id, UX_RESPONSE_BUDGET).await
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

async fn wait_for_responses(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_ids: &[&str],
) -> BTreeMap<String, Value> {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        let mut responses = BTreeMap::new();
        while responses.len() < request_ids.len() {
            let frame = next_json_frame(socket).await;
            if frame["type"] != "response" {
                continue;
            }
            let Some(request_id) = frame["request_id"].as_str() else {
                continue;
            };
            if request_ids.contains(&request_id) {
                assert!(
                    frame["error"].is_null(),
                    "kernel websocket response should not contain an error: {frame}"
                );
                responses.insert(request_id.to_string(), frame);
            }
        }
        responses
    })
    .await
    .expect("timed out waiting for kernel websocket responses")
}

async fn wait_for_response_with_timeout(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
    deadline: Duration,
) -> Value {
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

async fn wait_for_error_response(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                assert!(
                    !frame["error"].is_null(),
                    "kernel websocket response should contain an error: {frame}"
                );
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket error response")
}

async fn wait_for_error_code(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    code: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["error"]["code"] == code {
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket error code")
}

async fn wait_for_request_completion(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    request_id: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        loop {
            let frame = next_json_frame(socket).await;
            if frame["type"] == "response" && frame["request_id"] == request_id {
                return frame;
            }
        }
    })
    .await
    .expect("timed out waiting for kernel websocket request completion")
}

fn provider_run_id_from_launch_response(frame: &Value) -> String {
    let response = &frame["response"];
    let provider_run = response
        .get("ProviderRunLaunched")
        .or_else(|| response.get("ProviderRunLaunchAccepted"))
        .and_then(|value| value.get("provider_run"))
        .unwrap_or_else(|| panic!("expected provider launch response, got: {frame}"));
    provider_run["id"]
        .as_str()
        .expect("provider run id should be present")
        .to_string()
}

async fn wait_for_provider_run_state(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    provider_run_id: &str,
    expected_state: &str,
) -> Value {
    let deadline = Duration::from_secs(5);
    timeout(deadline, async {
        let mut attempt = 0_u64;
        loop {
            let request_id = format!("provider-run-state-{provider_run_id}-{attempt}");
            let frame = send_request(
                socket,
                &request_id,
                LocalDaemonRequest::GetProviderRun(GetProviderRunRequest {
                    provider_run_id: provider_run_id.to_string(),
                }),
            )
            .await;
            if response_variant(&frame, "ProviderRun")["provider_run"]["state"] == expected_state {
                return frame;
            }
            attempt += 1;
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("timed out waiting for provider run state")
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
