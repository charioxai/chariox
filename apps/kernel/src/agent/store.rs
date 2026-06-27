use std::collections::{HashMap, HashSet};

use super::{AgentInstance, AgentState};

#[derive(Debug, Default, Clone)]
pub struct AgentStore {
    agents: HashMap<String, AgentInstance>,
    next_id: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionSummary {
    pub(crate) count: usize,
    pub(crate) aliases: HashSet<String>,
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

    pub(crate) fn insert_many(&mut self, agents: Vec<AgentInstance>) {
        self.agents.reserve(agents.len());
        for agent in agents {
            self.agents.insert(agent.id().to_string(), agent);
        }
    }

    pub fn insert_restored(&mut self, agent: AgentInstance) -> AgentInstance {
        if let Some(number) = agent
            .id()
            .strip_prefix("agent-")
            .and_then(|value| value.parse::<u64>().ok())
        {
            self.next_id = self.next_id.max(number);
        }
        self.insert(agent)
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
        let mut agents = self
            .agents
            .values()
            .filter(|agent| agent.session_id() == session_id)
            .cloned()
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| {
            left.position()
                .row
                .cmp(&right.position().row)
                .then_with(|| left.position().col.cmp(&right.position().col))
                .then_with(|| left.created_at_ms().cmp(&right.created_at_ms()))
                .then_with(|| left.id().cmp(right.id()))
        });
        agents
    }

    pub fn list(&self) -> Vec<AgentInstance> {
        let mut agents = self.agents.values().cloned().collect::<Vec<_>>();
        agents.sort_by(|left, right| left.id().cmp(right.id()));
        agents
    }

    pub fn count_by_session(&self, session_id: &str) -> usize {
        self.agents
            .values()
            .filter(|agent| agent.session_id() == session_id)
            .count()
    }

    pub(crate) fn session_summary(&self, session_id: &str) -> AgentSessionSummary {
        let mut summary = AgentSessionSummary::default();
        for agent in self
            .agents
            .values()
            .filter(|agent| agent.session_id() == session_id)
        {
            summary.count += 1;
            if let Some(alias) = agent.alias() {
                summary.aliases.insert(alias.to_lowercase());
            }
        }
        summary
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::GridPosition;

    fn agent(id: &str, session_id: &str, alias: Option<&str>) -> AgentInstance {
        AgentInstance::new(
            id,
            format!("ref-{id}"),
            session_id,
            alias.map(str::to_string),
            "codex",
            None,
            None,
            None,
            GridPosition::new(0, 0, 1, 1),
        )
    }

    #[test]
    fn session_summary_counts_agents_and_normalizes_aliases_without_sorting() {
        let mut store = AgentStore::new();
        store.insert(agent("agent-1", "session-a", Some("Alpha")));
        store.insert(agent("agent-2", "session-a", Some("BETA")));
        store.insert(agent("agent-3", "session-a", None));
        store.insert(agent("agent-4", "session-b", Some("alpha")));

        let summary = store.session_summary("session-a");

        assert_eq!(summary.count, 3);
        assert!(summary.aliases.contains("alpha"));
        assert!(summary.aliases.contains("beta"));
        assert_eq!(summary.aliases.len(), 2);
    }

    #[test]
    fn insert_many_stores_batch_without_recloning_inputs() {
        let mut store = AgentStore::new();
        store.insert_many(vec![
            agent("agent-1", "session-a", Some("one")),
            agent("agent-2", "session-a", Some("two")),
        ]);

        assert_eq!(store.len(), 2);
        assert!(store.get("agent-1").is_some());
        assert!(store.get("agent-2").is_some());
    }
}
