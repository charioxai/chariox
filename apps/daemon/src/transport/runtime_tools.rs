use arroba_relay::protocol::ClientTarget;
use jsonschema::JSONSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::execution_lease::RemoteWorkflowTurnContext;
use crate::session::{
    WorkflowArtifactRef, WorkflowNodeRunStatus, WorkflowOutputPayload,
    WorkflowRuntimeToolCallEvent, WorkflowTurnRuntimeState,
};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

pub const ACK_WORKFLOW_TURN_TOOL: &str = "ack_workflow_turn";
pub const VALIDATE_WORKFLOW_OUTPUT_TOOL: &str = "validate_workflow_output";
pub const VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL: &str =
    "validate_and_submit_workflow_run_output";
pub const VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL: &str =
    "validate_and_submit_intermediate_workflow_run_output";
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
    pub workflow_run_output_schema_ref: Option<String>,
    pub workflow_intermediate_output_schema_ref: Option<String>,
    pub can_complete_workflow_run: bool,
    pub can_emit_intermediate_workflow_run_output: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateAndSubmitWorkflowRunOutputArgs {
    pub workflow_output_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowRunOutputSubmissionToolKind {
    Final,
    Intermediate,
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
            name: VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.to_string(),
            description: "Validate and submit the final workflow run output for the current workflow turn.".to_string(),
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
            description: "Validate and submit an intermediate workflow run output for the current workflow turn without terminating the run.".to_string(),
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
    let canonical_tool_name = canonical_runtime_tool_name(call.tool_name.as_str()).to_string();
    let arguments_json = serde_json::to_string(&call.arguments)
        .unwrap_or_else(|_| String::from("<unserializable runtime tool arguments>"));
    let result = match canonical_tool_name.as_str() {
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
        VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL => dispatch_workflow_run_output_submission(
            app,
            &call,
            WorkflowRunOutputSubmissionToolKind::Final,
        ),
        VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL => {
            dispatch_workflow_run_output_submission(
                app,
                &call,
                WorkflowRunOutputSubmissionToolKind::Intermediate,
            )
        }
        WORKFLOW_CONSOLE_READ_TOOL => {
            let workflow_run = app.sessions().resolve_workflow_run_ref(
                &call.context.session_id,
                &call.context.workflow_run_ref,
            )?;
            let console = app.read_workflow_console_from_runtime(
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
            let args = serde_json::from_value::<WorkflowConsoleWriteArgs>(call.arguments).map_err(
                |error| DaemonError::LocalTransport {
                    operation: "runtime_tool_workflow_console_write",
                    message: format!("invalid tool arguments: {error}"),
                },
            )?;
            let workflow_run = app.sessions().resolve_workflow_run_ref(
                &call.context.session_id,
                &call.context.workflow_run_ref,
            )?;
            let entry = app.write_workflow_console_from_runtime(
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
            let workflow_run = app.sessions().resolve_workflow_run_ref(
                &call.context.session_id,
                &call.context.workflow_run_ref,
            )?;
            app.clear_workflow_console_from_runtime(
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
    };

    let result_json = match &result {
        Ok(result) => Some(
            serde_json::to_string(&result.payload)
                .unwrap_or_else(|_| String::from("<unserializable runtime tool result>")),
        ),
        Err(error) => Some(serde_json::json!({"error": error.to_string()}).to_string()),
    };
    let ok = result.as_ref().map(|entry| entry.ok).unwrap_or(false);
    let _ = app.sessions_mut().record_workflow_runtime_tool_call(
        &call.context.session_id,
        &call.context.workflow_node_run_id,
        WorkflowRuntimeToolCallEvent::new(canonical_tool_name, arguments_json, result_json, ok),
    );
    let _ = app.publish_session_projection(&call.context.session_id);

    result
}

fn dispatch_workflow_run_output_submission(
    app: &mut DaemonApp,
    call: &RuntimeToolCall,
    kind: WorkflowRunOutputSubmissionToolKind,
) -> Result<RuntimeToolResult, DaemonError> {
    enforce_workflow_run_output_submission_permission(call, kind)?;
    let args =
        serde_json::from_value::<ValidateAndSubmitWorkflowRunOutputArgs>(call.arguments.clone())
            .map_err(|error| DaemonError::LocalTransport {
                operation: workflow_run_output_submission_operation(kind),
                message: format!("invalid tool arguments: {error}"),
            })?;
    let workflow_run_id = app
        .sessions()
        .resolve_workflow_run_ref(&call.context.session_id, &call.context.workflow_run_ref)?
        .id()
        .to_string();
    let warning =
        workflow_run_output_submission_schema_ref(&call.context, kind).and_then(|schema_ref| {
            validate_workflow_output_schema(schema_ref, &args.workflow_output_json).err()
        });
    let output =
        WorkflowOutputPayload::new(args.workflow_output_json, Vec::<WorkflowArtifactRef>::new());
    let workflow_run = match kind {
        WorkflowRunOutputSubmissionToolKind::Final => {
            app.sessions_mut().submit_workflow_run_final_output(
                &call.context.session_id,
                &workflow_run_id,
                &call.context.workflow_node_run_id,
                output,
                warning.is_none(),
                warning.clone(),
            )?
        }
        WorkflowRunOutputSubmissionToolKind::Intermediate => {
            app.sessions_mut().submit_workflow_run_intermediate_output(
                &call.context.session_id,
                &workflow_run_id,
                &call.context.workflow_node_run_id,
                output,
                warning.is_none(),
                warning.clone(),
            )?
        }
    };
    Ok(RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({
            "submitted": true,
            "valid": warning.is_none(),
            "warning": warning,
            "workflow_run_id": workflow_run.id(),
            "workflow_node_run_id": call.context.workflow_node_run_id,
        }),
    })
}

fn enforce_workflow_run_output_submission_permission(
    call: &RuntimeToolCall,
    kind: WorkflowRunOutputSubmissionToolKind,
) -> Result<(), DaemonError> {
    let allowed = match kind {
        WorkflowRunOutputSubmissionToolKind::Final => call.context.can_complete_workflow_run,
        WorkflowRunOutputSubmissionToolKind::Intermediate => {
            call.context.can_emit_intermediate_workflow_run_output
        }
    };
    if allowed {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: workflow_run_output_submission_operation(kind),
        message: match kind {
            WorkflowRunOutputSubmissionToolKind::Final => {
                "current workflow node run is not allowed to complete the workflow run".to_string()
            }
            WorkflowRunOutputSubmissionToolKind::Intermediate => {
                "current workflow node run is not allowed to emit intermediate workflow run output"
                    .to_string()
            }
        },
    })
}

fn workflow_run_output_submission_schema_ref<'a>(
    context: &'a WorkflowRuntimeToolContext,
    kind: WorkflowRunOutputSubmissionToolKind,
) -> Option<&'a str> {
    match kind {
        WorkflowRunOutputSubmissionToolKind::Final => {
            context.workflow_run_output_schema_ref.as_deref()
        }
        WorkflowRunOutputSubmissionToolKind::Intermediate => {
            context.workflow_intermediate_output_schema_ref.as_deref()
        }
    }
}

fn workflow_run_output_submission_operation(
    kind: WorkflowRunOutputSubmissionToolKind,
) -> &'static str {
    match kind {
        WorkflowRunOutputSubmissionToolKind::Final => {
            "runtime_tool_validate_and_submit_workflow_run_output"
        }
        WorkflowRunOutputSubmissionToolKind::Intermediate => {
            "runtime_tool_validate_and_submit_intermediate_workflow_run_output"
        }
    }
}

pub fn dispatch_authenticated_runtime_tool_call(
    app: &mut DaemonApp,
    auth_token: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<RuntimeToolResult, DaemonError> {
    let canonical_tool_name = canonical_runtime_tool_name(tool_name).to_string();
    let provider_runs = app
        .providers()
        .get_runs_by_runtime_mcp_auth_token(auth_token);
    if provider_runs.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "dispatch_authenticated_runtime_tool_call",
            message: "invalid runtime MCP auth token".to_string(),
        });
    }
    let requested_delivery_token = match canonical_tool_name.as_str() {
        ACK_WORKFLOW_TURN_TOOL => serde_json::from_value::<AckWorkflowTurnArgs>(arguments.clone())
            .ok()
            .map(|args| args.delivery_token),
        VALIDATE_WORKFLOW_OUTPUT_TOOL => {
            serde_json::from_value::<ValidateWorkflowOutputArgs>(arguments.clone())
                .ok()
                .and_then(|args| args.delivery_token)
        }
        VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL => {
            serde_json::from_value::<ValidateAndSubmitWorkflowRunOutputArgs>(arguments.clone())
                .ok()
                .and_then(|args| args.delivery_token)
        }
        VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL => {
            serde_json::from_value::<ValidateAndSubmitWorkflowRunOutputArgs>(arguments.clone())
                .ok()
                .and_then(|args| args.delivery_token)
        }
        WORKFLOW_CONSOLE_READ_TOOL | WORKFLOW_CONSOLE_WRITE_TOOL | WORKFLOW_CONSOLE_CLEAR_TOOL => {
            None
        }
        _ => None,
    };
    let leased_binding_candidates = provider_runs
        .iter()
        .filter_map(|run| {
            app.leased_workflow_turn_binding_for_provider_run(run.id())
                .map(|binding| (run, binding))
        })
        .collect::<Vec<_>>();
    let mut selected_leased_binding = requested_delivery_token
        .as_deref()
        .and_then(|delivery_token| {
            leased_binding_candidates
                .iter()
                .find(|(_, binding)| binding.context.delivery_token == delivery_token)
                .map(|(_, binding)| binding.clone())
        })
        .or_else(|| {
            let delivery_token = requested_delivery_token.as_deref()?;
            let workflow_node_run_id = delivery_token.strip_prefix("workflow-ack:")?;
            let mut binding = leased_binding_candidates
                .first()
                .map(|(_, binding)| binding.clone())?;
            binding.context.workflow_node_run_id = workflow_node_run_id.to_string();
            binding.context.delivery_token = delivery_token.to_string();
            Some(binding)
        });
    if selected_leased_binding.is_none() {
        let mut active_leased_bindings = Vec::new();
        for (run, binding) in &leased_binding_candidates {
            let Some(agent_id) = run.agent_instance_id() else {
                continue;
            };
            if app
                .prompt_owner_active_prompt_for_agent(run.session_id(), agent_id)?
                .is_some()
            {
                active_leased_bindings.push(binding.clone());
            }
        }
        selected_leased_binding = if active_leased_bindings.len() == 1 {
            active_leased_bindings.into_iter().next()
        } else if leased_binding_candidates.len() == 1 {
            leased_binding_candidates
                .first()
                .map(|(_, binding)| binding.clone())
        } else {
            None
        };
    }
    if let Some(binding) = selected_leased_binding {
        let response = app.block_on_relay_future(send_peer_request_via_temporary_connection(
            app.config(),
            ClientTarget {
                daemon_id: Some(binding.context.home_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::ForwardWorkflowRuntimeTool {
                context: binding.context,
                tool_name: canonical_tool_name.clone(),
                arguments,
            },
        ))?;
        return match response {
            RelayPeerResponse::WorkflowRuntimeToolHandled { result } => Ok(result),
            other => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: format!("unexpected forwarded workflow runtime tool response: {other:?}"),
            }),
        };
    }
    let session_id = provider_runs[0].session_id().to_string();
    let candidate_agent_ids = provider_runs
        .iter()
        .filter_map(|run| run.agent_instance_id().map(str::to_string))
        .collect::<Vec<_>>();
    let (workflow_run_ref, workflow_node_run_id) = resolve_authenticated_workflow_turn(
        app,
        &session_id,
        &candidate_agent_ids,
        requested_delivery_token.as_deref(),
    )?;
    let allowed_output_schema_refs = allowed_output_schema_refs_for_active_workflow_turn(
        app,
        &session_id,
        &workflow_run_ref,
        &workflow_node_run_id,
    )?;
    let (
        workflow_run_output_schema_ref,
        workflow_intermediate_output_schema_ref,
        can_complete_workflow_run,
        can_emit_intermediate_workflow_run_output,
    ) = workflow_run_completion_context_for_active_workflow_turn(
        app,
        &session_id,
        &workflow_run_ref,
        &workflow_node_run_id,
    )?;

    dispatch_runtime_tool_call(
        app,
        RuntimeToolCall {
            tool_name: canonical_tool_name,
            arguments,
            context: WorkflowRuntimeToolContext {
                session_id,
                workflow_run_ref,
                workflow_node_run_id,
                delivery_token: None,
                allowed_output_schema_refs,
                workflow_run_output_schema_ref,
                workflow_intermediate_output_schema_ref,
                can_complete_workflow_run,
                can_emit_intermediate_workflow_run_output,
            },
        },
    )
}

pub fn dispatch_forwarded_workflow_runtime_tool_call(
    app: &mut DaemonApp,
    context: RemoteWorkflowTurnContext,
    tool_name: String,
    arguments: Value,
) -> Result<RuntimeToolResult, DaemonError> {
    let allowed_output_schema_refs = allowed_output_schema_refs_for_active_workflow_turn(
        app,
        &context.home_session_id,
        &context.workflow_run_id,
        &context.workflow_node_run_id,
    )?;
    let (
        workflow_run_output_schema_ref,
        workflow_intermediate_output_schema_ref,
        can_complete_workflow_run,
        can_emit_intermediate_workflow_run_output,
    ) = workflow_run_completion_context_for_active_workflow_turn(
        app,
        &context.home_session_id,
        &context.workflow_run_id,
        &context.workflow_node_run_id,
    )?;
    dispatch_runtime_tool_call(
        app,
        RuntimeToolCall {
            tool_name,
            arguments,
            context: WorkflowRuntimeToolContext {
                session_id: context.home_session_id,
                workflow_run_ref: context.workflow_run_id,
                workflow_node_run_id: context.workflow_node_run_id,
                delivery_token: Some(context.delivery_token),
                allowed_output_schema_refs,
                workflow_run_output_schema_ref,
                workflow_intermediate_output_schema_ref,
                can_complete_workflow_run,
                can_emit_intermediate_workflow_run_output,
            },
        },
    )
}

fn resolve_authenticated_workflow_turn(
    app: &mut DaemonApp,
    session_id: &str,
    candidate_agent_ids: &[String],
    delivery_token: Option<&str>,
) -> Result<(String, String), DaemonError> {
    let session = app.sessions().get_session(session_id)?;
    let agent_matches = |node_run: &crate::session::WorkflowNodeRun| {
        candidate_agent_ids.is_empty()
            || candidate_agent_ids
                .iter()
                .any(|agent_id| node_run.agent_id() == agent_id)
    };
    for prompt in active_workflow_prompts_for_candidates(app, &session, candidate_agent_ids)? {
        let (Some(workflow_run_ref), Some(workflow_node_run_id)) =
            (prompt.workflow_run_id(), prompt.workflow_node_run_id())
        else {
            continue;
        };
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
                .filter(|node_run| agent_matches(node_run))
                .and_then(|node_run| node_run.turn_envelope())
                .is_some_and(|envelope| envelope.delivery_token() == requested_delivery_token);
            if matches_active_token {
                return Ok((
                    workflow_run_ref.to_string(),
                    workflow_node_run_id.to_string(),
                ));
            }
        } else {
            let matches_active_agent = session
                .workflow_runs()
                .iter()
                .find(|workflow_run| workflow_run.id() == workflow_run_ref)
                .and_then(|workflow_run| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| node_run.id() == workflow_node_run_id)
                })
                .is_some_and(agent_matches);
            if matches_active_agent {
                return Ok((
                    workflow_run_ref.to_string(),
                    workflow_node_run_id.to_string(),
                ));
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
                    if !agent_matches(node_run) {
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
            candidate_agent_ids.is_empty()
                || candidate_agent_ids
                    .iter()
                    .any(|agent_id| candidate_agent_id == agent_id)
        })
        .map(
            |(workflow_run_id, workflow_node_run_id, _, candidate_delivery_token)| {
                (
                    workflow_run_id.clone(),
                    workflow_node_run_id.clone(),
                    candidate_delivery_token.clone(),
                )
            },
        )
        .collect::<Vec<_>>();

    if let Some(requested_delivery_token) = delivery_token {
        candidates.retain(|(_, _, candidate_delivery_token)| {
            candidate_delivery_token == requested_delivery_token
        });
    }

    match candidates.len() {
        1 => {
            let (workflow_run_id, workflow_node_run_id, _) = candidates
                .into_iter()
                .next()
                .expect("candidate should exist");
            Ok((workflow_run_id, workflow_node_run_id))
        }
        0 => {
            if running_turns.len() == 1 {
                let (workflow_run_id, workflow_node_run_id, _, _) = running_turns
                    .into_iter()
                    .next()
                    .expect("candidate should exist");
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

fn active_workflow_prompts_for_candidates(
    app: &mut DaemonApp,
    session: &crate::session::RuntimeSession,
    candidate_agent_ids: &[String],
) -> Result<Vec<crate::session::PromptQueueItem>, DaemonError> {
    let mut agent_ids = if candidate_agent_ids.is_empty() {
        let mut ids = session
            .agents()
            .iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        ids.extend(session.prompt_states().keys().cloned());
        ids
    } else {
        candidate_agent_ids.to_vec()
    };
    agent_ids.sort();
    agent_ids.dedup();

    let mut prompts = Vec::new();
    for agent_id in agent_ids {
        if let Some(prompt) = app.prompt_owner_active_prompt_for_agent(session.id(), &agent_id)? {
            prompts.push(prompt);
        }
    }
    Ok(prompts)
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

fn workflow_run_completion_context_for_active_workflow_turn(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_ref: &str,
    workflow_node_run_id: &str,
) -> Result<(Option<String>, Option<String>, bool, bool), DaemonError> {
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
            message: "workflow node run was not found while resolving completion scope",
        })?;
    let node = workflow.node(node_id);
    let can_complete = node.is_some_and(|node| node.can_complete_workflow_run());
    let can_emit_intermediate = node.is_some_and(|node| node.can_emit_intermediate_run_output());
    let intermediate_schema_ref = node
        .and_then(|node| node.intermediate_output_schema_ref())
        .map(str::to_string)
        .or_else(|| {
            workflow
                .intermediate_output_schema_ref()
                .map(str::to_string)
        });
    Ok((
        workflow.run_output_schema_ref().map(str::to_string),
        intermediate_schema_ref,
        can_complete,
        can_emit_intermediate,
    ))
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
    use crate::session::{
        CreateSessionRequest, WorkflowTurnRuntimeState, WorkflowTurnSubmissionKind,
    };
    use crate::{DaemonApp, DaemonConfig};

    use super::{
        dispatch_authenticated_runtime_tool_call, dispatch_runtime_tool_call,
        resolve_authenticated_workflow_turn, workflow_runtime_tool_specs, RuntimeToolCall,
        WorkflowRuntimeToolContext, ACK_WORKFLOW_TURN_TOOL,
        VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL,
        VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL, VALIDATE_WORKFLOW_OUTPUT_TOOL,
        WORKFLOW_CONSOLE_CLEAR_TOOL, WORKFLOW_CONSOLE_READ_TOOL, WORKFLOW_CONSOLE_WRITE_TOOL,
    };

    #[test]
    fn workflow_runtime_tool_specs_expose_ack_and_validation() {
        let specs = workflow_runtime_tool_specs();
        assert_eq!(specs.len(), 7);
        assert_eq!(specs[0].name, ACK_WORKFLOW_TURN_TOOL);
        assert_eq!(specs[1].name, VALIDATE_WORKFLOW_OUTPUT_TOOL);
        assert_eq!(specs[2].name, VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL);
        assert_eq!(
            specs[3].name,
            VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL
        );
        assert_eq!(specs[4].name, WORKFLOW_CONSOLE_READ_TOOL);
        assert_eq!(specs[5].name, WORKFLOW_CONSOLE_WRITE_TOOL);
        assert_eq!(specs[6].name, WORKFLOW_CONSOLE_CLEAR_TOOL);
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
                    workflow_run_output_schema_ref: None,
                    workflow_intermediate_output_schema_ref: None,
                    can_complete_workflow_run: false,
                    can_emit_intermediate_workflow_run_output: false,
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
                machine_ref: None,
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
        let node_run_id = node_run.id().to_string();
        let delivery_token = envelope.delivery_token().to_string();
        assert_eq!(envelope.state(), WorkflowTurnRuntimeState::Dispatched);
        app.remove_session_projection(session.id());

        let result = dispatch_runtime_tool_call(
            &mut app,
            RuntimeToolCall {
                tool_name: ACK_WORKFLOW_TURN_TOOL.to_string(),
                arguments: serde_json::json!({
                    "delivery_token": delivery_token.clone(),
                }),
                context: WorkflowRuntimeToolContext {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: node_run_id.clone(),
                    delivery_token: Some(delivery_token),
                    allowed_output_schema_refs: Vec::new(),
                    workflow_run_output_schema_ref: None,
                    workflow_intermediate_output_schema_ref: None,
                    can_complete_workflow_run: false,
                    can_emit_intermediate_workflow_run_output: false,
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
            .find(|candidate| candidate.id() == node_run_id)
            .expect("updated node run should exist");
        assert_eq!(
            updated_node_run
                .turn_envelope()
                .expect("updated envelope should exist")
                .state(),
            WorkflowTurnRuntimeState::Acknowledged
        );
        let projected_session = app
            .session_state_projection_store()
            .get(session.id())
            .expect("runtime tool mutation should republish session projection");
        let projected_node_run = projected_session
            .workflow_run(workflow_run.id())
            .expect("projected workflow run should exist")
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == node_run_id)
            .expect("projected node run should exist");
        assert_eq!(
            projected_node_run
                .turn_envelope()
                .expect("projected envelope should exist")
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
                    workflow_run_output_schema_ref: None,
                    workflow_intermediate_output_schema_ref: None,
                    can_complete_workflow_run: false,
                    can_emit_intermediate_workflow_run_output: false,
                },
            },
        )
        .expect("allowed schema ref should validate");
        assert_eq!(result.payload["valid"], true);
    }

    #[test]
    fn validate_and_submit_workflow_run_output_stores_invalid_output_with_warning() {
        let temp_dir = std::env::temp_dir().join(format!(
            "arroba-runtime-tools-run-output-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&temp_dir);
        let schema_path = temp_dir.join("workflow-run-output-schema.json");
        fs::write(
            &schema_path,
            r#"{"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}}"#,
        )
        .expect("schema fixture should be written");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-run-output", "worktree-run-output"),
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
            "client-run-output",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
        let agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-run-output".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-run-output".to_string()),
                machine_ref: None,
            }))
            .expect("agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let workflow_id = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("wf-run-output".to_string()),
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
        app.sessions_mut()
            .set_workflow_node_can_complete_run(session.id(), &workflow_id, &node_id, true)
            .expect("node completion setting should update");
        app.sessions_mut()
            .set_workflow_run_output_schema_ref(
                session.id(),
                &workflow_id,
                Some(schema_path.to_string_lossy().to_string()),
            )
            .expect("workflow run output schema should update");
        app.handle_local_request(LocalDaemonRequest::SetWorkflowFlushContext(
            crate::local::SetWorkflowFlushContextRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                flush_agent_context_before_run: false,
            },
        ))
        .expect("workflow flush context should update");
        app.handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                entry_node_id: node_id.clone(),
                alias: Some("entry".to_string()),
            },
        ))
        .expect("endpoint should be added");
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

        let result = dispatch_runtime_tool_call(
            &mut app,
            RuntimeToolCall {
                tool_name: VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL.to_string(),
                arguments: serde_json::json!({
                    "workflow_output_json": "{\"value\":\"bad\"}"
                }),
                context: WorkflowRuntimeToolContext {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: node_run.id().to_string(),
                    delivery_token: None,
                    allowed_output_schema_refs: Vec::new(),
                    workflow_run_output_schema_ref: Some(schema_path.to_string_lossy().to_string()),
                    workflow_intermediate_output_schema_ref: None,
                    can_complete_workflow_run: true,
                    can_emit_intermediate_workflow_run_output: false,
                },
            },
        )
        .expect("workflow run output submission should succeed");

        assert_eq!(result.payload["submitted"], true);
        assert_eq!(result.payload["valid"], false);
        let updated_run = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should still exist");
        assert!(updated_run.final_output().is_none());
        let updated_node_run = updated_run
            .node_runs()
            .iter()
            .find(|candidate| candidate.id() == node_run.id())
            .expect("updated node run should exist");
        let pending = updated_node_run
            .turn_envelope()
            .and_then(|envelope| {
                envelope.pending_output_submission(WorkflowTurnSubmissionKind::Final)
            })
            .expect("pending final output should exist");
        assert_eq!(pending.output().message(), "{\"value\":\"bad\"}");
        assert!(!pending.valid());
        assert!(pending.warning().is_some());
    }

    #[test]
    fn intermediate_workflow_run_output_is_buffered_until_turn_completion() {
        let temp_dir = std::env::temp_dir().join(format!(
            "arroba-runtime-tools-intermediate-output-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&temp_dir);
        let schema_path = temp_dir.join("workflow-intermediate-output-schema.json");
        fs::write(
            &schema_path,
            r#"{"type":"object","required":["value"],"properties":{"value":{"type":"integer"}}}"#,
        )
        .expect("schema fixture should be written");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new(
                    "workspace-intermediate-output",
                    "worktree-intermediate-output",
                ),
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
            "client-intermediate-output",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
        let agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-intermediate-output".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-intermediate-output".to_string()),
                machine_ref: None,
            }))
            .expect("agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let workflow_id = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("wf-intermediate-output".to_string()),
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
        app.sessions_mut()
            .set_workflow_node_can_emit_intermediate_output(
                session.id(),
                &workflow_id,
                &node_id,
                true,
            )
            .expect("node intermediate output capability should update");
        app.sessions_mut()
            .set_workflow_intermediate_output_schema_ref(
                session.id(),
                &workflow_id,
                Some(schema_path.to_string_lossy().to_string()),
            )
            .expect("workflow intermediate output schema should update");
        app.handle_local_request(LocalDaemonRequest::SetWorkflowFlushContext(
            crate::local::SetWorkflowFlushContextRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                flush_agent_context_before_run: false,
            },
        ))
        .expect("workflow flush context should update");
        app.handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                entry_node_id: node_id.clone(),
                alias: Some("entry".to_string()),
            },
        ))
        .expect("endpoint should be added");
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

        let result = dispatch_runtime_tool_call(
            &mut app,
            RuntimeToolCall {
                tool_name: VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL.to_string(),
                arguments: serde_json::json!({
                    "workflow_output_json": "{\"value\":1842}"
                }),
                context: WorkflowRuntimeToolContext {
                    session_id: session.id().to_string(),
                    workflow_run_ref: workflow_run.id().to_string(),
                    workflow_node_run_id: node_run.id().to_string(),
                    delivery_token: None,
                    allowed_output_schema_refs: Vec::new(),
                    workflow_run_output_schema_ref: None,
                    workflow_intermediate_output_schema_ref: Some(
                        schema_path.to_string_lossy().to_string(),
                    ),
                    can_complete_workflow_run: false,
                    can_emit_intermediate_workflow_run_output: true,
                },
            },
        )
        .expect("intermediate workflow run output submission should succeed");
        assert_eq!(result.payload["submitted"], true);
        let buffered_run = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should still exist");
        assert!(buffered_run.intermediate_outputs().is_empty());

        app.sessions_mut()
            .complete_workflow_node_run(
                session.id(),
                workflow_run.id(),
                node_run.id(),
                Some(crate::session::WorkflowCompletionSnapshot::new(
                    "done", None,
                )),
                None,
            )
            .expect("node completion should succeed");
        let committed_run = app
            .sessions()
            .resolve_workflow_run_ref(session.id(), workflow_run.id())
            .expect("workflow run should still exist");
        assert_eq!(committed_run.intermediate_outputs().len(), 1);
        assert_eq!(
            committed_run.intermediate_outputs()[0].output().message(),
            "{\"value\":1842}"
        );
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
                machine_ref: None,
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
        app.prompt_owner_complete_active_prompt_only(session.id(), &agent_id)
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

    #[test]
    fn authenticated_turn_resolution_handles_shared_auth_candidates() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-auth-shared", "worktree-auth-shared"),
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
            "client-auth-shared",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");

        let active_agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-active".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-auth-shared".to_string()),
                machine_ref: None,
            }))
            .expect("active agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };
        let inactive_agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-inactive".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-auth-shared".to_string()),
                machine_ref: None,
            }))
            .expect("inactive agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };

        let workflow_id = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("wf-auth-shared".to_string()),
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
                    agent_id: active_agent_id.clone(),
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
        app.prompt_owner_complete_active_prompt_only(session.id(), &active_agent_id)
            .expect("active prompt should be clearable");

        let (resolved_workflow_run_id, resolved_node_run_id) = resolve_authenticated_workflow_turn(
            &mut app,
            session.id(),
            &[inactive_agent_id, active_agent_id],
            Some(&delivery_token),
        )
        .expect("shared auth candidates should still resolve exact delivery token");

        assert_eq!(resolved_workflow_run_id, workflow_run.id());
        assert_eq!(resolved_node_run_id, node_run.id());
    }
}
