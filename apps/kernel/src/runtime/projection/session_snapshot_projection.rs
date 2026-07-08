use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    queued_prompt_controls_projection, AgentQueuedPromptControlProjection, ProjectionMetadata,
};
use crate::agent::AgentState;
use crate::app::{ActivePromptState, ActiveTurnPhase, ActiveTurnState, DaemonApp};
use crate::error::DaemonError;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::session::{PromptOrigin, PromptQueueItem, PromptStatus, RuntimeSession};

pub(crate) const SESSION_SNAPSHOT_PROJECTION_VERSION: u64 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshotProjection {
    pub metadata: ProjectionMetadata,
    pub session: RuntimeSession,
    pub provider_run: Option<RuntimeProviderRun>,
    pub agent_activity: BTreeMap<String, AgentRuntimeActivity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeStatus {
    Idle,
    Working,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPromptRuntimeStatus {
    None,
    Queued,
    Dispatching,
    Running,
    Cancelling,
    Settling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnRuntimePhase {
    Accepted,
    AwaitingFirstOutput,
    Streaming,
    Settling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeActivity {
    pub status: AgentRuntimeStatus,
    pub prompt_status: AgentPromptRuntimeStatus,
    pub busy: bool,
    pub active_prompt_count: usize,
    pub queued_prompt_count: usize,
    #[serde(default)]
    pub unread_idle_output: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub queued_prompt_controls: BTreeMap<String, AgentQueuedPromptControlProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<AgentActiveTurnProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_turn: Option<crate::git_observer::CompletedGitTurnActionProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActiveTurnProjection {
    pub prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_origin: Option<PromptOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_provider_turn_id: Option<String>,
    pub status: AgentPromptRuntimeStatus,
    pub phase: AgentTurnRuntimePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
}

impl SessionSnapshotProjection {
    pub fn from_daemon_app(
        app: &mut DaemonApp,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<Self, DaemonError> {
        Self::from_daemon_app_for_user(app, session_id, last_event_id, None)
    }

    pub fn from_daemon_app_for_user(
        app: &mut DaemonApp,
        session_id: &str,
        last_event_id: u64,
        unread_for_user_id: Option<&str>,
    ) -> Result<Self, DaemonError> {
        let mut session = app.sessions().get_session(session_id)?;
        let agents = app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        app.project_session_runtime_view(&mut session);
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| {
                app.providers()
                    .get_run(provider_run_id)
                    .ok()
                    .or_else(|| app.provider_run_projection_store().get(provider_run_id))
            });
        let prompt_activity = app.prompt_activity_store();
        let prompt_activity = prompt_activity.read();
        let active_turns = app.active_turn_store().snapshot();
        let completed_git_turn_snapshots = app.completed_git_turn_snapshot_store();
        let agent_activity = agent_activity_for_session_projection(
            &session,
            |agent_id| {
                app.providers()
                    .get_run_for_agent(session.id(), agent_id)
                    .or_else(|| {
                        app.provider_run_projection_store()
                            .get_for_agent(session.id(), agent_id)
                    })
            },
            &prompt_activity,
            &active_turns,
            unread_for_user_id,
            |agent_id| {
                completed_git_turn_snapshots.latest_projection_for_agent(session.id(), agent_id)
            },
        );
        Ok(Self {
            metadata: ProjectionMetadata::new(SESSION_SNAPSHOT_PROJECTION_VERSION, last_event_id),
            session,
            provider_run,
            agent_activity,
        })
    }
}

pub(crate) fn agent_activity_for_session_projection(
    session: &RuntimeSession,
    provider_run_for_agent: impl Fn(&str) -> Option<RuntimeProviderRun>,
    prompt_activity: &BTreeMap<String, ActivePromptState>,
    active_turns: &BTreeMap<String, ActiveTurnState>,
    unread_for_user_id: Option<&str>,
    completed_turn_for_agent: impl Fn(
        &str,
    )
        -> Option<crate::git_observer::CompletedGitTurnActionProjection>,
) -> BTreeMap<String, AgentRuntimeActivity> {
    let mut activity = BTreeMap::new();

    for agent in session.agents() {
        let prompt_state = session.prompt_states().get(agent.id());
        let active_prompt = prompt_state.and_then(|state| state.active_prompt());
        let queued_prompt_count = prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0);
        let provider_run = provider_run_for_agent(agent.id());
        let provider_turn_activity =
            active_turn_for_session_agent(active_turns, session.id(), agent.id());
        let provider_prompt_activity = provider_turn_activity
            .and_then(|turn| prompt_activity.get(&turn.provider_run_id))
            .or_else(|| {
                provider_run
                    .as_ref()
                    .and_then(|run| prompt_activity.get(run.id()))
            });
        let prompt_status = match active_prompt.map(PromptQueueItem::status) {
            Some(PromptStatus::Cancelling) => AgentPromptRuntimeStatus::Cancelling,
            Some(PromptStatus::Dispatching) => AgentPromptRuntimeStatus::Dispatching,
            Some(PromptStatus::Running) => {
                let settlement_requested = provider_turn_activity
                    .map(|state| state.settlement_requested)
                    .or_else(|| provider_prompt_activity.map(|state| state.settlement_requested))
                    .unwrap_or(false);
                if settlement_requested {
                    AgentPromptRuntimeStatus::Settling
                } else {
                    AgentPromptRuntimeStatus::Running
                }
            }
            Some(PromptStatus::Queued) => AgentPromptRuntimeStatus::Queued,
            Some(PromptStatus::Completed) | Some(PromptStatus::Cancelled) | None => {
                if provider_turn_activity.is_some_and(|state| state.settlement_requested) {
                    AgentPromptRuntimeStatus::Settling
                } else if provider_turn_activity.is_some() {
                    AgentPromptRuntimeStatus::Running
                } else if queued_prompt_count > 0 {
                    AgentPromptRuntimeStatus::Queued
                } else {
                    AgentPromptRuntimeStatus::None
                }
            }
        };
        let provider_busy = provider_turn_activity.is_some()
            && provider_run.as_ref().map_or(true, |run| {
                matches!(
                    run.state(),
                    ProviderRunState::Starting | ProviderRunState::Running
                )
            });
        let active_turn = provider_turn_activity
            .map(|turn| {
                let active_prompt_for_turn = active_prompt
                    .filter(|prompt| prompt_matches_active_turn(prompt, &turn.prompt_id));
                let prompt_origin = active_prompt_for_turn
                    .map(PromptQueueItem::prompt_origin)
                    .or(turn.prompt_origin);
                let external_observed_id = active_prompt_for_turn
                    .and_then(PromptQueueItem::external_observed_id)
                    .or_else(|| turn.external_observed_id.clone());
                active_turn_projection(
                    turn.prompt_id.clone(),
                    Some(turn.provider_run_id.clone()),
                    active_prompt_for_turn
                        .map(|prompt| prompt.source_attachment_id().to_string())
                        .or_else(|| turn.source_attachment_id.clone()),
                    prompt_origin,
                    external_observed_id,
                    prompt_status.clone(),
                    AgentTurnRuntimePhase::from(&turn.phase),
                    Some(turn.started_at_ms),
                )
            })
            .or_else(|| {
                active_prompt.map(|prompt| {
                    active_turn_projection(
                        prompt.id().to_string(),
                        provider_run.as_ref().map(|run| run.id().to_string()),
                        Some(prompt.source_attachment_id().to_string()),
                        Some(prompt.prompt_origin()),
                        prompt.external_observed_id(),
                        prompt_status.clone(),
                        AgentTurnRuntimePhase::Accepted,
                        None,
                    )
                })
            });
        let prompt_busy = agent_prompt_runtime_status_is_active_prompt(&prompt_status);
        let agent_busy =
            agent.is_processing() || agent.state() == AgentState::Working || provider_busy;
        let status = if agent.state() == AgentState::Error {
            AgentRuntimeStatus::Error
        } else if prompt_busy || agent_busy {
            AgentRuntimeStatus::Working
        } else {
            AgentRuntimeStatus::Idle
        };
        let active_prompt_count = usize::from(
            agent_prompt_runtime_status_is_active_prompt(&prompt_status) || active_turn.is_some(),
        );
        activity.insert(
            agent.id().to_string(),
            AgentRuntimeActivity {
                busy: status == AgentRuntimeStatus::Working,
                active_prompt_count,
                queued_prompt_count,
                unread_idle_output: status == AgentRuntimeStatus::Idle
                    && unread_for_user_id.is_some_and(|user_id| {
                        session.agent_has_unread_output(user_id, agent.id())
                    }),
                queued_prompt_controls: queued_prompt_controls_projection(
                    prompt_state,
                    active_turn.as_ref().and_then(|turn| turn.prompt_origin),
                ),
                status,
                prompt_status,
                active_turn,
                last_completed_turn: completed_turn_for_agent(agent.id()),
            },
        );
    }

    activity
}

fn active_turn_for_session_agent<'a>(
    active_turns: &'a BTreeMap<String, ActiveTurnState>,
    session_id: &str,
    agent_id: &str,
) -> Option<&'a ActiveTurnState> {
    active_turns
        .values()
        .filter(|turn| turn.session_id == session_id && turn.agent_id == agent_id)
        .max_by(|left, right| {
            left.started_at_ms
                .cmp(&right.started_at_ms)
                .then_with(|| left.provider_run_id.cmp(&right.provider_run_id))
        })
}

fn prompt_matches_active_turn(prompt: &PromptQueueItem, active_turn_prompt_id: &str) -> bool {
    prompt.id() == active_turn_prompt_id
        || prompt.pending_prompt_id() == Some(active_turn_prompt_id)
}

fn agent_prompt_runtime_status_is_active_prompt(status: &AgentPromptRuntimeStatus) -> bool {
    matches!(
        status,
        AgentPromptRuntimeStatus::Running
            | AgentPromptRuntimeStatus::Dispatching
            | AgentPromptRuntimeStatus::Cancelling
            | AgentPromptRuntimeStatus::Settling
    )
}

fn active_turn_projection(
    prompt_id: String,
    provider_run_id: Option<String>,
    source_attachment_id: Option<String>,
    prompt_origin: Option<PromptOrigin>,
    external_observed_id: Option<crate::history::ExternalProviderObservedId>,
    status: AgentPromptRuntimeStatus,
    phase: AgentTurnRuntimePhase,
    started_at_ms: Option<u64>,
) -> AgentActiveTurnProjection {
    let external = external_observed_id;
    AgentActiveTurnProjection {
        prompt_id,
        provider_run_id,
        source_attachment_id,
        prompt_origin,
        external_provider: external.as_ref().map(|metadata| metadata.provider.clone()),
        external_provider_session_id: external
            .as_ref()
            .map(|metadata| metadata.provider_session_id.clone()),
        external_provider_turn_id: external.map(|metadata| metadata.provider_turn_id),
        status,
        phase,
        started_at_ms,
    }
}

impl From<&ActiveTurnPhase> for AgentTurnRuntimePhase {
    fn from(value: &ActiveTurnPhase) -> Self {
        match value {
            ActiveTurnPhase::Accepted => Self::Accepted,
            ActiveTurnPhase::AwaitingFirstOutput => Self::AwaitingFirstOutput,
            ActiveTurnPhase::Streaming => Self::Streaming,
            ActiveTurnPhase::Settling => Self::Settling,
        }
    }
}

#[cfg(test)]
mod tests;
