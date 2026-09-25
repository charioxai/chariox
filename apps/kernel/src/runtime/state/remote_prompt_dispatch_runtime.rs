//! Remote prompt dispatch transport runtime.
//!
//! This module owns leased-agent prompt submission, binding refresh, and remote dispatch result
//! settlement after owned prompt state has already admitted the prompt.

use super::remote_prompt_worker_submission_runtime::{
    persist_remote_prompt_reconciliation_pending, query_remote_prompt_worker_receipt,
    remote_prompt_error_is_reconciliation_pending, remote_prompt_error_should_refresh_binding,
    remote_prompt_error_should_retry_transport, remote_prompt_reconciliation_pending,
    remote_prompt_reconciliation_pending_error, remote_prompt_transport_retry_delay,
    submit_remote_prompt_to_worker_with_binding_refresh,
};
use super::*;
use crate::transport::relay_peer::RelayPeerEvent;

const REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(5);

// Duplicate starts advance the generation so release and restart are one atomic decision.
pub(super) struct RemotePromptAgentClaim {
    key: (String, String),
    claims: Arc<std::sync::Mutex<BTreeMap<(String, String), u64>>>,
    seen_generation: u64,
    released: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum RemotePromptRunBindingRecovery {
    Recovered,
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
enum RemotePromptReceiptAction {
    AssociateActiveRun(String),
    DrainCompletedProjection(String),
}

fn remote_prompt_receipt_action(
    expected_prompt_id: &str,
    receipt: Option<crate::transport::relay_peer::LeasedPromptReceipt>,
) -> Result<RemotePromptReceiptAction, &'static str> {
    let Some(receipt) = receipt else {
        return Err("worker returned no receipt for the exact home prompt");
    };
    if receipt.home_prompt_id != expected_prompt_id {
        return Err("worker receipt names a different home prompt");
    }
    if receipt.worker_provider_run_id.trim().is_empty() {
        return Err("worker receipt has no provider-run identity");
    }
    match receipt.phase {
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Active => Ok(
            RemotePromptReceiptAction::AssociateActiveRun(receipt.worker_provider_run_id),
        ),
        crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed => Ok(
            RemotePromptReceiptAction::DrainCompletedProjection(receipt.worker_provider_run_id),
        ),
    }
}

fn remote_prompt_receipt_action_requires_projection(action: &RemotePromptReceiptAction) -> bool {
    matches!(
        action,
        RemotePromptReceiptAction::DrainCompletedProjection(_)
    )
}

impl RemotePromptAgentClaim {
    fn try_acquire(
        claims: Arc<std::sync::Mutex<BTreeMap<(String, String), u64>>>,
        session_id: &str,
        agent_id: &str,
    ) -> Option<Self> {
        let key = (session_id.to_string(), agent_id.to_string());
        let mut claims_guard = claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(generation) = claims_guard.get_mut(&key) {
            *generation = generation.saturating_add(1);
            return None;
        }
        claims_guard.insert(key.clone(), 0);
        drop(claims_guard);
        Some(Self {
            key,
            claims,
            seen_generation: 0,
            released: false,
        })
    }

    fn release_or_restart(&mut self) -> bool {
        let mut claims = self
            .claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(generation) = claims.get(&self.key).copied() else {
            self.released = true;
            return false;
        };
        if generation != self.seen_generation {
            self.seen_generation = generation;
            return true;
        }
        claims.remove(&self.key);
        self.released = true;
        false
    }
}

impl Drop for RemotePromptAgentClaim {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

impl KernelRuntimeState {
    pub(super) fn try_claim_remote_prompt_cancellation_send(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<RemotePromptAgentClaim> {
        RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            session_id,
            agent_id,
        )
    }

    pub(super) async fn recover_remote_prompt_after_kernel_restart(
        &self,
        session_id: &str,
        agent_id: &str,
        delivery_phase: Option<crate::session::DurablePromptDeliveryPhase>,
        delivery_provider_run_id: Option<&str>,
    ) -> Result<bool, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_store.get_session(session_id)?;
        let active_prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id);
        if agent.remote_execution().is_some()
            && active_prompt.as_ref().is_some_and(|prompt| {
                prompt.status() == crate::session::PromptStatus::Cancelling
            })
        {
            let prompt = active_prompt.as_ref().expect("checked above");
            match prompt.durable_delivery_phase() {
                Some(crate::session::DurablePromptDeliveryPhase::Accepted)
                    if delivery_phase
                        != Some(crate::session::DurablePromptDeliveryPhase::Dispatching) =>
                {
                    // Accepted is persisted before transport starts. On restart no worker run can
                    // belong to this prompt, so finish its durable cancellation without replay.
                    self.owned
                        .finalize_remote_prompt_cancellation_after_worker_settled(
                            session_id,
                            agent_id,
                            prompt.source_attachment_id(),
                        )?;
                    return Ok(true);
                }
                Some(crate::session::DurablePromptDeliveryPhase::Delivered) => {
                    let Some(dispatch) = self.remote_prompt_recovery_dispatch(&agent)? else {
                        return Ok(true);
                    };
                    if let Err(error) = self
                        .resume_delivered_remote_prompt_cancellation_after_restart(
                            &dispatch,
                            prompt.durable_delivery_provider_run_id(),
                        )
                        .await
                    {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "restart kept delivered remote cancellation held because its worker receipt was not reconciled",
                            serde_json::json!({
                                "session_id": dispatch.session_id,
                                "agent_id": dispatch.agent_id,
                                "worker_kernel_id": dispatch.worker_kernel_id,
                                "leased_agent_id": dispatch.leased_agent_id,
                                "prompt_id": dispatch.prompt_id,
                                "error": error.to_string(),
                            }),
                        );
                    }
                    return Ok(true);
                }
                Some(crate::session::DurablePromptDeliveryPhase::Dispatching) => {}
                None => {
                    // Older or incomplete state has no proof tying this cancellation to a worker
                    // run. Keep the same prompt held; never redispatch or cancel a lease by guess.
                    return Ok(true);
                }
            }
        }
        if agent.remote_execution().is_some()
            && active_prompt.as_ref().is_some_and(|prompt| {
                prompt.durable_delivery_reconciliation_pending()
                    || prompt.durable_delivery_phase()
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
                    || delivery_phase
                        == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            })
        {
            let Some(dispatch) = self.remote_prompt_recovery_dispatch(&agent)? else {
                return Ok(true);
            };
            match self.reconcile_remote_prompt_worker_receipt(&dispatch).await {
                Ok(provider_run_id) => {
                    self.finish_remote_prompt_dispatch(dispatch, Ok(provider_run_id))
                        .await?;
                }
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "restart kept remote prompt held because its worker receipt was not reconciled",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
            return Ok(true);
        }
        let active_worker_run = agent
            .remote_execution()
            .and_then(|binding| binding.active_worker_provider_run_id.as_deref())
            .is_some();
        if delivery_phase != Some(crate::session::DurablePromptDeliveryPhase::Accepted)
            && !active_worker_run
        {
            if let Some(provider_run_id) = delivery_provider_run_id {
                self.owned
                    .agent_store
                    .set_remote_execution_active_worker_provider_run_id(
                        agent_id,
                        Some(provider_run_id.to_string()),
                    )?;
            }
        }
        let active_worker_run = self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            .and_then(|binding| binding.active_worker_provider_run_id.as_deref())
            .is_some();
        if active_worker_run
            && delivery_phase != Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        {
            self.spawn_remote_prompt_projection_drain(session_id.to_string(), agent_id.to_string());
            return Ok(true);
        }
        let Some(mut dispatch) =
            self.remote_prompt_recovery_dispatch_for_phase(&agent, delivery_phase)?
        else {
            return Ok(false);
        };
        self.populate_remote_prompt_recovery_workflow_context(&mut dispatch)
            .await?;
        self.spawn_remote_prompt_dispatch(dispatch);
        Ok(true)
    }

    pub(super) async fn reconcile_remote_prompt_worker_receipt(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<String, DaemonError> {
        match self.remote_prompt_receipt_prompt_is_current(dispatch) {
            Ok(true) => {}
            Ok(false) => {
                return Err(DaemonError::NoActivePrompt {
                    session_id: dispatch.session_id.clone(),
                });
            }
            Err(error) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(
                    dispatch,
                    format!("could not verify the active home prompt before querying: {error}"),
                ));
            }
        }
        let _ = persist_remote_prompt_reconciliation_pending(self, dispatch, true);
        let binding_before = match self.remote_prompt_receipt_binding(dispatch) {
            Ok(binding) => binding,
            Err(detail) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail))
            }
        };
        let receipt = match query_remote_prompt_worker_receipt(self, dispatch).await {
            Ok(receipt) => receipt,
            Err(error) => {
                let detail = format!("read-only worker receipt query failed: {error}");
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail));
            }
        };
        match self.remote_prompt_receipt_prompt_is_current(dispatch) {
            Ok(true) => {}
            Ok(false) => {
                return Err(DaemonError::NoActivePrompt {
                    session_id: dispatch.session_id.clone(),
                });
            }
            Err(error) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(
                    dispatch,
                    format!("could not verify the active home prompt after querying: {error}"),
                ));
            }
        }
        let action = match remote_prompt_receipt_action(&dispatch.prompt_id, receipt) {
            Ok(action) => action,
            Err(detail) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail))
            }
        };
        let binding_after = match self.remote_prompt_receipt_binding(dispatch) {
            Ok(binding) => binding,
            Err(detail) => {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail))
            }
        };
        if !same_remote_prompt_worker_binding(&binding_before, &binding_after)
            || binding_before.active_worker_provider_run_id
                != binding_after.active_worker_provider_run_id
        {
            return Err(self.hold_remote_prompt_receipt_reconciliation(
                dispatch,
                "remote worker binding changed while its receipt was being queried",
            ));
        }
        let completed = remote_prompt_receipt_action_requires_projection(&action);
        let worker_provider_run_id = match action {
            RemotePromptReceiptAction::AssociateActiveRun(provider_run_id) => provider_run_id,
            RemotePromptReceiptAction::DrainCompletedProjection(provider_run_id) => provider_run_id,
        };
        if let Err(error) = self
            .owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(
                &dispatch.agent_id,
                Some(worker_provider_run_id.clone()),
            )
        {
            return Err(self.hold_remote_prompt_receipt_reconciliation(
                dispatch,
                format!("could not associate the verified worker provider run: {error}"),
            ));
        }
        if let Err(error) = self.owned.session_snapshot(&dispatch.session_id) {
            return Err(self.hold_remote_prompt_receipt_reconciliation(
                dispatch,
                format!("could not persist the verified worker provider run: {error}"),
            ));
        }

        if completed {
            if let Err(detail) = self
                .drain_completed_remote_prompt_receipt_projection(dispatch, &worker_provider_run_id)
                .await
            {
                return Err(self.hold_remote_prompt_receipt_reconciliation(dispatch, detail));
            }
        }
        Ok(worker_provider_run_id)
    }

    async fn resume_delivered_remote_prompt_cancellation_after_restart(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        expected_provider_run_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        let expected_provider_run_id = expected_provider_run_id.ok_or_else(|| {
            DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "delivered cancellation has no durable worker provider-run identity"
                    .to_string(),
            }
        })?;
        if !self.remote_prompt_receipt_prompt_is_current(dispatch)? {
            return Err(DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            });
        }
        let binding_before = self
            .remote_prompt_receipt_binding(dispatch)
            .map_err(|detail| DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: detail,
            })?;
        if binding_before.active_worker_provider_run_id.as_deref()
            != Some(expected_provider_run_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "the current worker binding no longer owns the delivered prompt run"
                    .to_string(),
            });
        }
        let receipt = query_remote_prompt_worker_receipt(self, dispatch).await?;
        if !self.remote_prompt_receipt_prompt_is_current(dispatch)? {
            return Err(DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            });
        }
        let action = remote_prompt_receipt_action(&dispatch.prompt_id, receipt).map_err(
            |detail| DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: detail.to_string(),
            },
        )?;
        let binding_after = self
            .remote_prompt_receipt_binding(dispatch)
            .map_err(|detail| DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: detail,
            })?;
        if !same_remote_prompt_worker_binding(&binding_before, &binding_after)
            || binding_after.active_worker_provider_run_id.as_deref()
                != Some(expected_provider_run_id)
        {
            return Err(DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "the verified worker binding changed while its receipt was queried"
                    .to_string(),
            });
        }
        match action {
            RemotePromptReceiptAction::AssociateActiveRun(provider_run_id)
                if provider_run_id.as_str() == expected_provider_run_id =>
            {
                self.cancel_remote_agent_prompt_if_remote(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.source_attachment_id,
                )
                .await?;
                Ok(())
            }
            RemotePromptReceiptAction::DrainCompletedProjection(provider_run_id)
                if provider_run_id.as_str() == expected_provider_run_id =>
            {
                self.drain_completed_remote_prompt_receipt_projection(
                    dispatch,
                    expected_provider_run_id,
                )
                .await
                .map_err(|detail| DaemonError::LocalTransport {
                    operation: "resume remote prompt cancellation",
                    message: detail,
                })?;
                let binding = self.remote_prompt_receipt_binding(dispatch).map_err(|detail| {
                    DaemonError::LocalTransport {
                        operation: "resume remote prompt cancellation",
                        message: detail,
                    }
                })?;
                let session = self.owned.session_store.get_session(&dispatch.session_id)?;
                if let Some(prompt) = self
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &dispatch.agent_id)
                    .filter(|prompt| {
                        prompt.id() == dispatch.prompt_id
                            && prompt.status() == crate::session::PromptStatus::Cancelling
                            && prompt.durable_delivery_phase()
                                == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                            && prompt.durable_delivery_provider_run_id()
                                == Some(expected_provider_run_id)
                            && binding.active_worker_provider_run_id.as_deref()
                                == Some(expected_provider_run_id)
                    })
                {
                    self.owned
                        .finalize_remote_prompt_cancellation_after_worker_settled(
                            &dispatch.session_id,
                            &dispatch.agent_id,
                            prompt.source_attachment_id(),
                        )?;
                }
                Ok(())
            }
            _ => Err(DaemonError::LocalTransport {
                operation: "resume remote prompt cancellation",
                message: "worker receipt did not prove the exact delivered provider run"
                    .to_string(),
            }),
        }
    }

    fn remote_prompt_receipt_prompt_is_current(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(&dispatch.session_id)?;
        Ok(self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .is_some_and(|prompt| prompt.id() == dispatch.prompt_id))
    }

    fn remote_prompt_receipt_binding(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
    ) -> Result<crate::agent::RemoteAgentBinding, String> {
        let agent = self
            .owned
            .agent_store
            .get_agent(&dispatch.agent_id)
            .map_err(|error| format!("could not read current remote worker binding: {error}"))?;
        agent
            .remote_execution()
            .filter(|binding| {
                binding.worker_kernel_id == dispatch.worker_kernel_id
                    && binding.leased_agent_id == dispatch.leased_agent_id
            })
            .cloned()
            .ok_or_else(|| {
                "current remote worker binding conflicts with the uncertain dispatch".to_string()
            })
    }

    fn hold_remote_prompt_receipt_reconciliation(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        detail: impl std::fmt::Display,
    ) -> DaemonError {
        let detail = detail.to_string();
        let detail = match persist_remote_prompt_reconciliation_pending(self, dispatch, true) {
            Ok(()) => detail,
            Err(error) => format!(
                "{detail}; durable reconciliation marker could not be refreshed ({error}); the Dispatching phase continues to block replay"
            ),
        };
        self.report_remote_prompt_reconciliation_pending(dispatch, &detail);
        remote_prompt_reconciliation_pending_error(dispatch, &detail)
    }

    async fn drain_completed_remote_prompt_receipt_projection(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        worker_provider_run_id: &str,
    ) -> Result<(), String> {
        let binding = self.remote_prompt_receipt_binding(dispatch)?;
        if binding.active_worker_provider_run_id.as_deref() != Some(worker_provider_run_id) {
            return Err(
                "worker provider-run association changed before projection drain".to_string(),
            );
        }
        let relay_config = self
            .with_app_side_effect(move |app| app.relay_config_for_remote_execution(&binding))
            .await;
        let target = ClientTarget {
            daemon_id: Some(dispatch.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let request = RelayPeerRequest::DrainLeasedRuntimeProjection {
            leased_agent_id: dispatch.leased_agent_id.clone(),
            provider_run_id: worker_provider_run_id.to_string(),
            pump_output: true,
        };
        // Relay I/O runs after relay configuration was copied and the app lock was released.
        let relay_state = self.connected_relay_state_for_config(&relay_config).await;
        let response = match relay_state {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                    &relay_config,
                    &relay_state,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    &relay_config,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
        }
        .map_err(|error| format!("completed receipt projection drain failed: {error}"))?;
        let event = match response {
            RelayPeerResponse::LeasedRuntimeProjectionDrained { event: Some(event) } => event,
            RelayPeerResponse::LeasedRuntimeProjectionDrained { event: None } => {
                return Err(
                    "worker returned no replayable projection for its completed receipt"
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "unexpected completed receipt drain response: {other:?}"
                ));
            }
        };
        if !completed_receipt_projection_matches(dispatch, worker_provider_run_id, &event) {
            return Err(
                "worker projection did not prove completion of the exact home prompt and provider run"
                    .to_string(),
            );
        }
        if !self
            .remote_prompt_receipt_prompt_is_current(dispatch)
            .map_err(|error| error.to_string())?
        {
            return Err(
                "home prompt changed before the completed worker projection was applied"
                    .to_string(),
            );
        }
        self.project_remote_runtime_projection_event(event)
            .await
            .map_err(|error| format!("completed worker projection could not be applied: {error}"))
    }

    pub(super) fn spawn_remote_prompt_projection_drain_if_needed(
        &self,
        submission: &crate::app::KernelPromptSubmission,
    ) {
        let prompt = match &submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
        };
        let session_id = submission.session.id().to_string();
        let agent_id = prompt.target_agent_id().to_string();
        self.spawn_remote_prompt_projection_drain(session_id, agent_id);
    }

    fn spawn_remote_prompt_projection_drain(&self, session_id: String, agent_id: String) {
        let Some(mut claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_projection_drains),
            &session_id,
            &agent_id,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            let mut transport_retry_attempt = 0_u32;
            loop {
                match state
                    .drain_remote_prompt_projection_once(&session_id, &agent_id)
                    .await
                {
                    Ok(true) => transport_retry_attempt = 0,
                    Ok(false) => {
                        if claim.release_or_restart() {
                            continue;
                        }
                        return;
                    }
                    Err(error) if remote_prompt_error_should_retry_transport(&error) => {
                        transport_retry_attempt = transport_retry_attempt.saturating_add(1);
                        if transport_retry_attempt == 1 || transport_retry_attempt % 12 == 0 {
                            crate::logging::warn_with_fields(
                                "daemon.remote_prompt_dispatch",
                                "remote projection transport unavailable; retrying active prompt",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "agent_id": agent_id,
                                    "attempt": transport_retry_attempt,
                                    "error": error.to_string(),
                                }),
                            );
                        }
                        if state
                            .remote_prompt_projection_drain_target(&session_id, &agent_id)
                            .is_none()
                        {
                            if claim.release_or_restart() {
                                continue;
                            }
                            return;
                        }
                        tokio::time::sleep(remote_prompt_transport_retry_delay(
                            transport_retry_attempt,
                        ))
                        .await;
                        continue;
                    }
                    Err(error) => {
                        crate::logging::warn_with_fields(
                            "daemon.remote_prompt_dispatch",
                            "remote projection drain failed",
                            serde_json::json!({
                                "session_id": session_id,
                                "agent_id": agent_id,
                                "error": error.to_string(),
                            }),
                        );
                        if claim.release_or_restart() {
                            continue;
                        }
                        return;
                    }
                }
                if state
                    .remote_prompt_projection_drain_target(&session_id, &agent_id)
                    .is_none()
                {
                    if claim.release_or_restart() {
                        continue;
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }

    fn remote_prompt_projection_drain_target(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<(crate::agent::RemoteAgentBinding, String)> {
        let owned = &self.owned;
        let session = owned.session_store.get_session(session_id).ok()?;
        let active_prompt = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)?;
        if matches!(
            active_prompt.durable_delivery_phase(),
            Some(
                crate::session::DurablePromptDeliveryPhase::Accepted
                    | crate::session::DurablePromptDeliveryPhase::Dispatching
            )
        ) {
            return None;
        }
        let remote_execution = owned
            .agent_store
            .get_agent(agent_id)
            .ok()?
            .remote_execution()
            .cloned()?;
        let provider_run_id = remote_execution
            .active_worker_provider_run_id
            .clone()
            .or_else(|| {
                (active_prompt.durable_delivery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Delivered))
                .then(|| {
                    active_prompt
                        .durable_delivery_provider_run_id()
                        .filter(|provider_run_id| !provider_run_id.trim().is_empty())
                        .map(str::to_string)
                })
                .flatten()
            })?;
        Some((remote_execution, provider_run_id))
    }

    fn recover_remote_prompt_run_binding_from_projection(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_binding: &crate::agent::RemoteAgentBinding,
        provider_run_id: &str,
        event: &crate::transport::relay_peer::RelayPeerEvent,
    ) -> Result<RemotePromptRunBindingRecovery, DaemonError> {
        let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id,
            home_agent_id,
            provider_run_id: projected_provider_run_id,
            completions,
            ..
        } = event;
        if home_session_id != session_id
            || home_agent_id != agent_id
            || projected_provider_run_id != provider_run_id
        {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        };
        if active_prompt.durable_delivery_phase()
            != Some(crate::session::DurablePromptDeliveryPhase::Delivered)
            || active_prompt.durable_delivery_provider_run_id() != Some(provider_run_id)
            || (!completions.is_empty()
                && !completions.iter().any(|completion| {
                    completion.home_prompt_id.as_deref() == Some(active_prompt.id())
                }))
        {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let Some(current_binding) = agent.remote_execution() else {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        };
        if !same_remote_prompt_worker_binding(current_binding, expected_binding) {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        match current_binding.active_worker_provider_run_id.as_deref() {
            Some(current_provider_run_id) if current_provider_run_id == provider_run_id => {
                return Ok(RemotePromptRunBindingRecovery::Recovered);
            }
            Some(_) => return Ok(RemotePromptRunBindingRecovery::Rejected),
            None => {}
        }
        self.owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(
                agent_id,
                Some(provider_run_id.to_string()),
            )?;
        let _ = self.owned.session_snapshot(session_id)?;
        Ok(RemotePromptRunBindingRecovery::Recovered)
    }

    pub(super) async fn drain_active_remote_prompt_projections_for_session(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> Result<(), DaemonError> {
        for agent_id in self
            .owned
            .prompt_state_owner
            .active_prompt_agent_ids(session)
        {
            let _ = self
                .drain_remote_prompt_projection_once(session.id(), &agent_id)
                .await?;
        }
        Ok(())
    }

    async fn drain_remote_prompt_projection_once(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let Some((remote_execution, provider_run_id)) =
            self.remote_prompt_projection_drain_target(session_id, agent_id)
        else {
            return Ok(false);
        };
        let recover_missing_run_binding = remote_execution.active_worker_provider_run_id.is_none();
        let relay_config = self
            .with_app_side_effect(|app| app.relay_config_for_remote_execution(&remote_execution))
            .await;
        let target = ClientTarget {
            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
            daemon_alias: None,
        };
        let request = RelayPeerRequest::DrainLeasedRuntimeProjection {
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            provider_run_id: provider_run_id.clone(),
            pump_output: true,
        };
        let response = match self.connected_relay_state_for_config(&relay_config).await {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay_with_timeout(
                    &relay_config,
                    &relay_state,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    &relay_config,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
        };
        let response = match response {
            Ok(response) => response,
            Err(error)
                if remote_prompt_projection_error_should_refresh_binding(
                    recover_missing_run_binding,
                    &error,
                ) =>
            {
                self.spawn_stale_remote_prompt_recovery(
                    session_id.to_string(),
                    agent_id.to_string(),
                    remote_execution,
                    provider_run_id,
                    error.to_string(),
                );
                return Ok(true);
            }
            Err(error) => return Err(error),
        };
        match response {
            RelayPeerResponse::LeasedRuntimeProjectionDrained { event } => {
                if let Some(event) = event {
                    if recover_missing_run_binding {
                        match self.recover_remote_prompt_run_binding_from_projection(
                            session_id,
                            agent_id,
                            &remote_execution,
                            &provider_run_id,
                            &event,
                        )? {
                            RemotePromptRunBindingRecovery::Recovered => {}
                            RemotePromptRunBindingRecovery::Rejected => return Ok(false),
                        }
                    }
                    self.project_remote_runtime_projection_event(event).await?;
                }
                Ok(true)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "drain remote prompt projection",
                message: format!("unexpected remote projection drain response: {other:?}"),
            }),
        }
    }

    fn spawn_stale_remote_prompt_recovery(
        &self,
        session_id: String,
        agent_id: String,
        stale_binding: crate::agent::RemoteAgentBinding,
        stale_provider_run_id: String,
        trigger_error: String,
    ) {
        let Some(claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            &session_id,
            &agent_id,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            if let Err(error) = state
                .recover_stale_remote_prompt(
                    &session_id,
                    &agent_id,
                    &stale_binding,
                    &stale_provider_run_id,
                    &trigger_error,
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "stale remote prompt recovery stopped",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "worker_kernel_id": stale_binding.worker_kernel_id,
                        "leased_agent_id": stale_binding.leased_agent_id,
                        "error": error.to_string(),
                    }),
                );
            }
            state
                .run_remote_prompt_dispatch_with_claim(claim, None)
                .await;
        });
    }

    async fn recover_stale_remote_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        stale_binding: &crate::agent::RemoteAgentBinding,
        stale_provider_run_id: &str,
        trigger_error: &str,
    ) -> Result<(), DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|prompt| prompt.status() == crate::session::PromptStatus::Cancelling)
        {
            // Cancellation owns the exact old worker run. A stale projection must not refresh
            // its binding and replay the prompt onto a replacement worker.
            return Ok(());
        }
        let refresh_agent_id = agent_id.to_string();
        let stale_leased_agent_id = stale_binding.leased_agent_id.clone();
        let refresh_stale_provider_run_id = stale_provider_run_id.to_string();
        let rebound = self
            .with_app_side_effect_blocking(move |app| {
                let agent = app.agents().get_agent(&refresh_agent_id)?;
                let Some(current_binding) = agent.remote_execution() else {
                    return Ok(None);
                };
                if current_binding.leased_agent_id != stale_leased_agent_id
                    || current_binding.active_worker_provider_run_id.as_deref()
                        != Some(refresh_stale_provider_run_id.as_str())
                {
                    return Ok(None);
                }
                app.refresh_remote_agent_binding(&refresh_agent_id)
                    .map(Some)
            })
            .await?;
        let Some(rebound) = rebound else {
            return Ok(());
        };
        let Some(mut dispatch) = self.remote_prompt_recovery_dispatch_for_phase(&rebound, None)?
        else {
            return Ok(());
        };
        self.populate_remote_prompt_recovery_workflow_context(&mut dispatch)
            .await?;
        crate::logging::warn_with_fields(
            "daemon.remote_prompt_dispatch",
            "replaying active prompt after stale worker binding",
            serde_json::json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "previous_worker_kernel_id": stale_binding.worker_kernel_id,
                "previous_leased_agent_id": stale_binding.leased_agent_id,
                "worker_kernel_id": dispatch.worker_kernel_id,
                "leased_agent_id": dispatch.leased_agent_id,
                "prompt_id": dispatch.prompt_id,
                "trigger_error": trigger_error,
            }),
        );

        let prompt_id = dispatch.prompt_id.clone();
        let mut attempt = 0_u32;
        loop {
            if !self.remote_prompt_recovery_is_current(
                session_id,
                agent_id,
                &prompt_id,
                &dispatch.leased_agent_id,
            )? {
                return Ok(());
            }
            let agent = self.owned.agent_store.get_agent(agent_id)?;
            let (prompt, _) = match self
                .prepare_remote_prompt_skill_context(&agent, &dispatch.prompt)
                .await
            {
                Ok(context) => context,
                Err(error) => {
                    attempt = attempt.saturating_add(1);
                    self.log_remote_prompt_recovery_retry(
                        session_id, agent_id, &dispatch, attempt, &error,
                    );
                    tokio::time::sleep(remote_prompt_recovery_delay(attempt)).await;
                    continue;
                }
            };
            let attachments = dispatch.attachments.clone();
            let attachments = match tokio::task::spawn_blocking(move || {
                crate::app::serialize_remote_prompt_attachments(&attachments)
            })
            .await
            {
                Ok(Ok(attachments)) => attachments,
                Ok(Err(error)) => return Err(error),
                Err(error) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "serialize recovered remote prompt attachments",
                        message: error.to_string(),
                    });
                }
            };
            if !self.remote_prompt_recovery_is_current(
                session_id,
                agent_id,
                &prompt_id,
                &dispatch.leased_agent_id,
            )? {
                return Ok(());
            }
            let result = submit_remote_prompt_to_worker_with_binding_refresh(
                self,
                &mut dispatch,
                prompt,
                attachments,
            )
            .await;
            match result {
                Ok(provider_run_id) => {
                    self.finish_stale_remote_prompt_recovery(
                        session_id,
                        agent_id,
                        stale_binding,
                        stale_provider_run_id,
                        &dispatch,
                        &provider_run_id,
                    )?;
                    crate::logging::info_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "active remote prompt recovered on refreshed worker binding",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "provider_run_id": provider_run_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": attempt + 1,
                        }),
                    );
                    let state = self.clone();
                    let session_id = session_id.to_string();
                    let agent_id = agent_id.to_string();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        state.spawn_remote_prompt_projection_drain(session_id, agent_id);
                    });
                    return Ok(());
                }
                Err(error) => {
                    if remote_prompt_error_is_reconciliation_pending(&error)
                        || remote_prompt_reconciliation_pending(self, &dispatch).unwrap_or(true)
                    {
                        return Err(error);
                    }
                    attempt = attempt.saturating_add(1);
                    self.log_remote_prompt_recovery_retry(
                        session_id, agent_id, &dispatch, attempt, &error,
                    );
                    tokio::time::sleep(remote_prompt_recovery_delay(attempt)).await;
                }
            }
        }
    }

    fn remote_prompt_recovery_dispatch(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Option<crate::app::KernelRemotePromptDispatch>, DaemonError> {
        let Some(remote_execution) = agent.remote_execution() else {
            return Ok(None);
        };
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
        else {
            return Ok(None);
        };
        Ok(Some(crate::app::KernelRemotePromptDispatch {
            session_id: session.id().to_string(),
            agent_id: agent.id().to_string(),
            prompt_id: active_prompt.id().to_string(),
            worker_kernel_id: remote_execution.worker_kernel_id.clone(),
            leased_agent_id: remote_execution.leased_agent_id.clone(),
            relay_url: remote_execution.relay_url.clone(),
            relay_token: remote_execution.relay_token.clone(),
            source_attachment_id: active_prompt.source_attachment_id().to_string(),
            prompt: active_prompt.prompt().to_string(),
            hidden_system_context: active_prompt.hidden_system_context().to_string(),
            attachments: active_prompt.attachments().to_vec(),
            workspace_live_sync_mode: Some(
                crate::provider::provider_workspace_live_sync_mode_for_session(
                    agent.provider(),
                    &self.owned.config_projection.snapshot(),
                    Some(&session),
                ),
            ),
            prompt_origin: active_prompt.prompt_origin(),
            external_provider: active_prompt.external_provider().map(str::to_string),
            external_provider_session_id: active_prompt
                .external_provider_session_id()
                .map(str::to_string),
            external_provider_turn_id: active_prompt
                .external_provider_turn_id()
                .map(str::to_string),
            workflow_context: None,
        }))
    }

    fn remote_prompt_recovery_dispatch_for_phase(
        &self,
        agent: &crate::agent::AgentInstance,
        observed_delivery_phase: Option<crate::session::DurablePromptDeliveryPhase>,
    ) -> Result<Option<crate::app::KernelRemotePromptDispatch>, DaemonError> {
        let Some(dispatch) = self.remote_prompt_recovery_dispatch(agent)? else {
            return Ok(None);
        };
        let session = self.owned.session_store.get_session(&dispatch.session_id)?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
        else {
            return Ok(None);
        };
        if active_prompt.id() != dispatch.prompt_id {
            return Ok(None);
        }
        let must_hold = active_prompt.durable_delivery_reconciliation_pending()
            || active_prompt.durable_delivery_phase()
                == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            || observed_delivery_phase
                == Some(crate::session::DurablePromptDeliveryPhase::Dispatching);
        if must_hold {
            let marker_was_durable = active_prompt.durable_delivery_reconciliation_pending()
                || persist_remote_prompt_reconciliation_pending(self, &dispatch, true).is_ok();
            let detail = if marker_was_durable {
                "restart found a remote prompt in Dispatching without a durable submission acknowledgement"
            } else {
                "restart found a remote prompt in Dispatching without a durable submission acknowledgement; the reconciliation marker could not be persisted, so automatic replay remains blocked by the durable Dispatching phase"
            };
            self.report_remote_prompt_reconciliation_pending(&dispatch, detail);
            return Ok(None);
        }
        Ok(Some(dispatch))
    }

    async fn populate_remote_prompt_recovery_workflow_context(
        &self,
        dispatch: &mut crate::app::KernelRemotePromptDispatch,
    ) -> Result<(), DaemonError> {
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&dispatch.source_attachment_id)
        {
            return Ok(());
        }
        let session = self.owned.session_store.get_session(&dispatch.session_id)?;
        let prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: dispatch.session_id.clone(),
            })?;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        dispatch.workflow_context = Some(
            self.with_app_side_effect(move |app| {
                crate::app::RemoteWorkflowTurnContextResolver::new(app)
                    .remote_workflow_turn_context_for_prompt(&session_id, &agent_id, &prompt)
            })
            .await?,
        );
        Ok(())
    }

    fn remote_prompt_recovery_is_current(
        &self,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        leased_agent_id: &str,
    ) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        let active_prompt = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id);
        if let Some(prompt) = active_prompt.as_ref().filter(|prompt| {
            prompt.id() == prompt_id
                && prompt.status() == crate::session::PromptStatus::Cancelling
                && prompt.durable_delivery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        }) {
            self.owned
                .finalize_remote_prompt_cancellation_after_worker_settled(
                    session_id,
                    agent_id,
                    prompt.source_attachment_id(),
                )?;
            return Ok(false);
        }
        let prompt_is_current = active_prompt.as_ref().is_some_and(|prompt| {
            prompt.id() == prompt_id && prompt.status() != crate::session::PromptStatus::Cancelling
        });
        let binding_is_current = self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            .is_some_and(|binding| binding.leased_agent_id == leased_agent_id);
        Ok(prompt_is_current && binding_is_current)
    }

    fn finish_stale_remote_prompt_recovery(
        &self,
        session_id: &str,
        agent_id: &str,
        stale_binding: &crate::agent::RemoteAgentBinding,
        stale_provider_run_id: &str,
        recovered_dispatch: &crate::app::KernelRemotePromptDispatch,
        recovered_provider_run_id: &str,
    ) -> Result<(), DaemonError> {
        if !self.remote_prompt_recovery_is_current(
            session_id,
            agent_id,
            &recovered_dispatch.prompt_id,
            &recovered_dispatch.leased_agent_id,
        )? {
            return Ok(());
        }
        let stale_projected_run_id = crate::provider::projected_leased_provider_run_id(
            &stale_binding.leased_agent_id,
            stale_provider_run_id,
        );
        if let Some(mut stale_run) = self
            .owned
            .provider_run_projection
            .get(&stale_projected_run_id)
        {
            stale_run.mark_ended();
            self.owned
                .clear_active_provider_run_session_pointer(session_id, stale_run.id())?;
            self.owned.clear_prompt_activity(stale_run.id());
            self.owned.provider_run_projection.update(stale_run);
        }
        self.owned
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(
                agent_id,
                Some(recovered_provider_run_id.to_string()),
            )?;
        let _ = self.owned.session_snapshot(session_id)?;
        Ok(())
    }

    fn log_remote_prompt_recovery_retry(
        &self,
        session_id: &str,
        agent_id: &str,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        attempt: u32,
        error: &DaemonError,
    ) {
        if attempt == 1 || attempt % 12 == 0 {
            crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "active remote prompt recovery retrying",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "prompt_id": dispatch.prompt_id,
                    "attempt": attempt,
                    "error": error.to_string(),
                }),
            );
        }
    }

    pub(super) async fn connected_relay_state_for_config(
        &self,
        relay_config: &crate::config::DaemonConfig,
    ) -> Option<Arc<tokio::sync::RwLock<crate::transport::relay_client::RelayClientState>>> {
        let relay_url = relay_config.relay_url.as_deref()?;
        if self
            .owned
            .relay_state
            .read()
            .await
            .connected_relay_url()
            .as_deref()
            == Some(relay_url)
        {
            return Some(Arc::clone(&self.owned.relay_state));
        }
        let slice_states = {
            let connectors = self.owned.slice_private_relay_connectors.lock().await;
            connectors
                .values()
                .filter(|connector| connector.relay_url == relay_url)
                .map(|connector| Arc::clone(&connector.state))
                .collect::<Vec<_>>()
        };
        for state in slice_states {
            if state.read().await.connected_relay_url().as_deref() == Some(relay_url) {
                return Some(state);
            }
        }
        None
    }

    async fn project_remote_runtime_projection_event(
        &self,
        event: crate::transport::relay_peer::RelayPeerEvent,
    ) -> Result<(), DaemonError> {
        match event {
            crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                home_session_id,
                home_agent_id,
                provider_run_id,
                provider_run,
                prompts,
                output_chunks,
                notices,
                completions,
            } => {
                self.project_relay_remote_runtime_projection(
                    &home_session_id,
                    &home_agent_id,
                    &provider_run_id,
                    provider_run,
                    prompts,
                    output_chunks,
                    notices,
                    completions,
                )
                .await
            }
        }
    }

    pub(super) fn report_remote_prompt_reconciliation_pending(
        &self,
        dispatch: &crate::app::KernelRemotePromptDispatch,
        detail: &str,
    ) {
        let message = format!(
            "Remote delivery of prompt `{}` to worker kernel `{}` is indeterminate: {detail}. The same prompt remains active in durable Dispatching state; it was not cancelled, promoted, or replayed. Automatic replay is blocked until the exact worker receipt is reconciled.",
            dispatch.prompt_id, dispatch.worker_kernel_id
        );
        crate::logging::warn_with_fields(
            "daemon.remote_prompt_dispatch",
            &message,
            serde_json::json!({
                "session_id": dispatch.session_id,
                "agent_id": dispatch.agent_id,
                "prompt_id": dispatch.prompt_id,
                "worker_kernel_id": dispatch.worker_kernel_id,
                "leased_agent_id": dispatch.leased_agent_id,
                "delivery_uncertain": true,
                "prompt_replayed": false,
                "worker_prompt_receipt_required": true,
            }),
        );
        let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
        let merge_key = Some(format!("remote-dispatch-uncertain:{}", dispatch.prompt_id));
        self.owned.fan_out_remote_dispatch_error(
            dispatch,
            &provider_run_id,
            merge_key.clone(),
            &message,
        );
        let workflow_run_id = self
            .owned
            .session_store
            .get_session(&dispatch.session_id)
            .ok()
            .and_then(|session| {
                self.owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &dispatch.agent_id)
                    .filter(|prompt| prompt.id() == dispatch.prompt_id)
                    .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
            });
        let workflow_id = workflow_run_id.as_deref().and_then(|run_id| {
            self.owned
                .session_store
                .get_session(&dispatch.session_id)
                .ok()
                .and_then(|session| {
                    session
                        .workflow_run(run_id)
                        .map(|run| run.workflow_id().to_string())
                })
        });
        self.owned.append_operational_history_entry_with_context(
            &SessionHistoryEntry::provider_output(
                &dispatch.session_id,
                &provider_run_id,
                Some(&dispatch.agent_id),
                crate::terminal::TerminalOutputKind::ProviderError,
                merge_key,
                message,
            )
            .with_prompt_origin(dispatch.prompt_origin)
            .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
            crate::history::HistoryEventTurnContext {
                session_id: Some(dispatch.session_id.clone()),
                agent_id: Some(dispatch.agent_id.clone()),
                provider_run_id: Some(provider_run_id),
                prompt_id: Some(dispatch.prompt_id.clone()),
                turn_id: Some(dispatch.prompt_id.clone()),
                workflow_id,
                workflow_run_id,
                ..Default::default()
            },
        );
    }

    pub(super) async fn finish_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        self.finish_remote_prompt_dispatch_with_retry_hook(dispatch, result, || {})
            .await
    }

    async fn finish_remote_prompt_dispatch_with_retry_hook(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
        mut after_failed_ack: impl FnMut() + Send,
    ) -> Result<(), DaemonError> {
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let delivery_succeeded = result.is_ok();
        use super::remote_prompt_owned_state::RemotePromptDispatchSettlement;
        let mut append_retry = 0_u8;
        let settlement = loop {
            match self.owned.settle_remote_dispatch_if_current(
                &dispatch,
                result.as_ref().ok().map(String::as_str),
            ) {
                Err(error)
                    if result.is_ok()
                        && (matches!(&error, DaemonError::SessionHistoryFailed { .. })
                            || crate::durable_state::is_retryable_durable_write_error(&error))
                        && append_retry < 2 =>
                {
                    append_retry += 1;
                    after_failed_ack();
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "worker accepted prompt but durable acknowledgement failed; retrying acknowledgement only",
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "attempt": append_retry,
                            "error": error.to_string(),
                        }),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(
                        50 * u64::from(append_retry),
                    ))
                    .await;
                }
                outcome => break outcome?,
            }
        };
        let settled_prompt = match settlement {
            RemotePromptDispatchSettlement::Settled(prompt) => prompt,
            RemotePromptDispatchSettlement::Superseded => return Ok(()),
            RemotePromptDispatchSettlement::BindingChanged(prompt) => {
                let workflow_id = prompt.workflow_run_id().and_then(|run_id| {
                    self.owned
                        .session_store
                        .get_session(&dispatch.session_id)
                        .ok()
                        .and_then(|session| {
                            session
                                .workflow_run(run_id)
                                .map(|run| run.workflow_id().to_string())
                        })
                });
                let message = format!(
                    "Remote delivery of prompt `{}` is uncertain because its worker binding changed before acknowledgement. The prompt remains pending; it was not replayed or cancelled. Check the previous worker's turn before deciding whether to retry.",
                    dispatch.prompt_id,
                );
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    &message,
                    serde_json::json!({
                        "session_id": dispatch.session_id,
                        "agent_id": dispatch.agent_id,
                        "prompt_id": dispatch.prompt_id,
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "delivery_uncertain": true,
                        "prompt_replayed": false,
                    }),
                );
                let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
                let merge_key = Some(format!("remote-dispatch-uncertain:{}", dispatch.prompt_id));
                self.owned.fan_out_remote_dispatch_error(
                    &dispatch,
                    &provider_run_id,
                    merge_key.clone(),
                    &message,
                );
                self.owned.append_operational_history_entry_with_context(
                    &SessionHistoryEntry::provider_output(
                        &dispatch.session_id,
                        &provider_run_id,
                        Some(&dispatch.agent_id),
                        crate::terminal::TerminalOutputKind::ProviderError,
                        merge_key,
                        message,
                    )
                    .with_prompt_origin(dispatch.prompt_origin)
                    .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
                    crate::history::HistoryEventTurnContext {
                        session_id: Some(dispatch.session_id.clone()),
                        agent_id: Some(dispatch.agent_id.clone()),
                        provider_run_id: Some(provider_run_id),
                        prompt_id: Some(dispatch.prompt_id.clone()),
                        turn_id: Some(dispatch.prompt_id.clone()),
                        workflow_id,
                        workflow_run_id: prompt.workflow_run_id().map(str::to_string),
                        workflow_node_id: prompt.workflow_node_run_id().map(str::to_string),
                        ..Default::default()
                    },
                );
                return Ok(());
            }
        };
        self.owned.provider_process_projection.invalidate();
        let echo_to_all_attachments = settled_prompt.durable_initially_queued() == Some(true);
        let should_start_projection_drain = {
            let owned = &self.owned;
            match result {
                Ok(remote_provider_run_id) => {
                    let _ = owned.session_snapshot(&dispatch.session_id)?;
                    if echo_to_all_attachments {
                        let projected_provider_run_id =
                            crate::provider::projected_leased_provider_run_id(
                                &dispatch.leased_agent_id,
                                &remote_provider_run_id,
                            );
                        owned.echo_promoted_queued_prompt_to_attachments(
                            &dispatch.session_id,
                            &projected_provider_run_id,
                            &dispatch.prompt_id,
                            &dispatch.source_attachment_id,
                            &dispatch.prompt,
                            &dispatch.attachments,
                        );
                    } else {
                        owned.echo_prompt_to_other_attachments(
                            &dispatch.session_id,
                            &remote_provider_run_id,
                            &dispatch.prompt_id,
                            &dispatch.source_attachment_id,
                            &dispatch.prompt,
                            &dispatch.attachments,
                        );
                    }
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Delivered,
                        None,
                    );
                    Ok(true)
                }
                Err(error) => {
                    let message =
                        format!("Remote prompt dispatch failed after acknowledgement: {error}");
                    let provider_run_id = format!("remote-dispatch:{}", dispatch.prompt_id);
                    let merge_key = Some(format!("remote-dispatch-error:{}", dispatch.prompt_id));
                    owned.fan_out_remote_dispatch_error(
                        &dispatch,
                        &provider_run_id,
                        merge_key.clone(),
                        &message,
                    );
                    let workflow_id = settled_prompt.workflow_run_id().and_then(|run_id| {
                        owned
                            .session_store
                            .get_session(&dispatch.session_id)
                            .ok()
                            .and_then(|session| {
                                session
                                    .workflow_run(run_id)
                                    .map(|run| run.workflow_id().to_string())
                            })
                    });
                    owned.append_operational_history_entry_with_context(
                        &SessionHistoryEntry::provider_output(
                            &dispatch.session_id,
                            &provider_run_id,
                            Some(&dispatch.agent_id),
                            crate::terminal::TerminalOutputKind::ProviderError,
                            merge_key,
                            message,
                        )
                        .with_prompt_origin(dispatch.prompt_origin)
                        .with_source_attachment_id(Some(dispatch.source_attachment_id.clone())),
                        crate::history::HistoryEventTurnContext {
                            workflow_id,
                            session_id: Some(dispatch.session_id.clone()),
                            agent_id: Some(dispatch.agent_id.clone()),
                            provider_run_id: Some(provider_run_id.clone()),
                            prompt_id: Some(dispatch.prompt_id.clone()),
                            turn_id: Some(dispatch.prompt_id.clone()),
                            workflow_run_id: settled_prompt.workflow_run_id().map(str::to_string),
                            workflow_node_id: settled_prompt
                                .workflow_node_run_id()
                                .map(str::to_string),
                            ..Default::default()
                        },
                    );
                    owned.update_metaagent_event_prompt_delivery_for_prompt(
                        &dispatch.prompt_id,
                        crate::runtime::metaagent_event::MetaagentEventPromptDeliveryStatus::Failed,
                        Some(error.to_string()),
                    );
                    owned.record_cancelled_prompt_settlement(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        &settled_prompt,
                        settled_prompt.durable_delivery_provider_run_id(),
                    );
                    if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
                        settled_prompt.workflow_run_id(),
                        settled_prompt.workflow_node_run_id(),
                    ) {
                        owned.release_workflow_node_workspace_claim(
                            &dispatch.session_id,
                            workflow_run_id,
                            workflow_node_run_id,
                        );
                        owned.workflow_fail_node_after_dispatch_error(
                            &dispatch.session_id,
                            workflow_run_id,
                            workflow_node_run_id,
                            &error,
                        );
                    }
                    let _ = owned.session_snapshot(&dispatch.session_id);
                    Err(error)
                }
            }
        }?;
        if should_start_projection_drain
            && settled_prompt.status() != crate::session::PromptStatus::Cancelling
        {
            self.spawn_remote_prompt_projection_drain(session_id.clone(), agent_id.clone());
        }
        if delivery_succeeded && settled_prompt.status() == crate::session::PromptStatus::Cancelling
        {
            if let Err(error) = self
                .cancel_remote_agent_prompt_if_remote(
                    &session_id,
                    &agent_id,
                    settled_prompt.source_attachment_id(),
                )
                .await
            {
                crate::logging::warn_with_fields(
                    "daemon.remote_prompt_dispatch",
                    "durable remote cancellation intent remains pending after worker ACK",
                    serde_json::json!({
                        "session_id": session_id,
                        "agent_id": agent_id,
                        "prompt_id": settled_prompt.id(),
                        "worker_kernel_id": dispatch.worker_kernel_id,
                        "leased_agent_id": dispatch.leased_agent_id,
                        "provider_run_id": settled_prompt.durable_delivery_provider_run_id(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
        Ok(())
    }

    async fn remote_prompt_dispatch_after_claim_restart(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::app::KernelRemotePromptDispatch>, DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Ok(None);
        }
        let Some(mut dispatch) = self.remote_prompt_recovery_dispatch_for_phase(&agent, None)?
        else {
            return Ok(None);
        };
        if dispatch.session_id != session_id {
            return Ok(None);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(None);
        };
        if prompt.status() == crate::session::PromptStatus::Cancelling {
            if prompt.durable_delivery_phase()
                == Some(crate::session::DurablePromptDeliveryPhase::Accepted)
            {
                self.owned
                    .finalize_remote_prompt_cancellation_after_worker_settled(
                        session_id,
                        agent_id,
                        prompt.source_attachment_id(),
                    )?;
            }
            return Ok(None);
        }
        if prompt.id() != dispatch.prompt_id
            || prompt.durable_delivery_reconciliation_pending()
            || matches!(
                prompt.durable_delivery_phase(),
                Some(
                    crate::session::DurablePromptDeliveryPhase::Dispatching
                        | crate::session::DurablePromptDeliveryPhase::Delivered
                )
            )
        {
            return Ok(None);
        }
        self.populate_remote_prompt_recovery_workflow_context(&mut dispatch)
            .await?;

        // Context resolution can await app work. Recheck exact prompt ownership before handing
        // the rebuilt dispatch to the network path.
        if !self.remote_prompt_recovery_is_current(
            session_id,
            agent_id,
            &dispatch.prompt_id,
            &dispatch.leased_agent_id,
        )? {
            return Ok(None);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(None);
        };
        if prompt.id() != dispatch.prompt_id
            || prompt.durable_delivery_reconciliation_pending()
            || matches!(
                prompt.durable_delivery_phase(),
                Some(
                    crate::session::DurablePromptDeliveryPhase::Dispatching
                        | crate::session::DurablePromptDeliveryPhase::Delivered
                )
            )
        {
            return Ok(None);
        }
        Ok(Some(dispatch))
    }

    async fn run_remote_prompt_dispatch_with_claim(
        &self,
        mut claim: RemotePromptAgentClaim,
        mut dispatch: Option<crate::app::KernelRemotePromptDispatch>,
    ) {
        let (session_id, agent_id) = claim.key.clone();
        loop {
            if let Some(dispatch) = dispatch.take() {
                self.dispatch_remote_prompt_once(dispatch).await;
            }
            if !claim.release_or_restart() {
                return;
            }
            dispatch = match self
                .remote_prompt_dispatch_after_claim_restart(&session_id, &agent_id)
                .await
            {
                Ok(dispatch) => dispatch,
                Err(error) => {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt claim restart could not prepare the current prompt",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": agent_id,
                            "error": error.to_string(),
                        }),
                    );
                    None
                }
            };
        }
    }

    async fn dispatch_remote_prompt_once(
        &self,
        mut dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        crate::logging::info_with_fields(
            "daemon.remote_prompt_dispatch",
            "remote prompt dispatch starting",
            serde_json::json!({
                "session_id": dispatch.session_id,
                "agent_id": dispatch.agent_id,
                "worker_kernel_id": dispatch.worker_kernel_id,
                "leased_agent_id": dispatch.leased_agent_id,
                "source_attachment_id": dispatch.source_attachment_id,
            }),
        );
        if let Err(error) = self.owned.mark_active_prompt_delivery(
            &dispatch.session_id,
            &dispatch.agent_id,
            &dispatch.prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        ) {
            let _ = self
                .finish_remote_prompt_dispatch(dispatch, Err(error))
                .await;
            return;
        }
        let agent = match self.owned.agent_store.get_agent(&dispatch.agent_id) {
            Ok(agent) => agent,
            Err(error) => {
                let _ = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return;
            }
        };
        let (prompt, _) = match self
            .prepare_remote_prompt_skill_context(&agent, &dispatch.prompt)
            .await
        {
            Ok(context) => context,
            Err(error) => {
                let _ = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return;
            }
        };
        let attachments = dispatch.attachments.clone();
        let serialized_attachments = match tokio::task::spawn_blocking(move || {
            crate::app::serialize_remote_prompt_attachments(&attachments)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => Err(DaemonError::LocalTransport {
                operation: "serialize remote prompt attachments",
                message: error.to_string(),
            }),
        };
        let attachments = match serialized_attachments {
            Ok(attachments) => attachments,
            Err(error) => {
                let _ = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await;
                return;
            }
        };
        let result = submit_remote_prompt_to_worker_with_binding_refresh(
            self,
            &mut dispatch,
            prompt,
            attachments,
        )
        .await;
        match &result {
            Ok(provider_run_id) => crate::logging::info_with_fields(
                "daemon.remote_prompt_dispatch",
                "remote prompt dispatch submitted",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "remote_provider_run_id": provider_run_id,
                }),
            ),
            Err(error) => crate::logging::warn_with_fields(
                "daemon.remote_prompt_dispatch",
                "remote prompt dispatch failed",
                serde_json::json!({
                    "session_id": dispatch.session_id,
                    "agent_id": dispatch.agent_id,
                    "worker_kernel_id": dispatch.worker_kernel_id,
                    "leased_agent_id": dispatch.leased_agent_id,
                    "error": error.to_string(),
                }),
            ),
        }
        if result
            .as_ref()
            .err()
            .is_some_and(remote_prompt_error_is_reconciliation_pending)
        {
            // An indeterminate submission must keep the same durable prompt active.
            // The ordinary error finalizer would cancel it and promote the backlog.
            return;
        }
        match result {
            Ok(remote_provider_run_id) => {
                if let Err(error) = self
                    .finish_remote_prompt_dispatch(dispatch.clone(), Ok(remote_provider_run_id))
                    .await
                {
                    let message = format!(
                        "Worker accepted prompt `{}`, but the home kernel could not record its acknowledgement: {error}. Delivery is uncertain; do not replay this prompt without checking the worker.",
                        dispatch.prompt_id,
                    );
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        &message,
                        serde_json::json!({
                            "session_id": dispatch.session_id,
                            "agent_id": dispatch.agent_id,
                            "prompt_id": dispatch.prompt_id,
                            "worker_kernel_id": dispatch.worker_kernel_id,
                            "leased_agent_id": dispatch.leased_agent_id,
                            "delivery_uncertain": true,
                        }),
                    );
                    self.owned.fan_out_remote_dispatch_error(
                        &dispatch,
                        &format!("remote-dispatch:{}", dispatch.prompt_id),
                        Some(format!(
                            "remote-dispatch-ack-persist-failed:{}",
                            dispatch.prompt_id
                        )),
                        &message,
                    );
                }
            }
            Err(error) => {
                if let Err(settlement_error) = self
                    .finish_remote_prompt_dispatch(dispatch, Err(error))
                    .await
                {
                    crate::logging::warn_with_fields(
                        "daemon.remote_prompt_dispatch",
                        "remote prompt failure could not be settled",
                        serde_json::json!({"error": settlement_error.to_string()}),
                    );
                }
            }
        }
    }

    pub(crate) fn spawn_remote_prompt_dispatch(
        &self,
        dispatch: crate::app::KernelRemotePromptDispatch,
    ) {
        // A stale projection drain can discover a dead lease while the initial
        // dispatch is already refreshing that same binding. Both paths submit
        // the active prompt, so serialize them per agent to prevent one browser
        // prompt from starting on two freshly-created worker agents.
        let Some(claim) = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&self.owned.remote_prompt_recoveries),
            &dispatch.session_id,
            &dispatch.agent_id,
        ) else {
            return;
        };
        let state = self.clone();
        tokio::spawn(async move {
            state
                .run_remote_prompt_dispatch_with_claim(claim, Some(dispatch))
                .await;
        });
    }
}

fn same_remote_prompt_worker_binding(
    current: &crate::agent::RemoteAgentBinding,
    expected: &crate::agent::RemoteAgentBinding,
) -> bool {
    current.worker_kernel_id == expected.worker_kernel_id
        && current.worker_machine_id == expected.worker_machine_id
        && current.execution_lease_id == expected.execution_lease_id
        && current.leased_agent_id == expected.leased_agent_id
        && current.relay_url == expected.relay_url
        && current.relay_token == expected.relay_token
        && current.relay_peer_protocol_version == expected.relay_peer_protocol_version
}

fn completed_receipt_projection_matches(
    dispatch: &crate::app::KernelRemotePromptDispatch,
    worker_provider_run_id: &str,
    event: &RelayPeerEvent,
) -> bool {
    let RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id,
        home_agent_id,
        provider_run_id,
        provider_run,
        completions,
        ..
    } = event;
    home_session_id == &dispatch.session_id
        && home_agent_id == &dispatch.agent_id
        && provider_run_id == worker_provider_run_id
        && provider_run
            .as_ref()
            .is_none_or(|run| run.id() == worker_provider_run_id)
        && completions.iter().any(|completion| {
            completion.home_prompt_id.as_deref() == Some(dispatch.prompt_id.as_str())
        })
}

fn remote_prompt_projection_error_should_refresh_binding(
    recovering_missing_run_binding: bool,
    error: &DaemonError,
) -> bool {
    !recovering_missing_run_binding && remote_prompt_error_should_refresh_binding(error)
}

fn remote_prompt_recovery_delay(attempt: u32) -> std::time::Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(3);
    std::time::Duration::from_millis(500_u64.saturating_mul(multiplier))
}

#[cfg(test)]
mod tests {
    use super::super::remote_prompt_worker_submission_runtime::query_remote_prompt_worker_receipt_with_transport;
    use super::*;
    use chariox_relay::protocol::RelayEnvelope;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::Arc;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::Mutex;
    use tokio_tungstenite::{accept_async, tungstenite::Message, WebSocketStream};

    async fn receive_claim_test_envelope(socket: &mut WebSocketStream<TcpStream>) -> RelayEnvelope {
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .expect("temporary relay should receive a client envelope before timeout")
            .expect("temporary relay socket should remain open")
            .expect("temporary relay frame should decode");
        serde_json::from_str(
            message
                .to_text()
                .expect("temporary relay envelope should be text"),
        )
        .expect("temporary relay envelope should deserialize")
    }

    async fn send_claim_test_envelope(
        socket: &mut WebSocketStream<TcpStream>,
        envelope: RelayEnvelope,
    ) {
        socket
            .send(Message::Text(
                serde_json::to_string(&envelope)
                    .expect("temporary relay envelope should serialize")
                    .into(),
            ))
            .await
            .expect("temporary relay envelope should send");
    }

    async fn accept_claim_test_prompt(
        listener: &TcpListener,
        worker_id: &str,
        machine_id: &str,
        worker_public_key: &str,
        worker_private_key: &str,
    ) -> (WebSocketStream<TcpStream>, String, String, String, String) {
        let (stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                .await
                .expect("temporary relay should accept discovery connection before timeout")
                .expect("temporary relay listener should remain open");
        let mut discovery = accept_async(stream)
            .await
            .expect("temporary relay should upgrade discovery connection");
        let RelayEnvelope::ClientMetadataRequest { request_id, .. } =
            receive_claim_test_envelope(&mut discovery).await
        else {
            panic!("expected relay metadata request");
        };
        let presence = serde_json::from_value(serde_json::json!({
            "kernel_id": worker_id,
            "machine_id": machine_id,
            "public_key": worker_public_key,
        }))
        .expect("fake worker presence should deserialize");
        send_claim_test_envelope(
            &mut discovery,
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: None,
                kernel: Some(presence),
                error: None,
            },
        )
        .await;
        drop(discovery);

        let (stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(2), listener.accept())
                .await
                .expect("temporary relay should accept peer connection before timeout")
                .expect("temporary relay listener should remain open");
        let mut peer = accept_async(stream)
            .await
            .expect("temporary relay should upgrade peer connection");
        assert!(matches!(
            receive_claim_test_envelope(&mut peer).await,
            RelayEnvelope::DaemonRegister { .. }
        ));
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            encrypted_request,
            ..
        } = receive_claim_test_envelope(&mut peer).await
        else {
            panic!("expected leased prompt submission");
        };
        let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
            worker_private_key,
            &encrypted_request,
        )
        .expect("fake worker should decrypt the prompt submission");
        let crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
            leased_agent_id,
            prompt,
            git_context: Some(git_context),
            ..
        } = serde_json::from_slice(&decrypted.plaintext)
            .expect("fake worker request should decode")
        else {
            panic!("expected SubmitLeasedPrompt with home turn context");
        };
        (
            peer,
            request_id,
            decrypted.sender_public_key,
            git_context.home_prompt_id,
            format!("{leased_agent_id}\n{prompt}"),
        )
    }

    async fn acknowledge_claim_test_prompt(
        peer: &mut WebSocketStream<TcpStream>,
        request_id: String,
        sender_public_key: &str,
        worker_id: &str,
        worker_private_key: &str,
        prompt_id: &str,
        leased_agent_id: &str,
        prompt: &str,
        run_id: &str,
    ) {
        let response = crate::transport::relay_peer::RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id: run_id.to_string(),
            outcome: crate::session::PromptSubmissionOutcome::Started {
                prompt: crate::session::PromptQueueItem::new(
                    format!("worker-{prompt_id}"),
                    "worker-attachment",
                    leased_agent_id,
                    prompt,
                    crate::session::PromptStatus::Running,
                ),
            },
        };
        let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
            worker_private_key,
            sender_public_key,
            &serde_json::to_vec(&response).expect("fake worker response should encode"),
        )
        .expect("fake worker should encrypt the response");
        send_claim_test_envelope(
            peer,
            RelayEnvelope::DaemonPeerResponse {
                request_id,
                from_daemon_id: worker_id.to_string(),
                encrypted_response: Some(encrypted_response),
                error: None,
            },
        )
        .await;
    }

    async fn assert_no_duplicate_claim_submission(
        listener: &TcpListener,
        worker_id: &str,
        machine_id: &str,
        worker_public_key: &str,
        worker_private_key: &str,
    ) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(300);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let Ok(Ok((stream, _))) = tokio::time::timeout(remaining, listener.accept()).await
            else {
                return;
            };
            let mut socket = accept_async(stream)
                .await
                .expect("temporary relay should upgrade observation connection");
            match receive_claim_test_envelope(&mut socket).await {
                RelayEnvelope::ClientMetadataRequest { request_id, .. } => {
                    let presence = serde_json::from_value(serde_json::json!({
                        "kernel_id": worker_id,
                        "machine_id": machine_id,
                        "public_key": worker_public_key,
                    }))
                    .expect("fake worker presence should deserialize");
                    send_claim_test_envelope(
                        &mut socket,
                        RelayEnvelope::ClientMetadataResponse {
                            request_id,
                            machines: None,
                            kernels: None,
                            kernel: Some(presence),
                            error: None,
                        },
                    )
                    .await;
                }
                RelayEnvelope::DaemonRegister { .. } => {
                    let RelayEnvelope::DaemonPeerRequest {
                        request_id,
                        encrypted_request,
                        ..
                    } = receive_claim_test_envelope(&mut socket).await
                    else {
                        panic!("expected a fake worker peer request after registration");
                    };
                    let decrypted =
                        crate::transport::relay_crypto::decrypt_payload_for_private_key(
                            worker_private_key,
                            &encrypted_request,
                        )
                        .expect("fake worker should decrypt the observed request");
                    let request: crate::transport::relay_peer::RelayPeerRequest =
                        serde_json::from_slice(&decrypted.plaintext)
                            .expect("observed worker request should decode");
                    match request {
                        crate::transport::relay_peer::RelayPeerRequest::SubmitLeasedPrompt {
                            ..
                        } => panic!("the same active successor must not be submitted twice"),
                        crate::transport::relay_peer::RelayPeerRequest::DrainLeasedRuntimeProjection {
                            ..
                        } => {
                            let response = crate::transport::relay_peer::RelayPeerResponse::LeasedRuntimeProjectionDrained {
                                event: None,
                            };
                            let encrypted_response =
                                crate::transport::relay_crypto::encrypt_payload_for_peer(
                                    worker_private_key,
                                    &decrypted.sender_public_key,
                                    &serde_json::to_vec(&response)
                                        .expect("projection response should encode"),
                                )
                                .expect("fake worker should encrypt projection response");
                            send_claim_test_envelope(
                                &mut socket,
                                RelayEnvelope::DaemonPeerResponse {
                                    request_id,
                                    from_daemon_id: worker_id.to_string(),
                                    encrypted_response: Some(encrypted_response),
                                    error: None,
                                },
                            )
                            .await;
                        }
                        other => panic!("unexpected observed worker request: {other:?}"),
                    }
                }
                other => panic!("unexpected fake relay observation envelope: {other:?}"),
            }
        }
    }

    struct FakeRelayPeerRequest {
        socket: WebSocketStream<TcpStream>,
        request_id: String,
        target_id: String,
        request: RelayPeerRequest,
    }

    async fn receive_fake_relay_envelope(socket: &mut WebSocketStream<TcpStream>) -> RelayEnvelope {
        let message = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .expect("fake relay frame should arrive before timeout")
            .expect("fake relay socket should remain open")
            .expect("fake relay frame should decode");
        serde_json::from_str(
            message
                .to_text()
                .expect("fake relay envelope should be text"),
        )
        .expect("fake relay envelope should deserialize")
    }

    async fn send_fake_relay_envelope(
        socket: &mut WebSocketStream<TcpStream>,
        envelope: RelayEnvelope,
    ) {
        socket
            .send(Message::Text(
                serde_json::to_string(&envelope)
                    .expect("fake relay envelope should serialize")
                    .into(),
            ))
            .await
            .expect("fake relay envelope should send");
    }

    async fn receive_fake_worker_peer_request(
        listener: &TcpListener,
        worker_id: &str,
        worker_machine_id: &str,
        worker_public_key: &str,
        worker_private_key: &str,
    ) -> FakeRelayPeerRequest {
        let (stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                .await
                .expect("fake relay should accept metadata connection")
                .expect("fake relay metadata listener should accept");
        let mut discovery = accept_async(stream)
            .await
            .expect("fake relay should upgrade metadata connection");
        let RelayEnvelope::ClientMetadataRequest { request_id, .. } =
            receive_fake_relay_envelope(&mut discovery).await
        else {
            panic!("expected public kernel metadata request");
        };
        let presence = serde_json::from_value(serde_json::json!({
            "kernel_id": worker_id,
            "machine_id": worker_machine_id,
            "public_key": worker_public_key,
        }))
        .expect("fake worker presence should deserialize");
        send_fake_relay_envelope(
            &mut discovery,
            RelayEnvelope::ClientMetadataResponse {
                request_id,
                machines: None,
                kernels: None,
                kernel: Some(presence),
                error: None,
            },
        )
        .await;
        drop(discovery);

        let (stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                .await
                .expect("fake relay should accept temporary peer connection")
                .expect("fake relay peer listener should accept");
        let mut socket = accept_async(stream)
            .await
            .expect("fake relay should upgrade peer connection");
        assert!(matches!(
            receive_fake_relay_envelope(&mut socket).await,
            RelayEnvelope::DaemonRegister { .. }
        ));
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            target,
            encrypted_request,
        } = receive_fake_relay_envelope(&mut socket).await
        else {
            panic!("expected a public kernel peer request");
        };
        let target_id = target
            .daemon_id
            .or(target.daemon_alias)
            .expect("kernel peer request should identify its target");
        let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
            worker_private_key,
            &encrypted_request,
        )
        .expect("fake worker should decrypt the public kernel request");
        let request = serde_json::from_slice(&decrypted.plaintext)
            .expect("fake worker request should deserialize");
        FakeRelayPeerRequest {
            socket,
            request_id,
            target_id,
            request,
        }
    }

    async fn send_fake_worker_peer_response(
        request: FakeRelayPeerRequest,
        worker_id: &str,
        worker_private_key: &str,
        home_public_key: &str,
        response: RelayPeerResponse,
    ) {
        let encrypted_response = crate::transport::relay_crypto::encrypt_payload_for_peer(
            worker_private_key,
            home_public_key,
            &serde_json::to_vec(&response).expect("fake worker response should serialize"),
        )
        .expect("fake worker should encrypt its public kernel response");
        let mut socket = request.socket;
        send_fake_relay_envelope(
            &mut socket,
            RelayEnvelope::DaemonPeerResponse {
                request_id: request.request_id,
                from_daemon_id: worker_id.to_string(),
                encrypted_response: Some(encrypted_response),
                error: None,
            },
        )
        .await;
    }

    struct ReceiptReconciliationFixture {
        app: Arc<Mutex<DaemonApp>>,
        runtime: KernelRuntimeState,
        session_id: String,
        agent_id: String,
        dispatch: crate::app::KernelRemotePromptDispatch,
        successor_prompt: crate::session::PromptQueueItem,
        successor_prompt_id: String,
        home_public_key: String,
        worker_id: String,
        worker_machine_id: String,
        worker_public_key: String,
        worker_private_key: String,
        leased_agent_id: String,
    }

    async fn make_receipt_reconciliation_fixture(
        relay_url: &str,
        suffix: &str,
    ) -> ReceiptReconciliationFixture {
        let mut home_config = crate::config::DaemonConfig::for_tests();
        home_config.relay_url = Some(relay_url.to_string());
        home_config.relay_token = Some(format!("receipt-home-token-{suffix}"));
        home_config.relay_request_timeout_ms = 3_000;
        let home_public_key = home_config.relay_public_key.clone();
        let worker_config = crate::config::DaemonConfig::for_tests();
        let worker_public_key = worker_config.relay_public_key.clone();
        let worker_private_key = worker_config.relay_private_key.clone();
        let worker_id = format!("worker-receipt-{suffix}");
        let worker_machine_id = format!("machine-receipt-{suffix}");
        let leased_agent_id = format!("leased-agent-receipt-{suffix}");

        let mut app = DaemonApp::bootstrap(home_config).expect("home app should bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                format!("workspace-receipt-{suffix}"),
                format!("worktree-receipt-{suffix}"),
            ))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                format!("receipt-client-{suffix}"),
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("home attachment should be created");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: worker_id.clone(),
                    worker_machine_id: worker_machine_id.clone(),
                    execution_lease_id: format!("lease-receipt-{suffix}"),
                    leased_agent_id: leased_agent_id.clone(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("home agent should bind to fake worker");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let requested_prompt_id = format!("uncertain-prompt-{suffix}");
        let started = app
            .prompt_owner_submit_prepared_prompt(
                &session_id,
                crate::session::PromptQueueItem::new(
                    &requested_prompt_id,
                    &attachment_id,
                    &agent_id,
                    "exact prompt whose worker admission is uncertain",
                    crate::session::PromptStatus::Queued,
                ),
                false,
            )
            .expect("uncertain home prompt should start");
        let crate::session::PromptSubmissionOutcome::Started {
            prompt: active_prompt,
        } = started
        else {
            panic!("uncertain home prompt should be active");
        };
        let prompt_id = active_prompt.id().to_string();
        app.mark_active_prompt_delivery(
            &session_id,
            &agent_id,
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        )
        .expect("home prompt should persist its uncertain Dispatching phase");

        let queued = app
            .prompt_owner_submit_prepared_prompt(
                &session_id,
                crate::session::PromptQueueItem::new(
                    format!("successor-input-{suffix}"),
                    &attachment_id,
                    &agent_id,
                    "one successor must remain ordered behind the uncertain prompt",
                    crate::session::PromptStatus::Queued,
                ),
                true,
            )
            .expect("successor should queue");
        let crate::session::PromptSubmissionOutcome::Queued {
            prompt: successor_prompt,
        } = queued
        else {
            panic!("successor should remain queued behind Dispatching prompt");
        };
        let successor_prompt_id = successor_prompt.id().to_string();

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let agent_instance = runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .expect("home agent should remain available");
        let dispatch = runtime
            .remote_prompt_recovery_dispatch(&agent_instance)
            .expect("recovery dispatch should reconstruct")
            .expect("uncertain prompt should remain active");

        ReceiptReconciliationFixture {
            app,
            runtime,
            session_id,
            agent_id,
            dispatch,
            successor_prompt,
            successor_prompt_id,
            home_public_key,
            worker_id,
            worker_machine_id,
            worker_public_key,
            worker_private_key,
            leased_agent_id,
        }
    }

    #[tokio::test]
    async fn completed_worker_receipt_drains_projection_and_durably_promotes_one_successor() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake relay listener should bind");
        let relay_url = format!("ws://{}", listener.local_addr().unwrap());
        let fixture = make_receipt_reconciliation_fixture(&relay_url, "completed").await;
        // Completion admits the queued successor and normally starts its network
        // dispatch. Hold the per-agent claim so this test ends at the durable,
        // observable admission boundary without issuing a second prompt request.
        let _dispatch_claim = RemotePromptAgentClaim::try_acquire(
            Arc::clone(&fixture.runtime.owned.remote_prompt_recoveries),
            &fixture.session_id,
            &fixture.agent_id,
        )
        .expect("test should hold the successor dispatch claim");

        let app = Arc::clone(&fixture.app);
        let listener_worker_id = fixture.worker_id.clone();
        let listener_worker_machine_id = fixture.worker_machine_id.clone();
        let listener_worker_public_key = fixture.worker_public_key.clone();
        let listener_worker_private_key = fixture.worker_private_key.clone();
        let listener_home_public_key = fixture.home_public_key.clone();
        let listener_leased_agent_id = fixture.leased_agent_id.clone();
        let listener_prompt_id = fixture.dispatch.prompt_id.clone();
        let listener_session_id = fixture.session_id.clone();
        let listener_agent_id = fixture.agent_id.clone();
        let server = tokio::spawn(async move {
            let receipt_request = receive_fake_worker_peer_request(
                &listener,
                &listener_worker_id,
                &listener_worker_machine_id,
                &listener_worker_public_key,
                &listener_worker_private_key,
            )
            .await;
            assert_eq!(receipt_request.target_id, listener_worker_id);
            assert!(
                app.try_lock().is_ok(),
                "receipt query must not hold DaemonApp"
            );
            assert!(matches!(
                &receipt_request.request,
                RelayPeerRequest::GetLeasedPromptReceipt {
                    leased_agent_id,
                    home_prompt_id,
                } if leased_agent_id == &listener_leased_agent_id
                    && home_prompt_id == &listener_prompt_id
            ));
            send_fake_worker_peer_response(
                receipt_request,
                &listener_worker_id,
                &listener_worker_private_key,
                &listener_home_public_key,
                RelayPeerResponse::LeasedPromptReceiptQueried {
                    receipt: Some(crate::transport::relay_peer::LeasedPromptReceipt {
                        home_prompt_id: listener_prompt_id.clone(),
                        worker_provider_run_id: "worker-run-completed-receipt".to_string(),
                        phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed,
                    }),
                },
            )
            .await;

            let projection_request = receive_fake_worker_peer_request(
                &listener,
                &listener_worker_id,
                &listener_worker_machine_id,
                &listener_worker_public_key,
                &listener_worker_private_key,
            )
            .await;
            assert_eq!(projection_request.target_id, listener_worker_id);
            assert!(
                app.try_lock().is_ok(),
                "projection drain must not hold DaemonApp"
            );
            assert!(matches!(
                &projection_request.request,
                RelayPeerRequest::DrainLeasedRuntimeProjection {
                    leased_agent_id,
                    provider_run_id,
                    pump_output: true,
                } if leased_agent_id == &listener_leased_agent_id
                    && provider_run_id.as_str() == "worker-run-completed-receipt"
            ));
            send_fake_worker_peer_response(
                projection_request,
                &listener_worker_id,
                &listener_worker_private_key,
                &listener_home_public_key,
                RelayPeerResponse::LeasedRuntimeProjectionDrained {
                    event: Some(RelayPeerEvent::LeasedRuntimeProjection {
                        home_session_id: listener_session_id,
                        home_agent_id: listener_agent_id,
                        provider_run_id: "worker-run-completed-receipt".to_string(),
                        provider_run: None,
                        prompts: Vec::new(),
                        output_chunks: Vec::new(),
                        notices: Vec::new(),
                        completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                            message_id: "completed-receipt-assistant-message".to_string(),
                            completed_at_ms: 1,
                            home_prompt_id: Some(listener_prompt_id),
                            provider_termination: None,
                        }],
                    }),
                },
            )
            .await;
        });

        assert!(fixture
            .runtime
            .recover_remote_prompt_after_kernel_restart(
                &fixture.session_id,
                &fixture.agent_id,
                Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
                None,
            )
            .await
            .expect("completed worker receipt should reconcile"));
        server
            .await
            .expect("fake relay should serve receipt and projection");

        let session = fixture
            .runtime
            .owned
            .session_store
            .get_session(&fixture.session_id)
            .expect("home session should remain available");
        let (active, queued) = fixture
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &fixture.agent_id);
        let active = active.expect("exactly one queued successor should be admitted");
        assert_ne!(active.id(), fixture.successor_prompt_id);
        assert!(active.id().starts_with("prompt-"));
        assert_eq!(active.pending_prompt_id(), None);
        assert_eq!(active.prompt(), fixture.successor_prompt.prompt());
        assert!(queued.is_empty(), "the successor should be promoted once");

        let events = fixture
            .runtime
            .owned
            .operational_history_store
            .load_session_events_for_agent_sequence_range(
                &fixture.session_id,
                &fixture.agent_id,
                0,
                i64::MAX as u64,
            )
            .expect("durable home history should load");
        let settlements = events
            .iter()
            .filter(|event| {
                event.prompt_id.as_deref() == Some(fixture.dispatch.prompt_id.as_str())
                    && event
                        .metadata
                        .contains_key(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
            })
            .count();
        assert_eq!(
            settlements, 1,
            "home settlement should be durable exactly once"
        );
    }

    #[tokio::test]
    async fn conflicting_worker_receipt_keeps_same_prompt_held_without_replay() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake relay listener should bind");
        let relay_url = format!("ws://{}", listener.local_addr().unwrap());
        let fixture = make_receipt_reconciliation_fixture(&relay_url, "conflict").await;
        let app = Arc::clone(&fixture.app);
        let listener_worker_id = fixture.worker_id.clone();
        let listener_worker_machine_id = fixture.worker_machine_id.clone();
        let listener_worker_public_key = fixture.worker_public_key.clone();
        let listener_worker_private_key = fixture.worker_private_key.clone();
        let listener_home_public_key = fixture.home_public_key.clone();
        let listener_leased_agent_id = fixture.leased_agent_id.clone();
        let listener_prompt_id = fixture.dispatch.prompt_id.clone();
        let server = tokio::spawn(async move {
            let receipt_request = receive_fake_worker_peer_request(
                &listener,
                &listener_worker_id,
                &listener_worker_machine_id,
                &listener_worker_public_key,
                &listener_worker_private_key,
            )
            .await;
            assert_eq!(receipt_request.target_id, listener_worker_id);
            assert!(
                app.try_lock().is_ok(),
                "receipt query must not hold DaemonApp"
            );
            assert!(matches!(
                &receipt_request.request,
                RelayPeerRequest::GetLeasedPromptReceipt {
                    leased_agent_id,
                    home_prompt_id,
                } if leased_agent_id == &listener_leased_agent_id
                    && home_prompt_id == &listener_prompt_id
            ));
            send_fake_worker_peer_response(
                receipt_request,
                &listener_worker_id,
                &listener_worker_private_key,
                &listener_home_public_key,
                RelayPeerResponse::LeasedPromptReceiptQueried {
                    receipt: Some(crate::transport::relay_peer::LeasedPromptReceipt {
                        home_prompt_id: "different-home-prompt".to_string(),
                        worker_provider_run_id: "worker-run-conflicting-receipt".to_string(),
                        phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Active,
                    }),
                },
            )
            .await;

            // A replay would issue a second public kernel relay request. It must
            // not be sent when the read-only receipt conflicts with the prompt.
            tokio::time::timeout(std::time::Duration::from_secs(1), listener.accept())
                .await
                .is_ok()
        });

        assert!(fixture
            .runtime
            .recover_remote_prompt_after_kernel_restart(
                &fixture.session_id,
                &fixture.agent_id,
                Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
                None,
            )
            .await
            .expect("conflicting receipt should leave recovery handled"));
        assert!(
            !server
                .await
                .expect("fake relay should finish conflict check"),
            "conflicting receipt must not trigger a replay request"
        );

        let session = fixture
            .runtime
            .owned
            .session_store
            .get_session(&fixture.session_id)
            .expect("home session should remain available");
        let (active, queued) = fixture
            .runtime
            .owned
            .prompt_state_owner
            .state_parts(&session, &fixture.agent_id);
        let active = active.expect("uncertain prompt should remain active");
        assert_eq!(active.id(), fixture.dispatch.prompt_id);
        assert_eq!(
            active.prompt(),
            "exact prompt whose worker admission is uncertain"
        );
        assert_eq!(
            active.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
        assert!(active.durable_delivery_reconciliation_pending());
        assert_eq!(queued.len(), 1, "successor must remain queued");
        assert_eq!(queued[0].id(), fixture.successor_prompt_id);
        assert_eq!(queued[0].prompt(), fixture.successor_prompt.prompt());
        assert!(fixture
            .runtime
            .owned
            .agent_store
            .get_agent(&fixture.agent_id)
            .expect("home agent should remain available")
            .remote_execution()
            .expect("home agent should remain remote")
            .active_worker_provider_run_id
            .is_none());
        let events = fixture
            .runtime
            .owned
            .operational_history_store
            .load_session_events_for_agent_sequence_range(
                &fixture.session_id,
                &fixture.agent_id,
                0,
                i64::MAX as u64,
            )
            .expect("durable home history should load");
        assert!(!events.iter().any(|event| {
            event.prompt_id.as_deref() == Some(fixture.dispatch.prompt_id.as_str())
                && event
                    .metadata
                    .contains_key(crate::history::PROMPT_SETTLED_AT_MS_METADATA_KEY)
        }));
    }

    #[test]
    fn exact_worker_receipt_selects_active_association_completed_drain_or_hold() {
        let active = crate::transport::relay_peer::LeasedPromptReceipt {
            home_prompt_id: "home-prompt-1".to_string(),
            worker_provider_run_id: "worker-run-1".to_string(),
            phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Active,
        };
        assert_eq!(
            remote_prompt_receipt_action("home-prompt-1", Some(active.clone())),
            Ok(RemotePromptReceiptAction::AssociateActiveRun(
                "worker-run-1".to_string()
            ))
        );
        let active_action = remote_prompt_receipt_action("home-prompt-1", Some(active)).unwrap();
        assert!(!remote_prompt_receipt_action_requires_projection(
            &active_action
        ));
        let completed = crate::transport::relay_peer::LeasedPromptReceipt {
            home_prompt_id: "home-prompt-1".to_string(),
            worker_provider_run_id: "worker-run-2".to_string(),
            phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed,
        };
        assert_eq!(
            remote_prompt_receipt_action("home-prompt-1", Some(completed.clone())),
            Ok(RemotePromptReceiptAction::DrainCompletedProjection(
                "worker-run-2".to_string()
            ))
        );
        let completed_action =
            remote_prompt_receipt_action("home-prompt-1", Some(completed)).unwrap();
        assert!(remote_prompt_receipt_action_requires_projection(
            &completed_action
        ));
        assert_eq!(
            remote_prompt_receipt_action("home-prompt-1", None),
            Err("worker returned no receipt for the exact home prompt")
        );
        assert_eq!(
            remote_prompt_receipt_action(
                "home-prompt-1",
                Some(crate::transport::relay_peer::LeasedPromptReceipt {
                    home_prompt_id: "other-prompt".to_string(),
                    worker_provider_run_id: "worker-run-3".to_string(),
                    phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Active,
                })
            ),
            Err("worker receipt names a different home prompt")
        );
    }

    #[tokio::test]
    async fn receipt_query_targets_exact_prompt_after_releasing_the_app_lock() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-receipt-query",
                "worktree-receipt-query",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "receipt-query-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-receipt-query".to_string(),
                    worker_machine_id: "machine-receipt-query".to_string(),
                    execution_lease_id: "lease-receipt-query".to_string(),
                    leased_agent_id: "leased-receipt-query".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let submitted = app
            .prompt_owner_submit_prepared_prompt(
                session.id(),
                crate::session::PromptQueueItem::new(
                    "home-prompt-receipt-query",
                    attachment.id(),
                    agent.id(),
                    "uncertain prompt",
                    crate::session::PromptStatus::Queued,
                ),
                false,
            )
            .unwrap();
        let prompt_id = match submitted {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("receipt query prompt should start")
            }
        };
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            None,
            None,
        )
        .unwrap();

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let agent = runtime.owned.agent_store.get_agent(agent.id()).unwrap();
        let dispatch = runtime
            .remote_prompt_recovery_dispatch(&agent)
            .unwrap()
            .unwrap();
        let app_during_send = Arc::clone(&app);
        let active_prompt_id = prompt_id.clone();
        let receipt = query_remote_prompt_worker_receipt_with_transport(
            &runtime,
            &dispatch,
            move |_config, target, request| async move {
                assert!(
                    app_during_send.try_lock().is_ok(),
                    "receipt transport must not run while DaemonApp is locked"
                );
                assert_eq!(target.daemon_id.as_deref(), Some("worker-receipt-query"));
                assert!(matches!(
                    request,
                    RelayPeerRequest::GetLeasedPromptReceipt {
                        leased_agent_id,
                        home_prompt_id,
                    } if leased_agent_id == "leased-receipt-query"
                        && home_prompt_id == active_prompt_id
                ));
                Ok(RelayPeerResponse::LeasedPromptReceiptQueried {
                    receipt: Some(crate::transport::relay_peer::LeasedPromptReceipt {
                        home_prompt_id: active_prompt_id,
                        worker_provider_run_id: "worker-receipt-run".to_string(),
                        phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Active,
                    }),
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(
            receipt.unwrap().worker_provider_run_id,
            "worker-receipt-run"
        );
        let completed_receipt = query_remote_prompt_worker_receipt_with_transport(
            &runtime,
            &dispatch,
            move |_config, _target, _request| async move {
                assert!(app.try_lock().is_ok());
                Ok(RelayPeerResponse::LeasedPromptReceiptQueried {
                    receipt: Some(crate::transport::relay_peer::LeasedPromptReceipt {
                        home_prompt_id: prompt_id.to_string(),
                        worker_provider_run_id: "worker-completed-run".to_string(),
                        phase: crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed,
                    }),
                })
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            completed_receipt.phase,
            crate::transport::relay_peer::LeasedPromptReceiptPhase::Completed
        );
        let no_receipt = query_remote_prompt_worker_receipt_with_transport(
            &runtime,
            &dispatch,
            |_config, _target, _request| async move {
                Ok(RelayPeerResponse::LeasedPromptReceiptQueried { receipt: None })
            },
        )
        .await
        .unwrap();
        assert!(no_receipt.is_none());
        let completion_event = RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: dispatch.session_id.clone(),
            home_agent_id: dispatch.agent_id.clone(),
            provider_run_id: "worker-receipt-run".to_string(),
            provider_run: None,
            prompts: Vec::new(),
            output_chunks: Vec::new(),
            notices: Vec::new(),
            completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "assistant-message-1".to_string(),
                completed_at_ms: 1,
                home_prompt_id: Some(dispatch.prompt_id.clone()),
                provider_termination: None,
            }],
        };
        assert!(completed_receipt_projection_matches(
            &dispatch,
            "worker-receipt-run",
            &completion_event
        ));
        let conflicting_event = RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: dispatch.session_id.clone(),
            home_agent_id: dispatch.agent_id.clone(),
            provider_run_id: "worker-receipt-run".to_string(),
            provider_run: None,
            prompts: Vec::new(),
            output_chunks: Vec::new(),
            notices: Vec::new(),
            completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "assistant-message-other".to_string(),
                completed_at_ms: 2,
                home_prompt_id: Some("another-home-prompt".to_string()),
                provider_termination: None,
            }],
        };
        assert!(!completed_receipt_projection_matches(
            &dispatch,
            "worker-receipt-run",
            &conflicting_event
        ));
    }

    #[tokio::test]
    async fn recovered_queued_remote_prompt_echo_uses_durable_queue_origin() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "queued-recovery-source",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-1".into(),
                    worker_machine_id: "machine-1".into(),
                    execution_lease_id: "lease-1".into(),
                    leased_agent_id: "leased-agent-1".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt: crate::session::PromptQueueItem::new(
                    "pending",
                    attachment.id(),
                    agent.id(),
                    "queued recovery prompt",
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: true,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap();
        let promoted = runtime
            .owned
            .advance_next_queued_remote_prompt_dispatch(session.id(), agent.id())
            .unwrap()
            .unwrap();
        let original_dispatch = promoted.remote_dispatch.unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap();
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent.id())
            .unwrap();
        assert_eq!(active.durable_initially_queued(), Some(true));
        assert!(
            active.durable_operation_id().is_none(),
            "ordinary queue origins must not require a command operation ID"
        );
        let private =
            crate::session::DurablePromptPrivateState::from_prompt(session.id(), &active).unwrap();
        let private: crate::session::DurablePromptPrivateState =
            serde_json::from_value(serde_json::to_value(private).unwrap()).unwrap();
        let mut restored: crate::session::PromptQueueItem =
            serde_json::from_value(serde_json::to_value(&active).unwrap()).unwrap();
        assert_eq!(restored.durable_initially_queued(), None);
        restored.restore_durable_private_state(&private);
        assert!(runtime
            .owned
            .prompt_state_owner
            .replace_active_prompt_if_matches(&session, agent.id(), &active, restored));
        // Discard the original dispatch intent, as happens on restart. Recovery
        // must reconstruct its echo policy from durable prompt ownership alone.
        drop(original_dispatch);
        let recovered = runtime
            .remote_prompt_recovery_dispatch(
                &runtime.owned.agent_store.get_agent(agent.id()).unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(recovered.prompt_id, active.id());
        assert!(runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .iter()
            .all(|record| record.kind != crate::terminal::TerminalOutputKind::PromptEcho));
        runtime
            .finish_remote_prompt_dispatch(recovered, Ok("worker-run-recovered".into()))
            .await
            .unwrap();
        let echoes = runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .into_iter()
            .filter(|record| record.kind == crate::terminal::TerminalOutputKind::PromptEcho)
            .collect::<Vec<_>>();
        assert_eq!(
            echoes.len(),
            1,
            "recovered queued prompt must echo once to its submitting attachment"
        );
        assert_eq!(echoes[0].prompt_id.as_deref(), Some(active.id()));
        assert_eq!(
            echoes[0].provider_run_id,
            crate::provider::projected_leased_provider_run_id(
                "leased-agent-1",
                "worker-run-recovered"
            )
        );
    }

    #[tokio::test]
    async fn dispatching_remote_prompt_without_durable_ack_is_held_on_restart() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-uncertain-restart",
                "worktree-uncertain-restart",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "uncertain-restart-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-uncertain".to_string(),
                    worker_machine_id: "machine-uncertain".to_string(),
                    execution_lease_id: "lease-uncertain".to_string(),
                    leased_agent_id: "leased-uncertain".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let started = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt: crate::session::PromptQueueItem::new(
                    "uncertain-active",
                    attachment.id(),
                    agent.id(),
                    "prompt that may have been accepted",
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            })
            .unwrap()
            .unwrap();
        let dispatch = started.remote_dispatch.unwrap();
        runtime
            .owned
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                &dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .unwrap();
        let queued_successor_submission = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt: crate::session::PromptQueueItem::new(
                    "queued-successor",
                    attachment.id(),
                    agent.id(),
                    "must remain queued",
                    crate::session::PromptStatus::Queued,
                ),
                force_queue: true,
                refresh_projection: true,
            })
            .unwrap()
            .expect("queued successor should be handled");
        let queued_successor_id = match queued_successor_submission.outcome {
            crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                panic!("forced successor should remain queued")
            }
        };
        let session_before_recovery = runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap();
        assert!(!runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session_before_recovery, agent.id())
            .unwrap()
            .durable_delivery_reconciliation_pending());

        let recovered = runtime
            .recover_remote_prompt_after_kernel_restart(
                session.id(),
                agent.id(),
                Some(crate::session::DurablePromptDeliveryPhase::Dispatching),
                None,
            )
            .await
            .unwrap();

        assert!(
            recovered,
            "uncertain dispatch should be handled without replay"
        );
        let session_after = runtime
            .owned
            .session_store
            .get_session(session.id())
            .unwrap();
        let (active, queued) = runtime
            .owned
            .prompt_state_owner
            .state_parts(&session_after, agent.id());
        let active = active.expect("same active prompt should remain");
        assert_eq!(active.id(), dispatch.prompt_id);
        assert_eq!(
            active.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
        );
        assert!(active.durable_delivery_reconciliation_pending());
        assert_eq!(queued.len(), 1, "successor must not be promoted");
        assert_eq!(queued[0].id(), queued_successor_id);
        assert_eq!(queued[0].prompt(), "must remain queued");
        assert!(runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .unwrap()
            .remote_execution()
            .unwrap()
            .active_worker_provider_run_id
            .is_none());
        let errors = runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .into_iter()
            .filter(|record| record.kind == crate::terminal::TerminalOutputKind::ProviderError)
            .collect::<Vec<_>>();
        assert!(errors.iter().any(|record| {
            let message = String::from_utf8_lossy(&record.bytes);
            message.contains(&dispatch.prompt_id) && message.contains("exact worker receipt")
        }));
    }

    fn completed_worker_projection(
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        home_prompt_id: &str,
    ) -> crate::transport::relay_peer::RelayPeerEvent {
        crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            home_session_id: session_id.to_string(),
            home_agent_id: agent_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            provider_run: None,
            prompts: Vec::new(),
            output_chunks: Vec::new(),
            notices: Vec::new(),
            completions: vec![crate::transport::relay_peer::RelayProjectedCompletion {
                message_id: "assistant-completed".to_string(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some(home_prompt_id.to_string()),
                provider_termination: None,
            }],
        }
    }

    fn actual_worker_output_then_completion(
        home_session_id: &str,
        home_agent_id: &str,
        home_prompt_id: &str,
    ) -> (
        crate::agent::RemoteAgentBinding,
        String,
        crate::transport::relay_peer::RelayPeerEvent,
        crate::transport::relay_peer::RelayPeerEvent,
    ) {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut worker = DaemonApp::bootstrap(config).expect("worker bootstrap should succeed");
        let lease = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .create_execution_lease(
                "home-kernel-live-recovery",
                home_session_id,
                home_agent_id,
                false,
                "owner-live-recovery",
            )
            .expect("worker lease should be created");
        let leased_agent = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                Some("default".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("worker leased agent should be created");
        let (provider_run_id, outcome) = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .submit_leased_prompt_with_workflow_context(
                &leased_agent.id,
                "remote prompt\n",
                Vec::new(),
                None,
                Some(crate::transport::relay_peer::RemoteGitTurnContext {
                    home_session_id: home_session_id.to_string(),
                    home_agent_id: home_agent_id.to_string(),
                    home_prompt_id: home_prompt_id.to_string(),
                    home_turn_id: home_prompt_id.to_string(),
                    source_attachment_id: None,
                    workspace_live_sync_mode: None,
                    prompt_origin: Some(crate::session::PromptOrigin::Chariox),
                    external_provider: None,
                    external_provider_session_id: None,
                    external_provider_turn_id: None,
                    prompt_summary: "remote prompt".to_string(),
                }),
                Vec::new(),
                None,
                crate::extension::RemoteExtensionManifest::default(),
            )
            .expect("worker prompt should submit");
        assert!(matches!(
            outcome,
            crate::session::PromptSubmissionOutcome::Started { .. }
        ));
        crate::app::RemoteLeaseRuntime::new(&mut worker)
            .set_leased_agent_provider_for_test(&leased_agent.id, "codex");
        let launch_request = crate::provider::LaunchProviderRequest::new(
            &leased_agent.backing_session_id,
            "codex",
            "codex",
            "default",
            "gpt-5.4",
        )
        .with_agent_id(&leased_agent.backing_agent_id)
        .with_client_interface(crate::provider::ProviderClientInterface::Chariox);
        let mut running_provider = crate::provider::RuntimeProviderRun::new(
            &provider_run_id,
            &launch_request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "codex:codex:gpt-5.4".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        running_provider.mark_running();
        worker.providers_mut().insert_run_for_test(running_provider);
        worker.fan_out_output_for_agent(
            &leased_agent.backing_session_id,
            &provider_run_id,
            Some(&leased_agent.backing_agent_id),
            crate::terminal::TerminalOutputKind::ProviderOutput,
            Some("assistant-output".to_string()),
            vec![leased_agent.backing_attachment_id.clone()],
            b"LIVE_RECOVERY_OUTPUT_ONCE",
        );
        worker.record_notice_for_agent(
            &leased_agent.backing_session_id,
            Some(&provider_run_id),
            Some(&leased_agent.backing_agent_id),
            vec![leased_agent.backing_attachment_id.clone()],
            "LIVE_RECOVERY_NOTICE_ONCE",
        );
        let output_event = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("worker output drain should succeed")
            .expect("worker output should project")
            .1;
        if let Some((_, duplicate_event)) = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("worker duplicate drain should succeed")
        {
            let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
                output_chunks,
                notices,
                ..
            } = duplicate_event;
            assert!(output_chunks.is_empty(), "worker output must not repeat");
            assert!(notices.is_empty(), "worker notice must not repeat");
        }
        worker
            .complete_active_prompt(
                &leased_agent.backing_session_id,
                &leased_agent.backing_agent_id,
                Some(&provider_run_id),
            )
            .expect("worker prompt should settle");
        let completion_event = crate::app::RemoteLeaseRuntime::new(&mut worker)
            .drain_leased_runtime_projection_with_recovery(
                &leased_agent.id,
                &provider_run_id,
                false,
                true,
            )
            .expect("worker completion drain should succeed")
            .expect("worker completion should project")
            .1;
        let binding = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-kernel-live-recovery".to_string(),
            worker_machine_id: "worker-machine-live-recovery".to_string(),
            execution_lease_id: lease.id,
            leased_agent_id: leased_agent.id,
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        (binding, provider_run_id, output_event, completion_event)
    }

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[tokio::test]
    async fn remote_prompt_projection_drain_respects_durable_delivery_phase() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-projection-drain",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("provider-run-worker-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let prompt = crate::session::PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "remote prompt\n",
            crate::session::PromptStatus::Queued,
        );
        let outcome = app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("remote prompt should start locally");
        let crate::session::PromptSubmissionOutcome::Started { prompt } = outcome else {
            panic!("remote prompt should start locally");
        };
        assert_eq!(
            app.prompt_owner_queued_prompt_count_for_agent(session.id(), agent.id())
                .expect("queue count should load"),
            0
        );

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let active = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available"),
                agent.id(),
            )
            .expect("remote prompt should remain active");
        assert_eq!(
            active.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Accepted)
        );
        assert!(
            runtime
                .remote_prompt_projection_drain_target(session.id(), agent.id())
                .is_none(),
            "an accepted prompt must not drain the prior worker run"
        );

        runtime
            .owned
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                prompt.id(),
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some("provider-run-worker-1".to_string()),
                None,
            )
            .expect("delivered phase should persist");
        assert!(
            runtime
                .remote_prompt_projection_drain_target(session.id(), agent.id())
                .is_some(),
            "a delivered prompt should drain its worker run"
        );
    }

    #[tokio::test]
    async fn delivered_remote_prompt_restores_worker_run_before_restart_drain() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-remote-restart",
                "worktree-remote-restart",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-restart",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-remote-restart",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let prompt_id = match app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("remote prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("remote prompt should start")
            }
        };
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some("provider-run-worker-1".to_string()),
            None,
        )
        .expect("delivery metadata should persist");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        assert!(runtime
            .recover_remote_prompt_after_kernel_restart(
                session.id(),
                agent.id(),
                Some(crate::session::DurablePromptDeliveryPhase::Delivered),
                Some("provider-run-worker-1"),
            )
            .await
            .expect("remote recovery should start"));
        let restored = runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("agent should remain available");
        assert_eq!(
            restored
                .remote_execution()
                .and_then(|binding| binding.active_worker_provider_run_id.as_deref()),
            Some("provider-run-worker-1")
        );
    }

    #[tokio::test]
    async fn delivered_remote_prompt_uses_durable_run_for_live_projection_drain() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-live-recovery",
                "worktree-live-recovery",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-live-recovery",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-live-recovery",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let prompt_id = match app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("remote prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("remote prompt should start")
            }
        };
        let (binding, provider_run_id, output_event, completion_event) =
            actual_worker_output_then_completion(session.id(), agent.id(), &prompt_id);
        let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            provider_run,
            output_chunks,
            notices,
            completions,
            ..
        } = &output_event;
        assert!(provider_run.is_some(), "worker status should project");
        assert_eq!(output_chunks.len(), 1);
        assert_eq!(notices, &["LIVE_RECOVERY_NOTICE_ONCE"]);
        assert!(completions.is_empty());
        let crate::transport::relay_peer::RelayPeerEvent::LeasedRuntimeProjection {
            output_chunks,
            notices,
            completions,
            ..
        } = &completion_event;
        assert!(
            output_chunks.is_empty(),
            "worker output must be deduplicated"
        );
        assert!(notices.is_empty(), "worker notice must be consumed once");
        assert_eq!(completions.len(), 1);
        assert_eq!(
            completions[0].home_prompt_id.as_deref(),
            Some(prompt_id.as_str())
        );
        app.agents
            .bind_remote_execution(agent.id(), binding.clone())
            .expect("agent should bind to the worker projection source");
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run_id.clone()),
            None,
        )
        .expect("delivery metadata should persist");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let (drain_binding, drain_provider_run_id) = runtime
            .remote_prompt_projection_drain_target(session.id(), agent.id())
            .expect("live drain must recover the durable delivered worker run");

        assert_eq!(drain_binding, binding);
        assert_eq!(drain_provider_run_id, provider_run_id);
        let mismatched_prompt = completed_worker_projection(
            session.id(),
            agent.id(),
            &provider_run_id,
            "other-home-prompt",
        );
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &mismatched_prompt,
                )
                .expect("mismatched prompt projection should be rejected safely"),
            RemotePromptRunBindingRecovery::Rejected,
        );
        let stale_run =
            completed_worker_projection(session.id(), agent.id(), "provider-run-stale", &prompt_id);
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &stale_run,
                )
                .expect("stale run projection should be rejected safely"),
            RemotePromptRunBindingRecovery::Rejected,
        );
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &output_event,
                )
                .expect("output-only worker projection should recover the binding"),
            RemotePromptRunBindingRecovery::Recovered,
        );
        runtime
            .project_remote_runtime_projection_event(output_event)
            .await
            .expect("output-only worker projection should reach the home session");
        let first_output = runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(
            first_output
                .iter()
                .filter(|record| record.bytes == b"LIVE_RECOVERY_OUTPUT_ONCE")
                .count(),
            1
        );
        let first_notices = runtime
            .owned
            .terminal_stream
            .drain_notice_records(session.id(), attachment.id());
        assert_eq!(
            first_notices
                .iter()
                .filter(|record| record.message == "LIVE_RECOVERY_NOTICE_ONCE")
                .count(),
            1
        );
        let projected_provider_run_id = crate::provider::projected_leased_provider_run_id(
            &binding.leased_agent_id,
            &provider_run_id,
        );
        assert!(runtime
            .owned
            .provider_run_projection
            .get(&projected_provider_run_id)
            .is_some());
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("home session should remain available"),
                agent.id(),
            )
            .is_some());

        runtime
            .owned
            .agent_store
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    execution_lease_id: "lease-other".to_string(),
                    leased_agent_id: "leased-agent-other".to_string(),
                    ..binding.clone()
                },
            )
            .expect("test should replace the current lease binding");
        assert_eq!(
            runtime
                .recover_remote_prompt_run_binding_from_projection(
                    session.id(),
                    agent.id(),
                    &binding,
                    &provider_run_id,
                    &completion_event,
                )
                .expect("other lease projection should be rejected safely"),
            RemotePromptRunBindingRecovery::Rejected,
        );
        runtime
            .owned
            .agent_store
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    active_worker_provider_run_id: Some(provider_run_id.clone()),
                    ..binding.clone()
                },
            )
            .expect("test should restore the recovered worker binding");

        runtime
            .project_remote_runtime_projection_event(completion_event)
            .await
            .expect("worker completion should settle the home prompt");
        assert!(runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id())
            .is_empty());
        assert!(runtime
            .owned
            .terminal_stream
            .drain_notice_records(session.id(), attachment.id())
            .is_empty());
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("home session should remain available"),
                agent.id(),
            )
            .is_none());
    }

    #[tokio::test]
    async fn settlement_projection_order_remote_completion_publishes_bound_idle_snapshot_after_completion_event(
    ) {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-settlement-projection-order-remote",
                "worktree-settlement-projection-order-remote",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-settlement-projection-order-remote",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let prompt = crate::session::PromptQueueItem::new(
            "prompt-settlement-projection-order-remote",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let prompt_id = match app
            .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
            .expect("remote prompt should start")
        {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("remote prompt should start")
            }
        };
        let (binding, provider_run_id, output_event, completion_event) =
            actual_worker_output_then_completion(session.id(), agent.id(), &prompt_id);
        let home_binding = crate::agent::RemoteAgentBinding {
            active_worker_provider_run_id: Some(provider_run_id.clone()),
            ..binding.clone()
        };
        app.agents
            .bind_remote_execution(agent.id(), home_binding)
            .expect("home agent should bind to the worker projection source");
        app.mark_active_prompt_delivery(
            session.id(),
            agent.id(),
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run_id.clone()),
            None,
        )
        .expect("delivery metadata should persist");

        // This is a real run binding for flow-control, not a manually inserted active turn. The
        // run is ended only after the pre-completion snapshot so the public watch cannot invoke a
        // second local PTY settlement while the worker completion is being projected.
        let request = crate::provider::LaunchProviderRequest::new(
            session.id(),
            "managed-dev-stub",
            "managed-dev-stub",
            "default",
            "test-model",
        )
        .with_agent_id(agent.id());
        let mut home_run = crate::provider::RuntimeProviderRun::new(
            &provider_run_id,
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::External,
                process_label: "remote-worker-settlement-projection-order".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        home_run.mark_running();
        app.providers_mut().insert_run_for_test(home_run.clone());
        app.sessions_mut()
            .set_active_provider_run(session.id(), Some(provider_run_id.clone()))
            .expect("home run should be selected for the pre-completion snapshot");
        app.update_provider_run_projection(home_run.clone());
        crate::transport::flow_control::note_prompt_started(&mut app, &provider_run_id);

        let initial_snapshot =
            crate::runtime::projection::SessionSnapshotProjection::from_daemon_app(
                &mut app,
                session.id(),
                0,
            )
            .expect("public pre-completion snapshot should be available");
        let initial_activity = initial_snapshot
            .agent_activity
            .get(agent.id())
            .expect("agent activity should be present before remote output");
        assert_eq!(
            initial_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Working
        );
        assert!(initial_activity.busy);
        assert_eq!(initial_activity.active_prompt_count, 1);
        assert!(initial_activity.active_turn.is_some());

        let mut ended_home_run = home_run;
        ended_home_run.mark_ended();
        app.providers_mut()
            .insert_run_for_test(ended_home_run.clone());
        app.update_provider_run_projection(ended_home_run);

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .project_remote_runtime_projection_event(output_event)
            .await
            .expect("worker output projection should reach the home session");
        let output_records = runtime
            .owned
            .terminal_stream
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(
            output_records
                .iter()
                .filter(|record| record.bytes == b"LIVE_RECOVERY_OUTPUT_ONCE")
                .count(),
            1,
            "the remote callback must deliver the worker output before completion"
        );
        let output_snapshot = {
            let mut app = runtime.app.lock().await;
            crate::runtime::projection::SessionSnapshotProjection::from_daemon_app(
                &mut app,
                session.id(),
                0,
            )
            .expect("public output snapshot should be available")
        };
        let output_activity = output_snapshot
            .agent_activity
            .get(agent.id())
            .expect("agent activity should remain present after output");
        assert_eq!(
            output_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Working,
            "a completion projection must not publish idle before its completion callback"
        );
        assert!(output_activity.busy);
        assert_eq!(output_activity.active_prompt_count, 1);
        assert!(output_activity.active_turn.is_some());
        let projection_before_completion = runtime.owned.session_projection.change_sequence();

        runtime
            .project_remote_runtime_projection_event(completion_event)
            .await
            .expect("worker completion projection should settle the home prompt");
        assert!(
            runtime.owned.session_projection.change_sequence() > projection_before_completion,
            "remote completion must invalidate the public session projection"
        );

        // This is the same public watch boundary used by the transport loop: completion records
        // are drained before the activity snapshot is projected. Requiring both in this result
        // catches an idle snapshot emitted before the completion event, not only eventual idle.
        let watch_result = {
            let mut app = runtime.app.lock().await;
            crate::runtime_transport::watch_subscription_state(
                &mut app,
                session.id(),
                attachment.id(),
                true,
                Some(output_snapshot.clone()),
                0,
            )
        };
        let crate::runtime_transport::WatchResult::Ok {
            records,
            completions,
            workflow_run_updates,
            snapshot,
            ..
        } = watch_result
        else {
            panic!("remote settlement subscription should remain available");
        };
        assert!(records.is_empty(), "remote output was already observed");
        assert!(
            workflow_run_updates.is_empty(),
            "the non-workflow settlement should not select a workflow-run event instead of the public activity/snapshot path"
        );
        assert_eq!(
            completions.len(),
            1,
            "the worker completion callback must expose exactly one public completion"
        );
        assert_eq!(completions[0].provider_run_id, provider_run_id);
        assert_eq!(
            completions[0].agent_id.as_deref(),
            Some(agent.id()),
            "the completion event must retain the remote run's home agent binding"
        );
        let settled_snapshot = snapshot
            .as_ref()
            .clone()
            .expect("remote settlement should carry a changed public snapshot");
        let settled_activity = settled_snapshot
            .agent_activity
            .get(agent.id())
            .expect("settled public snapshot should include the remote agent");
        assert_eq!(
            settled_activity.status,
            crate::runtime::projection::AgentRuntimeStatus::Idle
        );
        assert!(!settled_activity.busy);
        assert_eq!(settled_activity.active_prompt_count, 0);
        assert!(settled_activity.active_turn.is_none());
        // The public subscription emits a narrow activity delta when the session permits one.
        // Otherwise its actual transport loop falls back to the snapshot payload above. Check
        // the same selection boundary without manufacturing a fallback event in this test.
        match crate::transport::kernel_protocol::agent_activity_changed_event(
            &settled_snapshot,
            Some(&output_snapshot),
        ) {
            Some(crate::transport::kernel_protocol::KernelEvent::AgentActivityChanged {
                agent_activity,
                ..
            }) => {
                let activity = agent_activity
                    .get(agent.id())
                    .expect("idle event should include the settled remote agent");
                assert_eq!(
                    activity.status,
                    crate::runtime::projection::AgentRuntimeStatus::Idle
                );
                assert!(!activity.busy);
                assert_eq!(activity.active_prompt_count, 0);
                assert!(activity.active_turn.is_none());
            }
            Some(event) => panic!("unexpected public remote settlement event: {event:?}"),
            None => {
                assert!(
                    crate::transport::kernel_protocol::provider_run_changed_event(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no activity delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::session_metadata_changed_event(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no metadata delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::runtime_interactions_changed_event(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_none(),
                    "the public transport must use the full snapshot when no interaction delta applies"
                );
                assert!(
                    crate::transport::kernel_protocol::workflow_run_updated_events(
                        &settled_snapshot,
                        Some(&output_snapshot),
                    )
                    .is_empty()
                        && !crate::transport::kernel_protocol::workflow_run_only_changed(
                            &settled_snapshot,
                            Some(&output_snapshot),
                        ),
                    "the public transport must use the full snapshot when no narrow projection applies"
                );
                let fallback_activity = settled_snapshot
                    .agent_activity
                    .get(agent.id())
                    .expect("full snapshot fallback should include the settled remote agent");
                assert_eq!(
                    fallback_activity.status,
                    crate::runtime::projection::AgentRuntimeStatus::Idle
                );
                assert!(!fallback_activity.busy);
                assert_eq!(fallback_activity.active_prompt_count, 0);
                assert!(fallback_activity.active_turn.is_none());
            }
        }
    }

    #[test]
    fn live_run_binding_recovery_never_refreshes_the_lease() {
        let missing_lease = DaemonError::ExecutionLeaseNotFound {
            lease_id: "lease-2".to_string(),
        };

        assert!(!remote_prompt_projection_error_should_refresh_binding(
            true,
            &missing_lease,
        ));
        assert!(remote_prompt_projection_error_should_refresh_binding(
            false,
            &missing_lease,
        ));
    }

    #[tokio::test]
    async fn remote_prompt_dispatch_success_refreshes_session_projection() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-dispatch-projection",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");

        let projection_store = app.session_state_projection_store();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let prompt = crate::session::PromptQueueItem::new(
            "pending:remote-dispatch-projection",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should submit")
            .expect("remote prompt should be handled");
        let dispatch = submission
            .remote_dispatch
            .expect("started remote prompt should dispatch");

        runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("provider-run-worker-1".to_string()))
            .await
            .expect("remote prompt dispatch should settle");

        let projected = projection_store
            .get(session.id())
            .expect("session projection should remain available");
        let projected_agent = projected
            .agents()
            .iter()
            .find(|candidate| candidate.id() == agent.id())
            .expect("remote agent should remain projected");
        assert_eq!(
            projected_agent
                .remote_execution()
                .and_then(|remote| remote.active_worker_provider_run_id.as_deref()),
            Some("provider-run-worker-1"),
            "dispatch settlement must refresh the warm session projection"
        );
    }

    #[tokio::test]
    async fn remote_prompt_dispatch_failure_projects_agent_error() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .expect("session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-remote-dispatch-error",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("agent should bind to remote execution");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let prompt = crate::session::PromptQueueItem::new(
            "pending:remote-dispatch-error",
            attachment.id(),
            agent.id(),
            "remote prompt",
            crate::session::PromptStatus::Queued,
        );
        let submission = runtime
            .owned
            .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("remote prompt should submit")
            .expect("remote prompt should be handled");
        let dispatch = submission
            .remote_dispatch
            .expect("started remote prompt should dispatch");

        let result = runtime
            .finish_remote_prompt_dispatch(
                dispatch,
                Err(crate::error::DaemonError::LocalTransport {
                    operation: "submit remote prompt",
                    message: "provider rejected the prompt".to_string(),
                }),
            )
            .await;

        assert!(result.is_err(), "dispatch failure must be preserved");
        let failed_agent = runtime
            .owned
            .agent_store
            .get_agent(agent.id())
            .expect("failed agent should remain available");
        assert_eq!(failed_agent.state(), crate::agent::AgentState::Error);
        assert!(!failed_agent.is_processing());
        {
            let mut sessions = runtime.owned.session_store.write();
            runtime
                .owned
                .agent_store
                .focus_agent(session.id(), agent.id(), &mut sessions)
                .expect("failed agent should remain focusable");
        }
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(agent.id())
                .expect("focused failed agent should remain available")
                .state(),
            crate::agent::AgentState::Error,
            "focusing or restoring a failed pane must not erase its error badge",
        );
        let snapshot = runtime
            .owned
            .session_snapshot(session.id())
            .expect("failed session should remain projectable");
        assert_eq!(
            runtime
                .agent_activity_for_session(&snapshot)
                .get(agent.id())
                .expect("failed agent activity should remain projected")
                .status,
            crate::runtime::projection::AgentRuntimeStatus::Error,
        );
        assert!(runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(
                &runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("session should remain available"),
                agent.id(),
            )
            .is_none());
    }

    #[tokio::test]
    async fn dispatch_claim_releases_to_successor_after_predecessor_ack_once() {
        const WORKER_ID: &str = "worker-claim-restart";
        const MACHINE_ID: &str = "machine-claim-restart";
        const LEASED_AGENT_ID: &str = "leased-agent-claim-restart";

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("temporary relay listener should bind");
        let relay_url = format!("ws://{}", listener.local_addr().unwrap());
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_url = Some(relay_url);
        config.relay_token = Some("claim-restart-test-token".to_string());
        config.relay_request_timeout_ms = 2_000;
        let worker_config = crate::config::DaemonConfig::for_tests();
        let worker_private_key = worker_config.relay_private_key.clone();
        let worker_public_key = worker_config.relay_public_key.clone();

        let mut app = DaemonApp::bootstrap(config).expect("home app should bootstrap");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "claim-restart-workspace",
                "claim-restart-worktree",
            ))
            .expect("home session should be created");
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "claim-restart-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .expect("home attachment should be created");
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: WORKER_ID.to_string(),
                    worker_machine_id: MACHINE_ID.to_string(),
                    execution_lease_id: "lease-claim-restart".to_string(),
                    leased_agent_id: LEASED_AGENT_ID.to_string(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .expect("home agent should bind to the fake worker");

        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        let make_submission = |prompt_id: &str, prompt: &str| {
            runtime
                .owned
                .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                    session_id: session.id().to_string(),
                    prompt: crate::session::PromptQueueItem::new(
                        prompt_id,
                        attachment.id(),
                        agent.id(),
                        prompt,
                        crate::session::PromptStatus::Queued,
                    ),
                    force_queue: false,
                    refresh_projection: true,
                })
                .expect("remote prompt admission should succeed")
                .expect("remote prompt should be handled")
                .remote_dispatch
                .expect("started remote prompt should have a dispatch")
        };
        let predecessor = make_submission("home-prompt-claim-a", "predecessor prompt");
        let predecessor_id = predecessor.prompt_id.clone();
        runtime.spawn_remote_prompt_dispatch(predecessor);

        let (predecessor_seen_tx, predecessor_seen_rx) = tokio::sync::oneshot::channel();
        let (release_predecessor_tx, release_predecessor_rx) =
            tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (
                mut predecessor_peer,
                predecessor_request_id,
                predecessor_sender_key,
                predecessor_home_prompt_id,
                predecessor_identity,
            ) = accept_claim_test_prompt(
                &listener,
                WORKER_ID,
                MACHINE_ID,
                &worker_public_key,
                &worker_private_key,
            )
            .await;
            predecessor_seen_tx
                .send((
                    predecessor_home_prompt_id.clone(),
                    predecessor_identity.clone(),
                ))
                .expect("test should still await the predecessor request");
            release_predecessor_rx
                .await
                .expect("test should release the predecessor acknowledgement");
            acknowledge_claim_test_prompt(
                &mut predecessor_peer,
                predecessor_request_id,
                &predecessor_sender_key,
                WORKER_ID,
                &worker_private_key,
                &predecessor_home_prompt_id,
                LEASED_AGENT_ID,
                "predecessor prompt",
                "worker-run-claim-a",
            )
            .await;

            let (
                mut successor_peer,
                successor_request_id,
                successor_sender_key,
                successor_home_prompt_id,
                successor_identity,
            ) = accept_claim_test_prompt(
                &listener,
                WORKER_ID,
                MACHINE_ID,
                &worker_public_key,
                &worker_private_key,
            )
            .await;
            acknowledge_claim_test_prompt(
                &mut successor_peer,
                successor_request_id,
                &successor_sender_key,
                WORKER_ID,
                &worker_private_key,
                &successor_home_prompt_id,
                LEASED_AGENT_ID,
                "successor prompt",
                "worker-run-claim-b",
            )
            .await;
            assert_no_duplicate_claim_submission(
                &listener,
                WORKER_ID,
                MACHINE_ID,
                &worker_public_key,
                &worker_private_key,
            )
            .await;
            vec![
                (predecessor_home_prompt_id, predecessor_identity),
                (successor_home_prompt_id, successor_identity),
            ]
        });

        let (seen_predecessor_id, seen_predecessor_identity) =
            tokio::time::timeout(std::time::Duration::from_secs(2), predecessor_seen_rx)
                .await
                .expect("fake relay should receive predecessor before timeout")
                .expect("fake relay should report predecessor request");
        assert_eq!(seen_predecessor_id, predecessor_id);
        assert_eq!(
            seen_predecessor_identity,
            format!("{LEASED_AGENT_ID}\npredecessor prompt")
        );

        let cancelled = runtime
            .owned
            .cancel_active_prompt_only(session.id(), agent.id())
            .expect("predecessor should be locally cancelled");
        assert_eq!(cancelled.id(), predecessor_id);
        let successor = make_submission("home-prompt-claim-b", "successor prompt");
        let successor_id = successor.prompt_id.clone();
        assert_ne!(successor_id, predecessor_id);
        // This duplicate start only bumps the in-flight claim generation. The task
        // holding A's ACK must observe it and dispatch the now-current B exactly once.
        runtime.spawn_remote_prompt_dispatch(successor);
        release_predecessor_tx
            .send(())
            .expect("fake relay should still hold predecessor ACK");

        let requests = tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .expect("temporary relay requests should complete before timeout")
            .expect("temporary relay fixture should join");
        assert_eq!(
            requests,
            vec![
                (
                    predecessor_id,
                    format!("{LEASED_AGENT_ID}\npredecessor prompt")
                ),
                (
                    successor_id.clone(),
                    format!("{LEASED_AGENT_ID}\nsuccessor prompt")
                ),
            ],
            "predecessor and exact current successor should each be submitted once"
        );

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let session_state = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .expect("home session should remain available");
                if runtime
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session_state, agent.id())
                    .is_some_and(|prompt| {
                        prompt.id() == successor_id
                            && prompt.durable_delivery_phase()
                                == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                    })
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("successor ACK should become the durable active prompt");
    }

    async fn superseded_remote_dispatch_fixture(
        rebind: bool,
    ) -> (KernelRuntimeState, crate::app::KernelRemotePromptDispatch) {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-1",
                "worktree-1",
            ))
            .unwrap();
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "client-late-remote-dispatch",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let binding = crate::agent::RemoteAgentBinding {
            worker_kernel_id: "worker-a".to_string(),
            worker_machine_id: "machine-a".to_string(),
            execution_lease_id: "lease-a".to_string(),
            leased_agent_id: "leased-agent-a".to_string(),
            active_worker_provider_run_id: None,
            relay_url: None,
            relay_token: None,
            relay_peer_protocol_version: Some(
                crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
            ),
        };
        app.agents
            .bind_remote_execution(agent.id(), binding.clone())
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;
        runtime
            .ensure_managed_activity_tracking("remote-dispatch-settlement-fixture")
            .unwrap();
        let submit = |text: &str| {
            runtime
                .owned
                .submit_remote_prepared_prompt(&crate::app::KernelPreparedPromptSubmission {
                    session_id: session.id().to_string(),
                    prompt: crate::session::PromptQueueItem::new(
                        "pending-fixture",
                        attachment.id(),
                        agent.id(),
                        text,
                        crate::session::PromptStatus::Queued,
                    ),
                    force_queue: false,
                    refresh_projection: true,
                })
                .unwrap()
                .unwrap()
                .remote_dispatch
                .unwrap()
        };
        let stale_dispatch = submit("prompt A");
        runtime
            .owned
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                &stale_dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .unwrap();
        let cancelled = runtime
            .owned
            .cancel_active_prompt_only(session.id(), agent.id())
            .unwrap();
        assert_eq!(cancelled.id(), stale_dispatch.prompt_id);
        let mut successor_binding = binding;
        if rebind {
            successor_binding.worker_kernel_id = "worker-b".to_string();
            successor_binding.worker_machine_id = "machine-b".to_string();
            successor_binding.execution_lease_id = "lease-b".to_string();
            successor_binding.leased_agent_id = "leased-agent-b".to_string();
        }
        successor_binding.active_worker_provider_run_id = Some("worker-run-b".to_string());
        runtime
            .owned
            .agent_store
            .bind_remote_execution(agent.id(), successor_binding)
            .unwrap();
        let successor = submit("prompt B");
        assert_ne!(successor.prompt_id, stale_dispatch.prompt_id);
        runtime
            .owned
            .mark_active_prompt_delivery(
                session.id(),
                agent.id(),
                &successor.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some("worker-run-b".to_string()),
                None,
            )
            .unwrap();
        runtime.owned.session_snapshot(session.id()).unwrap();
        (runtime, stale_dispatch)
    }

    async fn assert_late_remote_dispatch_preserves_successor(
        result: Result<String, DaemonError>,
        rebind: bool,
    ) {
        let (runtime, stale_dispatch) = superseded_remote_dispatch_fixture(rebind).await;
        let session_id = stale_dispatch.session_id.clone();
        let agent_id = stale_dispatch.agent_id.clone();
        let before_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        let before_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&before_session, &agent_id)
            .unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();

        // Exercise the actual async dispatch settlement entry point after A has
        // lost ownership. Whether the stale result is reported or ignored is
        // secondary; it must not mutate B's state or transcript.
        let _ = runtime
            .finish_remote_prompt_dispatch(stale_dispatch, result)
            .await;

        let after_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        let after_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let after_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&after_session, &agent_id);
        let after_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        assert_eq!(
            after_prompt.as_ref(),
            Some(&before_prompt),
            "late A settlement must preserve B's active prompt"
        );
        assert_eq!(
            after_agent.remote_execution(),
            before_agent.remote_execution(),
            "late A settlement must preserve B's worker binding"
        );
        assert_eq!(after_agent.state(), before_agent.state());
        assert_eq!(after_agent.is_processing(), before_agent.is_processing());
        assert_eq!(
            after_history, before_history,
            "late A settlement must not append an error to B's transcript"
        );
        assert_eq!(
            after_prompt.unwrap().durable_delivery_provider_run_id(),
            Some("worker-run-b")
        );
    }

    #[tokio::test]
    async fn late_remote_dispatch_success_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(Ok("worker-run-a-late".to_string()), true)
            .await;
    }

    async fn assert_duplicate_remote_settlement_is_inert(result: Result<String, DaemonError>) {
        let (runtime, mut dispatch) = superseded_remote_dispatch_fixture(false).await;
        let session = runtime
            .owned
            .session_store
            .get_session(&dispatch.session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .unwrap();
        assert_eq!(
            before_prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        dispatch.prompt_id = before_prompt.id().to_string();
        dispatch.prompt = before_prompt.prompt().to_string();
        {
            let mut app = runtime.app.lock().await;
            crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::new(
                    &dispatch.session_id,
                    "duplicate-dispatch-observer",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ))
                .unwrap();
        }
        let before_agent = runtime
            .owned
            .agent_store
            .get_agent(&dispatch.agent_id)
            .unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&dispatch.session_id, Some(&dispatch.agent_id))
            .unwrap();
        let before_terminal_count = runtime.owned.terminal_stream.output_records().len();
        let before_terminal_sequence = runtime.owned.terminal_stream.change_sequence();
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let _ = runtime
            .finish_remote_prompt_dispatch(dispatch, result)
            .await;
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id),
            Some(before_prompt)
        );
        let after_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        assert_eq!(
            after_agent.remote_execution(),
            before_agent.remote_execution()
        );
        assert_eq!(after_agent.state(), before_agent.state());
        assert_eq!(after_agent.is_processing(), before_agent.is_processing());
        assert_eq!(
            runtime
                .owned
                .operational_history_store
                .load_session_history_entries(&session_id, Some(&agent_id))
                .unwrap(),
            before_history
        );
        assert_eq!(
            runtime.owned.terminal_stream.output_records().len(),
            before_terminal_count
        );
        assert_eq!(
            runtime.owned.terminal_stream.change_sequence(),
            before_terminal_sequence,
            "duplicate settlement must not echo or notify terminal output"
        );
    }

    #[tokio::test]
    async fn duplicate_remote_dispatch_success_after_delivered_is_inert() {
        assert_duplicate_remote_settlement_is_inert(Ok("worker-run-b".to_string())).await;
    }

    async fn pending_remote_settlement_fixture(
    ) -> (KernelRuntimeState, crate::app::KernelRemotePromptDispatch) {
        let (runtime, mut dispatch) = superseded_remote_dispatch_fixture(false).await;
        let session = runtime
            .owned
            .session_store
            .get_session(&dispatch.session_id)
            .unwrap();
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &dispatch.agent_id)
            .unwrap();
        dispatch.prompt_id = prompt.id().to_string();
        dispatch.prompt = prompt.prompt().to_string();
        runtime
            .owned
            .mark_active_prompt_delivery(
                &dispatch.session_id,
                &dispatch.agent_id,
                &dispatch.prompt_id,
                crate::session::DurablePromptDeliveryPhase::Dispatching,
                None,
                None,
            )
            .unwrap();
        (runtime, dispatch)
    }

    #[tokio::test]
    async fn remote_dispatch_durable_append_failure_rolls_back_then_manual_settlement_succeeds() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        let before_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&before_session, &agent_id)
            .unwrap();
        let before_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        let before_terminal = runtime.owned.terminal_stream.change_sequence();
        let before_activity = runtime.managed_activity_change_sequence();
        let event_count = || {
            runtime
                .owned
                .durable_state_store
                .load_events_by_kind("session.prompt_state.updated")
                .unwrap()
                .len()
        };
        let before_events = event_count();
        let connection =
            rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_remote_settlement_append BEFORE INSERT ON durable_state_events
             WHEN NEW.kind = 'session.prompt_state.updated'
             BEGIN SELECT RAISE(FAIL, 'injected remote settlement append failure'); END;",
            )
            .unwrap();
        let result = runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("worker-run-after-retry".to_string()))
            .await;
        connection
            .execute_batch("DROP TRIGGER fail_remote_settlement_append;")
            .unwrap();
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("injected remote settlement append failure"));
        let after_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&after_session, &agent_id),
            Some(before_prompt.clone())
        );
        assert_eq!(
            after_session.active_prompt_for_agent(&agent_id),
            Some(&before_prompt)
        );
        let after_agent = runtime.owned.agent_store.get_agent(&agent_id).unwrap();
        assert_eq!(
            after_agent.remote_execution(),
            before_agent.remote_execution()
        );
        assert_eq!(after_agent.state(), before_agent.state());
        assert_eq!(after_agent.is_processing(), before_agent.is_processing());
        assert_eq!(
            runtime
                .owned
                .operational_history_store
                .load_session_history_entries(&session_id, Some(&agent_id))
                .unwrap(),
            before_history
        );
        assert_eq!(
            runtime.owned.terminal_stream.change_sequence(),
            before_terminal
        );
        assert_eq!(runtime.managed_activity_change_sequence(), before_activity);
        assert_eq!(event_count(), before_events);

        let retry = runtime
            .remote_prompt_recovery_dispatch(&after_agent)
            .unwrap()
            .unwrap();
        assert_eq!(retry.prompt_id, prompt_id);
        runtime
            .finish_remote_prompt_dispatch(retry, Ok("worker-run-after-retry".to_string()))
            .await
            .unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let delivered = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap();
        assert_eq!(delivered.id(), prompt_id);
        assert_eq!(
            delivered.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            delivered.durable_delivery_provider_run_id(),
            Some("worker-run-after-retry")
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .unwrap()
                .remote_execution()
                .unwrap()
                .active_worker_provider_run_id
                .as_deref(),
            Some("worker-run-after-retry")
        );
        assert_eq!(
            event_count(),
            before_events + 1,
            "retry must publish exactly one prompt-state settlement"
        );
    }

    #[tokio::test]
    async fn acknowledged_remote_dispatch_retries_only_durable_settlement() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        let before_events = runtime
            .owned
            .durable_state_store
            .load_events_by_kind("session.prompt_state.updated")
            .unwrap()
            .len();
        let connection =
            rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_first_remote_ack BEFORE INSERT ON durable_state_events
                 WHEN NEW.kind = 'session.prompt_state.updated'
                 BEGIN SELECT RAISE(FAIL, 'injected transient acknowledgement failure'); END;",
            )
            .unwrap();
        let injected_failures = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let retry_count = std::sync::Arc::clone(&injected_failures);
        let durable_path = runtime.owned.durable_state_store.path().to_path_buf();
        runtime
            .finish_remote_prompt_dispatch_with_retry_hook(
                dispatch,
                Ok("worker-run-accepted".to_string()),
                move || {
                    retry_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    rusqlite::Connection::open(&durable_path)
                        .unwrap()
                        .execute_batch("DROP TRIGGER fail_first_remote_ack;")
                        .unwrap();
                },
            )
            .await
            .unwrap();
        assert_eq!(
            injected_failures.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the first ACK append alone must fail"
        );
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let delivered = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap();
        assert_eq!(delivered.id(), prompt_id);
        assert_eq!(
            delivered.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            delivered.durable_delivery_provider_run_id(),
            Some("worker-run-accepted")
        );
        assert_eq!(
            runtime
                .owned
                .durable_state_store
                .load_events_by_kind("session.prompt_state.updated")
                .unwrap()
                .len(),
            before_events + 1,
            "retry must persist exactly one acknowledgement for the already accepted run"
        );
    }

    #[tokio::test]
    async fn remote_dispatch_success_preserves_cancellation_in_flight() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        runtime
            .owned
            .begin_remote_prompt_cancellation(
                &session_id,
                &agent_id,
                &dispatch.source_attachment_id,
            )
            .unwrap();
        runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("worker-run-cancelling".to_string()))
            .await
            .unwrap();
        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
            .unwrap();
        assert_eq!(prompt.id(), prompt_id);
        assert_eq!(prompt.status(), crate::session::PromptStatus::Cancelling);
        assert_eq!(
            prompt.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(
            prompt.durable_delivery_provider_run_id(),
            Some("worker-run-cancelling")
        );
        assert_eq!(
            session.active_prompt_for_agent(&agent_id).unwrap().status(),
            crate::session::PromptStatus::Cancelling
        );
    }

    #[tokio::test]
    async fn current_remote_dispatch_binding_change_is_not_silently_settled() {
        let (runtime, dispatch) = pending_remote_settlement_fixture().await;
        let session_id = dispatch.session_id.clone();
        let agent_id = dispatch.agent_id.clone();
        let prompt_id = dispatch.prompt_id.clone();
        let before_session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        let before_prompt = runtime
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&before_session, &agent_id)
            .unwrap();
        assert_eq!(before_prompt.id(), prompt_id);
        assert!(before_prompt.delivery_pending());
        let mut replacement = runtime
            .owned
            .agent_store
            .get_agent(&agent_id)
            .unwrap()
            .remote_execution()
            .unwrap()
            .clone();
        replacement.worker_kernel_id = "replacement-worker".to_string();
        replacement.leased_agent_id = "replacement-leased-agent".to_string();
        replacement.execution_lease_id = "replacement-lease".to_string();
        replacement.active_worker_provider_run_id = None;
        runtime
            .owned
            .agent_store
            .bind_remote_execution(&agent_id, replacement.clone())
            .unwrap();
        let before_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        let source_attachment_id = dispatch.source_attachment_id.clone();
        let before_terminal = runtime
            .owned
            .terminal_stream
            .attachment_change_sequence(&session_id, &source_attachment_id);

        assert!(matches!(
            runtime.owned.settle_remote_dispatch_if_current(&dispatch, Some("old-worker-run")).unwrap(),
            super::super::remote_prompt_owned_state::RemotePromptDispatchSettlement::BindingChanged(ref prompt)
                if prompt.id() == prompt_id && prompt.delivery_pending()
        ));
        let result = runtime
            .finish_remote_prompt_dispatch(dispatch, Ok("old-worker-run".to_string()))
            .await;

        let session = runtime
            .owned
            .session_store
            .get_session(&session_id)
            .unwrap();
        assert_eq!(
            runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id),
            Some(before_prompt)
        );
        assert_eq!(
            runtime
                .owned
                .agent_store
                .get_agent(&agent_id)
                .unwrap()
                .remote_execution(),
            Some(&replacement)
        );
        let after_history = runtime
            .owned
            .operational_history_store
            .load_session_history_entries(&session_id, Some(&agent_id))
            .unwrap();
        assert_eq!(after_history.len(), before_history.len() + 1);
        let warning = after_history
            .iter()
            .find(|entry| {
                entry.merge_key.as_deref()
                    == Some(format!("remote-dispatch-uncertain:{prompt_id}").as_str())
            })
            .expect("delivery-uncertain diagnostic must be durable");
        assert!(warning.text.contains("remains pending"));
        assert!(warning.text.contains("not replayed or cancelled"));
        assert!(
            runtime
                .owned
                .terminal_stream
                .attachment_change_sequence(&session_id, &source_attachment_id)
                > before_terminal
        );
        assert!(
            runtime
                .remote_prompt_projection_drain_target(&session_id, &agent_id)
                .is_none(),
            "ordinary projection drain cannot reconcile pending delivery after a binding change"
        );
        assert!(result.is_ok());
        assert!(
            runtime
                .owned
                .remote_prompt_recoveries
                .lock()
                .unwrap()
                .is_empty(),
            "uncertain delivery must not replay the original prompt"
        );
    }

    #[tokio::test]
    async fn late_remote_dispatch_error_after_delivered_is_inert() {
        assert_duplicate_remote_settlement_is_inert(Err(DaemonError::LocalTransport {
            operation: "submit remote prompt",
            message: "late transport error after delivery".to_string(),
        }))
        .await;
    }

    #[tokio::test]
    async fn late_remote_dispatch_error_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(
            Err(DaemonError::LocalTransport {
                operation: "submit remote prompt",
                message: "late rejection of prompt A".to_string(),
            }),
            true,
        )
        .await;
    }

    #[tokio::test]
    async fn late_remote_dispatch_success_on_same_binding_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(Ok("worker-run-a-late".to_string()), false)
            .await;
    }

    #[tokio::test]
    async fn late_remote_dispatch_error_on_same_binding_preserves_successor() {
        assert_late_remote_dispatch_preserves_successor(
            Err(DaemonError::LocalTransport {
                operation: "submit remote prompt",
                message: "late rejection of prompt A on the same binding".to_string(),
            }),
            false,
        )
        .await;
    }

    #[test]
    fn remote_prompt_projection_drain_claims_coalesce_restart_before_release() {
        let claims = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let mut first =
            RemotePromptAgentClaim::try_acquire(Arc::clone(&claims), "session-1", "agent-1")
                .expect("first drain should claim the agent");

        let other_agent =
            RemotePromptAgentClaim::try_acquire(Arc::clone(&claims), "session-2", "agent-2")
                .expect("a different agent must remain independently dispatchable");

        assert!(
            RemotePromptAgentClaim::try_acquire(Arc::clone(&claims), "session-1", "agent-1",)
                .is_none(),
            "a duplicate drain must not start while the first owner is alive"
        );
        assert!(
            first.release_or_restart(),
            "the active owner must consume a start request that arrived before release"
        );
        assert!(
            !first.release_or_restart(),
            "the owner must release once no newer start request remains"
        );

        assert!(
            RemotePromptAgentClaim::try_acquire(claims, "session-1", "agent-1",).is_some(),
            "an atomically released claim must allow reconnect recovery to start a new drain"
        );
        drop(other_agent);
    }

    #[test]
    fn remote_prompt_projection_transport_retry_delay_is_bounded() {
        assert_eq!(
            remote_prompt_transport_retry_delay(1),
            std::time::Duration::from_millis(250)
        );
        assert_eq!(
            remote_prompt_transport_retry_delay(2),
            std::time::Duration::from_millis(500)
        );
        assert_eq!(
            remote_prompt_transport_retry_delay(4),
            std::time::Duration::from_millis(2_000)
        );
        assert_eq!(
            remote_prompt_transport_retry_delay(100),
            std::time::Duration::from_millis(2_000)
        );
    }
}
