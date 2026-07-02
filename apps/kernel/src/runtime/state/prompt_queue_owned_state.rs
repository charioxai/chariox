//! Prompt queue mirror and advancement mutations.
//!
//! This module owns synchronizing prompt-owner state back into sessions and advancing queued
//! prompts onto an existing provider run.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn mirror_prompt_owner_agent_state(
        &self,
        session_id: &str,
        agent_id: &str,
        active_prompt: Option<crate::session::PromptQueueItem>,
        queued_prompts: std::collections::VecDeque<crate::session::PromptQueueItem>,
    ) -> Result<(), DaemonError> {
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        self.provider_process_projection.invalidate();
        let _ = self.session_snapshot(session_id)?;
        Ok(())
    }

    pub(super) fn mirror_prompt_owner_session_state(
        &self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let mut agent_ids = self
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        let session = self.session_store.get_session(session_id)?;
        agent_ids.extend(session.prompt_states().keys().cloned());
        agent_ids.sort();
        agent_ids.dedup();
        for agent_id in agent_ids {
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, &agent_id);
            self.mirror_prompt_owner_agent_state(
                session_id,
                &agent_id,
                active_prompt,
                queued_prompts,
            )?;
        }
        Ok(())
    }

    pub(super) fn remove_queued_prompts_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<usize, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let removed = self
            .prompt_state_owner
            .remove_queued_prompts_for_agent(&session, agent_id);
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(removed)
    }

    pub(super) fn activate_next_queued_prompt_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        expected_prompt_id: Option<&str>,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                expected_prompt_id,
                self.session_store.reserve_prompt_id(),
            )?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(prompt)
    }

    pub(super) fn advance_next_queued_prompt_dispatch(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) -> Result<Option<crate::app::KernelPromptDispatch>, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let Some(next_prompt) = self
            .prompt_state_owner
            .peek_next_queued_prompt(&session, agent_id)
        else {
            return Ok(None);
        };
        if self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some_and(|prompt| prompt.is_external())
        {
            return Ok(None);
        }
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "advance queued prompt",
            });
        }
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                Some(next_prompt.id()),
                self.session_store.reserve_prompt_id(),
            )?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "advance queued prompt",
                message: format!(
                    "expected queued prompt `{}` but no queued prompt was available",
                    next_prompt.id()
                ),
            })?;
        let prompt_sent_at_ms = crate::session::unix_epoch_ms();
        self.append_user_prompt_history(
            session_id,
            started_next.source_attachment_id(),
            started_next.target_agent_id(),
            started_next.prompt(),
            started_next.attachments(),
            Some(started_next.id()),
            started_next.workflow_run_id(),
            started_next.workflow_node_run_id(),
        )?;
        self.echo_promoted_queued_prompt_to_attachments(
            session_id,
            provider_run_id,
            started_next.id(),
            started_next.source_attachment_id(),
            started_next.prompt(),
            started_next.attachments(),
        );
        self.agent_store
            .note_prompt_sent_at(agent_id, prompt_sent_at_ms)?;
        self.session_store
            .note_prompt_sent(session_id, agent_id, prompt_sent_at_ms)?;
        self.capture_git_turn_snapshot_for_started_prompt(
            &session,
            agent_id,
            &provider_run,
            &started_next,
            Some(prompt_sent_at_ms),
        );
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        if let (Some(workflow_run_id), Some(workflow_node_run_id)) = (
            started_next.workflow_run_id(),
            started_next.workflow_node_run_id(),
        ) {
            let _ = self.session_store.write().mark_workflow_turn_dispatched(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            let _ = self.workflow_start_prompt(session_id, &started_next)?;
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelPromptDispatch {
            session_id: session_id.to_string(),
            provider_run_id: provider_run_id.to_string(),
            agent_id: agent_id.to_string(),
            prompt_id: started_next.id().to_string(),
            source_attachment_id: started_next.source_attachment_id().to_string(),
            prompt: started_next.prompt().to_string(),
            hidden_system_context: started_next.hidden_system_context().to_string(),
            attachments: started_next.attachments().to_vec(),
            steering: false,
        }))
    }

    pub(super) fn steer_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<Option<crate::app::KernelQueuedPromptSteer>, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let Some(active_prompt) = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
        else {
            return Err(DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            });
        };
        if active_prompt.is_external() {
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "queued prompts cannot be steered into externally started provider turns"
                    .to_string(),
            });
        }
        let provider_run = self
            .provider_run_projection
            .get_for_agent(session_id, agent_id)
            .or_else(|| self.provider_store.get_run_for_agent(session_id, agent_id))
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run.id())?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run.id().to_string(),
                state: provider_run.state(),
                operation: "steer queued prompt",
            });
        }
        let (_, queued_prompts) = self.prompt_state_owner.state_parts(&session, agent_id);
        if queued_prompts
            .iter()
            .find(|prompt| prompt.id() == prompt_id)
            .is_some_and(|prompt| prompt.workflow_run_id().is_some())
        {
            return Err(DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: "workflow queued prompts cannot be steered manually".to_string(),
            });
        }
        let prompt = self
            .prompt_state_owner
            .remove_queued_prompt(&session, agent_id, prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "steer queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.record_notice(
            session_id,
            Some(provider_run.id()),
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` steered queued prompt `{}` to agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(Some(crate::app::KernelQueuedPromptSteer {
            dispatch: crate::app::KernelPromptDispatch {
                session_id: session_id.to_string(),
                provider_run_id: provider_run.id().to_string(),
                agent_id: agent_id.to_string(),
                prompt_id: prompt.id().to_string(),
                source_attachment_id: prompt.source_attachment_id().to_string(),
                prompt: prompt.prompt().to_string(),
                hidden_system_context: prompt.hidden_system_context().to_string(),
                attachments: prompt.attachments().to_vec(),
                steering: true,
            },
            prompt,
            session,
        }))
    }

    pub(super) fn cancel_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
    ) -> Result<crate::app::KernelQueuedPromptCancellation, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let mut prompt = self
            .prompt_state_owner
            .remove_queued_prompt(&session, agent_id, prompt_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "cancel queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        prompt.set_status(crate::session::PromptStatus::Cancelled);
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` cancelled queued prompt `{}` for agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelQueuedPromptCancellation { prompt, session })
    }

    pub(super) fn update_queued_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        attachment_id: &str,
        prompt_id: &str,
        prompt_text: &str,
    ) -> Result<crate::app::KernelQueuedPromptUpdate, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .update_queued_prompt(&session, agent_id, prompt_id, prompt_text)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "update queued prompt",
                message: format!(
                    "queued prompt `{prompt_id}` was not found for agent `{agent_id}`"
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{}` updated queued prompt `{}` for agent `{}`.",
                attachment_id,
                prompt.id(),
                agent_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelQueuedPromptUpdate { prompt, session })
    }
}
