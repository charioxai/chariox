use super::*;
use crate::config::WorkflowCodeLimitsConfig;
use crate::session::{
    CreateSessionRequest, WorkflowHandoffValidationPolicy, WorkflowOutputPayload,
    WorkflowScheduleOverlapPolicy, WorkflowScheduleTrigger,
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

mod definition_application;
mod queues_and_watchdogs;
mod rebuild;
mod routed_handoffs;
mod validation_and_patterns;
