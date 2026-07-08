use crate::support::kernel_websocket::*;
use arroba_kernel::local::{GetDaemonHealthRequest, ListSessionsRequest, LocalDaemonRequest};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;

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
async fn kernel_websocket_reuses_completed_command_after_server_restart() {
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
    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "create-session-before-restart",
            "command_id": "restart-stable-create-session",
            "request": LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-kernel-restart-idempotent",
                "worktree-kernel-restart-idempotent",
            )),
        }),
    )
    .await;
    let first_response = wait_for_response(&mut socket, "create-session-before-restart").await;
    let first_session_id = response_variant(&first_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("first session id should be present")
        .to_string();

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("kernel websocket task should join")
        .expect("kernel websocket server should shut down cleanly");

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
            "type": "request",
            "request_id": "create-session-after-restart",
            "command_id": "restart-stable-create-session",
            "request": LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-kernel-restart-idempotent",
                "worktree-kernel-restart-idempotent",
            )),
        }),
    )
    .await;
    let retry_response =
        wait_for_response(&mut restart_socket, "create-session-after-restart").await;
    let retry_session_id = response_variant(&retry_response, "SessionCreated")["session"]["id"]
        .as_str()
        .expect("retry session id should be present");
    assert_eq!(first_session_id, retry_session_id);

    let sessions_response = send_request(
        &mut restart_socket,
        "list-sessions-after-restart-command-replay",
        LocalDaemonRequest::ListSessions(ListSessionsRequest),
    )
    .await;
    let matching_sessions = response_variant(&sessions_response, "SessionsListed")["sessions"]
        .as_array()
        .expect("sessions should be listed")
        .iter()
        .filter(|session| {
            session["workspace_id"].as_str() == Some("workspace-kernel-restart-idempotent")
        })
        .count();
    assert_eq!(
        matching_sessions, 1,
        "replayed completed command should not dispatch again after websocket restart: {sessions_response}"
    );

    let _ = restart_shutdown_tx.send(());
    restart_server
        .await
        .expect("kernel websocket restart task should join")
        .expect("kernel websocket restart server should shut down cleanly");
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
