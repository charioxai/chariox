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

/// The spawn key of the agent a peer spawn request of `kernel` creates.
pub(crate) fn peer_spawn_key(kernel: &str, request_id: &str) -> String {
    format!("peer-spawn-{kernel}-{request_id}")
}

/// The inbox source of a message from an agent on another kernel.
fn peer_message_source(kernel: &str, agent: &str) -> String {
    format!("peer:{kernel}:{agent}")
}

/// The stamp back out of such a source, for its inbox view; the sender's room is in its payload.
pub(crate) fn peer_message_stamp(
    source: &str,
    payload: &serde_json::Value,
) -> Option<PeerAgentStamp> {
    let (kernel, agent) = source.strip_prefix("peer:")?.split_once(':')?;
    Some(PeerAgentStamp {
        kernel_id: kernel.into(),
        room_id: payload["sender_room_id"].as_str()?.into(),
        agent_id: agent.into(),
    })
}

pub(crate) type PeerSpawnsInFlight =
    Arc<std::sync::Mutex<BTreeMap<String, tokio::sync::watch::Sender<()>>>>;

/// A peer spawn request this kernel life is acting on, or the first attempt a retry waits for.
pub(crate) enum PeerSpawnClaim {
    Claimed(PeerSpawnInFlight),
    /// Closes once the attempt holding the claim ends, with or without an answer.
    Busy(tokio::sync::watch::Receiver<()>),
}

/// Held while a peer spawn creates its agent; dropping it lets a waiting retry look again.
pub(crate) struct PeerSpawnInFlight {
    spawns: PeerSpawnsInFlight,
    identity: String,
}

impl Drop for PeerSpawnInFlight {
    fn drop(&mut self) {
        self.spawns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.identity);
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
                request_id,
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
                self.cancel_peer_agent_turn(kernel, &request_id, &target)
                    .await
            }
            request @ (RelayPeerRequest::SubscribePeerAgent { .. }
            | RelayPeerRequest::UnsubscribePeerAgent { .. }) => {
                self.receive_peer_follow(kernel, owner, request)
            }
            request @ RelayPeerRequest::DeliverPeerAgentOutcome { .. } => {
                self.receive_peer_agent_outcome(kernel, owner, request)
                    .await
            }
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

    /// One `request_id` of one sending kernel cancels at most the turn it first found running.
    /// The receipt names that turn before anything is cancelled, so a retry after a lost answer,
    /// or one that overlaps the first, cancels only that turn and never a later one.
    async fn cancel_peer_agent_turn(
        &self,
        kernel: &str,
        request_id: &str,
        target: &crate::agent::AgentInstance,
    ) -> Result<RelayPeerResponse, DaemonError> {
        if request_id.trim().is_empty() || request_id.len() > 128 {
            return Err(ledger::error("invalid peer request id"));
        }
        let store = &self.owned.durable_state_store;
        let receipt_id = format!("cancel:{request_id}");
        let duplicate = store
            .peer_agent_request_answer(kernel, &receipt_id)?
            .is_some();
        if !duplicate {
            let session = self.owned.session_store.get_session(target.session_id())?;
            let running = self
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, target.id())
                .map(|prompt| prompt.id().to_string());
            store.remember_peer_agent_request_answer(
                kernel,
                &receipt_id,
                &serde_json::json!({ "prompt_id": running }).to_string(),
            )?;
        }
        // The first receipt wins, also against an overlapping first attempt.
        let receipt = store
            .peer_agent_request_answer(kernel, &receipt_id)?
            .ok_or_else(|| ledger::error("peer cancel receipt was not kept"))?;
        let receipt: serde_json::Value =
            serde_json::from_str(&receipt).map_err(|error| ledger::error(error.to_string()))?;
        let Some(prompt) = receipt["prompt_id"].as_str() else {
            // The first request found the agent idle. Keep that receipt a no-op even
            // if a later retry arrives while the agent is working on another turn.
            return Ok(RelayPeerResponse::PeerAgentAccepted {
                protocol_version: VERSION,
                duplicate,
                target: self.peer_agent_stamp(target),
            });
        };
        // As for a peer in another room: the kernel cancels through the target's own
        // delivery seat, so the caller never joins the room.
        let attachment = self.ensure_agent_message_attachment(target.session_id(), target)?;
        match self
            .cancel_agent_prompt_if_matches_with_external_authority(
                target.session_id(),
                target.id(),
                &attachment,
                Some(prompt),
                None,
            )
            .await
        {
            Ok(_) => {}
            // The receipt's turn already ended, by this cancel's first attempt or otherwise.
            Err(DaemonError::NoActivePrompt { .. }) if duplicate => {}
            Err(error) => return Err(error),
        }
        Ok(RelayPeerResponse::PeerAgentAccepted {
            protocol_version: VERSION,
            duplicate,
            target: self.peer_agent_stamp(target),
        })
    }

    /// Waits for the attempt acting on this spawn request, if any, and takes its place. The claim
    /// serializes a spawn with the delete of the peer it creates.
    pub(crate) async fn claim_peer_spawn_when_free(
        &self,
        kernel: &str,
        request_id: &str,
    ) -> PeerSpawnInFlight {
        loop {
            match self.claim_peer_spawn(kernel, request_id) {
                PeerSpawnClaim::Claimed(in_flight) => return in_flight,
                // Closed when the first attempt ends; its answer, if any, is kept by then.
                PeerSpawnClaim::Busy(mut first) => {
                    let _ = first.changed().await;
                }
            }
        }
    }

    /// One attempt at a time acts on one spawn request of one sending kernel.
    pub(crate) fn claim_peer_spawn(&self, kernel: &str, request_id: &str) -> PeerSpawnClaim {
        let identity = format!("{kernel}:{request_id}");
        let mut spawns = self
            .peer_spawns_in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(first) = spawns.get(&identity) {
            return PeerSpawnClaim::Busy(first.subscribe());
        }
        spawns.insert(identity.clone(), tokio::sync::watch::channel(()).0);
        PeerSpawnClaim::Claimed(PeerSpawnInFlight {
            spawns: self.peer_spawns_in_flight.clone(),
            identity,
        })
    }

    /// The one peer resolver, with a room this owner cannot address told apart from an agent
    /// that is not there: the caller already names a room of its own owner.
    pub(super) fn peer_agent_target(
        &self,
        owner: &str,
        address: &PeerAgentAddress,
    ) -> Result<crate::agent::AgentInstance, PeerAgentRefusal> {
        let room = self.peer_room(owner, &address.room)?;
        self.resolve_peer_agent(owner, room.id(), &address.agent)
            .map_err(|_| PeerAgentRefusal::AgentUnavailable)
    }

    /// A room of `owner` named by its id or alias on this kernel, through the visible, non-ended
    /// room resolver a spawn on this kernel uses.
    pub(super) fn peer_room(
        &self,
        owner: &str,
        room: &str,
    ) -> Result<crate::session::RuntimeSession, PeerAgentRefusal> {
        let room = self
            .owned
            .session_store
            .read()
            .resolve_session_ref(room, None)
            .map_err(|_| PeerAgentRefusal::RoomUnavailable)?;
        if !self.acts_for_room_owner(&room, owner) {
            return Err(PeerAgentRefusal::RoomUnavailable);
        }
        Ok(room)
    }

    /// A peer spawn from another kernel: the room is one of `owner`'s, and the new agent gets
    /// what it asks for lowered to the caller's authority, itself lowered to this kernel's own
    /// record of the room. Answers the room's id with it.
    pub(crate) fn admit_room_peer_spawn(
        &self,
        owner: &str,
        room: &str,
        claimed: PeerCallerAuthority,
        mode: Option<crate::provider::AgentExecutionMode>,
        level: Option<crate::provider::AgentPermissionLevel>,
    ) -> Result<(String, crate::session::EffectiveAgentExecutionConfig), PeerAgentRefusal> {
        let session = self.peer_room(owner, room)?;
        let record = crate::session::effective_agent_execution_config(&session, None);
        let mine = crate::session::EffectiveAgentExecutionConfig {
            mode: claimed.mode.lower(record.mode),
            permission_level: claimed.permission_level.lower(record.permission_level),
        };
        Ok((
            session.id().to_string(),
            crate::session::EffectiveAgentExecutionConfig {
                mode: mode.unwrap_or(mine.mode).lower(mine.mode),
                permission_level: level
                    .unwrap_or(mine.permission_level)
                    .lower(mine.permission_level),
            },
        ))
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
        let source = peer_message_source(kernel, &sender.agent_id);
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
