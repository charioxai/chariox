//! Worker-confirmed profile changes for remote agents.

use super::*;

const REMOTE_PROFILE_QUEUE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(20);

impl KernelRuntimeState {
    pub(super) async fn confirm_remote_agent_profile(
        &self,
        agent_id: &str,
        update: &owned::OwnedRemoteAgentProfileUpdate,
    ) -> Result<(), DaemonError> {
        // The home reserves admission while the worker prepares its profile.
        // Show that wait on every home attachment without running or predicting
        // the worker's provider session command here.
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let notify = |message: String| {
            self.owned.record_notice_for_agent(
                agent.session_id(),
                None,
                Some(agent_id),
                self.owned
                    .attachment_store
                    .list_session_attachment_ids(agent.session_id()),
                message,
            )
        };
        notify("Preparing the worker's profile change…".into());
        let result = self.request_remote_agent_profile(agent_id, update).await;
        if let Err(error) = &result {
            // The home keeps the profile it had; every attachment learns why.
            notify(format!(
                "The worker's profile change failed; the agent keeps {} · {}. {error}",
                agent.provider(),
                agent.model().unwrap_or("default model"),
            ));
        }
        result
    }

    async fn request_remote_agent_profile(
        &self,
        agent_id: &str,
        update: &owned::OwnedRemoteAgentProfileUpdate,
    ) -> Result<(), DaemonError> {
        let mut config = self.config_snapshot().await;
        if let (Some(url), Some(token)) = (update.relay_url.clone(), update.relay_token.clone()) {
            config.apply_remote_relay_override(url, token);
        }
        self.ensure_remote_profile_account(agent_id, update, &config)
            .await?;
        let request = RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id: update.leased_agent_id.clone(),
            provider: update.provider.clone(),
            account_profile: update.account_profile.clone(),
            model: update.model.clone(),
            effort: update.effort.clone(),
        };
        let response = self
            .send_remote_profile_request(&config, &update.worker_kernel_id, request)
            .await?;
        match response {
            RelayPeerResponse::LeasedAgentProfileUpdated { leased_agent } => {
                update.validate_worker_acknowledgement(agent_id, &leased_agent)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "update remote leased agent profile",
                message: format!("unexpected remote profile response: {other:?}"),
            }),
        }
    }

    /// A worker restarted since the last prompt no longer owns the leased agent,
    /// and its relay rejects any request for it. Prompts recover through the
    /// binding refresh; a profile change must too, before it claims the agent,
    /// so it never half-applies against a lease the worker has lost.
    pub(super) async fn ensure_remote_agent_binding_current(
        &self,
        agent_id: &str,
    ) -> Result<(), DaemonError> {
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let Some(remote) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let mut config = self.config_snapshot().await;
        if let (Some(url), Some(token)) = (remote.relay_url.clone(), remote.relay_token.clone()) {
            config.apply_remote_relay_override(url, token);
        }
        // The receipt of a prompt that never existed: the relay authorizes the
        // leased agent first, and nothing is changed or replayed on the worker.
        let probe = RelayPeerRequest::GetLeasedPromptReceipt {
            leased_agent_id: remote.leased_agent_id.clone(),
            home_prompt_id: "binding-probe".to_string(),
        };
        match self
            .send_remote_profile_request(&config, &remote.worker_kernel_id, probe)
            .await
        {
            Err(error) if remote_binding_lost(&error) => {
                crate::logging::warn_with_fields(
                    "daemon.remote_agent_profile",
                    "remote lease lost before a profile change; refreshing binding",
                    serde_json::json!({ "agent_id": agent_id, "worker_kernel_id": remote.worker_kernel_id }),
                );
                self.refresh_remote_agent_binding_authorized(agent_id)
                    .await?;
                // The runs of the lost lease can never end on their own: retire
                // their home projection and session pointer so the agent does
                // not keep advertising a run the worker no longer has.
                self.owned.provider_run_projection.retire_leased_agent_runs(
                    agent.session_id(),
                    agent_id,
                    &remote.leased_agent_id,
                );
                let session = self.owned.session_store.get_session(agent.session_id())?;
                if let Some(run_id) = session.active_provider_run_id() {
                    if crate::provider::worker_provider_run_id_from_projected_leased_id(
                        &remote.leased_agent_id,
                        run_id,
                    )
                    .is_some()
                    {
                        self.owned.clear_active_provider_run_session_pointer(
                            agent.session_id(),
                            run_id,
                        )?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Profile requests ride out a worker's brief relay absence the way a
    /// prompt does; each is idempotent on the worker.
    pub(super) async fn send_remote_profile_request(
        &self,
        config: &crate::config::DaemonConfig,
        worker_kernel_id: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let started = tokio::time::Instant::now();
        let mut attempt = 0_u32;
        loop {
            match self
                .send_remote_profile_request_once(config, worker_kernel_id, request.clone())
                .await
            {
                Err(error)
                    if super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_retry_transport(&error)
                        && !super::remote_prompt_worker_submission_runtime::remote_prompt_transport_retry_window_expired(started.elapsed()) =>
                {
                    attempt = attempt.saturating_add(1);
                    if attempt == 1 {
                        crate::logging::warn_with_fields(
                            "daemon.remote_agent_profile",
                            "remote profile request transport unavailable; retrying",
                            serde_json::json!({ "worker_kernel_id": worker_kernel_id, "error": error.to_string() }),
                        );
                    }
                    tokio::time::sleep(
                        super::remote_prompt_worker_submission_runtime::remote_prompt_transport_retry_delay(attempt),
                    )
                    .await;
                }
                result => return result,
            }
        }
    }

    async fn send_remote_profile_request_once(
        &self,
        config: &crate::config::DaemonConfig,
        worker_kernel_id: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let response_timeout =
            request.off_lock_response_timeout(remote_profile_response_timeout(config, &request));
        let target = ClientTarget {
            daemon_id: Some(worker_kernel_id.to_string()),
            daemon_alias: None,
        };
        crate::logging::debug_with_fields(
            "daemon.remote_agent_profile",
            "remote profile request sending",
            serde_json::json!({
                "worker_kernel_id": worker_kernel_id,
                "request": match &request {
                    RelayPeerRequest::EnsureRemoteProviderAccount { .. } => "ensure_account",
                    RelayPeerRequest::UpdateLeasedAgentProfile { .. } => "update_profile",
                    RelayPeerRequest::GetLeasedPromptReceipt { .. } => "lease_probe",
                    _ => "other",
                },
            }),
        );
        // Only the reply may take long; reaching the relay never waits forever.
        let queued = tokio::time::timeout(REMOTE_PROFILE_QUEUE_DEADLINE, async {
            match self.connected_relay_state_for_config(config).await {
                Some(relay_state) => crate::transport::relay_client::enqueue_peer_request_via_connected_relay_with_timeout(
                    config,
                    &relay_state,
                    target.clone(),
                    request.clone(),
                    response_timeout,
                )
                .await
                .map(Some),
                None => Ok(None),
            }
        })
        .await
        .map_err(|_| DaemonError::LocalTransport {
            operation: "send relay peer request",
            message: format!(
                "relay is not connected: the request was not queued within {}s",
                REMOTE_PROFILE_QUEUE_DEADLINE.as_secs()
            ),
        })??;
        match queued {
            Some(waiter) => waiter.wait().await,
            None => tokio::time::timeout(
                response_timeout.saturating_add(REMOTE_PROFILE_QUEUE_DEADLINE),
                crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
                    config, target, request, response_timeout,
                ),
            )
            .await
            .map_err(|_| DaemonError::LocalTransport {
                operation: "read temporary relay peer response",
                message: "timed out waiting for relay peer response on a temporary connection".into(),
            })?,
        }
    }

    pub(super) async fn finish_remote_agent_profile_transition(
        &self,
        session_id: &str,
        agent_id: &str,
        claim: crate::runtime::prompt_state::AgentProfileTransitionClaim,
    ) -> Result<(), DaemonError> {
        if let Some(mut submission) = self
            .owned
            .finish_remote_profile_transition(session_id, agent_id, claim)?
        {
            self.finish_owned_prompt_submission(&mut submission, false)
                .await?;
            self.spawn_remote_prompt_projection_drain_if_needed(&submission);
            if let Some(dispatch) = submission.remote_dispatch.take() {
                self.spawn_remote_prompt_dispatch(dispatch);
            }
        }
        Ok(())
    }
}

fn remote_binding_lost(error: &DaemonError) -> bool {
    super::remote_prompt_worker_submission_runtime::remote_prompt_error_should_refresh_binding(
        error,
    ) || matches!(
        error,
        DaemonError::RelayTransport {
            code,
            message,
            retryable: false,
            ..
        } if code == "unauthorized"
            && message == "authenticated home kernel does not own the leased resource"
    )
}

// A worker may spend the full native session-command deadline preparing a
// Claude profile. Account sync and other profile requests keep their normal
// response deadline. This changes no relay or client request shape.
fn remote_profile_response_timeout(
    config: &crate::config::DaemonConfig,
    request: &RelayPeerRequest,
) -> std::time::Duration {
    let normal = std::time::Duration::from_millis(config.relay_request_timeout_ms);
    match request {
        RelayPeerRequest::UpdateLeasedAgentProfile { provider, .. } => {
            super::leased_profile_transition_runtime::worker_profile_response_timeout(
                normal, provider,
            )
        }
        _ => normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_restarted_worker_that_lost_the_leased_agent_marks_the_binding_stale() {
        let lost = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "unauthorized".to_string(),
            message: "authenticated home kernel does not own the leased resource".to_string(),
            retryable: false,
        };
        assert!(remote_binding_lost(&lost));
        let other = DaemonError::RelayTransport {
            operation: "read relay peer response",
            code: "unauthorized".to_string(),
            message: "execution lease claims do not match the authenticated home kernel"
                .to_string(),
            retryable: false,
        };
        assert!(!remote_binding_lost(&other));
        assert!(!remote_binding_lost(&DaemonError::LocalTransport {
            operation: "update remote leased agent profile",
            message: "relay unavailable".to_string(),
        }));
    }

    #[test]
    fn remote_claude_profile_deadline_includes_the_worker_compaction_budget() {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_request_timeout_ms = 60_000;
        let mut request = RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id: "worker".into(),
            provider: "claude".into(),
            account_profile: "work".into(),
            model: Some("haiku".into()),
            effort: None,
        };
        assert_eq!(
            remote_profile_response_timeout(&config, &request),
            std::time::Duration::from_secs(360)
        );
        assert_eq!(
            super::super::leased_profile_transition_runtime::worker_profile_response_timeout(
                crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
                "claude"
            ),
            std::time::Duration::from_secs(540)
        );
        if let RelayPeerRequest::UpdateLeasedAgentProfile { provider, .. } = &mut request {
            *provider = "codex".into();
        }
        assert_eq!(
            remote_profile_response_timeout(&config, &request),
            std::time::Duration::from_secs(60)
        );
    }
}
