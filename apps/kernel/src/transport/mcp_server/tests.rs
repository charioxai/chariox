use std::sync::Arc;

use http_body_util::BodyExt;
use hyper::StatusCode;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::agent::CreateAgentRequest;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::runtime::router::CommandRouter;
use crate::session::CreateSessionRequest;
use crate::{DaemonApp, DaemonConfig};

use super::handle_json_rpc_value;

fn run_mcp_server_large_stack_test<Fut>(name: &str, test: fn() -> Fut)
where
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("mcp server test runtime should build")
                .block_on(test());
        })
        .expect("mcp server test thread should spawn")
        .join()
        .expect("mcp server test thread should not panic");
}

mod slice_tools;

#[tokio::test]
async fn mcp_initialize_and_tools_list_return_runtime_tools() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));

    let initialize = handle_json_rpc_value(
        router.clone(),
        "unused-token",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26"
            }
        }),
    )
    .await
    .expect("initialize should succeed");
    assert_eq!(initialize.status(), StatusCode::OK);
    let initialize_body = initialize
        .into_body()
        .collect()
        .await
        .expect("initialize body should collect")
        .to_bytes();
    let initialize_value: Value =
        serde_json::from_slice(&initialize_body).expect("initialize body should be json");
    assert_eq!(
        initialize_value["result"]["serverInfo"]["name"],
        "arroba-runtime"
    );

    let tools_list = handle_json_rpc_value(
        router.clone(),
        "unused-token",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await
    .expect("tools/list should succeed");
    assert_eq!(tools_list.status(), StatusCode::OK);
    let tools_body = tools_list
        .into_body()
        .collect()
        .await
        .expect("tools list body should collect")
        .to_bytes();
    let tools_value: Value =
        serde_json::from_slice(&tools_body).expect("tools list body should be json");
    let tools = tools_value["result"]["tools"]
        .as_array()
        .expect("tools should be an array");
    assert!(tools.iter().any(|tool| tool["name"] == "ack_workflow_turn"));
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "validate_workflow_handoff")
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "workflow_console_read")
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "workflow_console_write")
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "workflow_console_clear")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "arroba.read_artifact")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "arroba.list_extensions")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "arroba.request_extension")
    );
    assert!(!tools.iter().any(|tool| tool["name"] == "list_extensions"));
    assert!(!tools.iter().any(|tool| tool["name"] == "request_extension"));
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "arroba.list_credential_handles")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "list_credential_handles")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "arroba.http_request_with_credential")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "http_request_with_credential")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "arroba.send_secret_to_terminal")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "send_secret_to_terminal")
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "arroba.slice_screenshot")
    );
}

#[tokio::test]
async fn mcp_resource_and_prompt_discovery_return_empty_lists() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));

    for (id, method, result_key) in [
        (1, "resources/list", "resources"),
        (2, "resources/templates/list", "resourceTemplates"),
        (3, "prompts/list", "prompts"),
    ] {
        let response = handle_json_rpc_value(
            router.clone(),
            "unused-token",
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": {}
            }),
        )
        .await
        .expect("discovery request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("discovery body should collect")
            .to_bytes();
        let value: Value = serde_json::from_slice(&body).expect("discovery body json");
        assert_eq!(value["id"], id);
        assert_eq!(
            value["result"][result_key]
                .as_array()
                .expect("discovery result should be an array")
                .len(),
            0
        );
    }
}

#[tokio::test]
async fn mcp_http_tools_call_acknowledges_active_workflow_turn() {
    let mut config = DaemonConfig::for_tests();
    config.user_config.providers.workspace_live_sync.mode =
        crate::config::WorkspaceLiveSyncMode::Tracked;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
        .expect("session should exist");
    crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree("worktree-1"),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    let (workflow_run, _, _) = app
        .invoke_workflow_endpoint_and_schedule(
            session.id(),
            &workflow_id,
            "entry",
            Some("start".to_string()),
        )
        .expect("workflow should invoke");
    let node_run = workflow_run
        .node_runs()
        .first()
        .expect("node run should exist");
    let envelope = node_run.turn_envelope().expect("envelope should exist");
    let auth_token = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("provider run should exist")
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "ack_workflow_turn",
                "arguments": {
                    "delivery_token": envelope.delivery_token(),
                }
            }
        }),
    )
    .await
    .expect("mcp request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("body should be json");
    assert_eq!(
        value["result"]["structuredContent"]["state"],
        "acknowledged"
    );
}

#[test]
fn mcp_http_tools_call_reads_and_edits_workspace_live_sync_artifact() {
    run_mcp_server_large_stack_test(
        "mcp-http-tools-call-reads-and-edits-workspace-live-sync-artifact",
        mcp_http_tools_call_reads_and_edits_workspace_live_sync_artifact_inner,
    );
}

async fn mcp_http_tools_call_reads_and_edits_workspace_live_sync_artifact_inner() {
    let root = std::env::temp_dir().join(format!(
        "arroba-workspace-live-sync-mcp-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("test root should be created");
    std::fs::write(root.join("notes.txt"), "alpha\nbeta\n").expect("file should be written");

    let mut config = DaemonConfig::for_tests();
    config.user_config.providers.workspace_live_sync.mode =
        crate::config::WorkspaceLiveSyncMode::Tracked;
    let mut app = DaemonApp::bootstrap(config).expect("daemon should boot");
    let worktree = root.to_string_lossy().to_string();
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace-1", &worktree))
        .expect("session should exist");
    crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(
            session.id(),
            "mcp-workspace-live-sync-test",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
    let agent_id = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree(&worktree),
        )
        .expect("agent should spawn")
        .id()
        .to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    app.invoke_workflow_endpoint_and_schedule(
        session.id(),
        &workflow_id,
        "entry",
        Some("start".to_string()),
    )
    .expect("workflow should invoke");
    let auth_token = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("provider run should exist")
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let read_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "arroba.read_artifact",
                "arguments": {
                    "path": "notes.txt"
                }
            }
        }),
    )
    .await
    .expect("read request should succeed");
    assert_eq!(read_response.status(), StatusCode::OK);
    let read_body = read_response
        .into_body()
        .collect()
        .await
        .expect("read body should collect")
        .to_bytes();
    let read_value: Value = serde_json::from_slice(&read_body).expect("read body json");
    assert_eq!(
        read_value["result"]["structuredContent"]["content_text"],
        "alpha\nbeta\n"
    );
    assert_eq!(
        read_value["result"]["structuredContent"]["workspace"]["identity_changed"],
        false
    );
    let snapshot_id = read_value["result"]["structuredContent"]["snapshot_id"]
        .as_str()
        .expect("snapshot id should be present")
        .to_string();
    let edit_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "arroba.edit_artifact",
                "arguments": {
                    "path": "notes.txt",
                    "snapshot_id": snapshot_id,
                    "old_text": "beta",
                    "new_text": "gamma"
                }
            }
        }),
    )
    .await
    .expect("edit request should succeed");
    assert_eq!(edit_response.status(), StatusCode::OK);
    let edit_body = edit_response
        .into_body()
        .collect()
        .await
        .expect("edit body should collect")
        .to_bytes();
    let edit_value: Value = serde_json::from_slice(&edit_body).expect("edit body json");
    assert_eq!(edit_value["result"]["structuredContent"]["applied"], true);
    assert_eq!(
        edit_value["result"]["structuredContent"]["workspace"]["identity_changed"],
        false
    );
    assert_eq!(
        edit_value["result"]["structuredContent"]["change"]["kind"],
        "update"
    );
    assert!(
        edit_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("edit diff should be present")
            .contains("-beta")
    );
    assert!(
        edit_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("edit diff should be present")
            .contains("+gamma")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notes.txt")).expect("file should be readable"),
        "alpha\ngamma\n"
    );

    let patch_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "arroba.apply_patch",
                "arguments": {
                    "patch_text": "*** Begin Patch\n*** Update File: notes.txt\n@@\n alpha\n-gamma\n+delta\n*** End Patch"
                }
            }
        }),
    )
    .await
    .expect("patch request should succeed");
    assert_eq!(patch_response.status(), StatusCode::OK);
    let patch_body = patch_response
        .into_body()
        .collect()
        .await
        .expect("patch body should collect")
        .to_bytes();
    let patch_value: Value = serde_json::from_slice(&patch_body).expect("patch body json");
    assert_eq!(patch_value["result"]["structuredContent"]["applied"], true);
    assert!(
        patch_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("patch diff should be present")
            .contains("+delta")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notes.txt")).expect("file should be readable"),
        "alpha\ndelta\n"
    );

    let write_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "arroba.write_artifact",
                "arguments": {
                    "path": "created.txt",
                    "content_text": "created through arroba\n"
                }
            }
        }),
    )
    .await
    .expect("write request should succeed");
    assert_eq!(write_response.status(), StatusCode::OK);
    let write_body = write_response
        .into_body()
        .collect()
        .await
        .expect("write body should collect")
        .to_bytes();
    let write_value: Value = serde_json::from_slice(&write_body).expect("write body json");
    assert_eq!(write_value["result"]["structuredContent"]["applied"], true);
    assert_eq!(
        write_value["result"]["structuredContent"]["change"]["kind"],
        "add"
    );
    assert!(
        write_value["result"]["structuredContent"]["change"]["diff"]
            .as_str()
            .expect("write diff should be present")
            .contains("+created through arroba")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("created.txt")).expect("file should be readable"),
        "created through arroba\n"
    );

    let move_delete_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "arroba.apply_patch",
                "arguments": {
                    "patch_text": "*** Begin Patch\n*** Update File: notes.txt\n*** Move to: archive/notes.txt\n@@\n-alpha\n+omega\n delta\n*** Delete File: created.txt\n*** End Patch"
                }
            }
        }),
    )
    .await
    .expect("move/delete patch request should succeed");
    assert_eq!(move_delete_response.status(), StatusCode::OK);
    let move_delete_body = move_delete_response
        .into_body()
        .collect()
        .await
        .expect("move/delete body should collect")
        .to_bytes();
    let move_delete_value: Value =
        serde_json::from_slice(&move_delete_body).expect("move/delete body json");
    assert_eq!(
        move_delete_value["result"]["structuredContent"]["applied"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(root.join("archive/notes.txt"))
            .expect("moved file should be readable"),
        "omega\ndelta\n"
    );
    assert!(!root.join("notes.txt").exists());
    assert!(!root.join("created.txt").exists());

    let rejected_patch_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "arroba.apply_patch",
                "arguments": {
                    "patch_text": "*** Begin Patch\n*** Add File: should-not-exist.txt\n+nope\n*** Update File: archive/notes.txt\n@@\n-missing\n+bad\n*** End Patch"
                }
            }
        }),
    )
    .await
    .expect("rejected patch request should return a tool result");
    assert_eq!(rejected_patch_response.status(), StatusCode::OK);
    let rejected_patch_body = rejected_patch_response
        .into_body()
        .collect()
        .await
        .expect("rejected patch body should collect")
        .to_bytes();
    let rejected_patch_value: Value =
        serde_json::from_slice(&rejected_patch_body).expect("rejected patch body json");
    assert_eq!(
        rejected_patch_value["result"]["structuredContent"]["applied"],
        false
    );
    assert!(!root.join("should-not-exist.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("archive/notes.txt"))
            .expect("moved file should remain unchanged"),
        "omega\ndelta\n"
    );

    let direct_move_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "arroba.move_artifact",
                "arguments": {
                    "from_path": "archive/notes.txt",
                    "to_path": "final.txt",
                    "old_text": "omega",
                    "new_text": "final"
                }
            }
        }),
    )
    .await
    .expect("direct move request should succeed");
    assert_eq!(direct_move_response.status(), StatusCode::OK);
    let direct_move_body = direct_move_response
        .into_body()
        .collect()
        .await
        .expect("direct move body should collect")
        .to_bytes();
    let direct_move_value: Value =
        serde_json::from_slice(&direct_move_body).expect("direct move body json");
    assert_eq!(
        direct_move_value["result"]["structuredContent"]["applied"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(root.join("final.txt")).expect("direct moved file should read"),
        "final\ndelta\n"
    );
    assert!(!root.join("archive/notes.txt").exists());

    let direct_delete_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "arroba.delete_artifact",
                "arguments": {
                    "path": "final.txt"
                }
            }
        }),
    )
    .await
    .expect("direct delete request should succeed");
    assert_eq!(direct_delete_response.status(), StatusCode::OK);
    let direct_delete_body = direct_delete_response
        .into_body()
        .collect()
        .await
        .expect("direct delete body should collect")
        .to_bytes();
    let direct_delete_value: Value =
        serde_json::from_slice(&direct_delete_body).expect("direct delete body json");
    assert_eq!(
        direct_delete_value["result"]["structuredContent"]["applied"],
        true
    );
    assert_eq!(
        direct_delete_value["result"]["structuredContent"]["change"]["kind"],
        "delete"
    );
    assert!(!root.join("final.txt").exists());
}

#[tokio::test]
async fn mcp_http_tools_call_lists_and_requests_capabilities() {
    let root = std::env::temp_dir().join(format!(
        "arroba-capability-mcp-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join(".arroba").join("skills").join("browser-qa"))
        .expect("skill root should be created");
    std::fs::write(
        root.join(".arroba")
            .join("skills")
            .join("browser-qa")
            .join("SKILL.md"),
        "---\nname: browser-qa\ndescription: Browser QA\n---\nUse the browser.\n",
    )
    .expect("skill should be written");
    let mcp_registry = crate::mcp::ArrobaMcpRegistry::new(vec![root.join(".arroba").join("mcps")]);
    mcp_registry
        .install(&crate::mcp::ArrobaMcpServerConfig::stdio(
            "browser",
            "npx",
            vec!["@playwright/mcp".to_string()],
        ))
        .expect("mcp should install");

    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
    let workspace = root.to_string_lossy().to_string();
    let (session, _default_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(&workspace, &workspace))
        .expect("session should exist");
    let agent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("agent-a")
                .with_model("test-model")
                .with_worktree(&workspace),
        )
        .expect("agent should spawn");
    let agent_id = agent.id().to_string();
    let agent_ref = agent.agent_ref().to_string();
    let workflow_id = app
        .sessions_mut()
        .create_workflow(session.id(), Some("wf".to_string()))
        .expect("workflow should exist")
        .id()
        .to_string();
    let node_id = app
        .sessions_mut()
        .add_workflow_node(session.id(), &workflow_id, &agent_id)
        .expect("node should be added")
        .id()
        .to_string();
    app.sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            &workflow_id,
            &node_id,
            Some("entry".to_string()),
        )
        .expect("endpoint should exist");
    app.invoke_workflow_endpoint_and_schedule(
        session.id(),
        &workflow_id,
        "entry",
        Some("start".to_string()),
    )
    .expect("workflow should invoke");
    let auth_token = app
        .providers()
        .get_run_for_agent(session.id(), &agent_id)
        .expect("provider run should exist")
        .runtime_mcp_auth_token()
        .expect("mcp auth token should exist")
        .to_string();

    let app = Arc::new(Mutex::new(app));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app.clone(), 8));
    let list_response = handle_json_rpc_value(
        router.clone(),
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "arroba.list_extensions",
                "arguments": {"kind": "all"}
            }
        }),
    )
    .await
    .expect("list request should succeed");
    let list_body = list_response
        .into_body()
        .collect()
        .await
        .expect("list body should collect")
        .to_bytes();
    let list_value: Value = serde_json::from_slice(&list_body).expect("list body json");
    assert_eq!(
        list_value["result"]["structuredContent"]["agent_ref"],
        agent_ref
    );
    assert_eq!(
        list_value["result"]["structuredContent"]["extensions"]["mcps"][0]["name"],
        "browser"
    );
    assert_eq!(
        list_value["result"]["structuredContent"]["extensions"]["skills"][0]["name"],
        "browser-qa"
    );

    let request_response = handle_json_rpc_value(
        router,
        &auth_token,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "arroba.request_extension",
                "arguments": {"kind": "skill", "name": "browser-qa"}
            }
        }),
    )
    .await
    .expect("request capability should succeed");
    let request_body = request_response
        .into_body()
        .collect()
        .await
        .expect("request body should collect")
        .to_bytes();
    let request_value: Value = serde_json::from_slice(&request_body).expect("request body json");
    assert_eq!(
        request_value["result"]["structuredContent"]["granted"],
        true
    );
    assert_eq!(
        request_value["result"]["structuredContent"]["effective"],
        "now"
    );
    assert_eq!(
        request_value["result"]["structuredContent"]["requires_provider_restart"],
        false
    );
    assert!(
        request_value["result"]["structuredContent"]["skill"]["body"]
            .as_str()
            .expect("skill body should be returned")
            .contains("Use the browser.")
    );
    let agent = app
        .lock()
        .await
        .agents()
        .get_agent(&agent_id)
        .expect("agent should exist");
    assert_eq!(agent.skill_grants(), vec!["browser-qa".to_string()]);

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn mcp_tools_call_rejects_invalid_auth_token() {
    let app = Arc::new(Mutex::new(
        DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot"),
    ));
    let router = Arc::new(CommandRouter::with_interactive_capacity(app, 8));
    let response = handle_json_rpc_value(
        router,
        "invalid-token",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "ack_workflow_turn",
                "arguments": {
                    "delivery_token": "workflow-ack:missing"
                }
            }
        }),
    )
    .await
    .expect("request should return a json-rpc response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();
    let value: Value = serde_json::from_slice(&body).expect("body should be json");
    assert_eq!(value["error"]["code"], -32000);
}
