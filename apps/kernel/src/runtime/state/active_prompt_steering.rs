//! Active-turn steering over the shared provider paths.

use super::*;

impl KernelRuntimeState {
    pub(crate) async fn steer_active_prompt(
        &self,
        request: &crate::local::SteerActivePromptRequest,
    ) -> Result<Option<(String, crate::session::PromptQueueItem)>, DaemonError> {
        self.authorize_current_external_command()?;
        let owned = &self.owned;
        owned.require_publication_activation()?;
        let attachment =
            owned.ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
        let agent = owned.agent_store.get_agent(&request.target_agent_id)?;
        if agent.session_id() != request.session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: request.session_id.clone(),
                agent_id: request.target_agent_id.clone(),
            });
        }
        let session = owned.session_store.get_session(&request.session_id)?;
        let unavailable = |message: &'static str| {
            if request.require_exact_turn {
                Err(DaemonError::LocalTransport {
                    operation: "steer active prompt",
                    message: message.into(),
                })
            } else {
                Ok(None)
            }
        };
        let active = owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &request.target_agent_id)
            .filter(|active| {
                active.status() == crate::session::PromptStatus::Running
                    && !active.is_external()
                    && active.prompt_origin() != crate::session::PromptOrigin::CompletionCheck
                    && (!request.require_exact_turn
                        || active.id() == request.expected_active_prompt_id)
            });
        let Some(active) = active else {
            return unavailable("active prompt changed before steering");
        };
        let active_prompt_id = active.id();
        let prompt = {
            let _admission = owned.begin_managed_activity_admission()?;
            crate::session::PromptQueueItem::new(
                owned.session_store.reserve_prompt_id(),
                &request.attachment_id,
                &request.target_agent_id,
                &request.prompt,
                crate::session::PromptStatus::Running,
            )
            .with_attachments(request.attachments.clone())
            .with_source_attribution(attachment.client_id(), attachment.owner_user_id())
        };
        if agent.remote_execution().is_some() {
            let delivered = self
                .steer_remote_agent_message(
                    &request.session_id,
                    &prompt,
                    Some(active_prompt_id),
                    !request.require_exact_turn,
                )
                .await;
            match delivered {
                Ok(Some(_)) => {}
                Ok(None) => return unavailable("active prompt changed before steering"),
                Err(DaemonError::RelayTransport { code, .. })
                    if !request.require_exact_turn
                        && matches!(
                            code.as_str(),
                            "no_active_prompt" | "prompt_steer_rejected"
                        ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        } else {
            let dispatch = self.prepare_local_active_prompt_steer_dispatch_for_turn(
                &request.session_id,
                &prompt,
                Some(active_prompt_id),
            )?;
            let Some(dispatch) = dispatch else {
                return unavailable("active prompt changed before steering");
            };
            let _permit = self
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            self.authorize_current_external_command()?;
            // Only a definitive refusal can fall back to the next prompt.
            match self
                .enqueue_prompt_dispatch_with_acceptance(&dispatch)
                .await
            {
                Ok(true) => {}
                Ok(false) => return unavailable("active prompt changed before steering delivery"),
                Err(DaemonError::ProviderPromptSteerRejected { .. })
                    if !request.require_exact_turn =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
            let sequence = owned.append_steering_prompt_history(
                &request.session_id,
                &dispatch.provider_run_id,
                active_prompt_id,
                &request.attachment_id,
                &request.target_agent_id,
                prompt.id(),
                prompt.prompt(),
                prompt.attachments(),
                prompt.prompt_origin(),
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
                Some(sequence),
            );
        }
        // The provider already holds the steer, so a retry would deliver it twice.
        owned
            .publish_outside_steer(&request.session_id, &request.target_agent_id, prompt.id())
            .map_err(|error| {
                tracing::warn!(%error, steer_id = prompt.id(), "steer delivered but its outside event was not recorded");
                DaemonError::SteerDeliveredEventUnrecorded {
                    steer_id: prompt.id().to_string(),
                }
            })?;
        Ok(Some((active_prompt_id.to_owned(), prompt)))
    }

    pub(in crate::runtime::state) fn prepare_local_active_prompt_steer_dispatch(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<crate::app::KernelPromptDispatch>, DaemonError> {
        self.prepare_local_active_prompt_steer_dispatch_for_turn(session_id, prompt, None)
    }

    fn prepare_local_active_prompt_steer_dispatch_for_turn(
        &self,
        session_id: &str,
        prompt: &crate::session::PromptQueueItem,
        expected_active_prompt_id: Option<&str>,
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
        if expected_active_prompt_id.is_some_and(|id| {
            active_prompt.id() != id
                || active_prompt.status() != crate::session::PromptStatus::Running
        }) {
            return Ok(None);
        }
        // Recheck at steer preparation: the active work turn can have ended
        // and entered assessment since inbox delivery first inspected it.
        if active_prompt.prompt_origin() == crate::session::PromptOrigin::CompletionCheck {
            return Ok(None);
        }
        if active_prompt.status() != crate::session::PromptStatus::Running {
            return Err(DaemonError::LocalTransport {
                operation: "steer agent message",
                message: "target agent is stopping; message was not delivered".to_string(),
            });
        }
        if self.owned.prompt_state_owner.prompt_is_sudo_bound(
            &session,
            agent_id,
            active_prompt.id(),
        ) {
            return Err(DaemonError::LocalTransport {
                operation: "steer agent message",
                message: "agent messages cannot steer a human-authorized sudo turn".into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn flexible_steer_falls_back_when_the_turn_ends_while_waiting_for_the_provider_lane() {
        let (_worktree, runtime, session_id, agent_id, attachment_id, active_prompt_id, run_id) =
            super::super::local_prompt_dispatch_runtime::tests::runtime_with_active_prompt().await;
        let request = crate::local::SteerActivePromptRequest {
            session_id,
            attachment_id,
            target_agent_id: agent_id,
            expected_active_prompt_id: active_prompt_id,
            require_exact_turn: false,
            prompt: "change direction".into(),
            attachments: vec![],
        };

        let lane = runtime.provider_runtime_lanes.acquire(&run_id).await;
        let mut steer = std::pin::pin!(runtime.steer_active_prompt(&request));
        assert!(futures_util::poll!(&mut steer).is_pending());

        let session = runtime
            .owned
            .session_store
            .get_session(&request.session_id)
            .unwrap();
        runtime
            .owned
            .prompt_state_owner
            .complete_active_prompt_only(&session, &request.target_agent_id)
            .expect("the selected turn was running");
        drop(lane);

        let result = steer.await;
        assert!(
            matches!(result, Ok(None)),
            "the caller can submit a next prompt: {result:?}"
        );
    }

    #[tokio::test]
    async fn active_prompt_steer_rejects_missing_grant_before_reading_session() {
        let app =
            crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let state =
            super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(app);
        let request = crate::local::SteerActivePromptRequest {
            session_id: "missing-session".into(),
            attachment_id: "missing-attachment".into(),
            target_agent_id: "missing-agent".into(),
            expected_active_prompt_id: "old-prompt".into(),
            require_exact_turn: false,
            prompt: "steer".into(),
            attachments: vec![],
        };
        let local_request = crate::local::LocalDaemonRequest::SteerActivePrompt(request.clone());
        let result = state
            .with_external_command_authority(Some(("revoked-grant", &local_request)))
            .steer_active_prompt(&request)
            .await;
        assert!(
            matches!(result, Err(DaemonError::LocalTransport { operation: "kernel access", message })
            if message == "grant revoked or expired")
        );
    }
}
