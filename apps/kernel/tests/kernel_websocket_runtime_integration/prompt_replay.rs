use crate::support::kernel_websocket::*;
use arroba_kernel::attachment::ClientCapabilityLevel;
use arroba_kernel::local::{
    AttachToSessionRequest, GetSessionStateRequest, LaunchProviderRunRequest, LocalDaemonRequest,
    SubmitPromptRequest,
};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;

#[test]
fn kernel_websocket_replayed_prompt_submit_reuses_original_prompt() {
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

        let mut first_socket = connect_with_retry(&config.kernel_websocket_url()).await;
        let mut retry_socket = connect_with_retry(&config.kernel_websocket_url()).await;

        let create_response = send_request(
            &mut first_socket,
            "create-session-for-prompt-replay",
            LocalDaemonRequest::CreateSession(CreateSessionRequest::new(
                "workspace-prompt-replay",
                "worktree-prompt-replay",
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
            &mut first_socket,
            "attach-session-for-prompt-replay",
            LocalDaemonRequest::AttachToSession(AttachToSessionRequest {
                session_id: session_id.clone(),
                client_id: "ws-prompt-replay-client".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            }),
        )
        .await;
        let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]
            ["id"]
            .as_str()
            .expect("attachment id should be present")
            .to_string();

        let provider_response = send_request(
            &mut first_socket,
            "launch-provider-for-prompt-replay",
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
        wait_for_provider_run_state(&mut first_socket, &provider_run_id, "Running").await;

        let submit_request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
            session_id: session_id.clone(),
            attachment_id: attachment_id.clone(),
            target_agent_id: Some(agent_id.clone()),
            prompt: "replayed prompt submit must not duplicate user intent".to_string(),
            attachments: Vec::new(),
        });
        for (socket, request_id) in [
            (&mut first_socket, "submit-prompt-original"),
            (&mut retry_socket, "submit-prompt-retry"),
        ] {
            send_frame(
                socket,
                json!({
                    "type": "request",
                    "request_id": request_id,
                    "command_id": "stable-prompt-submit-command",
                    "request": submit_request.clone(),
                }),
            )
            .await;
        }

        let first_response = wait_for_response(&mut first_socket, "submit-prompt-original").await;
        let retry_response = wait_for_response(&mut retry_socket, "submit-prompt-retry").await;
        let first_prompt_id = response_variant(&first_response, "PromptSubmitted")["outcome"]
            ["Started"]["prompt"]["id"]
            .as_str()
            .expect("first prompt id should be present")
            .to_string();
        let retry_prompt_id = response_variant(&retry_response, "PromptSubmitted")["outcome"]
            ["Started"]["prompt"]["id"]
            .as_str()
            .expect("retry prompt id should be present");
        assert_eq!(first_prompt_id, retry_prompt_id);

        let state_response = send_request(
            &mut first_socket,
            "state-after-prompt-replay",
            LocalDaemonRequest::GetSessionState(GetSessionStateRequest {
                session_id: session_id.clone(),
            }),
        )
        .await;
        let session = &response_variant(&state_response, "SessionState")["session"];
        let agent_prompt_state = &session["prompt_states"][&agent_id];
        assert_eq!(
            agent_prompt_state["active_prompt"]["id"].as_str(),
            Some(first_prompt_id.as_str()),
            "replayed submit should leave one active prompt: {state_response}"
        );
        assert_eq!(
            agent_prompt_state["queued_prompts"]
                .as_array()
                .map(Vec::len)
                .unwrap_or_default(),
            0,
            "replayed submit must not enqueue a duplicate prompt: {state_response}"
        );

        let _ = shutdown_tx.send(());
        server
            .await
            .expect("kernel websocket task should join")
            .expect("kernel websocket server should shut down cleanly");
    });
}
