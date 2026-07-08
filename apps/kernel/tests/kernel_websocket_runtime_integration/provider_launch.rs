use crate::support::kernel_websocket::*;
use arroba_kernel::attachment::ClientCapabilityLevel;
use arroba_kernel::local::{AttachToSessionRequest, LaunchProviderRunRequest, LocalDaemonRequest};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;

#[test]
fn kernel_websocket_reports_async_provider_launch_failure() {
    crate::run_kernel_websocket_runtime_test(async {
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
        let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]
            ["id"]
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            }),
        )
        .await;
        let provider_run_id = provider_run_id_from_launch_response(&launch_response);
        assert_eq!(
            response_variant(&launch_response, "ProviderRunLaunchAccepted")["provider_run"]
                ["state"],
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
    });
}

#[test]
fn kernel_websocket_replaces_starting_provider_launch() {
    crate::run_kernel_websocket_runtime_test(async {
        let mut config = DaemonConfig::for_tests();
        let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
        config.kernel_websocket_port = kernel_websocket_port;
        config.runtime_mcp_port = unused_tcp_port();
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
    });
}
