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
    if !session.agents().iter().any(|agent| agent.id() == agent_id) {
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

#[cfg(test)]
mod tests {
    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::runtime::projection::test_support::{launch_dev_stub_provider, submit_prompt};
    use crate::runtime::projection::AgentRuntimeProjectionStore;
    use crate::session::CreateSessionRequest;
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn agent_runtime_projection_reads_agent_prompt_state() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-agent-runtime-projection",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "first prompt",
        );
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "queued prompt",
        );

        let session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        let store = AgentRuntimeProjectionStore::default();
        store.update_session(&session);

        let projection = store
            .get(&agent_id)
            .expect("agent projection should be available");
        assert_eq!(projection.session_id, session_id);
        assert_eq!(projection.agent_id, agent_id);
        assert!(projection.active_prompt.is_some());
        assert_eq!(
            projection
                .next_queued_prompt
                .as_ref()
                .map(|prompt| prompt.prompt()),
            Some("queued prompt")
        );
        assert_eq!(projection.queued_prompt_count, 1);
        assert_eq!(
            store
                .next_queued_prompt(&projection.session_id, &projection.agent_id)
                .as_ref()
                .map(|prompt| prompt.prompt()),
            Some("queued prompt")
        );
        assert_eq!(
            store.list_for_session(&projection.session_id),
            vec![projection]
        );
        assert_eq!(store.health_snapshot().active_prompts, 1);
        assert_eq!(store.health_snapshot().queued_prompts, 1);
    }

    #[test]
    fn agent_runtime_projection_can_refresh_one_agent_without_stomping_peers() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, first_agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let first_agent_id = first_agent.id().to_string();
        let second_agent_id = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(CreateAgentRequest::new(&session_id, "claude-code").with_alias("peer"))
            .expect("second agent should spawn")
            .id()
            .to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-agent-runtime-one-agent-refresh",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        for agent_id in [&first_agent_id, &second_agent_id] {
            launch_dev_stub_provider(&mut app, &session_id, agent_id);
        }

        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &first_agent_id,
            "first active",
        );
        let first_only_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("first snapshot should load");
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &second_agent_id,
            "second active",
        );
        let both_snapshot = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("second snapshot should load");

        let store = AgentRuntimeProjectionStore::default();
        store.update_session(&both_snapshot);
        assert!(store
            .get(&second_agent_id)
            .and_then(|projection| projection.active_prompt)
            .is_some());

        store.update_agent_from_session(&first_only_snapshot, &first_agent_id);
        assert!(
            store
                .get(&second_agent_id)
                .and_then(|projection| projection.active_prompt)
                .is_some(),
            "single-agent refresh should not erase newer peer prompt state"
        );
    }

    #[test]
    fn agent_runtime_projection_ignores_prompt_state_without_session_agent() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                &session_id,
                "cli-agent-runtime-prompt-state-only",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attach should succeed");
        launch_dev_stub_provider(&mut app, &session_id, &agent_id);
        submit_prompt(
            &mut app,
            &session_id,
            attachment.id(),
            &agent_id,
            "ghost prompt",
        );
        let mut session = crate::app::KernelSessionReadService::new(&app)
            .session_snapshot(&session_id)
            .expect("session snapshot should load");
        assert!(
            session.prompt_states().contains_key(&agent_id),
            "fixture should retain prompt state before removing projected agents"
        );
        session.set_agents(Vec::new());

        let store = AgentRuntimeProjectionStore::default();
        store.update_session(&session);

        assert_eq!(store.get(&agent_id), None);
        assert!(store.list_for_session(&session_id).is_empty());
        store.update_agent_from_session(&session, &agent_id);
        assert_eq!(store.get(&agent_id), None);
    }
}
