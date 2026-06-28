use std::collections::{HashMap, HashSet};

use super::{calculate_agent_layout, AgentInstance, AgentState};

#[derive(Debug, Default, Clone)]
pub struct AgentStore {
    agents: HashMap<String, AgentInstance>,
    agent_ids_by_session: HashMap<String, Vec<String>>,
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

    pub(crate) fn next_agent_ids(&mut self, count: usize) -> Vec<String> {
        if count == 0 {
            return Vec::new();
        }
        let start = self.next_id + 1;
        self.next_id += count as u64;
        (start..=self.next_id)
            .map(|id| format!("agent-{id}"))
            .collect()
    }

    pub fn insert(&mut self, agent: AgentInstance) -> AgentInstance {
        self.insert_owned(agent.clone());
        agent
    }

    pub(crate) fn insert_session_batch_and_apply_layout(
        &mut self,
        session_id: &str,
        mut agents: Vec<AgentInstance>,
        focused_agent_id: Option<&str>,
    ) -> Vec<AgentInstance> {
        let existing_agent_ids = self
            .session_agent_ids(session_id)
            .cloned()
            .unwrap_or_default();
        let positions = calculate_agent_layout(existing_agent_ids.len() + agents.len());

        for (index, agent_id) in existing_agent_ids.iter().enumerate() {
            if let Some(agent) = self.agents.get_mut(agent_id) {
                if let Some(position) = positions.get(index) {
                    agent.set_position(position.clone());
                }
                agent.set_state(if focused_agent_id == Some(agent.id()) {
                    AgentState::Focused
                } else {
                    AgentState::Idle
                });
            }
        }

        self.agents.reserve(agents.len());
        let session_index = self
            .agent_ids_by_session
            .entry(session_id.to_string())
            .or_default();
        session_index.reserve(agents.len());

        let existing_count = existing_agent_ids.len();
        let mut stored_agents = Vec::with_capacity(agents.len());
        for (index, mut agent) in agents.drain(..).enumerate() {
            debug_assert_eq!(agent.session_id(), session_id);
            if let Some(position) = positions.get(existing_count + index) {
                agent.set_position(position.clone());
            }
            agent.set_state(if focused_agent_id == Some(agent.id()) {
                AgentState::Focused
            } else {
                AgentState::Idle
            });
            let agent_id = agent.id().to_string();
            if !session_index.iter().any(|existing| existing == &agent_id) {
                session_index.push(agent_id.clone());
            }
            stored_agents.push(agent.clone());
            self.agents.insert(agent_id, agent);
        }
        stored_agents
    }

    fn insert_owned(&mut self, agent: AgentInstance) {
        let agent_id = agent.id().to_string();
        let existing_session_id = self
            .agents
            .get(&agent_id)
            .map(|existing| existing.session_id().to_string());
        if existing_session_id
            .as_deref()
            .is_some_and(|session_id| session_id != agent.session_id())
        {
            if let Some(session_id) = existing_session_id {
                self.remove_session_index_entry(&session_id, &agent_id);
            }
        }
        let session_index = self
            .agent_ids_by_session
            .entry(agent.session_id().to_string())
            .or_default();
        if !session_index.iter().any(|existing| existing == &agent_id) {
            session_index.push(agent_id.clone());
        }
        self.agents.insert(agent_id, agent);
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
        let removed = self.agents.remove(agent_id)?;
        self.remove_session_index_entry(removed.session_id(), agent_id);
        Some(removed)
    }

    pub fn get_by_ref(&self, agent_ref: &str) -> Option<&AgentInstance> {
        self.agents
            .values()
            .find(|agent| agent.agent_ref() == agent_ref)
    }

    pub fn get_by_session(&self, session_id: &str) -> Vec<AgentInstance> {
        self.session_agent_ids(session_id)
            .into_iter()
            .flatten()
            .filter_map(|agent_id| self.agents.get(agent_id).cloned())
            .collect()
    }

    pub(crate) fn apply_session_layout_and_focus(
        &mut self,
        session_id: &str,
        focused_agent_id: Option<&str>,
    ) {
        let agent_ids = self
            .session_agent_ids(session_id)
            .cloned()
            .unwrap_or_default();
        let positions = calculate_agent_layout(agent_ids.len());
        for (index, agent_id) in agent_ids.iter().enumerate() {
            if let Some(agent) = self.agents.get_mut(agent_id) {
                if let Some(position) = positions.get(index) {
                    agent.set_position(position.clone());
                }
                let next_state = if focused_agent_id == Some(agent.id()) {
                    AgentState::Focused
                } else {
                    AgentState::Idle
                };
                agent.set_state(next_state);
            }
        }
    }

    pub fn list(&self) -> Vec<AgentInstance> {
        let mut agents = self.agents.values().cloned().collect::<Vec<_>>();
        agents.sort_by(|left, right| left.id().cmp(right.id()));
        agents
    }

    pub fn count_by_session(&self, session_id: &str) -> usize {
        self.session_agent_ids(session_id).map_or(0, Vec::len)
    }

    pub(crate) fn session_summary(&self, session_id: &str) -> AgentSessionSummary {
        let mut summary = AgentSessionSummary::default();
        let Some(agent_ids) = self.session_agent_ids(session_id) else {
            return summary;
        };
        summary.count = agent_ids.len();
        for agent_id in agent_ids {
            if let Some(agent) = self.agents.get(agent_id) {
                if let Some(alias) = agent.alias() {
                    summary.aliases.insert(alias.to_lowercase());
                }
            }
        }
        summary
    }

    pub fn focused_agent(&self, session_id: &str) -> Option<&AgentInstance> {
        self.session_agent_ids(session_id)?
            .iter()
            .filter_map(|agent_id| self.agents.get(agent_id))
            .find(|agent| agent.state() == AgentState::Focused)
    }

    pub fn remove_by_session(&mut self, session_id: &str) -> Vec<AgentInstance> {
        let to_remove = self
            .agent_ids_by_session
            .remove(session_id)
            .unwrap_or_default();
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

    fn session_agent_ids(&self, session_id: &str) -> Option<&Vec<String>> {
        self.agent_ids_by_session.get(session_id)
    }

    fn remove_session_index_entry(&mut self, session_id: &str, agent_id: &str) {
        let should_remove_session =
            if let Some(agent_ids) = self.agent_ids_by_session.get_mut(session_id) {
                agent_ids.retain(|id| id != agent_id);
                agent_ids.is_empty()
            } else {
                false
            };
        if should_remove_session {
            self.agent_ids_by_session.remove(session_id);
        }
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
    fn session_index_updates_when_agent_moves_or_is_removed() {
        let mut store = AgentStore::new();
        store.insert(agent("agent-1", "session-a", Some("one")));
        store.insert(agent("agent-2", "session-a", Some("two")));
        store.insert(agent("agent-1", "session-b", Some("moved")));

        assert_eq!(store.count_by_session("session-a"), 1);
        assert_eq!(store.count_by_session("session-b"), 1);
        assert_eq!(store.session_summary("session-b").count, 1);
        assert!(store.session_summary("session-b").aliases.contains("moved"));

        let removed = store
            .remove("agent-1")
            .expect("moved agent should be removed");

        assert_eq!(removed.session_id(), "session-b");
        assert_eq!(store.count_by_session("session-b"), 0);
        assert!(store.get_by_session("session-b").is_empty());
    }

    #[test]
    fn remove_by_session_uses_and_clears_session_index() {
        let mut store = AgentStore::new();
        store.insert(agent("agent-1", "session-a", Some("one")));
        store.insert(agent("agent-2", "session-a", Some("two")));
        store.insert(agent("agent-3", "session-b", Some("three")));

        let removed = store.remove_by_session("session-a");

        assert_eq!(removed.len(), 2);
        assert_eq!(store.count_by_session("session-a"), 0);
        assert!(store.get("agent-1").is_none());
        assert!(store.get("agent-2").is_none());
        assert!(store.get("agent-3").is_some());
        assert_eq!(store.count_by_session("session-b"), 1);
    }

    #[test]
    fn session_index_preserves_creation_order_independent_of_positions() {
        let mut store = AgentStore::new();
        let mut second = agent("agent-2", "session-a", Some("two"));
        second.set_position(GridPosition::new(0, 0, 1, 1));
        let mut first = agent("agent-1", "session-a", Some("one"));
        first.set_position(GridPosition::new(9, 9, 1, 1));

        store.insert(first);
        store.insert(second);

        let session_agents = store.get_by_session("session-a");
        assert_eq!(
            session_agents
                .iter()
                .map(|agent| agent.id())
                .collect::<Vec<_>>(),
            vec!["agent-1", "agent-2"]
        );
    }

    #[test]
    fn replacing_agent_in_same_session_does_not_duplicate_session_index() {
        let mut store = AgentStore::new();
        store.insert(agent("agent-1", "session-a", Some("one")));
        store.insert(agent("agent-1", "session-a", Some("renamed")));

        let session_agents = store.get_by_session("session-a");
        assert_eq!(session_agents.len(), 1);
        assert_eq!(session_agents[0].alias(), Some("renamed"));
    }

    #[test]
    fn apply_session_layout_and_focus_updates_indexed_session_only() {
        let mut store = AgentStore::new();
        store.insert(agent("agent-1", "session-a", Some("one")));
        store.insert(agent("agent-2", "session-a", Some("two")));
        store.insert(agent("agent-3", "session-a", Some("three")));
        store.insert(agent("agent-4", "session-b", Some("four")));

        store.apply_session_layout_and_focus("session-a", Some("agent-2"));

        let session_agents = store.get_by_session("session-a");
        assert_eq!(
            session_agents
                .iter()
                .map(|agent| agent.id())
                .collect::<Vec<_>>(),
            vec!["agent-1", "agent-2", "agent-3"]
        );
        assert_eq!(session_agents[0].position(), &GridPosition::new(0, 0, 1, 1));
        assert_eq!(session_agents[1].position(), &GridPosition::new(0, 1, 1, 1));
        assert_eq!(session_agents[2].position(), &GridPosition::new(1, 0, 1, 1));
        assert_eq!(session_agents[0].state(), AgentState::Idle);
        assert_eq!(session_agents[1].state(), AgentState::Focused);
        assert_eq!(session_agents[2].state(), AgentState::Idle);

        let other_session_agent = store.get("agent-4").expect("agent should remain stored");
        assert_eq!(other_session_agent.state(), AgentState::Idle);
        assert_eq!(
            other_session_agent.position(),
            &GridPosition::new(0, 0, 1, 1)
        );
    }

    #[test]
    fn insert_session_batch_applies_layout_in_batch_order_without_resorting_new_agents() {
        let mut store = AgentStore::new();
        store.insert(agent("agent-1", "session-a", Some("existing")));

        let created = store.insert_session_batch_and_apply_layout(
            "session-a",
            vec![
                agent("agent-2", "session-a", Some("first")),
                agent("agent-10", "session-a", Some("second")),
            ],
            Some("agent-10"),
        );

        assert_eq!(
            created.iter().map(|agent| agent.id()).collect::<Vec<_>>(),
            vec!["agent-2", "agent-10"]
        );
        let session_agents = store.get_by_session("session-a");
        assert_eq!(
            session_agents
                .iter()
                .map(|agent| agent.id())
                .collect::<Vec<_>>(),
            vec!["agent-1", "agent-2", "agent-10"]
        );
        assert_eq!(session_agents[0].position(), &GridPosition::new(0, 0, 1, 1));
        assert_eq!(session_agents[1].position(), &GridPosition::new(0, 1, 1, 1));
        assert_eq!(session_agents[2].position(), &GridPosition::new(1, 0, 1, 1));
        assert_eq!(session_agents[2].state(), AgentState::Focused);
        assert_eq!(store.count_by_session("session-a"), 3);
    }
}
