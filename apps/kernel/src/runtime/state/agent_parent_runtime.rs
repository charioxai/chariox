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
        if let Some(parent_id) = request.parent_agent_id.as_deref() {
            let agents = self
                .owned
                .agent_store
                .get_session_agents(&request.session_id);
            let parent = agents
                .iter()
                .find(|candidate| candidate.id() == parent_id)
                .ok_or_else(|| DaemonError::AgentNotInSession {
                    session_id: request.session_id.clone(),
                    agent_id: parent_id.to_owned(),
                })?;
            // An agent under itself or under its own descendant would make a cycle.
            if parent.id() == agent.id()
                || crate::runtime::room_tool_admission::in_subtree(&agent, parent, &agents).is_ok()
            {
                return Err(crate::runtime::room_tool_admission::denied(
                    "an agent cannot be placed under itself or under one of its descendants",
                ));
            }
        }
        let agent = self.owned.agent_store.set_parent_agent(
            &request.agent_id,
            request.parent_agent_id.map(AgentRef::local),
        )?;
        self.append_agent_durable_event("agent.updated", &agent, None)
            .await?;
        Ok(LocalDaemonResponse::AgentParentChanged { agent })
    }
}
