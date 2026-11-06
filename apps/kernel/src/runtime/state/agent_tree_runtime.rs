//! `ListDescendants`: the sub-agents below one agent, read from the room.
use super::*;
use crate::local::{ListDescendantsRequest, LocalDaemonResponse};

impl KernelRuntimeState {
    pub(crate) fn list_descendants(
        &self,
        caller_user_id: &str,
        request: ListDescendantsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let session = self.owned.session_store.get_session(&request.session_id)?;
        if !self.is_room_member(&session, caller_user_id) {
            return Err(DaemonError::SessionAccessDenied {
                session_id: request.session_id,
                user_id: caller_user_id.to_string(),
            });
        }
        let store = &self.owned.durable_state_store;
        let tree = crate::runtime::agent_tree_projection::project_agent_tree(
            &session,
            &self
                .owned
                .agent_store
                .get_session_agents(&request.session_id),
            &request.agent_id,
            crate::agent::current_tree_rev(),
            &|agent_id| {
                store
                    .agent_deadline(&request.session_id, agent_id)
                    .ok()
                    .flatten()
                    .map(|deadline| deadline.deadline_ms)
            },
        )
        .ok_or_else(|| DaemonError::AgentNotInSession {
            session_id: request.session_id.clone(),
            agent_id: request.agent_id.clone(),
        })?;
        Ok(LocalDaemonResponse::AgentTree { tree })
    }
}
