//! Provider prompt failure forwarding, local settlement, and substitute activation.

use super::*;

impl KernelRuntimeState {
    pub(super) async fn fail_owned_provider_prompt(
        &self,
        session_id: &str,
        provider_run_id: &str,
        message: &str,
    ) -> Result<(), DaemonError> {
        let owned = &self.owned;
        let provider_run = owned.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;

        if self
            .forward_leased_workflow_provider_failure(provider_run_id, message)
            .await?
        {
            return Ok(());
        }

        let session = owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &agent_id)
        else {
            return Ok(());
        };
        let _ = self.inject_metaagent_turn_failure_event(
            session_id,
            &agent_id,
            &active_prompt,
            Some(provider_run_id),
            message,
        );
        if active_prompt.workflow_run_id().is_some() {
            owned.workflow_fail_provider_prompt(
                session_id,
                &active_prompt,
                Some(provider_run_id),
                message,
            )?;
        }
        let completion = owned.complete_local_prompt_without_advance(
            session_id,
            &agent_id,
            Some(provider_run_id),
        )?;
        if completion
            .as_ref()
            .is_some_and(|completion| completion.released_claim)
            && active_prompt.workflow_run_id().is_none()
        {
            self.spawn_workflow_prompt_dispatches(owned.workflow_retry_blocked_claims());
        }
        if let Some(reason) = crate::provider::classify_provider_substitutable_failure_text(
            provider_run.adapter_key(),
            message,
        ) {
            self.activate_substitute_after_provider_failure(
                session_id,
                &agent_id,
                provider_run_id,
                &reason,
            )
            .await;
        }
        Ok(())
    }

    async fn forward_leased_workflow_provider_failure(
        &self,
        provider_run_id: &str,
        message: &str,
    ) -> Result<bool, DaemonError> {
        let leased_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workflow_turn_context_for_provider_run(provider_run_id)
            })
            .await;
        let Some(context) = leased_context else {
            return Ok(false);
        };
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        arroba_relay::protocol::ClientTarget {
                            daemon_id: Some(context.home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        crate::transport::relay_peer::RelayPeerRequest::ForwardWorkflowProviderFailure {
                            context,
                            message: message.to_string(),
                        },
                    ),
                )
            })
            .await?;
        if !matches!(
            response,
            crate::transport::relay_peer::RelayPeerResponse::WorkflowProviderFailureHandled
        ) {
            return Err(DaemonError::LocalTransport {
                operation: "forward workflow provider failure",
                message: format!("unexpected workflow provider failure response: {response:?}"),
            });
        }
        let _ = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .complete_leased_workflow_prompt_for_provider_run(provider_run_id)
            })
            .await?;
        Ok(true)
    }

    async fn activate_substitute_after_provider_failure(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        reason: &str,
    ) {
        if let Err(error) = self
            .activate_next_agent_substitute_after_failure(session_id, agent_id, reason)
            .await
        {
            crate::logging::warn_with_fields(
                "daemon.provider",
                "automatic substitute activation after provider failure failed",
                serde_json::json!({
                    "session_id": session_id,
                    "agent_id": agent_id,
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}
