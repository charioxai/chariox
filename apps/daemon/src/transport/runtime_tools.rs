use jsonschema::JSONSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{WorkflowNodeRunStatus, WorkflowTurnRuntimeState};

pub const ACK_WORKFLOW_TURN_TOOL: &str = "ack_workflow_turn";
pub const VALIDATE_WORKFLOW_OUTPUT_TOOL: &str = "validate_workflow_output";
pub const WORKFLOW_CONSOLE_READ_TOOL: &str = "workflow_console_read";
pub const WORKFLOW_CONSOLE_WRITE_TOOL: &str = "workflow_console_write";
pub const WORKFLOW_CONSOLE_CLEAR_TOOL: &str = "workflow_console_clear";

fn canonical_runtime_tool_name(tool_name: &str) -> &str {
    tool_name.strip_prefix("arroba_").unwrap_or(tool_name)
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConsoleWriteArgs {
    pub text: String,
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
                    "output_json": {"type": "string"},
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
    ]
}

pub fn dispatch_runtime_tool_call(
    app: &mut DaemonApp,
    call: RuntimeToolCall,
) -> Result<RuntimeToolResult, DaemonError> {
    match canonical_runtime_tool_name(call.tool_name.as_str()) {
        ACK_WORKFLOW_TURN_TOOL => {
            let args =
                serde_json::from_value::<AckWorkflowTurnArgs>(call.arguments).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "runtime_tool_ack_workflow_turn",
                        message: format!("invalid tool arguments: {error}"),
                    }
                })?;
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
            let args = serde_json::from_value::<ValidateWorkflowOutputArgs>(call.arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_validate_workflow_output",
                    message: format!("invalid tool arguments: {error}"),
                })?;
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
        WORKFLOW_CONSOLE_READ_TOOL => {
            let workflow_run = app
                .sessions()
                .resolve_workflow_run_ref(&call.context.session_id, &call.context.workflow_run_ref)?;
            let console = crate::scheduler::runtime::read_workflow_console(
                app,
                &call.context.session_id,
                workflow_run.workflow_id(),
            )?;
            Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "workflow_id": console.workflow_id(),
                    "entries": console.entries().iter().map(|entry| serde_json::json!({
                        "timestamp_ms": entry.timestamp_ms(),
                        "source_node_run_id": entry.source_node_run_id(),
                        "source_agent_id": entry.source_agent_id(),
                        "text": entry.text(),
                    })).collect::<Vec<_>>(),
                }),
            })
        }
        WORKFLOW_CONSOLE_WRITE_TOOL => {
            let args =
                serde_json::from_value::<WorkflowConsoleWriteArgs>(call.arguments).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "runtime_tool_workflow_console_write",
                        message: format!("invalid tool arguments: {error}"),
                    }
                })?;
            let workflow_run = app
                .sessions()
                .resolve_workflow_run_ref(&call.context.session_id, &call.context.workflow_run_ref)?;
            let entry = crate::scheduler::runtime::write_workflow_console(
                app,
                &call.context.session_id,
                workflow_run.workflow_id(),
                &call.context.workflow_node_run_id,
                &args.text,
            )?;
            Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "timestamp_ms": entry.timestamp_ms(),
                    "source_node_run_id": entry.source_node_run_id(),
                    "source_agent_id": entry.source_agent_id(),
                    "text": entry.text(),
                }),
            })
        }
        WORKFLOW_CONSOLE_CLEAR_TOOL => {
            let workflow_run = app
                .sessions()
                .resolve_workflow_run_ref(&call.context.session_id, &call.context.workflow_run_ref)?;
            crate::scheduler::runtime::clear_workflow_console(
                app,
                &call.context.session_id,
                workflow_run.workflow_id(),
            )?;
            Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "cleared": true,
                    "workflow_id": workflow_run.workflow_id(),
                }),
            })
        }
        other => Err(DaemonError::LocalTransport {
            operation: "dispatch_runtime_tool_call",
            message: format!("unsupported runtime tool `{other}`"),
        }),
    }
}

pub fn dispatch_authenticated_runtime_tool_call(
    app: &mut DaemonApp,
    auth_token: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<RuntimeToolResult, DaemonError> {
    let canonical_tool_name = canonical_runtime_tool_name(tool_name).to_string();
    let provider_run = app
        .providers()
        .get_run_by_runtime_mcp_auth_token(auth_token)
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "dispatch_authenticated_runtime_tool_call",
            message: "invalid runtime MCP auth token".to_string(),
        })?;
    let agent_id = provider_run.agent_instance_id();
    let requested_delivery_token = match canonical_tool_name.as_str() {
        ACK_WORKFLOW_TURN_TOOL => serde_json::from_value::<AckWorkflowTurnArgs>(arguments.clone())
            .ok()
            .map(|args| args.delivery_token),
        VALIDATE_WORKFLOW_OUTPUT_TOOL => {
            serde_json::from_value::<ValidateWorkflowOutputArgs>(arguments.clone())
                .ok()
                .and_then(|args| args.delivery_token)
        }
        WORKFLOW_CONSOLE_READ_TOOL | WORKFLOW_CONSOLE_WRITE_TOOL | WORKFLOW_CONSOLE_CLEAR_TOOL => {
            None
        }
        _ => None,
    };
    let (workflow_run_ref, workflow_node_run_id) = resolve_authenticated_workflow_turn(
        app,
        provider_run.session_id(),
        agent_id,
        requested_delivery_token.as_deref(),
    )?;
    let allowed_output_schema_refs = allowed_output_schema_refs_for_active_workflow_turn(
        app,
        provider_run.session_id(),
        &workflow_run_ref,
        &workflow_node_run_id,
    )?;

    dispatch_runtime_tool_call(
        app,
        RuntimeToolCall {
            tool_name: canonical_tool_name,
            arguments,
            context: WorkflowRuntimeToolContext {
                session_id: provider_run.session_id().to_string(),
                workflow_run_ref,
                workflow_node_run_id,
                delivery_token: None,
                allowed_output_schema_refs,
            },
        },
    )
}

fn resolve_authenticated_workflow_turn(
    app: &DaemonApp,
    session_id: &str,
    agent_id: Option<&str>,
    delivery_token: Option<&str>,
) -> Result<(String, String), DaemonError> {
    let session = app.sessions().get_session(session_id)?;
    if let Some(prompt) = session.active_prompt() {
        if let (Some(workflow_run_ref), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        {
            if let Some(requested_delivery_token) = delivery_token {
                let matches_active_token = session
                    .workflow_runs()
                    .iter()
                    .find(|workflow_run| workflow_run.id() == workflow_run_ref)
                    .and_then(|workflow_run| {
                        workflow_run
                            .node_runs()
                            .iter()
                            .find(|node_run| node_run.id() == workflow_node_run_id)
                    })
                    .and_then(|node_run| node_run.turn_envelope())
                    .is_some_and(|envelope| envelope.delivery_token() == requested_delivery_token);
                if matches_active_token {
                    return Ok((
                        workflow_run_ref.to_string(),
                        workflow_node_run_id.to_string(),
                    ));
                }
            } else {
                return Ok((workflow_run_ref.to_string(), workflow_node_run_id.to_string()));
            }
        }
    }
    if let Some(requested_delivery_token) = delivery_token {
        let mut exact_matches = session
            .workflow_runs()
            .iter()
            .flat_map(|workflow_run| {
                workflow_run.node_runs().iter().filter_map(|node_run| {
                    let envelope = node_run.turn_envelope()?;
                    if envelope.delivery_token() != requested_delivery_token {
                        return None;
                    }
                    if agent_id.is_some_and(|agent_id| node_run.agent_id() != agent_id) {
                        return None;
                    }
                    Some((workflow_run.id().to_string(), node_run.id().to_string()))
                })
            })
            .collect::<Vec<_>>();
        if exact_matches.len() == 1 {
            return Ok(exact_matches.remove(0));
        }
    }

    let running_turns = session
        .workflow_runs()
        .iter()
        .flat_map(|workflow_run| {
            workflow_run.node_runs().iter().filter_map(|node_run| {
                let envelope = node_run.turn_envelope()?;
                if node_run.status() != WorkflowNodeRunStatus::Running
                    || !matches!(
                        envelope.state(),
                        WorkflowTurnRuntimeState::Prepared
                            | WorkflowTurnRuntimeState::Dispatched
                            | WorkflowTurnRuntimeState::Acknowledged
                    )
                {
                    return None;
                }
                Some((
                    workflow_run.id().to_string(),
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    envelope.delivery_token().to_string(),
                ))
            })
        })
        .collect::<Vec<_>>();

    if let Some(requested_delivery_token) = delivery_token {
        let mut matching_by_delivery_token = running_turns
            .iter()
            .filter(|(_, _, _, candidate_delivery_token)| {
                candidate_delivery_token == requested_delivery_token
            })
            .map(|(workflow_run_id, workflow_node_run_id, _, _)| {
                (workflow_run_id.clone(), workflow_node_run_id.clone())
            })
            .collect::<Vec<_>>();
        if matching_by_delivery_token.len() == 1 {
            return Ok(matching_by_delivery_token.remove(0));
        }
    }

    let mut candidates = running_turns
        .iter()
        .filter(|(_, _, candidate_agent_id, _)| {
            agent_id.is_some_and(|agent_id| candidate_agent_id == agent_id)
        })
        .map(|(workflow_run_id, workflow_node_run_id, _, candidate_delivery_token)| {
            (
                workflow_run_id.clone(),
                workflow_node_run_id.clone(),
                candidate_delivery_token.clone(),
            )
        })
        .collect::<Vec<_>>();

    if let Some(requested_delivery_token) = delivery_token {
        candidates.retain(|(_, _, candidate_delivery_token)| {
            candidate_delivery_token == requested_delivery_token
        });
    }

    match candidates.len() {
        1 => {
            let (workflow_run_id, workflow_node_run_id, _) =
                candidates.into_iter().next().expect("candidate should exist");
            Ok((workflow_run_id, workflow_node_run_id))
        }
        0 => {
            if running_turns.len() == 1 {
                let (workflow_run_id, workflow_node_run_id, _, _) =
                    running_turns.into_iter().next().expect("candidate should exist");
                return Ok((workflow_run_id, workflow_node_run_id));
            }
            Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "no active workflow turn for authenticated provider run".to_string(),
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "dispatch_authenticated_runtime_tool_call",
            message: "multiple workflow turns matched the authenticated provider run".to_string(),
        }),
    }
}

fn allowed_output_schema_refs_for_active_workflow_turn(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_ref: &str,
    workflow_node_run_id: &str,
) -> Result<Vec<String>, DaemonError> {
    let workflow_run = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_ref)?;
    let workflow = app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())?;
    let node_id = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)
        .map(|node_run| node_run.node_id())
        .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: workflow.id().to_string(),
            reference: workflow_node_run_id.to_string(),
            message: "workflow node run was not found while resolving runtime tool scope",
        })?;
    Ok(workflow
        .edges()
        .iter()
        .filter(|edge| edge.from_node_id() == node_id)
        .filter_map(|edge| edge.output_schema_ref().map(str::to_string))
        .collect())
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
        dispatch_authenticated_runtime_tool_call, dispatch_runtime_tool_call,
        workflow_runtime_tool_specs, RuntimeToolCall, WorkflowRuntimeToolContext,
        ACK_WORKFLOW_TURN_TOOL, VALIDATE_WORKFLOW_OUTPUT_TOOL, WORKFLOW_CONSOLE_CLEAR_TOOL,
        WORKFLOW_CONSOLE_READ_TOOL, WORKFLOW_CONSOLE_WRITE_TOOL,
    };

    #[test]
    fn workflow_runtime_tool_specs_expose_ack_and_validation() {
        let specs = workflow_runtime_tool_specs();
        assert_eq!(specs.len(), 5);
        assert_eq!(specs[0].name, ACK_WORKFLOW_TURN_TOOL);
        assert_eq!(specs[1].name, VALIDATE_WORKFLOW_OUTPUT_TOOL);
        assert_eq!(specs[2].name, WORKFLOW_CONSOLE_READ_TOOL);
        assert_eq!(specs[3].name, WORKFLOW_CONSOLE_WRITE_TOOL);
        assert_eq!(specs[4].name, WORKFLOW_CONSOLE_CLEAR_TOOL);
    }

    #[test]
    fn validation_tool_enforces_allowed_schema_refs() {
        let temp_dir =
            std::env::temp_dir().join(format!("arroba-runtime-tools-{}", std::process::id()));
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
            crate::local::LocalDaemonResponse::WorkflowCreated { workflow, .. } => {
                workflow.id().to_string()
            }
            other => panic!("unexpected response: {other:?}"),
        };
        let node_id = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    agent_id: agent_id.clone(),
                },
            ))
            .expect("node should be added")
        {
            crate::local::LocalDaemonResponse::WorkflowNodeAdded { node, .. } => {
                node.id().to_string()
            }
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
        match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
                CreateWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    entry_node_id: node_id.clone(),
                    alias: Some("entry".to_string()),
                },
            ))
            .expect("endpoint should be added")
        {
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
            crate::local::LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => {
                workflow_run
            }
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

    #[test]
    fn authenticated_ack_runtime_tool_resolves_workflow_turn_without_active_prompt() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-auth", "worktree-auth"),
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
            "client-auth",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
        let agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-auth".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-auth".to_string()),
            }))
            .expect("agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let workflow_id = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("wf-auth".to_string()),
            }))
            .expect("workflow should exist")
        {
            crate::local::LocalDaemonResponse::WorkflowCreated { workflow, .. } => {
                workflow.id().to_string()
            }
            other => panic!("unexpected response: {other:?}"),
        };
        let node_id = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    agent_id: agent_id.clone(),
                },
            ))
            .expect("node should be added")
        {
            crate::local::LocalDaemonResponse::WorkflowNodeAdded { node, .. } => {
                node.id().to_string()
            }
            other => panic!("unexpected response: {other:?}"),
        };
        app.handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                entry_node_id: node_id,
                alias: Some("entry".to_string()),
            },
        ))
        .expect("endpoint should be added");
        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id,
                    endpoint_ref: "entry".to_string(),
                    prompt: Some("start".to_string()),
                },
            ))
            .expect("workflow should invoke")
        {
            crate::local::LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => {
                workflow_run
            }
            other => panic!("unexpected response: {other:?}"),
        };
        let node_run = workflow_run
            .node_runs()
            .first()
            .expect("node run should exist");
        let delivery_token = node_run
            .turn_envelope()
            .expect("turn envelope should be prepared")
            .delivery_token()
            .to_string();
        let auth_token = app
            .providers()
            .get_run_for_agent(session.id(), &agent_id)
            .and_then(|run| run.runtime_mcp_auth_token().map(str::to_string))
            .expect("runtime auth token should exist");
        app.sessions_mut()
            .complete_active_prompt_only(session.id())
            .expect("active prompt should be clearable for test");

        let result = dispatch_authenticated_runtime_tool_call(
            &mut app,
            &auth_token,
            ACK_WORKFLOW_TURN_TOOL,
            serde_json::json!({
                "delivery_token": delivery_token,
            }),
        )
        .expect("authenticated ack runtime tool should succeed");

        assert_eq!(result.payload["state"], "acknowledged");
    }

}
