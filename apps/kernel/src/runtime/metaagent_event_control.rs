use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::state::KernelRuntimeState;
use crate::transport::runtime_tools::{
    META_ACK_EVENT_TOOL, META_LIST_EVENTS_TOOL, META_READ_EVENT_TOOL, META_SEARCH_COMMANDS_TOOL,
    META_TURN_BLOB_TOOL, META_TURN_OVERVIEW_TOOL,
};

pub(crate) async fn execute_metaagent_event_request(
    runtime_state: &KernelRuntimeState,
    request: LocalDaemonRequest,
    caller_user_id: &str,
) -> Result<LocalDaemonResponse, DaemonError> {
    match request {
        LocalDaemonRequest::SearchMetaagentCommands(request) => {
            ensure_metaagent_owner(
                runtime_state,
                &request.session_id,
                &request.metaagent_id,
                caller_user_id,
            )
            .await?;
            let result = runtime_state
                .dispatch_meta_runtime_tool_call_for_agent(
                    &request.session_id,
                    &request.metaagent_id,
                    META_SEARCH_COMMANDS_TOOL,
                    serde_json::json!({
                        "query": request.query,
                        "tag": request.tag,
                        "scope": request.scope,
                        "mutates": request.mutates,
                        "policy": request.policy,
                        "limit": request.limit,
                    }),
                )
                .await?;
            Ok(LocalDaemonResponse::MetaagentCommandsSearched {
                commands: array_payload(result, "commands")?,
            })
        }
        LocalDaemonRequest::ListMetaagentEvents(request) => {
            ensure_metaagent_owner(
                runtime_state,
                &request.session_id,
                &request.metaagent_id,
                caller_user_id,
            )
            .await?;
            let result = runtime_state
                .dispatch_meta_runtime_tool_call_for_agent(
                    &request.session_id,
                    &request.metaagent_id,
                    META_LIST_EVENTS_TOOL,
                    serde_json::json!({
                        "limit": request.limit,
                        "status": request.status,
                        "kind": request.kind,
                    }),
                )
                .await?;
            Ok(LocalDaemonResponse::MetaagentEventsListed {
                events: array_payload(result, "events")?,
            })
        }
        LocalDaemonRequest::GetMetaagentTurnOverview(request) => {
            ensure_metaagent_owner(
                runtime_state,
                &request.session_id,
                &request.metaagent_id,
                caller_user_id,
            )
            .await?;
            let result = runtime_state
                .dispatch_meta_runtime_tool_call_for_agent(
                    &request.session_id,
                    &request.metaagent_id,
                    META_TURN_OVERVIEW_TOOL,
                    serde_json::json!({
                        "agent_ref": request.agent_ref,
                        "turn_ref": request.turn_ref,
                        "turns_back": request.turns_back,
                        "limit": request.limit,
                    }),
                )
                .await?;
            Ok(LocalDaemonResponse::MetaagentTurnOverview {
                overview: ok_payload(result)?,
            })
        }
        LocalDaemonRequest::GetMetaagentTurnBlob(request) => {
            ensure_metaagent_owner(
                runtime_state,
                &request.session_id,
                &request.metaagent_id,
                caller_user_id,
            )
            .await?;
            let result = runtime_state
                .dispatch_meta_runtime_tool_call_for_agent(
                    &request.session_id,
                    &request.metaagent_id,
                    META_TURN_BLOB_TOOL,
                    serde_json::json!({
                        "blob_id": request.blob_id,
                    }),
                )
                .await?;
            Ok(LocalDaemonResponse::MetaagentTurnBlob {
                blob: ok_payload(result)?,
            })
        }
        LocalDaemonRequest::ReadMetaagentEvent(request) => {
            ensure_metaagent_owner(
                runtime_state,
                &request.session_id,
                &request.metaagent_id,
                caller_user_id,
            )
            .await?;
            let result = runtime_state
                .dispatch_meta_runtime_tool_call_for_agent(
                    &request.session_id,
                    &request.metaagent_id,
                    META_READ_EVENT_TOOL,
                    serde_json::json!({
                        "event_id": request.event_id,
                    }),
                )
                .await?;
            Ok(LocalDaemonResponse::MetaagentEventRead {
                event: value_payload(result, "event")?,
            })
        }
        LocalDaemonRequest::AckMetaagentEvents(request) => {
            ensure_metaagent_owner(
                runtime_state,
                &request.session_id,
                &request.metaagent_id,
                caller_user_id,
            )
            .await?;
            let result = runtime_state
                .dispatch_meta_runtime_tool_call_for_agent(
                    &request.session_id,
                    &request.metaagent_id,
                    META_ACK_EVENT_TOOL,
                    serde_json::json!({
                        "event_id": request.event_id,
                        "event_ids": request.event_ids,
                        "up_to_sequence": request.up_to_sequence,
                    }),
                )
                .await?;
            Ok(LocalDaemonResponse::MetaagentEventsAcked {
                acked: array_payload(result, "acked")?,
            })
        }
        _ => Err(DaemonError::LocalTransport {
            operation: "metaagent event request",
            message: "unsupported metaagent event request".to_string(),
        }),
    }
}

async fn ensure_metaagent_owner(
    runtime_state: &KernelRuntimeState,
    session_id: &str,
    metaagent_id: &str,
    caller_user_id: &str,
) -> Result<(), DaemonError> {
    let session = runtime_state.session_snapshot(session_id).await?;
    let Some(agent) = runtime_state.list_agents().into_iter().find(|agent| {
        agent.id() == metaagent_id
            && agent.session_id() == session.id()
            && agent.is_metaagent()
            && agent.owner_user_id() == caller_user_id
    }) else {
        return Err(DaemonError::LocalTransport {
            operation: "metaagent event request",
            message: "metaagent event access requires an owned session metaagent".to_string(),
        });
    };
    if !session.has_member(caller_user_id) {
        return Err(DaemonError::LocalTransport {
            operation: "metaagent event request",
            message: "metaagent event access requires session membership".to_string(),
        });
    }
    if agent.session_id() != session.id() {
        return Err(DaemonError::LocalTransport {
            operation: "metaagent event request",
            message: "metaagent event access requires an owned session metaagent".to_string(),
        });
    }
    Ok(())
}

fn array_payload(
    result: crate::transport::runtime_tools::RuntimeToolResult,
    field: &str,
) -> Result<Vec<serde_json::Value>, DaemonError> {
    let payload = ok_payload(result)?;
    payload
        .get(field)
        .and_then(serde_json::Value::as_array)
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "metaagent event request",
            message: format!("metaagent event response missing `{field}` array"),
        })
}

fn value_payload(
    result: crate::transport::runtime_tools::RuntimeToolResult,
    field: &str,
) -> Result<serde_json::Value, DaemonError> {
    let payload = ok_payload(result)?;
    payload
        .get(field)
        .cloned()
        .ok_or_else(|| DaemonError::LocalTransport {
            operation: "metaagent event request",
            message: format!("metaagent event response missing `{field}`"),
        })
}

fn ok_payload(
    result: crate::transport::runtime_tools::RuntimeToolResult,
) -> Result<serde_json::Value, DaemonError> {
    if result.ok {
        return Ok(result.payload);
    }
    Err(DaemonError::LocalTransport {
        operation: "metaagent event request",
        message: result
            .payload
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("metaagent event request failed")
            .to_string(),
    })
}
