use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse};
use crate::runtime::projection::{
    publish_session_runtime_projection, AgentRuntimeActivity, AgentRuntimeProjectionStore,
    ProviderProcessProjectionStore, ProviderRunProjectionStore, SessionStateProjectionStore,
};
use crate::runtime::provider_launch_executor::ProviderLaunchPendingTracker;
use crate::runtime::session_actor::FocusedAgentProjection;
use crate::session::RuntimeSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FocusProjectionRefresh {
    None,
    AgentSpawn,
    SnapshotSession { session_id: String },
}

pub(crate) fn focus_projection_refresh(request: &LocalDaemonRequest) -> FocusProjectionRefresh {
    match request {
        LocalDaemonRequest::SpawnAgent(_) | LocalDaemonRequest::SpawnAgents(_) => {
            FocusProjectionRefresh::AgentSpawn
        }
        LocalDaemonRequest::AliasAgent(request) => FocusProjectionRefresh::SnapshotSession {
            session_id: request.session_id.clone(),
        },
        LocalDaemonRequest::UpdateAgentConfig(request) => FocusProjectionRefresh::SnapshotSession {
            session_id: request.session_id.clone(),
        },
        LocalDaemonRequest::UpdateAgentProfile(request) => {
            FocusProjectionRefresh::SnapshotSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::UpdateAgentSubstitutes(request) => {
            FocusProjectionRefresh::SnapshotSession {
                session_id: request.session_id.clone(),
            }
        }
        LocalDaemonRequest::DestroyAgent(request) => FocusProjectionRefresh::SnapshotSession {
            session_id: request.session_id.clone(),
        },
        _ => FocusProjectionRefresh::None,
    }
}

pub(crate) async fn apply_focus_projection_refresh(
    app: &Arc<Mutex<DaemonApp>>,
    focus_projection: &FocusedAgentProjection,
    session_projection: &SessionStateProjectionStore,
    refresh: FocusProjectionRefresh,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    if result.is_err() {
        return;
    }
    match refresh {
        FocusProjectionRefresh::None => {}
        FocusProjectionRefresh::AgentSpawn => {
            if let Ok(LocalDaemonResponse::AgentSpawned { agent }) = result {
                focus_projection
                    .update(agent.session_id(), Some(agent.id()))
                    .await;
            } else if let Ok(LocalDaemonResponse::AgentsSpawned { agents }) = result {
                if let Some(agent) = agents.last() {
                    focus_projection
                        .update(agent.session_id(), Some(agent.id()))
                        .await;
                }
            }
        }
        FocusProjectionRefresh::SnapshotSession { session_id } => {
            let focused_agent_id = if let Some(session) = session_projection.get(&session_id) {
                session.focused_agent_id().map(str::to_string)
            } else if let Ok(app) = app.try_lock() {
                app.sessions()
                    .get_session(&session_id)
                    .ok()
                    .and_then(|session| session.focused_agent_id().map(str::to_string))
            } else {
                return;
            };
            focus_projection
                .update(&session_id, focused_agent_id.as_deref())
                .await;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionProjectionRefresh {
    None,
    SnapshotAgentResponse,
}

impl SessionProjectionRefresh {
    pub(crate) fn session_ids(&self, response: &LocalDaemonResponse) -> Vec<String> {
        match self {
            SessionProjectionRefresh::None => Vec::new(),
            SessionProjectionRefresh::SnapshotAgentResponse => match response {
                LocalDaemonResponse::AgentSpawned { agent }
                | LocalDaemonResponse::AgentAliased { agent, .. }
                | LocalDaemonResponse::AgentConfigUpdated { agent, .. }
                | LocalDaemonResponse::AgentProfileUpdated { agent, .. }
                | LocalDaemonResponse::AgentDestroyed { agent }
                | LocalDaemonResponse::AgentFocused { agent } => {
                    vec![agent.session_id().to_string()]
                }
                LocalDaemonResponse::AgentOutputSeenAcknowledged { session_id, .. } => {
                    vec![session_id.clone()]
                }
                LocalDaemonResponse::AgentFocusCycled { agent: Some(agent) } => {
                    vec![agent.session_id().to_string()]
                }
                LocalDaemonResponse::AgentsSpawned { agents } => agents
                    .iter()
                    .map(|agent| agent.session_id().to_string())
                    .collect(),
                _ => Vec::new(),
            },
        }
    }
}

pub(crate) fn session_projection_refresh(request: &LocalDaemonRequest) -> SessionProjectionRefresh {
    match request {
        LocalDaemonRequest::AttachToSession(_)
        | LocalDaemonRequest::DetachFromSession(_)
        | LocalDaemonRequest::FocusAgent(_)
        | LocalDaemonRequest::AcknowledgeAgentOutputSeen(_)
        | LocalDaemonRequest::CycleAgentFocus(_) => SessionProjectionRefresh::None,
        LocalDaemonRequest::SpawnAgent(_)
        | LocalDaemonRequest::SpawnAgents(_)
        | LocalDaemonRequest::AliasAgent(_)
        | LocalDaemonRequest::UpdateAgentConfig(_)
        | LocalDaemonRequest::UpdateAgentProfile(_)
        | LocalDaemonRequest::UpdateAgentSubstitutes(_)
        | LocalDaemonRequest::DestroyAgent(_) => SessionProjectionRefresh::SnapshotAgentResponse,
        LocalDaemonRequest::CompletePrompt(_) | LocalDaemonRequest::CancelActivePrompt(_) => {
            SessionProjectionRefresh::None
        }
        LocalDaemonRequest::PumpTerminalOutput(_)
        | LocalDaemonRequest::SendTerminalInput(_)
        | LocalDaemonRequest::AppendNativeProviderOutput(_)
        | LocalDaemonRequest::AppendNativeProviderOutputBatch(_) => SessionProjectionRefresh::None,
        LocalDaemonRequest::PollRuntimeNotices(_) | LocalDaemonRequest::ResizeTerminal(_) => {
            SessionProjectionRefresh::None
        }
        _ => SessionProjectionRefresh::None,
    }
}

pub(crate) struct SessionProjectionRefreshContext<'a> {
    pub(crate) app: &'a Arc<Mutex<DaemonApp>>,
    pub(crate) session_projection: &'a SessionStateProjectionStore,
    pub(crate) agent_runtime_projection: &'a AgentRuntimeProjectionStore,
    pub(crate) provider_process_projection: &'a ProviderProcessProjectionStore,
    pub(crate) provider_launch_pending: &'a ProviderLaunchPendingTracker,
    pub(crate) provider_run_projection: &'a ProviderRunProjectionStore,
}

pub(crate) async fn apply_session_projection_refresh(
    context: SessionProjectionRefreshContext<'_>,
    refresh: SessionProjectionRefresh,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    let response = match result {
        Ok(response) => response,
        Err(_) => return,
    };

    let mut refreshed_session_ids = Vec::new();
    for session in response_sessions(response) {
        refreshed_session_ids.push(session.id().to_string());
        if should_update_agent_runtime_projection_from_response(response) {
            publish_session_runtime_projection(
                context.session_projection,
                context.agent_runtime_projection,
                &session,
            );
        } else {
            context.session_projection.update(session);
        }
    }
    if let LocalDaemonResponse::SessionsListed { sessions } = response {
        for session in sessions {
            context.agent_runtime_projection.update_session(session);
        }
        context.session_projection.update_list(sessions.clone());
    }
    for session_id in response_removed_session_ids(response) {
        context.agent_runtime_projection.remove_session(session_id);
        context.session_projection.remove(session_id);
        refreshed_session_ids.push(session_id.to_string());
    }

    let mut snapshot_session_ids = refresh.session_ids(response);
    snapshot_session_ids.sort();
    snapshot_session_ids.dedup();
    match refresh {
        SessionProjectionRefresh::None => {}
        SessionProjectionRefresh::SnapshotAgentResponse => {
            for session_id in snapshot_session_ids {
                if let Some(session) = context.session_projection.get(&session_id) {
                    refreshed_session_ids.push(session.id().to_string());
                    context.agent_runtime_projection.update_session(&session);
                }
            }
        }
    }

    if !matches!(refresh, SessionProjectionRefresh::None) || !refreshed_session_ids.is_empty() {
        context.provider_process_projection.invalidate();
    }

    refreshed_session_ids.sort();
    refreshed_session_ids.dedup();
    for session_id in refreshed_session_ids {
        context
            .provider_launch_pending
            .clear_if_settled(
                context.app,
                &session_id,
                context.session_projection,
                context.provider_run_projection,
            )
            .await;
    }
}

pub(crate) fn response_sessions(response: &LocalDaemonResponse) -> Vec<RuntimeSession> {
    match response {
        LocalDaemonResponse::SessionCreated { session, .. }
        | LocalDaemonResponse::SessionResolved { session }
        | LocalDaemonResponse::SessionState { session, .. }
        | LocalDaemonResponse::InteractionResponded { session, .. }
        | LocalDaemonResponse::PromptSubmitted { session, .. }
        | LocalDaemonResponse::QueuedPromptSteered { session, .. }
        | LocalDaemonResponse::QueuedPromptCancelled { session, .. }
        | LocalDaemonResponse::QueuedPromptUpdated { session, .. }
        | LocalDaemonResponse::SessionConfigUpdated { session, .. }
        | LocalDaemonResponse::MetaagentTaskUpdated { session, .. }
        | LocalDaemonResponse::AgentAliased { session, .. }
        | LocalDaemonResponse::AgentConfigUpdated { session, .. }
        | LocalDaemonResponse::AgentProfileUpdated { session, .. }
        | LocalDaemonResponse::SessionEnded { session }
        | LocalDaemonResponse::SessionAliased { session }
        | LocalDaemonResponse::WorkspaceLinkCreated { session, .. }
        | LocalDaemonResponse::WorkspaceLinkAttached { session, .. }
        | LocalDaemonResponse::WorkspaceLinkDetached { session, .. }
        | LocalDaemonResponse::WorkspaceLiveSyncModeUpdated { session, .. }
        | LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowCodeApplied { session, .. }
        | LocalDaemonResponse::WorkflowCodeRun { session, .. }
        | LocalDaemonResponse::WorkflowDesignOpAccepted { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointCreated { session, .. }
        | LocalDaemonResponse::WorkflowEndpointAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointBound { session, .. }
        | LocalDaemonResponse::WorkflowNodeAdded { session, .. }
        | LocalDaemonResponse::WorkflowNodeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowNodeInstructionsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEdgeAdded { session, .. }
        | LocalDaemonResponse::WorkflowEdgeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowCanvasLayoutUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunInvoked { session, .. }
        | LocalDaemonResponse::WorkflowPromptEnqueued { session, .. }
        | LocalDaemonResponse::WorkflowRunCancelled { session, .. }
        | LocalDaemonResponse::WorkflowRunPaused { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowScheduleCreated { session, .. }
        | LocalDaemonResponse::WorkflowScheduleUpdated { session, .. }
        | LocalDaemonResponse::WorkflowScheduleRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueCreated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueUpdated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowPromptUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowPromptRemoved { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => vec![session.clone()],
        _ => Vec::new(),
    }
}

pub(crate) fn redact_agent_activity_for_session(
    mut agent_activity: BTreeMap<String, AgentRuntimeActivity>,
    session: &RuntimeSession,
) -> BTreeMap<String, AgentRuntimeActivity> {
    agent_activity.retain(|agent_id, _| {
        session
            .agents()
            .iter()
            .any(|agent| agent.id() == agent_id.as_str())
    });
    agent_activity
}

pub(crate) fn should_update_agent_runtime_projection_from_response(
    response: &LocalDaemonResponse,
) -> bool {
    !matches!(
        response,
        LocalDaemonResponse::PromptSubmitted { .. }
            | LocalDaemonResponse::QueuedPromptSteered { .. }
    )
}

pub(crate) fn response_removed_session_ids(response: &LocalDaemonResponse) -> Vec<&str> {
    match response {
        LocalDaemonResponse::SessionDeleted { session } => vec![session.id()],
        LocalDaemonResponse::KernelDeleted {
            deleted_sessions, ..
        } => deleted_sessions
            .iter()
            .map(|session| session.id())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{
        AliasAgentRequest, CompletePromptRequest, DestroyAgentRequest, SendTerminalInputRequest,
        SpawnAgentRequest, SpawnAgentsRequest,
    };
    use crate::session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

    #[test]
    fn focus_projection_refresh_tracks_agent_identity_changes() {
        assert_eq!(
            focus_projection_refresh(&LocalDaemonRequest::SpawnAgent(spawn_request())),
            FocusProjectionRefresh::AgentSpawn,
        );
        assert_eq!(
            focus_projection_refresh(&LocalDaemonRequest::SpawnAgents(SpawnAgentsRequest {
                session_id: "session-1".to_string(),
                agents: Vec::new(),
            })),
            FocusProjectionRefresh::AgentSpawn,
        );
        assert_eq!(
            focus_projection_refresh(&LocalDaemonRequest::AliasAgent(AliasAgentRequest {
                session_id: "session-1".to_string(),
                agent_id: "agent-1".to_string(),
                alias: "Main".to_string(),
            })),
            FocusProjectionRefresh::SnapshotSession {
                session_id: "session-1".to_string(),
            },
        );
        assert_eq!(
            focus_projection_refresh(&LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
                session_id: "session-2".to_string(),
                agent_id: "agent-1".to_string(),
            })),
            FocusProjectionRefresh::SnapshotSession {
                session_id: "session-2".to_string(),
            },
        );
    }

    #[test]
    fn session_projection_refresh_snapshots_agent_mutations_only() {
        assert_eq!(
            session_projection_refresh(&LocalDaemonRequest::SpawnAgent(spawn_request())),
            SessionProjectionRefresh::SnapshotAgentResponse,
        );
        assert_eq!(
            session_projection_refresh(&LocalDaemonRequest::SpawnAgents(SpawnAgentsRequest {
                session_id: "session-1".to_string(),
                agents: Vec::new(),
            })),
            SessionProjectionRefresh::SnapshotAgentResponse,
        );
        assert_eq!(
            session_projection_refresh(&LocalDaemonRequest::CompletePrompt(
                CompletePromptRequest {
                    session_id: "session-1".to_string(),
                },
            )),
            SessionProjectionRefresh::None,
        );
        assert_eq!(
            session_projection_refresh(&LocalDaemonRequest::SendTerminalInput(
                SendTerminalInputRequest {
                    session_id: "session-1".to_string(),
                    attachment_id: "attachment-1".to_string(),
                    provider_run_id: None,
                    data_base64: "aGVsbG8=".to_string(),
                },
            )),
            SessionProjectionRefresh::None,
        );
    }

    #[test]
    fn prompt_responses_with_session_snapshots_refresh_session_projection() {
        let session = runtime_session("session-prompt");
        let prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "hello",
            PromptStatus::Running,
        );

        let submitted = LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started {
                prompt: prompt.clone(),
            },
            session: session.clone(),
            agent_activity: BTreeMap::new(),
            agent_activity_revision: 0,
        };
        let steered = LocalDaemonResponse::QueuedPromptSteered {
            prompt: prompt.clone(),
            session: session.clone(),
            agent_activity: BTreeMap::new(),
            agent_activity_revision: 0,
        };
        let cancelled = LocalDaemonResponse::QueuedPromptCancelled {
            prompt: prompt.clone(),
            session: session.clone(),
            agent_activity: BTreeMap::new(),
            agent_activity_revision: 0,
        };
        let updated = LocalDaemonResponse::QueuedPromptUpdated {
            prompt,
            session: session.clone(),
            agent_activity: BTreeMap::new(),
            agent_activity_revision: 0,
        };

        for response in [submitted, steered, cancelled, updated] {
            assert_eq!(response_sessions(&response), vec![session.clone()]);
        }
    }

    #[test]
    fn prompt_submit_and_steer_do_not_overwrite_agent_runtime_projection_from_response() {
        let session = runtime_session("session-agent-runtime");
        let prompt = PromptQueueItem::new(
            "prompt-1",
            "attachment-1",
            "agent-1",
            "hello",
            PromptStatus::Running,
        );

        assert!(!should_update_agent_runtime_projection_from_response(
            &LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started {
                    prompt: prompt.clone(),
                },
                session: session.clone(),
                agent_activity: BTreeMap::new(),
                agent_activity_revision: 0,
            },
        ));
        assert!(!should_update_agent_runtime_projection_from_response(
            &LocalDaemonResponse::QueuedPromptSteered {
                prompt,
                session,
                agent_activity: BTreeMap::new(),
                agent_activity_revision: 0,
            },
        ));
    }

    fn spawn_request() -> SpawnAgentRequest {
        SpawnAgentRequest {
            session_id: "session-1".to_string(),
            alias: None,
            provider: None,
            model: None,
            effort: None,
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }
    }

    fn runtime_session(id: &str) -> RuntimeSession {
        RuntimeSession::new(id, None, "workspace", "worktree", "machine", "kernel")
    }
}
