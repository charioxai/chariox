//! The user moves an agent in the agent tree; no agent request reaches this.
use super::*;
use crate::agent::AgentRef;
use crate::local::{LocalDaemonResponse, SetAgentParentRequest};

impl KernelRuntimeState {
    pub(crate) async fn set_agent_parent(
        &self,
        caller_user_id: &str,
        request: SetAgentParentRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let agent = self
            .ensure_agent_owner(&request.agent_id, caller_user_id, "set agent parent")
            .await?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id,
                agent_id: request.agent_id,
            });
        }
        let (agent, moved, announce) = self.owned.place_agent(&request)?;
        self.append_agent_durable_event("agent.updated", &agent, None)
            .await?;
        self.finish_moved_decisions(&request.session_id, moved, announce)
            .await?;
        Ok(LocalDaemonResponse::AgentParentChanged { agent })
    }
}

impl KernelRuntimeOwnedState {
    /// Checks the new edge, sets it and moves the open decisions of the moved subtree as one step
    /// under the interaction lock, so no decision is raised or answered against a half-moved tree.
    fn place_agent(
        &self,
        request: &SetAgentParentRequest,
    ) -> Result<(crate::agent::AgentInstance, usize, Vec<String>), DaemonError> {
        let _mutation =
            self.pending_interactions
                .mutation
                .lock()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "runtime interaction",
                    message: "Interaction store is unavailable".into(),
                })?;
        if let Some(parent_id) = request.parent_agent_id.as_deref() {
            let agents = self.agent_store.get_session_agents(&request.session_id);
            let find = |id: &str| {
                agents
                    .iter()
                    .find(|candidate| candidate.id() == id)
                    .ok_or_else(|| DaemonError::AgentNotInSession {
                        session_id: request.session_id.clone(),
                        agent_id: id.to_owned(),
                    })
            };
            let (agent, parent) = (find(&request.agent_id)?, find(parent_id)?);
            crate::agent::authority_limit::check_placement(agent, parent, &agents)
                .map_err(crate::runtime::room_tool_admission::denied)?;
        }
        let agent = self.agent_store.set_parent_agent(
            &request.agent_id,
            request.parent_agent_id.clone().map(AgentRef::local),
        )?;
        let (moved, announce) = self.move_subtree_decisions(&request.session_id, agent.id())?;
        Ok((agent, moved, announce))
    }
}
