use std::time::Duration;

use crate::support::kernel_websocket::*;
use arroba_kernel::attachment::ClientCapabilityLevel;
use arroba_kernel::local::{
    AttachToSessionRequest, GetProviderCatalogRequest, GetSessionHistoryOutlineRequest,
    GetSessionStateRequest, LaunchProviderRunRequest, ListProviderProcessesRequest,
    LocalDaemonRequest, RunShellCapabilityRequest, SubmitPromptRequest,
};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::sleep;

#[test]
fn kernel_websocket_prompt_submit_acks_while_history_read_is_slow() {
    crate::run_kernel_websocket_runtime_test(async {
        let mut config = DaemonConfig::for_tests();
        let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
        config.kernel_websocket_port = kernel_websocket_port;
        config.runtime_mcp_port = unused_tcp_port();
        config.operational_history_read_delay_ms = 2_000;
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
        let attachment_id =
            response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
            "request": LocalDaemonRequest::GetSessionHistoryOutline(GetSessionHistoryOutlineRequest {
                session_id: session_id.clone(),
                agent_ids: Some(vec![agent_id.clone()]),
                latest_prompt_count: Some(1),
                cursor: None,
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
        let submit_response = wait_for_first_response(
            &mut socket,
            &["submit-prompt", "slow-history"],
            Duration::from_secs(3),
        )
        .await;
        assert_eq!(
            submit_response["request_id"].as_str(),
            Some("submit-prompt"),
            "prompt should respond before the delayed history read completes: {submit_response}"
        );
        assert!(
            response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
            "prompt should start before the delayed history read completes: {submit_response}"
        );

        let history_response =
            wait_for_response_with_timeout(&mut socket, "slow-history", Duration::from_secs(10))
                .await;
        assert!(
            response_variant(&history_response, "SessionHistoryOutline")["agents"].is_array(),
            "history response should still complete: {history_response}"
        );

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}

#[test]
fn kernel_websocket_prompt_submit_acks_while_provider_catalog_is_slow() {
    crate::run_kernel_websocket_runtime_test(async {
        let mut config = DaemonConfig::for_tests();
        let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
        config.kernel_websocket_port = kernel_websocket_port;
        config.runtime_mcp_port = unused_tcp_port();
        config.provider_catalog_read_delay_ms = 2_000;
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
        let attachment_id =
            response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
        let submit_response = wait_for_first_response(
            &mut socket,
            &["submit-prompt", "slow-provider-catalog"],
            Duration::from_secs(3),
        )
        .await;
        assert_eq!(
            submit_response["request_id"].as_str(),
            Some("submit-prompt"),
            "prompt should respond before the delayed provider catalog completes: {submit_response}"
        );
        assert!(
            response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
            "prompt should start before the delayed provider catalog completes: {submit_response}"
        );

        let _catalog_response = wait_for_response_with_timeout(
            &mut socket,
            "slow-provider-catalog",
            Duration::from_secs(10),
        )
        .await;

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}

#[test]
fn kernel_websocket_prompt_submit_acks_while_provider_process_list_is_slow() {
    crate::run_kernel_websocket_runtime_test(async {
        let mut config = DaemonConfig::for_tests();
        let (kernel_websocket_port, kernel_websocket_listener) = reserved_kernel_listener();
        config.kernel_websocket_port = kernel_websocket_port;
        config.runtime_mcp_port = unused_tcp_port();
        config.provider_process_list_delay_ms = 2_000;
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
        let attachment_id =
            response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
        let submit_response = wait_for_first_response(
            &mut socket,
            &["submit-prompt", "slow-provider-process-list"],
            Duration::from_secs(3),
        )
        .await;
        assert_eq!(
            submit_response["request_id"].as_str(),
            Some("submit-prompt"),
            "prompt should respond before the delayed provider process list completes: {submit_response}"
        );
        assert!(
            response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
            "prompt should start before the delayed provider process list completes: {submit_response}"
        );

        let _process_response = wait_for_response_with_timeout(
            &mut socket,
            "slow-provider-process-list",
            Duration::from_secs(10),
        )
        .await;

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}

#[test]
fn kernel_websocket_prompt_submit_acks_while_provider_launch_is_initializing() {
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
        let attachment_id =
            response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
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
                    provider: "slow-structured".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
        let submit_response = wait_for_response_with_timeout(
            &mut socket,
            "submit-prompt",
            Duration::from_millis(250),
        )
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
            response_variant(&state_response, "SessionState")["session"]["prompt_states"]
                [&agent_id]["active_prompt"]
                .is_object(),
            "queued prompt should start after provider launch finishes: {state_response}"
        );

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}

#[test]
fn kernel_websocket_prompt_submit_acks_while_shell_capability_is_slow() {
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
        let attachment_id =
            response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
                    args: vec!["-c".to_string(), "sleep 2".to_string()],
                    working_directory: None,
                    timeout_ms: Some(3_000),
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
        let submit_response = wait_for_first_response(
            &mut socket,
            &["submit-prompt", "slow-shell"],
            Duration::from_secs(3),
        )
        .await;
        assert_eq!(
            submit_response["request_id"].as_str(),
            Some("submit-prompt"),
            "prompt should respond before the shell capability completes: {submit_response}"
        );
        assert!(
            response_variant(&submit_response, "PromptSubmitted")["outcome"]["Started"].is_object(),
            "prompt should start before the shell capability completes: {submit_response}"
        );

        let shell_response =
            wait_for_response_with_timeout(&mut socket, "slow-shell", Duration::from_secs(10))
                .await;
        assert!(
            response_variant(&shell_response, "ShellCommandCompleted")["result"].is_object(),
            "shell capability should still complete: {shell_response}"
        );

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}
