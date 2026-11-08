//! Peers across kernels, receiving side: a same-owner kernel's agent messages, steers or cancels
//! an ordinary agent of a room here. Admission (owner, stamp) happened at the relay seam; this
//! module resolves the target with the one peer resolver and applies the same inbox and cancel
//! paths a peer in another room of this kernel uses.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation, Outcome};
use crate::transport::relay_peer::{
    PeerAgentAddress, PeerAgentRefusal, PeerAgentStamp, PeerCallerAuthority, RelayPeerRequest,
    RelayPeerResponse,
};

const VERSION: u32 = crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION;

fn refused(refusal: PeerAgentRefusal) -> RelayPeerResponse {
    RelayPeerResponse::PeerAgentRefused {
        protocol_version: VERSION,
        refusal,
    }
}

impl KernelRuntimeState {
    /// `kernel` is the authenticated sending kernel, `owner` the owner both kernels share.
    pub(crate) async fn receive_peer_agent_request(
        &self,
        kernel: &str,
        owner: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        match request {
            RelayPeerRequest::PeerAgentMessage {
                protocol_version: VERSION,
                request_id,
                sender,
                target,
                message,
                urgent,
                reply_requested,
                caller_authority,
            } => {
                let target = match self.peer_agent_target(owner, &target) {
                    Ok(target) => target,
                    Err(refusal) => return Ok(refused(refusal)),
                };
                if urgent {
                    if let Err(refusal) = self.peer_caller_covers(caller_authority, &target) {
                        return Ok(refused(refusal));
                    }
                }
                self.admit_peer_agent_message(
                    kernel,
                    &request_id,
                    &sender,
                    &target,
                    message,
                    urgent,
                    reply_requested,
                )
                .await
            }
            RelayPeerRequest::PeerAgentCancel {
                protocol_version: VERSION,
                request_id: _,
                sender: _,
                target,
                caller_authority,
            } => {
                let target = match self.peer_agent_target(owner, &target) {
                    Ok(target) => target,
                    Err(refusal) => return Ok(refused(refusal)),
                };
                if let Err(refusal) = self.peer_caller_covers(caller_authority, &target) {
                    return Ok(refused(refusal));
                }
                // As for a peer in another room: the kernel cancels through the target's own
                // delivery seat, so the caller never joins the room.
                let attachment =
                    self.ensure_agent_message_attachment(target.session_id(), &target)?;
                self.cancel_agent_prompt_with_external_authority(
                    target.session_id(),
                    target.id(),
                    &attachment,
                    None,
                )
                .await?;
                Ok(RelayPeerResponse::PeerAgentAccepted {
                    protocol_version: VERSION,
                    duplicate: false,
                    target: self.peer_agent_stamp(&target),
                })
            }
            RelayPeerRequest::SubscribePeerAgent { .. }
            | RelayPeerRequest::UnsubscribePeerAgent { .. }
            | RelayPeerRequest::DeliverPeerAgentOutcome { .. } => Err(ledger::error(
                "this kernel does not carry follow or outcome requests across kernels yet",
            )),
            RelayPeerRequest::PeerAgentMessage { .. }
            | RelayPeerRequest::PeerAgentCancel { .. } => {
                Err(ledger::error("peer agent protocol version mismatch"))
            }
            RelayPeerRequest::SpawnRoomPeer { .. } => Err(ledger::error(
                "a peer spawn from another kernel runs through the command router",
            )),
            _ => Err(ledger::error("not a peer agent request")),
        }
    }

    /// The one peer resolver, with a room this owner cannot address told apart from an agent
    /// that is not there: the caller already names a room of its own owner.
    fn peer_agent_target(
        &self,
        owner: &str,
        address: &PeerAgentAddress,
    ) -> Result<crate::agent::AgentInstance, PeerAgentRefusal> {
        let room = self
            .owned
            .session_store
            .get_session(&address.room)
            .map_err(|_| PeerAgentRefusal::RoomUnavailable)?;
        if room.owner_user_id() != owner {
            return Err(PeerAgentRefusal::RoomUnavailable);
        }
        self.resolve_peer_agent(owner, &address.room, &address.agent)
            .map_err(|_| PeerAgentRefusal::AgentUnavailable)
    }

    /// A peer spawn from another kernel: the room is one of `owner`'s, and the new agent gets
    /// what it asks for lowered to the caller's authority, itself lowered to this kernel's own
    /// record of the room.
    pub(crate) fn admit_room_peer_spawn(
        &self,
        owner: &str,
        room: &str,
        claimed: PeerCallerAuthority,
        mode: Option<crate::provider::AgentExecutionMode>,
        level: Option<crate::provider::AgentPermissionLevel>,
    ) -> Result<crate::session::EffectiveAgentExecutionConfig, PeerAgentRefusal> {
        let session = self
            .owned
            .session_store
            .get_session(room)
            .map_err(|_| PeerAgentRefusal::RoomUnavailable)?;
        if session.owner_user_id() != owner {
            return Err(PeerAgentRefusal::RoomUnavailable);
        }
        let record = crate::session::effective_agent_execution_config(&session, None);
        let mine = crate::session::EffectiveAgentExecutionConfig {
            mode: claimed.mode.lower(record.mode),
            permission_level: claimed.permission_level.lower(record.permission_level),
        };
        Ok(crate::session::EffectiveAgentExecutionConfig {
            mode: mode.unwrap_or(mine.mode).lower(mine.mode),
            permission_level: level
                .unwrap_or(mine.permission_level)
                .lower(mine.permission_level),
        })
    }

    /// The caller's authority, lowered to this kernel's own record of the target room, must
    /// cover the target's.
    fn peer_caller_covers(
        &self,
        claimed: PeerCallerAuthority,
        target: &crate::agent::AgentInstance,
    ) -> Result<(), PeerAgentRefusal> {
        let session = self
            .owned
            .session_store
            .get_session(target.session_id())
            .map_err(|_| PeerAgentRefusal::RoomUnavailable)?;
        let room = crate::session::effective_agent_execution_config(&session, None);
        let mine = crate::session::EffectiveAgentExecutionConfig {
            mode: claimed.mode.lower(room.mode),
            permission_level: claimed.permission_level.lower(room.permission_level),
        };
        self.peer_authority_covers(mine, &session, target)
            .map_err(|theirs| PeerAgentRefusal::MoreAuthority {
                mode: theirs.mode,
                permission_level: theirs.permission_level,
            })
    }

    pub(crate) fn peer_agent_stamp(&self, agent: &crate::agent::AgentInstance) -> PeerAgentStamp {
        PeerAgentStamp {
            kernel_id: self.owned.config_projection.snapshot().daemon_id,
            room_id: agent.session_id().into(),
            agent_id: agent.id().into(),
        }
    }

    /// A message event with a kernel-stamped sender, never a prompt. One `request_id` of one
    /// sending kernel is one delivery. A requested reply goes back as a message to the stamped
    /// sender on its kernel; nothing here waits for it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn admit_peer_agent_message(
        &self,
        kernel: &str,
        request_id: &str,
        sender: &PeerAgentStamp,
        target: &crate::agent::AgentInstance,
        message: String,
        urgent: bool,
        reply_requested: bool,
    ) -> Result<RelayPeerResponse, DaemonError> {
        if message.trim().is_empty() {
            return Err(ledger::error("message is empty"));
        }
        if request_id.trim().is_empty() || request_id.len() > 128 {
            return Err(ledger::error("invalid message occurrence"));
        }
        let room = target.session_id();
        let source = format!("peer:{kernel}:{}", sender.agent_id);
        let occurrence = format!("{kernel}:{request_id}");
        let duplicate = self.owned.durable_state_store.agent_inbox_has_occurrence(
            room,
            target.id(),
            &source,
            &occurrence,
        )?;
        let mut event = ledger::occurrence(
            room,
            target.id(),
            &source,
            &occurrence,
            "message",
            serde_json::json!({"message":message,"sender_agent_id":sender.agent_id,"sender_room_id":sender.room_id,"sender_kernel_id":kernel,"no_reply":!reply_requested,"attachments":[]}),
        );
        event.urgent = urgent;
        let Outcome::Event(event) = self
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Occur(event))?
        else {
            unreachable!()
        };
        if !duplicate {
            self.owned.record_notice_for_agent(
                room,
                None,
                Some(target.id()),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(room),
                format!(
                    "Agent inbox: event {} from {} on kernel {kernel} ({})",
                    event.sequence,
                    sender.agent_id,
                    if urgent { "urgent" } else { "next turn" }
                ),
            );
        }
        if let Err(error) = Box::pin(self.deliver_agent_inbox(room, target.id())).await {
            self.owned.record_notice_for_agent(
                room,
                None,
                Some(target.id()),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(room),
                format!(
                    "Event {} is durable and pending: {}",
                    event.sequence,
                    crate::secret_redaction::redact_secrets(&error.to_string())
                ),
            );
        }
        Ok(RelayPeerResponse::PeerAgentAccepted {
            protocol_version: VERSION,
            duplicate,
            target: self.peer_agent_stamp(target),
        })
    }
}
