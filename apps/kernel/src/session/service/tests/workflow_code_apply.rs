use super::*;
use crate::config::WorkflowCodeLimitsConfig;
use crate::session::{
    CreateSessionRequest, WorkflowHandoffValidationPolicy, WorkflowScheduleOverlapPolicy,
    WorkflowScheduleTrigger,
};
use crate::workflow_code::{
    apply_workflow_code_provider_rebindings, compile_workflow_code_javascript,
    discover_workflow_code_node_path, WorkflowCodeAgentBinding, WorkflowCodeAgentCreate,
    WorkflowCodeCanvasEdge, WorkflowCodeCanvasPoint, WorkflowCodeDefinition,
    WorkflowCodeEdgeDefinition, WorkflowCodeEndpointDefinition, WorkflowCodeNodeDefinition,
    WorkflowCodeProviderRebinding, WorkflowCodeQueueDefinition, WorkflowCodeScheduleDefinition,
    WorkflowCodeSchemaDefinition, WorkflowCodeWorkflow, WORKFLOW_CODE_PATTERN_EXAMPLES,
    WORKFLOW_CODE_SCHEMA_VERSION,
};
use std::collections::BTreeSet;

fn completion_with_message(message: impl Into<String>) -> WorkflowCompletionSnapshot {
    WorkflowCompletionSnapshot::new(
        "done",
        Some(crate::session::WorkflowOutputPayload::new(
            message.into(),
            Vec::new(),
        )),
    )
}

fn workflow_code_definition() -> WorkflowCodeDefinition {
    WorkflowCodeDefinition {
        schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
        parameters_schema: None,
        workflow: WorkflowCodeWorkflow {
            alias: Some("coded_flow".to_string()),
            prompt: Some("Run the coded flow.".to_string()),
            flush_agent_context_before_run: Some(false),
            max_concurrent: Some(2),
            run_output_schema: Some("final".to_string()),
        },
        schemas: vec![
            WorkflowCodeSchemaDefinition {
                handle: "final".to_string(),
                alias: Some("Final".to_string()),
                description: Some("Final output".to_string()),
                schema: serde_json::json!({
                    "type": "object",
                    "required": ["answer"],
                    "properties": {
                        "answer": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
            WorkflowCodeSchemaDefinition {
                handle: "progress".to_string(),
                alias: Some("Progress".to_string()),
                description: None,
                schema: serde_json::json!({
                    "type": "object",
                    "required": ["status"],
                    "properties": {
                        "status": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
            WorkflowCodeSchemaDefinition {
                handle: "handoff".to_string(),
                alias: Some("Handoff".to_string()),
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
                handle: "planner".to_string(),
                agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                    alias: Some("planner".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                public_label: Some("Planner".to_string()),
                instructions: Some("Plan the task.".to_string()),
                can_complete_workflow_run: Some(false),
                can_emit_intermediate_run_output: Some(true),
                wait_for_all_inputs: None,
                intermediate_output_schema: Some("progress".to_string()),
                max_turns: Some(4),
                extensions: Vec::new(),
                canvas: Some(WorkflowCodeCanvasPoint { x: 0, y: 20 }),
            },
            WorkflowCodeNodeDefinition {
                handle: "worker".to_string(),
                agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                    alias: Some("worker".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                public_label: Some("Worker".to_string()),
                instructions: Some("Do the work.".to_string()),
                can_complete_workflow_run: Some(true),
                can_emit_intermediate_run_output: None,
                wait_for_all_inputs: Some(true),
                intermediate_output_schema: None,
                max_turns: None,
                extensions: Vec::new(),
                canvas: Some(WorkflowCodeCanvasPoint { x: 280, y: 20 }),
            },
        ],
        edges: vec![WorkflowCodeEdgeDefinition {
            handle: "planner_to_worker".to_string(),
            from_node: "planner".to_string(),
            to_node: "worker".to_string(),
            source_side: None,
            target_side: None,
            handoff_schema: Some("handoff".to_string()),
            validation_policy: Some(WorkflowHandoffValidationPolicy::Warn),
            canvas: Some(WorkflowCodeCanvasEdge {
                points: vec![WorkflowCodeCanvasPoint { x: 120, y: 40 }],
            }),
        }],
        endpoints: vec![WorkflowCodeEndpointDefinition {
            handle: "entry".to_string(),
            entry_node: "planner".to_string(),
            alias: Some("entry".to_string()),
            canvas: Some(WorkflowCodeCanvasPoint { x: -220, y: 20 }),
        }],
        queues: vec![WorkflowCodeQueueDefinition {
            handle: "urgent".to_string(),
            alias: "urgent".to_string(),
            priority: 10,
            enabled: false,
        }],
        schedules: vec![WorkflowCodeScheduleDefinition {
            handle: "entry_watchdog".to_string(),
            endpoint: "entry".to_string(),
            queue: Some("urgent".to_string()),
            enabled: Some(false),
            trigger: WorkflowScheduleTrigger::interval(60),
            invocation_prompt: "Check for stale work.".to_string(),
            overlap_policy: WorkflowScheduleOverlapPolicy::Skip,
            max_runs: Some(2),
        }],
    }
}

#[test]
fn applies_workflow_code_definition_to_session_primitives() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let definition = workflow_code_definition();
    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);
    let owner_user_id = "workflow-code-owner".to_string();
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            owner_user_id.clone(),
            Some("meta-1".to_string()),
        )
        .expect("workflow-code should apply");

    assert_eq!(report.node_ids.len(), 2);
    assert_eq!(report.schema_refs.len(), 3);
    assert_ne!(
        report.schema_refs.get("final").map(String::as_str),
        Some("final")
    );
    assert_eq!(report.edge_ids.len(), 1);
    assert_eq!(report.endpoint_ids.len(), 1);
    assert_eq!(report.queue_ids.len(), 1);
    assert_eq!(report.schedule_ids.len(), 1);
    assert_ne!(
        report.node_ids.get("planner").map(String::as_str),
        Some("planner"),
        "script node handles must not become kernel node ids"
    );
    assert_ne!(
        report.node_ids.get("worker").map(String::as_str),
        Some("worker"),
        "script node handles must not become kernel node ids"
    );
    assert_ne!(
        report.edge_ids.get("planner_to_worker").map(String::as_str),
        Some("planner_to_worker"),
        "script edge handles must not become kernel edge ids"
    );
    assert_ne!(
        report.endpoint_ids.get("entry").map(String::as_str),
        Some("entry"),
        "script endpoint handles must not become kernel endpoint ids"
    );
    assert_ne!(
        report.queue_ids.get("urgent").map(String::as_str),
        Some("urgent"),
        "script queue handles must not become kernel queue ids"
    );
    assert_ne!(
        report
            .schedule_ids
            .get("entry_watchdog")
            .map(String::as_str),
        Some("entry_watchdog"),
        "script watchdog handles must not become kernel watchdog ids"
    );
    assert!(report.canvas_layout_applied);
    assert!(report.warnings.is_empty());

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    assert_eq!(workflow.alias(), Some("coded_flow"));
    assert_eq!(workflow.controlled_by_metaagent_id(), Some("meta-1"));
    assert!(!workflow.flush_agent_context_before_run());
    assert_eq!(workflow.max_concurrent(), 2);
    assert_eq!(workflow.nodes().len(), 2);
    assert_eq!(workflow.edges().len(), 1);
    assert_eq!(workflow.endpoints().len(), 1);
    assert_eq!(workflow.schemas().len(), 3);
    let final_schema_id = report.schema_refs.get("final").expect("final schema id");
    let progress_schema_id = report
        .schema_refs
        .get("progress")
        .expect("progress schema id");
    let handoff_schema_id = report
        .schema_refs
        .get("handoff")
        .expect("handoff schema id");
    assert_eq!(
        workflow.run_output_schema_ref(),
        Some(final_schema_id.as_str())
    );
    assert_eq!(
        workflow
            .schema(final_schema_id)
            .and_then(|schema| schema.alias()),
        Some("Final")
    );

    let planner_id = report.node_ids.get("planner").expect("planner id");
    let planner = workflow.node(planner_id).expect("planner node");
    assert_eq!(planner.agent_id(), "agent-1");
    assert_eq!(planner.public_label(), "Planner");
    assert_eq!(planner.instructions(), Some("Plan the task."));
    assert!(planner.can_emit_intermediate_run_output());
    assert_eq!(
        planner.intermediate_output_schema_ref(),
        Some(progress_schema_id.as_str())
    );
    assert_eq!(planner.max_turns(), Some(4));

    let edge_id = report.edge_ids.get("planner_to_worker").expect("edge id");
    let edge = workflow.edge(edge_id).expect("workflow edge");
    assert_eq!(edge.handoff_schema_ref(), Some(handoff_schema_id.as_str()));

    let endpoint_id = report.endpoint_ids.get("entry").expect("entry id");
    let endpoint = workflow.endpoint(endpoint_id).expect("workflow endpoint");
    assert_eq!(endpoint.owner_user_id(), owner_user_id);

    let urgent_queue_id = report.queue_ids.get("urgent").expect("urgent queue id");
    let urgent_queue = session
        .workflow_prompt_queue(&report.workflow_id, urgent_queue_id)
        .expect("urgent queue should exist");
    assert_eq!(urgent_queue.alias(), "urgent");
    assert_eq!(urgent_queue.priority(), 10);
    assert!(!urgent_queue.enabled());

    let watchdog = session
        .workflow_watchdog(
            report
                .schedule_ids
                .get("entry_watchdog")
                .expect("watchdog id"),
        )
        .expect("watchdog should exist");
    assert_eq!(watchdog.workflow_id(), report.workflow_id);
    assert!(!watchdog.enabled());
    assert_eq!(watchdog.interval_seconds(), 60);
    assert_eq!(watchdog.max_wakeups(), Some(2));
    assert_eq!(watchdog.queue_id(), Some(urgent_queue_id.as_str()));

    let layout = workflow
        .canvas_layout()
        .expect("canvas layout should exist");
    assert!(layout.nodes.contains_key(planner_id));
    assert!(layout.endpoints.contains_key(endpoint_id));
    assert!(layout
        .edges
        .contains_key(report.edge_ids.get("planner_to_worker").expect("edge id")));
}

#[test]
fn workflow_code_definition_alias_base_allocates_template_aliases() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let definition = workflow_code_definition();
    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);

    let first = service
        .apply_workflow_code_definition_with_alias_base(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            "workflow-code-owner".to_string(),
            None,
            Some("Prompt Chaining"),
        )
        .expect("first template workflow should apply");
    let second = service
        .apply_workflow_code_definition_with_alias_base(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            "workflow-code-owner".to_string(),
            None,
            Some("Prompt Chaining"),
        )
        .expect("second template workflow should apply");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    assert_eq!(
        session
            .workflow(&first.workflow_id)
            .and_then(|workflow| workflow.alias()),
        Some("prompt-chaining-1")
    );
    assert_eq!(
        session
            .workflow(&second.workflow_id)
            .and_then(|workflow| workflow.alias()),
        Some("prompt-chaining-2")
    );
}

#[test]
fn workflow_code_apply_uses_configured_default_concurrency_when_script_omits_it() {
    let mut config = test_config();
    config.user_config.workflow.code = Some(crate::config::UserWorkflowCodeConfig {
        max_concurrent: Some(7),
        ..Default::default()
    });
    let mut service = SessionService::new(&config);
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.workflow.max_concurrent = None;
    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
    ]);

    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &config.workflow_code_limits(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect("workflow-code should apply with configured default concurrency");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    assert_eq!(workflow.max_concurrent(), 7);
}

#[test]
fn workflow_code_apply_preserves_node_intermediate_schema_override() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.schemas.push(WorkflowCodeSchemaDefinition {
        handle: "node_progress".to_string(),
        alias: Some("node-progress".to_string()),
        description: Some("Node-specific progress output".to_string()),
        schema: serde_json::json!({
            "type": "object",
            "required": ["node_value"],
            "properties": {
                "node_value": { "type": "number" }
            },
            "additionalProperties": false
        }),
    });
    definition.nodes[0].intermediate_output_schema = Some("node_progress".to_string());

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
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
        .expect("workflow-code should apply with node-specific intermediate schema");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    let planner_id = report.node_ids.get("planner").expect("planner id");
    let planner = workflow
        .node(planner_id)
        .expect("planner node should exist");

    assert_eq!(
        planner.intermediate_output_schema_ref(),
        report.schema_refs.get("node_progress").map(String::as_str)
    );
}

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
        schemas: Vec::new(),
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
                can_emit_intermediate_run_output: None,
                wait_for_all_inputs: None,
                intermediate_output_schema: None,
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
                handoff_schema: None,
                validation_policy: None,
                canvas: None,
            },
            WorkflowCodeEdgeDefinition {
                handle: "router_to_b".to_string(),
                from_node: "router".to_string(),
                to_node: "worker_b".to_string(),
                source_side: None,
                target_side: None,
                handoff_schema: None,
                validation_policy: None,
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
            workflow_run.node_runs()[0].id(),
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
            "message": "target-node selected task"
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
    assert_eq!(second_output.message(), "target-node selected task");
}

#[test]
fn workflow_code_apply_auto_layouts_missing_canvas_coordinates() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    for node in &mut definition.nodes {
        node.canvas = None;
    }
    for endpoint in &mut definition.endpoints {
        endpoint.canvas = None;
    }
    for edge in &mut definition.edges {
        edge.canvas = None;
    }

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
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

    assert!(report.canvas_layout_applied);
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.code == "canvas_auto_layout_applied"));
    assert!(!report
        .warnings
        .iter()
        .any(|warning| warning.code == "default_queue_created"));

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    let layout = workflow
        .canvas_layout()
        .expect("auto-layout should create canvas layout");
    assert_eq!(layout.nodes.len(), 2);
    assert_eq!(layout.endpoints.len(), 1);
    assert!(layout.edges.is_empty());

    let planner_id = report.node_ids.get("planner").expect("planner id");
    let worker_id = report.node_ids.get("worker").expect("worker id");
    let entry_id = report.endpoint_ids.get("entry").expect("entry id");
    let planner_point = layout.nodes.get(planner_id).expect("planner point");
    let worker_point = layout.nodes.get(worker_id).expect("worker point");
    let entry_point = layout.endpoints.get(entry_id).expect("entry point");
    assert!(worker_point.x > planner_point.x);
    assert!(entry_point.x < planner_point.x);
    assert_eq!(entry_point.y, planner_point.y);
}

#[test]
fn workflow_code_apply_maps_omitted_queues_to_kernel_default_queue() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.queues.clear();
    definition.schedules.clear();

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
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
        .expect("workflow-code should apply with the default prompt queue");

    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.code == "default_queue_created"
            && warning.handle.as_deref() == Some("default")));

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let default_queue = session
        .workflow_prompt_queue(&report.workflow_id, "default")
        .expect("kernel default workflow queue should exist");
    assert_eq!(
        report.queue_ids.get("default").map(String::as_str),
        Some(default_queue.id())
    );
    assert_eq!(default_queue.alias(), "default");
    assert_eq!(default_queue.priority(), 0);
    assert!(default_queue.enabled());
}

#[test]
fn workflow_code_apply_normalizes_explicit_default_queue_alias() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.queues.clear();
    definition.queues.push(WorkflowCodeQueueDefinition {
        handle: "script_default".to_string(),
        alias: " Default ".to_string(),
        priority: 7,
        enabled: false,
    });
    definition.schedules.clear();

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
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
        .expect("workflow-code should apply with normalized default queue alias");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let default_queue = session
        .workflow_prompt_queue(&report.workflow_id, "default")
        .expect("kernel default workflow queue should exist");

    assert_eq!(
        report.queue_ids.get("script_default").map(String::as_str),
        Some(default_queue.id())
    );
    assert_eq!(default_queue.alias(), "default");
    assert_eq!(default_queue.priority(), 7);
    assert!(!default_queue.enabled());
    assert_eq!(
        session
            .workflow_prompt_queues_for_workflow(&report.workflow_id)
            .len(),
        1
    );
}

#[test]
fn workflow_code_apply_maps_watchdogs_to_implicit_default_queue_when_other_queues_exist() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1", "agent-2"]);

    let mut definition = workflow_code_definition();
    definition.schedules[0].queue = Some("default".to_string());

    let agent_ids = BTreeMap::from([
        ("planner".to_string(), "agent-1".to_string()),
        ("worker".to_string(), "agent-2".to_string()),
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
        .expect("workflow-code should apply with explicit and implicit queues");

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let default_queue = session
        .workflow_prompt_queue(&report.workflow_id, "default")
        .expect("kernel default workflow queue should exist");
    let urgent_queue = session
        .workflow_prompt_queue(
            &report.workflow_id,
            report.queue_ids.get("urgent").expect("urgent queue id"),
        )
        .expect("scripted urgent queue should exist");
    let watchdog = session
        .workflow_watchdog(
            report
                .schedule_ids
                .get("entry_watchdog")
                .expect("watchdog id"),
        )
        .expect("watchdog should exist");

    assert_eq!(
        report.queue_ids.get("default").map(String::as_str),
        Some(default_queue.id())
    );
    assert_ne!(default_queue.id(), urgent_queue.id());
    assert_eq!(watchdog.queue_id(), Some(default_queue.id()));
    assert!(!report
        .warnings
        .iter()
        .any(|warning| warning.code == "default_queue_created"));
}

#[test]
fn workflow_code_apply_rejects_missing_agent_resolution() {
    let mut service = SessionService::new(&test_config());
    let session = service
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .expect("session should create");
    seed_agents(&mut service, session.id(), &["agent-1"]);

    let definition = workflow_code_definition();
    let agent_ids = BTreeMap::from([("planner".to_string(), "agent-1".to_string())]);
    let error = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            None,
        )
        .expect_err("unresolved worker should fail");

    assert!(format!("{error}").contains("worker"));
}

#[test]
fn workflow_code_canonical_patterns_compile_and_apply_with_provider_rebindings() {
    let node_path = match discover_workflow_code_node_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("skipping workflow-code canonical pattern apply test: {error}");
            return;
        }
    };

    for example in WORKFLOW_CODE_PATTERN_EXAMPLES {
        let mut compiled = compile_workflow_code_javascript(
            &node_path,
            example.source,
            &WorkflowCodeLimitsConfig::default(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "workflow-code pattern example `{}` at `{}` should compile: {error}",
                example.slug, example.path
            )
        })
        .definition;

        let rebindings = compiled
            .nodes
            .iter()
            .filter_map(|node| match node.agent {
                WorkflowCodeAgentBinding::Create(_) => Some(WorkflowCodeProviderRebinding {
                    node: node.handle.clone(),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                WorkflowCodeAgentBinding::Existing(_) => None,
            })
            .collect::<Vec<_>>();
        apply_workflow_code_provider_rebindings(&mut compiled, &rebindings).unwrap_or_else(
            |error| {
                panic!(
                    "`{}` provider rebindings should apply: {error}",
                    example.slug
                )
            },
        );
        assert!(
            compiled
                .validate_with_limits(&WorkflowCodeLimitsConfig::default())
                .ok,
            "`{}` should remain valid after provider rebinding",
            example.slug
        );

        let mut service = SessionService::new(&test_config());
        let session = service
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should create");
        let agent_ids = compiled
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node.handle.clone(),
                    format!("agent-{}-{index}", example.slug),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let seeded_agent_ids = agent_ids.values().map(String::as_str).collect::<Vec<_>>();
        seed_agents(&mut service, session.id(), &seeded_agent_ids);

        let report = service
            .apply_workflow_code_definition(
                session.id(),
                &compiled,
                &agent_ids,
                &WorkflowCodeLimitsConfig::default(),
                DEFAULT_LOCAL_USER_ID.to_string(),
                Some("meta-1".to_string()),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "`{}` should apply to session primitives: {error}",
                    example.slug
                )
            });

        assert_eq!(
            report.node_ids.len(),
            compiled.nodes.len(),
            "`{}` should allocate one kernel node id per script node",
            example.slug
        );
        assert_eq!(
            report.edge_ids.len(),
            compiled.edges.len(),
            "`{}` should allocate one kernel edge id per script edge",
            example.slug
        );
        assert_eq!(
            report.endpoint_ids.len(),
            compiled.endpoints.len(),
            "`{}` should allocate one kernel endpoint id per script endpoint",
            example.slug
        );
        assert_eq!(
            report.schema_refs.len(),
            compiled.schemas.len(),
            "`{}` should allocate one kernel schema ref per script schema",
            example.slug
        );
        for node in &compiled.nodes {
            assert_ne!(
                report.node_ids.get(&node.handle).map(String::as_str),
                Some(node.handle.as_str()),
                "`{}` should not reuse script node handle as kernel node id",
                example.slug
            );
        }
        let applied_session = service
            .get_session(session.id())
            .expect("session should remain readable");
        assert!(
            applied_session
                .workflows()
                .iter()
                .any(|workflow| workflow.id() == report.workflow_id),
            "`{}` workflow should appear in the session projection",
            example.slug
        );
    }
}

#[test]
fn planner_worker_reviewer_pattern_preserves_goal_workflow_contract() {
    let node_path = match discover_workflow_code_node_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("skipping planner-worker-reviewer pattern test: {error}");
            return;
        }
    };
    let example = WORKFLOW_CODE_PATTERN_EXAMPLES
        .iter()
        .find(|example| example.slug == "planner-worker-reviewer")
        .expect("planner-worker-reviewer pattern should be bundled");
    let compiled = compile_workflow_code_javascript(
        &node_path,
        example.source,
        &WorkflowCodeLimitsConfig::default(),
    )
    .expect("planner-worker-reviewer should compile")
    .definition;

    assert_eq!(
        compiled.workflow.alias.as_deref(),
        Some("pattern-planner-worker-reviewer")
    );
    assert_eq!(compiled.nodes.len(), 3);
    assert_eq!(compiled.edges.len(), 4);
    assert_eq!(compiled.endpoints.len(), 1);
    assert_eq!(compiled.schemas.len(), 5);
    assert_eq!(
        compiled.workflow.run_output_schema.as_deref(),
        Some("final_output")
    );

    let planner = compiled
        .nodes
        .iter()
        .find(|node| node.handle == "planner")
        .expect("planner node should exist");
    assert_eq!(planner.can_complete_workflow_run, Some(true));
    assert!(planner
        .instructions
        .as_deref()
        .unwrap_or_default()
        .contains("only node allowed to finish"));
    for node in compiled
        .nodes
        .iter()
        .filter(|node| node.handle == "worker" || node.handle == "reviewer")
    {
        assert_ne!(
            node.can_complete_workflow_run,
            Some(true),
            "{} must not complete the workflow",
            node.handle
        );
    }

    let edge_pairs = compiled
        .edges
        .iter()
        .map(|edge| {
            (
                edge.handle.as_str(),
                edge.from_node.as_str(),
                edge.to_node.as_str(),
                edge.handoff_schema.as_deref(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert!(edge_pairs.contains(&(
        "planner_to_worker",
        "planner",
        "worker",
        Some("implementation_assignment"),
    )));
    assert!(edge_pairs.contains(&(
        "worker_to_reviewer",
        "worker",
        "reviewer",
        Some("implementation_result"),
    )));
    assert!(edge_pairs.contains(&(
        "reviewer_to_worker",
        "reviewer",
        "worker",
        Some("revision_request"),
    )));
    assert!(edge_pairs.contains(&(
        "reviewer_to_planner",
        "reviewer",
        "planner",
        Some("accepted_step_report"),
    )));
    assert_eq!(compiled.endpoints[0].handle, "entry");
    assert_eq!(compiled.endpoints[0].entry_node, "planner");
    assert!(compiled
        .parameters_schema
        .as_ref()
        .and_then(|schema| schema.pointer("/properties/max_review_cycles_per_step/default"))
        .is_some_and(|value| value == 6));
}
