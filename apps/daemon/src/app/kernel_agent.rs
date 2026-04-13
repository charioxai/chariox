use super::prompt_lifecycle::{
    KernelPreparedPromptSubmission, KernelPromptAbortDispatch, KernelPromptCancellation,
    KernelPromptDispatch, KernelPromptSubmission, KernelRemotePromptDispatch,
};
use super::DaemonApp;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::session::{
    PromptAttachment, PromptCancellation, PromptCompletion, PromptQueueItem, PromptStatus,
    PromptSubmissionOutcome,
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

    pub(crate) fn cancel_active_after_prompt_start_failure(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
    ) {
        let _ = self
            .app
            .prompt_owner_cancel_active_prompt_only(session_id, agent_id);
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
        let queued_while_active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &target_agent_id)?
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

        let prepared_prompt = PromptQueueItem::new(
            self.app.sessions_mut().reserve_prompt_id(),
            attachment_id,
            &target_agent_id,
            prompt,
            PromptStatus::Queued,
        )
        .with_attachments(attachments.clone());
        let outcome = self.app.prompt_owner_submit_prepared_prompt(
            session_id,
            prepared_prompt,
            provider_run_is_starting,
        )?;

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
                            let _ = self.app.prompt_owner_cancel_active_prompt_only(
                                session_id,
                                &target_agent_id,
                            );
                            return Err(DaemonError::LocalTransport {
                                operation: "submit remote prompt",
                                message: format!("unexpected remote prompt response: {other:?}"),
                            });
                        }
                        Err(error) => {
                            let _ = self.app.prompt_owner_cancel_active_prompt_only(
                                session_id,
                                &target_agent_id,
                            );
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
                let queue_depth = self
                    .app
                    .prompt_owner_queued_prompt_count_for_agent(session_id, &target_agent_id)
                    .unwrap_or(0);
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
                        queue_depth
                    ),
                );
            }
        }

        self.app.publish_session_projection(session_id)?;
        Ok(outcome)
    }

    pub(crate) fn submit_prepared_prompt_for_kernel(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        let session_id = prepared.session_id.as_str();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        self.app
            .ensure_attachment_in_session(session_id, &attachment_id)?;

        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            self.app.spawn_user_prompt_history_append(
                session_id,
                &attachment_id,
                &target_agent_id,
                prepared.prompt.prompt(),
                prepared.prompt.attachments(),
            )?;
            let outcome = self.app.prompt_owner_submit_prepared_prompt(
                session_id,
                prepared.prompt.clone(),
                prepared.force_queue,
            )?;
            let mut remote_dispatch = None;
            if let PromptSubmissionOutcome::Started { prompt } = &outcome {
                remote_dispatch = Some(KernelRemotePromptDispatch {
                    session_id: session_id.to_string(),
                    agent_id: target_agent_id.clone(),
                    worker_kernel_id: remote_execution.worker_kernel_id,
                    leased_agent_id: remote_execution.leased_agent_id,
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                    workflow_context: None,
                });
            }
            let session = self.app.local_api_session_snapshot(session_id)?;
            return Ok(KernelPromptSubmission {
                outcome,
                session,
                dispatch: None,
                remote_dispatch,
            });
        }

        let queued_while_active = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &target_agent_id)?
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

        self.app.spawn_user_prompt_history_append(
            session_id,
            &attachment_id,
            &target_agent_id,
            prepared.prompt.prompt(),
            prepared.prompt.attachments(),
        )?;

        let outcome = self.app.prompt_owner_submit_prepared_prompt(
            session_id,
            prepared.prompt.clone(),
            prepared.force_queue || provider_run_is_starting,
        )?;

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
                dispatch = Some(KernelPromptDispatch {
                    session_id: session_id.to_string(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: target_agent_id.clone(),
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                });
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                let queue_depth = self
                    .app
                    .prompt_owner_queued_prompt_count_for_agent(session_id, &target_agent_id)
                    .unwrap_or(0);
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
                    self.app.other_attachment_ids(session_id, &attachment_id),
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

        let session = self.app.local_api_session_snapshot(session_id)?;
        Ok(KernelPromptSubmission {
            outcome,
            session,
            dispatch,
            remote_dispatch: None,
        })
    }

    pub(crate) fn complete_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCompletion, DaemonError> {
        self.complete_active_prompt_for_kernel(session_id, agent_id, provider_run_id, None)
    }

    pub(crate) fn complete_active_prompt_for_kernel(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: Option<&PromptQueueItem>,
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
            let completed = self
                .app
                .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
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
                .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
                .is_none()
            {
                self.advance_next_queued_prompt_remote(
                    session_id,
                    agent_id,
                    &remote_execution.worker_kernel_id,
                    &remote_execution.leased_agent_id,
                    next_queued_prompt,
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
        let completed = self
            .app
            .prompt_owner_complete_active_prompt_only(session_id, agent_id)?;
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
        self.app
            .complete_workflow_prompt_from_runtime(session_id, &completed, provider_run_id)?;
        let completion_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.app
                .providers
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        if let Some(provider_run_id) = completion_provider_run_id.as_deref() {
            flow_control::clear_prompt_activity(self.app, provider_run_id);
        }
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_none()
        {
            self.advance_next_queued_prompt(session_id, agent_id, next_queued_prompt)?
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
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        loop {
            let next_candidate =
                self.next_queued_prompt_candidate(session_id, agent_id, expected_next)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let target_agent_id = peeked.target_agent_id().to_string();
            let is_workflow_prompt = self
                .app
                .is_workflow_prompt_source(peeked.source_attachment_id());
            let provider_run_id = match self
                .app
                .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)
            {
                Ok(provider_run_id) => provider_run_id,
                Err(DaemonError::NoActiveProviderRun { .. }) if is_workflow_prompt => {
                    match self
                        .app
                        .ensure_workflow_provider_run_from_runtime(session_id, &target_agent_id)
                    {
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

            let (_session, next_candidate) = self.activate_next_queued_prompt_for_mirror(
                session_id,
                &target_agent_id,
                expected_next,
            )?;
            let Some(next) = next_candidate else {
                flow_control::clear_prompt_activity(self.app, &provider_run_id);
                continue;
            };

            if let Err(error) = self
                .app
                .ensure_attachment_in_session(session_id, next.source_attachment_id())
            {
                if is_workflow_prompt {
                    let active = self.app.prompt_owner_activate_prompt(session_id, next)?;
                    if let Err(dispatch_error) = self.app.dispatch_prompt_to_provider(
                        session_id,
                        &provider_run_id,
                        active.source_attachment_id(),
                        active.prompt(),
                        active.attachments(),
                    ) {
                        let cancelled = self
                            .app
                            .prompt_owner_cancel_active_prompt_only(session_id, &target_agent_id)?;
                        self.app
                            .cancel_workflow_prompt_from_runtime(session_id, &cancelled)?;
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
                    self.app
                        .start_workflow_prompt_from_runtime(session_id, &active)?;
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
                    .prompt_owner_cancel_active_prompt_only(session_id, &target_agent_id);
                flow_control::clear_prompt_activity(self.app, &provider_run_id);
                continue;
            }

            let active = self.app.prompt_owner_activate_prompt(session_id, next)?;
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                (active.workflow_run_id(), active.workflow_node_run_id())
            {
                self.app.sessions_mut().mark_workflow_turn_dispatched(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
            }
            self.app
                .start_workflow_prompt_from_runtime(session_id, &active)?;
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
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        loop {
            let next_candidate =
                self.next_queued_prompt_candidate(session_id, agent_id, expected_next)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let is_workflow_prompt = self
                .app
                .is_workflow_prompt_source(peeked.source_attachment_id());
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
                    let _ = self.activate_next_queued_prompt_for_mirror(
                        session_id,
                        agent_id,
                        expected_next,
                    )?;
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
            let (_session, next_candidate) =
                self.activate_next_queued_prompt_for_mirror(session_id, agent_id, expected_next)?;
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
            self.app
                .start_workflow_prompt_from_runtime(session_id, &active)?;
            self.app.publish_session_projection(session_id)?;
            return Ok(Some(active));
        }
    }

    fn peek_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        if let Some(prompt) = self
            .app
            .agent_runtime_projection_store()
            .next_queued_prompt(session_id, agent_id)
        {
            return Ok(Some(prompt));
        }
        self.app
            .prompt_owner_peek_next_queued_prompt(session_id, agent_id)
    }

    fn next_queued_prompt_candidate(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        if let Some(expected_next) = expected_next {
            return Ok(select_next_queued_prompt_candidate(
                Some(expected_next),
                None,
            ));
        }
        Ok(select_next_queued_prompt_candidate(
            None,
            self.peek_next_queued_prompt(session_id, agent_id)?,
        ))
    }

    fn activate_next_queued_prompt_for_mirror(
        &mut self,
        session_id: &str,
        agent_id: &str,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<
        (
            crate::session::RuntimeSession,
            Option<crate::session::PromptQueueItem>,
        ),
        DaemonError,
    > {
        let expected_prompt_id = expected_next.map(PromptQueueItem::id);
        let next = self.app.prompt_owner_activate_next_queued_prompt(
            session_id,
            agent_id,
            expected_prompt_id,
        )?;
        let session = self.app.local_api_session_snapshot(session_id)?;
        Ok((session, next))
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
            .prompt_owner_active_prompt_agent_id(session_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        self.cancel_active_prompt_internal(session_id, &target_agent_id, Some(attachment_id))
    }

    pub(crate) fn cancel_active_prompt_for_runtime(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        let target_agent_id = self
            .app
            .prompt_owner_active_prompt_agent_id(session_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
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
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
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
            let prompt = self
                .app
                .prompt_owner_begin_cancelling_active_prompt(session_id, agent_id)?;
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

        let prompt = self
            .app
            .prompt_owner_begin_cancelling_active_prompt(session_id, agent_id)?;
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
        let prompt = self
            .app
            .prompt_owner_finalize_active_prompt_cancellation(session_id, agent_id)?;
        self.app
            .cancel_workflow_prompt_from_runtime(session_id, &prompt)?;
        let cancellation_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
            self.app
                .providers
                .get_run_for_agent(session_id, agent_id)
                .map(|run| run.id().to_string())
        });
        if let Some(provider_run_id) = cancellation_provider_run_id.as_deref() {
            flow_control::clear_prompt_activity(self.app, provider_run_id);
        }
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, agent_id)?
            .is_none()
        {
            self.advance_next_queued_prompt(session_id, agent_id, None)?
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
            let cancellation = self.cancel_active_prompt_internal(
                session_id,
                target_agent_id,
                Some(attachment_id),
            )?;
            let session = self.app.local_api_session_snapshot(session_id)?;
            return Ok(KernelPromptCancellation {
                cancellation,
                session,
                dispatch: None,
            });
        }

        let active_prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, target_agent_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == PromptStatus::Cancelling {
            let session = self.app.local_api_session_snapshot(session_id)?;
            return Ok(KernelPromptCancellation {
                cancellation: PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
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

        let prompt = self
            .app
            .prompt_owner_begin_cancelling_active_prompt(session_id, target_agent_id)?;
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
        let session = self.app.publish_session_projection(session_id)?;

        Ok(KernelPromptCancellation {
            cancellation: PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch,
        })
    }
}

fn select_next_queued_prompt_candidate(
    expected_next: Option<&PromptQueueItem>,
    fallback_next: Option<PromptQueueItem>,
) -> Option<PromptQueueItem> {
    expected_next.cloned().or(fallback_next)
}

#[cfg(test)]
mod tests {
    use super::select_next_queued_prompt_candidate;
    use crate::agent::RemoteAgentBinding;
    use crate::app::KernelPreparedPromptSubmission;
    use crate::attachment::ClientCapabilityLevel;
    use crate::local::{
        AttachToSessionRequest, LaunchProviderRunRequest, LocalDaemonRequest, LocalDaemonResponse,
    };
    use crate::session::{
        CreateSessionRequest, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
    };
    use crate::{DaemonApp, DaemonConfig};

    #[test]
    fn queue_candidate_selection_prefers_runtime_expected_prompt() {
        let runtime_expected = prompt_item("prompt-runtime");
        let stale_fallback = prompt_item("prompt-fallback");

        let selected =
            select_next_queued_prompt_candidate(Some(&runtime_expected), Some(stale_fallback))
                .expect("candidate should be selected");

        assert_eq!(selected.id(), "prompt-runtime");
    }

    #[test]
    fn prepared_remote_submit_returns_dispatch_without_relay_io() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "cli-remote-submit".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                },
            )
            .expect("agent should bind to remote execution");
        let prompt = PromptQueueItem::new(
            app.sessions_mut().reserve_prompt_id(),
            attachment.id(),
            agent.id(),
            "remote prompt should dispatch after ack",
            PromptStatus::Queued,
        );

        let prepared = app
            .kernel_agents()
            .submit_prepared_prompt_for_kernel(KernelPreparedPromptSubmission {
                session_id: session.id().to_string(),
                prompt,
                force_queue: false,
            })
            .expect("prepared remote submit should not require relay I/O");

        assert!(prepared.dispatch.is_none());
        let remote_dispatch = prepared
            .remote_dispatch
            .expect("started remote prompt should return deferred relay dispatch");
        assert_eq!(remote_dispatch.worker_kernel_id, "worker-kernel-1");
        assert_eq!(remote_dispatch.leased_agent_id, "leased-agent-1");
        match prepared.outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                assert_eq!(prompt.prompt(), "remote prompt should dispatch after ack");
            }
            PromptSubmissionOutcome::Queued { .. } => panic!("remote prompt should start"),
        }
        assert!(
            app.prompt_owner_active_prompt_for_agent(session.id(), agent.id())
                .expect("prompt owner should resolve")
                .is_some(),
            "remote relay dispatch is now a deferred side effect; prompt ownership is already recorded"
        );
    }

    #[test]
    fn completion_uses_prompt_owner_when_session_mirror_is_stale() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace", "worktree"))
            .expect("session should be created");
        let attachment = match app
            .handle_local_request(LocalDaemonRequest::AttachToSession(
                AttachToSessionRequest {
                    session_id: session.id().to_string(),
                    client_id: "cli-1".to_string(),
                    capability_level: ClientCapabilityLevel::FullTerminal,
                },
            ))
            .expect("attach should succeed")
        {
            LocalDaemonResponse::SessionAttached { attachment } => attachment,
            _ => panic!("unexpected local response"),
        };
        let provider_run = match app
            .handle_local_request(LocalDaemonRequest::LaunchProviderRun(
                LaunchProviderRunRequest {
                    session_id: session.id().to_string(),
                    agent_id: Some(agent.id().to_string()),
                    adapter_key: "dev-stub".to_string(),
                    provider: "claude-code".to_string(),
                    account_profile: "default".to_string(),
                    model: "sonnet".to_string(),
                    variant: None,
                },
            ))
            .expect("provider launch should succeed")
        {
            LocalDaemonResponse::ProviderRunLaunched { provider_run } => provider_run,
            _ => panic!("unexpected local response"),
        };

        let outcome = app
            .submit_prompt(
                session.id(),
                attachment.id(),
                Some(agent.id()),
                "hello",
                Vec::new(),
            )
            .expect("prompt submit should succeed");
        let prompt_id = match outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            _ => panic!("prompt should start"),
        };

        app.sessions_mut()
            .cancel_active_prompt(session.id(), agent.id())
            .expect("test should be able to corrupt only the compatibility mirror");
        assert!(
            app.sessions()
                .get_session(session.id())
                .expect("session mirror should exist")
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "compatibility mirror is intentionally stale"
        );

        let completion = app
            .complete_active_prompt(session.id(), agent.id(), Some(provider_run.id()))
            .expect("prompt owner should still complete active prompt");

        assert_eq!(completion.completed.id(), prompt_id);
        assert!(
            app.sessions()
                .get_session(session.id())
                .expect("session mirror should exist")
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "owner completion should remirror the idle state"
        );
    }

    fn prompt_item(id: &str) -> PromptQueueItem {
        PromptQueueItem::new(
            id.to_string(),
            "attachment-1",
            "agent-1",
            "prompt",
            PromptStatus::Queued,
        )
    }
}
