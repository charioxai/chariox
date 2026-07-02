//! Remote-agent prompt ownership transitions.
//!
//! This module owns prompt queue state for agents leased to remote kernels. Local provider prompt
//! lifecycle remains in `prompt`.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn submit_remote_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
            let _ = self.ensure_attachment_in_session(&session_id, &attachment_id)?;
        }
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id,
                agent_id: target_agent_id,
            });
        }
        let Some(remote_execution) = target_agent.remote_execution().cloned() else {
            return Ok(None);
        };
        let session = self.session_store.get_session(&session_id)?;
        let queued_while_active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_some();
        let will_queue = prepared.force_queue || queued_while_active;
        let prompt = if will_queue {
            prepared.prompt.clone()
        } else {
            prepared
                .prompt
                .clone()
                .with_id(self.session_store.reserve_prompt_id())
        };
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prompt,
            prepared.force_queue,
        )?;
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.mirror_prompt_owner_agent_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let prompt_sent_at_ms = crate::session::unix_epoch_ms();
        let remote_dispatch =
            if let crate::session::PromptSubmissionOutcome::Started { prompt } = &outcome {
                self.append_user_prompt_history(
                    &session_id,
                    prompt.source_attachment_id(),
                    prompt.target_agent_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                    Some(prompt.id()),
                    prompt.workflow_run_id(),
                    prompt.workflow_node_run_id(),
                )?;
                self.agent_store
                    .note_prompt_sent_at(&outcome_agent_id, prompt_sent_at_ms)?;
                self.session_store.note_prompt_sent(
                    &session_id,
                    &outcome_agent_id,
                    prompt_sent_at_ms,
                )?;
                Some(crate::app::KernelRemotePromptDispatch {
                    session_id: session_id.clone(),
                    agent_id: target_agent_id,
                    prompt_id: prompt.id().to_string(),
                    worker_kernel_id: remote_execution.worker_kernel_id,
                    leased_agent_id: remote_execution.leased_agent_id,
                    relay_url: remote_execution.relay_url,
                    relay_token: remote_execution.relay_token,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workspace_live_sync_mode: Some(
                        crate::provider::provider_workspace_live_sync_mode_for_session(
                            target_agent.provider(),
                            &self.config_projection.snapshot(),
                            Some(&session),
                        ),
                    ),
                    workflow_context: None,
                })
            } else {
                None
            };
        let session = if prepared.refresh_projection {
            self.session_snapshot(&session_id)?
        } else {
            self.session_snapshot_without_projection_update(&session_id)?
        };
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch: None,
            remote_dispatch,
        }))
    }

    pub(super) fn complete_remote_prompt_owner(
        &self,
        session_id: &str,
        agent_id: &str,
        remote_provider_run_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let _ = self
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(agent_id, None)?;
        let session = self.session_store.get_session(session_id)?;
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.record_assistant_message_completion(
            session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completed.id()),
            crate::session::unix_epoch_ms(),
        );
        let started_next = if self
            .prompt_state_owner
            .active_prompt_for_agent(&self.session_store.get_session(session_id)?, agent_id)
            .is_none()
        {
            if let Some(expected_next) = next_queued_prompt {
                let session = self.session_store.get_session(session_id)?;
                let active = self
                    .prompt_state_owner
                    .activate_next_queued_prompt_with_prompt_id(
                        &session,
                        agent_id,
                        Some(expected_next.id()),
                        self.session_store.reserve_prompt_id(),
                    )?;
                if let Some(active_prompt) = active.as_ref() {
                    let prompt_sent_at_ms = crate::session::unix_epoch_ms();
                    self.append_user_prompt_history(
                        session_id,
                        active_prompt.source_attachment_id(),
                        active_prompt.target_agent_id(),
                        active_prompt.prompt(),
                        active_prompt.attachments(),
                        Some(active_prompt.id()),
                        active_prompt.workflow_run_id(),
                        active_prompt.workflow_node_run_id(),
                    )?;
                    self.agent_store
                        .note_prompt_sent_at(agent_id, prompt_sent_at_ms)?;
                    self.session_store
                        .note_prompt_sent(session_id, agent_id, prompt_sent_at_ms)?;
                }
                let (active_prompt, queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                self.mirror_prompt_owner_agent_state(
                    session_id,
                    agent_id,
                    active_prompt,
                    queued_prompts,
                )?;
                active
            } else {
                None
            }
        } else {
            None
        };
        let _ = self.session_snapshot(session_id)?;
        Ok(crate::session::PromptCompletion {
            completed,
            started_next,
        })
    }

    pub(super) fn begin_remote_prompt_cancellation(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: target_agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            });
        }
        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, target_agent_id);
        self.mirror_prompt_owner_agent_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let worker_kernel_id = target_agent
            .remote_execution()
            .map(|remote| remote.worker_kernel_id.clone())
            .unwrap_or_else(|| "remote".to_string());
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                prompt.id(),
                worker_kernel_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: None,
        })
    }
}
