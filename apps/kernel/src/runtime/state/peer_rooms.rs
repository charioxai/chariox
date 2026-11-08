//! Peers across the rooms of one owner on this kernel: any agent that is neither above nor
//! below the caller. A peer is addressed in its room; sub-agents stay hidden inside their tree.
use super::*;
use crate::runtime::room_tool_admission::{denied, resolve_agent};

/// Unknown rooms, other owners' rooms and hidden sub-agents give the same answer, so a peer
/// learns nothing about what it cannot address.
fn unavailable() -> DaemonError {
    denied("agent is unavailable in that room")
}

impl KernelRuntimeState {
    /// The message target: in the caller's room by default, or an ordinary agent of another
    /// room of the same owner when `room` names one.
    pub(crate) fn resolve_message_target(
        &self,
        session: &crate::session::RuntimeSession,
        sender: &crate::agent::AgentInstance,
        room: Option<&str>,
        reference: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let reference = reference.trim_start_matches('@');
        let room = room
            .map(str::trim)
            .filter(|room| !room.is_empty())
            .unwrap_or(session.id());
        if room == session.id() {
            return resolve_agent(&self.session_agents(room), room, reference).cloned();
        }
        let other = self
            .owned
            .session_store
            .get_session(room)
            .map_err(|_| unavailable())?;
        if other.owner_user_id() != sender.owner_user_id() {
            return Err(unavailable());
        }
        let agents = self.session_agents(room);
        let target = resolve_agent(&agents, room, reference).map_err(|_| unavailable())?;
        if target.parent_agent_id().is_some() || target.archived() {
            return Err(unavailable());
        }
        Ok(target.clone())
    }
}
