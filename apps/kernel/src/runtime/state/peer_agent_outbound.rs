//! Peers across kernels, sending side: a room agent here messages, steers, cancels or spawns an
//! ordinary agent of a room on another kernel of its owner. The receiving kernel decides with its
//! own records (`peer_agent_inbound.rs`); this side names and stamps the caller and turns every
//! failure into one typed refusal the agent can act on. An unknown room, kernel or agent and one
//! the caller may not address give the same refusal.
use super::*;
use crate::durable_state::agent_lifecycle::created_peers::CreatedPeer;
use crate::durable_state::agent_lifecycle::Operation;
use crate::runtime::room_tool_admission::{denied, retry_later};
use crate::transport::relay_peer::{
    PeerAgentAddress, PeerAgentRefusal, PeerAgentStamp, PeerCallerAuthority, RelayPeerRequest,
    RelayPeerResponse, RELAY_PEER_PROTOCOL_VERSION,
};

const PEER_AGENT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// A subscription id names one follow, used once: it has one generation.
const PEER_FOLLOW_GENERATION: u64 = 1;

fn refusal(reason: &str, detail: impl std::fmt::Display) -> DaemonError {
    denied(format!("{reason}: {detail}"))
}

fn kernel_unreachable(kernel: &str) -> DaemonError {
    retry_later(format!(
        "kernel_unreachable: kernel `{kernel}` is not reachable now; try again when it is online"
    ))
}

const AMBIGUOUS_MACHINE: &str = "kernel_ambiguous";

fn ambiguous_machine(kernel_ref: &str) -> DaemonError {
    refusal(
        AMBIGUOUS_MACHINE,
        format!("machine `{kernel_ref}` runs several kernels; name one with --kernel"),
    )
}

fn uncertain(kernel: &str, retry: &str) -> DaemonError {
    retry_later(format!(
        "uncertain: kernel `{kernel}` may have acted on it before its answer was lost; {retry} and it lands once"
    ))
}

/// How the caller repeats `request` so the other kernel takes it once.
fn retry_hint(request: &RelayPeerRequest) -> String {
    match request {
        RelayPeerRequest::PeerAgentMessage { request_id, .. } => {
            format!("make the same call again (the send_agent_message tool needs idempotency_key `{request_id}`)")
        }
        RelayPeerRequest::PeerAgentCancel { .. } => "make the same call again in this turn".into(),
        _ => "make the same call again".into(),
    }
}

/// A peer request that got no answer, as its caller can act on it: the other kernel's own
/// refusal as it is, an answer lost or late after the relay handed the request over as
/// `uncertain`, and anything else as `kernel_unreachable`.
fn unanswered(kernel: &str, retry: &str, error: DaemonError) -> DaemonError {
    if error.is_peer_answer_lost() {
        return uncertain(kernel, retry);
    }
    match &error {
        DaemonError::RelayTransport { code, .. }
            if error.is_answered_peer_refusal() && code != "target_not_connected" =>
        {
            error
        }
        // What the other kernel answered through the test link reaches the caller as it is.
        #[cfg(test)]
        _ if !matches!(
            error,
            DaemonError::RelayTransport { .. } | DaemonError::LocalTransport { .. }
        ) =>
        {
            error
        }
        _ => kernel_unreachable(kernel),
    }
}

fn room_unavailable(kernel: &str) -> DaemonError {
    refusal(
        "room_unavailable",
        format!("that room is not available on kernel `{kernel}`"),
    )
}

/// What the receiving kernel answered, as the caller sees it.
fn refused(kernel: &str, refusal_reason: PeerAgentRefusal) -> DaemonError {
    match refusal_reason {
        PeerAgentRefusal::RoomUnavailable => room_unavailable(kernel),
        PeerAgentRefusal::AgentUnavailable | PeerAgentRefusal::Ambiguous => refusal(
            "agent_unavailable",
            format!("agent is unavailable in that room on kernel `{kernel}`"),
        ),
        PeerAgentRefusal::MoreAuthority {
            mode,
            permission_level,
        } => refusal(
            "not_permitted",
            format!(
                "the agent has more authority than you ({permission_level:?} in {mode} mode); send it a message instead"
            ),
        ),
        PeerAgentRefusal::SubAgentCaller => refusal(
            "not_permitted",
            "only a room agent reaches agents on another kernel",
        ),
        PeerAgentRefusal::RoomFull => refusal(
            "room_full",
            format!("that room on kernel `{kernel}` has no room for another agent"),
        ),
        PeerAgentRefusal::AttachmentsLocalOnly => attachments_local_only(),
    }
}

fn attachments_local_only() -> DaemonError {
    refusal(
        "attachments_local_only",
        "attachments stay on this kernel; send the message without them",
    )
}

/// The calling agent as the other kernel sees it: its owner, its stamp and its authority.
struct PeerAgentCaller {
    owner: String,
    stamp: PeerAgentStamp,
    authority: PeerCallerAuthority,
}

/// What a room agent here asks of an agent on another kernel.
pub(crate) enum PeerAgentCall {
    Message {
        message: String,
        urgent: bool,
        reply_requested: bool,
    },
    Cancel,
}

/// A peer to create in a room on another kernel.
pub(crate) struct RoomPeerSpawn {
    pub(crate) room: String,
    pub(crate) provider: String,
    pub(crate) account_profile: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) alias: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) permission_level: Option<crate::provider::AgentPermissionLevel>,
    /// Names the call across turns, as `agent spawn --command-id` does on this kernel.
    pub(crate) command_id: Option<String>,
    /// The caller's turn hears of the new peer's turn end, as a spawn in a room here does.
    pub(crate) follow: bool,
}

/// The peer a spawn on another kernel created; `duplicate` when an earlier attempt of the same
/// call already had.
pub(crate) struct RoomPeerSpawned {
    pub(crate) agent: PeerAgentStamp,
    pub(crate) alias: String,
    pub(crate) duplicate: bool,
    /// The caller's follow of the new peer, when the spawn asked for one.
    pub(crate) follow_registration_id: Option<String>,
}

impl KernelRuntimeState {
    /// The other kernel `kernel_ref` names, by id or name as `agent spawn --kernel` takes them,
    /// or by the name of the machine it runs on (`--machine`); `None` when it names this kernel.
    pub(crate) async fn resolve_peer_kernel(
        &self,
        kernel_ref: &str,
    ) -> Result<Option<String>, DaemonError> {
        let kernel_ref = kernel_ref.trim();
        let config = self.owned.config_projection.snapshot();
        if super::session_lifecycle_runtime_state::kernel_ref_matches_local_config(
            &config, kernel_ref,
        ) {
            return Ok(None);
        }
        #[cfg(test)]
        match peer_agent_link_for_tests::resolve(kernel_ref).as_slice() {
            [] => {}
            [kernel] => return Ok((*kernel != config.daemon_id).then(|| kernel.clone())),
            _ => return Err(ambiguous_machine(kernel_ref)),
        }
        if let Ok(presence) =
            crate::transport::relay_discovery::get_live_kernel(&config, kernel_ref).await
        {
            return Ok(Some(presence.kernel_id));
        }
        // A machine of the owner names the one kernel it runs.
        let kernels =
            crate::transport::relay_discovery::list_live_kernels_for_machine(&config, kernel_ref)
                .await
                .map_err(|_| kernel_unreachable(kernel_ref))?;
        match kernels.as_slice() {
            [] => Err(kernel_unreachable(kernel_ref)),
            [kernel] if kernel.kernel_id == config.daemon_id => Ok(None),
            [kernel] => Ok(Some(kernel.kernel_id.clone())),
            _ => Err(ambiguous_machine(kernel_ref)),
        }
    }

    /// The kernel reference of a request to create something, settled before it is placed: this
    /// kernel by id or machine name keeps the work here (`None`); a machine that runs
    /// several kernels is refused; any other reference stays for placement, which also refuses an
    /// unknown one.
    pub(crate) async fn settle_kernel_ref(
        &self,
        kernel_ref: Option<String>,
    ) -> Result<Option<String>, DaemonError> {
        let Some(name) = kernel_ref.as_deref() else {
            return Ok(None);
        };
        match self.resolve_peer_kernel(name).await {
            Ok(None) => Ok(None),
            Err(error) if error.to_string().contains(AMBIGUOUS_MACHINE) => Err(error),
            Ok(Some(_)) | Err(_) => Ok(kernel_ref),
        }
    }

    /// Messages, steers or cancels `agent` of `room` on `kernel`; answers the target's stamp and
    /// whether the other kernel had already seen this request.
    pub(crate) async fn relay_peer_agent(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
        room: &str,
        agent: &str,
        call: PeerAgentCall,
        request_id: String,
    ) -> Result<(PeerAgentStamp, bool), DaemonError> {
        // Nothing on the other kernel answers a reply request yet; accepting one would drop it.
        if let PeerAgentCall::Message {
            reply_requested: true,
            ..
        } = call
        {
            return Err(denied(
                "a reply to a message on another kernel is not supported yet: ask the agent there to message you back",
            ));
        }
        let caller = self.peer_agent_caller(sender, kernel)?;
        let target = PeerAgentAddress {
            room: room.trim().into(),
            agent: agent.trim().trim_start_matches('@').into(),
        };
        let request = match call {
            PeerAgentCall::Message {
                message,
                urgent,
                reply_requested,
            } => RelayPeerRequest::PeerAgentMessage {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                request_id,
                sender: caller.stamp,
                target,
                message,
                urgent,
                reply_requested,
                caller_authority: caller.authority,
            },
            PeerAgentCall::Cancel => RelayPeerRequest::PeerAgentCancel {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                request_id,
                sender: caller.stamp,
                target,
                caller_authority: caller.authority,
            },
        };
        match self.peer_agent_link(&caller.owner, kernel, request).await? {
            RelayPeerResponse::PeerAgentAccepted {
                target, duplicate, ..
            } => Ok((target, duplicate)),
            RelayPeerResponse::PeerAgentRefused { refusal, .. } => Err(refused(kernel, refusal)),
            _ => Err(kernel_unreachable(kernel)),
        }
    }

    /// Follows `target` on `kernel` under `subscription`; answers the followed agent's stamp.
    pub(super) async fn follow_peer_agent(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
        target: PeerAgentAddress,
        subscription: String,
    ) -> Result<PeerAgentStamp, DaemonError> {
        if sender.is_sub_agent() {
            return Err(refused(kernel, PeerAgentRefusal::SubAgentCaller));
        }
        let caller = self.peer_agent_caller(sender, kernel)?;
        let request = RelayPeerRequest::SubscribePeerAgent {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            request_id: subscription.clone(),
            sender: caller.stamp,
            target,
            subscription_id: subscription,
            generation: PEER_FOLLOW_GENERATION,
        };
        match self.peer_agent_link(&caller.owner, kernel, request).await? {
            RelayPeerResponse::PeerAgentSubscribed { target, .. } => Ok(target),
            RelayPeerResponse::PeerAgentRefused { refusal, .. } => Err(refused(kernel, refusal)),
            _ => Err(kernel_unreachable(kernel)),
        }
    }

    /// Ends a follow this kernel holds on `kernel`.
    pub(super) async fn unfollow_peer_agent(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
        subscription: &str,
    ) -> Result<(), DaemonError> {
        let caller = self.peer_agent_caller(sender, kernel)?;
        let request = RelayPeerRequest::UnsubscribePeerAgent {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            subscription_id: subscription.into(),
            generation: PEER_FOLLOW_GENERATION,
        };
        match self.peer_agent_link(&caller.owner, kernel, request).await? {
            RelayPeerResponse::PeerAgentOutcomeAck { .. } => Ok(()),
            _ => Err(kernel_unreachable(kernel)),
        }
    }

    /// Creates an ordinary agent in `spawn.room` on `kernel`. A retry of the same call after a
    /// lost answer reaches the peer the first attempt created.
    pub(crate) async fn spawn_room_peer(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
        spawn: RoomPeerSpawn,
    ) -> Result<RoomPeerSpawned, DaemonError> {
        if sender.is_sub_agent() {
            return Err(refused(kernel, PeerAgentRefusal::SubAgentCaller));
        }
        let caller = self.peer_agent_caller(sender, kernel)?;
        let request_id = self.peer_spawn_request_id(sender, &spawn)?;
        let task = if spawn.follow {
            Some(self.peer_follow_task(sender)?)
        } else {
            None
        };
        // The follow is named by the call and the turn's task: a retry in the same turn re-asks
        // for the same follow, and the same call from a later turn holds a follow of its own.
        let follow = task
            .as_ref()
            .map(|(task, _)| crate::transport::relay_peer::PeerFollow {
                subscription_id: peer_spawn_follow_id(&request_id, task),
                generation: PEER_FOLLOW_GENERATION,
            });
        // A retry after a lost answer finds the follow still held, with what it recorded since.
        let still_held = match (&follow, &task) {
            (Some(follow), Some((task, _))) => self
                .owned
                .durable_state_store
                .agent_registration(&follow.subscription_id)?
                .filter(|held| held.live && held.task_id == *task)
                .map(|held| held.source_cursor),
            _ => None,
        };
        let source_cursor = match still_held {
            Some(cursor) => cursor,
            None => self.owned.durable_state_store.agent_source_cursor()?,
        };
        // The follow is held here before the other kernel creates the peer, so a first turn that
        // ends before the answer arrives is recorded and reaches the follow once the peer is known.
        let held = match (&follow, &task) {
            (Some(follow), Some((task, prompt))) => {
                let pending = PeerAgentStamp {
                    kernel_id: kernel.into(),
                    room_id: spawn.room.trim().into(),
                    agent_id: follow.subscription_id.clone(),
                };
                Some(self.register_peer_follow(
                    task,
                    prompt,
                    follow.subscription_id.clone(),
                    &pending,
                    source_cursor,
                )?)
            }
            _ => None,
        };
        // The peer lives and dies with its creator. The link is written before the request goes
        // out, named by the request, so a lost answer still leaves the peer to delete.
        let created = CreatedPeer {
            creator_room: sender.session_id().into(),
            creator_agent: sender.id().into(),
            kernel: kernel.into(),
            peer: request_id.clone(),
            created_at_ms: 0,
        };
        // An earlier attempt of this spawn may have written the link and may have created the
        // peer: only a link this attempt wrote is its to take back.
        let link_is_new = self.record_created_peer(created.clone())?;
        let request = RelayPeerRequest::SpawnRoomPeer {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            request_id,
            sender: caller.stamp,
            room: spawn.room.trim().into(),
            provider: spawn.provider,
            account_profile: spawn.account_profile,
            model: spawn.model,
            effort: spawn.effort,
            alias: spawn.alias,
            prompt: spawn.prompt,
            execution_mode: None,
            permission_level: spawn.permission_level,
            caller_authority: caller.authority,
            follow: follow.clone(),
        };
        let retry = retry_hint(&request);
        let answer = self.peer_agent_send(&caller.owner, kernel, request).await;
        // The other kernel may have created the peer: the follow stays held for the retry.
        let lost = matches!(&answer, Err(error) if error.is_peer_answer_lost());
        let answered = match answer.map_err(|error| unanswered(kernel, &retry, error)) {
            Ok(RelayPeerResponse::RoomPeerSpawned {
                agent,
                alias,
                duplicate,
                ..
            }) => Ok((agent, alias, duplicate)),
            Ok(RelayPeerResponse::PeerAgentRefused { refusal, .. }) => {
                Err(refused(kernel, refusal))
            }
            Ok(_) => Err(kernel_unreachable(kernel)),
            Err(error) => Err(error),
        };
        let (agent, alias, duplicate) = match answered {
            Ok(spawned) => spawned,
            Err(error) => {
                if !lost && link_is_new {
                    self.release_created_peer(created)?;
                }
                if let (Some(registration), Some((task, prompt)), false) = (held, task, lost) {
                    self.owned
                        .durable_state_store
                        .agent_lifecycle(Operation::Unsubscribe {
                            task,
                            prompt,
                            registration,
                        })?;
                }
                return Err(error);
            }
        };
        let follow_registration_id = match (follow, task) {
            (Some(follow), Some((task, prompt))) => Some(self.register_peer_follow(
                &task,
                &prompt,
                follow.subscription_id,
                &agent,
                source_cursor,
            )?),
            _ => None,
        };
        Ok(RoomPeerSpawned {
            agent,
            alias,
            duplicate,
            follow_registration_id,
        })
    }

    /// Deletes the peer a spawn created on another kernel for a creator that has ended. The
    /// kernel finds it by the spawn's request id, whatever its room is called now; a peer that is
    /// not there any more is the outcome asked for.
    pub(crate) async fn delete_created_peer_on_kernel(
        &self,
        created: &CreatedPeer,
    ) -> Result<(), DaemonError> {
        let config = self.owned.config_projection.snapshot();
        let Some(owner) = config
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .filter(|owner| !owner.is_empty())
        else {
            return Err(refusal(
                "owner_unverified",
                "this kernel is not signed in to the Cloud account of the creator's owner, so it cannot reach the peer to delete it",
            ));
        };
        let request = RelayPeerRequest::DeleteRoomPeer {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            request_id: created.peer.clone(),
            sender: PeerAgentStamp {
                kernel_id: config.daemon_id,
                room_id: created.creator_room.clone(),
                agent_id: created.creator_agent.clone(),
            },
        };
        match self
            .peer_agent_link(&owner, &created.kernel, request)
            .await?
        {
            RelayPeerResponse::RoomPeerDeleted { .. } => Ok(()),
            // A peer in a room this owner cannot address still exists: the link stays.
            RelayPeerResponse::PeerAgentRefused { refusal, .. } => {
                Err(refused(&created.kernel, refusal))
            }
            _ => Err(kernel_unreachable(&created.kernel)),
        }
    }

    /// One spawn call is one request id, which the receiving kernel remembers with its answer.
    /// The id is derived from what names the call durably: the command id, or else the
    /// caller's running turn and the spawn's arguments. Nothing random, so nothing to lose.
    fn peer_spawn_request_id(
        &self,
        sender: &crate::agent::AgentInstance,
        spawn: &RoomPeerSpawn,
    ) -> Result<String, DaemonError> {
        use sha2::{Digest, Sha256};
        let call = match &spawn.command_id {
            Some(command_id) => serde_json::json!(["command", command_id]),
            None => {
                let session = self.owned.session_store.get_session(sender.session_id())?;
                let turn = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, sender.id())
                    .ok_or_else(|| {
                        crate::durable_state::agent_lifecycle::error(
                            "agent spawn on another kernel needs a running turn",
                        )
                    })?;
                serde_json::json!([
                    "turn",
                    turn.id(),
                    spawn.room.trim(),
                    spawn.provider,
                    spawn.account_profile,
                    spawn.model,
                    spawn.effort,
                    spawn.alias,
                    spawn.prompt,
                    spawn.permission_level,
                ])
            }
        };
        let digest = Sha256::digest(serde_json::json!([sender.id(), call]).to_string());
        Ok(digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    /// A cancel names the caller's turn and its target, so the same call again in that turn,
    /// after a lost answer, is the same request: the other kernel answers it with the receipt of
    /// the turn it first cancelled and leaves any later turn running.
    pub(crate) fn peer_cancel_request_id(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
        room: &str,
        agent: &str,
    ) -> Result<String, DaemonError> {
        use sha2::{Digest, Sha256};
        let session = self.owned.session_store.get_session(sender.session_id())?;
        let turn = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, sender.id())
            .ok_or_else(|| {
                crate::durable_state::agent_lifecycle::error(
                    "agent cancel on another kernel needs a running turn",
                )
            })?;
        let call = serde_json::json!([
            sender.id(),
            "cancel",
            turn.id(),
            kernel,
            room.trim(),
            agent.trim().trim_start_matches('@'),
        ]);
        let digest = Sha256::digest(call.to_string());
        Ok(digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    /// The caller's owner must be the Cloud owner both kernels share, and a guest of a room
    /// reaches no agent outside it: it gets the answer of a room that is not there.
    fn peer_agent_caller(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
    ) -> Result<PeerAgentCaller, DaemonError> {
        let session = self.owned.session_store.get_session(sender.session_id())?;
        if self.drives_as_room_guest(&session, sender) {
            return Err(room_unavailable(kernel));
        }
        let config = self.owned.config_projection.snapshot();
        let Some(owner) = config
            .cloud_relay
            .as_ref()
            .map(|profile| profile.user_id.clone())
            .filter(|owner| !owner.is_empty() && self.acts_for_room_owner(&session, owner))
        else {
            return Err(refusal(
                "owner_unverified",
                "this kernel is not signed in to the Cloud account of this room's owner, so it cannot reach agents on other kernels",
            ));
        };
        let authority = crate::session::effective_agent_execution_config_in_tree(
            &session,
            sender,
            &self.session_agents(sender.session_id()),
        );
        Ok(PeerAgentCaller {
            owner,
            stamp: PeerAgentStamp {
                kernel_id: config.daemon_id,
                room_id: sender.session_id().into(),
                agent_id: sender.id().into(),
            },
            authority: PeerCallerAuthority {
                mode: authority.mode,
                permission_level: authority.permission_level,
            },
        })
    }

    /// The Cloud owner whose kernels `sender` reaches, under the rules of its peer agent calls.
    pub(super) fn peer_owner_of_agent(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
    ) -> Result<String, DaemonError> {
        self.peer_agent_caller(sender, kernel)
            .map(|caller| caller.owner)
    }

    /// The one link to a kernel of the same owner. A kernel that is gone before it takes the
    /// request is unreachable, one gone or silent after it is uncertain; any answer it gave
    /// reaches the caller as it is.
    pub(super) async fn peer_agent_link(
        &self,
        owner: &str,
        kernel: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let retry = retry_hint(&request);
        self.peer_agent_send(owner, kernel, request)
            .await
            .map_err(|error| unanswered(kernel, &retry, error))
    }

    /// The request on the link, with the relay's own failure when no answer came back.
    async fn peer_agent_send(
        &self,
        owner: &str,
        kernel: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        self.peer_send(owner, kernel, request, PEER_AGENT_REQUEST_TIMEOUT)
            .await
    }

    /// One request to a kernel of the same owner and its raw answer, waited for up to `timeout`.
    pub(super) async fn peer_send(
        &self,
        owner: &str,
        kernel: &str,
        request: RelayPeerRequest,
        timeout: std::time::Duration,
    ) -> Result<RelayPeerResponse, DaemonError> {
        // The test link stands for the relay: its transport failures are the relay's.
        #[cfg(test)]
        if let Some(link) = peer_agent_link_for_tests::link(kernel) {
            let from = self.owned.config_projection.snapshot().daemon_id;
            return link(from, owner.to_string(), request).await;
        }
        self.send_same_owner_peer_request(owner, kernel, request, timeout)
            .await
    }
}

fn peer_spawn_follow_id(request_id: &str, task: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(serde_json::json!([request_id, task]).to_string());
    let hex: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("follow-{hex}")
}

/// Peer refusals for attachments are decided here: files stay on this kernel.
pub(crate) fn refuse_peer_attachments(
    attachments: &[crate::session::PromptAttachment],
) -> Result<(), DaemonError> {
    if attachments.is_empty() {
        Ok(())
    } else {
        Err(attachments_local_only())
    }
}

/// Two kernels in one test process: a request to a registered kernel id reaches its router as
/// the relay would deliver it, after the same-owner admission.
#[cfg(test)]
mod retry_flag_tests {
    use super::*;
    use crate::transport::kernel_protocol::map_kernel_error;

    /// An offline kernel or a lost answer comes back; the caller is told to repeat the call.
    #[test]
    fn a_peer_that_may_answer_on_a_retry_is_reported_as_retryable() {
        for error in [
            kernel_unreachable("builder"),
            uncertain("builder", "make the same call again"),
        ] {
            let mapped = map_kernel_error(&error);
            assert!(mapped.retryable, "{error}");
            assert_eq!(mapped.message, error.to_string());
        }
    }
}

#[cfg(test)]
pub(crate) mod peer_agent_link_for_tests {
    use super::*;
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, LazyLock, Mutex};

    type Answer = Pin<Box<dyn Future<Output = Result<RelayPeerResponse, DaemonError>> + Send>>;
    pub(crate) type Link = Arc<dyn Fn(String, String, RelayPeerRequest) -> Answer + Send + Sync>;

    static LINKS: LazyLock<Mutex<HashMap<String, (String, Link)>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// `kernel` answers under its id and under `name`.
    pub(crate) fn register(kernel: &str, name: &str, link: Link) {
        LINKS
            .lock()
            .unwrap()
            .insert(kernel.to_string(), (name.to_string(), link));
    }

    pub(crate) fn unregister(kernel: &str) {
        LINKS.lock().unwrap().remove(kernel);
    }

    pub(super) fn resolve(kernel_ref: &str) -> Vec<String> {
        LINKS
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, (name, _))| *id == kernel_ref || name == kernel_ref)
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub(super) fn link(kernel: &str) -> Option<Link> {
        LINKS
            .lock()
            .unwrap()
            .get(kernel)
            .map(|(_, link)| link.clone())
    }
}
