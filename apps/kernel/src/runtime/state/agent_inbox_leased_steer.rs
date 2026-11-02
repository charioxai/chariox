//! MP-08 / MP-10 / MP-11 A10: an urgent inbox event steers a busy leased
//! agent's running worker turn, as it steers a local agent's provider turn.
use super::*;
use crate::durable_state::agent_lifecycle::{Operation, Outcome};

impl KernelRuntimeState {
    /// Steers `prompt` into `active`, the leased agent's running home turn.
    /// A steer that provably never reached that turn returns the event to the
    /// inbox for the agent's next turn; an unknown outcome stays uncertain until
    /// the worker's exact receipt settles it, so the event is never run twice.
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
        self.record_agent_delivery_receipt(room, agent, sequence, receipt)?;
        if receipt == "uncertain" {
            self.spawn_leased_inbox_steer_reconciliation(
                room.to_string(),
                agent.to_string(),
                prompt.id().to_string(),
            );
        }
        Ok(())
    }

    /// Asks the worker for the exact receipt of an uncertain steer until it
    /// answers. The worker keys its receipts by steer id, so the query never
    /// applies the steer again; one it never received is answered as rejected.
    pub(super) fn spawn_leased_inbox_steer_reconciliation(
        &self,
        room: String,
        agent: String,
        steer_id: String,
    ) {
        let key = (room.clone(), agent.clone(), steer_id.clone());
        if !self
            .owned
            .remote_steer_receipt_reconciliations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.clone())
        {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            let mut attempt = 0_u64;
            let mut delay = std::time::Duration::from_secs(1);
            loop {
                match state
                    .reconcile_leased_inbox_steer_once(&room, &agent, &steer_id)
                    .await
                {
                    Ok(LeasedInboxSteerReconciliation::Settled) => break,
                    Ok(LeasedInboxSteerReconciliation::Pending) => {}
                    Err(error) => {
                        if attempt.is_multiple_of(30) {
                            crate::logging::warn_with_fields(
                                "daemon.agent_lifecycle",
                                "uncertain leased inbox steer is waiting for its exact worker receipt",
                                serde_json::json!({
                                    "session_id": room,
                                    "agent_id": agent,
                                    "steer_id": steer_id,
                                    "error": crate::secret_redaction::redact_secrets(&error.to_string()),
                                }),
                            );
                        }
                    }
                }
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(std::time::Duration::from_secs(30));
            }
            state
                .owned
                .remote_steer_receipt_reconciliations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
        });
    }

    async fn reconcile_leased_inbox_steer_once(
        &self,
        room: &str,
        agent: &str,
        steer_id: &str,
    ) -> Result<LeasedInboxSteerReconciliation, DaemonError> {
        let Some(event) = self
            .owned
            .durable_state_store
            .agent_event_for_prompt(room, agent, steer_id)?
            .filter(|event| matches!(event.state.as_str(), "uncertain" | "blocked"))
        else {
            return Ok(LeasedInboxSteerReconciliation::Settled);
        };
        let (Some(target), Some(run)) = (
            event.target_prompt_id.clone(),
            event.provider_run_id.clone(),
        ) else {
            return Ok(LeasedInboxSteerReconciliation::Settled);
        };
        let binding = self
            .owned
            .agent_store
            .get_agent(agent)?
            .remote_execution()
            .cloned();
        // The projected run names the leased agent, and so the lease, that took the steer.
        let Some((binding, worker_run)) = binding.and_then(|binding| {
            let worker_run = crate::provider::worker_provider_run_id_from_projected_leased_id(
                &binding.leased_agent_id,
                &run,
            )?;
            Some((binding, worker_run))
        }) else {
            crate::logging::warn_with_fields(
                "daemon.agent_lifecycle",
                "the lease that took an uncertain inbox steer is gone; the owner resolves the delivery",
                serde_json::json!({"session_id": room, "agent_id": agent, "steer_id": steer_id, "sequence": event.sequence}),
            );
            return Ok(LeasedInboxSteerReconciliation::Settled);
        };
        let Some(receipt) =
            super::remote_prompt_worker_submission_runtime::query_remote_queued_steer_receipt(
                self,
                agent,
                steer_id,
                &binding.worker_kernel_id,
                &binding.worker_machine_id,
                &binding.leased_agent_id,
                &target,
                &worker_run,
                &binding.execution_lease_id,
            )
            .await?
        else {
            return Ok(LeasedInboxSteerReconciliation::Pending);
        };
        if receipt.worker_provider_run_id != worker_run
            || receipt.target_home_prompt_id.as_deref() != Some(target.as_str())
            || receipt.execution_lease_id.as_deref() != Some(binding.execution_lease_id.as_str())
        {
            return Err(DaemonError::LocalTransport {
                operation: "reconcile leased inbox steer",
                message: format!(
                    "worker receipt for steer `{steer_id}` did not match its run, target and lease"
                ),
            });
        }
        let receipt = match receipt.phase {
            crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerAccepted => {
                self.owned
                    .durable_state_store
                    .agent_lifecycle(Operation::BindSubmission {
                        room: room.into(),
                        agent: agent.into(),
                        sequence: event.sequence,
                        prompt: steer_id.into(),
                        target: Some(target),
                        run,
                        submit_epoch: 0,
                        now: crate::session::unix_epoch_ms(),
                    })?;
                "accepted"
            }
            crate::transport::relay_peer::LeasedPromptReceiptPhase::SteerRejected => "rejected",
            _ => return Ok(LeasedInboxSteerReconciliation::Pending),
        };
        self.record_agent_delivery_receipt(room, agent, event.sequence, receipt)?;
        self.schedule_agent_lifecycle_sweep();
        Ok(LeasedInboxSteerReconciliation::Settled)
    }
}

enum LeasedInboxSteerReconciliation {
    Settled,
    Pending,
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
