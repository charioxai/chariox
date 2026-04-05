use jsonschema::JSONSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::DaemonApp;
use crate::error::DaemonError;

pub const ACK_WORKFLOW_TURN_TOOL: &str = "ack_workflow_turn";
pub const VALIDATE_WORKFLOW_OUTPUT_TOOL: &str = "validate_workflow_output";

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRuntimeToolContext {
    pub session_id: String,
    pub workflow_run_ref: String,
    pub workflow_node_run_id: String,
    pub delivery_token: Option<String>,
    pub allowed_output_schema_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeToolCall {
    pub tool_name: String,
    pub arguments: Value,
    pub context: WorkflowRuntimeToolContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeToolResult {
    pub ok: bool,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckWorkflowTurnArgs {
    pub delivery_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateWorkflowOutputArgs {
    pub output_schema_ref: String,
    pub output_json: String,
}

#[allow(dead_code)]
pub fn workflow_runtime_tool_specs() -> Vec<RuntimeToolSpec> {
    vec![
        RuntimeToolSpec {
            name: ACK_WORKFLOW_TURN_TOOL.to_string(),
            description: "Acknowledge that the current workflow turn was received.".to_string(),
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
            name: VALIDATE_WORKFLOW_OUTPUT_TOOL.to_string(),
            description: "Validate workflow output JSON against an allowed schema ref for the current workflow turn.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["output_schema_ref", "output_json"],
                "properties": {
                    "output_schema_ref": {"type": "string"},
                    "output_json": {"type": "string"}
                },
                "additionalProperties": false
            }),
        },
    ]
}

pub fn dispatch_runtime_tool_call(
    app: &mut DaemonApp,
    call: RuntimeToolCall,
) -> Result<RuntimeToolResult, DaemonError> {
    match call.tool_name.as_str() {
        ACK_WORKFLOW_TURN_TOOL => {
            let args = serde_json::from_value::<AckWorkflowTurnArgs>(call.arguments).map_err(
                |error| DaemonError::LocalTransport {
                    operation: "runtime_tool_ack_workflow_turn",
                    message: format!("invalid tool arguments: {error}"),
                },
            )?;
            let workflow_run_id = app
                .sessions()
                .resolve_workflow_run_ref(&call.context.session_id, &call.context.workflow_run_ref)?
                .id()
                .to_string();
            let workflow_run = app.sessions_mut().ack_workflow_turn(
                &call.context.session_id,
                &workflow_run_id,
                &call.context.workflow_node_run_id,
                &args.delivery_token,
            )?;
            Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "workflow_run_id": workflow_run.id(),
                    "workflow_node_run_id": call.context.workflow_node_run_id,
                    "state": "acknowledged",
                }),
            })
        }
        VALIDATE_WORKFLOW_OUTPUT_TOOL => {
            let args =
                serde_json::from_value::<ValidateWorkflowOutputArgs>(call.arguments).map_err(
                    |error| DaemonError::LocalTransport {
                        operation: "runtime_tool_validate_workflow_output",
                        message: format!("invalid tool arguments: {error}"),
                    },
                )?;
            if !call.context.allowed_output_schema_refs.is_empty()
                && !call
                    .context
                    .allowed_output_schema_refs
                    .iter()
                    .any(|schema_ref| schema_ref == &args.output_schema_ref)
            {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_validate_workflow_output",
                    message: format!(
                        "schema ref `{}` is not allowed for workflow node run `{}`",
                        args.output_schema_ref, call.context.workflow_node_run_id
                    ),
                });
            }
            match validate_workflow_output_schema(&args.output_schema_ref, &args.output_json) {
                Ok(()) => Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "valid": true,
                        "warning": Value::Null,
                    }),
                }),
                Err(message) => Ok(RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "valid": false,
                        "warning": message,
                    }),
                }),
            }
        }
        other => Err(DaemonError::LocalTransport {
            operation: "dispatch_runtime_tool_call",
            message: format!("unsupported runtime tool `{other}`"),
        }),
    }
}

pub fn validate_workflow_output_schema(schema_ref: &str, output_json: &str) -> Result<(), String> {
    let schema_source = std::fs::read_to_string(schema_ref)
        .map_err(|error| format!("schema ref `{schema_ref}` could not be read: {error}"))?;
    let schema_value = serde_json::from_str::<Value>(&schema_source)
        .map_err(|error| format!("schema ref `{schema_ref}` is not valid JSON: {error}"))?;
    let output_value = serde_json::from_str::<Value>(output_json)
        .map_err(|error| format!("output is not valid JSON: {error}"))?;
    let compiled = JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema_value)
        .map_err(|error| format!("schema ref `{schema_ref}` failed to compile: {error}"))?;
    if let Err(errors) = compiled.validate(&output_value) {
        let message = errors
            .into_iter()
            .next()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "schema validation failed".to_string());
        return Err(message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::local::{
        AddWorkflowNodeRequest, CreateWorkflowEndpointRequest, CreateWorkflowRequest,
        InvokeWorkflowEndpointRequest, LocalDaemonRequest, SpawnAgentRequest,
        UpdateWorkflowNodeInstructionsRequest,
    };
    use crate::session::{CreateSessionRequest, WorkflowTurnRuntimeState};
    use crate::{DaemonApp, DaemonConfig};

    use super::{
        dispatch_runtime_tool_call, workflow_runtime_tool_specs, RuntimeToolCall,
        WorkflowRuntimeToolContext, ACK_WORKFLOW_TURN_TOOL, VALIDATE_WORKFLOW_OUTPUT_TOOL,
    };

    #[test]
    fn workflow_runtime_tool_specs_expose_ack_and_validation() {
        let specs = workflow_runtime_tool_specs();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, ACK_WORKFLOW_TURN_TOOL);
        assert_eq!(specs[1].name, VALIDATE_WORKFLOW_OUTPUT_TOOL);
    }

    #[test]
    fn validation_tool_enforces_allowed_schema_refs() {
        let temp_dir = std::env::temp_dir().join(format!(
            "arroba-runtime-tools-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&temp_dir);
        let schema_path = temp_dir.join("schema.json");
        fs::write(
            &schema_path,
            r#"{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}"#,
        )
        .expect("schema fixture should be written");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");

        let error = dispatch_runtime_tool_call(
            &mut app,
            RuntimeToolCall {
                tool_name: VALIDATE_WORKFLOW_OUTPUT_TOOL.to_string(),
                arguments: serde_json::json!({
                    "output_schema_ref": schema_path.to_string_lossy().to_string(),
                    "output_json": "{\"message\":\"ok\"}"
                }),
                context: WorkflowRuntimeToolContext {
                    session_id: "session-x".to_string(),
                    workflow_run_ref: "workflow-run-x".to_string(),
                    workflow_node_run_id: "workflow-node-run-x".to_string(),
                    delivery_token: None,
                    allowed_output_schema_refs: vec!["/not/allowed.json".to_string()],
                },
            },
        )
        .expect_err("disallowed schema ref should fail");

        assert!(error
            .to_string()
            .contains("is not allowed for workflow node run"));
    }

    #[test]
    fn ack_runtime_tool_transitions_envelope_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-1", "worktree-1"),
            ))
            .expect("session should exist")
        {
            crate::local::LocalDaemonResponse::SessionCreated { session, agent } => {
                (session, agent)
            }
            other => panic!("unexpected response: {other:?}"),
        };
        app.attach(AttachRequest::new(
            session.id(),
            "client-a",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
        let agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-a".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-1".to_string()),
            }))
            .expect("agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let workflow_id = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("wf".to_string()),
            }))
            .expect("workflow should exist")
        {
            crate::local::LocalDaemonResponse::WorkflowCreated { workflow, .. } => workflow.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let node_id = match app.handle_local_request(LocalDaemonRequest::AddWorkflowNode(AddWorkflowNodeRequest {
            session_id: session.id().to_string(),
            workflow_ref: workflow_id.clone(),
            agent_id: agent_id.clone(),
        }))
        .expect("node should be added") {
            crate::local::LocalDaemonResponse::WorkflowNodeAdded { node, .. } => node.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        app.handle_local_request(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                node_id: node_id.clone(),
                instructions: Some("Follow instructions".to_string()),
            },
        ))
        .expect("instructions should update");
        match app.handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                entry_node_id: node_id.clone(),
                alias: Some("entry".to_string()),
            },
        ))
        .expect("endpoint should be added") {
            crate::local::LocalDaemonResponse::WorkflowEndpointCreated { .. } => {}
            other => panic!("unexpected response: {other:?}"),
        };
        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    endpoint_ref: "entry".to_string(),
                    prompt: Some("start".to_string()),
                },
            ))
            .expect("workflow should invoke")
        {
            crate::local::LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => workflow_run,
            other => panic!("unexpected response: {other:?}"),
        };
        let node_run = workflow_run
            .node_runs()
            .first()
            .expect("node run should exist");
        let envelope = node_run
            .turn_envelope()
            .expect("turn envelope should be prepared");
        assert_eq!(envelope.state(), WorkflowTurnRuntimeState::Dispatched);

        let result = dispatch_runtime_tool_call(
            &mut app,
            RuntimeToolCall {
                tool_name: ACK_WORKFLOW_TURN_TOOL.to_string(),
                arguments: serde_json::json!({
                    "delivery_token": envelope.delivery_token(),
                }),
                context: WorkflowRuntimeToolContext {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: node_run.id().to_string(),
                    delivery_token: Some(envelope.delivery_token().to_string()),
                    allowed_output_schema_refs: Vec::new(),
                },
            },
        )
        .expect("ack runtime tool should succeed");

        assert_eq!(result.payload["state"], "acknowledged");
        let updated_run = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should still exist");
        let updated_node_run = updated_run
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == node_run.id())
            .expect("updated node run should exist");
        assert_eq!(
            updated_node_run
                .turn_envelope()
                .expect("updated envelope should exist")
                .state(),
            WorkflowTurnRuntimeState::Acknowledged
        );
    }

    #[test]
    fn validation_runtime_tool_accepts_allowed_schema_refs() {
        let temp_dir = std::env::temp_dir().join(format!(
            "arroba-runtime-tools-allowed-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&temp_dir);
        let schema_path = temp_dir.join("schema-allowed.json");
        fs::write(
            &schema_path,
            r#"{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}"#,
        )
        .expect("schema fixture should be written");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let result = dispatch_runtime_tool_call(
            &mut app,
            RuntimeToolCall {
                tool_name: VALIDATE_WORKFLOW_OUTPUT_TOOL.to_string(),
                arguments: serde_json::json!({
                    "output_schema_ref": schema_path.to_string_lossy().to_string(),
                    "output_json": "{\"message\":\"ok\"}"
                }),
                context: WorkflowRuntimeToolContext {
                    session_id: "session-x".to_string(),
                    workflow_run_ref: "workflow-run-x".to_string(),
                    workflow_node_run_id: "workflow-node-run-x".to_string(),
                    delivery_token: None,
                    allowed_output_schema_refs: vec![schema_path.to_string_lossy().to_string()],
                },
            },
        )
        .expect("allowed schema ref should validate");
        assert_eq!(result.payload["valid"], true);
    }
}
