use std::time::Duration;

use crate::support::kernel_websocket::*;
use chariox_kernel::attachment::ClientCapabilityLevel;
use chariox_kernel::local::{
    AttachToSessionRequest, CancelActivePromptRequest, FocusAgentRequest, GetSessionStateRequest,
    LaunchProviderRunRequest, LocalDaemonRequest, PumpTerminalOutputRequest, ResizeTerminalRequest,
    SpawnAgentRequest, SubmitPromptRequest,
};
use chariox_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use chariox_kernel::session::CreateSessionRequest;
use chariox_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::sleep;

#[test]
fn kernel_websocket_state_and_cancel_ack_while_structured_provider_io_is_slow() {
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
        let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]
            ["id"]
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
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
                provider: Some("dev-stub".to_string()),
                model: Some("sonnet".to_string()),
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
                structured_endpoint: None,
                provider_session_id: None,
                native_tui: false,
            }),
        )
        .await;
        let second_provider_run_id =
            provider_run_id_from_launch_response(&second_provider_response);
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
            response_variant(&state_response, "SessionState")["session"]["prompt_states"]
                [&agent_id]["active_prompt"]
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
                provider_run_id: None,
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
                prompt: "second agent prompt should ack during another run's provider I/O"
                    .to_string(),
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
                target_agent_id: None,
            }),
        )
        .await;
        assert!(
            response_variant(&cancel_response, "PromptCancelled")["cancellation"]["prompt"]
                ["status"]
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
                provider_run_id: None,
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
    });
}
