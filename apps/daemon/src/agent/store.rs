use std::collections::HashMap;

use super::{AgentInstance, AgentState};

#[derive(Debug, Default, Clone)]
pub struct AgentStore {
    agents: HashMap<String, AgentInstance>,
    next_id: u64,
}

impl AgentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn next_agent_id(&mut self) -> String {
        self.next_id += 1;
        format!("agent-{}", self.next_id)
    }

    pub fn insert(&mut self, agent: AgentInstance) -> AgentInstance {
        self.agents.insert(agent.id().to_string(), agent.clone());
        agent
    }

    pub fn get(&self, agent_id: &str) -> Option<&AgentInstance> {
        self.agents.get(agent_id)
    }

    pub fn get_mut(&mut self, agent_id: &str) -> Option<&mut AgentInstance> {
        self.agents.get_mut(agent_id)
    }

    pub fn remove(&mut self, agent_id: &str) -> Option<AgentInstance> {
        self.agents.remove(agent_id)
    }

    pub fn get_by_ref(&self, agent_ref: &str) -> Option<&AgentInstance> {
        self.agents
            .values()
            .find(|agent| agent.agent_ref() == agent_ref)
    }

    pub fn get_by_session(&self, session_id: &str) -> Vec<AgentInstance> {
        self.agents
            .values()
            .filter(|agent| agent.session_id() == session_id)
            .cloned()
            .collect()
    }

    pub fn count_by_session(&self, session_id: &str) -> usize {
        self.agents
            .values()
            .filter(|agent| agent.session_id() == session_id)
            .count()
    }

    pub fn focused_agent(&self, session_id: &str) -> Option<&AgentInstance> {
        self.agents
            .values()
            .find(|agent| agent.session_id() == session_id && agent.state() == AgentState::Focused)
    }

    pub fn remove_by_session(&mut self, session_id: &str) -> Vec<AgentInstance> {
        let to_remove: Vec<String> = self
            .agents
            .values()
            .filter(|agent| agent.session_id() == session_id)
            .map(|agent| agent.id().to_string())
            .collect();

        to_remove
            .into_iter()
            .filter_map(|id| self.agents.remove(&id))
            .collect()
    }

    pub fn all(&self) -> impl Iterator<Item = &AgentInstance> {
        self.agents.values()
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
}
