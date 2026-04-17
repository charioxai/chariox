use super::owned::{OwnedPromptCancellation, OwnedPromptCompletion};
use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn cancel_active_prompt_only(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let cancelled = self
            .prompt_state_owner
            .cancel_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        Ok(cancelled)
    }

    pub(super) fn complete_local_prompt_without_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<Option<OwnedPromptCompletion>, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
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
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;

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

    pub(super) fn submit_local_prepared_prompt(
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
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(&session_id)?;
        let queued_while_active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_some();
        let provider_run_id = self
            .provider_store
            .get_run_for_agent(&session_id, &target_agent_id)
            .map(|run| run.id().to_string());
        if !queued_while_active && provider_run_id.is_none() {
            return Ok(None);
        }
        if !queued_while_active {
            if let Some(provider_run_id) = provider_run_id.as_deref() {
                let provider_run =
                    self.ensure_provider_run_in_session(&session_id, provider_run_id)?;
                if provider_run.state() == crate::provider::ProviderRunState::Parked {
                    let _ = self.resume_provider_run_for_session(&session_id, provider_run_id)?;
                }
            }
        }
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.provider_store.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Starting);

        self.append_user_prompt_history(
            &session_id,
            &attachment_id,
            &target_agent_id,
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;
        let force_queue = prepared.force_queue || provider_run_is_starting;
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prepared.prompt.clone(),
            force_queue,
        );
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;

        let mut dispatch = None;
        match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.clone(),
                        })?;
                self.echo_prompt_to_other_attachments(
                    &session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.acquire_provider_prompt_claim(
                    &session_id,
                    provider_run_id,
                    &target_agent_id,
                    Some(prompt.source_attachment_id()),
                ) {
                    let _ = self.cancel_active_prompt_only(&session_id, &target_agent_id);
                    let _ = self.clear_prompt_activity(provider_run_id);
                    return Err(error);
                }
                dispatch = Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: target_agent_id.clone(),
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                });
            }
            crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                let queue_depth = self
                    .prompt_state_owner
                    .queued_prompt_count_for_agent(&session, &target_agent_id);
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.echo_prompt_to_other_attachments(
                        &session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.record_notice(
                    &session_id,
                    provider_run_id.as_deref(),
                    self.other_attachment_ids(&session_id, &attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        target_agent_id,
                        session_id,
                        prompt.id(),
                        queue_depth
                    ),
                );
            }
        }
        let session = self.session_snapshot(&session_id)?;
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch,
            remote_dispatch: None,
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
        if target_agent.remote_execution().is_some() {
            return Ok(None);
        }
        let session = self.session_store.get_session(session_id)?;
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(
            next_queued_prompt.source_attachment_id(),
        ) {
            let _ = self.ensure_attachment_in_session(
                session_id,
                next_queued_prompt.source_attachment_id(),
            )?;
        }
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
        self.acquire_provider_prompt_claim(
            session_id,
            &provider_run_id,
            agent_id,
            Some(next_queued_prompt.source_attachment_id()),
        )?;

        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let started_next = self
            .prompt_state_owner
            .activate_next_queued_prompt(&session, agent_id, Some(next_queued_prompt.id()))?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "advance queued prompt",
                message: format!(
                    "expected queued prompt `{}` but no queued prompt was available",
                    next_queued_prompt.id()
                ),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
        if self
            .provider_store
            .run_uses_structured_prompt_io(&provider_run)
        {
            if let Err(error) = self.provider_store.enqueue_structured_prompt_submit(
                session_id.to_string(),
                provider_run_id.clone(),
                agent_id.to_string(),
                &provider_run,
                started_next.prompt(),
                started_next.attachments(),
            ) {
                let _ = self.cancel_active_prompt_only(session_id, agent_id);
                let _ = self.clear_prompt_activity(&provider_run_id);
                return Err(error);
            }
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
                source_attachment_id: started_next.source_attachment_id().to_string(),
                prompt: started_next.prompt().to_string(),
                attachments: started_next.attachments().to_vec(),
            }),
        }))
    }

    pub(super) fn finalize_local_prompt_cancellation_with_queued_advance(
        &self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<OwnedPromptCancellation, DaemonError> {
        let session = self.session_store.get_session(session_id)?;
        let prompt = self
            .prompt_state_owner
            .finalize_active_prompt_cancellation(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
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
                    self.acquire_provider_prompt_claim(
                        session_id,
                        provider_run_id,
                        agent_id,
                        Some(next_prompt.source_attachment_id()),
                    )?;
                    self.prompt_state_owner.activate_next_queued_prompt(
                        &self.session_store.get_session(session_id)?,
                        agent_id,
                        Some(next_prompt.id()),
                    )?
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
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
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
                self.provider_store.enqueue_structured_prompt_submit(
                    session_id.to_string(),
                    provider_run_id.to_string(),
                    agent_id.to_string(),
                    &provider_run,
                    started_next.prompt(),
                    started_next.attachments(),
                )?;
                self.note_prompt_started(provider_run_id);
                None
            } else {
                Some(crate::app::KernelPromptDispatch {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: agent_id.to_string(),
                    source_attachment_id: started_next.source_attachment_id().to_string(),
                    prompt: started_next.prompt().to_string(),
                    attachments: started_next.attachments().to_vec(),
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
        self.session_store.mirror_agent_prompt_state(
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
        let Some(remote_execution) = target_agent.remote_execution().cloned() else {
            return Ok(None);
        };
        self.append_user_prompt_history(
            &session_id,
            &attachment_id,
            &target_agent_id,
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;
        let session = self.session_store.get_session(&session_id)?;
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prepared.prompt.clone(),
            prepared.force_queue,
        );
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.session_store.mirror_agent_prompt_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let remote_dispatch =
            if let crate::session::PromptSubmissionOutcome::Started { prompt } = &outcome {
                Some(crate::app::KernelRemotePromptDispatch {
                    session_id: session_id.clone(),
                    agent_id: target_agent_id,
                    worker_kernel_id: remote_execution.worker_kernel_id,
                    leased_agent_id: remote_execution.leased_agent_id,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workflow_context: None,
                })
            } else {
                None
            };
        let session = self.session_snapshot(&session_id)?;
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
        let session = self.session_store.get_session(session_id)?;
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.session_store.mirror_agent_prompt_state(
            session_id,
            agent_id,
            active_prompt,
            queued_prompts,
        )?;
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
                let active = self.prompt_state_owner.activate_next_queued_prompt(
                    &session,
                    agent_id,
                    Some(expected_next.id()),
                )?;
                let (active_prompt, queued_prompts) =
                    self.prompt_state_owner.state_parts(&session, agent_id);
                self.session_store.mirror_agent_prompt_state(
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
        self.session_store.mirror_agent_prompt_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let worker_kernel_id = self
            .agent_store
            .get_agent(target_agent_id)?
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

    pub(super) fn other_attachment_ids(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<String> {
        self.attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|id| id != attachment_id)
            .collect()
    }

    pub(super) fn prompt_completion_recorded(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| state.completion_recorded)
            .unwrap_or(false)
    }

    pub(super) fn mark_prompt_completion_recorded(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.completion_recorded = true;
        }
    }

    pub(super) fn record_assistant_message_completion(
        &self,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
        message_id: &str,
        completed_at_ms: u64,
    ) {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal_stream.record_assistant_message_completion(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            recipient_attachment_ids,
            message_id,
            completed_at_ms,
        );
    }

    pub(super) fn reap_structured_prompt_jobs(&self) {
        self.provider_store
            .apply_finished_provider_run_selection_sync_jobs();
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_submit_jobs()
        {
            if let Err(error) = finished.result {
                let _ = self.cancel_active_prompt_only(&finished.session_id, &finished.agent_id);
                let _ = self.session_snapshot(&finished.session_id);
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt dispatch failed after acknowledgement: {error}"),
                );
            }
        }
        for finished in self
            .provider_store
            .drain_finished_structured_prompt_abort_jobs()
        {
            if let Err(error) = finished.result {
                let recipients = self
                    .attachment_store
                    .list_session_attachment_ids(&finished.session_id);
                self.record_notice(
                    &finished.session_id,
                    Some(&finished.provider_run_id),
                    recipients,
                    format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
                );
            }
        }
    }

    pub(super) fn fan_out_terminal_output(
        &self,
        session_id: &str,
        provider_run_id: &str,
        kind: crate::terminal::TerminalOutputKind,
        merge_key: Option<String>,
        recipient_attachment_ids: Vec<String>,
        bytes: &[u8],
    ) -> crate::terminal::TerminalOutputRecord {
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        let record = self.terminal_stream.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            kind.clone(),
            merge_key.clone(),
            recipient_attachment_ids,
            bytes,
        );
        if kind != crate::terminal::TerminalOutputKind::PromptEcho {
            self.append_history_entry(
                session_id,
                SessionHistoryEntry::provider_output(
                    session_id,
                    provider_run_id,
                    agent_id.as_deref(),
                    kind,
                    merge_key,
                    String::from_utf8_lossy(bytes).into_owned(),
                ),
            );
        }
        record
    }

    pub(super) fn append_history_entry(&self, session_id: &str, entry: SessionHistoryEntry) {
        let session = match self.session_store.get_session(session_id) {
            Ok(session) => session,
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.history",
                    "skipping provider-output history append because session lookup failed",
                    serde_json::json!({
                        "session_id": session_id,
                        "error": error.to_string(),
                    }),
                );
                return;
            }
        };
        if let Err(error) = self.history_store.append(&session, &entry) {
            crate::logging::warn_with_fields(
                "daemon.history",
                "failed to append provider-output session history",
                serde_json::json!({
                    "session_id": session_id,
                    "error": error.to_string(),
                }),
            );
        } else {
            self.history_projection.append(entry);
        }
    }

    pub(super) fn clear_prompt_activity(&self, provider_run_id: &str) -> bool {
        self.prompt_activity.write().remove(provider_run_id);
        self.prompt_workspace_claims.remove(provider_run_id)
    }

    pub(super) fn release_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> bool {
        let owner = format!("{workflow_run_id}:{workflow_node_run_id}");
        self.prompt_workspace_claims.remove_matching(|claim| {
            claim.session_id == session_id
                && claim.attachment_id.as_deref() == Some(owner.as_str())
                && claim.operation == "workflow_node_dispatch"
        }) > 0
    }

    pub(super) fn note_prompt_started(&self, provider_run_id: &str) {
        self.prompt_activity.write().insert(
            provider_run_id.to_string(),
            crate::app::ActivePromptState {
                last_output_at: None,
                saw_response_content: false,
                completion_recorded: false,
            },
        );
    }

    pub(super) fn note_prompt_output(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
        }
    }

    pub(super) fn note_prompt_response_content(&self, provider_run_id: &str) {
        if let Some(state) = self.prompt_activity.write().get_mut(provider_run_id) {
            state.last_output_at = Some(Instant::now());
            state.saw_response_content = true;
        }
    }

    pub(super) fn note_prompt_settlement_requested(&self, provider_run_id: &str) {
        self.prompt_activity
            .write()
            .entry(provider_run_id.to_string())
            .and_modify(|state| {
                state.last_output_at = Some(Instant::now());
                state.saw_response_content = true;
            })
            .or_insert(crate::app::ActivePromptState {
                last_output_at: Some(Instant::now()),
                saw_response_content: true,
                completion_recorded: false,
            });
    }

    pub(super) fn prompt_should_settle(&self, provider_run_id: &str) -> bool {
        self.prompt_activity
            .read()
            .get(provider_run_id)
            .map(|state| {
                (state.saw_response_content || state.completion_recorded)
                    && state
                        .last_output_at
                        .map(|last_output_at| last_output_at.elapsed() >= self.prompt_idle_timeout)
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub(super) fn acquire_provider_prompt_claim(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(provider_run_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_provider_prompt_claim(
            workspace_id,
            worktree_id,
            session_id,
            attachment_id.map(str::to_string),
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(super) fn acquire_workflow_node_workspace_claim(
        &self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
    ) -> Result<(), DaemonError> {
        if self.prompt_workspace_claims.contains(provider_run_id) {
            return Ok(());
        }
        let session = self.session_store.get_session(session_id)?;
        let workspace_id = session.workspace_id().to_string();
        let worktree_id = self
            .agent_store
            .get_agent(agent_id)
            .ok()
            .and_then(|agent| agent.worktree_id().map(str::to_string))
            .unwrap_or_else(|| session.worktree_id().to_string());
        let claim = self.workspace_coordinator.acquire_worktree_write_claim(
            workspace_id,
            worktree_id,
            session_id,
            Some(format!("{workflow_run_id}:{workflow_node_run_id}")),
            "workflow_node_dispatch",
        )?;
        self.prompt_workspace_claims
            .insert(provider_run_id.to_string(), claim);
        Ok(())
    }

    pub(super) fn append_user_prompt_history(
        &self,
        session_id: &str,
        source_attachment_id: &str,
        agent_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) -> Result<(), DaemonError> {
        let session = self.session_snapshot(session_id)?;
        let entry = crate::history::SessionHistoryEntry::user_prompt(
            session_id,
            source_attachment_id,
            agent_id,
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments),
        );
        self.history_store.append(&session, &entry)?;
        self.history_projection.append(entry);
        Ok(())
    }

    pub(super) fn echo_prompt_to_other_attachments(
        &self,
        session_id: &str,
        provider_run_id: &str,
        source_attachment_id: &str,
        prompt: &str,
        attachments: &[crate::session::PromptAttachment],
    ) {
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id)
            .into_iter()
            .filter(|attachment_id| attachment_id != source_attachment_id)
            .collect::<Vec<_>>();
        if recipient_attachment_ids.is_empty() {
            return;
        }
        let mut bytes =
            crate::prompt_transcript::render_prompt_transcript(prompt, attachments).into_bytes();
        if !bytes.ends_with(b"\n") {
            bytes.push(b'\n');
        }
        let agent_id = self
            .provider_store
            .get_run(provider_run_id)
            .ok()
            .and_then(|run| run.agent_instance_id().map(str::to_string));
        self.terminal_stream.fan_out_output(
            session_id,
            provider_run_id,
            agent_id.as_deref(),
            crate::terminal::TerminalOutputKind::PromptEcho,
            None,
            recipient_attachment_ids,
            &bytes,
        );
    }
}
