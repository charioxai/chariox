//! MP-08 / MP-10: Exact home-turn admission over the shared provider paths.

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn steer_active_prompt(
        &self,
        request: crate::local::SteerActivePromptRequest,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let owned = &self.owned;
        owned.require_publication_activation()?;
        let attachment =
            owned.ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
        let agent = owned.agent_store.get_agent(&request.target_agent_id)?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id,
                agent_id: request.target_agent_id,
            });
        }
        let session = owned.session_store.get_session(&request.session_id)?;
        let active = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &request.target_agent_id);
        if active.as_ref().is_none_or(|active| {
            active.id() != request.expected_active_prompt_id
                || active.status() != crate::session::PromptStatus::Running
                || active.is_external()
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "steer active prompt",
                message: "active prompt changed before steering".into(),
            });
        }
        let prompt = {
            let _admission = owned.begin_managed_activity_admission()?;
            crate::session::PromptQueueItem::new(
                owned.session_store.reserve_prompt_id(),
                &request.attachment_id,
                &request.target_agent_id,
                &request.prompt,
                crate::session::PromptStatus::Running,
            )
            .with_attachments(request.attachments)
            .with_source_attribution(attachment.client_id(), attachment.owner_user_id())
        };
        if agent.remote_execution().is_some() {
            self.steer_remote_agent_message(
                &request.session_id,
                &prompt,
                Some(&request.expected_active_prompt_id),
            )
            .await?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer active prompt",
                message: "active prompt changed before steering".into(),
            })?;
        } else {
            let dispatch = self
                .prepare_local_active_prompt_steer_dispatch(&request.session_id, &prompt)?
                .filter(|d| {
                    d.target_active_prompt_id.as_deref()
                        == Some(request.expected_active_prompt_id.as_str())
                })
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "steer active prompt",
                    message: "active prompt changed before steering".into(),
                })?;
            let _permit = self
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            // The shared dispatch checks the same active prompt again after
            // liveness pumping and retains the provider's managed identities.
            if !self
                .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                .await?
            {
                return Err(DaemonError::LocalTransport {
                    operation: "steer active prompt",
                    message: "active prompt changed before steering delivery".into(),
                });
            }
            owned.append_steering_prompt_history(
                &request.session_id,
                &dispatch.provider_run_id,
                &request.expected_active_prompt_id,
                &request.attachment_id,
                &request.target_agent_id,
                prompt.id(),
                prompt.prompt(),
                prompt.attachments(),
            )?;
            owned.echo_steering_prompt_to_other_attachments(
                &request.session_id,
                &dispatch.provider_run_id,
                &request.target_agent_id,
                prompt.id(),
                &request.attachment_id,
                &request.attachment_id,
                prompt.prompt(),
                prompt.attachments(),
                prompt.prompt_origin(),
            );
        }
        Ok(prompt)
    }

    pub(in crate::runtime::state) fn prepare_local_active_prompt_steer_dispatch(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<crate::app::KernelPromptDispatch>, DaemonError> {
        let agent_id = prompt.target_agent_id();
        if self
            .owned
            .agent_store
            .get_agent(agent_id)?
            .remote_execution()
            .is_some()
        {
            return Ok(None);
        }
        let session = self.owned.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Ok(None);
        };
        if active_prompt.status() != crate::session::PromptStatus::Running {
            return Err(DaemonError::LocalTransport {
                operation: "steer agent message",
                message: "target agent is stopping; message was not delivered".to_string(),
            });
        }
        if active_prompt.is_external() {
            return Err(DaemonError::LocalTransport {
                operation: "steer agent message",
                message: "agent messages cannot steer an externally started provider turn"
                    .to_string(),
            });
        }
        let provider_run = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run.id().to_string(),
                state: provider_run.state(),
                operation: "steer agent message",
            });
        }
        Ok(Some(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run.id().to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: prompt.id().to_string(),
            target_active_prompt_id: Some(active_prompt.id().to_string()),
            source_attachment_id: prompt.source_attachment_id().to_string(),
            prompt: prompt.prompt().to_string(),
            hidden_system_context: prompt.hidden_system_context().to_string(),
            attachments: prompt.attachments().to_vec(),
            prompt_origin: prompt.prompt_origin(),
            external_provider: prompt.external_provider().map(str::to_string),
            external_provider_session_id: prompt.external_provider_session_id().map(str::to_string),
            external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
            steering: true,
        }))
    }
}
