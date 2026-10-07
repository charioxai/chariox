//! Remote worker projection drains and run-binding recovery.

use super::remote_prompt_claim_runtime::RemotePromptProjectionDrainClaim;
use super::remote_prompt_worker_submission_runtime::{
    remote_prompt_error_should_refresh_binding, remote_prompt_error_should_retry_transport,
    remote_prompt_transport_retry_delay,
};
use super::*;
use crate::transport::relay_peer::RelayPeerEvent;

pub(super) const REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RemotePromptRunBindingRecovery {
    Recovered,
    Rejected,
}

impl KernelRuntimeState {
    pub(super) async fn drain_completed_remote_prompt_receipt_projection(
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
        let relay_binding = binding.clone();
        let relay_config = self
            .with_app_side_effect(move |app| app.relay_config_for_remote_execution(&relay_binding))
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
                crate::transport::relay_client::send_peer_request_via_connected_relay_authenticated(
                    &relay_config,
                    &relay_state,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_authenticated(
                    &relay_config,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
        }
        .map_err(|error| format!("completed receipt projection drain failed: {error}"))?;
        let (response, peer) = response;
        let authority = crate::runtime::relay_peer_authority::RemoteProjectionAuthority {
            peer,
            expected_binding: Some(binding),
            expected_prompt_id: Some(dispatch.prompt_id.clone()),
        };
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
        self.project_remote_runtime_projection_event(authority, event)
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

    pub(super) fn spawn_remote_prompt_projection_drain(
        &self,
        session_id: String,
        agent_id: String,
    ) {
        let Some(mut claim) = RemotePromptProjectionDrainClaim::try_acquire(
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

    pub(super) fn remote_prompt_projection_drain_target(
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

    pub(super) fn recover_remote_prompt_run_binding_from_projection(
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
        // Exclusive session ownership excludes settlement/release lanes while agent
        // and prompt ownership are held together through run-binding recovery.
        let sessions = self.owned.session_store.write();
        let session = sessions.get_session(session_id)?;
        let mut agents = self.owned.agent_store.write();
        let agent = agents.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        let Some(current_binding) = agent.remote_execution() else {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        };
        if !same_remote_prompt_worker_binding(current_binding, expected_binding) {
            return Ok(RemotePromptRunBindingRecovery::Rejected);
        }
        let outcome = self.owned.prompt_state_owner.with_active_prompt(
            &session,
            agent_id,
            |active_prompt| {
                let Some(active_prompt) = active_prompt else {
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
                match current_binding.active_worker_provider_run_id.as_deref() {
                    Some(current) if current == provider_run_id => {
                        return Ok(RemotePromptRunBindingRecovery::Recovered)
                    }
                    Some(_) => return Ok(RemotePromptRunBindingRecovery::Rejected),
                    None => {}
                }
                agents.set_remote_execution_active_worker_provider_run_id(
                    agent_id,
                    Some(provider_run_id.to_string()),
                )?;
                Ok(RemotePromptRunBindingRecovery::Recovered)
            },
        )?;
        drop(agents);
        drop(sessions);
        if outcome == RemotePromptRunBindingRecovery::Recovered {
            let _ = self.owned.session_snapshot(session_id)?;
        }
        Ok(outcome)
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
        let expected_prompt_id = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&self.owned.session_store.get_session(session_id)?, agent_id)
            .map(|prompt| prompt.id().to_string());
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
                crate::transport::relay_client::send_peer_request_via_connected_relay_authenticated(
                    &relay_config,
                    &relay_state,
                    target,
                    request,
                    REMOTE_PROMPT_PROJECTION_RESPONSE_TIMEOUT,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection_authenticated(
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
        let (response, peer) = response;
        let mut authority = crate::runtime::relay_peer_authority::RemoteProjectionAuthority {
            peer,
            expected_binding: Some(remote_execution.clone()),
            expected_prompt_id,
        };
        match response {
            RelayPeerResponse::LeasedRuntimeProjectionDrained { event } => {
                if let Some(event) = event {
                    if !projection_reply_matches(session_id, agent_id, &provider_run_id, &event) {
                        return Err(DaemonError::LocalTransport {
                            operation: "drain remote prompt projection",
                            message: "worker projection returned unrelated resource identities"
                                .into(),
                        });
                    }
                    if recover_missing_run_binding {
                        match self
                            .with_app_side_effect(|_| {
                                authority
                                    .peer
                                    .authorize_worker(&remote_execution.worker_kernel_id)?;
                                self.recover_remote_prompt_run_binding_from_projection(
                                    session_id,
                                    agent_id,
                                    &remote_execution,
                                    &provider_run_id,
                                    &event,
                                )
                            })
                            .await?
                        {
                            RemotePromptRunBindingRecovery::Recovered => {
                                if let Some(binding) = authority.expected_binding.as_mut() {
                                    binding.active_worker_provider_run_id =
                                        Some(provider_run_id.clone());
                                }
                            }
                            RemotePromptRunBindingRecovery::Rejected => return Ok(false),
                        }
                    }
                    self.project_remote_runtime_projection_event(authority, event)
                        .await?;
                }
                Ok(true)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "drain remote prompt projection",
                message: format!("unexpected remote projection drain response: {other:?}"),
            }),
        }
    }

    pub(super) async fn project_remote_runtime_projection_event(
        &self,
        authority: crate::runtime::relay_peer_authority::RemoteProjectionAuthority,
        event: crate::transport::relay_peer::RelayPeerEvent,
    ) -> Result<(), DaemonError> {
        self.project_relay_remote_runtime_projection(authority, event)
            .await
    }
}

pub(super) fn same_remote_prompt_worker_binding(
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

pub(super) fn completed_receipt_projection_matches(
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

pub(super) fn remote_prompt_projection_error_should_refresh_binding(
    recovering_missing_run_binding: bool,
    error: &DaemonError,
) -> bool {
    !recovering_missing_run_binding && remote_prompt_error_should_refresh_binding(error)
}

fn projection_reply_matches(
    session_id: &str,
    agent_id: &str,
    expected_run_id: &str,
    event: &RelayPeerEvent,
) -> bool {
    let RelayPeerEvent::LeasedRuntimeProjection {
        home_session_id,
        home_agent_id,
        provider_run_id,
        provider_run,
        ..
    } = event;
    home_session_id == session_id
        && home_agent_id == agent_id
        && provider_run_id == expected_run_id
        && provider_run
            .as_ref()
            .is_none_or(|run| run.id() == expected_run_id)
}

#[cfg(test)]
mod mp11_drain_tests {
    use super::*;
    #[test]
    fn mp11_drain_reply_binds_requested_resource_identities() {
        let event = RelayPeerEvent::LeasedRuntimeProjection {
            account_copy_observations: Vec::new(),
            home_session_id: "home".into(),
            home_agent_id: "agent".into(),
            provider_run_id: "run".into(),
            provider_run: None,
            prompts: vec![],
            output_chunks: vec![],
            notices: vec![],
            completions: vec![],
        };
        assert!(projection_reply_matches("home", "agent", "run", &event));
        assert!(!projection_reply_matches("foreign", "agent", "run", &event));
        assert!(!projection_reply_matches(
            "home",
            "other-agent",
            "run",
            &event
        ));
        assert!(!projection_reply_matches(
            "home",
            "agent",
            "other-run",
            &event
        ));
    }
}
