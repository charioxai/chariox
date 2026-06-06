//! Workflow runtime-tool command handling.
//!
//! Workflow-executing agents use this path to resolve node context, complete nodes, trigger
//! retries, and surface workflow-specific tool results back through the runtime state.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn dispatch_workflow_runtime_tool_call(
        &self,
        tool_name: String,
        arguments: serde_json::Value,
        context: crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let canonical_tool_name = tool_name
            .strip_prefix("arroba_")
            .unwrap_or(tool_name.as_str())
            .to_string();
        let arguments_json = serde_json::to_string(&arguments)
            .unwrap_or_else(|_| String::from("<unserializable runtime tool arguments>"));
        let mut dispatches = WorkflowPromptDispatches::default();
        let result = match canonical_tool_name.as_str() {
            crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::AckWorkflowTurnArgs,
                >(arguments.clone())
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_ack_workflow_turn",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let workflow_run_id = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?
                    .id()
                    .to_string();
                let workflow_run = self.session_store.write().ack_workflow_turn(
                    &context.session_id,
                    &workflow_run_id,
                    &context.workflow_node_run_id,
                    &args.delivery_token,
                )?;
                Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::json!({
                        "workflow_run_id": workflow_run.id(),
                        "workflow_node_run_id": context.workflow_node_run_id,
                        "state": "acknowledged",
                        "next_action": "Continue this same workflow turn. This acknowledgement is not the final answer. If this turn requires final workflow run output, call validate_and_submit_workflow_run_output before stopping; otherwise emit the required final fenced json block before stopping.",
                    }),
                })
            }
            crate::transport::runtime_tools::VALIDATE_WORKFLOW_HANDOFF_TOOL => {
                self.workflow_validate_handoff_tool_result(&arguments, &context)
            }
            crate::transport::runtime_tools::READ_WORKFLOW_TURN_CONTEXT_TOOL => {
                self.workflow_turn_context_tool_result(&context)
            }
            crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL
            | crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_INTERMEDIATE_WORKFLOW_RUN_OUTPUT_TOOL =>
            {
                let is_final = canonical_tool_name
                    == crate::transport::runtime_tools::VALIDATE_AND_SUBMIT_WORKFLOW_RUN_OUTPUT_TOOL;
                self.workflow_submit_output_tool_result(&arguments, &context, is_final)
                    .map(|(result, next_dispatches)| {
                        dispatches.extend(next_dispatches);
                        result
                    })
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_READ_TOOL => {
                self.workflow_console_read_tool_result(&context)
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_WRITE_TOOL => {
                self.workflow_console_write_tool_result(&arguments, &context)
            }
            crate::transport::runtime_tools::WORKFLOW_CONSOLE_CLEAR_TOOL => {
                self.workflow_console_clear_tool_result(&context)
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
        let _ = self
            .session_store
            .write()
            .record_workflow_runtime_tool_call(
                &context.session_id,
                &context.workflow_node_run_id,
                crate::session::WorkflowRuntimeToolCallEvent::new(
                    canonical_tool_name,
                    arguments_json,
                    result_json,
                    ok,
                ),
            );
        let _ = self.session_snapshot(&context.session_id);
        result.map(|result| (result, dispatches))
    }

    fn workflow_turn_context_tool_result(
        &self,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
        let node_run = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == context.workflow_node_run_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_read_workflow_turn_context",
                message: format!(
                    "workflow node run `{}` not found in workflow run `{}`",
                    context.workflow_node_run_id,
                    workflow_run.id()
                ),
            })?;
        let node_id = node_run.node_id().to_string();
        let messages = workflow_run
            .messages()
            .iter()
            .filter(|message| {
                message.consumed_by_node_run_id() == Some(context.workflow_node_run_id.as_str())
                    || message.target_node_id() == node_id
            })
            .map(|message| {
                let parsed_handoff_payload =
                    serde_json::from_str::<serde_json::Value>(message.handoff_payload()).ok();
                serde_json::json!({
                    "id": message.id(),
                    "source_node_run_id": message.source_node_run_id(),
                    "target_node_id": message.target_node_id(),
                    "message_type": message.message_type(),
                    "summary": message.summary(),
                    "handoff_payload": message.handoff_payload(),
                    "parsed_handoff_payload": parsed_handoff_payload,
                    "consumed_by_node_run_id": message.consumed_by_node_run_id(),
                    "created_at_ms": message.created_at_ms(),
                })
            })
            .collect::<Vec<_>>();
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "session_id": context.session_id,
                "workflow_run_id": workflow_run.id(),
                "workflow_node_run_id": context.workflow_node_run_id,
                "node_id": node_id,
                "agent_id": node_run.agent_id(),
                "invocation_prompt": workflow_run.invocation_prompt(),
                "delivery_token": context.delivery_token,
                "messages": messages,
            }),
        })
    }

    pub(super) fn workflow_tool_context(
        &self,
        session_id: String,
        workflow_run_ref: String,
        workflow_node_run_id: String,
        delivery_token: Option<String>,
    ) -> Result<crate::transport::runtime_tools::WorkflowRuntimeToolContext, DaemonError> {
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(&session_id, &workflow_run_ref)?;
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&session_id, workflow_run.workflow_id())?;
        let node_id = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .map(|node_run| node_run.node_id().to_string())
            .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.clone(),
                workflow_id: workflow.id().to_string(),
                reference: workflow_node_run_id.clone(),
                message: "workflow node run was not found while resolving runtime tool scope",
            })?;
        let allowed_handoff_schema_refs = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .filter_map(|edge| edge.handoff_schema_ref().map(str::to_string))
            .collect();
        let node = workflow.node(&node_id);
        let can_complete_workflow_run = node.is_some_and(|node| node.can_complete_workflow_run());
        let can_emit_intermediate_workflow_run_output =
            node.is_some_and(|node| node.can_emit_intermediate_run_output());
        let workflow_intermediate_output_schema_ref = node
            .and_then(|node| node.intermediate_output_schema_ref())
            .map(str::to_string)
            .or_else(|| {
                workflow
                    .intermediate_output_schema_ref()
                    .map(str::to_string)
            });
        Ok(
            crate::transport::runtime_tools::WorkflowRuntimeToolContext {
                session_id,
                workflow_run_ref,
                workflow_node_run_id,
                delivery_token,
                allowed_handoff_schema_refs,
                workflow_run_output_schema_ref: workflow
                    .run_output_schema_ref()
                    .map(str::to_string),
                workflow_intermediate_output_schema_ref,
                can_complete_workflow_run,
                can_emit_intermediate_workflow_run_output,
            },
        )
    }

    pub(super) fn resolve_owned_authenticated_workflow_turn(
        &self,
        session_id: &str,
        candidate_agent_ids: &[String],
        delivery_token: Option<&str>,
    ) -> Result<(String, String), DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let agent_matches = |agent_id: &str| {
            candidate_agent_ids.is_empty()
                || candidate_agent_ids
                    .iter()
                    .any(|candidate| candidate == agent_id)
        };
        for agent_id in candidate_agent_ids {
            if let Some(prompt) = self
                .prompt_state_owner
                .active_prompt_for_agent(&session, agent_id)
            {
                let (Some(workflow_run_ref), Some(workflow_node_run_id)) =
                    (prompt.workflow_run_id(), prompt.workflow_node_run_id())
                else {
                    continue;
                };
                let matches_token = delivery_token.is_none_or(|requested| {
                    session
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
                        .is_some_and(|envelope| envelope.delivery_token() == requested)
                });
                if matches_token {
                    return Ok((
                        workflow_run_ref.to_string(),
                        workflow_node_run_id.to_string(),
                    ));
                }
            }
        }
        let mut running_turns = session
            .workflow_runs()
            .iter()
            .flat_map(|workflow_run| {
                workflow_run.node_runs().iter().filter_map(|node_run| {
                    let envelope = node_run.turn_envelope()?;
                    if node_run.status() != crate::session::WorkflowNodeRunStatus::Running
                        || !matches!(
                            envelope.state(),
                            crate::session::WorkflowTurnRuntimeState::Prepared
                                | crate::session::WorkflowTurnRuntimeState::Dispatched
                                | crate::session::WorkflowTurnRuntimeState::Acknowledged
                        )
                    {
                        return None;
                    }
                    if !agent_matches(node_run.agent_id()) {
                        return None;
                    }
                    if delivery_token
                        .is_some_and(|requested| envelope.delivery_token() != requested)
                    {
                        return None;
                    }
                    Some((workflow_run.id().to_string(), node_run.id().to_string()))
                })
            })
            .collect::<Vec<_>>();
        match running_turns.len() {
            1 => Ok(running_turns.remove(0)),
            0 => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "no active workflow turn for authenticated provider run".to_string(),
            }),
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_authenticated_runtime_tool_call",
                message: "multiple workflow turns matched the authenticated provider run"
                    .to_string(),
            }),
        }
    }
}
