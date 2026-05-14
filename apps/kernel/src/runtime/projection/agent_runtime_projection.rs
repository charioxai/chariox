use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};

use crate::session::{PromptQueueItem, RuntimeSession};

use super::AgentRuntimeProjectionHealthSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeProjection {
    pub session_id: String,
    pub agent_id: String,
    pub active_prompt: Option<PromptQueueItem>,
    pub next_queued_prompt: Option<PromptQueueItem>,
    pub queued_prompt_count: usize,
}

#[derive(Clone, Default)]
pub(crate) struct AgentRuntimeProjectionStore {
    agents: Arc<StdMutex<HashMap<String, AgentRuntimeProjection>>>,
}

impl AgentRuntimeProjectionStore {
    pub(crate) fn get(&self, agent_id: &str) -> Option<AgentRuntimeProjection> {
        self.agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .get(agent_id)
            .cloned()
    }

    pub(crate) fn list(&self) -> Vec<AgentRuntimeProjection> {
        let mut projections = self
            .agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        projections.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| left.agent_id.cmp(&right.agent_id))
        });
        projections
    }

    pub(crate) fn list_for_session(&self, session_id: &str) -> Vec<AgentRuntimeProjection> {
        self.list()
            .into_iter()
            .filter(|projection| projection.session_id == session_id)
            .collect()
    }

    pub(crate) fn next_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<PromptQueueItem> {
        self.get(agent_id)
            .filter(|projection| projection.session_id == session_id)
            .and_then(|projection| projection.next_queued_prompt)
    }

    pub(crate) fn update_session(&self, session: &RuntimeSession) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned");
        agents.retain(|_, projection| projection.session_id != session.id());
        for agent in session.agents() {
            let prompt_state = session.prompt_states().get(agent.id());
            agents.insert(
                agent.id().to_string(),
                AgentRuntimeProjection {
                    session_id: session.id().to_string(),
                    agent_id: agent.id().to_string(),
                    active_prompt: prompt_state.and_then(|state| state.active_prompt().cloned()),
                    next_queued_prompt: prompt_state
                        .and_then(|state| state.queued_prompts().front().cloned()),
                    queued_prompt_count: prompt_state
                        .map(|state| state.queued_prompts().len())
                        .unwrap_or(0),
                },
            );
        }
        for (agent_id, prompt_state) in session.prompt_states() {
            agents
                .entry(agent_id.clone())
                .or_insert_with(|| AgentRuntimeProjection {
                    session_id: session.id().to_string(),
                    agent_id: agent_id.clone(),
                    active_prompt: prompt_state.active_prompt().cloned(),
                    next_queued_prompt: prompt_state.queued_prompts().front().cloned(),
                    queued_prompt_count: prompt_state.queued_prompts().len(),
                });
        }
    }

    #[cfg(test)]
    pub(crate) fn update_agent_from_session(&self, session: &RuntimeSession, agent_id: &str) {
        let mut agents = self
            .agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned");
        let Some(projection) = agent_runtime_projection_from_session(session, agent_id) else {
            agents.remove(agent_id);
            return;
        };
        agents.insert(agent_id.to_string(), projection);
    }

    #[cfg(test)]
    pub(crate) fn update_agent_prompt_state(
        &self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<PromptQueueItem>,
        next_queued_prompt: Option<PromptQueueItem>,
        queued_prompt_count: usize,
    ) {
        self.agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .insert(
                agent_id.to_string(),
                AgentRuntimeProjection {
                    session_id: session_id.to_string(),
                    agent_id: agent_id.to_string(),
                    active_prompt,
                    next_queued_prompt,
                    queued_prompt_count,
                },
            );
    }

    pub(crate) fn remove_session(&self, session_id: &str) {
        self.agents
            .lock()
            .expect("agent runtime projection lock should not be poisoned")
            .retain(|_, projection| projection.session_id != session_id);
    }

    pub(crate) fn health_snapshot(&self) -> AgentRuntimeProjectionHealthSnapshot {
        let agents = self.list();
        AgentRuntimeProjectionHealthSnapshot {
            projected_agents: agents.len(),
            active_prompts: agents
                .iter()
                .filter(|projection| projection.active_prompt.is_some())
                .count(),
            queued_prompts: agents
                .iter()
                .map(|projection| projection.queued_prompt_count)
                .sum(),
        }
    }
}

#[cfg(test)]
fn agent_runtime_projection_from_session(
    session: &RuntimeSession,
    agent_id: &str,
) -> Option<AgentRuntimeProjection> {
    if !session.agents().iter().any(|agent| agent.id() == agent_id)
        && !session.prompt_states().contains_key(agent_id)
    {
        return None;
    }
    let prompt_state = session.prompt_states().get(agent_id);
    Some(AgentRuntimeProjection {
        session_id: session.id().to_string(),
        agent_id: agent_id.to_string(),
        active_prompt: prompt_state.and_then(|state| state.active_prompt().cloned()),
        next_queued_prompt: prompt_state.and_then(|state| state.queued_prompts().front().cloned()),
        queued_prompt_count: prompt_state
            .map(|state| state.queued_prompts().len())
            .unwrap_or(0),
    })
}
