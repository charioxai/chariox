use std::time::Duration;

use arroba_kernel::attachment::ClientCapabilityLevel;
use arroba_kernel::local::{
    AttachToSessionRequest, CancelActivePromptRequest, FocusAgentRequest,
    GetProviderCatalogRequest, GetSessionHistoryOutlineRequest, GetSessionStateRequest,
    LaunchProviderRunRequest, ListProviderProcessesRequest, LocalDaemonRequest,
    PumpTerminalOutputRequest, ResizeTerminalRequest, RunShellCapabilityRequest, SpawnAgentRequest,
    SubmitPromptRequest,
};
use arroba_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use arroba_kernel::session::CreateSessionRequest;
use arroba_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;
use tokio::time::sleep;

mod support;

use support::kernel_websocket::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_history_read_is_slow() {
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
        wait_for_response_with_timeout(&mut socket, "slow-history", Duration::from_secs(10)).await;
    assert!(
        response_variant(&history_response, "SessionHistoryOutline")["agents"].is_array(),
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_replayed_prompt_submit_reuses_original_prompt() {
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
    let attachment_id = response_variant(&attach_response, "SessionAttached")["attachment"]["id"]
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_provider_process_list_is_slow() {
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_provider_launch_is_initializing() {
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
            target_agent_id: None,
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
            structured_endpoint: None,
            provider_session_id: None,
            native_tui: false,
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kernel_websocket_prompt_submit_acks_while_shell_capability_is_slow() {
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
        wait_for_response_with_timeout(&mut socket, "slow-shell", Duration::from_secs(10)).await;
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
