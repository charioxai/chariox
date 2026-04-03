use std::time::Instant;

use crate::app::{ActivePromptState, DaemonApp};
use crate::error::DaemonError;
use crate::pty::PtyProcessState;
use crate::session::{PromptCancellation, PromptCompletion, PromptStatus, PromptSubmissionOutcome};

impl DaemonApp {
    pub fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        prompt: &str,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.sessions.get_session(session_id)?;

        let target_agent_id = session_before
            .focused_agent_id()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })?
            .to_string();
        let queued_while_active = session_before.active_prompt().is_some();
        let provider_run_id = if queued_while_active {
            self.providers
                .get_run_for_agent(session_id, &target_agent_id)
                .map(|run| run.id().to_string())
                .or_else(|| session_before.active_provider_run_id().map(str::to_string))
        } else {
            Some(self.ensure_active_provider_run_for_agent(session_id, &target_agent_id)?)
        };

        self.append_user_prompt_history(
            session_id,
            attachment_id,
            &target_agent_id,
            prompt,
            &attachments,
        );

        let (_session, outcome) = self.sessions.submit_prompt(
            session_id,
            attachment_id,
            &target_agent_id,
            prompt,
            attachments.clone(),
        )?;

        match &outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.to_string(),
                        })?;
                self.echo_prompt_to_other_attachments(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.dispatch_prompt_to_provider(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                ) {
                    let _ = self.sessions.cancel_active_prompt(session_id);
                    self.clear_prompt_activity(session_id);
                    return Err(error);
                }
                self.note_prompt_started(session_id);
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.echo_prompt_to_other_attachments(
                        session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.record_notice(
                    session_id,
                    provider_run_id.as_deref(),
                    self.other_attachment_ids(session_id, attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        session_id,
                        prompt.id(),
                        session_before.queued_prompts().len() + 1
                    ),
                );
            }
        }

        Ok(outcome)
    }

    pub fn complete_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCompletion, DaemonError> {
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        let (_session, completed) = self.sessions.complete_active_prompt_only(session_id)?;
        self.reconcile_workflow_prompt_completed(
            session_id,
            &completed,
            provider_run_id.as_deref(),
        )?;
        self.clear_prompt_activity(session_id);
        let started_next = if self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .is_none()
        {
            self.advance_next_queued_prompt(session_id)?
        } else {
            None
        };
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }

        Ok(PromptCompletion {
            completed,
            started_next,
        })
    }

    pub fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let active_prompt = self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .cloned()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == PromptStatus::Cancelling {
            return Ok(PromptCancellation {
                prompt: active_prompt,
                started_next: None,
            });
        }
        let provider_run_id = self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?
            .to_string();
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;

        if !self.providers.abort_structured_runtime(&provider_run_id)? {
            self.send_provider_input(session_id, &provider_run_id, attachment_id, b"\x03")?;
        }

        let (_session, prompt) = self.sessions.begin_cancelling_active_prompt(session_id)?;
        self.note_prompt_settlement_requested(session_id);
        self.record_notice(
            session_id,
            Some(&provider_run_id),
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
        );

        Ok(PromptCancellation {
            prompt,
            started_next: None,
        })
    }

    pub(crate) fn advance_next_queued_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        loop {
            let next_candidate = self.sessions.peek_next_queued_prompt(session_id)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let target_agent_id = peeked.target_agent_id().to_string();
            let is_workflow_prompt =
                Self::is_workflow_prompt_source_attachment_id(peeked.source_attachment_id());
            let provider_run_id = match self
                .ensure_active_provider_run_for_agent(session_id, &target_agent_id)
            {
                Ok(provider_run_id) => provider_run_id,
                Err(DaemonError::NoActiveProviderRun { .. }) if is_workflow_prompt => {
                    match self.ensure_workflow_provider_run_for_agent(session_id, &target_agent_id)
                    {
                        Ok(provider_run_id) => provider_run_id,
                        Err(error) => {
                            self.record_notice(
                                    session_id,
                                    None,
                                    self.attachments.list_session_attachment_ids(session_id),
                                    format!(
                                        "Deferred queued workflow prompt `{}` because Arroba could not launch the provider run for agent `{}`: {}",
                                        peeked.id(),
                                        target_agent_id,
                                        error
                                    ),
                                );
                            return Ok(None);
                        }
                    }
                }
                Err(error) => {
                    self.record_notice(
                            session_id,
                            None,
                            self.attachments.list_session_attachment_ids(session_id),
                            format!(
                                "Deferred queued prompt `{}` because Arroba could not activate the provider run for agent `{}`: {}",
                                peeked.id(),
                                target_agent_id,
                                error
                            ),
                        );
                    return Ok(None);
                }
            };

            let (_session, next_candidate) =
                self.sessions.activate_next_queued_prompt(session_id)?;
            let Some(next) = next_candidate else {
                continue;
            };

            if let Err(error) =
                self.ensure_attachment_in_session(session_id, next.source_attachment_id())
            {
                if is_workflow_prompt {
                    let active = self.sessions.activate_prompt(session_id, next)?.1;
                    if let Err(dispatch_error) = self.dispatch_prompt_to_provider(
                        session_id,
                        &provider_run_id,
                        active.source_attachment_id(),
                        active.prompt(),
                        active.attachments(),
                    ) {
                        let cancelled = self.sessions.cancel_active_prompt(session_id)?.1;
                        self.reconcile_workflow_prompt_cancelled(session_id, &cancelled)?;
                        self.clear_prompt_activity(session_id);
                        return Err(dispatch_error);
                    }
                    self.reconcile_workflow_prompt_started(session_id, &active)?;
                    self.note_prompt_started(session_id);
                    return Ok(Some(active));
                }
                self.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                        next.id(),
                        error
                    ),
                );
                continue;
            }

            if let Err(error) = self.dispatch_prompt_to_provider(
                session_id,
                &provider_run_id,
                next.source_attachment_id(),
                next.prompt(),
                next.attachments(),
            ) {
                self.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Skipped queued prompt `{}` after PTY delivery failure: {}",
                        next.id(),
                        error
                    ),
                );
                continue;
            }

            let active = self.sessions.activate_prompt(session_id, next)?.1;
            self.reconcile_workflow_prompt_started(session_id, &active)?;
            self.note_prompt_started(session_id);
            return Ok(Some(active));
        }
    }

    pub(crate) fn note_prompt_started(&mut self, session_id: &str) {
        self.prompt_activity.insert(
            session_id.to_string(),
            ActivePromptState {
                last_output_at: None,
            },
        );
    }

    pub(crate) fn note_prompt_output(&mut self, session_id: &str) {
        if let Some(state) = self.prompt_activity.get_mut(session_id) {
            state.last_output_at = Some(Instant::now());
        }
    }

    pub(crate) fn clear_prompt_activity(&mut self, session_id: &str) {
        self.prompt_activity.remove(session_id);
    }

    pub(crate) fn note_prompt_settlement_requested(&mut self, session_id: &str) {
        self.prompt_activity
            .entry(session_id.to_string())
            .and_modify(|state| state.last_output_at = Some(Instant::now()))
            .or_insert(ActivePromptState {
                last_output_at: Some(Instant::now()),
            });
    }

    pub(crate) fn maybe_complete_active_prompt(
        &mut self,
        session_id: &str,
    ) -> Result<(), DaemonError> {
        let should_complete = self
            .prompt_activity
            .get(session_id)
            .and_then(|state| state.last_output_at)
            .map(|last_output_at| last_output_at.elapsed() >= self.prompt_idle_timeout)
            .unwrap_or(false);

        if !should_complete {
            return Ok(());
        }

        if self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .is_none()
        {
            self.clear_prompt_activity(session_id);
            return Ok(());
        }

        if self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .map(|prompt| prompt.status())
            == Some(PromptStatus::Cancelling)
        {
            let _ = self.finalize_active_prompt_cancellation(session_id)?;
        } else {
            let _ = self.complete_active_prompt(session_id)?;
        }
        Ok(())
    }

    pub(crate) fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() == crate::provider::ProviderRunState::Ended {
            if self
                .sessions
                .get_session(session_id)?
                .active_provider_run_id()
                == Some(provider_run_id)
            {
                self.sessions.set_active_provider_run(session_id, None)?;
            }
            let _ = self.pty.remove_process(provider_run_id)?;
            self.providers.clear_runtime(provider_run_id);
            return Ok(true);
        }

        if provider_run.endpoint_mode() == crate::provider::AgentEndpointMode::External {
            return Ok(false);
        }

        let process_running = match self.pty.poll_process_state(provider_run_id) {
            Ok(PtyProcessState::Running) => true,
            Ok(PtyProcessState::Exited) => false,
            Err(DaemonError::PtyProcessNotFound { .. }) => false,
            Err(error) => return Err(error),
        };
        if process_running {
            return Ok(false);
        }

        let had_active_prompt = self
            .sessions
            .get_session(session_id)?
            .active_prompt()
            .is_some();
        let ended_run =
            self.providers
                .mark_run_ended(&mut self.sessions, session_id, provider_run_id)?;
        let _ = self.pty.remove_process(provider_run_id)?;

        if had_active_prompt {
            let active_prompt_status = self
                .sessions
                .get_session(session_id)?
                .active_prompt()
                .map(|prompt| prompt.status());
            if active_prompt_status == Some(PromptStatus::Cancelling) {
                let cancelled = self
                    .sessions
                    .finalize_active_prompt_cancellation(session_id)?
                    .1;
                self.reconcile_workflow_prompt_cancelled(session_id, &cancelled)?;
            } else {
                let completed = self.sessions.complete_active_prompt_only(session_id)?.1;
                self.reconcile_workflow_prompt_completed(
                    session_id,
                    &completed,
                    Some(provider_run_id),
                )?;
            }
            self.clear_prompt_activity(session_id);
        }
        self.providers.clear_runtime(provider_run_id);
        let started_next = if had_active_prompt {
            self.advance_next_queued_prompt(session_id)?
        } else {
            None
        };
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }

        self.record_notice(
            session_id,
            Some(provider_run_id),
            self.attachments.list_session_attachment_ids(session_id),
            format!(
                "Provider run `{}` for `{}` ended unexpectedly. {}",
                provider_run_id,
                ended_run.provider(),
                if had_active_prompt {
                    if started_next.is_some() {
                        "The active prompt was closed and Arroba advanced the queued backlog onto the next available provider run."
                    } else {
                        "The active prompt was closed without starting the queued backlog."
                    }
                } else {
                    "No active prompt was running."
                }
            ),
        );

        Ok(true)
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        let (_session, prompt) = self
            .sessions
            .finalize_active_prompt_cancellation(session_id)?;
        self.reconcile_workflow_prompt_cancelled(session_id, &prompt)?;
        self.clear_prompt_activity(session_id);
        let started_next = if self
            .sessions
            .get_session(session_id)?
            .active_provider_run_id()
            .is_some()
        {
            self.advance_next_queued_prompt(session_id)?
        } else {
            None
        };
        if started_next.is_none() {
            self.sync_focused_provider_run_if_idle(session_id)?;
        }

        Ok(PromptCancellation {
            prompt,
            started_next,
        })
    }
}
