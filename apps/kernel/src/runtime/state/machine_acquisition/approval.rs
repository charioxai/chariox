//! An approval is bound to the Room, asking agent and acquisition, even as service revisions change.
use super::*;
use crate::runtime::command::KernelCommand;
use crate::session::{
    MachineApprovalReceipt, MachineApprovalReceiptState, RuntimeInteraction,
    RuntimeInteractionChoice, RuntimeInteractionChoiceStyle, RuntimeSession,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;

const PREFIX: &str = "machine-acquisition-";
const DIRECT_PREFIX: &str = "machine-acquisition-direct-";
pub(in crate::runtime::state) const DECISION_EVENT: &str = "compute.acquisition_decided";
const DECISION_SECONDS: u64 = 300;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(in crate::runtime::state) enum MachineDecision {
    Approved { quote: String },
    Denied { person: String, at_ms: u64 },
    Expired { at_ms: u64 },
}

fn approval_id(
    request: &ControlPlaneRequest,
    room: Option<&str>,
    agent: Option<&str>,
) -> Option<String> {
    let (prefix, request_id) = match request {
        ControlPlaneRequest::Acquire { request_id, .. } => (DIRECT_PREFIX, request_id),
        ControlPlaneRequest::AcquireJob { request_id, .. } => (PREFIX, request_id),
        _ => return None,
    };
    Some(approval_id_for_request(prefix, room, agent, request_id))
}

fn approval_id_for_request(
    prefix: &str,
    room: Option<&str>,
    agent: Option<&str>,
    request_id: &str,
) -> String {
    let encoded = serde_json::to_vec(&(room, agent, request_id))
        .expect("machine acquisition identity serializes");
    format!("{prefix}{:x}", sha2::Sha256::digest(encoded))
}

pub(super) fn direct_approval_id(room: &str, agent: &str, request_id: &str) -> String {
    approval_id_for_request(DIRECT_PREFIX, Some(room), Some(agent), request_id)
}

pub(in crate::runtime::state) fn is_machine_approval(card: &RuntimeInteraction) -> bool {
    card.kernel_operation_id()
        .is_some_and(|id| id == card.id() && id.starts_with(PREFIX))
}

pub(in crate::runtime::state) fn timestamp(at_ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(at_ms as i64)
        .expect("machine approval timestamp")
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(super) fn decision_error(decision: &MachineDecision) -> DaemonError {
    match decision {
        MachineDecision::Denied { person, at_ms } => acquisition_error(
            "acquisition_denied",
            &format!("Denied by {person} at {}. The request was not sent to the service. Use a new request_id to ask again", timestamp(*at_ms)),
        ),
        MachineDecision::Expired { at_ms } => acquisition_error(
            "acquisition_expired",
            &format!("Expired at {} without an answer. Use a new request_id to ask again", timestamp(*at_ms)),
        ),
        MachineDecision::Approved { .. } => acquisition_error(
            "acquisition_offer_changed",
            "The machine offer changed since approval. Use a new request_id to ask again at the current rate",
        ),
    }
}

pub(in crate::runtime::state) fn decision_deadline(card: &RuntimeInteraction) -> u64 {
    card.requested_at_ms()
        .saturating_add(DECISION_SECONDS * 1000)
}

impl KernelRuntimeOwnedState {
    pub(super) fn machine_decision(
        &self,
        id: &str,
        owner: &str,
    ) -> Result<Option<MachineDecision>, DaemonError> {
        self.durable_state_store
            .load_subject_events_by_kind(id, DECISION_EVENT, 1)?
            .into_iter()
            .find(|event| event.payload["owner"] == owner)
            .map(|event| {
                serde_json::from_value(event.payload["decision"].clone()).map_err(|_| {
                    acquisition_error(
                        "acquisition_unavailable",
                        "Machine approval history is unreadable",
                    )
                })
            })
            .transpose()
    }

    pub(in crate::runtime::state) fn machine_decision_at(
        &self,
        card: &RuntimeInteraction,
        status: &str,
        choice: Option<&str>,
        user_id: Option<&str>,
        now: u64,
    ) -> Option<MachineDecision> {
        if !is_machine_approval(card) {
            return None;
        }
        match (status, choice) {
            ("answered", Some("approve")) => Some(MachineDecision::Approved {
                quote: card.message().into(),
            }),
            ("answered", Some("deny")) => {
                let config = self.config_projection.snapshot();
                let person = user_id
                    .map(|id| {
                        crate::runtime::cloud_relay_control::active_cloud_profile(&config)
                            .filter(|profile| profile.user_id == id && !profile.email.is_empty())
                            .map_or(id, |profile| profile.email.as_str())
                    })
                    .unwrap_or("the owner");
                Some(MachineDecision::Denied {
                    person: person.into(),
                    at_ms: now,
                })
            }
            ("timed_out", _) if now >= decision_deadline(card) => Some(MachineDecision::Expired {
                at_ms: decision_deadline(card),
            }),
            _ => None,
        }
    }

    pub(in crate::runtime::state) fn save_machine_decision(
        &self,
        session: &mut RuntimeSession,
        card: &RuntimeInteraction,
        decision: MachineDecision,
    ) -> Result<(), DaemonError> {
        // A task-owned decision is already committed with its wake result.
        if card.asking_task_id().is_none() {
            self.durable_state_store.append_event(
                DECISION_EVENT,
                Some(card.id().to_owned()),
                serde_json::json!({"owner":session.owner_user_id(),"decision":decision}),
            )?;
        }
        let (state, expires_at_ms) = match decision {
            MachineDecision::Approved { .. } => (MachineApprovalReceiptState::Approving, None),
            MachineDecision::Expired { at_ms } => {
                (MachineApprovalReceiptState::Expired, Some(at_ms))
            }
            MachineDecision::Denied { .. } => return Ok(()),
        };
        session.upsert_machine_approval_receipt(MachineApprovalReceipt {
            id: card.id().into(),
            shown_in_agent_id: card
                .shown_in_agent_id()
                .or(card.related_agent_id())
                .map(str::to_owned),
            message: card.message().into(),
            state,
            expires_at_ms,
        });
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(crate) fn machine_approval_sent(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
    ) -> Result<(), DaemonError> {
        let Some(id) = self.machine_approval_id(command, request) else {
            return Ok(());
        };
        let (Some(room), _) = self.machine_approval_subject(command) else {
            return Ok(());
        };
        let mut sessions = self.owned.session_store.write();
        let mut session = sessions.get_session(&room)?;
        if session.mark_machine_approval_sent(&id) {
            sessions.restore_session(session);
            drop(sessions);
            self.owned.session_snapshot(&room)?;
            self.owned
                .terminal_stream
                .notify_terminal_projection_change(&room);
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(crate) async fn approve_machine_acquisition(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
        capability: Option<&MachineAllocationCapability>,
    ) -> Result<Option<crate::local::ControlPlaneResponse>, DaemonError> {
        let Some(id) = self.machine_approval_id(command, request) else {
            return Ok(None);
        };
        let (room, agent) = self.machine_approval_subject(command);
        let automatic = self
            .owned
            .config_projection
            .snapshot()
            .user_config
            .agents
            .offload_without_asking
            == Some(true);
        let Some(room) = room else {
            return if automatic {
                Ok(None)
            } else {
                Err(acquisition_error(
                    "approval_required",
                    "Ask from an active Room so its owner can approve the machine",
                ))
            };
        };
        let session = self.owned.session_store.get_session(&room)?;
        let mut decision = self.owned.machine_decision(&id, session.owner_user_id())?;
        let direct = matches!(request, ControlPlaneRequest::Acquire { .. })
            && command.reach_caller_id().is_some();
        if direct && decision.is_none() {
            if let Some(wake) = self.owned.durable_state_store.agent_wake(&id)? {
                decision = self.owned.machine_decision(&id, session.owner_user_id())?;
                if decision.is_none() {
                    return self.pending_machine_acquisition(&wake, &room, agent.as_deref());
                }
            }
        }
        if let Some(decision @ (MachineDecision::Denied { .. } | MachineDecision::Expired { .. })) =
            decision.as_ref()
        {
            return Err(decision_error(decision));
        }
        let task_authorized = match agent.as_deref() {
            Some(agent) if !automatic => self.sudo_task_authorizes_agent(&room, agent).await,
            _ => false,
        };
        if decision.is_none() && (automatic || task_authorized) {
            return Ok(None);
        }
        let description = super::quote::approval_quote(request, capability)?;
        if let Some(decision) = decision {
            return approved_quote(decision, &description);
        }
        let mut interaction = RuntimeInteraction::for_kernel_operation(
            &id,
            &id,
            if matches!(request, ControlPlaneRequest::Acquire { .. }) {
                "Acquire a machine"
            } else {
                "Run this job on a machine"
            },
            &description,
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
        interaction = interaction.with_timeout_sec(DECISION_SECONDS);
        if let Some(agent) = agent.as_deref() {
            interaction = interaction.with_related_agent_id(agent.into());
        }
        if direct {
            let agent = agent.as_deref().ok_or_else(|| {
                acquisition_error("approval_required", "Ask from a running agent in a Room")
            })?;
            let armed = self.arm_interaction_wake(&room, agent, interaction);
            if let Some(decision) = self.owned.machine_decision(&id, session.owner_user_id())? {
                return approved_quote(decision, &description);
            }
            if let Err(error) = armed {
                if let Some(wake) = self.owned.durable_state_store.agent_wake(&id)? {
                    if matches!(wake.state.as_str(), "starting" | "running") {
                        return self.pending_machine_acquisition(&wake, &room, Some(agent));
                    }
                }
                return Err(error);
            }
            let wake = self
                .owned
                .durable_state_store
                .agent_wake(&id)?
                .ok_or_else(|| {
                    acquisition_error(
                        "acquisition_unavailable",
                        "The approval wait could not be saved. Ask again",
                    )
                })?;
            return self.pending_machine_acquisition(&wake, &room, Some(agent));
        }
        let receiver = self
            .create_kernel_operation_interaction(&room, session.owner_user_id(), interaction)
            .await?;
        let mut pending = PendingAcquisition(Some((self.clone(), room.clone(), id.clone())));
        let answer = receiver.await;
        pending.0 = None;
        if let Some(decision) = self.owned.machine_decision(&id, session.owner_user_id())? {
            return match decision {
                MachineDecision::Approved { .. } => Ok(None),
                _ => Err(decision_error(&decision)),
            };
        }
        Err(acquisition_error(
            "acquisition_unavailable",
            if answer.is_err() {
                "The approval wait closed before an answer arrived. Ask again"
            } else {
                "The approval ended before an answer arrived. Ask again"
            },
        ))
    }

    fn pending_machine_acquisition(
        &self,
        wake: &crate::durable_state::agent_lifecycle::AgentWake,
        room: &str,
        agent: Option<&str>,
    ) -> Result<Option<crate::local::ControlPlaneResponse>, DaemonError> {
        if wake.room_id != room
            || Some(wake.agent_id.as_str()) != agent
            || !matches!(wake.state.as_str(), "starting" | "running")
        {
            return Err(acquisition_error(
                "acquisition_unavailable",
                "The approval wait ended. Use a new request_id to ask again",
            ));
        }
        Ok(Some(
            crate::local::ControlPlaneResponse::AcquisitionWaiting {
                registration_id: wake.registration_id.clone(),
                expires_at_ms: wake.next_due_ms.unwrap_or(wake.created_at_ms),
            },
        ))
    }

    pub(crate) fn machine_approval_subject(
        &self,
        command: &KernelCommand,
    ) -> (Option<String>, Option<String>) {
        let agent = command
            .reach_caller_id()
            .and_then(|id| self.reach_for_id(&id))
            .map(|r| r.agent_id)
            .or_else(|| command.agent_id.clone())
            .or_else(|| command.caller.metaagent_id.clone());
        let room = agent
            .as_deref()
            .and_then(|agent| self.owned.agent_store.get_agent(agent).ok())
            .map(|agent| agent.session_id().to_owned())
            .or_else(|| command.session_id.clone());
        (room, agent)
    }

    pub(crate) fn machine_approval_id(
        &self,
        command: &KernelCommand,
        request: &ControlPlaneRequest,
    ) -> Option<String> {
        let agent_request = command.external_grant_id().is_some()
            || command.reach_caller_id().is_some()
            || command.caller.metaagent_id.is_some()
            || command.caller.connection_class
                == Some(crate::local::KernelConnectionClass::KernelAgent);
        if !agent_request {
            return None;
        }
        let (room, agent) = self.machine_approval_subject(command);
        approval_id(request, room.as_deref(), agent.as_deref())
    }
}
fn approved_quote(
    decision: MachineDecision,
    current_quote: &str,
) -> Result<Option<crate::local::ControlPlaneResponse>, DaemonError> {
    if matches!(&decision, MachineDecision::Approved { quote } if quote == current_quote) {
        Ok(None)
    } else {
        Err(decision_error(&decision))
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
