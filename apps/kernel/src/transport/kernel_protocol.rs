use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::WorkspaceLiveSyncMode;
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonRequest, RelayStatus, RemoteMachineRecord, WaitingRoomLaunchTarget,
    WaitingRoomPublicSessionSummary, WaitingRoomPublicSnapshot, WorkflowDesignOpForwarded,
};
use crate::provider::{OpenCodeProviderCatalog, RuntimeProviderRun};
use crate::runtime::projection::{AgentRuntimeActivity, SessionSnapshotProjection};
use crate::session::{RuntimeInteraction, RuntimeSession, WorkflowRun};
use crate::slice::SliceRecord;
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};

pub(crate) const WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE: &str = "waiting_room_inventory";
pub(crate) const WAITING_ROOM_INVENTORY_SENTINEL_ID: &str = "__waiting_room_inventory__";
pub(crate) const MAX_TERMINAL_OUTPUT_EVENT_JSON_BYTES: usize = 128 * 1024;

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
        #[serde(default)]
        agent_activity_revision: u64,
    },
    AgentActivityChanged {
        session_id: String,
        agent_activity: Box<BTreeMap<String, AgentRuntimeActivity>>,
        #[serde(default)]
        agent_activity_revision: u64,
    },
    ProviderRunChanged {
        session_id: String,
        provider_run: Option<RuntimeProviderRun>,
    },
    SessionMetadataChanged {
        session_id: String,
        metadata: SessionMetadataPatch,
    },
    RuntimeInteractionsChanged {
        session_id: String,
        active_interactions: Vec<RuntimeInteraction>,
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
    WaitingRoomRowsChanged {
        inventory_version: String,
        structural_version: String,
        activity_revision: String,
        schema_version: u32,
        generated_at_ms: u64,
        launch_target: Option<WaitingRoomLaunchTarget>,
        projects: Vec<crate::local::WaitingRoomPublicProjectSummary>,
        removed_project_ids: Vec<String>,
        sessions: Vec<WaitingRoomPublicSessionSummary>,
        removed_session_ids: Vec<String>,
    },
    ProviderCatalogChanged {
        generated_at_ms: u64,
        catalog: OpenCodeProviderCatalog,
    },
    SlicesChanged {
        generated_at_ms: u64,
        slices: Vec<SliceRecord>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionMetadataPatch {
    pub(crate) alias: Option<String>,
    pub(crate) last_used_at_ms: Option<u64>,
    pub(crate) last_prompt_sent_at_ms: Option<u64>,
    pub(crate) hidden: bool,
    pub(crate) focused_agent_id: Option<String>,
    pub(crate) workspace_live_sync_mode: Option<WorkspaceLiveSyncMode>,
}

impl SessionMetadataPatch {
    fn from_session(session: &RuntimeSession) -> Self {
        Self {
            alias: session.alias().map(str::to_string),
            last_used_at_ms: session.last_used_at_ms(),
            last_prompt_sent_at_ms: session.last_prompt_sent_at_ms(),
            hidden: session.is_hidden(),
            focused_agent_id: session.focused_agent_id().map(str::to_string),
            workspace_live_sync_mode: session.workspace_live_sync_mode(),
        }
    }
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
            agent_activity_revision,
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
            "agent_activity_revision": agent_activity_revision,
        }),
        KernelEvent::AgentActivityChanged {
            session_id,
            agent_activity,
            agent_activity_revision,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "agent_activity_changed",
            "session_id": session_id,
            "agent_activity": agent_activity,
            "agent_activity_revision": agent_activity_revision,
        }),
        KernelEvent::ProviderRunChanged {
            session_id,
            provider_run,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "provider_run_changed",
            "session_id": session_id,
            "provider_run_id": provider_run.as_ref().map(|run| run.id()),
            "agent_id": provider_run.as_ref().and_then(|run| run.agent_instance_id()),
            "state": provider_run.as_ref().map(|run| run.state()),
        }),
        KernelEvent::SessionMetadataChanged {
            session_id,
            metadata,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "session_metadata_changed",
            "session_id": session_id,
            "alias": metadata.alias,
            "focused_agent_id": metadata.focused_agent_id,
            "hidden": metadata.hidden,
            "workspace_live_sync_mode": metadata.workspace_live_sync_mode,
        }),
        KernelEvent::RuntimeInteractionsChanged {
            session_id,
            active_interactions,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "runtime_interactions_changed",
            "session_id": session_id,
            "active_interaction_count": active_interactions.len(),
            "agent_ids": active_interactions.iter().map(|interaction| interaction.agent_id()).collect::<Vec<_>>(),
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
        KernelEvent::ProviderCatalogChanged {
            generated_at_ms,
            catalog,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "provider_catalog_changed",
            "generated_at_ms": generated_at_ms,
            "provider_count": catalog.all.len(),
            "connected_provider_count": catalog.connected.len(),
        }),
        KernelEvent::SlicesChanged {
            generated_at_ms,
            slices,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "slices_changed",
            "generated_at_ms": generated_at_ms,
            "slice_count": slices.len(),
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

pub(crate) fn waiting_room_rows_changed_event(
    snapshot: WaitingRoomPublicSnapshot,
    previous_snapshot: Option<&WaitingRoomPublicSnapshot>,
) -> Option<KernelEvent> {
    if previous_snapshot.is_some_and(|previous| {
        previous.schema_version == snapshot.schema_version
            && previous.launch_target == snapshot.launch_target
            && previous.projects == snapshot.projects
            && previous.sessions == snapshot.sessions
    }) {
        return None;
    }
    let previous_sessions = previous_snapshot
        .map(|previous| {
            previous
                .sessions
                .iter()
                .map(|session| (session.id.as_str(), session))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let previous_projects = previous_snapshot
        .map(|previous| {
            previous
                .projects
                .iter()
                .map(|project| (project.id.as_str(), project))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let current_project_ids = snapshot
        .projects
        .iter()
        .map(|project| project.id.as_str())
        .collect::<BTreeSet<_>>();
    let projects = snapshot
        .projects
        .iter()
        .filter(|project| previous_projects.get(project.id.as_str()).copied() != Some(*project))
        .cloned()
        .collect::<Vec<_>>();
    let removed_project_ids = previous_snapshot
        .map(|previous| {
            previous
                .projects
                .iter()
                .filter(|project| !current_project_ids.contains(project.id.as_str()))
                .map(|project| project.id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let current_session_ids = snapshot
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect::<BTreeSet<_>>();
    let sessions = snapshot
        .sessions
        .iter()
        .filter(|session| previous_sessions.get(session.id.as_str()).copied() != Some(*session))
        .cloned()
        .collect::<Vec<_>>();
    let removed_session_ids = previous_snapshot
        .map(|previous| {
            previous
                .sessions
                .iter()
                .filter(|session| !current_session_ids.contains(session.id.as_str()))
                .map(|session| session.id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(KernelEvent::WaitingRoomRowsChanged {
        inventory_version: snapshot.inventory_version,
        structural_version: snapshot.structural_version,
        activity_revision: snapshot.activity_revision,
        schema_version: snapshot.schema_version,
        generated_at_ms: snapshot.generated_at_ms,
        launch_target: snapshot.launch_target,
        projects,
        removed_project_ids,
        sessions,
        removed_session_ids,
    })
}

pub(crate) fn agent_activity_changed_event(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
) -> Option<KernelEvent> {
    let previous_snapshot = previous_snapshot?;
    let session_allows_activity_delta = previous_snapshot.session == snapshot.session
        || previous_snapshot
            .session
            .equivalent_except_prompt_runtime(&snapshot.session);
    if !session_allows_activity_delta
        || previous_snapshot.provider_run != snapshot.provider_run
        || previous_snapshot.agent_activity == snapshot.agent_activity
    {
        return None;
    }
    Some(KernelEvent::AgentActivityChanged {
        session_id: snapshot.session.id().to_string(),
        agent_activity: Box::new(snapshot.agent_activity.clone()),
        agent_activity_revision: snapshot.metadata.last_event_id,
    })
}

pub(crate) fn provider_run_changed_event(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
) -> Option<KernelEvent> {
    let previous_snapshot = previous_snapshot?;
    if previous_snapshot.session != snapshot.session
        || previous_snapshot.agent_activity != snapshot.agent_activity
        || previous_snapshot.provider_run == snapshot.provider_run
    {
        return None;
    }
    Some(KernelEvent::ProviderRunChanged {
        session_id: snapshot.session.id().to_string(),
        provider_run: snapshot.provider_run.clone(),
    })
}

pub(crate) fn session_metadata_changed_event(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
) -> Option<KernelEvent> {
    let previous_snapshot = previous_snapshot?;
    if previous_snapshot.provider_run != snapshot.provider_run
        || previous_snapshot.agent_activity != snapshot.agent_activity
        || !previous_snapshot
            .session
            .equivalent_except_session_metadata(&snapshot.session)
    {
        return None;
    }
    let metadata = SessionMetadataPatch::from_session(&snapshot.session);
    if metadata == SessionMetadataPatch::from_session(&previous_snapshot.session) {
        return None;
    }
    Some(KernelEvent::SessionMetadataChanged {
        session_id: snapshot.session.id().to_string(),
        metadata,
    })
}

pub(crate) fn runtime_interactions_changed_event(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
) -> Option<KernelEvent> {
    let previous_snapshot = previous_snapshot?;
    if previous_snapshot.session.active_interactions() == snapshot.session.active_interactions() {
        return None;
    }
    Some(KernelEvent::RuntimeInteractionsChanged {
        session_id: snapshot.session.id().to_string(),
        active_interactions: snapshot.session.active_interactions().to_vec(),
    })
}

pub(crate) fn workflow_run_updated_events(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
) -> Vec<KernelEvent> {
    changed_workflow_runs(snapshot, previous_snapshot)
        .into_iter()
        .map(|workflow_run| KernelEvent::WorkflowRunUpdated {
            session_id: snapshot.session.id().to_string(),
            workflow_run,
        })
        .collect()
}

pub(crate) fn workflow_run_only_changed(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
) -> bool {
    let Some(previous_snapshot) = previous_snapshot else {
        return false;
    };
    previous_snapshot.provider_run == snapshot.provider_run
        && previous_snapshot.agent_activity == snapshot.agent_activity
        && previous_snapshot
            .session
            .equivalent_except_workflow_runs(&snapshot.session)
        && previous_snapshot.session.workflow_runs() != snapshot.session.workflow_runs()
}

fn changed_workflow_runs(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
) -> Vec<WorkflowRun> {
    let Some(previous_snapshot) = previous_snapshot else {
        return snapshot.session.workflow_runs().to_vec();
    };
    let previous_runs = previous_snapshot
        .session
        .workflow_runs()
        .iter()
        .map(|run| (run.id(), run))
        .collect::<BTreeMap<_, _>>();
    snapshot
        .session
        .workflow_runs()
        .iter()
        .filter(|run| previous_runs.get(run.id()).copied() != Some(*run))
        .cloned()
        .collect()
}

pub(crate) fn kernel_event_name(event: &KernelEvent) -> &'static str {
    match event {
        KernelEvent::TerminalOutput { .. } => "terminal_output",
        KernelEvent::RuntimeNotices { .. } => "runtime_notices",
        KernelEvent::AssistantMessageCompleted { .. } => "assistant_message_completed",
        KernelEvent::SessionSnapshot { .. } => "session_snapshot",
        KernelEvent::AgentActivityChanged { .. } => "agent_activity_changed",
        KernelEvent::ProviderRunChanged { .. } => "provider_run_changed",
        KernelEvent::SessionMetadataChanged { .. } => "session_metadata_changed",
        KernelEvent::RuntimeInteractionsChanged { .. } => "runtime_interactions_changed",
        KernelEvent::SessionUnavailable { .. } => "session_unavailable",
        KernelEvent::RelayStatusChanged { .. } => "relay_status_changed",
        KernelEvent::RemoteMachinesChanged { .. } => "remote_machines_changed",
        KernelEvent::WaitingRoomInventoryChanged { .. } => "waiting_room_inventory_changed",
        KernelEvent::WaitingRoomRowsChanged { .. } => "waiting_room_rows_changed",
        KernelEvent::ProviderCatalogChanged { .. } => "provider_catalog_changed",
        KernelEvent::SlicesChanged { .. } => "slices_changed",
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
        KernelEvent::AgentActivityChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::ProviderRunChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::SessionMetadataChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::RuntimeInteractionsChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::SessionUnavailable { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::RelayStatusChanged { .. } => None,
        KernelEvent::RemoteMachinesChanged { .. } => None,
        KernelEvent::WaitingRoomInventoryChanged { .. } => None,
        KernelEvent::WaitingRoomRowsChanged { .. } => None,
        KernelEvent::ProviderCatalogChanged { .. } => None,
        KernelEvent::SlicesChanged { .. } => None,
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
        | KernelEvent::AgentActivityChanged { .. }
        | KernelEvent::ProviderRunChanged { .. }
        | KernelEvent::SessionMetadataChanged { .. }
        | KernelEvent::RuntimeInteractionsChanged { .. }
        | KernelEvent::SessionUnavailable { .. }
        | KernelEvent::RelayStatusChanged { .. }
        | KernelEvent::RemoteMachinesChanged { .. }
        | KernelEvent::WaitingRoomInventoryChanged { .. }
        | KernelEvent::WaitingRoomRowsChanged { .. }
        | KernelEvent::ProviderCatalogChanged { .. }
        | KernelEvent::SlicesChanged { .. }
        | KernelEvent::WorkflowDesignOp { .. }
        | KernelEvent::WorkflowRunUpdated { .. }
        | KernelEvent::Heartbeat { .. }
        | KernelEvent::TransportResumed { .. }
        | KernelEvent::ReplayGap { .. } => true,
    }
}

pub(crate) fn terminal_output_event_batches(
    records: Vec<TerminalOutputRecord>,
) -> Vec<Vec<TerminalOutputRecord>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut current_json_bytes = empty_terminal_output_event_json_bytes();
    for record in records {
        let record_json_bytes = terminal_output_record_json_bytes(&record);
        if current.is_empty() {
            current_json_bytes = terminal_output_event_json_bytes_for_records(record_json_bytes, 1);
            current.push(record);
            continue;
        }
        let candidate_len = current_json_bytes
            .saturating_add(record_json_bytes)
            .saturating_add(1);
        if candidate_len <= MAX_TERMINAL_OUTPUT_EVENT_JSON_BYTES {
            current_json_bytes = candidate_len;
            current.push(record);
        } else {
            batches.push(current);
            current_json_bytes = terminal_output_event_json_bytes_for_records(record_json_bytes, 1);
            current = vec![record];
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

fn terminal_output_event_json_bytes(records: &[TerminalOutputRecord]) -> usize {
    serde_json::to_vec(&KernelEvent::TerminalOutput {
        records: records.to_vec(),
    })
    .map(|bytes| bytes.len())
    .unwrap_or(usize::MAX)
}

fn empty_terminal_output_event_json_bytes() -> usize {
    terminal_output_event_json_bytes(&[])
}

fn terminal_output_record_json_bytes(record: &TerminalOutputRecord) -> usize {
    serde_json::to_vec(record)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn terminal_output_event_json_bytes_for_records(record_bytes: usize, record_count: usize) -> usize {
    if record_count == 0 {
        return empty_terminal_output_event_json_bytes();
    }
    empty_terminal_output_event_json_bytes()
        .saturating_add(record_bytes)
        .saturating_add(record_count.saturating_sub(1))
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

#[cfg(test)]
mod tests;

fn kernel_error(code: &str, error: &DaemonError, retryable: bool) -> KernelTransportError {
    KernelTransportError {
        code: code.to_string(),
        message: error.to_string(),
        retryable,
    }
}

pub(crate) fn serialize_frame(frame: &KernelOutgoingFrame) -> Result<String, DaemonError> {
    let mut value = serde_json::to_value(frame).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize kernel websocket frame",
        message: error.to_string(),
    })?;
    crate::local::redact_client_response_value(&mut value);
    serde_json::to_string(&value).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize kernel websocket frame",
        message: error.to_string(),
    })
}
