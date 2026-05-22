use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::DaemonError;
use crate::local::{
    LocalDaemonRequest, RelayStatus, RemoteMachineRecord, WorkflowDesignOpForwarded,
};
use crate::provider::RuntimeProviderRun;
use crate::runtime::projection::AgentRuntimeActivity;
use crate::session::{RuntimeSession, WorkflowRun};
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};

pub(crate) const WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE: &str = "waiting_room_inventory";
pub(crate) const WAITING_ROOM_INVENTORY_SENTINEL_ID: &str = "__waiting_room_inventory__";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum KernelIncomingFrame {
    Request {
        request_id: String,
        #[serde(default)]
        command_id: Option<String>,
        #[serde(default)]
        causation_id: Option<String>,
        #[serde(default)]
        correlation_id: Option<String>,
        request: LocalDaemonRequest,
    },
    Subscribe {
        request_id: String,
        session_id: String,
        attachment_id: String,
        #[serde(default)]
        subscription_scope: Option<String>,
        #[serde(default)]
        resume_from_event_id: Option<u64>,
    },
    Unsubscribe {
        request_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KernelTransportError {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum KernelOutgoingFrame {
    Response {
        request_id: String,
        response: Box<Option<Value>>,
        error: Option<KernelTransportError>,
    },
    Event {
        event_id: u64,
        event: Box<KernelEvent>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum KernelEvent {
    TerminalOutput {
        records: Vec<TerminalOutputRecord>,
    },
    RuntimeNotices {
        notices: Vec<RuntimeNoticeRecord>,
    },
    AssistantMessageCompleted {
        session_id: String,
        provider_run_id: String,
        agent_id: Option<String>,
        recipient_attachment_ids: Vec<String>,
        message_id: String,
        completed_at_ms: u64,
    },
    SessionSnapshot {
        session: Box<RuntimeSession>,
        provider_run: Box<Option<RuntimeProviderRun>>,
        agent_activity: Box<BTreeMap<String, AgentRuntimeActivity>>,
    },
    SessionUnavailable {
        session_id: String,
        message: String,
    },
    RelayStatusChanged {
        status: RelayStatus,
    },
    RemoteMachinesChanged {
        machines: Vec<RemoteMachineRecord>,
    },
    WaitingRoomInventoryChanged {
        inventory_version: String,
    },
    WorkflowDesignOp {
        design_op: WorkflowDesignOpForwarded,
    },
    WorkflowRunUpdated {
        session_id: String,
        workflow_run: WorkflowRun,
    },
    Heartbeat {
        session_id: String,
    },
    TransportResumed {
        session_id: String,
        resumed_from_event_id: Option<u64>,
    },
    ReplayGap {
        session_id: String,
        requested_from_event_id: u64,
        first_retained_event_id: Option<u64>,
        latest_event_id: Option<u64>,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KernelSubscriptionScope {
    Session,
    WaitingRoomInventory,
}

pub(crate) fn kernel_subscription_scope(scope: Option<&str>) -> KernelSubscriptionScope {
    if scope == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE) {
        KernelSubscriptionScope::WaitingRoomInventory
    } else {
        KernelSubscriptionScope::Session
    }
}

pub(crate) fn kernel_event_trace_payload(event_id: u64, event: &KernelEvent) -> Value {
    match event {
        KernelEvent::TerminalOutput { records } => serde_json::json!({
            "event_id": event_id,
            "event": "terminal_output",
            "record_count": records.len(),
            "records": records.iter().map(|record| {
                serde_json::json!({
                    "kind": &record.kind,
                    "provider_run_id": &record.provider_run_id,
                    "agent_id": &record.agent_id,
                    "merge_key": &record.merge_key,
                    "byte_len": record.bytes.len(),
                })
            }).collect::<Vec<_>>(),
        }),
        KernelEvent::SessionSnapshot {
            session,
            provider_run,
            agent_activity,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "session_snapshot",
            "active_prompt": session.active_prompt().map(|prompt| serde_json::json!({
                "id": prompt.id(),
                "status": prompt.status(),
                "target_agent_id": prompt.target_agent_id(),
                "workflow_run_id": prompt.workflow_run_id(),
                "workflow_node_run_id": prompt.workflow_node_run_id(),
            })),
            "active_provider_run_id": session.active_provider_run_id(),
            "provider_run_id": provider_run.as_ref().as_ref().map(|run| run.id()),
            "agent_activity": agent_activity,
        }),
        KernelEvent::AssistantMessageCompleted {
            session_id,
            provider_run_id,
            agent_id,
            message_id,
            completed_at_ms,
            ..
        } => serde_json::json!({
            "event_id": event_id,
            "event": "assistant_message_completed",
            "session_id": session_id,
            "provider_run_id": provider_run_id,
            "agent_id": agent_id,
            "message_id": message_id,
            "completed_at_ms": completed_at_ms,
        }),
        KernelEvent::RuntimeNotices { notices } => serde_json::json!({
            "event_id": event_id,
            "event": "runtime_notices",
            "notice_count": notices.len(),
            "notices": notices.iter().map(|notice| {
                serde_json::json!({
                    "provider_run_id": &notice.provider_run_id,
                    "agent_id": &notice.agent_id,
                    "message_len": notice.message.len(),
                })
            }).collect::<Vec<_>>(),
        }),
        KernelEvent::WorkflowRunUpdated {
            session_id,
            workflow_run,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "workflow_run_updated",
            "session_id": session_id,
            "workflow_run_id": workflow_run.id(),
            "workflow_id": workflow_run.workflow_id(),
            "status": workflow_run.status(),
            "active_node_run_id": workflow_run.active_node_run_id(),
        }),
        other => serde_json::json!({
            "event_id": event_id,
            "event": kernel_event_name(other),
        }),
    }
}

pub(crate) fn kernel_event_name(event: &KernelEvent) -> &'static str {
    match event {
        KernelEvent::TerminalOutput { .. } => "terminal_output",
        KernelEvent::RuntimeNotices { .. } => "runtime_notices",
        KernelEvent::AssistantMessageCompleted { .. } => "assistant_message_completed",
        KernelEvent::SessionSnapshot { .. } => "session_snapshot",
        KernelEvent::SessionUnavailable { .. } => "session_unavailable",
        KernelEvent::RelayStatusChanged { .. } => "relay_status_changed",
        KernelEvent::RemoteMachinesChanged { .. } => "remote_machines_changed",
        KernelEvent::WaitingRoomInventoryChanged { .. } => "waiting_room_inventory_changed",
        KernelEvent::WorkflowDesignOp { .. } => "workflow_design_op",
        KernelEvent::WorkflowRunUpdated { .. } => "workflow_run_updated",
        KernelEvent::Heartbeat { .. } => "heartbeat",
        KernelEvent::TransportResumed { .. } => "transport_resumed",
        KernelEvent::ReplayGap { .. } => "replay_gap",
    }
}

pub(crate) fn event_session_id(event: &KernelEvent) -> Option<&str> {
    match event {
        KernelEvent::TerminalOutput { records } => {
            records.first().map(|record| record.session_id.as_str())
        }
        KernelEvent::RuntimeNotices { notices } => {
            notices.first().map(|notice| notice.session_id.as_str())
        }
        KernelEvent::AssistantMessageCompleted { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::SessionSnapshot { session, .. } => Some(session.id()),
        KernelEvent::SessionUnavailable { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::RelayStatusChanged { .. } => None,
        KernelEvent::RemoteMachinesChanged { .. } => None,
        KernelEvent::WaitingRoomInventoryChanged { .. } => None,
        KernelEvent::WorkflowDesignOp { design_op } => Some(design_op.session_id.as_str()),
        KernelEvent::WorkflowRunUpdated { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::Heartbeat { session_id } => Some(session_id.as_str()),
        KernelEvent::TransportResumed { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::ReplayGap { session_id, .. } => Some(session_id.as_str()),
    }
}

pub(crate) fn event_stream_id_for_event(
    event: &KernelEvent,
    fallback_session_id: Option<&str>,
) -> Option<String> {
    event_session_id(event)
        .or(fallback_session_id)
        .map(session_stream_id)
        .or_else(|| Some("daemon".to_string()))
}

fn session_stream_id(session_id: &str) -> String {
    format!("session:{session_id}")
}

pub(crate) fn subscription_event_stream_id(session_id: &str, attachment_id: &str) -> String {
    format!("session:{session_id}:attachment:{attachment_id}")
}

pub(crate) fn event_is_relevant_to_attachment(event: &KernelEvent, attachment_id: &str) -> bool {
    match event {
        KernelEvent::TerminalOutput { records } => records.iter().any(|record| {
            record
                .recipient_attachment_ids
                .iter()
                .any(|id| id == attachment_id)
        }),
        KernelEvent::RuntimeNotices { notices } => notices.iter().any(|notice| {
            notice.recipient_attachment_ids.is_empty()
                || notice
                    .recipient_attachment_ids
                    .iter()
                    .any(|id| id == attachment_id)
        }),
        KernelEvent::AssistantMessageCompleted {
            recipient_attachment_ids,
            ..
        } => recipient_attachment_ids
            .iter()
            .any(|id| id == attachment_id),
        KernelEvent::SessionSnapshot { .. }
        | KernelEvent::SessionUnavailable { .. }
        | KernelEvent::RelayStatusChanged { .. }
        | KernelEvent::RemoteMachinesChanged { .. }
        | KernelEvent::WaitingRoomInventoryChanged { .. }
        | KernelEvent::WorkflowDesignOp { .. }
        | KernelEvent::WorkflowRunUpdated { .. }
        | KernelEvent::Heartbeat { .. }
        | KernelEvent::TransportResumed { .. }
        | KernelEvent::ReplayGap { .. } => true,
    }
}

pub(crate) fn map_kernel_error(error: &DaemonError) -> KernelTransportError {
    match error {
        DaemonError::SessionNotFound { .. } => kernel_error("session_not_found", error, false),
        DaemonError::AttachmentNotFound { .. } => {
            kernel_error("attachment_not_found", error, false)
        }
        DaemonError::AttachmentNotInSession { .. } => {
            kernel_error("attachment_not_in_session", error, false)
        }
        DaemonError::NoActiveProviderRun { .. } => {
            kernel_error("no_active_provider_run", error, false)
        }
        DaemonError::ProviderRunNotFound { .. } => {
            kernel_error("provider_run_not_found", error, false)
        }
        DaemonError::WorkspaceClaimConflict { .. } => {
            kernel_error("workspace_claim_conflict", error, true)
        }
        DaemonError::ProviderAdapterNotFound { .. } => {
            kernel_error("provider_adapter_not_found", error, false)
        }
        DaemonError::ProviderProtocol { .. } => {
            kernel_error("provider_protocol_error", error, true)
        }
        DaemonError::LocalTransport { .. } => kernel_error("local_transport_error", error, true),
        DaemonError::PtySpawn { .. } => kernel_error("pty_spawn_failed", error, true),
        DaemonError::PtyCleanup { .. } => kernel_error("pty_cleanup_failed", error, true),
        DaemonError::PtyWrite { .. } => kernel_error("pty_write_failed", error, true),
        DaemonError::PtyResize { .. } => kernel_error("pty_resize_failed", error, true),
        _ => kernel_error("kernel_request_failed", error, false),
    }
}

fn kernel_error(code: &str, error: &DaemonError, retryable: bool) -> KernelTransportError {
    KernelTransportError {
        code: code.to_string(),
        message: error.to_string(),
        retryable,
    }
}

pub(crate) fn serialize_frame(frame: &KernelOutgoingFrame) -> Result<String, DaemonError> {
    serde_json::to_string(frame).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize kernel websocket frame",
        message: error.to_string(),
    })
}
