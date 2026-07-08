use super::*;

#[test]
fn workflow_code_apply_supports_multi_edge_routed_handoffs() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(
        &mut service,
        session.id(),
        &["router-agent", "worker-a-agent", "worker-b-agent"],
    );

    let definition = WorkflowCodeDefinition {
        schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: WorkflowCodeWorkflow {
            alias: Some("coded_router".to_string()),
            prompt: None,
            flush_agent_context_before_run: None,
            max_concurrent: Some(2),
            run_output_schema: None,
        },
        schemas: vec![
            WorkflowCodeSchemaDefinition {
                handle: "progress".to_string(),
                alias: Some("Progress event".to_string()),
                description: None,
                schema: serde_json::json!({
                    "type": "object",
                    "required": ["event", "status"],
                    "properties": {
                        "event": { "type": "string" },
                        "status": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
            WorkflowCodeSchemaDefinition {
                handle: "handoff".to_string(),
                alias: Some("Worker task".to_string()),
                description: None,
                schema: serde_json::json!({
                    "type": "object",
                    "required": ["task"],
                    "properties": {
                        "task": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
        ],
        nodes: vec![
            WorkflowCodeNodeDefinition {
                handle: "router".to_string(),
                agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                    alias: Some("router".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                public_label: Some("Router".to_string()),
                instructions: Some("Route the task to exactly one worker.".to_string()),
                can_complete_workflow_run: None,
                can_emit_intermediate_run_output: Some(true),
                wait_for_all_inputs: None,
                intermediate_output_schema: Some("progress".to_string()),
                max_turns: None,
                extensions: Vec::new(),
                canvas: None,
            },
            WorkflowCodeNodeDefinition {
                handle: "worker_a".to_string(),
                agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                    alias: Some("worker-a".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                public_label: Some("Worker A".to_string()),
                instructions: None,
                can_complete_workflow_run: Some(true),
                can_emit_intermediate_run_output: None,
                wait_for_all_inputs: None,
                intermediate_output_schema: None,
                max_turns: None,
                extensions: Vec::new(),
                canvas: None,
            },
            WorkflowCodeNodeDefinition {
                handle: "worker_b".to_string(),
                agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                    alias: Some("worker-b".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                public_label: Some("Worker B".to_string()),
                instructions: None,
                can_complete_workflow_run: Some(true),
                can_emit_intermediate_run_output: None,
                wait_for_all_inputs: None,
                intermediate_output_schema: None,
                max_turns: None,
                extensions: Vec::new(),
                canvas: None,
            },
        ],
        edges: vec![
            WorkflowCodeEdgeDefinition {
                handle: "router_to_a".to_string(),
                from_node: "router".to_string(),
                to_node: "worker_a".to_string(),
                source_side: None,
                target_side: None,
                handoff_schema: Some("handoff".to_string()),
                validation_policy: Some(WorkflowHandoffValidationPolicy::Halt),
                canvas: None,
            },
            WorkflowCodeEdgeDefinition {
                handle: "router_to_b".to_string(),
                from_node: "router".to_string(),
                to_node: "worker_b".to_string(),
                source_side: None,
                target_side: None,
                handoff_schema: Some("handoff".to_string()),
                validation_policy: Some(WorkflowHandoffValidationPolicy::Halt),
                canvas: None,
            },
        ],
        endpoints: vec![WorkflowCodeEndpointDefinition {
            handle: "entry".to_string(),
            entry_node: "router".to_string(),
            alias: Some("entry".to_string()),
            canvas: None,
        }],
        queues: Vec::new(),
        schedules: Vec::new(),
    };
    let agent_ids = BTreeMap::from([
        ("router".to_string(), "router-agent".to_string()),
        ("worker_a".to_string(), "worker-a-agent".to_string()),
        ("worker_b".to_string(), "worker-b-agent".to_string()),
    ]);
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply");
    let edge_a_id = report
        .edge_ids
        .get("router_to_a")
        .expect("router to worker a edge id");
    let progress_schema_id = report
        .schema_refs
        .get("progress")
        .expect("progress schema id");
    let handoff_schema_id = report
        .schema_refs
        .get("handoff")
        .expect("handoff schema id");
    let router_node_id = report
        .node_ids
        .get("router")
        .expect("router node id")
        .clone();
    let worker_a_node_id = report
        .node_ids
        .get("worker_a")
        .expect("worker a node id")
        .clone();
    let worker_b_node_id = report
        .node_ids
        .get("worker_b")
        .expect("worker b node id")
        .clone();
    let endpoint_id = report.endpoint_ids.get("entry").expect("entry endpoint id");
    let session_after_apply = service
        .get_session(session.id())
        .expect("session should resolve after workflow-code apply");
    let workflow_after_apply = session_after_apply
        .workflow(&report.workflow_id)
        .expect("workflow should exist after apply");
    let router_node = workflow_after_apply
        .node(&router_node_id)
        .expect("router node should exist");
    assert_eq!(
        router_node.intermediate_output_schema_ref(),
        Some(progress_schema_id.as_str())
    );
    assert!(
        workflow_after_apply
            .edges()
            .iter()
            .all(|edge| edge.handoff_schema_ref() == Some(handoff_schema_id.as_str()))
    );

    let workflow_run = service
        .invoke_workflow_endpoint(
            session.id(),
            &report.workflow_id,
            endpoint_id,
            Some("classify this task".to_string()),
        )
        .expect("workflow run should create");
    service
        .start_workflow_node_run(
            session.id(),
            workflow_run.id(),
            workflow_run.node_runs()[0].id(),
        )
        .expect("router should start");
    let node_run_id = workflow_run.node_runs()[0].id().to_string();
    service
        .prepare_workflow_turn(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            format!("workflow-ack:{node_run_id}"),
            "test workflow turn".to_string(),
            None,
            None,
        )
        .expect("router turn should prepare");
    service
        .submit_workflow_run_intermediate_output(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            WorkflowOutputPayload::new(r#"{"event":"started","status":"routing"}"#, Vec::new()),
            true,
            None,
        )
        .expect("first intermediate output should submit");
    let after_first_intermediate = service
        .record_workflow_intermediate_output_event(session.id(), workflow_run.id(), &node_run_id)
        .expect("first intermediate output should be recorded");
    assert_eq!(after_first_intermediate.intermediate_outputs().len(), 1);
    assert_eq!(after_first_intermediate.node_runs().len(), 1);
    assert_eq!(
        after_first_intermediate.messages().len(),
        workflow_run.messages().len()
    );
    service
        .submit_workflow_run_intermediate_output(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            WorkflowOutputPayload::new(r#"{"event":"checked","status":"routing"}"#, Vec::new()),
            true,
            None,
        )
        .expect("second intermediate output should submit in the same turn");
    let after_second_intermediate = service
        .record_workflow_intermediate_output_event(session.id(), workflow_run.id(), &node_run_id)
        .expect("second intermediate output should be recorded");
    assert_eq!(after_second_intermediate.intermediate_outputs().len(), 2);
    assert_eq!(after_second_intermediate.node_runs().len(), 1);
    assert_eq!(
        after_second_intermediate.messages().len(),
        workflow_run.messages().len()
    );
    let invalid_handoff = serde_json::json!({
        "workflow_handoffs": [{
            "edge_id": edge_a_id,
            "summary": "invalid progress-shaped handoff",
            "output": { "message": { "event": "not-a-task", "status": "wrong-channel" } }
        }]
    });
    let error = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            Some(completion_with_message(invalid_handoff.to_string())),
            None,
        )
        .expect_err("edge handoff schema should reject progress-shaped routed output");
    assert!(matches!(
        error,
        DaemonError::WorkflowHandoffValidationFailed { .. }
    ));
    let routed = serde_json::json!({
        "workflow_handoffs": [{
            "edge_id": edge_a_id,
            "summary": "send to worker a",
            "output": { "message": { "task": "only a" } }
        }]
    });

    let completion = service
        .complete_workflow_node_run(
            session.id(),
            workflow_run.id(),
            &node_run_id,
            Some(completion_with_message(routed.to_string())),
            None,
        )
        .expect("router completion should route only to the selected edge");

    assert_eq!(completion.dispatches.len(), 1);
    assert_eq!(
        completion.dispatches[0].node_run.node_id(),
        worker_a_node_id
    );
    assert_ne!(
        completion.dispatches[0].node_run.node_id(),
        worker_b_node_id
    );
    let payload: WorkflowHandoffPayload =
        serde_json::from_str(completion.dispatches[0].messages[0].handoff_payload())
            .expect("handoff payload should deserialize");
    let output = payload
        .completion()
        .and_then(|snapshot| snapshot.output())
        .expect("payload should include routed output");
    assert_eq!(output.message(), r#"{"task":"only a"}"#);
    assert!(!output.message().contains("started"));
    assert!(!output.message().contains("routing"));
    assert_eq!(completion.workflow_run.intermediate_outputs().len(), 2);

    let second_run = service
        .invoke_workflow_endpoint(
            session.id(),
            &report.workflow_id,
            endpoint_id,
            Some("classify another task".to_string()),
        )
        .expect("second workflow run should create");
    service
        .start_workflow_node_run(
            session.id(),
            second_run.id(),
            second_run.node_runs()[0].id(),
        )
        .expect("router should start for second run");
    let routed_by_target = serde_json::json!({
        "workflow_handoffs": [{
            "to_node_id": worker_b_node_id.clone(),
            "summary": "send to worker b",
            "message": { "task": "target-node selected task" }
        }]
    });

    let second_completion = service
        .complete_workflow_node_run(
            session.id(),
            second_run.id(),
            second_run.node_runs()[0].id(),
            Some(completion_with_message(routed_by_target.to_string())),
            None,
        )
        .expect("router completion should route by selected target node");

    assert_eq!(second_completion.dispatches.len(), 1);
    assert_eq!(
        second_completion.dispatches[0].node_run.node_id(),
        worker_b_node_id
    );
    let second_payload: WorkflowHandoffPayload =
        serde_json::from_str(second_completion.dispatches[0].messages[0].handoff_payload())
            .expect("second handoff payload should deserialize");
    let second_output = second_payload
        .completion()
        .and_then(|snapshot| snapshot.output())
        .expect("second payload should include routed output");
    assert_eq!(
        second_output.message(),
        r#"{"task":"target-node selected task"}"#
    );
}
