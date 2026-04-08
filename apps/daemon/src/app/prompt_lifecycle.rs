use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::pty::PtyProcessState;
use crate::session::{PromptCancellation, PromptCompletion, PromptStatus, PromptSubmissionOutcome};
use crate::transport::flow_control;

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
                    flow_control::clear_prompt_activity(self, session_id);
                    return Err(error);
                }
                flow_control::note_prompt_started(self, session_id);
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
        if !flow_control::prompt_completion_recorded(self, session_id) {
            let recipient_attachment_ids = self.attachments.list_session_attachment_ids(session_id);
            let completion_provider_run_id = provider_run_id
                .as_deref()
                .unwrap_or("provider-run-completed");
            self.record_assistant_message_completion(
                session_id,
                completion_provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            flow_control::mark_prompt_completion_recorded(self, session_id);
        }
        crate::scheduler::runtime::on_workflow_prompt_completed(
            self,
            session_id,
            &completed,
            provider_run_id.as_deref(),
        )?;
        flow_control::clear_prompt_activity(self, session_id);
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
        self.cancel_active_prompt_internal(session_id, Some(attachment_id))
    }

    pub(crate) fn cancel_active_prompt_for_runtime(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        self.cancel_active_prompt_internal(session_id, None)
    }

    fn cancel_active_prompt_internal(
        &mut self,
        session_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
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
            self.send_provider_input(
                session_id,
                &provider_run_id,
                attachment_id.unwrap_or(active_prompt.source_attachment_id()),
                b"\x03",
            )?;
        }

        let (_session, prompt) = self.sessions.begin_cancelling_active_prompt(session_id)?;
        flow_control::note_prompt_settlement_requested(self, session_id);
        let recipients = match attachment_id {
            Some(attachment_id) => self.other_attachment_ids(session_id, attachment_id),
            None => self.attachments.list_session_attachment_ids(session_id),
        };
        let message = match attachment_id {
            Some(attachment_id) => format!(
                "Attachment `{attachment_id}` requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
            None => format!(
                "Arroba requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
        };
        self.record_notice(session_id, Some(&provider_run_id), recipients, message);

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
            let is_workflow_prompt = crate::scheduler::runtime::is_workflow_prompt_attachment(
                peeked.source_attachment_id(),
            );
            let provider_run_id = match self
                .ensure_active_provider_run_for_agent(session_id, &target_agent_id)
            {
                Ok(provider_run_id) => provider_run_id,
                Err(DaemonError::NoActiveProviderRun { .. }) if is_workflow_prompt => {
                    match crate::scheduler::runtime::ensure_workflow_provider_run_for_agent(
                        self,
                        session_id,
                        &target_agent_id,
                    ) {
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
                        crate::scheduler::runtime::on_workflow_prompt_cancelled(
                            self, session_id, &cancelled,
                        )?;
                        flow_control::clear_prompt_activity(self, session_id);
                        return Err(dispatch_error);
                    }
                    if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                        (active.workflow_run_id(), active.workflow_node_run_id())
                    {
                        self.sessions_mut().mark_workflow_turn_dispatched(
                            session_id,
                            workflow_run_id,
                            workflow_node_run_id,
                        )?;
                    }
                    crate::scheduler::runtime::on_workflow_prompt_started(
                        self, session_id, &active,
                    )?;
                    flow_control::note_prompt_started(self, session_id);
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
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                (active.workflow_run_id(), active.workflow_node_run_id())
            {
                self.sessions_mut().mark_workflow_turn_dispatched(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
            }
            crate::scheduler::runtime::on_workflow_prompt_started(self, session_id, &active)?;
            flow_control::note_prompt_started(self, session_id);
            return Ok(Some(active));
        }
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
            let _ = self.remove_tracked_provider_process_for_run(provider_run_id)?;
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
        let _ = self.remove_tracked_provider_process_for_run(provider_run_id)?;

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
                crate::scheduler::runtime::on_workflow_prompt_cancelled(
                    self, session_id, &cancelled,
                )?;
            } else {
                let completed = self.sessions.complete_active_prompt_only(session_id)?.1;
                crate::scheduler::runtime::on_workflow_prompt_completed(
                    self,
                    session_id,
                    &completed,
                    Some(provider_run_id),
                )?;
            }
            flow_control::clear_prompt_activity(self, session_id);
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
        crate::scheduler::runtime::on_workflow_prompt_cancelled(self, session_id, &prompt)?;
        flow_control::clear_prompt_activity(self, session_id);
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
