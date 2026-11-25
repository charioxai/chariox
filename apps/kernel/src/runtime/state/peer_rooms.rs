//! Peers across the rooms of one owner on this kernel: any agent that is neither above nor
//! below the caller. A peer is addressed in its room. Row 405 (PA12): the owner's agents also name
//! the owner's sub-agents in another room; for guests and other owners they stay hidden in their tree.
use super::*;
use crate::runtime::room_tool_admission::{denied, resolve_agent};

/// Unknown rooms, other owners' rooms and hidden sub-agents give the same answer, so a peer
/// learns nothing about what it cannot address.
fn unavailable() -> DaemonError {
    denied("agent is unavailable in that room")
}

impl KernelRuntimeState {
    /// The message target: in the caller's room by default, or, when `room` names another room
    /// of the same owner, any of the owner's agents there (row 405, PA12: sub-agents too).
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
        if self.drives_as_room_guest(session, sender) {
            return Err(denied(
                "a guest of this room reaches only the agents in this room; ask the room's owner",
            ));
        }
        let room = self
            .owned
            .session_store
            .read()
            .resolve_session_ref(room, None)
            .map_err(|_| unavailable())?
            .id()
            .to_owned();
        let target =
            self.resolve_same_owner_room_agent(sender.owner_user_id(), &room, reference)?;
        // Row 405 (PA12): the caller is the owner's agent (not a guest) and the room is the
        // owner's, so the owner's sub-agents there are addressable as the owner can see them;
        // what it may then do is the peer check's (the permission comparison stays).
        if !target.archived() {
            return Ok(target);
        }
        Err(unavailable())
    }

    /// The rooms `--room` accepts for `caller` on this kernel: its own, marked current, and the
    /// other rooms of its owner. A guest of a room reaches no other room.
    pub(crate) fn reachable_rooms(
        &self,
        session: &crate::session::RuntimeSession,
        caller: &crate::agent::AgentInstance,
    ) -> Vec<serde_json::Value> {
        let guest = self.drives_as_room_guest(session, caller);
        let mut rooms = self
            .owned
            .session_store
            .list_sessions()
            .into_iter()
            .filter(|room| {
                room.id() == session.id()
                    || (!guest
                        && !room.is_hidden()
                        && room.status() != crate::session::SessionStatus::Ended
                        && self.same_owner(room.owner_user_id(), caller.owner_user_id()))
            })
            .collect::<Vec<_>>();
        rooms.sort_by(|a, b| a.id().cmp(b.id()));
        rooms
            .iter()
            .map(|room| {
                serde_json::json!({
                    "id": room.id(),
                    "name": room.alias(),
                    "current": room.id() == session.id(),
                })
            })
            .collect()
    }

    pub(crate) fn drives_as_room_guest(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
    ) -> bool {
        self.owned.drives_as_room_guest(session, agent)
    }

    /// An ordinary agent of a room of `owner`, the one address a peer has across rooms. Every
    /// other answer (another owner's room, a missing room, a sub-agent, an archived agent, an
    /// unknown or ambiguous name) is the same refusal.
    pub(crate) fn resolve_peer_agent(
        &self,
        owner: &str,
        room: &str,
        reference: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let target = self.resolve_same_owner_room_agent(owner, room, reference)?;
        if target.is_sub_agent() || target.archived() {
            return Err(unavailable());
        }
        Ok(target)
    }

    fn resolve_same_owner_room_agent(
        &self,
        owner: &str,
        room: &str,
        reference: &str,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let reference = reference.trim().trim_start_matches('@');
        let other = self
            .owned
            .session_store
            .get_session(room)
            .map_err(|_| unavailable())?;
        if !self.same_owner(other.owner_user_id(), owner) {
            return Err(unavailable());
        }
        let agents = self.session_agents(room);
        resolve_agent(&agents, room, reference)
            .cloned()
            .map_err(|_| unavailable())
    }
}

/// What a peer does to another agent's running turn.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerControl {
    Steer,
    Cancel,
}

impl KernelRuntimeState {
    /// Inside one tree the tree decides: the agents above an agent control it, and an agent
    /// only messages the agents above it. Any other agent of the same owner is a peer and may
    /// steer or cancel when its authority is at least the target's.
    /// Row 405: the owner's agents control any of the owner's agents: one above it, or a peer
    /// the peer check admits (same owner; the owner's per-agent permission comparison stays).
    pub(crate) fn authorize_agent_control(
        &self,
        actor: &crate::agent::AgentInstance,
        target: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        if actor.id() == target.id() {
            return Err(denied("only the agents above an agent can control it"));
        }
        if self.owned.agent_store.is_ancestor(actor.id(), target) {
            return Ok(());
        }
        self.authorize_peer_control(actor, target, PeerControl::Cancel)
    }

    pub(crate) fn authorize_peer_control(
        &self,
        actor: &crate::agent::AgentInstance,
        target: &crate::agent::AgentInstance,
        control: PeerControl,
    ) -> Result<(), DaemonError> {
        let store = &self.owned.agent_store;
        let actor_above = store.is_ancestor(actor.id(), target);
        let actor_below = store.is_ancestor(target.id(), actor);
        if actor_above || (actor_below && control == PeerControl::Steer) {
            if actor.session_id() != target.session_id() {
                let source = self.owned.session_store.get_session(actor.session_id())?;
                if self.drives_as_room_guest(&source, actor) {
                    return Err(unavailable());
                }
            }
            return Ok(());
        }
        if actor_below {
            return Err(denied("only the agents above an agent can control it"));
        }
        let target_session = self
            .owned
            .session_store
            .get_session(target.session_id())
            .map_err(|_| unavailable())?;
        // Row 405: any of the owner's agents, in any room.
        if !self.same_owner(target_session.owner_user_id(), actor.owner_user_id()) {
            return Err(unavailable());
        }
        let actor_session = self.owned.session_store.get_session(actor.session_id())?;
        let mine = self.owned.agent_store.effective_execution_config(
            &actor_session,
            actor,
            &self.owned.session_store,
        );
        self.peer_authority_covers(mine, &target_session, target)
            .map_err(|theirs| {
                denied(format!(
                    "{} has more authority than you ({:?} in {} mode, you run {:?} in {} mode); send it a message instead",
                    target.alias().unwrap_or(target.agent_ref()),
                    theirs.permission_level,
                    theirs.mode,
                    mine.permission_level,
                    mine.mode,
                ))
            })
    }

    /// The authority rule for a peer's steer or cancel: the caller's level and mode are at least
    /// the target's. Returns the target's authority when they are not.
    pub(crate) fn peer_authority_covers(
        &self,
        mine: crate::session::EffectiveAgentExecutionConfig,
        target_session: &crate::session::RuntimeSession,
        target: &crate::agent::AgentInstance,
    ) -> Result<(), crate::session::EffectiveAgentExecutionConfig> {
        let theirs = self.owned.agent_store.effective_execution_config(
            target_session,
            target,
            &self.owned.session_store,
        );
        if mine.permission_level.permits(theirs.permission_level) && mine.mode.permits(theirs.mode)
        {
            return Ok(());
        }
        Err(theirs)
    }
}

impl KernelRuntimeOwnedState {
    /// A guest of a room is a member that is not its owner under any identity: it owns the
    /// agent, or it started the agent's running turn. What a guest drives reaches only the agents
    /// of that room.
    pub(super) fn drives_as_room_guest(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
    ) -> bool {
        !self.acts_for_owner_in(
            session.owner_user_id(),
            agent.owner_user_id(),
            session.owner_user_id(),
        ) || self
            .prompt_state_owner
            .active_prompt_for_agent(session, agent.id())
            .and_then(|prompt| prompt.source_user_id().map(str::to_owned))
            .is_some_and(|user| {
                !self.acts_for_owner_in(session.owner_user_id(), &user, session.owner_user_id())
            })
    }
}
