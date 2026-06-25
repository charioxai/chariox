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
            crate::transport::runtime_tools::AGENT_APP_ACTION_TOOL => {
                self.workflow_agent_app_action_tool_result(&arguments, &context)
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

    fn workflow_agent_app_action_tool_result(
        &self,
        arguments: &serde_json::Value,
        context: &crate::transport::runtime_tools::WorkflowRuntimeToolContext,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<crate::transport::runtime_tools::AgentAppActionArgs>(
            arguments.clone(),
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_agent_app_action",
            message: format!("invalid tool arguments: {error}"),
        })?;
        let (action, action_context) = {
            let workflow_run = self
                .session_store
                .read()
                .resolve_workflow_run_ref(&context.session_id, &context.workflow_run_ref)?;
            let invocation = workflow_run.publication_invocation().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "runtime_tool_agent_app_action",
                    message: "current workflow run was not started by a publication invocation"
                        .to_string(),
                }
            })?;
            let proof = invocation
                .caller
                .get("proof")
                .and_then(serde_json::Value::as_object);
            let audit = proof
                .and_then(|value| value.get("agent_app_audit"))
                .and_then(serde_json::Value::as_object)
                .and_then(|value| {
                    let url = value.get("url").and_then(serde_json::Value::as_str)?;
                    let token = value.get("token").and_then(serde_json::Value::as_str)?;
                    if url.trim().is_empty() || token.trim().is_empty() {
                        return None;
                    }
                    Some(AgentAppActionAuditContext {
                        url: url.to_string(),
                        token: token.to_string(),
                    })
                });
            let action_context = AgentAppHttpActionContext {
                action_id: args.action_id.clone(),
                session: proof
                    .and_then(|value| value.get("agent_app_session"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                invocation_request_id: proof
                    .and_then(|value| value.get("agent_app_request_id"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                audit,
            };
            let action = invocation
                .caller
                .get("proof")
                .and_then(|value| value.get("agent_app_actions"))
                .and_then(serde_json::Value::as_object)
                .and_then(|actions| actions.get(&args.action_id))
                .cloned()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "runtime_tool_agent_app_action",
                    message: format!(
                        "agent app action `{}` is not allowed for this invocation",
                        args.action_id
                    ),
                })?;
            (action, action_context)
        };
        if let Some(schema) = action.get("input_schema") {
            let compiled = jsonschema::JSONSchema::compile(schema).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "runtime_tool_agent_app_action",
                    message: format!(
                        "agent app action `{}` has invalid input schema: {error}",
                        args.action_id
                    ),
                }
            })?;
            let messages = match compiled.validate(&args.input) {
                Ok(()) => Vec::new(),
                Err(errors) => errors.map(|error| error.to_string()).collect::<Vec<_>>(),
            };
            if !messages.is_empty() {
                send_agent_app_action_audit(
                    &action_context,
                    AgentAppActionAuditOutcome {
                        ok: false,
                        http_status: None,
                        duration_ms: None,
                        error: Some("input validation failed".to_string()),
                    },
                );
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({
                        "action_id": args.action_id,
                        "valid": false,
                        "errors": messages,
                    }),
                });
            }
        }
        let transport = action
            .get("transport")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_agent_app_action",
                message: format!("agent app action `{}` has no transport", args.action_id),
            })?;
        if transport
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("http")
            != "http"
        {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_agent_app_action",
                message: format!(
                    "agent app action `{}` uses unsupported transport",
                    args.action_id
                ),
            });
        }
        let url = transport
            .get("url")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "runtime_tool_agent_app_action",
                message: format!("agent app action `{}` has no URL", args.action_id),
            })?;
        let method = transport
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("POST")
            .to_ascii_uppercase();
        let allow_external = transport
            .get("allow_external")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let timeout_ms = transport
            .get("timeout_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(30_000)
            .clamp(1_000, 60_000);
        let max_response_bytes = transport
            .get("max_response_bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1_048_576)
            .clamp(1_024, 8_388_608);
        let started = std::time::Instant::now();
        let response = match call_agent_app_http_action(
            url,
            &method,
            &args.input,
            &action_context,
            AgentAppHttpActionOptions {
                allow_external,
                timeout_ms,
                max_response_bytes,
            },
        ) {
            Ok(response) => {
                send_agent_app_action_audit(
                    &action_context,
                    AgentAppActionAuditOutcome {
                        ok: (200..300).contains(&response.status),
                        http_status: Some(response.status),
                        duration_ms: Some(started.elapsed().as_millis() as u64),
                        error: None,
                    },
                );
                response
            }
            Err(error) => {
                send_agent_app_action_audit(
                    &action_context,
                    AgentAppActionAuditOutcome {
                        ok: false,
                        http_status: None,
                        duration_ms: Some(started.elapsed().as_millis() as u64),
                        error: Some(error.clone()),
                    },
                );
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_agent_app_action",
                    message: format!("agent app action `{}` failed: {error}", args.action_id),
                });
            }
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: (200..300).contains(&response.status),
            payload: serde_json::json!({
                "action_id": args.action_id,
                "status": response.status,
                "content_type": response.content_type,
                "body": response.body,
            }),
        })
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
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(&context.session_id, workflow_run.workflow_id())?;
        let outgoing_edges = workflow
            .edges()
            .iter()
            .filter(|edge| edge.from_node_id() == node_id)
            .map(|edge| {
                let target_node = workflow.node(edge.to_node_id());
                let target_instructions = target_node
                    .and_then(|node| node.instructions())
                    .map(compact_workflow_edge_contract_instructions)
                    .filter(|instructions| !instructions.is_empty());
                serde_json::json!({
                    "edge_id": edge.id(),
                    "from_node_id": edge.from_node_id(),
                    "to_node_id": edge.to_node_id(),
                    "to_node_public_label": target_node.map(|node| node.public_label()),
                    "target_instructions": target_instructions,
                    "to_agent_id": target_node.map(|node| node.agent_id()),
                    "handoff_schema_ref": edge.handoff_schema_ref(),
                    "validation_policy": edge.validation_policy(),
                })
            })
            .collect::<Vec<_>>();
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
                "outgoing_edges": outgoing_edges,
                "handoff_routing": {
                    "final_json_field": "workflow_handoffs",
                    "select_by": ["edge_id", "to_node_id"],
                    "message_fields": ["output.message", "message"],
                },
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

#[derive(Debug)]
struct AgentAppHttpActionResponse {
    status: u16,
    content_type: String,
    body: serde_json::Value,
}

struct AgentAppHttpActionContext {
    action_id: String,
    session: Option<String>,
    invocation_request_id: Option<String>,
    audit: Option<AgentAppActionAuditContext>,
}

struct AgentAppHttpActionOptions {
    allow_external: bool,
    timeout_ms: u64,
    max_response_bytes: u64,
}

struct AgentAppActionAuditContext {
    url: String,
    token: String,
}

struct AgentAppActionAuditOutcome {
    ok: bool,
    http_status: Option<u16>,
    duration_ms: Option<u64>,
    error: Option<String>,
}

fn call_agent_app_http_action(
    url: &str,
    method: &str,
    input: &serde_json::Value,
    context: &AgentAppHttpActionContext,
    options: AgentAppHttpActionOptions,
) -> Result<AgentAppHttpActionResponse, String> {
    validate_agent_app_action_url(url, options.allow_external)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_millis(options.timeout_ms))
        .build();
    let input_json = serde_json::to_string(input).map_err(|error| error.to_string())?;
    let mut request = match method {
        "GET" => agent.get(url).query("input", &input_json),
        "POST" => agent
            .post(url)
            .set("content-type", "application/json")
            .set("accept", "application/json"),
        other => return Err(format!("unsupported HTTP method `{other}`")),
    };
    request = request.set("x-arroba-agent-app-action-id", &context.action_id);
    if let Some(session) = context.session.as_deref() {
        request = request.set("x-arroba-agent-app-session", session);
    }
    if let Some(invocation_request_id) = context.invocation_request_id.as_deref() {
        request = request.set("x-arroba-publication-invocation", invocation_request_id);
    }
    let response = match method {
        "GET" => request.call(),
        "POST" => request.send_string(&input_json),
        _ => unreachable!(),
    };
    let response = match response {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(error) => return Err(error.to_string()),
    };
    let status = response.status();
    let content_type = response.content_type().to_string();
    let text = read_limited_response_body(response, options.max_response_bytes)?;
    let body = if content_type.contains("application/json") {
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::Value::String(text))
    } else {
        serde_json::Value::String(text)
    };
    Ok(AgentAppHttpActionResponse {
        status,
        content_type,
        body,
    })
}

fn validate_agent_app_action_url(url: &str, allow_external: bool) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|error| format!("invalid action URL: {error}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported action URL scheme `{other}`")),
    }
    if allow_external {
        return Ok(());
    }
    let Some(host) = parsed.host_str() else {
        return Err("action URL is missing a host".to_string());
    };
    if host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.") {
        return Ok(());
    }
    Err("external action URLs require transport.allow_external=true".to_string())
}

fn read_limited_response_body(
    response: ureq::Response,
    max_response_bytes: u64,
) -> Result<String, String> {
    use std::io::Read as _;

    let mut reader = response
        .into_reader()
        .take(max_response_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max_response_bytes {
        return Err(format!(
            "agent app action response exceeded {max_response_bytes} bytes"
        ));
    }
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn send_agent_app_action_audit(
    context: &AgentAppHttpActionContext,
    outcome: AgentAppActionAuditOutcome,
) {
    let Some(audit) = context.audit.as_ref() else {
        return;
    };
    let message = if outcome.ok {
        format!("agent app action `{}` completed", context.action_id)
    } else {
        format!("agent app action `{}` failed", context.action_id)
    };
    let payload = serde_json::json!({
        "token": audit.token,
        "entries": [{
            "level": if outcome.ok { "info" } else { "warn" },
            "message": message,
            "metadata": {
                "kind": "agent_app_action",
                "action_id": context.action_id.as_str(),
                "session": context.session.as_deref(),
                "invocation_request_id": context.invocation_request_id.as_deref(),
                "ok": outcome.ok,
                "http_status": outcome.http_status,
                "duration_ms": outcome.duration_ms,
                "error": outcome.error,
            }
        }]
    });
    let Ok(payload_json) = serde_json::to_string(&payload) else {
        return;
    };
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let _ = agent
        .post(&audit.url)
        .set("content-type", "application/json")
        .set("accept", "application/json")
        .send_string(&payload_json);
}

fn compact_workflow_edge_contract_instructions(instructions: &str) -> String {
    const MAX_CHARS: usize = 240;
    let compact = instructions
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() <= MAX_CHARS {
        return compact;
    }
    let truncated = compact.chars().take(MAX_CHARS).collect::<String>();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
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
        let history_store = app.history_store();
        let operational_history_store = app.operational_history_store();
        let durable_state_store = app.durable_state_store();
        let history_projection = app.session_history_projection_store();
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
            history_store,
            operational_history_store,
            durable_state_store,
            history_projection,
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
    fn workflow_turn_context_lists_outgoing_edge_options() {
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
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("worker-a"),
            )
            .expect("worker a should spawn");
        let worker_b = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("worker-b"),
            )
            .expect("worker b should spawn");

        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("routing".to_string()))
            .expect("workflow should be created");
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
                workflow_run_id,
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
        assert_eq!(
            option_a["target_instructions"],
            "Handle policy-sensitive routing tasks only."
        );
        assert_eq!(option_a["to_agent_id"], worker_a.id());
        assert_eq!(option_a["handoff_schema_ref"], "/schemas/route-a.json");
        assert_eq!(option_a["validation_policy"], "halt");
        let option_b = outgoing
            .iter()
            .find(|edge| edge["edge_id"] == edge_b.id())
            .expect("edge b should be present");
        assert_eq!(option_b["to_node_id"], node_b.id());
        assert_eq!(option_b["to_node_public_label"], "Worker B");
        assert_eq!(
            option_b["target_instructions"],
            "Handle quality and completeness routing tasks only."
        );
        assert_eq!(option_b["to_agent_id"], worker_b.id());
        assert_eq!(option_b["handoff_schema_ref"], "/schemas/route-b.json");
        assert_eq!(option_b["validation_policy"], "warn");
        assert_eq!(
            result.payload["handoff_routing"]["final_json_field"],
            "workflow_handoffs"
        );
        assert!(result.payload["handoff_routing"]["select_by"]
            .as_array()
            .expect("select_by should be an array")
            .iter()
            .any(|value| value == "edge_id"));
        assert_eq!(result.payload["workflow_node_run_id"], node_run_id);
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
        assert!(captured.contains("x-arroba-agent-app-action-id: cart.add"));
        assert!(captured.contains("x-arroba-agent-app-session: session-a"));
        assert!(captured.contains("x-arroba-publication-invocation: invocation-a"));
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
                    url: format!("http://{address}/.well-known/arroba/agent-app/audit-log"),
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
        assert!(captured.contains("POST /.well-known/arroba/agent-app/audit-log"));
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
}
