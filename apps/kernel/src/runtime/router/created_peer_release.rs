//! Peers a room agent created in other rooms are deleted when that agent ends: the agent is
//! deleted, or its room ends or is deleted. A peer goes through the same delete as `agent delete`
//! (on another kernel, a delete request over the peer link), and its link stays until the delete
//! is done, so a restart or an unreachable kernel in between only delays it.
use super::CommandRouter;
use crate::durable_state::agent_lifecycle::created_peers::CreatedPeer;
use crate::error::DaemonError;
use crate::local::{DestroyAgentRequest, LocalDaemonRequest};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::state::peer_agent_inbound::peer_spawn_key;
use crate::transport::relay_peer::{
    RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};
use std::time::Duration;

/// How often the pass runs without a delete to trigger it: it finishes what a restart or an
/// unreachable kernel left.
const RELEASE_INTERVAL: Duration = Duration::from_secs(30);
/// A peer that cannot be deleted for this long is given up on, loudly: its kernel is gone.
const GIVE_UP_AFTER_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

impl CommandRouter {
    /// Runs for the life of the kernel.
    pub(crate) async fn run_created_peer_release(&self) {
        loop {
            self.finish_created_peers().await;
            tokio::time::sleep(RELEASE_INTERVAL).await;
        }
    }

    /// An agent or room that ended may have left peers behind: delete them now, without making
    /// the caller of the delete wait for another kernel.
    pub(super) fn finish_created_peers_in_background(&self) {
        let router = self.clone();
        tokio::spawn(async move { router.finish_created_peers().await });
    }

    /// Deletes the peers of every creator that has ended. Returns how many links it finished.
    pub(crate) async fn finish_created_peers(&self) -> usize {
        let _one_pass = self
            .runtime_state
            .created_peer_release_lock()
            .lock_owned()
            .await;
        let links = match self.runtime_state.durable_state_store().created_peers() {
            Ok(links) => links,
            Err(error) => {
                tracing::warn!(%error, "created peers: the links could not be read; retried on the next pass");
                return 0;
            }
        };
        let mut finished = 0;
        for link in links {
            if !self.runtime_state.created_peer_creator_ended(&link) {
                continue;
            }
            if let Err(error) = self.delete_created_peer(&link).await {
                let age = crate::session::unix_epoch_ms().saturating_sub(link.created_at_ms);
                if age < GIVE_UP_AFTER_MS {
                    tracing::warn!(%error, creator = %link.creator_agent, kernel = %link.kernel, peer = %link.peer,
                        "created peers: the peer of an ended agent is not deleted yet; retried on the next pass");
                    continue;
                }
                tracing::error!(%error, creator = %link.creator_agent, kernel = %link.kernel, peer = %link.peer,
                    "created peers: giving up on the peer of an ended agent");
            }
            match self.runtime_state.release_created_peer(link) {
                Ok(()) => finished += 1,
                Err(error) => {
                    tracing::warn!(%error, "created peers: the link could not be released; retried on the next pass")
                }
            }
        }
        finished
    }

    /// The delete itself; a peer that is already gone is a finished delete.
    async fn delete_created_peer(&self, link: &CreatedPeer) -> Result<(), DaemonError> {
        if link.kernel.is_empty() {
            return self.destroy_created_agent(&link.peer).await;
        }
        self.runtime_state.delete_created_peer_on_kernel(link).await
    }

    /// `agent delete` for an agent whose creator ended: the owner's own delete, so the room, its
    /// clients and the audit trail see an ordinary deletion.
    async fn destroy_created_agent(&self, agent_id: &str) -> Result<(), DaemonError> {
        let Some(agent) = self.runtime_state.agent(agent_id) else {
            return Ok(());
        };
        let request = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: agent.session_id().into(),
            agent_id: agent_id.into(),
        });
        let source = KernelCommandSource::DaemonBackground;
        let caller = KernelCaller {
            user_id: Some(agent.owner_user_id().to_string()),
            ..KernelCaller::for_source(&source)
        };
        let command = KernelCommand::from_local_request_with_caller(
            format!("created-peer-delete-{agent_id}"),
            source,
            caller,
            None,
            None,
            &request,
        );
        match Box::pin(self.dispatch(command, request)).await {
            Ok(_)
            | Err(
                DaemonError::AgentNotFound { .. }
                | DaemonError::AgentNotInSession { .. }
                | DaemonError::SessionNotFound { .. },
            ) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// A creator on another kernel ended: delete the agent its spawn request created. The agent
    /// is found by the spawn's key, whatever its room is called now. A spawn that has not created
    /// it yet is answered first (it holds the claim) or cancelled: this delete is remembered as
    /// the spawn's answer, so the spawn that comes later creates nothing.
    pub(super) async fn delete_room_peer(
        &self,
        kernel: &str,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let RelayPeerRequest::DeleteRoomPeer {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            request_id,
            sender: _,
        } = request
        else {
            return Err(crate::durable_state::agent_lifecycle::error(
                "peer agent protocol version mismatch",
            ));
        };
        if request_id.trim().is_empty() || request_id.len() > 128 {
            return Err(crate::durable_state::agent_lifecycle::error(
                "invalid peer request id",
            ));
        }
        let _in_flight = self
            .runtime_state
            .claim_peer_spawn_when_free(kernel, &request_id)
            .await;
        let deleted = |duplicate| RelayPeerResponse::RoomPeerDeleted {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            duplicate,
        };
        let spawn_key = peer_spawn_key(kernel, &request_id);
        let Some(peer) = self
            .runtime_state
            .list_agents()
            .into_iter()
            .find(|agent| agent.spawn_key() == Some(spawn_key.as_str()))
        else {
            self.runtime_state
                .durable_state_store()
                .remember_peer_agent_request_answer(
                    kernel,
                    &request_id,
                    &serde_json::to_string(&deleted(true)).map_err(|error| {
                        crate::durable_state::agent_lifecycle::error(error.to_string())
                    })?,
                )?;
            return Ok(deleted(true));
        };
        if let Err(refusal) = self
            .runtime_state
            .peer_room_of_owner(owner, peer.session_id())
        {
            return Ok(RelayPeerResponse::PeerAgentRefused {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                refusal,
            });
        }
        self.destroy_created_agent(peer.id()).await?;
        Ok(deleted(false))
    }
}
