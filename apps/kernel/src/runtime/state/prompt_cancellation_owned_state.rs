//! Local prompt cancellation state transitions.

use super::owned::OwnedPromptCancellation;
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn cancel_active_prompt_only(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let cancelled = self
            .prompt_state_owner
            .cancel_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        Ok(cancelled)
    }

    pub(super) fn finalize_local_prompt_cancellation_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<OwnedPromptCancellation, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .finalize_active_prompt_cancellation(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        let provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        let released_claim = provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let started_next = if self
            .prompt_state_owner
            .active_prompt_for_agent(&self.session_store.get_session(session_id)?, agent_id)
            .is_none()
        {
            let next_prompt = self
                .prompt_state_owner
                .peek_next_queued_prompt(&self.session_store.get_session(session_id)?, agent_id);
            if let (Some(provider_run_id), Some(next_prompt)) =
                (provider_run_id.as_deref(), next_prompt.as_ref())
            {
                let provider_run =
                    self.ensure_provider_run_in_session(session_id, provider_run_id)?;
                if provider_run.state() == crate::provider::ProviderRunState::Running {
                    let started_next = self
                        .prompt_state_owner
                        .activate_next_queued_prompt_with_prompt_id(
                            &self.session_store.get_session(session_id)?,
                            agent_id,
                            Some(next_prompt.id()),
                            self.session_store.reserve_prompt_id(),
                        )?;
                    if let Some(started_next) = started_next.as_ref() {
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
                        self.session_store.note_prompt_sent(
                            session_id,
                            agent_id,
                            prompt_sent_at_ms,
                        )?;
                        self.capture_git_turn_snapshot_for_started_prompt(
                            &session,
                            agent_id,
                            &provider_run,
                            started_next,
                            Some(prompt_sent_at_ms),
                        );
                    }
                    started_next
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&self.session_store.get_session(session_id)?, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }
        let dispatch = if let (Some(provider_run_id), Some(started_next)) =
            (provider_run_id.as_deref(), started_next.as_ref())
        {
            let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
            if self
                .provider_store
                .run_uses_structured_prompt_io(&provider_run)
            {
                let prompt_with_handoff = self.prompt_with_pending_context_handoff(
                    session_id,
                    agent_id,
                    started_next.source_attachment_id(),
                    &provider_run,
                    started_next.prompt(),
                );
                let granted_skill_context =
                    self.granted_skill_hidden_context(session_id, agent_id, &prompt_with_handoff)?;
                let hidden_system_context = join_hidden_context(
                    started_next.hidden_system_context(),
                    &granted_skill_context,
                );
                let mode = if self.agent_store.get_agent(agent_id)?.is_metaagent() {
                    crate::prompt_assembly::PromptAssemblyMode::MetaagentProviderTurn
                } else {
                    crate::prompt_assembly::PromptAssemblyMode::NormalProviderTurn
                };
                self.provider_store.enqueue_structured_prompt_submit(
                    session_id.to_string(),
                    provider_run_id.to_string(),
                    agent_id.to_string(),
                    &provider_run,
                    &prompt_with_handoff,
                    &hidden_system_context,
                    started_next.attachments(),
                    mode,
                    false,
                )?;
                self.consume_pending_context_handoff(session_id, agent_id, &provider_run);
                self.note_prompt_started(provider_run_id);
                None
            } else {
                Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: agent_id.to_string(),
                    prompt_id: started_next.id().to_string(),
                    source_attachment_id: started_next.source_attachment_id().to_string(),
                    prompt: started_next.prompt().to_string(),
                    hidden_system_context: started_next.hidden_system_context().to_string(),
                    attachments: started_next.attachments().to_vec(),
                    steering: false,
                })
            }
        } else {
            None
        };
        let _ = self.session_snapshot(session_id)?;
        Ok(OwnedPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next,
            },
            released_claim,
            dispatch,
        })
    }

    pub(super) fn cancel_local_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<Option<crate::app::KernelPromptCancellation>, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: target_agent_id.to_string(),
            });
        }
        if target_agent.remote_execution().is_some() {
            return Ok(None);
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
            return Ok(Some(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            }));
        }

        let provider_run = self
            .provider_run_projection
            .get_for_agent(session_id, target_agent_id)
            .or_else(|| {
                self.provider_store
                    .get_run_for_agent(session_id, target_agent_id)
            })
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run.id())?;

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
        self.note_prompt_settlement_requested(provider_run.id());
        let recipients = self.other_attachment_ids(session_id, attachment_id);
        self.record_notice(
            session_id,
            Some(provider_run.id()),
            recipients,
            format!(
                "Attachment `{}` requested cancellation of active prompt `{}` on provider run `{}`.",
                attachment_id,
                prompt.id(),
                provider_run.id()
            ),
        );
        if provider_run.adapter_key() == "claude" {
            let cancellation = self.finalize_local_prompt_cancellation_with_queued_advance(
                session_id,
                target_agent_id,
                Some(provider_run.id()),
            )?;
            let session = self.session_snapshot(session_id)?;
            return Ok(Some(crate::app::KernelPromptCancellation {
                cancellation: cancellation.cancellation,
                session,
                dispatch: Some(crate::app::KernelPromptAbortDispatch {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run.id().to_string(),
                    source_attachment_id: attachment_id.to_string(),
                }),
            }));
        }
        let session = self.session_snapshot(session_id)?;

        Ok(Some(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: Some(crate::app::KernelPromptAbortDispatch {
                session_id: session_id.to_string(),
                provider_run_id: provider_run.id().to_string(),
                source_attachment_id: attachment_id.to_string(),
            }),
        }))
    }
}

fn join_hidden_context(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_string(),
        ("", second) => second.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}
