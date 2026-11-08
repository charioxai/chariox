//! Peers across kernels, sending side: a room agent here messages, steers, cancels or spawns an
//! ordinary agent of a room on another kernel of its owner. The receiving kernel decides with its
//! own records (`peer_agent_inbound.rs`); this side names and stamps the caller and turns every
//! failure into one typed refusal the agent can act on. An unknown room, kernel or agent and one
//! the caller may not address give the same refusal.
use super::*;
use crate::runtime::room_tool_admission::denied;
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
    refusal(
        "kernel_unreachable",
        format!("kernel `{kernel}` is not reachable now; try again when it is online"),
    )
}

fn uncertain(kernel: &str, retry: &str) -> DaemonError {
    refusal(
        "uncertain",
        format!("kernel `{kernel}` may have acted on it before its answer was lost; {retry} and it lands once"),
    )
}

/// How the caller repeats `request` so the other kernel takes it once.
fn retry_hint(request: &RelayPeerRequest) -> String {
    match request {
        RelayPeerRequest::PeerAgentMessage { request_id, .. } => {
            format!("send it again with idempotency_key `{request_id}`")
        }
        _ => "make the same call again".into(),
    }
}

/// A peer request that got no answer, as its caller can act on it: the other kernel's own
/// refusal as it is, an answer lost after that kernel took the request as `uncertain`, and
/// anything else as `kernel_unreachable`.
fn unanswered(kernel: &str, retry: &str, error: DaemonError) -> DaemonError {
    match &error {
        DaemonError::RelayTransport { code, .. } if code == "target_disconnected" => {
            uncertain(kernel, retry)
        }
        DaemonError::RelayTransport { code, .. }
            if error.is_answered_peer_refusal() && code != "target_not_connected" =>
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
    /// The other kernel `kernel_ref` names, by id or name as `agent spawn --kernel` takes them;
    /// `None` when it names this kernel.
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
        if let Some(kernel) = peer_agent_link_for_tests::resolve(kernel_ref) {
            return Ok(Some(kernel));
        }
        crate::transport::relay_discovery::get_live_kernel(&config, kernel_ref)
            .await
            .map(|presence| Some(presence.kernel_id))
            .map_err(|_| kernel_unreachable(kernel_ref))
    }

    /// Messages, steers or cancels `agent` of `room` on `kernel`; answers the target's stamp.
    pub(crate) async fn relay_peer_agent(
        &self,
        sender: &crate::agent::AgentInstance,
        kernel: &str,
        room: &str,
        agent: &str,
        call: PeerAgentCall,
        request_id: String,
    ) -> Result<PeerAgentStamp, DaemonError> {
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
            RelayPeerResponse::PeerAgentAccepted { target, .. } => Ok(target),
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
        // The follow is named by the call, so a retried spawn re-asks for the same follow.
        let follow = spawn
            .follow
            .then(|| crate::transport::relay_peer::PeerFollow {
                subscription_id: format!("follow-{request_id}"),
                generation: PEER_FOLLOW_GENERATION,
            });
        let task = match &follow {
            Some(_) => Some(self.peer_follow_task(sender)?),
            None => None,
        };
        let source_cursor = self.owned.durable_state_store.agent_source_cursor()?;
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
        match self.peer_agent_link(&caller.owner, kernel, request).await? {
            RelayPeerResponse::RoomPeerSpawned {
                agent,
                alias,
                duplicate,
                ..
            } => {
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
            RelayPeerResponse::PeerAgentRefused { refusal, .. } => Err(refused(kernel, refusal)),
            _ => Err(kernel_unreachable(kernel)),
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
            .filter(|owner| !owner.is_empty() && owner == session.owner_user_id())
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

    /// The one link to a kernel of the same owner. A kernel that is gone before it takes the
    /// request is unreachable, one gone after it is uncertain; any answer it gave reaches the
    /// caller as it is.
    pub(super) async fn peer_agent_link(
        &self,
        owner: &str,
        kernel: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let retry = retry_hint(&request);
        // The test link stands for the relay: what it answers is mapped as the relay's would be.
        #[cfg(test)]
        if let Some(link) = peer_agent_link_for_tests::link(kernel) {
            let from = self.owned.config_projection.snapshot().daemon_id;
            return match link(from, owner.to_string(), request).await {
                Err(error @ DaemonError::RelayTransport { .. }) => {
                    Err(unanswered(kernel, &retry, error))
                }
                answered => answered,
            };
        }
        self.send_same_owner_peer_request(owner, kernel, request, PEER_AGENT_REQUEST_TIMEOUT)
            .await
            .map_err(|error| unanswered(kernel, &retry, error))
    }
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

    pub(super) fn resolve(kernel_ref: &str) -> Option<String> {
        LINKS
            .lock()
            .unwrap()
            .iter()
            .find(|(id, (name, _))| *id == kernel_ref || name == kernel_ref)
            .map(|(id, _)| id.clone())
    }

    pub(super) fn link(kernel: &str) -> Option<Link> {
        LINKS
            .lock()
            .unwrap()
            .get(kernel)
            .map(|(_, link)| link.clone())
    }
}
