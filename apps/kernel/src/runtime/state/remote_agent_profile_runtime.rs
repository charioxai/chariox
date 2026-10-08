//! Worker-confirmed profile changes for remote agents.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn confirm_remote_agent_profile(
        &self,
        agent_id: &str,
        update: &owned::OwnedRemoteAgentProfileUpdate,
    ) -> Result<(), DaemonError> {
        let mut config = self.config_snapshot().await;
        if let (Some(url), Some(token)) = (update.relay_url.clone(), update.relay_token.clone()) {
            config.apply_remote_relay_override(url, token);
        }
        let receiving_account = self
            .ensure_remote_profile_account(agent_id, update, &config)
            .await?;
        let request = RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id: update.leased_agent_id.clone(),
            provider: update.provider.clone(),
            account_profile: receiving_account.clone(),
            model: update.model.clone(),
            effort: update.effort.clone(),
        };
        let response = self
            .send_remote_profile_request(&config, &update.worker_kernel_id, request)
            .await?;
        match response {
            RelayPeerResponse::LeasedAgentProfileUpdated { leased_agent } => {
                update.validate_worker_acknowledgement(agent_id, &receiving_account, &leased_agent)
            }
            other => Err(DaemonError::LocalTransport {
                operation: "update remote leased agent profile",
                message: format!("unexpected remote profile response: {other:?}"),
            }),
        }
    }

    pub(crate) async fn send_remote_profile_request(
        &self,
        config: &crate::config::DaemonConfig,
        worker_kernel_id: &str,
        request: RelayPeerRequest,
    ) -> Result<RelayPeerResponse, DaemonError> {
        let target = ClientTarget {
            daemon_id: Some(worker_kernel_id.to_string()),
            daemon_alias: None,
        };
        match self.connected_relay_state_for_config(config).await {
            Some(relay_state) => {
                crate::transport::relay_client::send_peer_request_via_connected_relay(
                    config,
                    &relay_state,
                    target,
                    request,
                )
                .await
            }
            None => {
                crate::transport::relay_client::send_peer_request_via_temporary_connection(
                    config, target, request,
                )
                .await
            }
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
            self.finish_owned_prompt_submission_workflow_start(&mut submission)
                .await?;
            self.spawn_remote_prompt_projection_drain_if_needed(&submission);
            if let Some(dispatch) = submission.remote_dispatch.take() {
                self.spawn_remote_prompt_dispatch(dispatch);
            }
        }
        Ok(())
    }
}
