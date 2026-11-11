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
        let placement = self.owned.place_agent(caller_user_id, &request)?;
        self.append_agent_durable_event("agent.updated", &placement.agent, None)
            .await?;
        self.finish_moved_decisions(&request.session_id, placement.moved, placement.announce)
            .await?;
        // The moved team's existing local runs start their next turn within the new parent chain.
        self.limit_local_runs(
            "set agent parent",
            &request.session_id,
            placement.local,
            "place the agent again to retry",
        )
        .await?;
        // A worker launches a leased agent's next turn with what it was last told, so every leased
        // agent of the moved team is told the limit of its new parent chain.
        self.tell_workers_leased_limits(
            "set agent parent",
            &request.session_id,
            placement.leased,
            "place the agent again to retry",
        )
        .await?;
        Ok(LocalDaemonResponse::AgentParentChanged {
            agent: placement.agent,
        })
    }
}

struct AgentPlacement {
    agent: crate::agent::AgentInstance,
    moved: usize,
    announce: Vec<String>,
    /// The leased agents of the moved team, by home agent id.
    leased: Vec<String>,
    /// The local agents of the moved team.
    local: Vec<String>,
}

impl KernelRuntimeOwnedState {
    /// Checks the new edge, sets it and moves the open decisions of the moved subtree as one step
    /// under the interaction lock, so no decision is raised or answered against a half-moved tree.
    fn place_agent(
        &self,
        caller_user_id: &str,
        request: &SetAgentParentRequest,
    ) -> Result<AgentPlacement, DaemonError> {
        let _mutation =
            self.pending_interactions
                .mutation
                .lock()
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "runtime interaction",
                    message: "Interaction store is unavailable".into(),
                })?;
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
        let agent = find(&request.agent_id)?;
        match request.parent_agent_id.as_deref() {
            Some(parent_id) => {
                let parent = find(parent_id)?;
                // A parent answers its children's decisions with its own authority, so the caller
                // places only under an agent it acts for; another user's agent reads as unknown.
                if !self.acts_for_owner(&request.session_id, caller_user_id, parent.owner_user_id())
                {
                    return Err(DaemonError::AgentNotInSession {
                        session_id: request.session_id.clone(),
                        agent_id: parent_id.to_owned(),
                    });
                }
                let session = self.session_store.get_session(&request.session_id)?;
                let unfinished_work = self
                    .durable_state_store
                    .agents_with_unfinished_work(&request.session_id)?;
                crate::agent::authority_limit::check_placement(
                    agent,
                    parent,
                    &agents,
                    &session,
                    &unfinished_work,
                )
                .map_err(crate::runtime::room_tool_admission::denied)?;
            }
            // A sub-agent made a room agent counts toward the room's limit, as a new one does.
            None if agent.is_sub_agent() => {
                let max_agents = self
                    .session_store
                    .get_session(&request.session_id)?
                    .max_agents();
                let room_agents = agents
                    .iter()
                    .filter(|other| !other.archived() && !other.is_sub_agent())
                    .count();
                if room_agents >= max_agents as usize {
                    return Err(DaemonError::AgentLimitReached {
                        session_id: request.session_id.clone(),
                        max_agents,
                    });
                }
            }
            None => {}
        }
        let agent = self.agent_store.set_parent_agent(
            &request.agent_id,
            request.parent_agent_id.clone().map(AgentRef::local),
        )?;
        let (moved, announce) = self.move_subtree_decisions(&request.session_id, agent.id())?;
        let leased = self.leased_subtree(&request.session_id, agent.id());
        let local = self.local_subtree(&request.session_id, agent.id());
        Ok(AgentPlacement {
            agent,
            moved,
            announce,
            leased,
            local,
        })
    }
}

#[cfg(test)]
#[path = "agent_parent_runtime_tests.rs"]
mod tests;
