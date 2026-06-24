use super::*;
use crate::config::WorkflowCodeLimitsConfig;
use crate::session::{CreateSessionRequest, WorkflowHandoffValidationPolicy};
use crate::workflow_code::{
    WorkflowCodeAgentBinding, WorkflowCodeAgentCreate, WorkflowCodeCanvasEdge,
    WorkflowCodeCanvasPoint, WorkflowCodeDefinition, WorkflowCodeEdgeDefinition,
    WorkflowCodeEndpointDefinition, WorkflowCodeNodeDefinition, WorkflowCodeQueueDefinition,
    WorkflowCodeWatchdogDefinition, WorkflowCodeWorkflow, WORKFLOW_CODE_SCHEMA_VERSION,
};

fn workflow_code_definition() -> WorkflowCodeDefinition {
    WorkflowCodeDefinition {
        schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
        workflow: WorkflowCodeWorkflow {
            alias: Some("coded_flow".to_string()),
            flush_agent_context_before_run: Some(false),
            max_concurrent: Some(2),
            run_output_schema: None,
            intermediate_output_schema: None,
        },
        schemas: Vec::new(),
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
                intermediate_output_schema: None,
                max_turns: Some(4),
                extensions: Vec::new(),
                canvas: Some(WorkflowCodeCanvasPoint { x: 10, y: 20 }),
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
                canvas: Some(WorkflowCodeCanvasPoint { x: 220, y: 20 }),
            },
        ],
        edges: vec![WorkflowCodeEdgeDefinition {
            handle: "planner_to_worker".to_string(),
            from_node: "planner".to_string(),
            to_node: "worker".to_string(),
            source_side: None,
            target_side: None,
            handoff_schema: None,
            validation_policy: Some(WorkflowHandoffValidationPolicy::Warn),
            canvas: Some(WorkflowCodeCanvasEdge {
                points: vec![WorkflowCodeCanvasPoint { x: 120, y: 40 }],
            }),
        }],
        endpoints: vec![WorkflowCodeEndpointDefinition {
            handle: "entry".to_string(),
            entry_node: "planner".to_string(),
            alias: Some("entry".to_string()),
            canvas: Some(WorkflowCodeCanvasPoint { x: -80, y: 20 }),
        }],
        queues: vec![WorkflowCodeQueueDefinition {
            handle: "urgent".to_string(),
            alias: "urgent".to_string(),
            priority: 10,
            enabled: false,
        }],
        watchdogs: vec![WorkflowCodeWatchdogDefinition {
            handle: "entry_watchdog".to_string(),
            endpoint: "entry".to_string(),
            queue: None,
            interval_seconds: 60,
            invocation_prompt: "Check for stale work.".to_string(),
            policy: WorkflowWatchdogPolicy::Skip,
            max_wakeups: Some(2),
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
    let report = service
        .apply_workflow_code_definition(
            session.id(),
            &definition,
            &agent_ids,
            &WorkflowCodeLimitsConfig::default(),
            DEFAULT_LOCAL_USER_ID.to_string(),
            Some("meta-1".to_string()),
        )
        .expect("workflow-code should apply");

    assert_eq!(report.node_ids.len(), 2);
    assert_eq!(report.edge_ids.len(), 1);
    assert_eq!(report.endpoint_ids.len(), 1);
    assert_eq!(report.queue_ids.len(), 1);
    assert_eq!(report.watchdog_ids.len(), 1);
    assert!(report.canvas_layout_applied);

    let session = service
        .get_session(session.id())
        .expect("session should still exist");
    let workflow = session
        .workflow(&report.workflow_id)
        .expect("workflow should exist");
    assert_eq!(workflow.alias(), Some("coded_flow"));
    assert_eq!(workflow.controlled_by_metaagent_id(), Some("meta-1"));
    assert!(!workflow.flush_agent_context_before_run());
    assert_eq!(workflow.nodes().len(), 2);
    assert_eq!(workflow.edges().len(), 1);
    assert_eq!(workflow.endpoints().len(), 1);

    let planner_id = report.node_ids.get("planner").expect("planner id");
    let planner = workflow.node(planner_id).expect("planner node");
    assert_eq!(planner.agent_id(), "agent-1");
    assert_eq!(planner.public_label(), "Planner");
    assert_eq!(planner.instructions(), Some("Plan the task."));
    assert!(planner.can_emit_intermediate_run_output());
    assert_eq!(planner.max_turns(), Some(4));

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
                .watchdog_ids
                .get("entry_watchdog")
                .expect("watchdog id"),
        )
        .expect("watchdog should exist");
    assert_eq!(watchdog.workflow_id(), report.workflow_id);
    assert_eq!(watchdog.interval_seconds(), 60);
    assert_eq!(watchdog.max_wakeups(), Some(2));

    let layout = workflow
        .canvas_layout()
        .expect("canvas layout should exist");
    assert!(layout.nodes.contains_key(planner_id));
    assert!(layout
        .endpoints
        .contains_key(report.endpoint_ids.get("entry").expect("entry id")));
    assert!(layout
        .edges
        .contains_key(report.edge_ids.get("planner_to_worker").expect("edge id")));
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
