use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::Mutex;

fn runtime_state_from_app(app: DaemonApp) -> KernelRuntimeState {
    let config_projection = app.config_projection_store();
    let session_store = app.session_state_store();
    let agent_store = app.agents().clone();
    let attachment_store = app.attachments().clone();
    let provider_store = app.providers().clone();
    let provider_process_tracking = app.provider_process_tracking_store();
    let slice_store = app.slices();
    let session_projection = app.session_state_projection_store();
    let provider_run_projection = app.provider_run_projection_store();
    let operational_history_store = app.operational_history_store();
    let durable_state_store = app.durable_state_store();
    let prompt_state_owner = app.prompt_state_owner();
    let active_turns = app.active_turn_store();
    let prompt_activity = app.prompt_activity_store();
    let prompt_workspace_claims = app.prompt_workspace_claim_store();
    let structured_output_records = app.structured_output_record_store();
    let terminal_stream = app.terminal_stream_store();
    let workflow_design_events = app.workflow_design_event_store();
    let metaagent_events = app.metaagent_event_store();
    let workspace_coordinator = app.workspace_coordinator();
    KernelRuntimeState::new_with_owned_state(
        Arc::new(Mutex::new(app)),
        config_projection,
        session_store,
        agent_store,
        attachment_store,
        provider_store,
        provider_process_tracking,
        slice_store,
        session_projection,
        provider_run_projection,
        operational_history_store,
        durable_state_store,
        prompt_state_owner,
        active_turns,
        prompt_activity,
        prompt_workspace_claims,
        structured_output_records,
        terminal_stream,
        workflow_design_events,
        metaagent_events,
        workspace_coordinator,
    )
}

#[test]
fn workflow_turn_context_lists_outgoing_edge_options() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, router_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace",
            "worktree",
        ))
        .expect("session should be created");
    let worker_a = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker-a"),
        )
        .expect("worker a should spawn");
    let worker_b = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias("worker-b"),
        )
        .expect("worker b should spawn");

    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("routing".to_string()))
        .expect("workflow should be created");
    app.sessions_mut()
        .apply_workflow_design_op(
            session.id(),
            crate::local::WorkflowDesignOp::SchemaAdd {
                workflow_id: workflow.id().to_string(),
                schema: crate::session::WorkflowSchemaDefinition::new(
                    "final-answer",
                    Some("Specialist answer".to_string()),
                    Some("Final answer returned by a specialist".to_string()),
                    serde_json::json!({
                        "type": "object",
                        "required": ["answer", "specialist"],
                        "properties": {
                            "answer": { "type": "string" },
                            "specialist": { "type": "integer" }
                        },
                        "additionalProperties": false
                    }),
                ),
            },
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
        )
        .expect("embedded final output schema should be added");
    app.sessions_mut()
        .set_workflow_run_output_schema_ref(
            session.id(),
            workflow.id(),
            Some("final-answer".to_string()),
        )
        .expect("run output schema should be selected");
    let router = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            workflow.id(),
            router_agent.id(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            "Router".to_string(),
        )
        .expect("router node should be added");
    let node_a = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            workflow.id(),
            worker_a.id(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            "Worker A".to_string(),
        )
        .expect("worker a node should be added");
    app.sessions_mut()
        .update_workflow_node_instructions(
            session.id(),
            workflow.id(),
            node_a.id(),
            Some("Handle policy-sensitive routing tasks only.".to_string()),
        )
        .expect("worker a instructions should update");
    let node_b = app
        .sessions_mut()
        .add_workflow_node_owned(
            session.id(),
            workflow.id(),
            worker_b.id(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            crate::session::DEFAULT_LOCAL_USER_ID.to_string(),
            "Worker B".to_string(),
        )
        .expect("worker b node should be added");
    app.sessions_mut()
        .update_workflow_node_instructions(
            session.id(),
            workflow.id(),
            node_b.id(),
            Some("Handle quality and completeness routing tasks only.".to_string()),
        )
        .expect("worker b instructions should update");
    let edge_a = app
        .sessions_mut()
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            node_a.id(),
            Some("/schemas/route-a.json".to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Halt),
        )
        .expect("edge a should be added");
    let edge_b = app
        .sessions_mut()
        .add_workflow_edge(
            session.id(),
            workflow.id(),
            router.id(),
            node_b.id(),
            Some("/schemas/route-b.json".to_string()),
            Some(crate::session::WorkflowHandoffValidationPolicy::Warn),
        )
        .expect("edge b should be added");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            router.id(),
            Some("entry".to_string()),
        )
        .expect("endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("route this".to_string()),
        )
        .expect("workflow run should be created");
    let workflow_run_id = workflow_run.id().to_string();
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    app.sessions_mut()
        .start_workflow_node_run(session.id(), &workflow_run_id, &node_run_id)
        .expect("router node run should start");

    let runtime = runtime_state_from_app(app);
    let context = runtime
        .owned
        .workflow_tool_context(
            session.id().to_string(),
            workflow_run_id,
            node_run_id.clone(),
            Some("delivery-token".to_string()),
        )
        .expect("workflow tool context should resolve");
    let (result, dispatches) = runtime
        .owned
        .dispatch_workflow_runtime_tool_call(
            crate::transport::runtime_tools::READ_WORKFLOW_TURN_CONTEXT_TOOL.to_string(),
            serde_json::json!({}),
            context,
        )
        .expect("read workflow context should succeed");

    assert!(dispatches.local.is_empty());
    assert!(dispatches.remote.is_empty());
    assert!(dispatches.starting_provider_runs.is_empty());
    assert!(result.ok);
    let outgoing = result.payload["outgoing_edges"]
        .as_array()
        .expect("outgoing edges should be an array");
    assert_eq!(outgoing.len(), 2);
    let option_a = outgoing
        .iter()
        .find(|edge| edge["edge_id"] == edge_a.id())
        .expect("edge a should be present");
    assert_eq!(option_a["to_node_id"], node_a.id());
    assert_eq!(option_a["to_node_public_label"], "Worker A");
    assert_eq!(
        option_a["target_instructions"],
        "Handle policy-sensitive routing tasks only."
    );
    assert_eq!(option_a["to_agent_id"], worker_a.id());
    assert_eq!(option_a["handoff_schema_ref"], "/schemas/route-a.json");
    assert_eq!(option_a["validation_policy"], "halt");
    let option_b = outgoing
        .iter()
        .find(|edge| edge["edge_id"] == edge_b.id())
        .expect("edge b should be present");
    assert_eq!(option_b["to_node_id"], node_b.id());
    assert_eq!(option_b["to_node_public_label"], "Worker B");
    assert_eq!(
        option_b["target_instructions"],
        "Handle quality and completeness routing tasks only."
    );
    assert_eq!(option_b["to_agent_id"], worker_b.id());
    assert_eq!(option_b["handoff_schema_ref"], "/schemas/route-b.json");
    assert_eq!(option_b["validation_policy"], "warn");
    assert_eq!(
        result.payload["handoff_routing"]["final_json_field"],
        "output.message.workflow_handoffs"
    );
    assert_eq!(
        result.payload["run_output_contract"],
        serde_json::Value::Null
    );
    assert!(result.payload["handoff_routing"]["select_by"]
        .as_array()
        .expect("select_by should be an array")
        .iter()
        .any(|value| value == "edge_id"));
    assert_eq!(result.payload["workflow_node_run_id"], node_run_id);
}

#[test]
fn workflow_run_output_contract_is_null_without_a_schema() {
    let workflow = crate::session::WorkflowDefinition::new("workflow-1", None);
    assert_eq!(
        workflow_run_output_contract(&workflow, true),
        serde_json::Value::Null
    );
}

#[test]
fn workflow_run_output_contract_is_null_for_nodes_that_cannot_complete() {
    let mut workflow = crate::session::WorkflowDefinition::new("workflow-1", None);
    workflow.add_schema(crate::session::WorkflowSchemaDefinition::new(
        "final-answer",
        None,
        None,
        serde_json::json!({ "type": "object" }),
    ));
    workflow.set_run_output_schema_ref(Some("final-answer".to_string()));

    assert_eq!(
        workflow_run_output_contract(&workflow, false),
        serde_json::Value::Null
    );
}

#[test]
fn workflow_run_output_contract_exposes_embedded_schema_to_completing_nodes() {
    let mut workflow = crate::session::WorkflowDefinition::new("workflow-1", None);
    workflow.add_schema(crate::session::WorkflowSchemaDefinition::new(
        "final-answer",
        Some("Specialist answer".to_string()),
        Some("Final answer returned by a specialist".to_string()),
        serde_json::json!({
            "type": "object",
            "required": ["answer", "specialist"],
            "properties": {
                "answer": { "type": "string" },
                "specialist": { "type": "integer" }
            },
            "additionalProperties": false
        }),
    ));
    workflow.set_run_output_schema_ref(Some("final-answer".to_string()));

    let contract = workflow_run_output_contract(&workflow, true);
    assert_eq!(contract["schema_ref"], "final-answer");
    assert_eq!(contract["source"], "embedded");
    assert_eq!(
        contract["schema"]["required"],
        serde_json::json!(["answer", "specialist"])
    );
    assert_eq!(contract["schema"]["properties"]["answer"]["type"], "string");
    assert_eq!(contract["schema"]["additionalProperties"], false);
}

#[test]
fn workflow_run_output_contract_does_not_read_external_refs() {
    let mut workflow = crate::session::WorkflowDefinition::new("workflow-1", None);
    workflow.set_run_output_schema_ref(Some("/private/host/final-output.json".to_string()));

    let contract = workflow_run_output_contract(&workflow, true);

    assert_eq!(contract["schema_ref"], "/private/host/final-output.json");
    assert_eq!(contract["source"], "external_ref");
    assert!(contract.get("schema").is_none());
}

#[test]
fn agent_app_http_action_forwards_invocation_context_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind action server");
    let address = listener.local_addr().expect("action server address");
    let (sender, receiver) = mpsc::channel::<String>();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept action request");
        let request_text = read_http_request(&mut stream);
        sender.send(request_text).expect("send captured request");
        let body = b"{\"ok\":true}";
        stream
                .write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    String::from_utf8_lossy(body),
                ).as_bytes())
                .expect("write response");
    });

    let response = call_agent_app_http_action(
        &format!("http://{address}/cart/add"),
        "POST",
        &serde_json::json!({"sku": "banana"}),
        &AgentAppHttpActionContext {
            action_id: "cart.add".to_string(),
            session: Some("session-a".to_string()),
            invocation_request_id: Some("invocation-a".to_string()),
            audit: None,
        },
        AgentAppHttpActionOptions {
            allow_external: false,
            timeout_ms: 30_000,
            max_response_bytes: 1_048_576,
        },
    )
    .expect("action response");

    assert_eq!(response.status, 200);
    let captured = receiver.recv().expect("captured request");
    assert!(captured.contains("x-arroba-agent-app-action-id: cart.add"));
    assert!(captured.contains("x-arroba-agent-app-session: session-a"));
    assert!(captured.contains("x-arroba-publication-invocation: invocation-a"));
    handle.join().expect("action server thread");
}

#[test]
fn agent_app_http_action_rejects_external_urls_by_default() {
    let error = call_agent_app_http_action(
        "https://example.com/cart/add",
        "POST",
        &serde_json::json!({"sku": "banana"}),
        &AgentAppHttpActionContext {
            action_id: "cart.add".to_string(),
            session: None,
            invocation_request_id: None,
            audit: None,
        },
        AgentAppHttpActionOptions {
            allow_external: false,
            timeout_ms: 1_000,
            max_response_bytes: 1_048_576,
        },
    )
    .expect_err("external URL should be rejected before network I/O");

    assert!(error.contains("allow_external=true"));
}

#[test]
fn agent_app_http_action_rejects_oversized_responses() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind action server");
    let address = listener.local_addr().expect("action server address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept action request");
        let _ = read_http_request(&mut stream);
        let body = "x".repeat(128);
        stream
                .write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                ).as_bytes())
                .expect("write response");
    });

    let error = call_agent_app_http_action(
        &format!("http://{address}/cart/add"),
        "POST",
        &serde_json::json!({"sku": "banana"}),
        &AgentAppHttpActionContext {
            action_id: "cart.add".to_string(),
            session: None,
            invocation_request_id: None,
            audit: None,
        },
        AgentAppHttpActionOptions {
            allow_external: false,
            timeout_ms: 30_000,
            max_response_bytes: 32,
        },
    )
    .expect_err("oversized response should be rejected");

    assert!(error.contains("exceeded 32 bytes"));
    handle.join().expect("action server thread");
}

#[test]
fn agent_app_action_audit_posts_deployment_log_entry() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind audit server");
    let address = listener.local_addr().expect("audit server address");
    let (sender, receiver) = mpsc::channel::<String>();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept audit request");
        let request_text = read_http_request(&mut stream);
        sender.send(request_text).expect("send captured request");
        let body = b"{\"accepted\":true}";
        stream
                .write_all(format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    String::from_utf8_lossy(body),
                ).as_bytes())
                .expect("write response");
    });

    send_agent_app_action_audit(
        &AgentAppHttpActionContext {
            action_id: "cart.checkout".to_string(),
            session: Some("session-a".to_string()),
            invocation_request_id: Some("invocation-a".to_string()),
            audit: Some(AgentAppActionAuditContext {
                url: format!("http://{address}/.well-known/arroba/agent-app/audit-log"),
                token: "audit-token".to_string(),
            }),
        },
        AgentAppActionAuditOutcome {
            ok: true,
            http_status: Some(200),
            duration_ms: Some(25),
            error: None,
        },
    );

    let captured = receiver.recv().expect("captured request");
    assert!(captured.contains("POST /.well-known/arroba/agent-app/audit-log"));
    assert!(captured.contains("\"token\":\"audit-token\""));
    assert!(captured.contains("\"kind\":\"agent_app_action\""));
    assert!(captured.contains("\"action_id\":\"cart.checkout\""));
    assert!(captured.contains("\"invocation_request_id\":\"invocation-a\""));
    assert!(captured.contains("\"http_status\":200"));
    handle.join().expect("audit server thread");
}

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set read timeout");
    let mut data = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream.read(&mut buffer).expect("read request chunk");
        if read == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..read]);
        if request_complete(&data) {
            break;
        }
    }
    String::from_utf8_lossy(&data).to_string()
}

fn request_complete(data: &[u8]) -> bool {
    let Some(header_end) = data.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let header_end = header_end + 4;
    let headers = String::from_utf8_lossy(&data[..header_end]).to_ascii_lowercase();
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    data.len() >= header_end + content_length
}
