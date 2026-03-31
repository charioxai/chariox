use crate::error::DaemonError;
use crate::session::{SessionService, SessionStatus};

use super::{
    calculate_agent_layout, generate_agent_ref, recalculate_positions, AgentInstance, AgentState,
    AgentStore, CreateAgentRequest, GridPosition,
};

#[derive(Debug, Clone)]
pub struct AgentService {
    store: AgentStore,
}

impl AgentService {
    pub fn new() -> Self {
        Self {
            store: AgentStore::new(),
        }
    }

    /// Create a new agent in a session
    pub fn create_agent(
        &mut self,
        request: CreateAgentRequest,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        // Validate session exists and is not ended
        let session = sessions.get_session(&request.session_id)?;

        if session.status() == SessionStatus::Ended {
            return Err(DaemonError::SessionOperationNotAllowed {
                session_id: request.session_id.clone(),
                status: session.status(),
                operation: "create agent",
            });
        }

        // Check max agents limit
        let current_count = self.store.count_by_session(&request.session_id);
        if current_count >= session.max_agents() as usize {
            return Err(DaemonError::AgentLimitReached {
                session_id: request.session_id.clone(),
                max_agents: session.max_agents(),
            });
        }

        // Validate alias uniqueness within session
        if let Some(ref alias) = request.alias {
            if self.is_alias_taken(&request.session_id, alias) {
                return Err(DaemonError::AgentAliasConflict {
                    session_id: request.session_id.clone(),
                    alias: alias.clone(),
                });
            }
        }

        // Calculate position for new agent
        let position = self.calculate_position_for_new_agent(&request.session_id);

        // Create agent
        let agent_ref = generate_agent_ref();
        let agent = AgentInstance::new(
            self.store.next_agent_id(),
            agent_ref,
            request.session_id,
            request.alias,
            request.provider,
            request.model,
            request.worktree_id,
            position,
        );
        let agent_id = agent.id().to_string();
        let session_id = agent.session_id().to_string();

        self.store.insert(agent);

        // Recalculate all positions
        let mut session_agents = self.store.get_by_session(&session_id);
        recalculate_positions(&mut session_agents);

        // Update stored positions
        for agent in &session_agents {
            if let Some(stored) = self.store.get_mut(agent.id()) {
                stored.set_position(agent.position().clone());
            }
        }

        // Focus the newly created agent so the next prompt targets it.
        for agent in &session_agents {
            if let Some(stored) = self.store.get_mut(agent.id()) {
                let next_state = if agent.id() == agent_id {
                    AgentState::Focused
                } else {
                    AgentState::Idle
                };
                stored.set_state(next_state);
            }
        }
        sessions.set_focused_agent(&session_id, Some(agent_id.clone()))?;

        Ok(self
            .store
            .get(&agent_id)
            .cloned()
            .expect("new agent should be stored"))
    }

    /// Create default agent for a new session
    pub fn create_default_agent(
        &mut self,
        session_id: &str,
        worktree_id: &str,
        provider: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        let request = CreateAgentRequest::new(session_id, provider).with_worktree(worktree_id);

        self.create_agent(request, sessions)
    }

    /// Destroy an agent
    pub fn destroy_agent(
        &mut self,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        let agent =
            self.store
                .get(agent_id)
                .cloned()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: agent_id.to_string(),
                })?;

        let session_id = agent.session_id().to_string();
        let was_focused = agent.state() == AgentState::Focused;

        // Remove the agent
        self.store.remove(agent_id);

        // Recalculate positions for remaining agents
        let mut remaining_agents: Vec<_> =
            self.store.get_by_session(&session_id).into_iter().collect();

        if !remaining_agents.is_empty() {
            recalculate_positions(&mut remaining_agents);

            // Update stored positions
            for agent in &remaining_agents {
                if let Some(stored) = self.store.get_mut(agent.id()) {
                    stored.set_position(agent.position().clone());
                }
            }

            // If the destroyed agent was focused, focus the first remaining agent
            if was_focused {
                if let Some(first) = remaining_agents.first() {
                    if let Some(stored) = self.store.get_mut(first.id()) {
                        stored.set_state(AgentState::Focused);
                    }
                    sessions.set_focused_agent(&session_id, Some(first.id().to_string()))?;
                }
            }
        } else {
            // No agents left, clear focused agent
            sessions.set_focused_agent(&session_id, None)?;
        }

        Ok(agent)
    }

    /// Focus an agent (tap navigation)
    pub fn focus_agent(
        &mut self,
        session_id: &str,
        agent_id: &str,
        sessions: &mut SessionService,
    ) -> Result<AgentInstance, DaemonError> {
        let agents: Vec<_> = self.store.get_by_session(session_id);

        // Validate agent exists in session
        let _target_agent = agents
            .iter()
            .find(|a| a.id() == agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            })?;

        // Unfocus all other agents in session
        for agent in &agents {
            if agent.id() != agent_id && agent.state() == AgentState::Focused {
                if let Some(stored) = self.store.get_mut(agent.id()) {
                    stored.set_state(AgentState::Idle);
                }
            }
        }

        // Focus target agent
        if let Some(stored) = self.store.get_mut(agent_id) {
            stored.set_state(AgentState::Focused);
        }

        sessions.set_focused_agent(session_id, Some(agent_id.to_string()))?;

        Ok(self.store.get(agent_id).cloned().unwrap())
    }

    /// Get next agent in session (for tap navigation)
    pub fn get_next_agent_in_session(
        &self,
        session_id: &str,
        current_agent_id: &str,
    ) -> Option<AgentInstance> {
        let agents = self.store.get_by_session(session_id);

        if let Some(current_index) = agents.iter().position(|a| a.id() == current_agent_id) {
            let next_index = (current_index + 1) % agents.len();
            agents.get(next_index).cloned()
        } else {
            agents.first().cloned()
        }
    }

    /// Cycle focus to next agent (tap navigation)
    pub fn cycle_focus(
        &mut self,
        session_id: &str,
        sessions: &mut SessionService,
    ) -> Result<Option<AgentInstance>, DaemonError> {
        let agents = self.store.get_by_session(session_id);

        if agents.is_empty() {
            return Ok(None);
        }

        // Find currently focused agent
        let current_focused = agents
            .iter()
            .find(|a| a.state() == AgentState::Focused)
            .map(|a| a.id().to_string());

        let next_agent_id = if let Some(current_id) = current_focused {
            self.get_next_agent_in_session(session_id, &current_id)
                .map(|a| a.id().to_string())
        } else {
            agents.first().map(|a| a.id().to_string())
        };

        if let Some(next_id) = next_agent_id {
            let agent = self.focus_agent(session_id, &next_id, sessions)?;
            Ok(Some(agent))
        } else {
            Ok(None)
        }
    }

    /// Update agent state
    pub fn set_agent_state(
        &mut self,
        agent_id: &str,
        state: AgentState,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;

        agent.set_state(state);
        Ok(agent.clone())
    }

    /// Set agent processing state
    pub fn set_agent_processing(
        &mut self,
        agent_id: &str,
        is_processing: bool,
    ) -> Result<AgentInstance, DaemonError> {
        let agent = self
            .store
            .get_mut(agent_id)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })?;

        agent.set_processing(is_processing);
        Ok(agent.clone())
    }

    /// Get agent by ID
    pub fn get_agent(&self, agent_id: &str) -> Result<AgentInstance, DaemonError> {
        self.store
            .get(agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })
    }

    /// Get agent by reference
    pub fn get_agent_by_ref(&self, agent_ref: &str) -> Result<AgentInstance, DaemonError> {
        self.store
            .get_by_ref(agent_ref)
            .cloned()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: agent_ref.to_string(),
            })
    }

    /// Get all agents in a session
    pub fn get_session_agents(&self, session_id: &str) -> Vec<AgentInstance> {
        self.store.get_by_session(session_id)
    }

    /// Get focused agent in session
    pub fn get_focused_agent(&self, session_id: &str) -> Option<AgentInstance> {
        self.store.focused_agent(session_id).cloned()
    }

    /// Remove all agents for a session (called when session ends)
    pub fn remove_session_agents(&mut self, session_id: &str) -> Vec<AgentInstance> {
        self.store.remove_by_session(session_id)
    }

    fn calculate_position_for_new_agent(&self, session_id: &str) -> GridPosition {
        let current_count = self.store.count_by_session(session_id);
        let positions = calculate_agent_layout(current_count + 1);

        positions
            .get(current_count)
            .cloned()
            .unwrap_or_else(|| GridPosition::new(0, 0, 1, 1))
    }

    fn is_alias_taken(&self, session_id: &str, alias: &str) -> bool {
        let normalized = alias.trim().to_lowercase();
        self.store.get_by_session(session_id).iter().any(|agent| {
            agent
                .alias()
                .map(|a| a.to_lowercase())
                .map(|a| a == normalized)
                .unwrap_or(false)
        })
    }

    pub fn store(&self) -> &AgentStore {
        &self.store
    }
}
