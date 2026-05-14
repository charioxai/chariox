use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ProjectionMetadata;
use crate::agent::AgentState;
use crate::app::{ActivePromptState, ActiveTurnState, DaemonApp};
use crate::error::DaemonError;
use crate::provider::{ProviderRunState, RuntimeProviderRun};
use crate::session::{PromptQueueItem, PromptStatus, RuntimeSession};

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
    Running,
    Cancelling,
    Settling,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeActivity {
    pub status: AgentRuntimeStatus,
    pub prompt_status: AgentPromptRuntimeStatus,
    pub busy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn: Option<AgentActiveTurnProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentActiveTurnProjection {
    pub prompt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_run_id: Option<String>,
    pub status: AgentPromptRuntimeStatus,
}

impl SessionSnapshotProjection {
    pub fn from_daemon_app(
        app: &mut DaemonApp,
        session_id: &str,
        last_event_id: u64,
    ) -> Result<Self, DaemonError> {
        let mut session = app.sessions().get_session(session_id)?;
        let agents = app.agents().get_session_agents(session_id);
        session.set_agents(agents);
        app.project_session_runtime_view(&mut session);
        let provider_run = session
            .active_provider_run_id()
            .and_then(|provider_run_id| app.providers().get_run(provider_run_id).ok());
        let prompt_activity = app.prompt_activity_store();
        let prompt_activity = prompt_activity.read();
        let active_turns = app.active_turn_store().snapshot();
        let agent_activity = agent_activity_for_session_projection(
            &session,
            |agent_id| app.providers().get_run_for_agent(session.id(), agent_id),
            &prompt_activity,
            &active_turns,
        );
        Ok(Self {
            metadata: ProjectionMetadata::new(2, last_event_id),
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
) -> BTreeMap<String, AgentRuntimeActivity> {
    let mut activity = BTreeMap::new();

    for agent in session.agents() {
        let prompt_state = session.prompt_states().get(agent.id());
        let active_prompt = prompt_state.and_then(|state| state.active_prompt());
        let queued_prompt_count = prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0);
        let provider_run = provider_run_for_agent(agent.id());
        let provider_prompt_activity = provider_run
            .as_ref()
            .and_then(|run| prompt_activity.get(run.id()));
        let provider_turn_activity = provider_run.as_ref().and_then(|run| {
            active_turns.get(run.id()).filter(|turn| {
                turn.session_id == session.id()
                    && turn.agent_id == agent.id()
                    && turn.provider_run_id == run.id()
            })
        });
        let prompt_status = match active_prompt.map(PromptQueueItem::status) {
            Some(PromptStatus::Cancelling) => AgentPromptRuntimeStatus::Cancelling,
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
        let provider_busy = provider_run.as_ref().is_some_and(|run| {
            matches!(
                run.state(),
                ProviderRunState::Starting | ProviderRunState::Running
            ) && provider_turn_activity.is_some()
        });
        let active_turn = active_prompt
            .map(|prompt| AgentActiveTurnProjection {
                prompt_id: prompt.id().to_string(),
                provider_run_id: provider_run.as_ref().map(|run| run.id().to_string()),
                status: prompt_status.clone(),
            })
            .or_else(|| {
                provider_turn_activity.map(|turn| AgentActiveTurnProjection {
                    prompt_id: turn.prompt_id.clone(),
                    provider_run_id: Some(turn.provider_run_id.clone()),
                    status: prompt_status.clone(),
                })
            });
        let prompt_busy = !matches!(prompt_status, AgentPromptRuntimeStatus::None);
        let agent_busy =
            agent.is_processing() || agent.state() == AgentState::Working || provider_busy;
        let status = if agent.state() == AgentState::Error {
            AgentRuntimeStatus::Error
        } else if prompt_busy || agent_busy {
            AgentRuntimeStatus::Working
        } else {
            AgentRuntimeStatus::Idle
        };
        activity.insert(
            agent.id().to_string(),
            AgentRuntimeActivity {
                busy: status == AgentRuntimeStatus::Working,
                status,
                prompt_status,
                active_turn,
            },
        );
    }

    activity
}
