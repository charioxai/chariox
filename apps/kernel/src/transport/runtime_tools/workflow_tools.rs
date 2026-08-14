use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckWorkflowTurnArgs {
    pub delivery_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateWorkflowHandoffArgs {
    pub handoff_schema_ref: String,
    pub handoff_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReadWorkflowTurnContextArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConsoleWriteArgs {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateAndSubmitWorkflowRunOutputArgs {
    pub workflow_output_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAppActionArgs {
    pub action_id: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyToEventArgs {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

pub fn workflow_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: ACK_WORKFLOW_TURN_TOOL.to_string(),
            description: "Acknowledge that the current workflow turn was received. This does not complete the turn; after this tool returns, continue the same response and emit the required final fenced JSON workflow output.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["delivery_token"],
                "properties": {
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: VALIDATE_WORKFLOW_HANDOFF_TOOL.to_string(),
            description: "Validate workflow handoff JSON against an allowed handoff schema ref for the current workflow turn.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["handoff_schema_ref", "handoff_json"],
                "properties": {
                    "handoff_schema_ref": {"type": "string"},
                    "handoff_json": {"type": "string"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.to_string(),
            description: "Validate and submit the final output for the current workflow run.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["workflow_output_json"],
                "properties": {
                    "workflow_output_json": {"type": "string"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL.to_string(),
            description: "Validate and submit one user-visible intermediate workflow output event for the current workflow run. This tool may be called multiple times in one workflow node turn and does not send data to downstream nodes.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["workflow_output_json"],
                "properties": {
                    "workflow_output_json": {"type": "string"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: READ_WORKFLOW_TURN_CONTEXT_TOOL.to_string(),
            description: "Read the current workflow turn context, including invocation prompt and upstream handoff messages for this node run.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WORKFLOW_CONSOLE_READ_TOOL.to_string(),
            description: "Read the shared workflow console for the current workflow.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WORKFLOW_CONSOLE_WRITE_TOOL.to_string(),
            description: "Append human-facing text to the shared workflow console for the current workflow.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: WORKFLOW_CONSOLE_CLEAR_TOOL.to_string(),
            description: "Clear the shared workflow console for the current workflow.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: AGENT_APP_ACTION_TOOL_QUALIFIED.to_string(),
            description: "Call a route-scoped Agent App action exposed by the current published workflow invocation. The action must be allowed by the matched endpoint route.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["action_id", "input"],
                "properties": {
                    "action_id": {"type": "string"},
                    "input": {"type": "object"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
        RuntimeToolSpec {
            name: REPLY_TO_EVENT_TOOL_QUALIFIED.to_string(),
            description: "Reply through the notification provider that delivered the current event. Omitting mode uses the event binding's configured reply mode; explicitly choose `thread` or `channel` when the binding permits it. This is only available for event-triggered workflow runs with reply permission enabled.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": {"type": "string", "minLength": 1, "maxLength": 40000},
                    "mode": {"type": "string", "enum": ["thread", "channel"]},
                    "idempotency_key": {"type": "string"},
                    "delivery_token": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
    ]
}

pub fn canonical_workflow_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        ACK_WORKFLOW_TURN_TOOL => Some(ACK_WORKFLOW_TURN_TOOL),
        VALIDATE_WORKFLOW_HANDOFF_TOOL => Some(VALIDATE_WORKFLOW_HANDOFF_TOOL),
        VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL => {
            Some(VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL)
        }
        VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL => {
            Some(VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL)
        }
        READ_WORKFLOW_TURN_CONTEXT_TOOL => Some(READ_WORKFLOW_TURN_CONTEXT_TOOL),
        WORKFLOW_CONSOLE_READ_TOOL => Some(WORKFLOW_CONSOLE_READ_TOOL),
        WORKFLOW_CONSOLE_WRITE_TOOL => Some(WORKFLOW_CONSOLE_WRITE_TOOL),
        WORKFLOW_CONSOLE_CLEAR_TOOL => Some(WORKFLOW_CONSOLE_CLEAR_TOOL),
        AGENT_APP_ACTION_TOOL
        | AGENT_APP_ACTION_TOOL_QUALIFIED
        | "chariox_agent_app_action"
        | "mcp__chariox__agent_app_action"
        | "mcp__chariox__chariox_agent_app_action" => Some(AGENT_APP_ACTION_TOOL),
        REPLY_TO_EVENT_TOOL
        | REPLY_TO_EVENT_TOOL_QUALIFIED
        | "chariox_reply_to_event"
        | "mcp__chariox__reply_to_event"
        | "mcp__chariox__chariox_reply_to_event" => Some(REPLY_TO_EVENT_TOOL),
        _ => None,
    }
}
