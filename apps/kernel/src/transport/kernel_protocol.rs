use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::WorkspaceLiveSyncMode;
use crate::error::DaemonError;
use crate::local::{
    LocalDaemonRequest, PasskeyPrompt, RelayStatus, RemoteMachineRecord, WaitingRoomLaunchTarget,
    WaitingRoomPublicSessionSummary, WaitingRoomPublicSnapshot, WorkflowDesignOpForwarded,
};
use crate::provider::{OpenCodeProviderCatalog, PublicProviderRun};
use crate::runtime::projection::{AgentRuntimeActivity, SessionSnapshotProjection};
use crate::session::{RuntimeInteraction, RuntimeSession, WorkflowRun};
use crate::slice::SliceRecord;
use crate::terminal::{RuntimeNoticeRecord, TerminalOutputRecord};

pub(crate) const WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE: &str = "waiting_room_inventory";
pub(crate) const WAITING_ROOM_INVENTORY_SENTINEL_ID: &str = "__waiting_room_inventory__";
pub(crate) const MAX_TERMINAL_OUTPUT_EVENT_JSON_BYTES: usize = 128 * 1024;

/// Import clients reuse a paired key. Authenticate the exact encrypted request
/// nonce inside the reply, rather than trusting the relay's outer request ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserImportRelayResponse {
    pub(crate) request_nonce: String,
    pub(crate) response: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve the existing KernelIncomingFrame typed actor payload layout"
)]
pub(crate) enum KernelIncomingFrame {
    Request {
        request_id: String,
        #[serde(default)]
        command_id: Option<String>,
        #[serde(default)]
        causation_id: Option<String>,
        #[serde(default)]
        correlation_id: Option<String>,
        /// Protocol 568: forward this request through the kernel gateway.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kernel: Option<String>,
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
        /// Protocol 596: how often the subscription's `heartbeat` arrives; at least
        /// `MIN_SUBSCRIPTION_HEARTBEAT_MS`. Left out, the default interval applies.
        #[serde(default)]
        heartbeat_interval_ms: Option<u64>,
        /// Protocol 568: subscribe on this machine through the gateway.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kernel: Option<String>,
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
        /// Protocol 568: the kernel that answered a forwarded request, so a
        /// client can tell a forwarded answer from a local one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kernel: Option<String>,
        response: Box<Option<Value>>,
        error: Option<KernelTransportError>,
    },
    Event {
        event_id: u64,
        /// Protocol 568: the kernel whose subscription produced this event; its
        /// `event_id` counts on that machine's stream. Absent for this kernel's own.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kernel: Option<String>,
        event: Box<KernelEvent>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum KernelEvent {
    KernelBrowserFrame {
        subscription_id: String,
        frame: Value,
    },
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
    /// Protocol 596: the agent's latest turn changed. `turn` is what
    /// `GetSessionHistoryOutline` answers for that turn at `history_revision`.
    AgentTurnChanged {
        session_id: String,
        agent_id: String,
        history_revision: u64,
        turn: Box<crate::local::SessionHistoryOutlineTurn>,
    },
    /// Protocol 596: a row `ListHomeExtensionAudit` returns for the agent was appended. The row is
    /// `audit_event`: `event` is the tag of every kernel event.
    HomeExtensionAuditAppended {
        session_id: String,
        agent_id: String,
        audit_event: Box<crate::durable_state::DurableStateEvent>,
    },
    /// Protocol 596: audit rows were lost. Re-read `ListHomeExtensionAudit` for the watched agents
    /// once, replacing their lists; appends after this signal continue normally.
    HomeExtensionAuditLagged {
        session_id: String,
    },
    RoomWorkflowsChanged {
        inventory: crate::runtime::projection::RoomWorkflowInventory,
    },
    SessionProcessesChanged {
        session_id: String,
        processes: Vec<crate::runtime::process_resources::SessionProcess>,
    },
    SessionSnapshot {
        room_workflows: crate::runtime::projection::RoomWorkflowInventory,
        session: Box<RuntimeSession>,
        provider_run: Box<Option<PublicProviderRun>>,
        #[allow(
            clippy::box_collection,
            reason = "Keeps the protocol enum payload bounded independently of map header size"
        )]
        agent_activity: Box<BTreeMap<String, AgentRuntimeActivity>>,
        #[serde(default)]
        agent_activity_revision: u64,
    },
    AgentTaskUpdated {
        session_id: String,
        task: Box<crate::durable_state::agent_lifecycle::AgentTaskExecution>,
    },
    AgentActivityChanged {
        session_id: String,
        #[allow(
            clippy::box_collection,
            reason = "Keeps the protocol enum payload bounded independently of map header size"
        )]
        agent_activity: Box<BTreeMap<String, AgentRuntimeActivity>>,
        #[serde(default)]
        agent_activity_revision: u64,
    },
    ProviderRunChanged {
        session_id: String,
        provider_run: Option<PublicProviderRun>,
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
        #[serde(default)]
        sessions_total: usize,
        #[serde(default)]
        sessions_next_cursor: Option<String>,
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
    ClientCatalogChanged {
        revision: u64,
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
    /// Protocol 403: every passkey prompt now pending for this terminal's
    /// user, on every subscription of a connection that may submit a passkey.
    /// Sent when the subscription starts and whenever the set changes; a
    /// prompt that leaves the set was answered or expired, so its popup
    /// closes. Never replayed: each subscription sends the current set.
    PasskeyPromptsChanged {
        prompts: Vec<PasskeyPrompt>,
        kernel_now_ms: u64,
        closed: Vec<crate::local::PasskeyPromptClosure>,
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

pub(crate) const MIN_SUBSCRIPTION_HEARTBEAT_MS: u64 = 500;
pub(crate) const INVALID_HEARTBEAT_INTERVAL_CODE: &str = "invalid_heartbeat_interval";

/// The interval a subscription asked for, or `default` when it did not ask.
pub(crate) fn requested_heartbeat_interval(
    requested_ms: Option<u64>,
    default: std::time::Duration,
) -> Result<std::time::Duration, KernelTransportError> {
    match requested_ms {
        None => Ok(default),
        Some(ms) if ms >= MIN_SUBSCRIPTION_HEARTBEAT_MS => Ok(std::time::Duration::from_millis(ms)),
        Some(ms) => Err(KernelTransportError {
            code: INVALID_HEARTBEAT_INTERVAL_CODE.to_string(),
            message: format!(
                "heartbeat_interval_ms {ms} is below the minimum of {MIN_SUBSCRIPTION_HEARTBEAT_MS}; leave it out for the default of {} ms",
                default.as_millis()
            ),
            retryable: false,
        }),
    }
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
        KernelEvent::RoomWorkflowsChanged { inventory } => {
            serde_json::json!({"event_id": event_id, "event": "room_workflows_changed", "inventory": inventory})
        }
        KernelEvent::SessionSnapshot {
            room_workflows,
            session,
            provider_run,
            agent_activity,
            agent_activity_revision,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "session_snapshot",
            "room_workflows": room_workflows,
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
        KernelEvent::AgentTurnChanged {
            session_id,
            agent_id,
            history_revision,
            turn,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "agent_turn_changed",
            "session_id": session_id,
            "agent_id": agent_id,
            "history_revision": history_revision,
            "turn_id": turn.turn_id,
            "lifecycle": turn.lifecycle,
        }),
        KernelEvent::HomeExtensionAuditAppended {
            session_id,
            agent_id,
            audit_event: audit,
        } => serde_json::json!({
            "event_id": event_id,
            "event": "home_extension_audit_appended",
            "session_id": session_id,
            "agent_id": agent_id,
            "kind": audit.kind,
            "sequence": audit.sequence,
        }),
        KernelEvent::HomeExtensionAuditLagged { session_id } => serde_json::json!({
            "event_id": event_id,
            "event": "home_extension_audit_lagged",
            "session_id": session_id,
        }),
        KernelEvent::AgentTaskUpdated { session_id, task } => serde_json::json!({
            "event_id": event_id, "event": "agent_task_updated", "session_id": session_id,
            "task_id": task.task_id, "revision": task.revision, "state": task.state,
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
        KernelEvent::ClientCatalogChanged { revision } => {
            serde_json::json!({ "event_id": event_id, "event": "client_catalog_changed", "revision": revision })
        }
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
        KernelEvent::PasskeyPromptsChanged { prompts, .. } => serde_json::json!({
            "event_id": event_id,
            "event": "passkey_prompts_changed",
            "prompt_count": prompts.len(),
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
            && previous.sessions_total == snapshot.sessions_total
            && previous.sessions_next_cursor == snapshot.sessions_next_cursor
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
        sessions_total: snapshot.sessions_total,
        sessions_next_cursor: snapshot.sessions_next_cursor,
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

pub(crate) fn waiting_room_inventory_changed_event(
    snapshot: &WaitingRoomPublicSnapshot,
    previous_snapshot: Option<&WaitingRoomPublicSnapshot>,
) -> Option<KernelEvent> {
    let requires_full_refresh = previous_snapshot.is_none_or(|previous| {
        previous.external_provider_sessions != snapshot.external_provider_sessions
            || previous.external_provider_sessions_has_more
                != snapshot.external_provider_sessions_has_more
            || previous.external_provider_sessions_next_cursor
                != snapshot.external_provider_sessions_next_cursor
            || previous.relay_status != snapshot.relay_status
            || previous.remote_machines != snapshot.remote_machines
            || previous.remote_kernels != snapshot.remote_kernels
            || previous.terminals != snapshot.terminals
            || previous.provider_accounts != snapshot.provider_accounts
            || previous.git_credentials != snapshot.git_credentials
    });
    requires_full_refresh.then(|| KernelEvent::WaitingRoomInventoryChanged {
        inventory_version: snapshot.inventory_version.clone(),
    })
}

#[derive(Default)]
pub(crate) struct WaitingRoomInventoryEventProjection {
    previous_snapshot: Option<WaitingRoomPublicSnapshot>,
}

impl WaitingRoomInventoryEventProjection {
    pub(crate) fn project(&mut self, snapshot: WaitingRoomPublicSnapshot) -> Vec<KernelEvent> {
        let inventory_event =
            waiting_room_inventory_changed_event(&snapshot, self.previous_snapshot.as_ref());
        let rows_event =
            waiting_room_rows_changed_event(snapshot.clone(), self.previous_snapshot.as_ref());
        self.previous_snapshot = Some(snapshot);
        [rows_event, inventory_event]
            .into_iter()
            .flatten()
            .collect()
    }
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
        provider_run: snapshot.provider_run.as_ref().map(PublicProviderRun::from),
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

/// MP-08/MP-10/MP-11: local and relay subscribers share the decision about
/// replacing a full session snapshot with smaller projection events.
// MP-08 / MP-09 / MP-10 / MP-11 A12: one delta selection for every terminal transport.
pub(crate) fn session_projection_changed_events(
    snapshot: &SessionSnapshotProjection,
    previous: Option<&SessionSnapshotProjection>,
) -> Vec<KernelEvent> {
    [
        (previous.is_none_or(|previous| previous.processes != snapshot.processes)
            && (previous.is_some() || !snapshot.processes.is_empty()))
        .then(|| KernelEvent::SessionProcessesChanged {
            session_id: snapshot.session.id().to_string(),
            processes: snapshot.processes.clone(),
        }),
        agent_activity_changed_event(snapshot, previous),
        provider_run_changed_event(snapshot, previous),
        session_metadata_changed_event(snapshot, previous),
        runtime_interactions_changed_event(snapshot, previous),
    ]
    .into_iter()
    .flatten()
    .chain(agent_task_updated_events(snapshot, previous))
    .collect()
}

fn agent_task_updated_events(
    snapshot: &SessionSnapshotProjection,
    previous: Option<&SessionSnapshotProjection>,
) -> Vec<KernelEvent> {
    let Some(previous) = previous else {
        return vec![];
    };
    snapshot
        .session
        .agent_tasks()
        .iter()
        .filter(|task| !previous.session.agent_tasks().contains(task))
        .cloned()
        .map(|task| KernelEvent::AgentTaskUpdated {
            session_id: snapshot.session.id().to_owned(),
            task: Box::new(task),
        })
        .collect()
}

pub(crate) fn can_skip_session_snapshot(
    snapshot: &SessionSnapshotProjection,
    previous_snapshot: Option<&SessionSnapshotProjection>,
    emitted_projection_delta: bool,
) -> bool {
    // Smaller events do not carry window status. Keep the full snapshot when
    // a window opens, warns, extends or ends alongside another projection delta.
    emitted_projection_delta
        && previous_snapshot.is_some_and(|previous| {
            // Interaction and task deltas cannot carry a changed execution route
            // or account. Keep the full agent projection alongside those deltas.
            if previous.session.agents() != snapshot.session.agents() {
                return false;
            }
            if previous.session.sudo_windows() != snapshot.session.sudo_windows() {
                return false;
            }
            // Wake changes have no event of their own.
            if previous.session.agent_wakes() != snapshot.session.agent_wakes() {
                return false;
            }
            if previous.session.agent_tasks() == snapshot.session.agent_tasks() {
                return true;
            }
            // MP-08/MP-10: task events carry no prompt, agent, activity or
            // provider state. Preserve a snapshot for accompanying changes.
            if previous.provider_run != snapshot.provider_run
                || previous.agent_activity != snapshot.agent_activity
            {
                return false;
            }
            // Task updates have no removal event.
            let task_ids = snapshot
                .session
                .agent_tasks()
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<BTreeSet<_>>();
            if previous
                .session
                .agent_tasks()
                .iter()
                .any(|task| !task_ids.contains(task.task_id.as_str()))
            {
                return false;
            }
            let mut previous_session = previous.session.clone();
            previous_session.set_agent_tasks(snapshot.session.agent_tasks().to_vec());
            previous_session == snapshot.session
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
    let previous_runs = previous_snapshot
        .into_iter()
        .flat_map(|snapshot| snapshot.session.workflow_runs())
        .map(|run| (run.id(), run))
        .collect::<BTreeMap<_, _>>();
    snapshot
        .session
        .workflow_runs()
        .iter()
        // MP-08/MP-10: archival publishes the authoritative terminal update.
        // A hot-snapshot diff must not duplicate that same terminal transition.
        .filter(|run| !run.status().is_terminal())
        .filter(|run| previous_runs.get(run.id()).copied() != Some(*run))
        .cloned()
        .collect()
}

pub(crate) fn kernel_event_name(event: &KernelEvent) -> &'static str {
    match event {
        KernelEvent::KernelBrowserFrame { .. } => "kernel_browser_frame",
        KernelEvent::TerminalOutput { .. } => "terminal_output",
        KernelEvent::RuntimeNotices { .. } => "runtime_notices",
        KernelEvent::AssistantMessageCompleted { .. } => "assistant_message_completed",
        KernelEvent::AgentTurnChanged { .. } => "agent_turn_changed",
        KernelEvent::HomeExtensionAuditAppended { .. } => "home_extension_audit_appended",
        KernelEvent::HomeExtensionAuditLagged { .. } => "home_extension_audit_lagged",
        KernelEvent::RoomWorkflowsChanged { .. } => "room_workflows_changed",
        KernelEvent::SessionProcessesChanged { .. } => "session_processes_changed",
        KernelEvent::SessionSnapshot { .. } => "session_snapshot",
        KernelEvent::AgentTaskUpdated { .. } => "agent_task_updated",
        KernelEvent::AgentActivityChanged { .. } => "agent_activity_changed",
        KernelEvent::ProviderRunChanged { .. } => "provider_run_changed",
        KernelEvent::SessionMetadataChanged { .. } => "session_metadata_changed",
        KernelEvent::RuntimeInteractionsChanged { .. } => "runtime_interactions_changed",
        KernelEvent::SessionUnavailable { .. } => "session_unavailable",
        KernelEvent::RelayStatusChanged { .. } => "relay_status_changed",
        KernelEvent::RemoteMachinesChanged { .. } => "remote_machines_changed",
        KernelEvent::WaitingRoomInventoryChanged { .. } => "waiting_room_inventory_changed",
        KernelEvent::WaitingRoomRowsChanged { .. } => "waiting_room_rows_changed",
        KernelEvent::ClientCatalogChanged { .. } => "client_catalog_changed",
        KernelEvent::ProviderCatalogChanged { .. } => "provider_catalog_changed",
        KernelEvent::SlicesChanged { .. } => "slices_changed",
        KernelEvent::WorkflowDesignOp { .. } => "workflow_design_op",
        KernelEvent::WorkflowRunUpdated { .. } => "workflow_run_updated",
        KernelEvent::Heartbeat { .. } => "heartbeat",
        KernelEvent::TransportResumed { .. } => "transport_resumed",
        KernelEvent::ReplayGap { .. } => "replay_gap",
        KernelEvent::PasskeyPromptsChanged { .. } => "passkey_prompts_changed",
    }
}

pub(crate) fn event_session_id(event: &KernelEvent) -> Option<&str> {
    match event {
        KernelEvent::KernelBrowserFrame { .. } => None,
        KernelEvent::TerminalOutput { records } => {
            records.first().map(|record| record.session_id.as_str())
        }
        KernelEvent::RuntimeNotices { notices } => {
            notices.first().map(|notice| notice.session_id.as_str())
        }
        KernelEvent::AssistantMessageCompleted { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::AgentTurnChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::HomeExtensionAuditAppended { session_id, .. }
        | KernelEvent::HomeExtensionAuditLagged { session_id } => Some(session_id.as_str()),
        KernelEvent::RoomWorkflowsChanged { inventory } => Some(&inventory.session_id),
        KernelEvent::SessionProcessesChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::SessionSnapshot { session, .. } => Some(session.id()),
        KernelEvent::AgentTaskUpdated { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::AgentActivityChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::ProviderRunChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::SessionMetadataChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::RuntimeInteractionsChanged { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::SessionUnavailable { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::RelayStatusChanged { .. } => None,
        KernelEvent::RemoteMachinesChanged { .. } => None,
        KernelEvent::WaitingRoomInventoryChanged { .. } => None,
        KernelEvent::WaitingRoomRowsChanged { .. } => None,
        KernelEvent::ClientCatalogChanged { .. } => None,
        KernelEvent::ProviderCatalogChanged { .. } => None,
        KernelEvent::SlicesChanged { .. } => None,
        KernelEvent::WorkflowDesignOp { design_op } => Some(design_op.session_id.as_str()),
        KernelEvent::WorkflowRunUpdated { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::Heartbeat { session_id } => Some(session_id.as_str()),
        KernelEvent::TransportResumed { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::ReplayGap { session_id, .. } => Some(session_id.as_str()),
        KernelEvent::PasskeyPromptsChanged { .. } => None,
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
        KernelEvent::KernelBrowserFrame { .. } => false,
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
        KernelEvent::AgentTurnChanged { .. }
        | KernelEvent::HomeExtensionAuditAppended { .. }
        | KernelEvent::HomeExtensionAuditLagged { .. }
        | KernelEvent::RoomWorkflowsChanged { .. }
        | KernelEvent::SessionProcessesChanged { .. }
        | KernelEvent::SessionSnapshot { .. }
        | KernelEvent::AgentTaskUpdated { .. }
        | KernelEvent::AgentActivityChanged { .. }
        | KernelEvent::ProviderRunChanged { .. }
        | KernelEvent::SessionMetadataChanged { .. }
        | KernelEvent::RuntimeInteractionsChanged { .. }
        | KernelEvent::SessionUnavailable { .. }
        | KernelEvent::RelayStatusChanged { .. }
        | KernelEvent::RemoteMachinesChanged { .. }
        | KernelEvent::WaitingRoomInventoryChanged { .. }
        | KernelEvent::WaitingRoomRowsChanged { .. }
        | KernelEvent::ClientCatalogChanged { .. }
        | KernelEvent::ProviderCatalogChanged { .. }
        | KernelEvent::SlicesChanged { .. }
        | KernelEvent::WorkflowDesignOp { .. }
        | KernelEvent::WorkflowRunUpdated { .. }
        | KernelEvent::Heartbeat { .. }
        | KernelEvent::TransportResumed { .. }
        | KernelEvent::ReplayGap { .. } => true,
        // The current set is sent at each subscription start instead.
        KernelEvent::PasskeyPromptsChanged { .. } => false,
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
    if crate::runtime::kernel_access::Refusal::grant_ended(error) {
        return crate::runtime::kernel_access::Refusal::Ended.transport_error();
    }
    match error {
        DaemonError::ProviderAccountNameTaken { .. } => {
            kernel_error("provider_account_name_taken", error, false)
        }
        DaemonError::ProviderLoginCodeIncomplete => {
            kernel_error("provider_login_code_incomplete", error, false)
        }
        DaemonError::ProviderLoginCodeInvalid => {
            kernel_error("provider_login_code_invalid", error, false)
        }
        DaemonError::ProviderAccountCopy {
            code, retryable, ..
        } => kernel_error(code, error, *retryable),
        DaemonError::VaultUnlock {
            code, retryable, ..
        } => kernel_error(code, error, *retryable),
        DaemonError::ExternalRequestFailed { code, retryable } => {
            kernel_error(code, error, *retryable)
        }
        DaemonError::UserDomainRefused { reason } => kernel_error(reason.code(), error, false),
        DaemonError::MachineRequest { code, .. } => {
            kernel_error(code.as_str(), error, code == "exec_input_busy")
        }
        DaemonError::AgentWorkerCleanup { source, .. } => {
            let mut mapped = map_kernel_error(source);
            mapped.message = error.to_string();
            mapped
        }
        DaemonError::ProviderProfileRefused { reason, .. } => {
            kernel_error(reason.code(), error, false)
        }
        DaemonError::AgentNotInSession { .. } => kernel_error("agent_not_in_session", error, false),
        DaemonError::AgentNotFound { .. } => kernel_error("agent_not_found", error, false),
        DaemonError::NoActivePrompt { .. } => kernel_error("no_active_prompt", error, false),
        DaemonError::ProviderPromptSteerRejected { .. } => {
            kernel_error("prompt_steer_rejected", error, false)
        }
        DaemonError::LocalTransport {
            operation: "steer active prompt",
            ..
        } => kernel_error("prompt_steer_rejected", error, false),
        DaemonError::SteerDeliveredEventUnrecorded { .. } => {
            kernel_error("prompt_steer_event_unrecorded", error, false)
        }
        DaemonError::SessionNotFound { .. } => kernel_error("session_not_found", error, false),
        DaemonError::SessionAliasConflict { .. } => {
            kernel_error("session_alias_conflict", error, false)
        }
        DaemonError::InvalidSessionAlias { .. } => {
            kernel_error("invalid_session_alias", error, false)
        }
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
        DaemonError::LocalTransport { operation, .. }
            if *operation == "credential_vault_locked" =>
        {
            kernel_error("credential_vault_locked", error, false)
        }
        DaemonError::RelayTransport {
            operation: "dictation",
            code,
            retryable,
            ..
        } => kernel_error(code, error, *retryable),
        DaemonError::RelayTransport {
            code,
            message,
            retryable,
            ..
        } if code == "provider_capacity_reached" => KernelTransportError {
            code: code.clone(),
            message: message.clone(),
            retryable: *retryable,
        },
        // A terminal on another kernel refuses in the terminal service's own words; they
        // are passed on as they came, so the caller that sent the id reads them.
        DaemonError::RelayTransport {
            code,
            message,
            retryable,
            ..
        } if crate::room_terminal::is_public_code(code) => KernelTransportError {
            code: code.clone(),
            message: message.clone(),
            retryable: *retryable,
        },
        DaemonError::RelayTransport {
            code, retryable, ..
        } if crate::error::actionable_refusal_message(code).is_some() => KernelTransportError {
            code: code.clone(),
            message: crate::error::external_request_failure_message(code).into(),
            retryable: *retryable,
        },
        // Keep the stable local transport code/message for CLI and app
        // clients, while preserving the relay's structured retryability.
        DaemonError::RelayTransport { retryable, .. } => {
            kernel_error("local_transport_error", error, *retryable)
        }
        DaemonError::RelayPeerCleanupAbsent { .. } => {
            kernel_error("local_transport_error", error, false)
        }
        // MP-08 / MP-10 / MP-11: profile absence/isolation needs an explicit
        // account action; repeating the same launch cannot repair it.
        DaemonError::LocalTransport { operation, .. }
            if *operation == "resolve account profile" =>
        {
            kernel_error("provider_profile_unavailable", error, false)
        }
        // MP-08 / MP-10 / MP-11: absent Projects need setup, not transport retries.
        DaemonError::LocalTransport {
            operation: "project.get",
            ..
        } => kernel_error("local_transport_error", error, false),
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
    let value = serde_json::to_value(frame).map_err(|error| DaemonError::LocalTransport {
        operation: "serialize kernel websocket frame",
        message: error.to_string(),
    })?;
    serialize_frame_value(value)
}

/// MP-08 / MP-10 / MP-11: projected frames retain the shared artifact budget.
pub(crate) fn serialize_frame_value(mut value: Value) -> Result<String, DaemonError> {
    crate::local::redact_client_response_value(&mut value);
    let encode = |value: &Value| {
        serde_json::to_string(value).map_err(|error| DaemonError::LocalTransport {
            operation: "serialize kernel websocket frame",
            message: error.to_string(),
        })
    };
    let encoded = encode(&value)?;
    // MP-08/MP-10/MP-11: kernel WebSocket transports share this budget. Large
    // artifacts remain readable in chunks; provider-native delivery is separate.
    if encoded.len() > 1024 * 1024 {
        if let Some(payload) = value
            .pointer_mut("/response/RoomBrowserArtifact/result/payload")
            .and_then(Value::as_object_mut)
        {
            if payload.remove("image_base64").is_some() {
                return encode(&value);
            }
        }
    }
    Ok(encoded)
}

/// Inventory replacements share the existing attachment event stream.
pub(crate) fn room_workflows_changed_event(
    snapshot: &SessionSnapshotProjection,
    previous: Option<&SessionSnapshotProjection>,
) -> Option<KernelEvent> {
    let previous = previous?;
    (snapshot.room_workflows != previous.room_workflows).then(|| {
        KernelEvent::RoomWorkflowsChanged {
            inventory: snapshot.room_workflows.clone(),
        }
    })
}
