use super::prompt_lifecycle::{
    KernelPromptAbortDispatch, KernelPromptCancellation, KernelPromptDispatch,
    KernelPromptSubmission,
};
use super::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::session::{
    PromptAttachment, PromptCancellation, PromptCompletion, PromptStatus, PromptSubmissionOutcome,
};
use crate::transport::flow_control;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

pub(crate) struct KernelAgentService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    fn acquire_provider_prompt_claim(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        agent_id: &str,
        attachment_id: &str,
    ) -> Result<(), DaemonError> {
        self.app.acquire_prompt_workspace_claim(
            session_id,
            provider_run_id,
            agent_id,
            Some(attachment_id),
        )
    }

    fn cancel_active_after_prompt_start_failure(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) {
        let _ = self.app.sessions.cancel_active_prompt(session_id, agent_id);
        flow_control::clear_prompt_activity(self.app, provider_run_id);
    }

    pub(crate) fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.app.sessions.get_session(session_id)?;

        let target_agent_id = target_agent_id
            .or_else(|| session_before.focused_agent_id())
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })?
            .to_string();
        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        let remote_execution = target_agent.remote_execution().cloned();
        let queued_while_active = session_before
            .active_prompt_for_agent(&target_agent_id)
            .is_some();
        let provider_run_id = if remote_execution.is_some() {
            None
        } else if queued_while_active {
            self.app
                .providers
                .get_run_for_agent(session_id, &target_agent_id)
                .map(|run| run.id().to_string())
        } else {
            Some(
                self.app
                    .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)?,
            )
        };
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.app.providers.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == ProviderRunState::Starting);

        self.app.append_user_prompt_history(
            session_id,
            attachment_id,
            &target_agent_id,
            prompt,
            &attachments,
        );

        let (_session, outcome) = if provider_run_is_starting {
            self.app.sessions.queue_prompt(
                session_id,
                attachment_id,
                &target_agent_id,
                prompt,
                attachments.clone(),
            )?
        } else {
            self.app.sessions.submit_prompt(
                session_id,
                attachment_id,
                &target_agent_id,
                prompt,
                attachments.clone(),
            )?
        };

        match &outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                if let Some(remote_execution) = remote_execution.as_ref() {
                    let response =
                        self.app
                            .block_on_relay_future(send_peer_request_via_temporary_connection(
                                self.app.config(),
                                ClientTarget {
                                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                    daemon_alias: None,
                                },
                                RelayPeerRequest::SubmitLeasedPrompt {
                                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                                    prompt: prompt.prompt().to_string(),
                                    attachments: self.app.serialize_remote_prompt_attachments(
                                        prompt.attachments(),
                                    )?,
                                    workflow_context: None,
                                },
                            ));
                    let remote_provider_run_id = match response {
                        Ok(RelayPeerResponse::LeasedPromptSubmitted {
                            provider_run_id, ..
                        }) => provider_run_id,
                        Ok(other) => {
                            let _ = self
                                .app
                                .sessions
                                .cancel_active_prompt(session_id, &target_agent_id);
                            return Err(DaemonError::LocalTransport {
                                operation: "submit remote prompt",
                                message: format!("unexpected remote prompt response: {other:?}"),
                            });
                        }
                        Err(error) => {
                            let _ = self
                                .app
                                .sessions
                                .cancel_active_prompt(session_id, &target_agent_id);
                            return Err(error);
                        }
                    };
                    self.app.echo_prompt_to_other_attachments(
                        session_id,
                        &remote_provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                    return Ok(outcome);
                }
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.to_string(),
                        })?;
                self.app.echo_prompt_to_other_attachments(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.acquire_provider_prompt_claim(
                    session_id,
                    provider_run_id,
                    &target_agent_id,
                    prompt.source_attachment_id(),
                ) {
                    self.cancel_active_after_prompt_start_failure(
                        session_id,
                        &target_agent_id,
                        provider_run_id,
                    );
                    return Err(error);
                }
                if let Err(error) = self.app.dispatch_prompt_to_provider(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                ) {
                    self.cancel_active_after_prompt_start_failure(
                        session_id,
                        &target_agent_id,
                        provider_run_id,
                    );
                    return Err(error);
                }
                flow_control::note_prompt_started(self.app, provider_run_id);
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.app.echo_prompt_to_other_attachments(
                        session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.app.record_notice(
                    session_id,
                    provider_run_id.as_deref(),
                    self.app.other_attachment_ids(session_id, attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        target_agent_id,
                        session_id,
                        prompt.id(),
                        session_before
                            .queued_prompts_for_agent(&target_agent_id)
                            .map(|queue| queue.len())
                            .unwrap_or(0)
                            + 1
                    ),
                );
            }
        }

        self.app.publish_session_projection(session_id)?;
        Ok(outcome)
    }

    pub(crate) fn submit_prompt_for_kernel(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.app.sessions.get_session(session_id)?;

        let target_agent_id = target_agent_id
            .or_else(|| session_before.focused_agent_id())
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })?
            .to_string();
        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            let outcome = self.submit_prompt(
                session_id,
                attachment_id,
                Some(&target_agent_id),
                prompt,
                attachments,
            )?;
            let session = self.app.local_api_session_snapshot(session_id)?;
            return Ok(KernelPromptSubmission {
                outcome,
                session,
                dispatch: None,
            });
        }

        let queued_while_active = session_before
            .active_prompt_for_agent(&target_agent_id)
            .is_some();
        let provider_run_id = if queued_while_active {
            self.app
                .providers
                .get_run_for_agent(session_id, &target_agent_id)
                .map(|run| run.id().to_string())
        } else {
            Some(
                self.app
                    .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)?,
            )
        };
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.app.providers.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == ProviderRunState::Starting);

        self.app.append_user_prompt_history(
            session_id,
            attachment_id,
            &target_agent_id,
            prompt,
            &attachments,
        );

        let (_session, outcome) = if provider_run_is_starting {
            self.app.sessions.queue_prompt(
                session_id,
                attachment_id,
                &target_agent_id,
                prompt,
                attachments.clone(),
            )?
        } else {
            self.app.sessions.submit_prompt(
                session_id,
                attachment_id,
                &target_agent_id,
                prompt,
                attachments.clone(),
            )?
        };

        let mut dispatch = None;
        match &outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: session_id.to_string(),
                        })?;
                self.app.echo_prompt_to_other_attachments(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.acquire_provider_prompt_claim(
                    session_id,
                    provider_run_id,
                    &target_agent_id,
                    prompt.source_attachment_id(),
                ) {
                    self.cancel_active_after_prompt_start_failure(
                        session_id,
                        &target_agent_id,
                        provider_run_id,
                    );
                    return Err(error);
                }
                let provider_run = match self
                    .app
                    .prepare_provider_prompt_dispatch(session_id, provider_run_id)
                {
                    Ok(provider_run) => provider_run,
                    Err(error) => {
                        self.cancel_active_after_prompt_start_failure(
                            session_id,
                            &target_agent_id,
                            provider_run_id,
                        );
                        return Err(error);
                    }
                };
                if self
                    .app
                    .providers
                    .run_uses_structured_prompt_io(&provider_run)
                {
                    flow_control::note_prompt_started(self.app, provider_run_id);
                    dispatch = Some(KernelPromptDispatch {
                        session_id: session_id.to_string(),
                        provider_run_id: provider_run_id.to_string(),
                        agent_id: target_agent_id.clone(),
                        prompt: prompt.prompt().to_string(),
                        attachments: prompt.attachments().to_vec(),
                    });
                } else if let Err(error) = self.app.send_provider_input(
                    session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt().as_bytes(),
                ) {
                    let _ = self
                        .app
                        .sessions
                        .cancel_active_prompt(session_id, &target_agent_id);
                    flow_control::clear_prompt_activity(self.app, provider_run_id);
                    return Err(error);
                } else {
                    flow_control::note_prompt_started(self.app, provider_run_id);
                }
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                if let Some(provider_run_id) = provider_run_id.as_deref() {
                    self.app.echo_prompt_to_other_attachments(
                        session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.app.record_notice(
                    session_id,
                    provider_run_id.as_deref(),
                    self.app.other_attachment_ids(session_id, attachment_id),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        attachment_id,
                        target_agent_id,
                        session_id,
                        prompt.id(),
                        session_before
                            .queued_prompts_for_agent(&target_agent_id)
                            .map(|queue| queue.len())
                            .unwrap_or(0)
                            + 1
                    ),
                );
            }
        }

        let session = self.app.local_api_session_snapshot(session_id)?;
        Ok(KernelPromptSubmission {
            outcome,
            session,
            dispatch,
        })
    }

    pub(crate) fn complete_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCompletion, DaemonError> {
        let target_agent = self.app.agents.get_agent(agent_id)?;
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            let remote_provider_run_id =
                match self
                    .app
                    .block_on_relay_future(send_peer_request_via_temporary_connection(
                        self.app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CompleteLeasedPrompt {
                            leased_agent_id: remote_execution.leased_agent_id.clone(),
                        },
                    ))? {
                    RelayPeerResponse::LeasedPromptCompleted {
                        provider_run_id, ..
                    } => provider_run_id,
                    other => {
                        return Err(DaemonError::LocalTransport {
                            operation: "complete remote prompt",
                            message: format!(
                                "unexpected remote prompt completion response: {other:?}"
                            ),
                        });
                    }
                };
            let (_session, completed) = self
                .app
                .sessions
                .complete_active_prompt_only(session_id, agent_id)?;
            let recipient_attachment_ids =
                self.app.attachments.list_session_attachment_ids(session_id);
            self.app.record_assistant_message_completion(
                session_id,
                remote_provider_run_id
                    .as_deref()
                    .unwrap_or("remote-provider-run-completed"),
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            let started_next = if self
                .app
                .sessions
                .get_session(session_id)?
                .active_prompt_for_agent(agent_id)
                .is_none()
            {
                self.advance_next_queued_prompt_remote(
                    session_id,
                    agent_id,
                    &remote_execution.worker_kernel_id,
                    &remote_execution.leased_agent_id,
                )?
            } else {
                None
            };
            if started_next.is_none() {
                self.app.sync_focused_provider_run_if_idle(session_id)?;
            }
            self.app.publish_session_projection(session_id)?;
            return Ok(PromptCompletion {
                completed,
                started_next,
            });
        }
        let (_session, completed) = self
            .app
            .sessions
            .complete_active_prompt_only(session_id, agent_id)?;
        if !flow_control::prompt_completion_recorded(self.app, provider_run_id.unwrap_or(agent_id))
        {
            let recipient_attachment_ids =
                self.app.attachments.list_session_attachment_ids(session_id);
            let completion_provider_run_id = provider_run_id.unwrap_or("provider-run-completed");
            self.app.record_assistant_message_completion(
                session_id,
                completion_provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            flow_control::mark_prompt_completion_recorded(self.app, completion_provider_run_id);
        }
        crate::scheduler::runtime::on_workflow_prompt_completed(
            self.app,
            session_id,
            &completed,
            provider_run_id,
        )?;
        if let Some(provider_run_id) = provider_run_id {
            flow_control::clear_prompt_activity(self.app, provider_run_id);
        }
        let started_next = if self
            .app
            .sessions
            .get_session(session_id)?
            .active_prompt_for_agent(agent_id)
            .is_none()
        {
            self.advance_next_queued_prompt(session_id, agent_id)?
        } else {
            None
        };
        if started_next.is_none() {
            self.app.sync_focused_provider_run_if_idle(session_id)?;
        }
        self.app.publish_session_projection(session_id)?;

        Ok(PromptCompletion {
            completed,
            started_next,
        })
    }

    pub(crate) fn advance_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        loop {
            let next_candidate = self
                .app
                .sessions
                .peek_next_queued_prompt(session_id, agent_id)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let target_agent_id = peeked.target_agent_id().to_string();
            let is_workflow_prompt = crate::scheduler::runtime::is_workflow_prompt_attachment(
                peeked.source_attachment_id(),
            );
            let provider_run_id = match self
                .app
                .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)
            {
                Ok(provider_run_id) => provider_run_id,
                Err(DaemonError::NoActiveProviderRun { .. }) if is_workflow_prompt => {
                    match crate::scheduler::runtime::ensure_workflow_provider_run_for_agent(
                        self.app,
                        session_id,
                        &target_agent_id,
                    ) {
                        Ok(provider_run_id) => provider_run_id,
                        Err(error) => {
                            self.app.record_notice(
                                session_id,
                                None,
                                self.app.attachments.list_session_attachment_ids(session_id),
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
                    self.app.record_notice(
                        session_id,
                        None,
                        self.app.attachments.list_session_attachment_ids(session_id),
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
            if let Err(error) = self.acquire_provider_prompt_claim(
                session_id,
                &provider_run_id,
                &target_agent_id,
                peeked.source_attachment_id(),
            ) {
                self.app.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.app.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Deferred queued prompt `{}` because worktree coordination rejected provider dispatch: {}",
                        peeked.id(),
                        error
                    ),
                );
                return Ok(None);
            }

            let (_session, next_candidate) = self
                .app
                .sessions
                .activate_next_queued_prompt(session_id, &target_agent_id)?;
            let Some(next) = next_candidate else {
                flow_control::clear_prompt_activity(self.app, &provider_run_id);
                continue;
            };

            if let Err(error) = self
                .app
                .ensure_attachment_in_session(session_id, next.source_attachment_id())
            {
                if is_workflow_prompt {
                    let active = self.app.sessions.activate_prompt(session_id, next)?.1;
                    if let Err(dispatch_error) = self.app.dispatch_prompt_to_provider(
                        session_id,
                        &provider_run_id,
                        active.source_attachment_id(),
                        active.prompt(),
                        active.attachments(),
                    ) {
                        let cancelled = self
                            .app
                            .sessions
                            .cancel_active_prompt(session_id, &target_agent_id)?
                            .1;
                        crate::scheduler::runtime::on_workflow_prompt_cancelled(
                            self.app, session_id, &cancelled,
                        )?;
                        flow_control::clear_prompt_activity(self.app, &provider_run_id);
                        return Err(dispatch_error);
                    }
                    if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                        (active.workflow_run_id(), active.workflow_node_run_id())
                    {
                        self.app.sessions_mut().mark_workflow_turn_dispatched(
                            session_id,
                            workflow_run_id,
                            workflow_node_run_id,
                        )?;
                    }
                    crate::scheduler::runtime::on_workflow_prompt_started(
                        self.app, session_id, &active,
                    )?;
                    flow_control::note_prompt_started(self.app, &provider_run_id);
                    self.app.publish_session_projection(session_id)?;
                    return Ok(Some(active));
                }
                self.app.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.app.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                        next.id(),
                        error
                    ),
                );
                flow_control::clear_prompt_activity(self.app, &provider_run_id);
                continue;
            }

            if let Err(error) = self.app.dispatch_prompt_to_provider(
                session_id,
                &provider_run_id,
                next.source_attachment_id(),
                next.prompt(),
                next.attachments(),
            ) {
                self.app.record_notice(
                    session_id,
                    Some(&provider_run_id),
                    self.app.attachments.list_session_attachment_ids(session_id),
                    format!(
                        "Skipped queued prompt `{}` after PTY delivery failure: {}",
                        next.id(),
                        error
                    ),
                );
                let _ = self
                    .app
                    .sessions
                    .cancel_active_prompt(session_id, &target_agent_id);
                flow_control::clear_prompt_activity(self.app, &provider_run_id);
                continue;
            }

            let active = self.app.sessions.activate_prompt(session_id, next)?.1;
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                (active.workflow_run_id(), active.workflow_node_run_id())
            {
                self.app.sessions_mut().mark_workflow_turn_dispatched(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
            }
            crate::scheduler::runtime::on_workflow_prompt_started(self.app, session_id, &active)?;
            flow_control::note_prompt_started(self.app, &provider_run_id);
            self.app.publish_session_projection(session_id)?;
            return Ok(Some(active));
        }
    }

    pub(crate) fn advance_next_queued_prompt_remote(
        &mut self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        loop {
            let next_candidate = self
                .app
                .sessions
                .peek_next_queued_prompt(session_id, agent_id)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let is_workflow_prompt = crate::scheduler::runtime::is_workflow_prompt_attachment(
                peeked.source_attachment_id(),
            );
            if let Err(error) = self
                .app
                .ensure_attachment_in_session(session_id, peeked.source_attachment_id())
            {
                if !is_workflow_prompt {
                    self.app.record_notice(
                        session_id,
                        None,
                        self.app.attachments.list_session_attachment_ids(session_id),
                        format!(
                            "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                            peeked.id(),
                            error
                        ),
                    );
                    let _ = self
                        .app
                        .sessions
                        .activate_next_queued_prompt(session_id, agent_id)?;
                    continue;
                }
            }
            let response =
                self.app
                    .block_on_relay_future(send_peer_request_via_temporary_connection(
                        self.app.config(),
                        ClientTarget {
                            daemon_id: Some(worker_kernel_id.to_string()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::SubmitLeasedPrompt {
                            leased_agent_id: leased_agent_id.to_string(),
                            prompt: peeked.prompt().to_string(),
                            attachments: self
                                .app
                                .serialize_remote_prompt_attachments(peeked.attachments())?,
                            workflow_context: if is_workflow_prompt {
                                Some(self.app.remote_workflow_turn_context_for_prompt(
                                    session_id, agent_id, &peeked,
                                )?)
                            } else {
                                None
                            },
                        },
                    ));
            let remote_provider_run_id = match response {
                Ok(RelayPeerResponse::LeasedPromptSubmitted {
                    provider_run_id, ..
                }) => provider_run_id,
                Ok(other) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "advance remote queued prompt",
                        message: format!("unexpected remote prompt response: {other:?}"),
                    });
                }
                Err(error) => return Err(error),
            };
            let (_session, next_candidate) = self
                .app
                .sessions
                .activate_next_queued_prompt(session_id, agent_id)?;
            let Some(active) = next_candidate else {
                continue;
            };
            self.app.echo_prompt_to_other_attachments(
                session_id,
                &remote_provider_run_id,
                active.source_attachment_id(),
                active.prompt(),
                active.attachments(),
            );
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                (active.workflow_run_id(), active.workflow_node_run_id())
            {
                self.app.sessions_mut().mark_workflow_turn_dispatched(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
            }
            crate::scheduler::runtime::on_workflow_prompt_started(self.app, session_id, &active)?;
            self.app.publish_session_projection(session_id)?;
            return Ok(Some(active));
        }
    }

    pub(crate) fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent_id = self
            .app
            .sessions
            .get_session(session_id)?
            .focused_agent_id()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?
            .to_string();
        self.cancel_active_prompt_internal(session_id, &target_agent_id, Some(attachment_id))
    }

    pub(crate) fn cancel_active_prompt_for_runtime(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        let target_agent_id = self
            .app
            .sessions
            .get_session(session_id)?
            .focused_agent_id()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?
            .to_string();
        self.cancel_active_prompt_internal(session_id, &target_agent_id, None)
    }

    pub(crate) fn cancel_active_prompt_internal(
        &mut self,
        session_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        let active_prompt = self
            .app
            .sessions
            .get_session(session_id)?
            .active_prompt_for_agent(agent_id)
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
        let target_agent = self.app.agents.get_agent(agent_id)?;
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            match self
                .app
                .block_on_relay_future(send_peer_request_via_temporary_connection(
                    self.app.config(),
                    ClientTarget {
                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::CancelLeasedPrompt {
                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                    },
                ))? {
                RelayPeerResponse::LeasedPromptCancelled { .. } => {}
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "cancel remote prompt",
                        message: format!(
                            "unexpected remote prompt cancellation response: {other:?}"
                        ),
                    });
                }
            }
            let (_session, prompt) = self
                .app
                .sessions
                .begin_cancelling_active_prompt(session_id, agent_id)?;
            let recipients = match attachment_id {
                Some(attachment_id) => self.app.other_attachment_ids(session_id, attachment_id),
                None => self.app.attachments.list_session_attachment_ids(session_id),
            };
            let message = match attachment_id {
                Some(attachment_id) => format!(
                    "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                    active_prompt.id(),
                    remote_execution.worker_kernel_id
                ),
                None => format!(
                    "Arroba requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                    active_prompt.id(),
                    remote_execution.worker_kernel_id
                ),
            };
            self.app
                .record_notice(session_id, None, recipients, message);
            self.app.publish_session_projection(session_id)?;
            return Ok(PromptCancellation {
                prompt,
                started_next: None,
            });
        }
        let provider_run_id = self
            .app
            .providers
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string())
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self
            .app
            .ensure_provider_run_in_session(session_id, &provider_run_id)?;

        let uses_structured_prompt_io = self
            .app
            .providers
            .run_uses_structured_prompt_io(&provider_run);
        if !uses_structured_prompt_io {
            self.app.send_provider_input(
                session_id,
                &provider_run_id,
                attachment_id.unwrap_or(active_prompt.source_attachment_id()),
                b"\x03",
            )?;
        }

        let (_session, prompt) = self
            .app
            .sessions
            .begin_cancelling_active_prompt(session_id, agent_id)?;
        flow_control::note_prompt_settlement_requested(self.app, &provider_run_id);
        if uses_structured_prompt_io {
            self.app
                .providers
                .enqueue_structured_prompt_abort(session_id.to_string(), provider_run_id.clone())?;
        }
        let recipients = match attachment_id {
            Some(attachment_id) => self.app.other_attachment_ids(session_id, attachment_id),
            None => self.app.attachments.list_session_attachment_ids(session_id),
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
        self.app
            .record_notice(session_id, Some(&provider_run_id), recipients, message);
        self.app.publish_session_projection(session_id)?;

        Ok(PromptCancellation {
            prompt,
            started_next: None,
        })
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        let (_session, prompt) = self
            .app
            .sessions
            .finalize_active_prompt_cancellation(session_id, agent_id)?;
        crate::scheduler::runtime::on_workflow_prompt_cancelled(self.app, session_id, &prompt)?;
        if let Some(provider_run_id) = provider_run_id {
            flow_control::clear_prompt_activity(self.app, provider_run_id);
        }
        let started_next = if self
            .app
            .sessions
            .get_session(session_id)?
            .active_prompt_for_agent(agent_id)
            .is_none()
        {
            self.advance_next_queued_prompt(session_id, agent_id)?
        } else {
            None
        };
        if started_next.is_none() {
            self.app.sync_focused_provider_run_if_idle(session_id)?;
        }
        self.app.publish_session_projection(session_id)?;

        Ok(PromptCancellation {
            prompt,
            started_next,
        })
    }

    pub(crate) fn cancel_agent_prompt_for_kernel(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return self
                .cancel_active_prompt_internal(session_id, target_agent_id, Some(attachment_id))
                .map(|cancellation| KernelPromptCancellation {
                    cancellation,
                    dispatch: None,
                });
        }

        let active_prompt = self
            .app
            .sessions
            .get_session(session_id)?
            .active_prompt_for_agent(target_agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == PromptStatus::Cancelling {
            return Ok(KernelPromptCancellation {
                cancellation: PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                dispatch: None,
            });
        }

        let provider_run_id = self
            .app
            .providers
            .get_run_for_agent(session_id, target_agent_id)
            .map(|run| run.id().to_string())
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self
            .app
            .ensure_provider_run_in_session(session_id, &provider_run_id)?;
        let dispatch = if self
            .app
            .providers
            .run_uses_structured_prompt_io(&provider_run)
        {
            Some(KernelPromptAbortDispatch {
                session_id: session_id.to_string(),
                provider_run_id: provider_run_id.clone(),
            })
        } else {
            self.app
                .send_provider_input(session_id, &provider_run_id, attachment_id, b"\x03")?;
            None
        };

        let (_session, prompt) = self
            .app
            .sessions
            .begin_cancelling_active_prompt(session_id, target_agent_id)?;
        flow_control::note_prompt_settlement_requested(self.app, &provider_run_id);
        self.app.record_notice(
            session_id,
            Some(&provider_run_id),
            self.app.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active prompt `{}` on provider run `{}`.",
                active_prompt.id(),
                provider_run.id()
            ),
        );
        self.app.publish_session_projection(session_id)?;

        Ok(KernelPromptCancellation {
            cancellation: PromptCancellation {
                prompt,
                started_next: None,
            },
            dispatch,
        })
    }
}
