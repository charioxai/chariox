//! Local prompt completion and queued-prompt advancement.
//!
//! This module owns local prompt completion once provider output settlement arrives, plus the
//! queued prompt activation that may follow completion.

use super::owned::OwnedPromptCompletion;
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn complete_local_prompt_without_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        if agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        let _active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;

        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;

        let completion_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.provider_store
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        let completion_record_key = provider_run_id.unwrap_or(agent_id);
        if !self.prompt_completion_recorded(completion_record_key) {
            let provider_run_id = completion_provider_run_id
                .as_deref()
                .unwrap_or("provider-run-completed");
            let recipient_attachment_ids = self
                .attachment_store
                .list_session_attachment_ids(session_id);
            self.record_assistant_message_completion(
                session_id,
                provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            self.mark_prompt_completion_recorded(provider_run_id);
        }
        let released_claim = completion_provider_run_id
            .as_deref()
            .map(|provider_run_id| self.clear_prompt_activity(provider_run_id))
            .unwrap_or(false);
        let _ = self.session_snapshot(session_id)?;

        Ok(Some(OwnedPromptCompletion {
            completion: crate::session::PromptCompletion {
                completed,
                started_next: None,
            },
            released_claim,
            dispatch: None,
        }))
    }

    pub(super) fn complete_local_prompt_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: &crate::session::PromptQueueItem,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
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
        let provider_run_id = provider_run_id
            .map(str::to_string)
            .or_else(|| {
                self.provider_store
                    .get_run_for_agent(session_id, agent_id)
                    .map(|run| run.id().to_string())
            })
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;
        if provider_run.state() != crate::provider::ProviderRunState::Running {
            return Ok(None);
        }
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let Some(started_next) = self
            .prompt_state_owner
            .activate_next_queued_prompt_with_prompt_id(
                &session,
                agent_id,
                Some(next_queued_prompt.id()),
                self.session_store.reserve_prompt_id(),
            )?
        else {
            let (active_prompt, queued_prompts) =
                self.prompt_state_owner.state_parts(&session, agent_id);
            self.mirror_prompt_owner_agent_state(
                session_id,
                agent_id,
                active_prompt,
                queued_prompts,
            )?;
            let released_claim = self.clear_prompt_activity(&provider_run_id);
            let _ = self.session_snapshot(session_id)?;
            return Ok(Some(OwnedPromptCompletion {
                completion: crate::session::PromptCompletion {
                    completed,
                    started_next: None,
                },
                released_claim,
                dispatch: None,
            }));
        };
        let source_attachment_id = self.promoted_prompt_source_attachment_id(
            session_id,
            started_next.source_attachment_id(),
        )?;
        let prompt_sent_at_ms = crate::session::unix_epoch_ms();
        self.append_user_prompt_history(
            session_id,
            &source_attachment_id,
            started_next.target_agent_id(),
            started_next.prompt(),
            started_next.attachments(),
            Some(started_next.id()),
            started_next.workflow_run_id(),
            started_next.workflow_node_run_id(),
        )?;
        self.echo_promoted_queued_prompt_to_attachments(
            session_id,
            &provider_run_id,
            started_next.id(),
            &source_attachment_id,
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
        if self
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            let prompt_with_handoff = self.prompt_with_pending_context_handoff(
                session_id,
                agent_id,
                &source_attachment_id,
                &provider_run,
                started_next.prompt(),
            );
            let granted_skill_context =
                self.granted_skill_hidden_context(session_id, agent_id, &prompt_with_handoff)?;
            let hidden_system_context =
                join_hidden_context(started_next.hidden_system_context(), &granted_skill_context);
            let mode = if self.agent_store.get_agent(agent_id)?.is_metaagent() {
                crate::prompt_assembly::PromptAssemblyMode::MetaagentProviderTurn
            } else {
                crate::prompt_assembly::PromptAssemblyMode::NormalProviderTurn
            };
            if let Err(error) = self.provider_store.enqueue_structured_prompt_submit(
                session_id.to_string(),
                provider_run_id.clone(),
                agent_id.to_string(),
                &provider_run,
                &prompt_with_handoff,
                &hidden_system_context,
                started_next.attachments(),
                mode,
                false,
            ) {
                let _ = self.cancel_active_prompt_only(session_id, agent_id);
                let _ = self.clear_prompt_activity(&provider_run_id);
                return Err(error);
            }
            self.consume_pending_context_handoff(session_id, agent_id, &provider_run);
            self.note_prompt_started(&provider_run_id);
            let _ = self.session_snapshot(session_id)?;
            return Ok(Some(OwnedPromptCompletion {
                completion: crate::session::PromptCompletion {
                    completed,
                    started_next: Some(started_next),
                },
                released_claim: false,
                dispatch: None,
            }));
        }
        let _ = self.session_snapshot(session_id)?;
        Ok(Some(OwnedPromptCompletion {
            completion: crate::session::PromptCompletion {
                completed,
                started_next: Some(started_next.clone()),
            },
            released_claim: false,
            dispatch: Some(crate::app::KernelPromptDispatch {
                session_id: session_id.to_string(),
                provider_run_id,
                agent_id: agent_id.to_string(),
                prompt_id: started_next.id().to_string(),
                target_active_prompt_id: None,
                source_attachment_id,
                prompt: started_next.prompt().to_string(),
                hidden_system_context: started_next.hidden_system_context().to_string(),
                attachments: started_next.attachments().to_vec(),
                steering: false,
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
