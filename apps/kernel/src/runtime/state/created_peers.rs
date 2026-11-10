//! Peers a room agent created in other rooms end with it. This module keeps the link between a
//! creator and each peer it made, and says whether the creator has ended; the router deletes
//! the peers (`router/created_peer_release.rs`).
use super::*;
use crate::durable_state::agent_lifecycle::created_peers::CreatedPeer;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};
use crate::error::DaemonError;
use crate::session::SessionStatus;

impl KernelRuntimeState {
    /// `creator` made `peer` in another room of this kernel.
    pub(crate) fn record_created_local_peer(
        &self,
        creator_agent: &str,
        peer: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let creator = self.owned.agent_store.get_agent(creator_agent)?;
        self.record_created_peer(CreatedPeer {
            creator_room: creator.session_id().into(),
            creator_agent: creator.id().into(),
            kernel: String::new(),
            peer: peer.id().into(),
            created_at_ms: 0,
        })
        .map(|_| ())
    }

    /// Whether this call wrote the link: an earlier attempt of the same spawn may have, and its
    /// peer may exist.
    pub(crate) fn record_created_peer(&self, mut peer: CreatedPeer) -> Result<bool, DaemonError> {
        peer.created_at_ms = crate::session::unix_epoch_ms();
        match self
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::RecordCreatedPeer { peer })?
        {
            Outcome::CreatedPeerRecorded { inserted } => Ok(inserted),
            _ => Err(crate::durable_state::agent_lifecycle::error(
                "created peer recorded without an outcome",
            )),
        }
    }

    pub(crate) fn release_created_peer(&self, peer: CreatedPeer) -> Result<(), DaemonError> {
        self.owned
            .durable_state_store
            .agent_lifecycle(Operation::ReleaseCreatedPeer { peer })
            .map(|_| ())
    }

    /// Whether the creator is gone for good: its room ended or was deleted, or the agent was
    /// deleted. An agent that merely is not in memory yet counts as present until its deletion
    /// is durable, so a restart never deletes the peers of an agent that is still there.
    pub(crate) fn created_peer_creator_ended(&self, link: &CreatedPeer) -> bool {
        match self.owned.session_store.get_session(&link.creator_room) {
            Err(DaemonError::SessionNotFound { .. }) => return true,
            Err(_) => return false,
            Ok(session) if session.status() == SessionStatus::Ended => return true,
            Ok(_) => {}
        }
        if self
            .owned
            .agent_store
            .get_agent(&link.creator_agent)
            .is_ok()
        {
            return false;
        }
        self.owned
            .durable_state_store
            .agent_deletion_recorded(&link.creator_agent)
            .unwrap_or(false)
    }

    pub(crate) fn agent(&self, agent_id: &str) -> Option<crate::agent::AgentInstance> {
        self.owned.agent_store.get_agent(agent_id).ok()
    }

    /// The room of `owner` that `room` names here, as a peer request from another kernel
    /// addresses it.
    pub(crate) fn peer_room_of_owner(
        &self,
        owner: &str,
        room: &str,
    ) -> Result<String, crate::transport::relay_peer::PeerAgentRefusal> {
        self.peer_room(owner, room)
            .map(|room| room.id().to_string())
    }

    /// One release pass at a time on this kernel.
    pub(crate) fn created_peer_release_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.created_peer_release.clone()
    }
}
