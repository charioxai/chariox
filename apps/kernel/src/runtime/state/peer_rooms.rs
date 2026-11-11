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
        if self.drives_as_room_guest(session, sender) {
            return Err(unavailable());
        }
        self.resolve_peer_agent(sender.owner_user_id(), room, reference)
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

    /// A guest of a room is a member that is not its owner under any identity: it owns the
    /// agent, or it started the agent's running turn. What a guest drives reaches only the agents
    /// of that room.
    pub(crate) fn drives_as_room_guest(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
    ) -> bool {
        !self.acts_for_room_owner(session, agent.owner_user_id())
            || self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(session, agent.id())
                .and_then(|prompt| prompt.source_user_id().map(str::to_owned))
                .is_some_and(|user| !self.acts_for_room_owner(session, &user))
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
        let target = resolve_agent(&agents, room, reference).map_err(|_| unavailable())?;
        if target.parent_agent_id().is_some() || target.archived() {
            return Err(unavailable());
        }
        Ok(target.clone())
    }
}

impl KernelRuntimeState {
    /// The turn that spawned `peer` in another room follows it: each of the peer's turn ends
    /// reaches the spawner as a peer event while this task waits on the returned registration.
    pub(crate) fn follow_spawned_peer(
        &self,
        spawner: &crate::agent::AgentInstance,
        peer: &crate::agent::AgentInstance,
    ) -> Result<String, DaemonError> {
        use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Registration};
        let session = self.owned.session_store.get_session(spawner.session_id())?;
        let prompt = session
            .active_prompt_for_agent(spawner.id())
            .ok_or_else(|| ledger::error("no live creating turn"))?;
        let task = self
            .owned
            .durable_state_store
            .agent_tasks(Some(spawner.session_id()), Some(spawner.id()))?
            .into_iter()
            .find(|task| task.prompt_id == prompt.id())
            .ok_or_else(|| ledger::error("no live creating turn"))?;
        let id = format!("follow-{}", peer.id());
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::Subscribe {
                task: task.task_id.clone(),
                prompt: prompt.id().into(),
                registration: Registration {
                    id: id.clone(),
                    task_id: task.task_id,
                    source_id: peer.id().into(),
                    obligation_id: None,
                    source_cursor: 0,
                    live: true,
                    peer: true,
                },
            })?;
        Ok(id)
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
    pub(crate) fn authorize_peer_control(
        &self,
        actor: &crate::agent::AgentInstance,
        target: &crate::agent::AgentInstance,
        control: PeerControl,
    ) -> Result<(), DaemonError> {
        if target.session_id() == actor.session_id() {
            let agents = self.session_agents(actor.session_id());
            let actor_above = crate::agent::authority_limit::ancestors(target, &agents)
                .iter()
                .any(|agent| agent.id() == actor.id());
            let actor_below = crate::agent::authority_limit::ancestors(actor, &agents)
                .iter()
                .any(|agent| agent.id() == target.id());
            if actor_above || (actor_below && control == PeerControl::Steer) {
                return Ok(());
            }
            if actor_below {
                return crate::runtime::room_tool_admission::in_subtree(actor, target, &agents);
            }
        }
        let target_session = self
            .owned
            .session_store
            .get_session(target.session_id())
            .map_err(|_| unavailable())?;
        if !self.same_owner(target_session.owner_user_id(), actor.owner_user_id())
            || (target.session_id() != actor.session_id() && target.parent_agent_id().is_some())
        {
            return Err(unavailable());
        }
        let actor_session = self.owned.session_store.get_session(actor.session_id())?;
        let mine = crate::session::effective_agent_execution_config_in_tree(
            &actor_session,
            actor,
            &self.session_agents(actor.session_id()),
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
        let theirs = crate::session::effective_agent_execution_config_in_tree(
            target_session,
            target,
            &self.session_agents(target.session_id()),
        );
        if mine.permission_level.permits(theirs.permission_level) && mine.mode.permits(theirs.mode)
        {
            return Ok(());
        }
        Err(theirs)
    }
}
