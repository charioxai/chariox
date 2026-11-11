//! Peers across kernels: a room agent on a kernel of the same owner creates an ordinary agent in
//! a room here, through the same spawn command a room agent of this kernel uses.
use super::CommandRouter;
use crate::error::DaemonError;
use crate::local::{LocalDaemonRequest, LocalDaemonResponse, SpawnAgentRequest};
use crate::runtime::command::{KernelCaller, KernelCommand, KernelCommandSource};
use crate::runtime::state::peer_agent_inbound::peer_spawn_key;
use crate::transport::relay_peer::{
    PeerAgentRefusal, RelayPeerRequest, RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

fn refused(refusal: PeerAgentRefusal) -> RelayPeerResponse {
    RelayPeerResponse::PeerAgentRefused {
        protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        refusal,
    }
}

impl CommandRouter {
    /// One `request_id` of one sending kernel creates at most one agent: a retry gets the
    /// first answer back, marked as a duplicate. A retry that overlaps the first attempt waits
    /// for it, and the new agent carries the request as its spawn key, so a retry after a
    /// restart that lost the answer finds that agent instead of creating another.
    pub(super) async fn spawn_room_peer(
        &self,
        kernel: &str,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let RelayPeerRequest::SpawnRoomPeer {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            request_id,
            sender,
            room,
            provider,
            account_profile,
            model,
            effort,
            alias,
            prompt,
            execution_mode,
            permission_level,
            caller_authority,
            follow,
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
        if follow.as_ref().is_some_and(|follow| {
            follow.subscription_id.trim().is_empty() || follow.subscription_id.len() > 128
        }) {
            return Err(crate::durable_state::agent_lifecycle::error(
                "invalid peer subscription id",
            ));
        }
        let store = self.runtime_state.durable_state_store();
        let _in_flight = self
            .runtime_state
            .claim_peer_spawn_when_free(kernel, &request_id)
            .await;
        if let Some(answer) = store.peer_agent_request_answer(kernel, &request_id)? {
            let mut answer: RelayPeerResponse = serde_json::from_str(&answer)
                .map_err(|error| crate::durable_state::agent_lifecycle::error(error.to_string()))?;
            // The creator ended before this spawn ran, and its delete cancelled it.
            if matches!(answer, RelayPeerResponse::RoomPeerDeleted { .. }) {
                return Ok(refused(PeerAgentRefusal::RoomUnavailable));
            }
            if let RelayPeerResponse::RoomPeerSpawned {
                duplicate, agent, ..
            } = &mut answer
            {
                *duplicate = true;
                // The same call from a later turn of the caller brings that turn's own follow.
                if let Some(follow) = follow {
                    self.runtime_state.follow_spawned_room_peer(
                        kernel,
                        owner,
                        follow,
                        &agent.room_id,
                        &agent.agent_id,
                    )?;
                }
            }
            return Ok(answer);
        }
        let (room, authority) = match self.runtime_state.admit_room_peer_spawn(
            owner,
            &room,
            caller_authority,
            execution_mode,
            permission_level,
        ) {
            Ok(admitted) => admitted,
            Err(refusal) => return Ok(refused(refusal)),
        };
        let spawn_key = peer_spawn_key(kernel, &request_id);
        let created_before = self
            .runtime_state
            .session_agents(&room)
            .iter()
            .any(|agent| agent.spawn_key() == Some(spawn_key.as_str()));
        // The owner as this room knows it: `local` in a room made before enrollment.
        let creator = self.runtime_state.room_member_user_id(&room, owner);
        let spawn = LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            no_subscribe: true,
            session_id: room,
            alias,
            provider: Some(provider),
            account_profile,
            model,
            effort,
            execution_mode: Some(authority.mode),
            permission_level: Some(authority.permission_level),
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        });
        // The owner both kernels share creates it; the room's own folder and limits apply.
        let source = KernelCommandSource::DaemonBackground;
        let caller = KernelCaller {
            user_id: Some(creator),
            ..KernelCaller::for_source(&source)
        };
        let mut command = KernelCommand::from_local_request_with_caller(
            spawn_key.clone(),
            source,
            caller,
            None,
            None,
            &spawn,
        );
        command.idempotency_key = Some(spawn_key);
        let agent = match Box::pin(self.dispatch(command, spawn)).await {
            Ok(LocalDaemonResponse::AgentSpawned { agent, .. }) => agent,
            Ok(_) => {
                return Err(crate::durable_state::agent_lifecycle::error(
                    "peer spawn answered without an agent",
                ))
            }
            Err(DaemonError::AgentLimitReached { .. }) => {
                return Ok(refused(PeerAgentRefusal::RoomFull))
            }
            Err(error) => return Err(error),
        };
        if let Some(follow) = follow {
            self.runtime_state.follow_spawned_room_peer(
                kernel,
                owner,
                follow,
                agent.session_id(),
                agent.id(),
            )?;
        }
        // The first task reaches the new peer as a message from its creator, never as a prompt;
        // it lands before the answer is kept, so a retry after a lost answer delivers it once.
        if let Some(prompt) = prompt.filter(|prompt| !prompt.trim().is_empty()) {
            self.runtime_state
                .admit_peer_agent_message(
                    kernel,
                    &format!("{request_id}:prompt"),
                    &sender,
                    &agent,
                    prompt,
                    false,
                    false,
                )
                .await?;
        }
        let mut answer = RelayPeerResponse::RoomPeerSpawned {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            duplicate: false,
            agent: self.runtime_state.peer_agent_stamp(&agent),
            alias: agent.alias().unwrap_or(agent.agent_ref()).to_string(),
        };
        store.remember_peer_agent_request_answer(
            kernel,
            &request_id,
            &serde_json::to_string(&answer)
                .map_err(|error| crate::durable_state::agent_lifecycle::error(error.to_string()))?,
        )?;
        // An agent a lost answer left behind is this request's earlier attempt.
        if let RelayPeerResponse::RoomPeerSpawned { duplicate, .. } = &mut answer {
            *duplicate = created_before;
        }
        Ok(answer)
    }
}
