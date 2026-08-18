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
fn runtime_tool_snapshot_policy_keeps_external_context_tools_out_of_large_writes() {
    assert!(super::runtime_tool_requires_session_snapshot(
        crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL
    ));
    assert!(super::runtime_tool_requires_session_snapshot(
        crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
    ));
    assert!(super::runtime_tool_requires_session_snapshot(
        crate::transport::runtime_tools::WORKFLOW_CONSOLE_WRITE_TOOL
    ));
    assert!(!super::runtime_tool_requires_session_snapshot(
        crate::transport::runtime_tools::READ_WORKFLOW_TURN_CONTEXT_TOOL
    ));
    assert!(!super::runtime_tool_requires_session_snapshot(
        crate::transport::runtime_tools::EVENT_CONTEXT_TOOL
    ));
    assert!(!super::runtime_tool_requires_session_snapshot(
        crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL
    ));
}

#[test]
fn event_context_idempotency_fingerprint_scopes_request_parameters() {
    let first = super::event_context_request_fingerprint("thread", 20, None, None);
    let same_request = super::event_context_request_fingerprint("thread", 20, None, None);
    let next_page = super::event_context_request_fingerprint("thread", 20, Some("cursor-2"), None);
    let different_limit = super::event_context_request_fingerprint("thread", 50, None, None);
    let different_users =
        super::event_context_request_fingerprint("users", 20, None, Some(&[String::from("U123")]));

    assert_eq!(first, same_request);
    assert_ne!(first, next_page);
    assert_ne!(first, different_limit);
    assert_ne!(first, different_users);
}

#[test]
fn event_context_runtime_receipts_redact_provider_payloads() {
    let result = crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "result": {
                "messages": [{"text": "private conversation body"}],
                "users": [{"id": "U123", "profile": "private profile"}]
            }
        }),
    };
    let receipt = super::workflow_runtime_tool_result_json(
        crate::transport::runtime_tools::EVENT_CONTEXT_TOOL,
        &result,
    );

    assert!(receipt.contains("\"redacted\":true"));
    assert!(!receipt.contains("private conversation body"));
    assert!(!receipt.contains("private profile"));

    let mut envelope = crate::session::WorkflowTurnEnvelope::new(
        "workflow-ack:test",
        "mention".to_string(),
        None,
        None,
    );
    envelope.add_runtime_tool_call(crate::session::WorkflowRuntimeToolCallEvent::new(
        crate::transport::runtime_tools::EVENT_CONTEXT_TOOL,
        "{\"kind\":\"thread\"}",
        Some(receipt),
        true,
    ));
    let snapshot = serde_json::to_string(&envelope).expect("turn envelope should serialize");
    assert!(!snapshot.contains("private conversation body"));
    assert!(!snapshot.contains("private profile"));
}

#[test]
fn event_context_tool_is_discovered_without_reply_tool() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "event-context-workspace",
            "event-context-worktree",
        ))
        .expect("session should be created");
    let run = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(agent.id())
            .with_workflow_event_context(true),
        )
        .expect("provider should launch");
    app.providers()
        .enable_workflow_tools(run.id())
        .expect("workflow tools should be enabled");
    let auth_token = run
        .runtime_mcp_auth_token()
        .expect("provider should expose runtime MCP auth")
        .to_string();
    let runtime = runtime_state_from_app(app);
    let specs = runtime.runtime_tool_specs_for_auth_token(&auth_token);
    assert!(specs.iter().any(|spec| {
        spec.name == crate::transport::runtime_tools::EVENT_CONTEXT_TOOL_QUALIFIED
    }));
    assert!(!specs
        .iter()
        .any(|spec| { spec.name == crate::transport::runtime_tools::EVENT_ACTION_TOOL_QUALIFIED }));
    assert!(!specs.iter().any(|spec| {
        spec.name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL_QUALIFIED
    }));
}

#[test]
fn starting_workflow_prompt_persists_running_node_for_restart_recovery() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workflow-start-persistence-workspace",
            "workflow-start-persistence-worktree",
        ))
        .expect("session should be created");
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some("restart-persistence".to_string()))
        .expect("workflow should be created");
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be created");
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = app
        .sessions_mut()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("resume after restart".to_string()),
        )
        .expect("workflow run should be created");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    let delivery_token = format!("workflow-ack:{node_run_id}");
    app.sessions_mut()
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            delivery_token,
            "resume after restart".to_string(),
            None,
            None,
        )
        .expect("workflow turn should be prepared");
    let prompt = crate::session::PromptQueueItem::new(
        "workflow-start-persistence-prompt",
        crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
        agent.id(),
        "resume after restart",
        crate::session::PromptStatus::Running,
    )
    .with_workflow_context(workflow_run.id(), &node_run_id);

    let runtime = runtime_state_from_app(app);
    runtime
        .owned
        .workflow_start_prompt(session.id(), &prompt)
        .expect("workflow start should persist");

    let latest = runtime
        .owned
        .durable_state_store
        .load_subject_events_by_kind(session.id(), "session.updated", 10)
        .expect("workflow start event should load")
        .into_iter()
        .last()
        .expect("workflow start should append a session event");
    assert_eq!(latest.payload["reason"], "workflow_prompt_started");
    let persisted: crate::session::RuntimeSession =
        serde_json::from_value(latest.payload["session"].clone())
            .expect("persisted session should deserialize");
    let persisted_node = persisted
        .workflow_run(workflow_run.id())
        .expect("workflow run should persist")
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == node_run_id)
        .expect("workflow node run should persist");
    assert_eq!(
        persisted_node.status(),
        crate::session::WorkflowNodeRunStatus::Running
    );
    assert_eq!(
        persisted_node
            .turn_envelope()
            .expect("turn envelope should persist")
            .state(),
        crate::session::WorkflowTurnRuntimeState::Prepared
    );
}

#[test]
fn workflow_admission_replaces_idle_ordinary_provider_before_dispatch() {
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
        .expect("daemon bootstrap should succeed");
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workflow-idle-provider-workspace",
            "workflow-idle-provider-worktree",
        ))
        .expect("session should be created");
    let ordinary = app
        .launch_provider(
            crate::provider::LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "sonnet",
            )
            .with_agent_id(agent.id()),
        )
        .expect("ordinary provider should launch");
    let ordinary_auth_token = ordinary
        .runtime_mcp_auth_token()
        .expect("ordinary provider should expose runtime MCP auth")
        .to_string();
    let ordinary_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "ordinary-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .expect("ordinary attachment should create");
    let ordinary_prompt = crate::session::PromptQueueItem::new(
        "ordinary-turn-before-workflow",
        ordinary_attachment.id(),
        agent.id(),
        "complete this ordinary turn before workflow admission",
        crate::session::PromptStatus::Queued,
    );
    let ordinary_submission = app
        .prompt_owner_submit_prepared_prompt(session.id(), ordinary_prompt, false)
        .expect("ordinary prompt should submit");
    assert!(matches!(
        ordinary_submission,
        crate::session::PromptSubmissionOutcome::Started { .. }
    ));

    let runtime = runtime_state_from_app(app);
    let ordinary_specs = runtime.runtime_tool_specs_for_auth_token(&ordinary_auth_token);
    assert!(!ordinary_specs.iter().any(|spec| {
        spec.name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL_QUALIFIED
    }));
    runtime
        .owned
        .complete_local_prompt_without_advance(session.id(), agent.id(), Some(ordinary.id()))
        .expect("ordinary turn should settle");
    assert_eq!(
        runtime
            .owned
            .provider_store
            .get_run(ordinary.id())
            .expect("ordinary provider should remain addressable")
            .state(),
        crate::provider::ProviderRunState::Running
    );
    assert!(!runtime
        .owned
        .provider_run_has_active_prompt(
            session.id(),
            &runtime
                .owned
                .provider_store
                .get_run(ordinary.id())
                .expect("ordinary provider should resolve")
        )
        .expect("ordinary prompt state should resolve"));

    let workflow = runtime
        .owned
        .session_store
        .write()
        .create_workflow(session.id(), Some("idle-provider-admission".to_string()))
        .expect("workflow should be created");
    let node = runtime
        .owned
        .session_store
        .write()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .expect("workflow node should be created");
    let endpoint = runtime
        .owned
        .session_store
        .write()
        .create_workflow_endpoint(
            session.id(),
            workflow.id(),
            node.id(),
            Some("entry".to_string()),
        )
        .expect("workflow endpoint should be created");
    let workflow_run = runtime
        .owned
        .session_store
        .write()
        .invoke_workflow_endpoint(
            session.id(),
            workflow.id(),
            endpoint.id(),
            Some("run from the queued workflow head".to_string()),
        )
        .expect("workflow run should be created");

    let dispatches = runtime
        .owned
        .workflow_schedule_entry_node(session.id(), &workflow_run)
        .expect("workflow prompt should be admitted");
    let workflow_provider_id = dispatches
        .starting_provider_runs
        .first()
        .expect("workflow admission should start a workflow provider");
    let workflow_provider = runtime
        .owned
        .provider_store
        .get_run(workflow_provider_id)
        .expect("workflow provider should resolve");

    assert_ne!(workflow_provider.id(), ordinary.id());
    assert!(workflow_provider.workflow_tools_enabled());
    assert!(!workflow_provider.workflow_event_reply_enabled());
    let workflow_auth_token = workflow_provider
        .runtime_mcp_auth_token()
        .expect("workflow provider should expose runtime MCP auth")
        .to_string();
    let workflow_specs = runtime.runtime_tool_specs_for_auth_token(&workflow_auth_token);
    assert!(!workflow_specs.iter().any(|spec| {
        spec.name == crate::transport::runtime_tools::REPLY_TO_EVENT_TOOL_QUALIFIED
    }));
    assert!(matches!(
        runtime
            .owned
            .provider_store
            .get_run(ordinary.id())
            .expect("ordinary provider should remain addressable")
            .state(),
        crate::provider::ProviderRunState::Parked | crate::provider::ProviderRunState::Ended
    ));
}

#[test]
fn workflow_turn_context_lists_public_outgoing_edges_without_downstream_instructions() {
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
            workflow_run_id.clone(),
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
    let durable_events = runtime
        .owned
        .durable_state_store
        .load_subject_events_by_kind(session.id(), "session.updated", 10)
        .expect("durable session events should load");
    assert!(
        durable_events
            .iter()
            .all(|event| event.payload["reason"] != "workflow_runtime_tool"),
        "read-only workflow tools must not append a full session snapshot"
    );
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
    assert!(option_a.get("target_instructions").is_none());
    assert!(!option_a
        .to_string()
        .contains("Handle policy-sensitive routing tasks only."));
    assert_eq!(option_a["to_agent_id"], worker_a.id());
    assert_eq!(option_a["handoff_schema_ref"], "/schemas/route-a.json");
    assert_eq!(option_a["validation_policy"], "halt");
    let option_b = outgoing
        .iter()
        .find(|edge| edge["edge_id"] == edge_b.id())
        .expect("edge b should be present");
    assert_eq!(option_b["to_node_id"], node_b.id());
    assert_eq!(option_b["to_node_public_label"], "Worker B");
    assert!(option_b.get("target_instructions").is_none());
    assert!(!option_b
        .to_string()
        .contains("Handle quality and completeness routing tasks only."));
    assert_eq!(option_b["to_agent_id"], worker_b.id());
    assert_eq!(option_b["handoff_schema_ref"], "/schemas/route-b.json");
    assert_eq!(option_b["validation_policy"], "warn");
    let payload = result.payload.to_string();
    assert!(!payload.contains("Handle policy-sensitive routing tasks only."));
    assert!(!payload.contains("Handle quality and completeness routing tasks only."));
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
    assert!(captured.contains("x-chariox-agent-app-action-id: cart.add"));
    assert!(captured.contains("x-chariox-agent-app-session: session-a"));
    assert!(captured.contains("x-chariox-publication-invocation: invocation-a"));
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
                url: format!("http://{address}/.well-known/chariox/agent-app/audit-log"),
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
    assert!(captured.contains("POST /.well-known/chariox/agent-app/audit-log"));
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
