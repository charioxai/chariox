//! MP-08 / MP-10 / MP-11 A10: an urgent inbox event steers a busy leased
//! agent's running worker turn, as it steers a local agent's provider turn.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};

impl KernelRuntimeState {
    /// Steers `prompt` into `active`, the leased agent's running home turn.
    /// A steer that provably never reached that turn returns the event to the
    /// inbox for the agent's next turn; an unknown outcome stays uncertain for
    /// the delivery sweep, so the event is never run twice.
    pub(super) async fn steer_leased_inbox_event(
        &self,
        room: &str,
        agent: &str,
        sequence: u64,
        active: &crate::session::PromptQueueItem,
        prompt: crate::session::PromptQueueItem,
        work: Option<String>,
    ) -> Result<(), DaemonError> {
        let binding = self
            .owned
            .agent_store
            .get_agent(agent)?
            .remote_execution()
            .cloned();
        let steerable = binding.and_then(|binding| {
            let run = crate::provider::projected_leased_provider_run_id(
                &binding.leased_agent_id,
                binding.active_worker_provider_run_id.as_deref()?,
            );
            (active.status() == crate::session::PromptStatus::Running && !active.is_external())
                .then_some((binding, run))
        });
        let Some((binding, run)) = steerable else {
            // As for a local turn that cannot take a steer: the event waits
            // for the agent's next turn.
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Defer {
                    room: room.into(),
                    agent: agent.into(),
                    sequence,
                    now: crate::session::unix_epoch_ms(),
                })?;
            return Ok(());
        };
        let target = Some(active.id().to_string());
        let Outcome::Event(attempt) =
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::Attempt {
                    room: room.into(),
                    agent: agent.into(),
                    sequence,
                    prompt: prompt.id().into(),
                    target: target.clone(),
                    run: Some(run.clone()),
                    now: crate::session::unix_epoch_ms(),
                    work,
                })?
        else {
            unreachable!()
        };
        if attempt.state == "blocked" {
            return Ok(());
        }
        let result = self
            .steer_remote_agent_message_for_turn(room, &prompt, Some(active.id()), Some(&binding))
            .await;
        let receipt = leased_steer_receipt(&result);
        if receipt == "accepted" {
            self.owned
                .durable_state_store
                .agent_lifecycle(Operation::BindSubmission {
                    room: room.into(),
                    agent: agent.into(),
                    sequence,
                    prompt: prompt.id().into(),
                    target,
                    run,
                    submit_epoch: 0,
                    now: crate::session::unix_epoch_ms(),
                })?;
        }
        if let Err(error) = &result {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "leased inbox steer did not reach the running worker turn",
                serde_json::json!({
                    "session_id": room,
                    "agent_id": agent,
                    "sequence": sequence,
                    "receipt": receipt,
                    "error": crate::secret_redaction::redact_secrets(&error.to_string()),
                }),
            );
        }
        self.record_agent_delivery_receipt(room, agent, sequence, receipt)
    }
}

/// The steer's exact receipt. Only a refusal the home made before sending, or
/// one the worker made before it prepared the steer, proves the turn never
/// received it; any other failure may follow the worker's acceptance.
fn leased_steer_receipt(result: &Result<Option<String>, DaemonError>) -> &'static str {
    match result {
        Ok(Some(_)) => "accepted",
        Ok(None) => "rejected",
        Err(error) if leased_steer_refused_before_sending(error) => "rejected",
        Err(_) => "uncertain",
    }
}

fn leased_steer_refused_before_sending(error: &DaemonError) -> bool {
    match error {
        DaemonError::LocalTransport { operation, .. } => {
            matches!(*operation, "steer notification" | "steer agent message")
                || super::remote_prompt_worker_submission_runtime::remote_prompt_error_is_definitely_pre_send_after_transport_start(error)
        }
        DaemonError::InvalidProviderRunState { .. }
        | DaemonError::NoActiveProviderRun { .. }
        | DaemonError::NoActivePrompt { .. } => true,
        DaemonError::RelayTransport { code, .. } => {
            code == "no_active_provider_run"
                || super::remote_prompt_worker_submission_runtime::remote_prompt_error_is_known_pre_admission_rejection(error)
                || super::remote_prompt_worker_submission_runtime::remote_prompt_error_is_definitely_pre_send_after_transport_start(error)
        }
        _ => false,
    }
}
