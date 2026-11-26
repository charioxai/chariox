//! One owner decision before an agent can start charged work, unless the owner allowed it without
//! asking within limits (`without_asking`).
use super::*;
use crate::durable_state::acquire_choices::AcquireOption;
use crate::local::{
    ControlPlaneRequest, MachineAcquisitionChoice, MachineAcquisitionEstimate,
    MachineAcquisitionRequest, MachineAcquisitionResult, MachineAllocationCapability,
};
use crate::runtime::command::KernelCommand;
use crate::session::{
    MachineApprovalReceipt, MachineApprovalReceiptState, RuntimeInteraction,
    RuntimeInteractionChoice, RuntimeInteractionChoiceStyle,
};
use sha2::Digest;

/// Every machine acquisition decision's id starts with this; the rest is derived from the request.
const ACQUISITION_DECISION_PREFIX: &str = "machine-acquisition-";

/// How long the owner has to decide before the request is not sent.
const ACQUISITION_DECISION_SECONDS: u64 = 300;
/// The request identities whose approved request has been sent (see `Approval::Replay`).
const ACQUISITION_SENT_PREFIX: &str = "machine-use-";
const ACQUISITION_SENT_EVENT: &str = "compute.acquisition_sent";

/// A machine acquisition decision is the one kernel operation an agent may answer for the person: the
/// holder above the requester (`agent answer`) or an agent of the same owner in another Room
/// (`agent answer --room`). The requester and its own sub-agents never may: see
/// `refuse_self_approval`.
pub(super) fn is_machine_acquisition_decision(id: &str) -> bool {
    id.starts_with(ACQUISITION_DECISION_PREFIX)
}

impl KernelRuntimeOwnedState {
    /// The capability revision the first send of this request identity went out at, if one did.
    fn first_acquisition_send(
        &self,
        identity: &str,
        owner: &str,
    ) -> Result<Option<String>, DaemonError> {
        Ok(self
            .durable_state_store
            .load_subject_events_by_kind_after(identity, ACQUISITION_SENT_EVENT, 0, 200)?
            .into_iter()
            .find(|event| event.payload["owner"] == owner)
            .and_then(|event| event.payload["revision"].as_str().map(str::to_owned)))
    }

    /// The one durable first use of an approved request: the first journal entry for the identity
    /// wins, and every later try reads it. It is written before the request is posted, so a post
    /// never exists without it: an approved request that has been sent may hold a charged allocation
    /// whose response was lost, and only it, replayed, can find that allocation again.
    fn spend_acquisition_send(
        &self,
        identity: &str,
        owner: &str,
        revision: &str,
    ) -> Result<String, DaemonError> {
        if let Some(first) = self.first_acquisition_send(identity, owner)? {
            return Ok(first);
        }
        self.durable_state_store.append_event(
            ACQUISITION_SENT_EVENT,
            Some(identity.to_owned()),
            serde_json::json!({"owner":owner,"revision":revision}),
        )?;
        self.first_acquisition_send(identity, owner)?
            .ok_or_else(|| {
                acquisition_error(
                    "acquisition_unavailable",
                    "The approval could not be recorded. Retry this request_id",
                )
            })
    }

    /// A machine acquisition needs an OK from outside the subtree that asked for it, so an agent cannot
    /// approve its own request or have a sub-agent do it. Checked by the answering agent's
    /// authenticated identity at the kernel, never by what the answer claims.
    pub(super) fn refuse_self_approval(
        &self,
        interaction: &RuntimeInteraction,
        answering: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(requester) = interaction
            .related_agent_id()
            .filter(|_| is_machine_acquisition_decision(interaction.id()))
        else {
            return Ok(());
        };
        // The whole agent tree, not the answering agent's Room: a sub-agent spawned into another
        // Room keeps its spawner as its parent, so the requester can sit in a different Room.
        let agents = self.agent_store.list_agents();
        let own_subtree = answering.id() == requester
            || crate::agent::authority_limit::ancestors(answering, &agents)
                .iter()
                .any(|ancestor| ancestor.id() == requester);
        if own_subtree {
            return Err(crate::runtime::room_tool_admission::denied(
                "An agent cannot approve a machine acquisition it or its sub-agents asked for; another agent or the person must answer it",
            ));
        }
        Ok(())
    }
}

/// How an acquisition was allowed to go ahead.
#[derive(Debug)]
pub(crate) enum Approval {
    /// The person asked, the owner approved it, or the owner's /sudo covers it.
    Owner,
    /// Within the owner's limits for acquiring without asking, or a retry of a request admitted
    /// before; the service's answer settles it (`settle_acquisition`).
    WithoutAsking(without_asking::AcquisitionHold),
    /// The owner approved this request and it has been sent, so a try at another quote (the rate
    /// or the offer changed, the response was lost) is not a new request: it goes again with the
    /// capability revision it was approved at. The service answers an accepted request with its
    /// allocation and refuses one it never accepted as stale, so nothing is allocated at terms
    /// nobody approved, and the owner is not asked again for a machine that may already be charged.
    Replay { capability_revision: String },
}

impl KernelRuntimeState {
    /// With no account limits to bound the owner's own (the tests that predate them).
    #[cfg(test)]
    pub(crate) async fn approve_machine_acquisition(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
        capability: Option<&MachineAllocationCapability>,
    ) -> Result<Approval, DaemonError> {
        self.approve_machine_acquisition_within(
            command,
            request,
            capability,
            &limits::AccountBound::None,
        )
        .await
    }

    /// `bound` is what the account's limits say now: the owner's no-ask limits never exceed them.
    pub(crate) async fn approve_machine_acquisition_within(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
        capability: Option<&MachineAllocationCapability>,
        bound: &limits::AccountBound,
    ) -> Result<Approval, DaemonError> {
        let Some(subject) = self.acquisition_subject(command, request)? else {
            return Ok(Approval::Owner);
        };
        let AcquisitionSubject {
            room,
            owner,
            agent,
            request_id,
            id,
            identity,
        } = subject;
        let agent = agent.as_deref();
        let request_id = &request_id;
        // The one machine acquisition toggle decides (row 405): no /sudo shortcut around its single card.
        if self
            .owned
            .durable_state_store
            .load_subject_events_by_kind(&id, "compute.acquisition_approved", 1)?
            .iter()
            .any(|event| event.payload["owner"] == owner.as_str())
        {
            return self.send_approved(&identity, &owner, request);
        }
        // An approved request that was sent keeps its approval whatever the quote says now: the
        // owner is not asked again for a machine that may already be charged under it.
        if let Some(first) = self.owned.first_acquisition_send(&identity, &owner)? {
            return Ok(Approval::Replay {
                capability_revision: first,
            });
        }
        let outside_limits = match self.admit_without_asking(
            &room,
            agent.unwrap_or_default(),
            request,
            capability,
            bound,
        )? {
            without_asking::Admission::Within(hold) => return Ok(Approval::WithoutAsking(hold)),
            without_asking::Admission::Off => None,
            without_asking::Admission::Outside(reason) => Some(reason),
        };
        let description = match (request, capability) {
            (
                ControlPlaneRequest::AcquireJob {
                    needs, timeout_s, ..
                },
                Some(capability),
            ) => {
                let class = super::job_compute::select_job_class(capability, needs)?;
                let minutes = timeout_s.saturating_add(59) / 60;
                format!("Run this job on a VM ({}, {} CPUs, {} MiB) at {} {} per minute? The command limit is {minutes} minutes; charges continue through startup and until deletion.", class.class_id, class.vcpu, class.usable_memory_mib, money(class.rate_per_minute.amount_micros), class.rate_per_minute.currency)
            }
            (
                ControlPlaneRequest::Acquire {
                    class_id: Some(class_id),
                    region,
                    ..
                },
                Some(capability),
            ) => {
                // The service validates the choice; the card names what it lists, or the bare request.
                match capability.offers.iter().find(|option| {
                    &option.class_id == class_id
                        && region.as_ref().is_none_or(|region| &option.region == region)
                }) {
                    Some(option) => format!(
                        "Acquire {} in {} ({} CPUs, {} GB) at {} {} per minute? The total depends on how long you keep it.",
                        option.class_id, option.region, option.vcpu, option.ram_gb,
                        money(option.rate_per_minute.amount_micros), option.rate_per_minute.currency
                    ),
                    None => format!(
                        "Acquire {class_id}{} at the connected service's quoted rate?",
                        region.as_ref().map(|region| format!(" in {region}")).unwrap_or_default()
                    ),
                }
            }
            (_, Some(capability)) => capability
                .offer
                .as_ref()
                .map(|offer| {
                    if let Some(rate) = &offer.rate_per_minute {
                        return format!(
                            "Acquire {} ({} CPUs, {} GB) at {} {} per minute? The total depends on how long you keep it.",
                            offer.machine_type, offer.vcpu, offer.ram_gb,
                            money(rate.amount_micros), rate.currency
                        );
                    }
                    format!(
                        "Acquire {} ({} CPUs, {} GB) for {} {} per {:?}?",
                        offer.machine_type,
                        offer.vcpu,
                        offer.ram_gb,
                        offer.terms.amount,
                        offer.terms.currency,
                        offer.terms.period
                    )
                })
                .unwrap_or_else(|| {
                    "Acquire a machine at the connected service's quoted rate?".into()
                }),
            _ => {
                return Err(acquisition_error(
                    "capacity_unavailable",
                    "The connected service has no machine offer, so this try was not sent",
                ))
            }
        };
        let description = match outside_limits {
            Some(reason) => format!("{description} Asked because {reason}."),
            None => description,
        };
        let description = match request {
            ControlPlaneRequest::Acquire {
                timeout_s: Some(timeout_s),
                ..
            } => format!(
                "{description} It stops by itself after {} minutes.",
                timeout_s.saturating_add(59) / 60
            ),
            _ => description,
        };
        // The owner tells the Room's agents apart by name, and decides on what the machine is for (row 257).
        let title = machine_request_title(
            agent.map(|agent| {
                self.owned
                    .agent_store
                    .get_agent(agent)
                    .ok()
                    .map(|known| known.alias().unwrap_or(known.agent_ref()).to_owned())
                    .or_else(|| {
                        self.owned
                            .session_store
                            .get_session(&room)
                            .ok()
                            .and_then(|session| session.agent_display_name(agent))
                    })
                    .unwrap_or_else(|| agent.to_owned())
            }),
            self.owned
                .acquire_purposes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(request_id)
                .cloned(),
        );
        let receipt_message = format!("{title} — {description}");
        let mut interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            title,
            description,
            vec![
                RuntimeInteractionChoice::new(
                    "approve",
                    "Approve",
                    "approve",
                    Some(RuntimeInteractionChoiceStyle::Primary),
                ),
                RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
        );
        if let Some(agent) = agent {
            interaction = interaction.with_related_agent_id(agent.into());
        }
        if room.is_empty() {
            interaction = interaction.with_timeout_sec(ACQUISITION_DECISION_SECONDS);
        }
        let receiver = self
            .create_kernel_operation_interaction(
                crate::runtime::state::CardKind::OwnerControl,
                &room,
                &owner,
                interaction,
            )
            .await?;
        let mut pending = PendingAcquisition(Some((self.clone(), room.clone(), id.clone())));
        let asked = std::time::Instant::now();
        let waited =
            tokio::time::timeout(Duration::from_secs(ACQUISITION_DECISION_SECONDS), receiver).await;
        // Only an answer is the owner's decision; every other end says what closed the card.
        let mut expired = false;
        let unanswered = match waited {
            Ok(Ok(answer)) if answer.status == "answered" => {
                pending.0 = None;
                if answer.choice_id.as_deref() != Some("approve") {
                    return Err(acquisition_error(
                        "acquisition_denied",
                        "The owner denied this request, so it was not sent to the service; nothing was allocated or charged",
                    ));
                }
                None
            }
            Ok(Ok(answer)) => Some(match answer.reply {
                Some(reason) => format!("it could not be answered: {reason}"),
                None => format!("nobody answered it within {} s", asked.elapsed().as_secs()),
            }),
            Ok(Err(_)) => Some(format!(
                "the kernel withdrew it after {} s, before anyone answered",
                asked.elapsed().as_secs()
            )),
            Err(_) => {
                expired = true;
                Some(format!(
                    "nobody answered it within {} s",
                    asked.elapsed().as_secs()
                ))
            }
        };
        if let Some(why) = unanswered {
            self.timeout_runtime_interaction(&room, &id).await?;
            if expired {
                // The card is gone and the agent's try ended: the receipt says when it ran out.
                self.save_machine_receipt(
                    &room,
                    MachineApprovalReceipt {
                        id: id.clone(),
                        shown_in_agent_id: agent.map(str::to_owned),
                        message: receipt_message.clone(),
                        state: MachineApprovalReceiptState::Expired,
                        expires_at_ms: Some(crate::session::unix_epoch_ms()),
                    },
                );
            }
            return Err(acquisition_error(
                "acquisition_unanswered",
                &format!(
                    "The owner's card {id} for this request closed without a decision: {why}. It was not sent to the service and nothing was allocated or charged; retry with the same request_id when the owner can answer."
                ),
            ));
        }
        self.owned.durable_state_store.append_event(
            "compute.acquisition_approved",
            Some(id.clone()),
            serde_json::json!({"owner":owner,"approved_at_ms":crate::session::unix_epoch_ms()}),
        )?;
        // The card is gone; the receipt keeps the approved request visible until the service answers.
        self.save_machine_receipt(
            &room,
            MachineApprovalReceipt {
                id,
                shown_in_agent_id: agent.map(str::to_owned),
                message: receipt_message,
                state: MachineApprovalReceiptState::Approving,
                expires_at_ms: None,
            },
        );
        self.send_approved(&identity, &owner, request)
    }

    /// The approved request goes out: its first use is recorded before it is posted, and a request
    /// already sent goes again at the revision it was sent at.
    fn send_approved(
        &self,
        identity: &str,
        owner: &str,
        request: &ControlPlaneRequest,
    ) -> Result<Approval, DaemonError> {
        let revision = request.capability_revision().ok_or_else(|| {
            acquisition_error(
                "invalid_request",
                "This request carries no capability revision",
            )
        })?;
        let first = self
            .owned
            .spend_acquisition_send(identity, owner, revision)?;
        Ok(if first == revision {
            Approval::Owner
        } else {
            Approval::Replay {
                capability_revision: first,
            }
        })
    }

    /// The agent behind a request (the Room it acts in and the owner it acts for), or `None` when the
    /// kernel owner asks themselves. An agent acting for the owner through an access grant has no Room.
    pub(crate) fn agent_caller(
        &self,
        command: &KernelCommand,
    ) -> Result<Option<AgentCaller>, DaemonError> {
        let agent_request = command.external_grant_id().is_some()
            || command.reach_caller_id().is_some()
            || command.caller.metaagent_id.is_some()
            || command.caller.connection_class
                == Some(crate::local::KernelConnectionClass::KernelAgent);
        if !agent_request {
            return Ok(None);
        }
        let reach = command
            .reach_caller_id()
            .and_then(|id| self.reach_for_id(&id));
        let agent = reach
            .as_ref()
            .map(|r| r.agent_id.as_str())
            .or(command.agent_id.as_deref())
            .or(command.caller.metaagent_id.as_deref());
        let agent_room = agent
            .and_then(|agent| self.owned.agent_store.get_agent(agent).ok())
            .map(|a| a.session_id().to_owned())
            .or_else(|| command.session_id.clone());
        // An agent acting for the owner through an access grant has no Room: its decision is one of
        // the owner's cards outside any Room, answered like an App's host offer.
        let (room, owner) = match agent_room {
            Some(room) => {
                let owner = self
                    .owned
                    .session_store
                    .get_session(&room)?
                    .owner_user_id()
                    .to_owned();
                (room, owner)
            }
            None if command.external_grant_id().is_some() => {
                (String::new(), self.kernel_owner_user_id())
            }
            None => {
                return Err(acquisition_error(
                    "approval_required",
                    "An agent allocation needs an active Room for the owner's approval; request it from your Room",
                ));
            }
        };
        Ok(Some(AgentCaller {
            room,
            owner,
            agent: agent.map(str::to_owned),
        }))
    }

    /// Who asks for what: the Room and agent, the id of the owner's card, and the identity under
    /// which its approved request is first sent. None when no owner decision applies.
    fn acquisition_subject(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
    ) -> Result<Option<AcquisitionSubject>, DaemonError> {
        let (ControlPlaneRequest::Acquire { request_id, .. }
        | ControlPlaneRequest::AcquireJob { request_id, .. }) = request
        else {
            return Ok(None);
        };
        let Some(AgentCaller { room, owner, agent }) = self.agent_caller(command)? else {
            return Ok(None);
        };
        let agent = agent.as_deref();
        let digest = crate::runtime::command::KernelCommand::from_local_request(
            request_id,
            None,
            None,
            &LocalDaemonRequest::ControlPlane(request.clone()),
        )
        .durable_request_fingerprint();
        let id = format!(
            "{ACQUISITION_DECISION_PREFIX}{:x}",
            sha2::Sha256::digest(format!("{digest}:{}", agent.unwrap_or_default()).as_bytes())
        );
        let identity = format!(
            "{ACQUISITION_SENT_PREFIX}{:x}",
            sha2::Sha256::digest(
                format!("{room}\0{}\0{request_id}", agent.unwrap_or_default()).as_bytes()
            )
        );
        Ok(Some(AcquisitionSubject {
            room,
            owner,
            agent: agent.map(str::to_owned),
            request_id: request_id.clone(),
            id,
            identity,
        }))
    }

    /// The service has answered (or failed to) the request the owner approved: its receipt closes.
    pub(crate) fn machine_approval_sent(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
    ) {
        let Ok(Some(subject)) = self.acquisition_subject(command, request) else {
            return;
        };
        if let Err(error) = self.change_machine_receipts(&subject.room, |session| {
            session.mark_machine_approval_sent(&subject.id)
        }) {
            tracing::warn!(%error, "could not close the machine approval receipt");
        }
    }

    /// Holds the approved request's "Approving" receipt until the try ends. Dropped before the
    /// post (a failed recheck, a cancelled caller) it withdraws the receipt; after the post began
    /// it settles it as sent, however the await ends.
    pub(crate) fn machine_approval_guard(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
    ) -> Option<MachineApprovalGuard> {
        let subject = self.acquisition_subject(command, request).ok().flatten()?;
        Some(MachineApprovalGuard {
            state: self.clone(),
            room: subject.room,
            id: subject.id,
            posting: false,
        })
    }

    /// A restart ended every try that was waiting on the service: no "Approving" line outlives it.
    pub(crate) fn withdraw_interrupted_machine_approvals(&self) {
        for session in self.owned.session_store.list_all_sessions() {
            if session
                .machine_approval_receipts()
                .iter()
                .any(|receipt| receipt.state == MachineApprovalReceiptState::Approving)
            {
                let room = session.id().to_string();
                if let Err(error) = self.change_machine_receipts(&room, |session| {
                    session.withdraw_machine_approval(None)
                }) {
                    tracing::warn!(%error, "could not withdraw an interrupted machine approval");
                }
            }
        }
    }

    fn save_machine_receipt(&self, room: &str, receipt: MachineApprovalReceipt) {
        if let Err(error) = self.change_machine_receipts(room, |session| {
            session.upsert_machine_approval_receipt(receipt);
            true
        }) {
            tracing::warn!(%error, "could not save the machine approval receipt");
        }
    }

    fn change_machine_receipts(
        &self,
        room: &str,
        change: impl FnOnce(&mut crate::session::RuntimeSession) -> bool,
    ) -> Result<(), DaemonError> {
        if room.is_empty() {
            // A grant's decision has no Room to show a receipt in.
            return Ok(());
        }
        let mut sessions = self.owned.session_store.write();
        let mut session = sessions.get_session(room)?;
        if change(&mut session) {
            sessions.restore_session(session);
            drop(sessions);
            self.owned.session_snapshot(room)?;
            self.owned
                .terminal_stream
                .notify_terminal_projection_change(room);
        }
        Ok(())
    }
}

pub(crate) struct MachineApprovalGuard {
    state: KernelRuntimeState,
    room: String,
    id: String,
    posting: bool,
}

impl MachineApprovalGuard {
    /// The request is about to go to the service.
    pub(crate) fn posting(&mut self) {
        self.posting = true;
    }
}

impl Drop for MachineApprovalGuard {
    fn drop(&mut self) {
        let (id, posting) = (self.id.clone(), self.posting);
        if let Err(error) = self.state.change_machine_receipts(&self.room, |session| {
            if posting {
                session.mark_machine_approval_sent(&id)
            } else {
                session.withdraw_machine_approval(Some(&id))
            }
        }) {
            tracing::warn!(%error, "could not settle the machine approval receipt");
        }
    }
}

/// The agent a request comes from (see `agent_caller`).
pub(crate) struct AgentCaller {
    pub(crate) room: String,
    pub(crate) owner: String,
    pub(crate) agent: Option<String>,
}

/// Who asks for what (see `acquisition_subject`).
struct AcquisitionSubject {
    room: String,
    owner: String,
    agent: Option<String>,
    request_id: String,
    id: String,
    identity: String,
}
/// The card's title: who asks, and the purpose it gave, as one sentence the card's message follows.
fn machine_request_title(agent: Option<String>, purpose: Option<String>) -> String {
    let who = agent.unwrap_or_else(|| "An agent".into());
    match purpose {
        Some(purpose) => {
            let end = if purpose.ends_with(['.', '?', '!']) {
                ""
            } else {
                "."
            };
            format!("{who} asks for a machine. Purpose: {purpose}{end}")
        }
        None => format!("{who} asks for a machine and gave no purpose."),
    }
}

/// The purpose an agent gave with one acquire, readable by the owner's card while the request is in flight.
struct AcquirePurpose(Option<(KernelRuntimeState, String)>);
impl AcquirePurpose {
    /// One line of at most 200 characters; an empty purpose is none.
    fn hold(state: &KernelRuntimeState, request_id: &str, purpose: Option<&str>) -> Self {
        let purpose = purpose
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| !text.is_empty())
            .map(|text| match text.char_indices().nth(200) {
                Some((end, _)) => format!("{}…", &text[..end]),
                None => text,
            });
        let Some(purpose) = purpose else {
            return Self(None);
        };
        state
            .owned
            .acquire_purposes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(request_id.to_owned(), purpose);
        Self(Some((state.clone(), request_id.to_owned())))
    }
}
impl Drop for AcquirePurpose {
    fn drop(&mut self) {
        if let Some((state, request_id)) = self.0.take() {
            state
                .owned
                .acquire_purposes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&request_id);
        }
    }
}

/// Dropping a waiting job withdraws its decision before an agent can leave a stale stripe.
struct PendingAcquisition(Option<(KernelRuntimeState, String, String)>);
impl Drop for PendingAcquisition {
    fn drop(&mut self) {
        if let Some((state, room, id)) = self.0.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = state.timeout_runtime_interaction(&room, &id).await;
                });
            }
        }
    }
}
/// The account's limits, when the connected service has them: `None` otherwise.
async fn read_account_limits<P, F>(
    capability: &MachineAllocationCapability,
    plane: &P,
) -> Result<Option<limits::AccountLimits>, DaemonError>
where
    P: Fn(ControlPlaneRequest, String) -> F,
    F: std::future::Future<Output = Result<crate::local::ControlPlaneResponse, DaemonError>>,
{
    if !capability.actions.iter().any(|action| action == "limits") {
        return Ok(None);
    }
    let response = plane(ControlPlaneRequest::Limits {}, "machines:limits".into()).await?;
    limits::AccountLimits::from_response(&response)
        .map(Some)
        .ok_or_else(|| {
            acquisition_error(
                "invalid_response",
                "The service returned no account limits",
            )
        })
}
fn money(amount_micros: u64) -> String {
    format!("{:.6}", amount_micros as f64 / 1_000_000.0)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
/// The service does not know the allocation: say so and where a valid one comes from.
fn unknown_allocation(error: DaemonError, allocation_ref: &str) -> DaemonError {
    if crate::runtime::cloud_api_client::cloud_error_code(&error) == Some("not_found") {
        return acquisition_error(
            "not_found",
            &format!(
                "No machine {allocation_ref} belongs to this account; use the allocation_ref an acquire returned"
            ),
        );
    }
    error
}
pub(super) fn acquisition_error(code: &str, message: &str) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

pub(crate) fn machine_tool_specs() -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
    use crate::transport::runtime_tools::RuntimeToolSpec;
    let schema = |properties, required| serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    [
        ("chariox.machines.options", "List the machines you can acquire now, as the connected service offers them: class_id, region, size, rate per minute in the account's currency and whether it is in stock. Pick one for chariox.machines.acquire. An option that is not available cannot be acquired.", schema(serde_json::json!({}),serde_json::json!([]))),
        ("chariox.machines.acquire", "Acquire a machine through Chariox for work you need to run elsewhere. Pass class_id (and region when the class is offered in several) from chariox.machines.options to acquire exactly that option; without it the first option in stock is acquired. A named option out of stock is refused with the options in stock, never replaced. The kernel asks the owner first unless the owner lets agents acquire without asking: within the owner's limits (allowed classes, machines at once, daily spending; service limits always apply) it goes ahead with no card (the Room records class, rate and cost); outside them the owner's card says why and the call waits up to 5 minutes for it. Returns allocation_ref, machine type and terms; poll status until access is available, then reach the kernel with Room commands. timeout_s is the machine's hard stop: the service stops the machine then; release earlier when done. Reuse request_id when retrying after a lost response, a timeout or a refusal: it keeps the first try's option and returns the allocation the service accepted. Use a new one only when a refusal says the service closed the old one. Say in purpose, in one line, what the machine is for; the card shows it.", schema(serde_json::json!({"request_id":{"type":"string"},"class_id":{"type":"string"},"region":{"type":"string"},"timeout_s":{"type":"integer","minimum":1},"purpose":{"type":"string"}}),serde_json::json!([]))),
        ("chariox.machines.status", "Read an allocation's state, its verified target kernel and the installed build toolchain version when the target answers. Use the allocation_ref returned by acquire. The machine remains charged until deletion is confirmed.", schema(serde_json::json!({"allocation_ref":{"type":"string"}}),serde_json::json!(["allocation_ref"]))),
        ("chariox.machines.release", "Delete an acquired machine when its work is finished. Returns the allocation state; poll status until deleted to confirm the allocation has ended.", schema(serde_json::json!({"allocation_ref":{"type":"string"}}),serde_json::json!(["allocation_ref"]))),
    ].into_iter().map(|(name,description,input_schema)|RuntimeToolSpec{name:name.into(),description:description.into(),input_schema}).collect()
}

/// The account's limits tools: listed only while the kernel has a connected account, like enrolling.
pub(crate) fn limits_tool_specs() -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
    use crate::transport::runtime_tools::RuntimeToolSpec;
    let schema = |properties, required| serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    [
        ("chariox.machines.limits", "Read the account's machine limits (machines at once, spend per day) and the owner's limits for acquiring without asking.", schema(serde_json::json!({}),serde_json::json!([]))),
        ("chariox.machines.set_limits", "Change a limit. A lower applies at once; a raise needs the person: their /sudo naming it, or a card you raise showing the old and new values (the call waits for it). scope account (default) or no_ask; spend in millionths.", schema(serde_json::json!({"scope":{"type":"string","enum":["account","no_ask"]},"machines_at_once":{"type":"integer","minimum":1},"spend_per_day_micros":{"type":"integer","minimum":0}}),serde_json::json!([]))),
    ].into_iter().map(|(name,description,input_schema)|RuntimeToolSpec{name:name.into(),description:description.into(),input_schema}).collect()
}
impl KernelRuntimeState {
    pub(crate) async fn machine_tool(
        &self,
        token: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            request_id: Option<String>,
            allocation_ref: Option<String>,
            class_id: Option<String>,
            region: Option<String>,
            timeout_s: Option<u64>,
            purpose: Option<String>,
            scope: Option<crate::local::LimitScope>,
            machines_at_once: Option<u32>,
            spend_per_day_micros: Option<u64>,
        }
        let args: Args = serde_json::from_value(args).map_err(|_| {
            acquisition_error(
                "invalid_request",
                "Use request_id, class_id, region, timeout_s and purpose for acquire, allocation_ref for status and release, scope, machines_at_once and spend_per_day_micros for set_limits, and no arguments for options and limits",
            )
        })?;
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(acquisition_error(
                "not_allowed",
                "Machine tools need one active agent",
            ));
        };
        let reach = self
            .reach_for_id(&format!("{}{}", super::reach::REACH_PREFIX, run.id()))
            .ok_or_else(|| {
                acquisition_error(
                    "not_allowed",
                    "Only a running agent of the kernel owner may acquire machines",
                )
            })?;
        let allocation_ref = || {
            args.allocation_ref
                .clone()
                .ok_or_else(|| acquisition_error("invalid_request", "allocation_ref is required"))
        };
        let request = match tool {
            "chariox.machines.options" => MachineAcquisitionRequest::Options {},
            "chariox.machines.acquire" => MachineAcquisitionRequest::Acquire {
                request_id: args.request_id.clone(),
                class_id: args.class_id.clone(),
                region: args.region.clone(),
                timeout_s: args.timeout_s,
                purpose: args.purpose.clone(),
            },
            "chariox.machines.status" => MachineAcquisitionRequest::Status {
                allocation_ref: allocation_ref()?,
            },
            "chariox.machines.release" => MachineAcquisitionRequest::Release {
                allocation_ref: allocation_ref()?,
            },
            "chariox.machines.limits" => MachineAcquisitionRequest::Limits {},
            "chariox.machines.set_limits" => MachineAcquisitionRequest::SetLimits {
                scope: args.scope.unwrap_or_default(),
                machines_at_once: args.machines_at_once,
                spend_per_day_micros: args.spend_per_day_micros,
            },
            _ => {
                return Err(acquisition_error(
                    "invalid_request",
                    "Unknown machine action",
                ));
            }
        };
        let origin = &reach.origin();
        let caller = self
            .owned
            .agent_store
            .get_agent(&reach.agent_id)
            .ok()
            .map(|agent| AgentCaller {
                room: agent.session_id().to_owned(),
                owner: reach.owner_user_id.clone(),
                agent: Some(reach.agent_id.clone()),
            });
        let result = self
            .machine_acquire(&reach.agent_id, request, caller, |request, command_id| async move {
                self.job_control_plane(origin, &request, &command_id).await
            })
            .await?;
        // A ready machine's status also tells the agent which build toolchain its target has installed.
        let ready_target = match &result {
            MachineAcquisitionResult::Machine(crate::local::ControlPlaneResponse::Machine {
                state,
                kernel_target,
                ..
            }) if tool == "chariox.machines.status" && state == "ready" => kernel_target.clone(),
            _ => None,
        };
        let mut payload = serde_json::to_value(result)
            .map_err(|e| acquisition_error("invalid_response", &e.to_string()))?;
        if let Some(target) = ready_target {
            payload["build_toolchain"] = self
                .target_build_toolchain(&target.kernel_id, &origin.harness)
                .await;
        }
        Ok(crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload })
    }

    /// Options, acquire, status and release for any caller: `key` names who asked (an agent, or the
    /// owner) for the request-id rows, and `plane` sends one request to the connected service
    /// under that caller's authority (and its owner card). The owner CLI, an agent acting through
    /// a grant and a Room agent all acquire through here.
    pub(crate) async fn machine_acquire<P, F>(
        &self,
        key: &str,
        request: MachineAcquisitionRequest,
        caller: Option<AgentCaller>,
        plane: P,
    ) -> Result<MachineAcquisitionResult, DaemonError>
    where
        P: Fn(ControlPlaneRequest, String) -> F,
        F: std::future::Future<Output = Result<crate::local::ControlPlaneResponse, DaemonError>>,
    {
        let caps = plane(
            ControlPlaneRequest::Capabilities {},
            "machines:capabilities".into(),
        )
        .await?;
        let crate::local::ControlPlaneResponse::Capabilities {
            machine_allocation: Some(capability),
        } = caps
        else {
            return Err(acquisition_error(
                "capacity_unavailable",
                "This kernel has no connected machine service",
            ));
        };
        match request {
            MachineAcquisitionRequest::Options {} => Ok(MachineAcquisitionResult::Options {
                options: capability.offers,
                default: capability.offer,
            }),
            MachineAcquisitionRequest::Limits {} => {
                let account = read_account_limits(&capability, &plane).await?;
                Ok(MachineAcquisitionResult::Limits {
                    limits: self.limits_reading(account.as_ref()),
                })
            }
            MachineAcquisitionRequest::SetLimits {
                scope,
                machines_at_once,
                spend_per_day_micros,
            } => {
                if machines_at_once.is_none() && spend_per_day_micros.is_none() {
                    return Err(acquisition_error(
                        "invalid_request",
                        "Name machines_at_once or spend_per_day_micros to change",
                    ));
                }
                let mut account = read_account_limits(&capability, &plane).await?;
                match scope {
                    crate::local::LimitScope::Account => {
                        let current = account.clone().ok_or_else(limits::no_account_limits)?;
                        let to = crate::local::AccountLimitValues {
                            machines_at_once: machines_at_once
                                .unwrap_or(current.values.machines_at_once),
                            spend_per_day_micros: spend_per_day_micros
                                .unwrap_or(current.values.spend_per_day_micros),
                        };
                        if to != current.values {
                            let response = plane(
                                ControlPlaneRequest::SetLimits {
                                    expected_version: current.version,
                                    machines_at_once: to.machines_at_once,
                                    spend_per_day_micros: to.spend_per_day_micros,
                                    approval: None,
                                },
                                "machines:set_limits".into(),
                            )
                            .await?;
                            account = Some(limits::AccountLimits::from_response(&response).ok_or_else(
                                || {
                                    acquisition_error(
                                        "invalid_response",
                                        "The service returned no account limits",
                                    )
                                },
                            )?);
                        }
                    }
                    crate::local::LimitScope::NoAsk => {
                        self.change_no_ask_limits(
                            caller,
                            machines_at_once,
                            spend_per_day_micros,
                            account.as_ref(),
                        )
                        .await?;
                    }
                }
                Ok(MachineAcquisitionResult::Limits {
                    limits: self.limits_reading(account.as_ref()),
                })
            }
            MachineAcquisitionRequest::Acquire {
                request_id,
                class_id,
                region,
                timeout_s,
                purpose,
            } => {
                let request_id = request_id
                    .unwrap_or_else(|| format!("machine-{:032x}", rand::random::<u128>()));
                let option =
                    self.acquire_option(key, &request_id, &capability, class_id, region)?;
                let service_request_id = choice::service_request_id(
                    &self.owned.config_projection.snapshot().daemon_id,
                    key,
                    &request_id,
                );
                let _purpose = AcquirePurpose::hold(self, &service_request_id, purpose.as_deref());
                let allocation = match plane(
                    ControlPlaneRequest::Acquire {
                        capability_revision: capability.revision.clone(),
                        request_id: service_request_id.clone(),
                        class_id: option.class_id().map(str::to_owned),
                        region: option.region().map(str::to_owned),
                        timeout_s,
                    },
                    format!("machine:{service_request_id}:acquire"),
                )
                .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        self.close_acquire_option(key, &request_id, &option, &error)?;
                        // The caller retries with its own request_id, not the service's derived one.
                        let error = without_asking::name_agent_request(
                            error,
                            &service_request_id,
                            &request_id,
                        );
                        return Err(choice::acquire_refusal(
                            error,
                            &capability,
                            &option,
                            &request_id,
                        ));
                    }
                };
                let crate::local::ControlPlaneResponse::Machine { allocation_ref, .. } =
                    &allocation
                else {
                    return Err(acquisition_error(
                        "invalid_response",
                        "The service returned no allocation",
                    ));
                };
                self.accept_acquire_option(key, &request_id, &option, allocation_ref)?;
                // The service decides whether the choice can be acquired, and replays a retry of one it accepted.
                let chosen = match option {
                    AcquireOption::Named { class_id, region } => MachineAcquisitionChoice::Option {
                        option: capability.offers.into_iter().find(|option| {
                            option.class_id == class_id
                                && region
                                    .as_ref()
                                    .is_none_or(|region| &option.region == region)
                        }),
                    },
                    AcquireOption::ServiceDefault => MachineAcquisitionChoice::ServiceDefault {
                        offer: capability.offer,
                    },
                };
                Ok(MachineAcquisitionResult::Acquired {
                    allocation,
                    chosen,
                    estimate: MachineAcquisitionEstimate::unknown(),
                })
            }
            MachineAcquisitionRequest::Status { allocation_ref } => {
                Ok(MachineAcquisitionResult::Machine(
                    plane(
                        ControlPlaneRequest::Get {
                            allocation_ref: allocation_ref.clone(),
                        },
                        "machines:status".into(),
                    )
                    .await
                    .map_err(|error| unknown_allocation(error, &allocation_ref))?,
                ))
            }
            MachineAcquisitionRequest::Release { allocation_ref } => {
                let current = plane(
                    ControlPlaneRequest::Get {
                        allocation_ref: allocation_ref.clone(),
                    },
                    "machines:release:get".into(),
                )
                .await
                .map_err(|error| unknown_allocation(error, &allocation_ref))?;
                let crate::local::ControlPlaneResponse::Machine { revision, .. } = current else {
                    return Err(acquisition_error(
                        "invalid_response",
                        "The service returned another allocation",
                    ));
                };
                Ok(MachineAcquisitionResult::Machine(
                    plane(
                        ControlPlaneRequest::Release {
                            allocation_ref: allocation_ref.clone(),
                            capability_revision: capability.revision,
                            request_id: format!("release:{allocation_ref}"),
                            expected_revision: revision,
                        },
                        "machines:release".into(),
                    )
                    .await?,
                ))
            }
        }
    }
}

#[cfg(test)]
mod acquire_tests;
mod build_toolchain;
mod choice;
pub(crate) mod limits;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(in crate::runtime::state) mod tool_tests;
pub(crate) mod without_asking;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod without_asking_tests;
