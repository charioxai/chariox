use crate::support::kernel_websocket::*;
use chariox_kernel::attachment::ClientCapabilityLevel;
use chariox_kernel::local::{
    AddWorkflowNodeRequest, AttachToSessionRequest, CreateWorkflowEndpointRequest,
    CreateWorkflowRequest, DeleteSessionRequest, InvokeWorkflowEndpointRequest, LocalDaemonRequest,
    SpawnAgentRequest,
};
use chariox_kernel::runtime_transport::run_kernel_websocket_server_on_listener;
use chariox_kernel::session::CreateSessionRequest;
use chariox_kernel::{DaemonApp, DaemonConfig};
use serde_json::json;
use tokio::sync::oneshot;

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

    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "delete-session",
            "request": LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: session_id,
                workspace_id: Some("workspace-kernel-ws".to_string()),
            }),
        }),
    )
    .await;
    let (_delete_response, unavailable_event) =
        wait_for_response_and_event(&mut socket, "delete-session", "session_unavailable").await;
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
            account_profile: None,
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
    send_frame(
        &mut socket,
        json!({
            "type": "request",
            "request_id": "invoke-workflow-endpoint",
            "request": LocalDaemonRequest::InvokeWorkflowEndpoint(InvokeWorkflowEndpointRequest {
                session_id: session_id.clone(),
                workflow_ref: workflow_id.clone(),
                endpoint_ref: endpoint_id,
                prompt: Some("stream workflow run update".to_string()),
                queue_ref: None,
                publication_invocation: None,
            }),
        }),
    )
    .await;
    let (invoke_response, run_update_event) = wait_for_response_and_event(
        &mut socket,
        "invoke-workflow-endpoint",
        "workflow_run_updated",
    )
    .await;
    let expected_run_id = response_variant(&invoke_response, "WorkflowRunInvoked")["workflow_run"]
        ["id"]
        .as_str()
        .expect("workflow run id should be present")
        .to_string();

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
